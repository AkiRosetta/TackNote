//! One sync: read the server's file, merge, upload if needed, and start over
//! when another device wrote in between.

use chrono::Utc;

use crate::merge::merge_notes;
use crate::model::Note;
use crate::order::{merge_order, NoteOrder};
use crate::webdav::{RemoteStore, SyncError, SyncErrorKind, WriteError};

pub const REMOTE_FORMAT_VERSION: u64 = 1;
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug)]
pub struct SyncOutcome {
    pub notes: Vec<Note>,
    pub conflicts: usize,
    /// The order the user dragged the notes into, merged like the notes.
    pub order: Option<NoteOrder>,
}

pub async fn sync<S: RemoteStore>(store: &S, local: &[Note], device_id: &str) -> Result<SyncOutcome, SyncError> {
    sync_with_order(store, local, None, device_id).await
}

pub async fn sync_with_order<S: RemoteStore>(
    store: &S,
    local: &[Note],
    local_order: Option<&NoteOrder>,
    device_id: &str,
) -> Result<SyncOutcome, SyncError> {
    for attempt in 1.. {
        let file = store.read().await?;
        let (remote, remote_order) = match &file {
            Some(file) => decode_file(&file.body)?,
            None => (Vec::new(), None),
        };
        let result = merge_notes(local, &remote, device_id, Utc::now());
        let order = merge_order(local_order, remote_order.as_ref());
        let outcome = SyncOutcome { notes: result.notes, conflicts: result.conflicts, order: order.order };
        if !result.remote_needs_update && !order.remote_needs_update && file.is_some() {
            return Ok(outcome);
        }
        match store.write(encode_file(&outcome.notes, outcome.order.as_ref()), file).await {
            Ok(()) => return Ok(outcome),
            Err(WriteError::Changed) if attempt < MAX_ATTEMPTS => continue,
            Err(WriteError::Changed) => {
                return Err(SyncError::new(SyncErrorKind::Server, "the file kept changing during sync"));
            }
            Err(WriteError::Failed(error)) => return Err(error),
        }
    }
    unreachable!("the loop returns on every path")
}

/// Fails rather than returning no notes, because treating a damaged file as
/// empty would overwrite the user's notes on the server.
pub fn decode(body: &str) -> Result<Vec<Note>, SyncError> {
    decode_file(body).map(|(notes, _)| notes)
}

/// The notes and, if any device has reordered them, the order.
pub fn decode_file(body: &str) -> Result<(Vec<Note>, Option<NoteOrder>), SyncError> {
    let corrupt = |detail: String| SyncError::new(SyncErrorKind::Corrupt, detail);
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| corrupt(error.to_string()))?;
    let Some(notes) = value.get("notes").filter(|notes| notes.is_array()) else {
        return Err(corrupt("unexpected structure".into()));
    };
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(version) if version <= REMOTE_FORMAT_VERSION => {}
        other => {
            return Err(SyncError::new(SyncErrorKind::UnsupportedVersion, format!("{other:?}")));
        }
    }
    let notes = serde_json::from_value(notes.clone()).map_err(|error| corrupt(error.to_string()))?;
    let order = match value.get("order") {
        None | Some(serde_json::Value::Null) => None,
        Some(order) => Some(serde_json::from_value(order.clone()).map_err(|error| corrupt(error.to_string()))?),
    };
    Ok((notes, order))
}

pub fn encode(notes: &[Note]) -> String {
    encode_file(notes, None)
}

pub fn encode_file(notes: &[Note], order: Option<&NoteOrder>) -> String {
    let uploaded: Vec<Note> = notes.iter().map(Note::for_upload).collect();
    let mut file = serde_json::json!({ "version": REMOTE_FORMAT_VERSION, "notes": uploaded });
    if let Some(order) = order {
        file["order"] = serde_json::to_value(order.for_upload()).expect("an order is plain data");
    }
    file.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::test_server::FakeWebDav;
    use crate::webdav::{http_client, WebDavConfig, WebDavStore};

    const FILE: &str = "/tacknote/notes.json";

    fn store(server: &FakeWebDav, password: &str) -> WebDavStore {
        let config = WebDavConfig { url: server.url(), username: "me".into(), password: password.into() };
        WebDavStore::new(config, http_client())
    }

    async fn run(server: &FakeWebDav, device: &str, local: &[Note]) -> Result<SyncOutcome, SyncError> {
        sync(&store(server, "secret"), local, device).await
    }

    fn note(text: &str, device: &str) -> Note {
        Note::new(device, text, Utc::now())
    }

    fn file_with(notes: &[Note]) -> String {
        encode(notes)
    }

    #[tokio::test]
    async fn the_first_sync_creates_the_folder_and_uploads() {
        let server = FakeWebDav::start();
        let outcome = run(&server, "a", &[note("你好", "a")]).await.unwrap();
        assert!(server.log().contains(&"MKCOL /tacknote/".to_string()));
        assert!(server.file(FILE).unwrap().contains("你好"));
        assert!(!outcome.notes[0].has_unsynced_changes());
    }

    #[tokio::test]
    async fn uploaded_data_never_contains_local_only_fields() {
        let server = FakeWebDav::start();
        run(&server, "a", &[note("x", "a")]).await.unwrap();
        assert!(!server.file(FILE).unwrap().contains("syncedRev"));
    }

    #[tokio::test]
    async fn a_second_device_receives_the_notes() {
        let server = FakeWebDav::start();
        run(&server, "a", &[note("from a", "a")]).await.unwrap();
        let outcome = run(&server, "b", &[]).await.unwrap();
        assert_eq!(outcome.notes[0].text, "from a");
    }

    #[tokio::test]
    async fn an_unchanged_sync_does_not_write_and_skips_mkcol() {
        let server = FakeWebDav::start();
        let first = run(&server, "a", &[note("x", "a")]).await.unwrap();
        server.clear_log();
        run(&server, "a", &first.notes).await.unwrap();
        assert_eq!(server.log(), vec![format!("GET {FILE}")]);
    }

    #[tokio::test]
    async fn later_uploads_do_not_send_mkcol() {
        let server = FakeWebDav::start();
        let first = run(&server, "a", &[note("x", "a")]).await.unwrap();
        server.clear_log();
        let mut local = first.notes.clone();
        local.push(note("y", "a"));
        run(&server, "a", &local).await.unwrap();
        assert_eq!(server.log(), vec![format!("GET {FILE}"), format!("PUT {FILE}")]);
    }

    #[tokio::test]
    async fn two_devices_uploading_the_first_file_at_once_keep_both() {
        let server = FakeWebDav::start();
        server.add_folder("/tacknote/");
        let other = note("from b", "b");
        let body = file_with(&[other]);
        server.before_next_put(move |state| state.set_file(FILE, &body));

        run(&server, "a", &[note("from a", "a")]).await.unwrap();

        let saved = server.file(FILE).unwrap();
        assert!(saved.contains("from a"));
        assert!(saved.contains("from b"));
    }

    #[tokio::test]
    async fn a_wrong_password_is_reported_as_an_auth_error() {
        let server = FakeWebDav::start();
        let error = sync(&store(&server, "wrong"), &[note("x", "a")], "a").await.unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::Auth);
    }

    #[tokio::test]
    async fn a_damaged_server_file_is_never_overwritten() {
        let server = FakeWebDav::start();
        server.add_folder("/tacknote/");
        server.set_file(FILE, "{not json");
        let error = run(&server, "a", &[note("x", "a")]).await.unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::Corrupt);
        assert_eq!(server.file(FILE).unwrap(), "{not json");
    }

    #[tokio::test]
    async fn a_file_from_a_newer_app_version_is_never_overwritten() {
        let server = FakeWebDav::start();
        server.add_folder("/tacknote/");
        let body = r#"{"version":99,"notes":[]}"#;
        server.set_file(FILE, body);
        let error = run(&server, "a", &[note("x", "a")]).await.unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::UnsupportedVersion);
        assert_eq!(server.file(FILE).unwrap(), body);
    }

    #[tokio::test]
    async fn a_write_that_races_another_device_retries_and_keeps_both() {
        let server = FakeWebDav::start();
        let first = run(&server, "a", &[note("from a", "a")]).await.unwrap();
        let other = note("from b", "b");
        let mut racing = first.notes.clone();
        racing.push(other);
        let body = file_with(&racing);
        server.before_next_put(move |state| state.set_file(FILE, &body));

        let mut local = first.notes.clone();
        local.push(note("second from a", "a"));
        let outcome = run(&server, "a", &local).await.unwrap();

        let texts: HashSet<String> = outcome.notes.iter().map(|n| n.text.clone()).collect();
        for expected in ["from a", "from b", "second from a"] {
            assert!(texts.contains(expected), "{expected}");
        }
        let saved = server.file(FILE).unwrap();
        assert!(saved.contains("from b") && saved.contains("second from a"));
    }

    #[tokio::test]
    async fn a_server_that_ignores_if_match_still_converges() {
        let server = FakeWebDav::start();
        server.set_honor_preconditions(false);
        let a = run(&server, "a", &[note("from a", "a")]).await.unwrap();
        let other = note("from b", "b");

        // Device A overwrites B's upload, then B simply uploads again.
        let mut with_b = a.notes.clone();
        with_b.push(other.clone());
        let body = file_with(&with_b);
        server.before_next_put(move |state| state.set_file(FILE, &body));
        let mut local = a.notes.clone();
        local.push(note("second from a", "a"));
        run(&server, "a", &local).await.unwrap();
        assert!(!server.file(FILE).unwrap().contains("from b"));

        let b = run(&server, "b", &[other]).await.unwrap();
        assert!(b.notes.iter().any(|n| n.text == "from b"));
        let saved = server.file(FILE).unwrap();
        assert!(saved.contains("from b") && saved.contains("second from a"));
    }

    /// reqwest already moves a password out of the address, so this checks the
    /// stronger promise: the error carries no part of the address at all.
    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error_without_the_address() {
        let config = WebDavConfig {
            url: "http://me:secret@127.0.0.1:9/private-folder/".into(),
            username: "me".into(),
            password: "secret".into(),
        };
        let error = sync(&WebDavStore::new(config, http_client()), &[note("x", "a")], "a").await.unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::Network);
        for hidden in ["secret", "private-folder", "127.0.0.1"] {
            assert!(!error.detail.contains(hidden), "{hidden} in {}", error.detail);
        }
    }

    #[tokio::test]
    async fn a_malformed_address_fails_before_any_request() {
        let config = WebDavConfig { url: "http://[bad".into(), username: "me".into(), password: "secret".into() };
        let error = sync(&WebDavStore::new(config, http_client()), &[], "a").await.unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::BadUrl);
    }

    #[test]
    fn decoding_rejects_bad_structures() {
        assert_eq!(decode("[]").unwrap_err().kind, SyncErrorKind::Corrupt);
        assert_eq!(decode(r#"{"notes":[]}"#).unwrap_err().kind, SyncErrorKind::UnsupportedVersion);
        assert_eq!(decode(r#"{"version":1,"notes":[{"id":1}]}"#).unwrap_err().kind, SyncErrorKind::Corrupt);
        assert!(decode(r#"{"version":1,"notes":[]}"#).unwrap().is_empty());
    }

    async fn run_ordered(server: &FakeWebDav, device: &str, local: &[Note], order: Option<&NoteOrder>) -> SyncOutcome {
        sync_with_order(&store(server, "secret"), local, order, device).await.unwrap()
    }

    #[tokio::test]
    async fn a_reorder_reaches_the_other_device() {
        let server = FakeWebDav::start();
        let notes = vec![note("one", "a"), note("two", "a")];
        let order = NoteOrder::new(vec![notes[1].id.clone(), notes[0].id.clone()], "a", Utc::now());
        run_ordered(&server, "a", &notes, Some(&order)).await;
        assert!(!server.file(FILE).unwrap().contains("syncedRev"));
        let other = run_ordered(&server, "b", &[], None).await;
        assert_eq!(other.order.unwrap().ids, order.ids);
    }

    #[tokio::test]
    async fn a_move_here_and_an_edit_there_keep_both_without_a_conflict_copy() {
        let server = FakeWebDav::start();
        let first = run(&server, "a", &[note("one", "a"), note("two", "a")]).await.unwrap();
        // Device b edits "one".
        let edited: Vec<Note> = first
            .notes
            .iter()
            .map(|n| if n.text == "one" { n.edited("b", crate::model::Edit { text: Some("one, edited".into()), ..Default::default() }, Utc::now()) } else { n.clone() })
            .collect();
        run(&server, "b", &edited).await.unwrap();
        // Device a only moves "one" to the end.
        let ids: Vec<String> = first.notes.iter().map(|n| n.id.clone()).rev().collect();
        let order = NoteOrder::new(ids.clone(), "a", Utc::now());
        let outcome = run_ordered(&server, "a", &first.notes, Some(&order)).await;
        assert_eq!(outcome.conflicts, 0);
        assert_eq!(outcome.notes.len(), 2);
        assert!(outcome.notes.iter().any(|n| n.text == "one, edited"));
        assert_eq!(outcome.order.unwrap().ids, ids);
        // Only the order changed here, and it still reached the server.
        assert_eq!(run_ordered(&server, "c", &[], None).await.order.unwrap().ids, ids);
    }

    #[test]
    fn a_file_without_an_order_still_reads_and_a_damaged_order_is_refused() {
        let (notes, order) = decode_file(r#"{"version":1,"notes":[]}"#).unwrap();
        assert!(notes.is_empty() && order.is_none());
        let error = decode_file(r#"{"version":1,"notes":[],"order":{"ids":5}}"#).unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::Corrupt);
    }
}

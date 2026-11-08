//! A check against a real WebDAV server, run by hand before trusting a new
//! server: debug builds only, `TACKNOTE_SELFTEST=webdav` with the account
//! saved in a test folder. Everything it writes goes into a folder of its own,
//! `tacknote/check-<id>/`, deleted at the end; the notes file the app syncs is
//! only read. The folder sits inside the app's folder because Jianguoyun
//! refuses to delete a folder at the top of the account.
//!
//! It confirms what sync relies on: every write gets a new ETag, a missing
//! folder is reported so it can be created, and two devices syncing through
//! the server keep each other's text, also when one writes in between the
//! other's read and write. Whether the server refuses an upload with
//! `If-None-Match: *` or a stale `If-Match` is reported too: sync is safe
//! either way, but a server that refuses them never even briefly holds an
//! upload that was written over.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use reqwest::header::{HeaderName, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Client, Method, StatusCode, Url};

use crate::model::{new_id, Note};
use crate::order::NoteOrder;
use crate::store::{Change, NoteStore};
use crate::sync_engine::{decode_file, sync_with_order, SyncOutcome};
use crate::webdav::{RemoteFile, RemoteStore, SyncError, WebDavConfig, WebDavStore, WriteError};

#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failed: bool,
}

impl Report {
    fn check(&mut self, ok: bool, name: impl AsRef<str>) -> bool {
        self.lines.push(format!("{} {}", if ok { "PASS" } else { "FAIL" }, name.as_ref()));
        self.failed |= !ok;
        ok
    }

    fn info(&mut self, text: impl AsRef<str>) {
        self.lines.push(format!("INFO {}", text.as_ref()));
    }
}

struct Reply {
    status: StatusCode,
    etag: Option<String>,
    body: String,
}

/// Requests sent as they are, to see what the server answers.
struct Raw<'a> {
    client: &'a Client,
    config: &'a WebDavConfig,
}

impl Raw<'_> {
    async fn send(&self, method: &str, url: &Url, headers: &[(HeaderName, &str)], body: Option<&str>) -> Result<Reply, String> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?;
        let mut request = self
            .client
            .request(method, url.clone())
            .basic_auth(&self.config.username, Some(&self.config.password));
        for (name, value) in headers {
            request = request.header(name, *value);
        }
        if let Some(body) = body {
            request = request.body(body.to_string());
        }
        let response = request.send().await.map_err(|error| error.without_url().to_string())?;
        let status = response.status();
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.text().await.unwrap_or_default();
        Ok(Reply { status, etag, body })
    }
}

/// Runs every check; the report says PASS or FAIL per line, and never shows
/// the password.
pub async fn run(config: WebDavConfig, client: Client) -> (String, bool) {
    let mut report = Report::default();
    let trimmed = config.url.trim();
    let base = match Url::parse(&if trimmed.ends_with('/') { trimmed.to_string() } else { format!("{trimmed}/") }) {
        Ok(base) => base,
        Err(_) => {
            report.check(false, "the saved address is a valid URL");
            return finish(report);
        }
    };
    let id = &new_id()[..8];
    let app_folder = base.join("tacknote/").expect("a plain folder name joins");
    let root = app_folder.join(&format!("check-{id}/")).expect("a plain folder name joins");
    let scratch = std::env::temp_dir().join(format!("tacknote-check-{id}"));
    let raw = Raw { client: &client, config: &config };

    // The app makes this folder on its first sync anyway.
    match raw.send("MKCOL", &app_folder, &[], None).await {
        Ok(reply) if reply.status.is_success() || reply.status == StatusCode::METHOD_NOT_ALLOWED => {}
        Ok(reply) => report.info(format!("making the app's folder answered HTTP {}", reply.status.as_u16())),
        Err(error) => report.info(format!("making the app's folder failed ({error})")),
    }
    if let Err(error) = protocol(&mut report, &raw, &root).await {
        report.check(false, format!("the server answers ({error})"));
    }
    if !report.failed {
        let account = WebDavConfig { url: root.to_string(), ..config.clone() };
        if let Err(error) = two_devices(&mut report, &raw, &root, &account, &client, &scratch).await {
            report.check(false, format!("two devices sync through the server ({error})"));
        }
    }
    app_file(&mut report, &raw, &base).await;

    // Cleans up after itself, also when a check failed.
    let left = format!("tacknote/check-{id}/");
    match raw.send("DELETE", &root, &[], None).await {
        Ok(reply) => {
            let deleted = reply.status.is_success();
            report.check(deleted, format!("the check folder is deleted again (HTTP {})", reply.status.as_u16()));
            if !deleted {
                report.info(format!("delete {left} by hand"));
            }
        }
        Err(error) => {
            report.check(false, format!("the check folder is deleted again ({error})"));
            report.info(format!("delete {left} by hand"));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    finish(report)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn finish(report: Report) -> (String, bool) {
    let passed = !report.failed && !report.lines.is_empty();
    (report.lines.join("\n"), passed)
}

/// What the server answers to the requests sync sends.
async fn protocol(report: &mut Report, raw: &Raw<'_>, root: &Url) -> Result<(), String> {
    let created = raw.send("MKCOL", root, &[], None).await?;
    if !report.check(created.status == StatusCode::CREATED, format!("a new folder can be made (HTTP {})", created.status.as_u16())) {
        return Ok(());
    }
    let file = root.join("probe.json").expect("a plain file name joins");
    let missing = raw.send("GET", &file, &[], None).await?;
    report.check(missing.status == StatusCode::NOT_FOUND, format!("a missing file reads as 404 (HTTP {})", missing.status.as_u16()));

    let first = raw.send("PUT", &file, &[(IF_NONE_MATCH, "*")], Some("one")).await?;
    report.check(first.status.is_success(), format!("the first upload with If-None-Match: * is accepted (HTTP {})", first.status.as_u16()));
    report.info(format!("an upload answers with an ETag: {}", if first.etag.is_some() { "yes" } else { "no" }));
    let read = raw.send("GET", &file, &[], None).await?;
    report.check(read.body == "one", "the uploaded text reads back");
    let Some(etag) = read.etag.clone() else {
        report.check(false, "a read gives an ETag");
        return Ok(());
    };
    report.check(true, "a read gives an ETag");
    report.info(format!("ETag looks like {etag} ({})", if etag.starts_with("W/") { "weak" } else { "strong" }));

    let again = raw.send("PUT", &file, &[(IF_NONE_MATCH, "*")], Some("two")).await?;
    let kept = raw.send("GET", &file, &[], None).await?;
    let refuses_create = again.status == StatusCode::PRECONDITION_FAILED && kept.body == "one";
    report.info(format!("If-None-Match: * on an existing file is refused: {} (HTTP {})", yes_no(refuses_create), again.status.as_u16()));
    // Back to the text the next steps expect, whatever the server did.
    let reset = raw.send("PUT", &file, &[], Some("one")).await?;
    let Some(etag) = raw.send("GET", &file, &[], None).await?.etag.filter(|_| reset.status.is_success()) else {
        report.check(false, "a plain upload is accepted and gives an ETag");
        return Ok(());
    };

    // Same length and straight away, so an ETag made from the size and a
    // time in whole seconds would not change.
    let update = raw.send("PUT", &file, &[(IF_MATCH, etag.as_str())], Some("uno")).await?;
    report.check(update.status.is_success(), format!("an upload with the current ETag in If-Match is accepted (HTTP {})", update.status.as_u16()));
    let updated = raw.send("GET", &file, &[], None).await?;
    report.check(updated.body == "uno", "the new text reads back");
    report.check(updated.etag.as_ref().is_some_and(|new| new != &etag), "a write of the same length right away gets a new ETag");

    let stale = raw.send("PUT", &file, &[(IF_MATCH, etag.as_str())], Some("dos")).await?;
    let unchanged = raw.send("GET", &file, &[], None).await?;
    let refuses_stale = stale.status == StatusCode::PRECONDITION_FAILED && unchanged.body == "uno";
    report.info(format!("an upload with an old ETag is refused: {} (HTTP {})", yes_no(refuses_stale), stale.status.as_u16()));

    let nowhere = root.join("missing/probe.json").expect("a plain path joins");
    let unread = raw.send("GET", &nowhere, &[], None).await?;
    report.check(
        matches!(unread.status, StatusCode::NOT_FOUND | StatusCode::CONFLICT),
        format!("a file in a missing folder reads as 404 or 409 (HTTP {})", unread.status.as_u16()),
    );
    let lost = raw.send("PUT", &nowhere, &[(IF_NONE_MATCH, "*")], Some("x")).await?;
    report.check(
        matches!(lost.status, StatusCode::NOT_FOUND | StatusCode::CONFLICT),
        format!("an upload into a missing folder is refused with 404 or 409 (HTTP {})", lost.status.as_u16()),
    );
    // Two devices may both make the folder on their first sync.
    let existing = raw.send("MKCOL", root, &[], None).await?;
    report.check(
        existing.status.is_success() || existing.status == StatusCode::METHOD_NOT_ALLOWED,
        format!("making a folder that exists is not an error (HTTP {})", existing.status.as_u16()),
    );
    Ok(())
}

type Hook = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The server, with another device's sync slipped in just before the first
/// upload, as happens when two devices sync at the same moment.
struct Interleaved {
    inner: WebDavStore,
    hook: Mutex<Option<Hook>>,
    refused: AtomicUsize,
}

impl RemoteStore for Interleaved {
    async fn read(&self) -> Result<Option<RemoteFile>, SyncError> {
        self.inner.read().await
    }

    async fn write(&self, body: String, previous: Option<RemoteFile>) -> Result<(), WriteError> {
        let hook = { self.hook.lock().unwrap().take() };
        if let Some(hook) = hook {
            hook.await;
        }
        let result = self.inner.write(body, previous).await;
        if matches!(result, Err(WriteError::Changed)) {
            self.refused.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

fn apply(store: &mut NoteStore, outcome: SyncOutcome, revs: &HashMap<String, String>, order: Option<&NoteOrder>) -> Result<usize, String> {
    store.apply_sync(outcome.notes, revs).map_err(|error| error.to_string())?;
    store.apply_order(outcome.order, order.map(|order| order.rev.as_str())).map_err(|error| error.to_string())?;
    Ok(outcome.conflicts)
}

/// One sync, as the app runs it. Returns how many conflict copies it made.
async fn sync_device(store: &mut NoteStore, remote: &impl RemoteStore) -> Result<usize, String> {
    let (local, revs, order) = (store.all(), store.revs(), store.order().cloned());
    let outcome = sync_with_order(remote, &local, order.as_ref(), store.device_id())
        .await
        .map_err(|error| error.to_string())?;
    apply(store, outcome, &revs, order.as_ref())
}

fn texts(store: &NoteStore) -> Vec<String> {
    let mut texts: Vec<String> = store.visible().into_iter().map(|note| note.text).collect();
    texts.sort();
    texts
}

fn open(dir: &Path, name: &str) -> Result<NoteStore, String> {
    NoteStore::open(dir.join(name)).map_err(|error| error.to_string())
}

/// Syncs `b`, with a whole sync of `a` slipped in between b's read and b's
/// upload. Says whether the server refused b's upload because of it.
async fn sync_racing(a: &mut NoteStore, b: &mut NoteStore, remote: &impl Fn() -> WebDavStore) -> Result<bool, String> {
    let (a_local, a_revs, a_order) = (a.all(), a.revs(), a.order().cloned());
    let a_device = a.device_id().to_string();
    let a_result: Arc<Mutex<Option<Result<SyncOutcome, SyncError>>>> = Arc::default();
    let hook: Hook = {
        let (a_remote, a_result, a_order) = (remote(), a_result.clone(), a_order.clone());
        Box::pin(async move {
            let outcome = sync_with_order(&a_remote, &a_local, a_order.as_ref(), &a_device).await;
            *a_result.lock().unwrap() = Some(outcome);
        })
    };
    let racing = Interleaved { inner: remote(), hook: Mutex::new(Some(hook)), refused: AtomicUsize::new(0) };
    sync_device(b, &racing).await?;
    let a_outcome = a_result.lock().unwrap().take().ok_or("the other device did not sync")?;
    apply(a, a_outcome.map_err(|error| error.to_string())?, &a_revs, a_order.as_ref())?;
    Ok(racing.refused.load(Ordering::SeqCst) > 0)
}

/// A, B, then A again: enough for anything either device has to reach the other.
async fn settle(a: &mut NoteStore, b: &mut NoteStore, remote: &impl Fn() -> WebDavStore) -> Result<(), String> {
    sync_device(a, &remote()).await?;
    sync_device(b, &remote()).await?;
    sync_device(a, &remote()).await?;
    Ok(())
}

/// Two devices, each with its own notes on this computer, syncing through
/// the check folder with the app's own sync code.
async fn two_devices(report: &mut Report, raw: &Raw<'_>, root: &Url, account: &WebDavConfig, client: &Client, scratch: &PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(scratch).map_err(|error| error.to_string())?;
    let remote = || WebDavStore::new(account.clone(), client.clone());
    let mut a = open(scratch, "a.json")?;
    let mut b = open(scratch, "b.json")?;
    let failed = |error: std::io::Error| error.to_string();
    let edit = |text: &str| Change { text: Some(text.into()), ..Default::default() };

    let note = a.create("来自 A", 0).map_err(failed)?;
    sync_device(&mut a, &remote()).await?;
    let file = root.join("tacknote/notes.json").expect("a plain path joins");
    let uploaded = raw.send("GET", &file, &[], None).await?;
    report.check(
        uploaded.status == StatusCode::OK && decode_file(&uploaded.body).is_ok_and(|(notes, _)| notes.len() == 1),
        "the first sync makes its folder and uploads the note",
    );
    sync_device(&mut b, &remote()).await?;
    report.check(texts(&b) == ["来自 A"], "the other device gets it");

    a.update(&note.id, edit("A 改的")).map_err(failed)?;
    b.update(&note.id, edit("B 改的")).map_err(failed)?;
    sync_device(&mut a, &remote()).await?;
    let conflicts = sync_device(&mut b, &remote()).await?;
    sync_device(&mut a, &remote()).await?;
    let both = vec!["A 改的".to_string(), "B 改的".to_string()];
    report.check(
        conflicts == 1 && texts(&b) == both && texts(&a) == both && b.visible().iter().filter(|n| n.conflict_copy).count() == 1,
        "the same note changed on both devices keeps both texts, one as a conflict copy",
    );

    // The races: A syncs in between B's read and B's upload. A server that
    // ignores If-Match then lets B write over what A just uploaded.
    let has = |store: &NoteStore, wanted: &[&str]| {
        let texts = texts(store);
        wanted.iter().all(|text| texts.contains(&text.to_string()))
    };
    let same = |a: &NoteStore, b: &NoteStore| texts(a) == texts(b);

    a.create("A 的新便签", 0).map_err(failed)?;
    b.create("B 的新便签", 0).map_err(failed)?;
    let refused = sync_racing(&mut a, &mut b, &remote).await?;
    report.info(format!("an upload racing another device is refused: {}", yes_no(refused)));
    settle(&mut a, &mut b, &remote).await?;
    report.check(has(&a, &["A 的新便签", "B 的新便签"]) && same(&a, &b), "new notes from two devices syncing at once both survive");

    let main = a.visible().into_iter().find(|n| n.text == "A 改的").ok_or("the note is there")?;
    a.update(&main.id, edit("A 同时改的")).map_err(failed)?;
    b.update(&main.id, edit("B 同时改的")).map_err(failed)?;
    sync_racing(&mut a, &mut b, &remote).await?;
    settle(&mut a, &mut b, &remote).await?;
    report.check(has(&a, &["A 同时改的", "B 同时改的"]) && same(&a, &b), "the same note changed on two devices syncing at once keeps both texts");

    let other = a.visible().into_iter().find(|n| n.text == "B 的新便签").ok_or("the note is there")?;
    let before = texts(&a).len();
    let replaced = a.get(&main.id).ok_or("the note is there")?.text.clone();
    a.update(&main.id, edit("A 最后改的")).map_err(failed)?;
    b.update(&other.id, edit("B 最后改的")).map_err(failed)?;
    sync_racing(&mut a, &mut b, &remote).await?;
    settle(&mut a, &mut b, &remote).await?;
    report.check(
        has(&a, &["A 最后改的", "B 最后改的"]) && !has(&a, &[&replaced]) && texts(&a).len() == before && same(&a, &b),
        "an edit written over by a device syncing at once comes back, without a conflict copy",
    );

    a.delete(&note.id).map_err(failed)?;
    sync_device(&mut a, &remote()).await?;
    sync_device(&mut b, &remote()).await?;
    let gone = |store: &NoteStore| store.get(&note.id).is_none_or(|n: &Note| n.deleted);
    report.check(gone(&b) && same(&a, &b), "a deletion reaches the other device and both show the same notes");
    Ok(())
}

/// The file the app itself syncs, read only.
async fn app_file(report: &mut Report, raw: &Raw<'_>, base: &Url) {
    let file = base.join("tacknote/notes.json").expect("a plain path joins");
    match raw.send("GET", &file, &[], None).await {
        Ok(reply) if reply.status == StatusCode::OK => match decode_file(&reply.body) {
            Ok((notes, _)) => report.info(format!("the app's own notes file is there with {} notes", notes.len())),
            Err(error) => {
                report.check(false, format!("the app's own notes file can be read ({error})"));
            }
        },
        Ok(reply) => report.info(format!("the app's own notes file is not there yet (HTTP {})", reply.status.as_u16())),
        Err(error) => report.info(format!("the app's own notes file could not be read ({error})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::FakeWebDav;
    use crate::webdav::http_client;

    fn account(server: &FakeWebDav) -> WebDavConfig {
        WebDavConfig { url: server.url(), username: "me".into(), password: "secret".into() }
    }

    #[tokio::test]
    async fn a_server_that_does_everything_sync_needs_passes() {
        let server = FakeWebDav::start();
        let (report, passed) = run(account(&server), http_client()).await;
        assert!(passed, "{report}");
        assert!(!report.contains("secret"));
    }

    #[tokio::test]
    async fn a_server_that_ignores_if_match_is_reported_and_loses_nothing() {
        let server = FakeWebDav::start();
        server.set_honor_preconditions(false);
        let (report, passed) = run(account(&server), http_client()).await;
        assert!(passed, "{report}");
        assert!(report.contains("INFO If-None-Match: * on an existing file is refused: no"), "{report}");
        assert!(report.contains("INFO an upload racing another device is refused: no"), "{report}");
    }

    #[tokio::test]
    async fn a_server_that_honours_if_match_is_reported() {
        let server = FakeWebDav::start();
        let (report, _) = run(account(&server), http_client()).await;
        assert!(report.contains("INFO an upload with an old ETag is refused: yes"), "{report}");
        assert!(report.contains("INFO an upload racing another device is refused: yes"), "{report}");
    }

    #[tokio::test]
    async fn it_cleans_up_and_only_reads_the_apps_file() {
        let server = FakeWebDav::start();
        server.add_folder("/tacknote/");
        server.set_file("/tacknote/notes.json", r#"{"version":1,"notes":[]}"#);
        let (report, passed) = run(account(&server), http_client()).await;
        assert!(passed, "{report}");
        assert!(report.contains("with 0 notes"), "{report}");
        assert!(server.paths().iter().all(|path| !path.contains("/check-")), "{:?}", server.paths());
        assert_eq!(server.file("/tacknote/notes.json").unwrap(), r#"{"version":1,"notes":[]}"#);
        let writes: Vec<String> = server
            .log()
            .into_iter()
            .filter(|line| !line.starts_with("GET") && line != "MKCOL /tacknote/")
            .collect();
        assert!(writes.iter().all(|line| line.contains("/tacknote/check-")), "{writes:?}");
    }

    #[tokio::test]
    async fn a_server_that_answers_like_jianguoyun_passes_and_is_cleaned_up() {
        let server = FakeWebDav::start();
        server.answer_like_jianguoyun();
        let (report, passed) = run(account(&server), http_client()).await;
        assert!(passed, "{report}");
        assert!(report.contains("INFO If-None-Match: * on an existing file is refused: no"), "{report}");
        assert!(report.contains("INFO an upload with an old ETag is refused: yes"), "{report}");
        assert!(server.paths().iter().all(|path| !path.contains("/check-")), "{:?}", server.paths());
    }
}

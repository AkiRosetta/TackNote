//! Automatic sync: a few seconds after a change, then every minute. Only one
//! sync runs at a time; a request during a sync runs once more afterwards.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::settings::Settings;
use crate::store::NoteStore;
use crate::sync_engine::sync_with_order;
use crate::webdav::{RemoteStore, SyncErrorKind, WebDavConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncStatus {
    NotConfigured,
    Syncing,
    Ok,
    Error,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub status: SyncStatus,
    pub error: Option<SyncErrorKind>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Flags {
    running: bool,
    run_again: bool,
    pending_conflicts: usize,
}

type Listener = Box<dyn Fn(&SyncState) + Send + Sync>;

pub struct SyncController<R, F>
where
    R: RemoteStore + 'static,
    F: Fn(WebDavConfig) -> R + Send + Sync + 'static,
{
    notes: Arc<Mutex<NoteStore>>,
    settings: Settings,
    make_remote: F,
    state: Mutex<SyncState>,
    flags: Mutex<Flags>,
    change_generation: AtomicU64,
    debounce: Duration,
    interval: Duration,
    on_state: Listener,
    on_notes_applied: Box<dyn Fn() + Send + Sync>,
}

impl<R, F> SyncController<R, F>
where
    R: RemoteStore + 'static,
    F: Fn(WebDavConfig) -> R + Send + Sync + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        notes: Arc<Mutex<NoteStore>>,
        settings: Settings,
        make_remote: F,
        debounce: Duration,
        interval: Duration,
        on_state: impl Fn(&SyncState) + Send + Sync + 'static,
        on_notes_applied: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(SyncController {
            notes,
            settings,
            make_remote,
            state: Mutex::new(SyncState { status: SyncStatus::NotConfigured, error: None, last_synced_at: None }),
            flags: Mutex::new(Flags::default()),
            change_generation: AtomicU64::new(0),
            debounce,
            interval,
            on_state: Box::new(on_state),
            on_notes_applied: Box::new(on_notes_applied),
        })
    }

    pub fn state(&self) -> SyncState {
        self.state.lock().unwrap().clone()
    }

    /// Conflicts since the last call, so each one is announced exactly once.
    pub fn take_conflicts(&self) -> usize {
        std::mem::take(&mut self.flags.lock().unwrap().pending_conflicts)
    }

    /// Syncs now, then every `interval`.
    pub fn start(self: &Arc<Self>) {
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                controller.sync_now().await;
                tokio::time::sleep(controller.interval).await;
            }
        });
    }

    /// Call after every local change; changes in quick succession sync once.
    pub fn notes_changed(self: &Arc<Self>) {
        let generation = self.change_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(controller.debounce).await;
            if controller.change_generation.load(Ordering::SeqCst) == generation {
                controller.sync_now().await;
            }
        });
    }

    pub async fn sync_now(&self) {
        {
            let mut flags = self.flags.lock().unwrap();
            if flags.running {
                flags.run_again = true;
                return;
            }
            flags.running = true;
        }
        loop {
            self.flags.lock().unwrap().run_again = false;
            self.sync_once().await;
            let mut flags = self.flags.lock().unwrap();
            if !flags.run_again {
                flags.running = false;
                return;
            }
        }
    }

    async fn sync_once(&self) {
        let config = match self.settings.load_webdav() {
            Ok(config) => config,
            Err(_) => return self.set(SyncStatus::Error, Some(SyncErrorKind::CredentialsUnavailable), false),
        };
        if !config.is_complete() {
            return self.set(SyncStatus::NotConfigured, None, false);
        }
        self.set(SyncStatus::Syncing, None, false);
        let (local, revs_at_start, device_id, order) = {
            let notes = self.notes.lock().unwrap();
            (notes.all(), notes.revs(), notes.device_id().to_string(), notes.order().cloned())
        };
        let remote = (self.make_remote)(config);
        match sync_with_order(&remote, &local, order.as_ref(), &device_id).await {
            Ok(outcome) => {
                let order_rev = order.as_ref().map(|order| order.rev.as_str());
                let saved = {
                    let mut notes = self.notes.lock().unwrap();
                    notes.apply_sync(outcome.notes, &revs_at_start).and_then(|()| notes.apply_order(outcome.order, order_rev))
                };
                if saved.is_err() {
                    return self.set(SyncStatus::Error, Some(SyncErrorKind::LocalStorage), false);
                }
                self.flags.lock().unwrap().pending_conflicts += outcome.conflicts;
                (self.on_notes_applied)();
                self.set(SyncStatus::Ok, None, true);
            }
            Err(error) => self.set(SyncStatus::Error, Some(error.kind), false),
        }
    }

    fn set(&self, status: SyncStatus, error: Option<SyncErrorKind>, synced: bool) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.status = status;
            state.error = error;
            if synced {
                state.last_synced_at = Some(Utc::now());
            }
            state.clone()
        };
        (self.on_state)(&snapshot);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::model::{Edit, Note};
    use crate::settings::tests::{temp_settings_path, FakeSecret};
    use crate::sync_engine::{decode, encode};
    use crate::webdav::{http_client, RemoteFile, SyncError, WebDavStore, WriteError};

    /// A server held in memory. `gate`, when set, holds every read until opened.
    #[derive(Clone, Default)]
    struct MemoryRemote {
        notes: Arc<Mutex<HashMap<String, Note>>>,
        reads: Arc<AtomicUsize>,
        gate: Option<Arc<tokio::sync::Semaphore>>,
    }

    impl RemoteStore for MemoryRemote {
        async fn read(&self) -> Result<Option<RemoteFile>, SyncError> {
            if let Some(gate) = &self.gate {
                let _permit = gate.acquire().await.unwrap();
            }
            self.reads.fetch_add(1, Ordering::SeqCst);
            let notes: Vec<Note> = self.notes.lock().unwrap().values().cloned().collect();
            Ok(Some(RemoteFile { body: encode(&notes), etag: None }))
        }

        async fn write(&self, body: String, _previous: Option<RemoteFile>) -> Result<(), WriteError> {
            let notes = decode(&body).map_err(WriteError::Failed)?;
            *self.notes.lock().unwrap() = notes.into_iter().map(|n| (n.id.clone(), n)).collect();
            Ok(())
        }
    }

    struct Setup {
        notes: Arc<Mutex<NoteStore>>,
        dir: std::path::PathBuf,
    }

    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn setup(account: Option<&str>, secret: FakeSecret) -> (Setup, Settings) {
        let path = temp_settings_path();
        let dir = path.parent().unwrap().to_path_buf();
        let notes = Arc::new(Mutex::new(NoteStore::open(dir.join("notes.json")).unwrap()));
        let settings = Settings::new(&path, Box::new(secret.clone()));
        if let Some(url) = account {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, format!(r#"{{"url":"{url}","username":"me"}}"#)).unwrap();
            *secret.value.lock().unwrap() = Some("secret".into());
        }
        (Setup { notes, dir }, settings)
    }

    fn controller<R, F>(setup: &Setup, settings: Settings, make_remote: F) -> Arc<SyncController<R, F>>
    where
        R: RemoteStore + 'static,
        F: Fn(WebDavConfig) -> R + Send + Sync + 'static,
    {
        SyncController::new(
            setup.notes.clone(),
            settings,
            make_remote,
            Duration::from_millis(50),
            Duration::from_secs(3600),
            |_| {},
            || {},
        )
    }

    #[tokio::test]
    async fn an_unreadable_password_is_not_reported_as_a_wrong_password() {
        let (setup, _) = setup(Some("https://example.test/dav/"), FakeSecret::default());
        // The same account file, but a keychain that refuses to answer.
        let broken = FakeSecret { broken: true, ..FakeSecret::default() };
        let settings = Settings::new(setup.dir.join("settings.json"), Box::new(broken));
        let c = controller(&setup, settings, |_| MemoryRemote::default());
        c.sync_now().await;
        assert_eq!(c.state().status, SyncStatus::Error);
        assert_eq!(c.state().error, Some(SyncErrorKind::CredentialsUnavailable));
    }

    #[tokio::test]
    async fn without_an_account_the_keychain_is_never_touched() {
        let secret = FakeSecret { broken: true, ..FakeSecret::default() };
        let (setup, settings) = setup(None, secret.clone());
        let c = controller(&setup, settings, |_| MemoryRemote::default());
        c.sync_now().await;
        assert_eq!(c.state().status, SyncStatus::NotConfigured);
        assert_eq!(c.state().error, None);
        assert_eq!(secret.reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_malformed_address_reports_an_error_instead_of_hanging() {
        let (setup, settings) = setup(Some("http://[bad"), FakeSecret::default());
        let client = http_client();
        let c = controller(&setup, settings, move |config| WebDavStore::new(config, client.clone()));
        c.sync_now().await;
        assert_eq!(c.state().status, SyncStatus::Error);
        assert_eq!(c.state().error, Some(SyncErrorKind::BadUrl));
    }

    #[tokio::test]
    async fn a_successful_sync_saves_notes_and_tells_the_page() {
        let (setup, settings) = setup(Some("https://example.test/dav/"), FakeSecret::default());
        let remote = MemoryRemote::default();
        let applied = Arc::new(AtomicUsize::new(0));
        let counter = applied.clone();
        let shared = remote.clone();
        let c = SyncController::new(
            setup.notes.clone(),
            settings,
            move |_| shared.clone(),
            Duration::from_millis(50),
            Duration::from_secs(3600),
            |_| {},
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        setup.notes.lock().unwrap().create("hello", 0).unwrap();
        c.sync_now().await;
        assert_eq!(c.state().status, SyncStatus::Ok);
        assert!(c.state().last_synced_at.is_some());
        assert_eq!(applied.load(Ordering::SeqCst), 1);
        assert_eq!(remote.notes.lock().unwrap().len(), 1);
        assert!(!setup.notes.lock().unwrap().all()[0].has_unsynced_changes());
    }

    #[tokio::test]
    async fn each_conflict_is_announced_once_even_two_in_a_row() {
        let (setup, settings) = setup(Some("https://example.test/dav/"), FakeSecret::default());
        let remote = MemoryRemote::default();
        let shared = remote.clone();
        let c = controller(&setup, settings, move |_| shared.clone());
        let id = setup.notes.lock().unwrap().create("base", 0).unwrap().id;
        c.sync_now().await;

        for round in ["1", "2"] {
            let base = setup.notes.lock().unwrap().get(&id).unwrap().clone();
            let theirs = base.edited("other", Edit { text: Some(format!("theirs {round}")), ..Edit::default() }, Utc::now());
            remote.notes.lock().unwrap().insert(id.clone(), theirs);
            setup
                .notes
                .lock()
                .unwrap()
                .update(&id, crate::store::Change { text: Some(format!("mine {round}")), ..Default::default() })
                .unwrap();
            c.sync_now().await;
            assert_eq!(c.take_conflicts(), 1, "round {round}");
            assert_eq!(c.take_conflicts(), 0, "round {round}");
        }
    }

    #[tokio::test]
    async fn a_request_during_a_sync_runs_exactly_once_more_afterwards() {
        let (setup, settings) = setup(Some("https://example.test/dav/"), FakeSecret::default());
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let remote = MemoryRemote { gate: Some(gate.clone()), ..MemoryRemote::default() };
        let shared = remote.clone();
        let c = controller(&setup, settings, move |_| shared.clone());

        let first = tokio::spawn({
            let c = c.clone();
            async move { c.sync_now().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.sync_now().await; // returns at once: a sync is running
        c.sync_now().await;
        assert_eq!(remote.reads.load(Ordering::SeqCst), 0);

        gate.add_permits(10);
        first.await.unwrap();
        assert_eq!(remote.reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn changes_in_quick_succession_sync_once() {
        let (setup, settings) = setup(Some("https://example.test/dav/"), FakeSecret::default());
        let remote = MemoryRemote::default();
        let shared = remote.clone();
        let c = controller(&setup, settings, move |_| shared.clone());
        for _ in 0..3 {
            c.notes_changed();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(remote.reads.load(Ordering::SeqCst), 1);
    }
}

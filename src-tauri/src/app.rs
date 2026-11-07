//! The bridge to the page: the commands it calls and the events it hears.
//!
//! Events: `notes-changed` after any change, with `source` "local" or "sync",
//! so every window showing a note can refresh it; `sync-state` whenever the
//! sync status changes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::checklist::{parse_lines, NoteLine};
use crate::model::Note;
use crate::settings::{NoSecureStorage, SecretStore, Settings, SettingsError};
use crate::store::{Change, NoteStore};
use crate::sync_controller::{SyncController, SyncState};
use crate::webdav::{http_client, WebDavConfig, WebDavStore};

/// The keychain entry is named after the app; Android has no keychain yet.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const IDENTIFIER: &str = "io.github.akirosetta.tacknote";
const SYNC_DELAY: Duration = Duration::from_secs(3);
const SYNC_INTERVAL: Duration = Duration::from_secs(60);

type MakeRemote = Box<dyn Fn(WebDavConfig) -> WebDavStore + Send + Sync>;
type AutoSync = SyncController<WebDavStore, MakeRemote>;

struct Core {
    notes: Arc<Mutex<NoteStore>>,
    settings: Settings,
    sync: Arc<AutoSync>,
}

/// `Err` when the saved notes could not be opened; the page then says why
/// instead of starting empty and overwriting them.
pub struct AppState(Result<Core, String>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The saved notes could not be opened when the app started.
    Startup,
    /// A change could not be written to this device.
    Storage,
    /// The keychain refused the password; nothing was saved.
    Keychain,
    NotAllowed,
}

#[derive(Debug, Serialize)]
pub struct CommandError {
    kind: ErrorKind,
    detail: String,
}

fn error(kind: ErrorKind, detail: impl ToString) -> CommandError {
    CommandError { kind, detail: detail.to_string() }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteView {
    id: String,
    text: String,
    color_index: u32,
    pinned: bool,
    conflict_copy: bool,
    deleted: bool,
    updated_at: DateTime<Utc>,
    lines: Vec<NoteLine>,
}

impl From<&Note> for NoteView {
    fn from(note: &Note) -> NoteView {
        NoteView {
            id: note.id.clone(),
            text: note.text.clone(),
            color_index: note.color_index,
            pinned: note.pinned,
            conflict_copy: note.conflict_copy,
            deleted: note.deleted,
            updated_at: note.updated_at,
            lines: parse_lines(&note.text),
        }
    }
}

#[derive(Clone, Serialize)]
struct NotesChanged {
    source: &'static str,
    window: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    url: String,
    username: String,
    password: String,
    /// The account is saved but its password could not be read.
    password_unavailable: bool,
}

impl AppState {
    fn core(&self) -> Result<&Core, CommandError> {
        self.0.as_ref().map_err(|detail| error(ErrorKind::Startup, detail))
    }
}

impl Core {
    fn notes(&self) -> MutexGuard<'_, NoteStore> {
        self.notes.lock().unwrap()
    }

    /// Tells every window and schedules a sync.
    fn changed(&self, app: &AppHandle, window: &WebviewWindow) {
        self.sync.notes_changed();
        let _ = app.emit("notes-changed", NotesChanged { source: "local", window: Some(window.label().into()) });
    }
}

/// In debug builds `TACKNOTE_TEST_DIR` keeps notes, settings and the keychain
/// entry apart from the real ones, for self-tests and experiments.
fn test_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("TACKNOTE_TEST_DIR").map(PathBuf::from)
    } else {
        None
    }
}

/// Only with a test folder, so a self-test never touches the real notes.
fn selftest_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var_os("TACKNOTE_SELFTEST").is_some() && test_dir().is_some()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn keychain_service() -> String {
    if test_dir().is_some() { format!("{IDENTIFIER}.test") } else { IDENTIFIER.to_string() }
}

fn secret_store() -> Box<dyn SecretStore> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if let Ok(keychain) = crate::settings::Keychain::new(&keychain_service()) {
            return Box::new(keychain);
        }
    }
    // Saving then fails loudly; the password never goes to a plain file.
    Box::new(NoSecureStorage)
}

fn open_core(app: &AppHandle) -> Result<Core, String> {
    let dir = match test_dir() {
        Some(dir) => dir,
        None => app.path().app_data_dir().map_err(|error| error.to_string())?,
    };
    let notes = NoteStore::open(dir.join("notes.json")).map_err(|error| error.to_string())?;
    let notes = Arc::new(Mutex::new(notes));
    let client = http_client();
    let make_remote: MakeRemote = Box::new(move |config| WebDavStore::new(config, client.clone()));
    let on_state = {
        let app = app.clone();
        move |state: &SyncState| {
            let _ = app.emit("sync-state", state);
        }
    };
    let on_notes_applied = {
        let app = app.clone();
        move || {
            let _ = app.emit("notes-changed", NotesChanged { source: "sync", window: None });
        }
    };
    let settings_path = dir.join("settings.json");
    let sync = SyncController::new(
        notes.clone(),
        Settings::new(&settings_path, secret_store()),
        make_remote,
        SYNC_DELAY,
        SYNC_INTERVAL,
        on_state,
        on_notes_applied,
    );
    Ok(Core { notes, settings: Settings::new(settings_path, secret_store()), sync })
}

pub fn setup(app: &tauri::App) {
    let core = open_core(app.handle());
    if let Ok(core) = &core {
        core.sync.start();
    }
    app.manage(AppState(core));
}

// Note commands are synchronous, so they run in the order the page sent them.

#[tauri::command]
fn list_notes(state: State<AppState>) -> Result<Vec<NoteView>, CommandError> {
    Ok(state.core()?.notes().visible().iter().map(NoteView::from).collect())
}

/// Also returns a note deleted elsewhere, so an open editor keeps its text;
/// typing into it brings the note back.
#[tauri::command]
fn get_note(state: State<AppState>, id: String) -> Result<Option<NoteView>, CommandError> {
    Ok(state.core()?.notes().get(&id).map(NoteView::from))
}

#[tauri::command]
fn create_note(app: AppHandle, window: WebviewWindow, state: State<AppState>, color_index: Option<u32>) -> Result<NoteView, CommandError> {
    let core = state.core()?;
    let note = core.notes().create("", color_index.unwrap_or(0)).map_err(|e| error(ErrorKind::Storage, e))?;
    core.changed(&app, &window);
    Ok(NoteView::from(&note))
}

/// `base` is the text the editor last showed, so typing never overwrites a
/// change it has not seen; see `NoteStore::edit_text`.
#[tauri::command]
fn edit_text(app: AppHandle, window: WebviewWindow, state: State<AppState>, id: String, base: String, text: String) -> Result<bool, CommandError> {
    let core = state.core()?;
    let changed = core.notes().edit_text(&id, &base, text).map_err(|e| error(ErrorKind::Storage, e))?;
    if changed {
        core.changed(&app, &window);
    }
    Ok(changed)
}

#[tauri::command]
fn update_note(
    app: AppHandle,
    window: WebviewWindow,
    state: State<AppState>,
    id: String,
    color_index: Option<u32>,
    pinned: Option<bool>,
) -> Result<bool, CommandError> {
    let core = state.core()?;
    let changed = core
        .notes()
        .update(&id, Change { text: None, color_index, pinned })
        .map_err(|e| error(ErrorKind::Storage, e))?;
    if changed {
        core.changed(&app, &window);
    }
    Ok(changed)
}

#[tauri::command]
fn toggle_checkbox(app: AppHandle, window: WebviewWindow, state: State<AppState>, id: String, line: usize) -> Result<bool, CommandError> {
    let core = state.core()?;
    let changed = core.notes().toggle_checkbox(&id, line).map_err(|e| error(ErrorKind::Storage, e))?;
    if changed {
        core.changed(&app, &window);
    }
    Ok(changed)
}

#[tauri::command]
fn delete_note(app: AppHandle, window: WebviewWindow, state: State<AppState>, id: String) -> Result<(), CommandError> {
    let core = state.core()?;
    core.notes().delete(&id).map_err(|e| error(ErrorKind::Storage, e))?;
    core.changed(&app, &window);
    Ok(())
}

/// Called when an editor closes. Checked under the lock, so text that a sync
/// brought in at the last moment is never thrown away.
#[tauri::command]
fn discard_if_empty(app: AppHandle, window: WebviewWindow, state: State<AppState>, id: String) -> Result<bool, CommandError> {
    let core = state.core()?;
    {
        let mut notes = core.notes();
        match notes.get(&id) {
            Some(note) if !note.deleted && note.text.trim().is_empty() => {
                notes.delete(&id).map_err(|e| error(ErrorKind::Storage, e))?;
            }
            _ => return Ok(false),
        }
    }
    core.changed(&app, &window);
    Ok(true)
}

/// Off the main thread: the keychain may ask the user first.
#[tauri::command(async)]
fn get_settings(state: State<AppState>) -> Result<SettingsView, CommandError> {
    let core = state.core()?;
    let (config, password_unavailable) = match core.settings.load_webdav() {
        Ok(config) => (config, false),
        // Still opens the page, so the password can be entered again.
        Err(_) => (core.settings.account(), true),
    };
    Ok(SettingsView { url: config.url, username: config.username, password: config.password, password_unavailable })
}

#[tauri::command(async)]
fn save_settings(state: State<AppState>, url: String, username: String, password: String) -> Result<(), CommandError> {
    let core = state.core()?;
    core.settings.save_webdav(&WebDavConfig { url, username, password }).map_err(|failure| match failure {
        SettingsError::Keychain(detail) => error(ErrorKind::Keychain, detail),
        SettingsError::Io(detail) => error(ErrorKind::Storage, detail),
    })?;
    let sync = core.sync.clone();
    tauri::async_runtime::spawn(async move { sync.sync_now().await });
    Ok(())
}

#[tauri::command]
fn url_is_insecure(url: String) -> bool {
    WebDavConfig { url, username: String::new(), password: String::new() }.is_insecure()
}

#[tauri::command]
fn sync_state(state: State<AppState>) -> Result<SyncState, CommandError> {
    Ok(state.core()?.sync.state())
}

#[tauri::command]
fn take_conflicts(state: State<AppState>) -> Result<usize, CommandError> {
    Ok(state.core()?.sync.take_conflicts())
}

// Self-test hooks: they only work in a debug build started with both
// TACKNOTE_SELFTEST and TACKNOTE_TEST_DIR set, and refuse otherwise.

#[tauri::command]
fn selftest_enabled_cmd() -> bool {
    selftest_enabled()
}

/// Applies `text` to a note as if a sync had brought it from another device.
#[tauri::command]
fn selftest_remote_edit(app: AppHandle, state: State<AppState>, id: String, text: String) -> Result<(), CommandError> {
    if !selftest_enabled() {
        return Err(error(ErrorKind::NotAllowed, "self-test only"));
    }
    let core = state.core()?;
    {
        let mut notes = core.notes();
        let revs = notes.revs();
        let edit = crate::model::Edit { text: Some(text), ..Default::default() };
        let merged: Vec<Note> = notes
            .all()
            .into_iter()
            .map(|note| if note.id == id { note.edited("remote", edit.clone(), Utc::now()) } else { note })
            .collect();
        notes.apply_sync(merged, &revs).map_err(|e| error(ErrorKind::Storage, e))?;
    }
    let _ = app.emit("notes-changed", NotesChanged { source: "sync", window: None });
    Ok(())
}

#[tauri::command]
fn selftest_finish(app: AppHandle, report: String, passed: bool) {
    if !selftest_enabled() {
        return;
    }
    // A test password left behind would make the next build ask the user
    // for keychain access, because the keychain trusts only the build that
    // saved it.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Ok(entry) = keyring::Entry::new(&keychain_service(), "webdav") {
        let _ = entry.delete_credential();
    }
    println!("{report}\nSELFTEST {}", if passed { "PASSED" } else { "FAILED" });
    app.exit(if passed { 0 } else { 1 });
}

pub fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        list_notes,
        get_note,
        create_note,
        edit_text,
        update_note,
        toggle_checkbox,
        delete_note,
        discard_if_empty,
        get_settings,
        save_settings,
        url_is_insecure,
        sync_state,
        take_conflicts,
        selftest_enabled_cmd,
        selftest_remote_edit,
        selftest_finish,
    ]
}

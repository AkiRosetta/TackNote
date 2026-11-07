//! Android only: the bridge to Kotlin (`BridgePlugin.kt`), the home screen
//! widget that shows the chosen note, and the password in the Keystore.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Wry};

use crate::settings::SecretStore;
use crate::store::NoteStore;
use crate::widget::WidgetFeed;

struct Bridge(PluginHandle<Wry>);

/// Registers `BridgePlugin.kt`, so Rust can ask Android for things.
pub fn plugin() -> TauriPlugin<Wry> {
    Builder::new("tacknote-bridge")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.akirosetta.tacknote", "BridgePlugin")?;
            app.manage(Bridge(handle));
            Ok(())
        })
        .build()
}

/// Asks Kotlin to run `command` on its own thread, since Kotlin may need the
/// thread the caller is on, and nothing waits for the answer.
fn call(app: &AppHandle, command: &'static str) {
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(bridge) = app.try_state::<Bridge>() else {
            return;
        };
        if let Err(error) = bridge.0.run_mobile_plugin::<serde_json::Value>(command, ()) {
            eprintln!("Android did not run {command}: {error}");
        }
    });
}

/// The WebDAV password, kept by `Secrets.kt` encrypted with an Android
/// Keystore key. Blocks until Android answers, so it is only used off the
/// main thread: by the sync and by async commands.
pub struct KeystoreSecrets {
    app: AppHandle,
}

impl KeystoreSecrets {
    pub fn new(app: &AppHandle) -> KeystoreSecrets {
        KeystoreSecrets { app: app.clone() }
    }
}

#[derive(serde::Deserialize)]
struct SavedPassword {
    password: Option<String>,
}

impl SecretStore for KeystoreSecrets {
    fn get(&self) -> Result<Option<String>, String> {
        let bridge = self.app.try_state::<Bridge>().ok_or("the Android bridge is not ready")?;
        bridge
            .0
            .run_mobile_plugin::<SavedPassword>("getPassword", ())
            .map(|saved| saved.password)
            .map_err(|error| error.to_string())
    }

    fn set(&self, password: &str) -> Result<(), String> {
        let bridge = self.app.try_state::<Bridge>().ok_or("the Android bridge is not ready")?;
        bridge
            .0
            .run_mobile_plugin::<serde_json::Value>("setPassword", serde_json::json!({ "password": password }))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

pub struct Widget {
    app: AppHandle,
    feed: Mutex<WidgetFeed>,
    actions: PathBuf,
}

#[derive(serde::Deserialize)]
struct Pinned {
    pinned: bool,
}

impl Widget {
    /// `dir` is the app data folder, where `NoteWidget.kt` looks.
    pub fn new(app: &AppHandle, dir: &Path) -> Arc<Widget> {
        Arc::new(Widget {
            app: app.clone(),
            feed: Mutex::new(WidgetFeed::open(dir.join("widget.json"))),
            actions: dir.join("widget-actions"),
        })
    }

    pub fn chosen(&self) -> Option<String> {
        self.feed.lock().unwrap().chosen().map(str::to_string)
    }

    /// Asks the launcher to add a widget showing this note. Says whether it
    /// could ask; some launchers only let the user add widgets by hand.
    /// Blocks until Android answers, so only call it off the main thread.
    pub fn add(&self, notes: &Mutex<NoteStore>, id: &str) -> Result<bool, String> {
        let visible = notes.lock().unwrap().visible();
        {
            let mut feed = self.feed.lock().unwrap();
            feed.choose(id);
            feed.write(&visible).map_err(|error| error.to_string())?;
        }
        let bridge = self.app.try_state::<Bridge>().ok_or("the Android bridge is not ready")?;
        bridge
            .0
            .run_mobile_plugin::<Pinned>("addWidget", serde_json::json!({ "noteId": id }))
            .map(|answer| answer.pinned)
            .map_err(|error| error.to_string())
    }

    /// After any change, local or from a sync: rewrites what the widgets show
    /// and redraws them, so the home screen never keeps old text.
    pub fn notes_changed(&self, notes: &Mutex<NoteStore>) {
        let visible = notes.lock().unwrap().visible();
        match self.feed.lock().unwrap().write(&visible) {
            Ok(true) => call(&self.app, "refreshWidget"),
            Ok(false) => {}
            Err(error) => eprintln!("could not update the widgets: {error}"),
        }
    }

    /// Applies the to-dos ticked on widgets. Says whether a note changed.
    pub fn apply_ticks(&self, notes: &Mutex<NoteStore>) -> io::Result<bool> {
        if !self.actions.exists() {
            return Ok(false);
        }
        crate::widget::apply_toggles(&mut notes.lock().unwrap(), &self.actions)
    }
}

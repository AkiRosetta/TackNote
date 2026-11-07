//! Android only: the bridge to Kotlin (`BridgePlugin.kt`) and the home screen
//! widget that shows the chosen note.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Wry};

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

pub struct Widget {
    app: AppHandle,
    feed: Mutex<WidgetFeed>,
}

impl Widget {
    pub fn new(app: &AppHandle, path: PathBuf) -> Arc<Widget> {
        Arc::new(Widget { app: app.clone(), feed: Mutex::new(WidgetFeed::open(path)) })
    }

    pub fn chosen(&self) -> Option<String> {
        self.feed.lock().unwrap().chosen().map(str::to_string)
    }

    /// Shows the note on the widget, and offers to add the widget to the home
    /// screen if there is none yet.
    pub fn choose(&self, notes: &Mutex<NoteStore>, id: &str) -> io::Result<()> {
        let note = notes.lock().unwrap().get(id).cloned();
        self.feed.lock().unwrap().show(note.as_ref())?;
        call(&self.app, "showWidget");
        Ok(())
    }

    /// After any change, local or from a sync: redraws when the shown note
    /// changed, so the home screen never shows old text.
    pub fn notes_changed(&self, notes: &Mutex<NoteStore>) {
        let mut feed = self.feed.lock().unwrap();
        let note = feed.chosen().and_then(|id| notes.lock().unwrap().get(id).cloned());
        match feed.show(note.as_ref()) {
            Ok(true) => call(&self.app, "refreshWidget"),
            Ok(false) => {}
            Err(error) => eprintln!("could not update the widget: {error}"),
        }
    }
}

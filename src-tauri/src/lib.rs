//! TackNote: sticky notes on the desktop and an Android home screen widget,
//! synced through the user's own WebDAV space.

mod app;
pub mod checklist;
#[cfg(desktop)]
mod desktop;
pub mod merge;
pub mod model;
pub mod settings;
pub mod stickies;
pub mod store;
pub mod sync_controller;
pub mod sync_engine;
pub mod webdav;

#[cfg(test)]
mod test_server;

#[cfg(desktop)]
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(app::handlers())
        .setup(|app| {
            app::setup(app);
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let Some(stickies) = app::stickies(window.app_handle()) {
                // Closing the main window quits, and the sticky windows come
                // back next time; there is no tray icon to reopen it from yet.
                if window.label() == "main" && matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                    stickies.quitting();
                    window.app_handle().exit(0);
                }
                stickies.window_event(window, event);
            }
            #[cfg(mobile)]
            let _ = (window, event);
        })
        .build(tauri::generate_context!())
        .expect("error while starting TackNote")
        .run(|app, event| {
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                if let Some(stickies) = app::stickies(app) {
                    stickies.quitting();
                }
            }
            #[cfg(mobile)]
            let _ = (app, event);
        });
}

//! TackNote: sticky notes on the desktop and an Android home screen widget,
//! synced through the user's own WebDAV space.

pub mod checklist;
pub mod merge;
pub mod model;
pub mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running TackNote");
}

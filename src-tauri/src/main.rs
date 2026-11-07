// Release builds on Windows have no console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Desktop entry point. Android starts the app through `run`'s mobile entry.
fn main() {
    tacknote_lib::run()
}

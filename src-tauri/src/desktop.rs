//! Sticky note windows on the desktop: one small frameless window per note,
//! dragged by its top bar. Where each one sits is remembered on this device.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

use crate::stickies::{Layout, Sticky};

const LABEL_PREFIX: &str = "sticky-";
const SAVE_DELAY: Duration = Duration::from_millis(500);

pub fn label(note_id: &str) -> String {
    format!("{LABEL_PREFIX}{note_id}")
}

pub struct Stickies {
    app: AppHandle,
    layout: Mutex<Layout>,
    path: PathBuf,
    generation: AtomicU64,
    /// Set once the app is quitting: windows closing then stay remembered.
    exiting: AtomicBool,
}

impl Stickies {
    pub fn new(app: &AppHandle, path: PathBuf) -> Arc<Stickies> {
        Arc::new(Stickies {
            app: app.clone(),
            layout: Mutex::new(Layout::open(&path)),
            path,
            generation: AtomicU64::new(0),
            exiting: AtomicBool::new(false),
        })
    }

    pub fn is_stuck(&self, note_id: &str) -> bool {
        self.layout.lock().unwrap().get(note_id).is_some()
    }

    pub fn on_top(&self, note_id: &str) -> Option<bool> {
        self.layout.lock().unwrap().get(note_id).map(|sticky| sticky.on_top)
    }

    /// What is saved on disk, for the self-test.
    pub fn saved(&self) -> Vec<Sticky> {
        Layout::open(&self.path).all().to_vec()
    }

    /// Shows the note's window, opening it if needed. Async, because creating
    /// a window inside a synchronous command can hang.
    pub async fn open(&self, note_id: &str) -> Result<(), String> {
        if let Some(window) = self.app.get_webview_window(&label(note_id)) {
            return window.set_focus().map_err(|error| error.to_string());
        }
        let sticky = {
            let mut layout = self.layout.lock().unwrap();
            let sticky = layout.get(note_id).cloned().unwrap_or_else(|| layout.new_sticky(note_id));
            layout.put(sticky.clone());
            sticky
        };
        self.save();
        self.build(&sticky)
    }

    fn build(&self, sticky: &Sticky) -> Result<(), String> {
        let (x, y) = if self.on_screen(sticky.x, sticky.y) { (sticky.x, sticky.y) } else { (120.0, 120.0) };
        WebviewWindowBuilder::new(&self.app, label(&sticky.note_id), WebviewUrl::App("sticky.html".into()))
            .title("TackNote")
            .position(x, y)
            .inner_size(sticky.width, sticky.height)
            .min_inner_size(160.0, 120.0)
            .always_on_top(sticky.on_top)
            .decorations(false)
            .maximizable(false)
            .skip_taskbar(true)
            .build()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// A screen was unplugged: a window there would be out of reach.
    fn on_screen(&self, x: f64, y: f64) -> bool {
        let Ok(monitors) = self.app.available_monitors() else {
            return true;
        };
        monitors.iter().any(|monitor| {
            let scale = monitor.scale_factor();
            let place = monitor.position().to_logical::<f64>(scale);
            let size = monitor.size().to_logical::<f64>(scale);
            let (px, py) = (x + 20.0, y + 10.0);
            px >= place.x && px < place.x + size.width && py >= place.y && py < place.y + size.height
        })
    }

    /// Reopens the windows of the last run, once the app has started.
    pub fn restore(self: &Arc<Self>, alive: impl Fn(&str) -> bool) {
        let gone = self.layout.lock().unwrap().retain(alive);
        if !gone.is_empty() {
            self.save();
        }
        let stickies = self.layout.lock().unwrap().all().to_vec();
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            for sticky in stickies {
                if let Err(error) = this.build(&sticky) {
                    eprintln!("could not reopen a sticky window: {error}");
                }
            }
        });
    }

    /// The user closed the window: it is no longer stuck on the desktop.
    pub fn close(&self, note_id: &str) {
        if self.layout.lock().unwrap().remove(note_id) {
            self.save();
        }
        if let Some(window) = self.app.get_webview_window(&label(note_id)) {
            let _ = window.close();
        }
    }

    /// Closes the windows of deleted notes, so nobody types into a note that
    /// no longer exists.
    pub fn prune(&self, alive: impl Fn(&str) -> bool) {
        let gone = self.layout.lock().unwrap().retain(alive);
        if gone.is_empty() {
            return;
        }
        self.save();
        for note_id in gone {
            if let Some(window) = self.app.get_webview_window(&label(&note_id)) {
                let _ = window.close();
            }
        }
    }

    pub fn set_on_top(&self, note_id: &str, on_top: bool) -> Result<(), String> {
        let window = self.app.get_webview_window(&label(note_id)).ok_or("the window is not open")?;
        window.set_always_on_top(on_top).map_err(|error| error.to_string())?;
        let changed = {
            let mut layout = self.layout.lock().unwrap();
            match layout.get(note_id).cloned() {
                Some(sticky) => {
                    layout.put(Sticky { on_top, ..sticky });
                    true
                }
                None => false,
            }
        };
        if changed {
            self.save();
        }
        Ok(())
    }

    pub fn quitting(&self) {
        self.exiting.store(true, Ordering::SeqCst);
        self.save();
    }

    pub fn window_event(self: &Arc<Self>, window: &Window, event: &WindowEvent) {
        let Some(note_id) = window.label().strip_prefix(LABEL_PREFIX) else {
            return;
        };
        match event {
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => self.remember(window, note_id),
            // Closed some other way than our button, such as Cmd+W.
            WindowEvent::CloseRequested { .. } if !self.exiting.load(Ordering::SeqCst) => {
                if self.layout.lock().unwrap().remove(note_id) {
                    self.save();
                }
            }
            _ => {}
        }
    }

    fn remember(self: &Arc<Self>, window: &Window, note_id: &str) {
        let (Ok(scale), Ok(place), Ok(size)) = (window.scale_factor(), window.outer_position(), window.inner_size()) else {
            return;
        };
        let place: LogicalPosition<f64> = place.to_logical(scale);
        let size: LogicalSize<f64> = size.to_logical(scale);
        {
            let mut layout = self.layout.lock().unwrap();
            let Some(sticky) = layout.get(note_id).cloned() else {
                return;
            };
            layout.put(Sticky { x: place.x, y: place.y, width: size.width, height: size.height, ..sticky });
        }
        self.save_soon();
    }

    /// Dragging moves a window many times a second; this writes once it stops.
    fn save_soon(self: &Arc<Self>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            if this.generation.load(Ordering::SeqCst) == generation {
                this.save();
            }
        });
    }

    fn save(&self) {
        if let Err(error) = self.layout.lock().unwrap().save() {
            eprintln!("could not save the sticky windows: {error}");
        }
    }
}

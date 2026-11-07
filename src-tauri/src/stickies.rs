//! Which notes are stuck on this desktop, where, and how big. Kept on this
//! device only and never synced, because every computer's screens differ.
//! Sizes and places are in logical pixels, as window builders take them.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FORMAT_VERSION: u32 = 1;
pub const DEFAULT_WIDTH: f64 = 260.0;
pub const DEFAULT_HEIGHT: f64 = 240.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sticky {
    pub note_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Stays above other windows; on unless the user turned it off.
    #[serde(default = "on")]
    pub on_top: bool,
}

fn on() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    stickies: Vec<Sticky>,
}

pub struct Layout {
    path: PathBuf,
    stickies: Vec<Sticky>,
}

impl Layout {
    /// A missing, damaged or newer file starts empty: these are only window
    /// places, never the user's text.
    pub fn open(path: impl Into<PathBuf>) -> Layout {
        let path = path.into();
        let stickies = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Saved>(&raw).ok())
            .filter(|saved| saved.version <= FORMAT_VERSION)
            .map(|saved| saved.stickies)
            .unwrap_or_default();
        Layout { path, stickies }
    }

    pub fn all(&self) -> &[Sticky] {
        &self.stickies
    }

    pub fn get(&self, note_id: &str) -> Option<&Sticky> {
        self.stickies.iter().find(|sticky| sticky.note_id == note_id)
    }

    /// Adds the sticky, or replaces the one for the same note.
    pub fn put(&mut self, sticky: Sticky) {
        match self.stickies.iter_mut().find(|old| old.note_id == sticky.note_id) {
            Some(old) => *old = sticky,
            None => self.stickies.push(sticky),
        }
    }

    pub fn remove(&mut self, note_id: &str) -> bool {
        let before = self.stickies.len();
        self.stickies.retain(|sticky| sticky.note_id != note_id);
        self.stickies.len() != before
    }

    /// Drops the stickies whose note is gone and returns their note ids.
    pub fn retain(&mut self, alive: impl Fn(&str) -> bool) -> Vec<String> {
        let (kept, gone): (Vec<Sticky>, Vec<Sticky>) = self.stickies.drain(..).partition(|sticky| alive(&sticky.note_id));
        self.stickies = kept;
        gone.into_iter().map(|sticky| sticky.note_id).collect()
    }

    /// A new sticky goes a little below and right of the previous one.
    pub fn new_sticky(&self, note_id: &str) -> Sticky {
        let step = 30.0 * (self.stickies.len() % 10) as f64;
        Sticky {
            note_id: note_id.to_string(),
            x: 120.0 + step,
            y: 120.0 + step,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            on_top: true,
        }
    }

    /// Through a temporary file and a rename, like the notes.
    pub fn save(&self) -> io::Result<()> {
        let saved = Saved { version: FORMAT_VERSION, stickies: self.stickies.clone() };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&saved)?)?;
        fs::rename(&temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Folder {
            let path = std::env::temp_dir().join(format!("tacknote-stickies-{}", crate::model::new_id()));
            fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("stickies.json")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn places_and_sizes_survive_a_restart() {
        let folder = Folder::new();
        let mut layout = Layout::open(folder.file());
        let mut sticky = layout.new_sticky("a");
        sticky.x = 400.5;
        sticky.on_top = false;
        layout.put(sticky.clone());
        layout.save().unwrap();
        assert_eq!(Layout::open(folder.file()).all(), &[sticky]);
    }

    #[test]
    fn putting_the_same_note_again_replaces_it() {
        let folder = Folder::new();
        let mut layout = Layout::open(folder.file());
        layout.put(layout.new_sticky("a"));
        let mut moved = layout.new_sticky("a");
        moved.y = 9.0;
        layout.put(moved);
        assert_eq!(layout.all().len(), 1);
        assert_eq!(layout.get("a").unwrap().y, 9.0);
    }

    #[test]
    fn new_stickies_do_not_cover_each_other() {
        let folder = Folder::new();
        let mut layout = Layout::open(folder.file());
        let first = layout.new_sticky("a");
        layout.put(first.clone());
        let second = layout.new_sticky("b");
        assert!(second.x > first.x && second.y > first.y);
        assert!(first.on_top && second.on_top);
    }

    #[test]
    fn stickies_of_gone_notes_are_dropped() {
        let folder = Folder::new();
        let mut layout = Layout::open(folder.file());
        for id in ["a", "b", "c"] {
            layout.put(layout.new_sticky(id));
        }
        let gone = layout.retain(|id| id != "b");
        assert_eq!(gone, vec!["b".to_string()]);
        assert!(layout.get("b").is_none() && layout.get("a").is_some() && layout.get("c").is_some());
        assert!(layout.remove("a"));
        assert!(!layout.remove("a"));
    }

    #[test]
    fn a_damaged_or_newer_file_starts_empty() {
        let folder = Folder::new();
        fs::write(folder.file(), "not json").unwrap();
        assert!(Layout::open(folder.file()).all().is_empty());
        fs::write(folder.file(), r#"{"version":99,"stickies":[{"noteId":"a","x":1,"y":1,"width":1,"height":1}]}"#).unwrap();
        assert!(Layout::open(folder.file()).all().is_empty());
    }

    #[test]
    fn an_older_file_without_on_top_keeps_windows_on_top() {
        let folder = Folder::new();
        fs::write(folder.file(), r#"{"version":1,"stickies":[{"noteId":"a","x":1,"y":2,"width":3,"height":4}]}"#).unwrap();
        assert!(Layout::open(folder.file()).get("a").unwrap().on_top);
    }
}

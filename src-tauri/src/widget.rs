//! The Android home screen widgets. Rust writes every note, ready to show, to
//! one small file next to the notes, `widget.json`; the Kotlin side
//! (`NoteWidget.kt`) only reads it, so widgets can draw while the app is
//! closed. Each widget remembers on the Kotlin side which note it shows.
//!
//! A to-do ticked on a widget comes back as a small file in
//! `widget-actions/`, which Rust applies to the note and then deletes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::checklist::parse_lines;
use crate::model::Note;
use crate::store::NoteStore;

const FORMAT_VERSION: u32 = 2;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry<'a> {
    id: &'a str,
    color_index: u32,
    lines: Vec<crate::checklist::NoteLine>,
}

pub struct WidgetFeed {
    path: PathBuf,
    /// The note chosen before each widget had its own: a widget that has no
    /// note of its own yet shows this one.
    chosen: Option<String>,
    written: Option<String>,
}

impl WidgetFeed {
    pub fn open(path: impl Into<PathBuf>) -> WidgetFeed {
        let path = path.into();
        let saved: Option<serde_json::Value> = fs::read_to_string(&path).ok().and_then(|raw| serde_json::from_str(&raw).ok());
        // Version 1 called it noteId.
        let chosen = saved
            .as_ref()
            .and_then(|value| value.get("chosen").or_else(|| value.get("noteId")))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        WidgetFeed { path, chosen, written: None }
    }

    pub fn chosen(&self) -> Option<&str> {
        self.chosen.as_deref()
    }

    pub fn choose(&mut self, id: &str) {
        self.chosen = Some(id.to_string());
    }

    /// Writes the notes to show, in the order shown in the app. Writes only
    /// when something changed, and says whether it did.
    pub fn write(&mut self, visible: &[Note]) -> io::Result<bool> {
        if self.chosen.as_ref().is_some_and(|id| !visible.iter().any(|note| &note.id == id)) {
            self.chosen = None;
        }
        let entries: Vec<Entry> = visible
            .iter()
            .map(|note| Entry { id: &note.id, color_index: note.color_index, lines: parse_lines(&note.text) })
            .collect();
        let json = serde_json::json!({ "version": FORMAT_VERSION, "chosen": self.chosen, "notes": entries }).to_string();
        if self.written.as_ref() == Some(&json) && self.path.exists() {
            return Ok(false);
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        fs::write(&temporary, &json)?;
        fs::rename(&temporary, &self.path)?;
        self.written = Some(json);
        Ok(true)
    }
}

/// A to-do ticked or unticked on a widget.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetToggle {
    pub note_id: String,
    pub line: usize,
    /// The line's text as the widget showed it.
    pub text: String,
    /// What the user wants it to be.
    pub checked: bool,
}

/// Applies a toggle only if that line still says what the widget showed and
/// is not already as wanted, so a note changed meanwhile is never mis-ticked
/// and the same tap is never applied twice.
pub fn apply_toggle(store: &mut NoteStore, toggle: &WidgetToggle) -> io::Result<bool> {
    let Some(note) = store.get(&toggle.note_id).filter(|note| !note.deleted) else {
        return Ok(false);
    };
    let lines = parse_lines(&note.text);
    match lines.get(toggle.line) {
        Some(line) if line.checked == Some(!toggle.checked) && line.text == toggle.text => {
            store.toggle_checkbox(&toggle.note_id, toggle.line)
        }
        _ => Ok(false),
    }
}

/// Applies the toggles waiting in `dir`, oldest first, deleting each file
/// once handled; a damaged file is deleted too. Says whether a note changed.
pub fn apply_toggles(store: &mut NoteStore, dir: &Path) -> io::Result<bool> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(false);
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    files.sort();
    let mut changed = false;
    for file in files {
        if let Some(toggle) = fs::read_to_string(&file).ok().and_then(|raw| serde_json::from_str::<WidgetToggle>(&raw).ok()) {
            changed |= apply_toggle(store, &toggle)?;
        }
        fs::remove_file(&file)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Folder {
            let path = std::env::temp_dir().join(format!("tacknote-widget-{}", crate::model::new_id()));
            fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn written(folder: &Folder) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(folder.file("widget.json")).unwrap()).unwrap()
    }

    #[test]
    fn every_note_is_written_with_its_lines_in_the_order_shown() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        store.create("买菜\n[ ] 牛奶\n[x] 面包", 2).unwrap();
        store.create("second", 0).unwrap();
        let mut feed = WidgetFeed::open(folder.file("widget.json"));
        assert!(feed.write(&store.visible()).unwrap());
        let value = written(&folder);
        assert_eq!(value["version"], 2);
        assert_eq!(value["notes"][0]["lines"][0]["text"], "second");
        assert_eq!(value["notes"][1]["colorIndex"], 2);
        assert_eq!(value["notes"][1]["lines"][1]["checked"], false);
        assert_eq!(value["notes"][1]["lines"][2]["checked"], true);
        assert_eq!(value["notes"][1]["lines"][0]["checked"], serde_json::Value::Null);
        assert!(!feed.write(&store.visible()).unwrap(), "nothing changed, nothing written");
    }

    #[test]
    fn the_old_single_choice_is_kept_until_its_note_is_gone() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        let note = store.create("a", 0).unwrap();
        fs::write(folder.file("widget.json"), format!(r#"{{"version":1,"noteId":"{}","text":"a","colorIndex":0}}"#, note.id)).unwrap();
        let mut feed = WidgetFeed::open(folder.file("widget.json"));
        assert_eq!(feed.chosen(), Some(note.id.as_str()));
        feed.write(&store.visible()).unwrap();
        assert_eq!(written(&folder)["chosen"], note.id.as_str());
        store.delete(&note.id).unwrap();
        feed.write(&store.visible()).unwrap();
        assert_eq!(feed.chosen(), None);
        assert_eq!(written(&folder)["chosen"], serde_json::Value::Null);
    }

    fn toggle(note: &Note, line: usize, text: &str, checked: bool) -> WidgetToggle {
        WidgetToggle { note_id: note.id.clone(), line, text: text.into(), checked }
    }

    #[test]
    fn a_tick_on_the_widget_changes_the_note() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        let note = store.create("买菜\n[ ] 牛奶", 0).unwrap();
        assert!(apply_toggle(&mut store, &toggle(&note, 1, "牛奶", true)).unwrap());
        assert_eq!(store.get(&note.id).unwrap().text, "买菜\n[x] 牛奶");
        assert!(apply_toggle(&mut store, &toggle(&note, 1, "牛奶", false)).unwrap());
        assert_eq!(store.get(&note.id).unwrap().text, "买菜\n[ ] 牛奶");
    }

    #[test]
    fn a_tick_for_a_line_that_changed_meanwhile_is_ignored() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        let note = store.create("[ ] 牛奶\n[ ] 面包", 0).unwrap();
        // The widget showed "牛奶" on line 0, but the note was edited since.
        store.update(&note.id, crate::store::Change { text: Some("[ ] 鸡蛋\n[ ] 面包".into()), ..Default::default() }).unwrap();
        assert!(!apply_toggle(&mut store, &toggle(&note, 0, "牛奶", true)).unwrap());
        assert!(!apply_toggle(&mut store, &toggle(&note, 9, "牛奶", true)).unwrap());
        assert_eq!(store.get(&note.id).unwrap().text, "[ ] 鸡蛋\n[ ] 面包");
    }

    #[test]
    fn the_same_tap_twice_or_on_a_deleted_note_changes_nothing() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        let note = store.create("[ ] 牛奶", 0).unwrap();
        assert!(apply_toggle(&mut store, &toggle(&note, 0, "牛奶", true)).unwrap());
        assert!(!apply_toggle(&mut store, &toggle(&note, 0, "牛奶", true)).unwrap());
        // Synced first, so deleting leaves a tombstone a tick must not revive.
        let synced: Vec<Note> = store.all().iter().map(|n| n.with_synced_rev(Some(n.rev.clone()))).collect();
        let revs = store.revs();
        store.apply_sync(synced, &revs).unwrap();
        store.delete(&note.id).unwrap();
        assert!(!apply_toggle(&mut store, &toggle(&note, 0, "牛奶", false)).unwrap());
        assert!(store.get(&note.id).unwrap().deleted, "still deleted");
    }

    #[test]
    fn waiting_toggles_are_applied_in_order_and_their_files_removed() {
        let folder = Folder::new();
        let mut store = NoteStore::open(folder.file("notes.json")).unwrap();
        let note = store.create("[ ] 牛奶", 0).unwrap();
        let dir = folder.file("widget-actions");
        assert!(!apply_toggles(&mut store, &dir).unwrap(), "no folder yet");
        fs::create_dir_all(&dir).unwrap();
        let action = |checked: bool| format!(r#"{{"noteId":"{}","line":0,"text":"牛奶","checked":{checked}}}"#, note.id);
        fs::write(dir.join("1-a.json"), action(true)).unwrap();
        fs::write(dir.join("2-b.json"), action(false)).unwrap();
        fs::write(dir.join("3-c.json"), "{damaged").unwrap();
        fs::write(dir.join("4-d.json.tmp"), action(true)).unwrap();
        assert!(apply_toggles(&mut store, &dir).unwrap());
        assert_eq!(store.get(&note.id).unwrap().text, "[ ] 牛奶", "ticked, then unticked");
        let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("4-d.json.tmp")], "a file still being written is left alone");
    }
}

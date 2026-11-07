//! The notes saved on this device: one JSON file in the app's data folder.
//! Every change is written at once, through a temporary file and a rename, so
//! a crash mid-write leaves the previous file intact.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::checklist::toggle_line;
use crate::model::{new_id, Edit, Note};

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedState {
    version: u32,
    device_id: String,
    notes: Vec<Note>,
}

/// Where the current time comes from; tests use a clock that always moves on.
pub type Clock = Box<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The fields a user can change in one edit; `None` keeps the current value.
#[derive(Clone, Debug, Default)]
pub struct Change {
    pub text: Option<String>,
    pub color_index: Option<u32>,
    pub pinned: Option<bool>,
}

pub struct NoteStore {
    path: PathBuf,
    device_id: String,
    notes: HashMap<String, Note>,
    clock: Clock,
}

impl NoteStore {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<NoteStore> {
        NoteStore::open_with_clock(path, Box::new(Utc::now))
    }

    /// A damaged file is moved aside, never overwritten, so it can still be
    /// recovered by hand. A file from a newer version is refused, not emptied.
    pub fn open_with_clock(path: impl Into<PathBuf>, clock: Clock) -> io::Result<NoteStore> {
        let path = path.into();
        let saved = match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<SavedState>(&raw) {
                Ok(state) if state.version > FORMAT_VERSION => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("the notes were saved by a newer version (format {})", state.version),
                    ));
                }
                Ok(state) => Some(state),
                Err(_) => {
                    keep_aside(&path, clock())?;
                    None
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let is_new = saved.is_none();
        let (device_id, notes) = match saved {
            Some(state) => (state.device_id, state.notes),
            None => (new_id()[..8].to_string(), Vec::new()),
        };
        let store = NoteStore {
            path,
            device_id,
            notes: notes.into_iter().map(|note| (note.id.clone(), note)).collect(),
            clock,
        };
        if is_new {
            // Saves the device id at once, so a restart keeps it.
            store.persist()?;
        }
        Ok(store)
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Everything, including tombstones. This is what sync works with.
    pub fn all(&self) -> Vec<Note> {
        self.notes.values().cloned().collect()
    }

    /// The notes to show: pinned first, then the most recently changed.
    pub fn visible(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self.notes.values().filter(|note| !note.deleted).cloned().collect();
        notes.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
        notes
    }

    pub fn get(&self, id: &str) -> Option<&Note> {
        self.notes.get(id)
    }

    /// The rev of every note, taken when a sync starts.
    pub fn revs(&self) -> HashMap<String, String> {
        self.notes.values().map(|note| (note.id.clone(), note.rev.clone())).collect()
    }

    pub fn create(&mut self, text: &str, color_index: u32) -> io::Result<Note> {
        let mut note = Note::new(&self.device_id, text, (self.clock)());
        note.color_index = color_index;
        self.put(note.clone())?;
        Ok(note)
    }

    /// Returns whether anything changed; an edit that changes nothing keeps
    /// the rev. Changing the text clears a conflict copy's label and, as in
    /// the merge, brings back a note deleted on another device.
    pub fn update(&mut self, id: &str, change: Change) -> io::Result<bool> {
        let Some(note) = self.notes.get(id) else {
            return Ok(false);
        };
        let unchanged = change.text.as_ref().is_none_or(|text| *text == note.text)
            && change.color_index.is_none_or(|color| color == note.color_index)
            && change.pinned.is_none_or(|pinned| pinned == note.pinned);
        if unchanged {
            return Ok(false);
        }
        let text_given = change.text.is_some();
        let edited = note.edited(
            &self.device_id,
            Edit {
                text: change.text,
                color_index: change.color_index,
                pinned: change.pinned,
                deleted: text_given.then_some(false),
                conflict_copy: text_given.then_some(false),
            },
            (self.clock)(),
        );
        self.put(edited)?;
        Ok(true)
    }

    pub fn toggle_checkbox(&mut self, id: &str, line: usize) -> io::Result<bool> {
        let Some(note) = self.notes.get(id) else {
            return Ok(false);
        };
        let text = toggle_line(&note.text, line);
        self.update(id, Change { text: Some(text), ..Change::default() })
    }

    /// A note that never reached the server is removed outright; any other
    /// becomes a tombstone so the deletion reaches other devices.
    pub fn delete(&mut self, id: &str) -> io::Result<()> {
        let Some(note) = self.notes.get(id) else {
            return Ok(());
        };
        if note.synced_rev.is_none() {
            self.notes.remove(id);
            return self.persist();
        }
        let removed = note.edited(&self.device_id, Edit { deleted: Some(true), ..Edit::default() }, (self.clock)());
        self.put(removed)
    }

    /// Applies a finished sync. A note edited while the sync was running keeps
    /// that newer edit, and the next sync uploads it.
    pub fn apply_sync(&mut self, merged: Vec<Note>, revs_at_start: &HashMap<String, String>) -> io::Result<()> {
        let merged_revs: HashMap<String, String> =
            merged.iter().map(|note| (note.id.clone(), note.rev.clone())).collect();
        let mut result: HashMap<String, Note> = HashMap::new();

        for current in self.notes.values() {
            let start_rev = revs_at_start.get(&current.id);
            if start_rev == Some(&current.rev) {
                continue;
            }
            // If the server's version won, synced_rev stays old on purpose: the
            // next sync then sees edits on both sides and keeps a conflict copy.
            let uploaded = merged_revs.get(&current.id) == start_rev;
            let kept = if uploaded {
                current.with_synced_rev(start_rev.cloned())
            } else {
                current.clone()
            };
            result.insert(current.id.clone(), kept);
        }
        for note in merged {
            if result.contains_key(&note.id) {
                continue;
            }
            let deleted_during_sync = revs_at_start.contains_key(&note.id) && !self.notes.contains_key(&note.id);
            let kept = if deleted_during_sync {
                note.edited(&self.device_id, Edit { deleted: Some(true), ..Edit::default() }, (self.clock)())
            } else {
                note
            };
            result.insert(kept.id.clone(), kept);
        }

        self.notes = result;
        self.persist()
    }

    fn put(&mut self, note: Note) -> io::Result<()> {
        self.notes.insert(note.id.clone(), note);
        self.persist()
    }

    fn persist(&self) -> io::Result<()> {
        let mut notes: Vec<&Note> = self.notes.values().collect();
        notes.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let state = serde_json::json!({
            "version": FORMAT_VERSION,
            "deviceId": self.device_id,
            "notes": notes,
        });
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temporary = sibling(&self.path, "tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&state)?)?;
        fs::rename(&temporary, &self.path)
    }
}

/// `notes.json` becomes `notes.json.<suffix>` in the same folder.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    name.push(format!(".{suffix}"));
    path.with_file_name(name)
}

fn keep_aside(path: &Path, now: DateTime<Utc>) -> io::Result<()> {
    let target = sibling(path, &format!("corrupt-{}", now.format("%Y%m%dT%H%M%S%.6fZ")));
    fs::rename(path, target)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    use chrono::Duration;

    use super::*;

    /// A fresh folder per test, removed when the test ends.
    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Folder {
            let path = std::env::temp_dir().join(format!("tacknote-test-{}", new_id()));
            fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("notes.json")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Each reading is one minute after the previous one.
    fn clock() -> Clock {
        let minutes = Arc::new(AtomicI64::new(0));
        let start = DateTime::parse_from_rfc3339("2026-11-07T10:00:00Z").unwrap().with_timezone(&Utc);
        Box::new(move || start + Duration::minutes(minutes.fetch_add(1, Ordering::SeqCst)))
    }

    fn open(folder: &Folder) -> NoteStore {
        NoteStore::open_with_clock(folder.file(), clock()).unwrap()
    }

    fn text(value: &str) -> Change {
        Change { text: Some(value.into()), ..Change::default() }
    }

    fn mark_synced(store: &NoteStore) -> Vec<Note> {
        store.all().iter().map(|note| note.with_synced_rev(Some(note.rev.clone()))).collect()
    }

    #[test]
    fn notes_survive_a_restart() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("保存我", 0).unwrap();
        let reopened = open(&folder);
        assert_eq!(reopened.get(&note.id).unwrap().text, "保存我");
        assert_eq!(reopened.device_id(), store.device_id());
    }

    #[test]
    fn a_new_device_keeps_its_id_across_restarts_before_any_note() {
        let folder = Folder::new();
        let first = open(&folder);
        assert_eq!(open(&folder).device_id(), first.device_id());
    }

    #[test]
    fn pinned_notes_come_first_then_the_newest() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let older = store.create("older", 0).unwrap();
        let newer = store.create("newer", 0).unwrap();
        assert_eq!(store.visible()[0].id, newer.id);
        store.update(&older.id, Change { pinned: Some(true), ..Change::default() }).unwrap();
        assert_eq!(store.visible()[0].id, older.id);
    }

    #[test]
    fn an_update_that_changes_nothing_keeps_the_rev() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("same", 0).unwrap();
        assert!(!store.update(&note.id, text("same")).unwrap());
        assert_eq!(store.get(&note.id).unwrap().rev, note.rev);
    }

    #[test]
    fn deleting_a_never_synced_note_removes_it_completely() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("x", 0).unwrap();
        store.delete(&note.id).unwrap();
        assert!(store.all().is_empty());
    }

    #[test]
    fn deleting_a_synced_note_leaves_a_tombstone_for_other_devices() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("x", 0).unwrap();
        let revs = store.revs();
        store.apply_sync(mark_synced(&store), &revs).unwrap();
        store.delete(&note.id).unwrap();
        assert!(store.get(&note.id).unwrap().deleted);
        assert!(store.visible().is_empty());
    }

    #[test]
    fn editing_a_conflict_copy_clears_its_label() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("x", 0).unwrap();
        let copy = Note { conflict_copy: true, ..note.clone() };
        let revs = store.revs();
        store.apply_sync(vec![copy], &revs).unwrap();
        store.update(&note.id, text("edited")).unwrap();
        assert!(!store.get(&note.id).unwrap().conflict_copy);
    }

    #[test]
    fn an_edit_made_during_a_sync_is_not_overwritten() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("before", 0).unwrap();
        let start_revs = store.revs();
        let merged = mark_synced(&store);

        store.update(&note.id, text("typed during sync")).unwrap();
        store.apply_sync(merged, &start_revs).unwrap();

        let result = store.get(&note.id).unwrap();
        assert_eq!(result.text, "typed during sync");
        assert!(result.has_unsynced_changes());
        assert_eq!(result.synced_rev.as_ref(), start_revs.get(&note.id));
    }

    #[test]
    fn a_note_created_during_a_sync_is_kept() {
        let folder = Folder::new();
        let mut store = open(&folder);
        store.create("first", 0).unwrap();
        let start_revs = store.revs();
        let merged = mark_synced(&store);
        let added = store.create("added during sync", 0).unwrap();
        store.apply_sync(merged, &start_revs).unwrap();
        assert_eq!(store.get(&added.id).unwrap().text, "added during sync");
    }

    #[test]
    fn a_note_deleted_during_its_first_sync_does_not_come_back() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("bye", 0).unwrap();
        let start_revs = store.revs();
        let merged = mark_synced(&store);
        store.delete(&note.id).unwrap();
        store.apply_sync(merged, &start_revs).unwrap();

        assert!(store.visible().is_empty());
        let tombstone = store.get(&note.id).unwrap();
        assert!(tombstone.deleted);
        assert!(tombstone.has_unsynced_changes());
    }

    #[test]
    fn a_synced_note_unchanged_during_sync_takes_the_merged_version() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("old", 0).unwrap();
        let start_revs = store.revs();
        let from_server = note.edited("b", Edit { text: Some("new".into()), ..Edit::default() }, note.updated_at);
        let merged = vec![from_server.with_synced_rev(Some(from_server.rev.clone()))];
        store.apply_sync(merged, &start_revs).unwrap();
        let result = store.get(&note.id).unwrap();
        assert_eq!(result.text, "new");
        assert!(!result.has_unsynced_changes());
    }

    #[test]
    fn a_damaged_local_file_is_kept_aside_instead_of_crashing() {
        let folder = Folder::new();
        fs::write(folder.file(), "not json").unwrap();
        let reopened = open(&folder);
        assert!(reopened.all().is_empty());
        let aside: Vec<String> = fs::read_dir(&folder.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("notes.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read_to_string(folder.0.join(&aside[0])).unwrap(), "not json");
    }

    #[test]
    fn a_file_from_a_newer_version_is_refused_not_emptied() {
        let folder = Folder::new();
        let newer = r#"{"version":99,"deviceId":"abc","notes":[]}"#;
        fs::write(folder.file(), newer).unwrap();
        assert!(NoteStore::open_with_clock(folder.file(), clock()).is_err());
        assert_eq!(fs::read_to_string(folder.file()).unwrap(), newer);
    }

    #[test]
    fn every_change_is_on_disk_and_no_temporary_file_is_left() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("a", 0).unwrap();
        store.update(&note.id, text("b")).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&fs::read_to_string(folder.file()).unwrap()).unwrap();
        assert_eq!(saved["notes"][0]["text"], "b");
        assert!(!sibling(&folder.file(), "tmp").exists());
    }

    #[test]
    fn typing_into_a_note_deleted_elsewhere_brings_it_back() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("old", 0).unwrap();
        let revs = store.revs();
        let removed = note.edited("b", Edit { deleted: Some(true), ..Edit::default() }, note.updated_at);
        store.apply_sync(vec![removed.with_synced_rev(Some(removed.rev.clone()))], &revs).unwrap();
        assert!(store.visible().is_empty());

        store.update(&note.id, text("old keep me")).unwrap();
        let visible = store.visible();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].text, "old keep me");
    }

    #[test]
    fn changing_only_the_color_of_a_deleted_note_keeps_it_deleted() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("x", 0).unwrap();
        let revs = store.revs();
        store.apply_sync(mark_synced(&store), &revs).unwrap();
        store.delete(&note.id).unwrap();
        store.update(&note.id, Change { color_index: Some(2), ..Change::default() }).unwrap();
        assert!(store.get(&note.id).unwrap().deleted);
    }

    #[test]
    fn toggling_a_checkbox_saves_the_new_text() {
        let folder = Folder::new();
        let mut store = open(&folder);
        let note = store.create("list\n[ ] milk", 0).unwrap();
        assert!(store.toggle_checkbox(&note.id, 1).unwrap());
        assert_eq!(open(&folder).get(&note.id).unwrap().text, "list\n[x] milk");
        assert!(!store.toggle_checkbox(&note.id, 0).unwrap());
    }
}

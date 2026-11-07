//! The note on the Android home screen widget. Rust writes one small file next
//! to the notes, `widget.json`; the widget's Kotlin code (`NoteWidget.kt`)
//! only reads it, so the widget can draw itself even when the app is closed.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::checklist::parse_lines;
use crate::model::Note;

const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetNote {
    /// `None` until the user chooses a note, or after it is deleted.
    pub note_id: Option<String>,
    /// Ready to show: to-do boxes are drawn as ☐ and ☑.
    pub text: String,
    pub color_index: u32,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    #[serde(flatten)]
    note: WidgetNote,
}

/// A widget cannot draw clickable boxes, so they become characters.
pub fn widget_text(text: &str) -> String {
    parse_lines(text)
        .into_iter()
        .map(|line| match line.checked {
            Some(true) => format!("☑ {}", line.text),
            Some(false) => format!("☐ {}", line.text),
            None => line.text,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct WidgetFeed {
    path: PathBuf,
    shown: WidgetNote,
}

impl WidgetFeed {
    pub fn open(path: impl Into<PathBuf>) -> WidgetFeed {
        let path = path.into();
        let shown = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Saved>(&raw).ok())
            .map(|saved| saved.note)
            .unwrap_or_default();
        WidgetFeed { path, shown }
    }

    pub fn chosen(&self) -> Option<&str> {
        self.shown.note_id.as_deref()
    }

    /// Shows `note` as it is now; a deleted or missing note clears the widget.
    /// Writes only when something changed, and says whether it did.
    pub fn show(&mut self, note: Option<&Note>) -> io::Result<bool> {
        let next = match note {
            Some(note) if !note.deleted => WidgetNote {
                note_id: Some(note.id.clone()),
                text: widget_text(&note.text),
                color_index: note.color_index,
            },
            _ => WidgetNote::default(),
        };
        if next == self.shown && self.path.exists() {
            return Ok(false);
        }
        let saved = Saved { version: FORMAT_VERSION, note: next.clone() };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        fs::write(&temporary, serde_json::to_vec(&saved)?)?;
        fs::rename(&temporary, &self.path)?;
        self.shown = next;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::model::{Edit, Note};

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Folder {
            let path = std::env::temp_dir().join(format!("tacknote-widget-{}", crate::model::new_id()));
            fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("widget.json")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn note(text: &str) -> Note {
        let mut note = Note::new("a", text, Utc::now());
        note.color_index = 2;
        note
    }

    #[test]
    fn to_do_boxes_become_characters() {
        assert_eq!(widget_text("买菜\n[ ] 牛奶\n[x] 面包"), "买菜\n☐ 牛奶\n☑ 面包");
    }

    #[test]
    fn the_chosen_note_is_written_for_the_widget_and_remembered() {
        let folder = Folder::new();
        let mut feed = WidgetFeed::open(folder.file());
        assert_eq!(feed.chosen(), None);
        let chosen = note("[ ] 牛奶");
        assert!(feed.show(Some(&chosen)).unwrap());
        let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(folder.file()).unwrap()).unwrap();
        assert_eq!(raw["text"], "☐ 牛奶");
        assert_eq!(raw["colorIndex"], 2);
        assert_eq!(raw["noteId"], chosen.id.as_str());
        assert_eq!(WidgetFeed::open(folder.file()).chosen(), Some(chosen.id.as_str()));
    }

    #[test]
    fn nothing_is_written_when_nothing_changed() {
        let folder = Folder::new();
        let mut feed = WidgetFeed::open(folder.file());
        let chosen = note("a");
        feed.show(Some(&chosen)).unwrap();
        assert!(!feed.show(Some(&chosen)).unwrap());
        let edited = chosen.edited("a", Edit { text: Some("b".into()), ..Edit::default() }, Utc::now());
        assert!(feed.show(Some(&edited)).unwrap());
    }

    #[test]
    fn a_deleted_note_clears_the_widget_and_the_choice() {
        let folder = Folder::new();
        let mut feed = WidgetFeed::open(folder.file());
        let chosen = note("a");
        feed.show(Some(&chosen)).unwrap();
        let deleted = chosen.edited("a", Edit { deleted: Some(true), ..Edit::default() }, Utc::now());
        assert!(feed.show(Some(&deleted)).unwrap());
        assert_eq!(feed.chosen(), None);
        assert!(fs::read_to_string(folder.file()).unwrap().contains("\"noteId\":null"));
        feed.show(Some(&chosen)).unwrap();
        assert!(feed.show(None).unwrap(), "a note gone entirely clears it too");
    }

    #[test]
    fn a_damaged_file_starts_with_no_choice() {
        let folder = Folder::new();
        fs::write(folder.file(), "{oops").unwrap();
        assert_eq!(WidgetFeed::open(folder.file()).chosen(), None);
    }
}

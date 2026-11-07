//! A note and how it changes. Every edit gets a new `rev`, so two copies with
//! the same rev are identical; `synced_rev` remembers the rev this device last
//! agreed on with the server and is never uploaded.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A random 16-character hex id.
pub fn new_id() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the system random source is unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The time in microseconds in base 36, then the device id.
pub fn new_rev(device_id: &str, now: DateTime<Utc>) -> String {
    format!("{}-{device_id}", base36(now.timestamp_micros().max(0) as u64))
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub rev: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub color_index: u32,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Deleted notes stay as tombstones so the deletion reaches other devices.
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub conflict_copy: bool,
    /// The rev this device last agreed on with the server. Never uploaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_rev: Option<String>,
}

/// The fields an edit changes; `None` keeps the current value.
#[derive(Clone, Debug, Default)]
pub struct Edit {
    pub text: Option<String>,
    pub color_index: Option<u32>,
    pub pinned: Option<bool>,
    pub deleted: Option<bool>,
    pub conflict_copy: Option<bool>,
}

impl Note {
    pub fn new(device_id: &str, text: impl Into<String>, now: DateTime<Utc>) -> Note {
        Note {
            id: new_id(),
            rev: new_rev(device_id, now),
            text: text.into(),
            color_index: 0,
            pinned: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            conflict_copy: false,
            synced_rev: None,
        }
    }

    pub fn edited(&self, device_id: &str, edit: Edit, now: DateTime<Utc>) -> Note {
        Note {
            id: self.id.clone(),
            rev: new_rev(device_id, now),
            text: edit.text.unwrap_or_else(|| self.text.clone()),
            color_index: edit.color_index.unwrap_or(self.color_index),
            pinned: edit.pinned.unwrap_or(self.pinned),
            created_at: self.created_at,
            updated_at: now,
            deleted: edit.deleted.unwrap_or(self.deleted),
            conflict_copy: edit.conflict_copy.unwrap_or(self.conflict_copy),
            synced_rev: self.synced_rev.clone(),
        }
    }

    pub fn with_synced_rev(&self, synced_rev: Option<String>) -> Note {
        Note {
            synced_rev,
            ..self.clone()
        }
    }

    pub fn has_unsynced_changes(&self) -> bool {
        self.synced_rev.as_deref() != Some(self.rev.as_str())
    }

    /// The note as it is uploaded: without the local-only `synced_rev`.
    pub fn for_upload(&self) -> Note {
        self.with_synced_rev(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-11-07T10:{minute:02}:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn ids_are_random_hex() {
        let a = new_id();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_id());
    }

    #[test]
    fn every_edit_gets_a_new_rev() {
        let note = Note::new("a", "x", at(0));
        let edited = note.edited("a", Edit::default(), at(1));
        assert_ne!(note.rev, edited.rev);
        assert!(edited.rev.ends_with("-a"));
    }

    #[test]
    fn upload_json_never_contains_local_fields() {
        let note = Note::new("a", "x", at(0));
        let synced = note.with_synced_rev(Some(note.rev.clone()));
        let local = serde_json::to_string(&synced).unwrap();
        let remote = serde_json::to_string(&synced.for_upload()).unwrap();
        assert!(local.contains("syncedRev"));
        assert!(!remote.contains("syncedRev"));
    }

    #[test]
    fn json_round_trip_keeps_every_field() {
        let mut note = Note::new("a", "你好\n[ ] 待办", at(0));
        note.color_index = 3;
        note.pinned = true;
        note.conflict_copy = true;
        let back: Note = serde_json::from_str(&serde_json::to_string(&note).unwrap()).unwrap();
        assert_eq!(back, note);
    }

    #[test]
    fn missing_optional_fields_get_defaults() {
        let json = r#"{"id":"1","rev":"r","createdAt":"2026-11-07T10:00:00Z","updatedAt":"2026-11-07T10:00:00Z"}"#;
        let note: Note = serde_json::from_str(json).unwrap();
        assert_eq!(note.text, "");
        assert!(!note.deleted && !note.pinned && note.synced_rev.is_none());
    }
}

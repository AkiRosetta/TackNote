//! The order the user dragged the notes into. It is synced apart from the
//! notes, so moving a note never touches its text, and a move on one device
//! with an edit on another never makes a conflict copy. When two devices both
//! reorder, the newer order wins; no text can be lost either way.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{new_rev, Note};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteOrder {
    /// Note ids, top first. Notes missing here come before all listed ones.
    pub ids: Vec<String>,
    pub rev: String,
    pub updated_at: DateTime<Utc>,
    /// The rev this device last agreed on with the server. Never uploaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_rev: Option<String>,
}

impl NoteOrder {
    pub fn new(ids: Vec<String>, device_id: &str, now: DateTime<Utc>) -> NoteOrder {
        NoteOrder { ids, rev: new_rev(device_id, now), updated_at: now, synced_rev: None }
    }

    pub fn for_upload(&self) -> NoteOrder {
        NoteOrder { synced_rev: None, ..self.clone() }
    }

    fn changed_since_sync(&self) -> bool {
        self.synced_rev.as_deref() != Some(self.rev.as_str())
    }
}

/// The notes to show: pinned first, then notes not in the order (the newest
/// first, such as ones just written), then the order the user chose.
pub fn sort(notes: &mut [Note], order: Option<&NoteOrder>) {
    let place: HashMap<&str, usize> = order
        .map(|order| order.ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect())
        .unwrap_or_default();
    notes.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| match (place.get(a.id.as_str()), place.get(b.id.as_str())) {
                (Some(x), Some(y)) => x.cmp(y),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
}

/// `shown` is the order on screen. Moves `id` to just before `before`, or to
/// the end of the list when `before` is `None`.
pub fn moved(shown: &[String], id: &str, before: Option<&str>) -> Vec<String> {
    let mut ids: Vec<String> = shown.iter().filter(|other| *other != id).cloned().collect();
    let at = before.and_then(|target| ids.iter().position(|other| other == target)).unwrap_or(ids.len());
    ids.insert(at, id.to_string());
    ids
}

#[derive(Debug, PartialEq)]
pub struct OrderMerge {
    /// With `synced_rev` set, on the assumption the upload succeeds.
    pub order: Option<NoteOrder>,
    pub remote_needs_update: bool,
}

/// Only one side changed: that side wins. Both changed: the newer wins.
pub fn merge_order(local: Option<&NoteOrder>, remote: Option<&NoteOrder>) -> OrderMerge {
    let winner = match (local, remote) {
        (None, None) => return OrderMerge { order: None, remote_needs_update: false },
        (Some(mine), None) => mine,
        (None, Some(theirs)) => theirs,
        (Some(mine), Some(theirs)) if mine.rev == theirs.rev => mine,
        (Some(mine), Some(theirs)) => {
            let changed_there = mine.synced_rev.as_deref() != Some(theirs.rev.as_str());
            match (mine.changed_since_sync(), changed_there) {
                (true, false) => mine,
                (false, _) => theirs,
                (true, true) => {
                    if (mine.updated_at, &mine.rev) >= (theirs.updated_at, &theirs.rev) {
                        mine
                    } else {
                        theirs
                    }
                }
            }
        }
    };
    let remote_needs_update = remote.map(|theirs| theirs.rev.as_str()) != Some(winner.rev.as_str());
    let order = NoteOrder { synced_rev: Some(winner.rev.clone()), ..winner.clone() };
    OrderMerge { order: Some(order), remote_needs_update }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-07T10:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(minutes)
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    fn synced(list: &[&str], minute: i64) -> NoteOrder {
        let order = NoteOrder::new(ids(list), "a", at(minute));
        NoteOrder { synced_rev: Some(order.rev.clone()), ..order }
    }

    fn note(id: &str, minute: i64, pinned: bool) -> Note {
        let mut note = Note::new("a", id, at(minute));
        note.id = id.to_string();
        note.pinned = pinned;
        note
    }

    #[test]
    fn moving_puts_the_note_before_the_target_or_at_the_end() {
        let shown = ids(&["a", "b", "c", "d"]);
        assert_eq!(moved(&shown, "d", Some("b")), ids(&["a", "d", "b", "c"]));
        assert_eq!(moved(&shown, "a", Some("d")), ids(&["b", "c", "a", "d"]));
        assert_eq!(moved(&shown, "a", None), ids(&["b", "c", "d", "a"]));
        assert_eq!(moved(&shown, "b", Some("gone")), ids(&["a", "c", "d", "b"]));
    }

    #[test]
    fn notes_follow_the_order_with_pinned_and_new_notes_first() {
        let mut notes = vec![note("old", 1, false), note("mid", 2, false), note("pin", 0, true), note("new", 9, false)];
        let order = NoteOrder::new(ids(&["old", "mid", "pin"]), "a", at(5));
        sort(&mut notes, Some(&order));
        let shown: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(shown, ["pin", "new", "old", "mid"]);
    }

    #[test]
    fn without_an_order_the_newest_comes_first() {
        let mut notes = vec![note("old", 1, false), note("new", 9, false), note("pin", 0, true)];
        sort(&mut notes, None);
        let shown: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(shown, ["pin", "new", "old"]);
    }

    #[test]
    fn a_local_reorder_is_uploaded() {
        let base = synced(&["a", "b"], 1);
        let mine = NoteOrder { synced_rev: base.synced_rev.clone(), ..NoteOrder::new(ids(&["b", "a"]), "a", at(2)) };
        let result = merge_order(Some(&mine), Some(&base));
        assert_eq!(result.order.as_ref().unwrap().ids, ids(&["b", "a"]));
        assert!(result.remote_needs_update);
        assert_eq!(result.order.unwrap().synced_rev, Some(mine.rev));
    }

    #[test]
    fn a_remote_reorder_is_taken_without_uploading() {
        let mine = synced(&["a", "b"], 1);
        let theirs = NoteOrder::new(ids(&["b", "a"]), "b", at(2));
        let result = merge_order(Some(&mine), Some(&theirs));
        assert_eq!(result.order.unwrap().ids, ids(&["b", "a"]));
        assert!(!result.remote_needs_update);
    }

    #[test]
    fn when_both_reordered_the_newer_wins_in_both_directions() {
        let base = synced(&["a", "b", "c"], 1);
        let older = NoteOrder { synced_rev: base.synced_rev.clone(), ..NoteOrder::new(ids(&["c", "a", "b"]), "a", at(2)) };
        let newer = NoteOrder::new(ids(&["b", "c", "a"]), "b", at(3));
        let result = merge_order(Some(&older), Some(&newer));
        assert_eq!(result.order.unwrap().ids, newer.ids);
        assert!(!result.remote_needs_update);

        let newer_here = NoteOrder { synced_rev: base.synced_rev.clone(), ..NoteOrder::new(ids(&["c", "a", "b"]), "a", at(4)) };
        let result = merge_order(Some(&newer_here), Some(&newer));
        assert_eq!(result.order.unwrap().ids, newer_here.ids);
        assert!(result.remote_needs_update);
    }

    #[test]
    fn the_first_order_reaches_the_server_and_a_new_device() {
        let mine = NoteOrder::new(ids(&["a"]), "a", at(1));
        let result = merge_order(Some(&mine), None);
        assert!(result.remote_needs_update);
        let result = merge_order(None, Some(&mine));
        assert_eq!(result.order.unwrap().ids, ids(&["a"]));
        assert!(!result.remote_needs_update);
        assert_eq!(merge_order(None, None), OrderMerge { order: None, remote_needs_update: false });
    }

    #[test]
    fn the_same_order_on_both_sides_needs_no_upload() {
        let mine = synced(&["a", "b"], 1);
        let result = merge_order(Some(&mine), Some(&mine.for_upload()));
        assert!(!result.remote_needs_update);
    }

    #[test]
    fn the_uploaded_order_never_carries_the_local_field() {
        let json = serde_json::to_string(&synced(&["a"], 1).for_upload()).unwrap();
        assert!(!json.contains("syncedRev"));
    }
}

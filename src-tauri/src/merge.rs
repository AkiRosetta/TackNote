//! Merges this device's notes with the server's, note by note. A note edited
//! on both sides is never silently lost: the newer edit keeps the note and the
//! older text becomes a conflict copy.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};

use crate::model::Note;

/// Tombstones are dropped after this long. A device offline for longer may
/// bring a deleted note back, which is safer than losing text.
pub fn tombstone_retention() -> Duration {
    Duration::days(90)
}

#[derive(Debug)]
pub struct MergeResult {
    /// Every note has `synced_rev` set, on the assumption the upload succeeds.
    pub notes: Vec<Note>,
    pub remote_needs_update: bool,
    pub conflicts: usize,
}

pub fn merge_notes(local: &[Note], remote: &[Note], device_id: &str, now: DateTime<Utc>) -> MergeResult {
    let local_by_id: HashMap<&str, &Note> = local.iter().map(|n| (n.id.as_str(), n)).collect();
    let remote_by_id: HashMap<&str, &Note> = remote.iter().map(|n| (n.id.as_str(), n)).collect();

    // Local order first, then notes only the server has.
    let mut seen = HashSet::new();
    let ids: Vec<&str> = local
        .iter()
        .chain(remote.iter())
        .map(|n| n.id.as_str())
        .filter(|id| seen.insert(*id))
        .collect();

    let mut merged: Vec<Note> = Vec::new();
    let mut conflicts = 0;

    for id in ids {
        let (mine, theirs) = (local_by_id.get(id), remote_by_id.get(id));
        let (Some(mine), Some(theirs)) = (mine, theirs) else {
            merged.push((*mine.or(theirs).unwrap()).clone());
            continue;
        };
        if mine.rev == theirs.rev {
            merged.push((*mine).clone());
            continue;
        }

        let changed_here = Some(mine.rev.as_str()) != mine.synced_rev.as_deref();
        let changed_there = Some(theirs.rev.as_str()) != mine.synced_rev.as_deref();
        if changed_here && !changed_there {
            merged.push((*mine).clone());
        } else if !changed_here {
            merged.push((*theirs).clone());
        } else if mine.deleted != theirs.deleted {
            // An edit beats a deletion, so the text survives.
            merged.push(if mine.deleted { (*theirs).clone() } else { (*mine).clone() });
        } else {
            let winner = newer(mine, theirs);
            merged.push(winner.clone());
            if mine.text != theirs.text && !mine.deleted {
                let loser = if std::ptr::eq(winner, *mine) { *theirs } else { *mine };
                let mut copy = Note::new(device_id, loser.text.clone(), now);
                copy.color_index = loser.color_index;
                copy.conflict_copy = true;
                copy.created_at = loser.created_at;
                merged.push(copy);
                conflicts += 1;
            }
        }
    }

    let kept: Vec<Note> = merged
        .into_iter()
        .filter(|note| !note.deleted || now - note.updated_at < tombstone_retention())
        .map(|note| {
            let rev = note.rev.clone();
            note.with_synced_rev(Some(rev))
        })
        .collect();

    let remote_revs: HashMap<&str, &str> = remote.iter().map(|n| (n.id.as_str(), n.rev.as_str())).collect();
    let remote_needs_update = kept.len() != remote.len()
        || kept.iter().any(|note| remote_revs.get(note.id.as_str()) != Some(&note.rev.as_str()));

    MergeResult {
        notes: kept,
        remote_needs_update,
        conflicts,
    }
}

fn newer<'a>(a: &'a Note, b: &'a Note) -> &'a Note {
    match a.updated_at.cmp(&b.updated_at) {
        std::cmp::Ordering::Greater => a,
        std::cmp::Ordering::Less => b,
        std::cmp::Ordering::Equal => {
            if a.rev >= b.rev {
                a
            } else {
                b
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Edit;

    fn t(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-07T10:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(minutes)
    }

    fn synced(text: &str) -> Note {
        let note = Note::new("a", text, t(0));
        note.with_synced_rev(Some(note.rev.clone()))
    }

    fn text(value: &str) -> Edit {
        Edit { text: Some(value.into()), ..Edit::default() }
    }

    fn merge(local: &[Note], remote: &[Note]) -> MergeResult {
        merge_notes(local, remote, "a", t(2))
    }

    fn texts(notes: &[Note]) -> HashSet<String> {
        notes.iter().map(|n| n.text.clone()).collect()
    }

    #[test]
    fn a_new_local_note_is_uploaded() {
        let note = Note::new("a", "hello", t(0));
        let result = merge(&[note.clone()], &[]);
        assert!(result.remote_needs_update);
        assert_eq!(result.notes[0].synced_rev.as_deref(), Some(note.rev.as_str()));
    }

    #[test]
    fn a_new_remote_note_is_downloaded_without_uploading() {
        let note = Note::new("b", "hello", t(0));
        let result = merge(&[], &[note]);
        assert_eq!(result.notes[0].text, "hello");
        assert!(!result.remote_needs_update);
    }

    #[test]
    fn identical_sides_need_no_upload() {
        let note = synced("same");
        let result = merge(&[note.clone()], &[note]);
        assert!(!result.remote_needs_update);
        assert_eq!(result.conflicts, 0);
    }

    #[test]
    fn a_local_edit_replaces_an_unchanged_remote_note() {
        let base = synced("old");
        let edited = base.edited("a", text("new"), t(1));
        let result = merge(&[edited], &[base]);
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].text, "new");
        assert!(result.remote_needs_update);
        assert_eq!(result.conflicts, 0);
    }

    #[test]
    fn a_remote_edit_replaces_an_unchanged_local_note() {
        let base = synced("old");
        let edited = base.edited("b", text("new"), t(1));
        let result = merge(&[base], &[edited]);
        assert_eq!(result.notes[0].text, "new");
        assert!(!result.remote_needs_update);
    }

    #[test]
    fn edits_on_both_sides_keep_both_texts() {
        let base = synced("old");
        let mine = base.edited("a", text("mine"), t(1));
        let theirs = base.edited("b", text("theirs"), t(2));
        let result = merge(&[mine], &[theirs]);

        assert_eq!(result.conflicts, 1);
        assert_eq!(texts(&result.notes), HashSet::from(["mine".into(), "theirs".into()]));
        let winner = result.notes.iter().find(|n| n.id == base.id).unwrap();
        assert_eq!(winner.text, "theirs");
        let copy = result.notes.iter().find(|n| n.id != base.id).unwrap();
        assert_eq!(copy.text, "mine");
        assert!(copy.conflict_copy);
        assert!(result.remote_needs_update);
    }

    #[test]
    fn the_newer_edit_keeps_the_note_when_it_is_the_local_one() {
        let base = synced("old");
        let theirs = base.edited("b", text("theirs"), t(1));
        let mine = base.edited("a", text("mine"), t(2));
        let result = merge(&[mine], &[theirs]);

        assert_eq!(result.conflicts, 1);
        let winner = result.notes.iter().find(|n| n.id == base.id).unwrap();
        assert_eq!(winner.text, "mine");
        let copy = result.notes.iter().find(|n| n.id != base.id).unwrap();
        assert_eq!(copy.text, "theirs");
        assert!(copy.conflict_copy);
    }

    #[test]
    fn a_color_change_on_both_sides_is_not_a_text_conflict() {
        let base = synced("same");
        let mine = base.edited("a", Edit { color_index: Some(1), ..Edit::default() }, t(1));
        let theirs = base.edited("b", Edit { color_index: Some(2), ..Edit::default() }, t(2));
        let result = merge(&[mine], &[theirs]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].color_index, 2);
    }

    #[test]
    fn an_edit_beats_a_deletion_from_the_other_device() {
        let base = synced("old");
        let edited = base.edited("a", text("keep me"), t(1));
        let removed = base.edited("b", Edit { deleted: Some(true), ..Edit::default() }, t(2));
        let result = merge(&[edited], &[removed]);
        assert!(!result.notes[0].deleted);
        assert_eq!(result.notes[0].text, "keep me");
    }

    #[test]
    fn a_deletion_reaches_a_device_that_did_not_change_the_note() {
        let base = synced("old");
        let removed = base.edited("b", Edit { deleted: Some(true), ..Edit::default() }, t(1));
        let result = merge(&[base], &[removed]);
        assert!(result.notes[0].deleted);
    }

    #[test]
    fn old_tombstones_are_dropped() {
        let base = synced("old");
        let removed = base.edited("b", Edit { deleted: Some(true), ..Edit::default() }, t(0));
        let later = t(0) + tombstone_retention() + Duration::days(1);
        let result = merge_notes(&[removed.clone()], &[removed], "a", later);
        assert!(result.notes.is_empty());
        assert!(result.remote_needs_update);
    }

    #[test]
    fn two_devices_converge_after_a_conflict() {
        let base = synced("old");
        let on_a = base.edited("a", text("from a"), t(1));
        let on_b = base.edited("b", text("from b"), t(2));

        let first = merge(&[on_b], &[base]);
        let second = merge(&[on_a], &first.notes);
        let third = merge(&first.notes, &second.notes);

        assert_eq!(texts(&third.notes), HashSet::from(["from a".into(), "from b".into()]));
        assert_eq!(texts(&third.notes), texts(&second.notes));
        assert!(!third.remote_needs_update);
    }
}

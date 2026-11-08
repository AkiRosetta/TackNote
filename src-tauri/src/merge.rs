//! Merges this device's notes with the server's, note by note. A note edited
//! on both sides is never silently lost: the newer edit keeps the note and the
//! older text becomes a conflict copy.
//!
//! Some servers ignore If-Match, so two devices syncing at the same moment can
//! write over each other, and the server may then hold an earlier copy of a
//! note than this device uploaded, or one edited apart from it. `versions`
//! tells these apart from an ordinary edit on another device, so the text
//! survives either way.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};

use crate::model::{Lineage, Note};

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
            merged.push(mine.absorbing(theirs));
            continue;
        }

        let changed_here = Some(mine.rev.as_str()) != mine.synced_rev.as_deref();
        let changed_there = Some(theirs.rev.as_str()) != mine.synced_rev.as_deref();
        let lineage = mine.lineage(theirs);
        if (changed_here && !changed_there) || lineage == Lineage::Older {
            // Older: the server went back to an earlier copy, because another
            // device wrote over our upload; ours goes up again.
            merged.push((*mine).clone());
        } else if !changed_here && lineage != Lineage::Apart {
            merged.push((*theirs).clone());
        } else if mine.deleted != theirs.deleted {
            // An edit beats a deletion, so the text survives.
            let (kept, other) = if mine.deleted { (*theirs, *mine) } else { (*mine, *theirs) };
            merged.push(kept.absorbing(other));
        } else {
            // Edited on both sides, or edited apart after our upload was
            // written over.
            let winner = newer(mine, theirs);
            let other = if std::ptr::eq(winner, *mine) { *theirs } else { *mine };
            merged.push(winner.absorbing(other));
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

    let remote_needs_update = kept.len() != remote.len()
        || kept.iter().any(|note| {
            remote_by_id
                .get(note.id.as_str())
                .is_none_or(|theirs| theirs.rev != note.rev || theirs.versions != note.versions)
        });

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

    /// What device "a" holds once its edit reached the server.
    fn uploaded(note: &Note, edit: Edit, minute: i64) -> Note {
        let edited = note.edited("a", edit, t(minute));
        edited.with_synced_rev(Some(edited.rev.clone()))
    }

    fn deletion() -> Edit {
        Edit { deleted: Some(true), ..Edit::default() }
    }

    #[test]
    fn the_server_going_back_to_an_earlier_copy_keeps_ours() {
        // Another device wrote over our upload with the copy it had read before.
        let base = synced("first");
        let ours = uploaded(&base, text("second"), 1);
        let result = merge(&[ours], &[base.for_upload()]);
        assert_eq!(texts(&result.notes), HashSet::from(["second".into()]));
        assert_eq!(result.conflicts, 0);
        assert!(result.remote_needs_update, "ours goes up again");
    }

    #[test]
    fn a_deletion_written_over_by_an_earlier_copy_stays_deleted() {
        let base = synced("first");
        let ours = uploaded(&base, deletion(), 1);
        let result = merge(&[ours], &[base.for_upload()]);
        assert!(result.notes[0].deleted);
        assert!(result.remote_needs_update);
    }

    #[test]
    fn a_copy_edited_apart_from_our_upload_keeps_both_texts() {
        // Another device edited the copy it read before our upload, then wrote over it.
        let base = synced("first");
        let ours = uploaded(&base, text("ours"), 1);
        let theirs = base.edited("b", text("theirs"), t(1)).for_upload();
        let result = merge(&[ours], &[theirs]);
        assert_eq!(texts(&result.notes), HashSet::from(["ours".into(), "theirs".into()]));
        assert_eq!(result.conflicts, 1);
    }

    #[test]
    fn after_a_conflict_both_devices_go_on_without_another_copy() {
        // Either side may be the newer one.
        for (ours_at, theirs_at) in [(1, 2), (2, 1)] {
            let base = synced("first");
            let ours = uploaded(&base, text("ours"), ours_at);
            let theirs = base.edited("b", text("theirs"), t(theirs_at));
            let on_a = merge(&[ours], &[theirs.for_upload()]);
            let from_a: Vec<Note> = on_a.notes.iter().map(Note::for_upload).collect();
            // Device b still holds the copy it uploaded.
            let on_b = merge_notes(&[theirs.with_synced_rev(Some(theirs.rev.clone()))], &from_a, "b", t(3));
            assert_eq!(on_b.conflicts, 0, "{ours_at} {theirs_at}");
            assert_eq!(texts(&on_b.notes), texts(&on_a.notes));
            // An edit there afterwards is an ordinary edit for device a.
            let from_b: Vec<Note> = on_b
                .notes
                .iter()
                .map(|n| if n.id == base.id { n.edited("b", text("later"), t(4)) } else { n.clone() }.for_upload())
                .collect();
            let back_on_a = merge(&on_a.notes, &from_b);
            assert_eq!(back_on_a.conflicts, 0, "{ours_at} {theirs_at}");
            assert!(texts(&back_on_a.notes).contains("later"));
        }
    }

    #[test]
    fn an_edit_that_beat_a_deletion_goes_up_with_the_deletion_counted_in() {
        // Our deletion was written over by an edit made apart from it.
        let base = synced("text");
        let ours = uploaded(&base, deletion(), 1);
        let theirs = base.edited("b", text("kept"), t(1));
        let on_a = merge(&[ours], &[theirs.for_upload()]);
        assert_eq!(texts(&on_a.notes), HashSet::from(["kept".into()]));
        assert!(!on_a.notes[0].deleted);
        assert!(on_a.remote_needs_update, "so device b learns the deletion was seen");
        let from_a: Vec<Note> = on_a.notes.iter().map(Note::for_upload).collect();
        let on_b = merge_notes(&[theirs.with_synced_rev(Some(theirs.rev.clone()))], &from_a, "b", t(3));
        let later = on_b.notes[0].edited("b", text("later"), t(4)).for_upload();
        let back_on_a = merge(&on_a.notes, &[later]);
        assert_eq!(back_on_a.conflicts, 0);
        assert_eq!(texts(&back_on_a.notes), HashSet::from(["later".into()]));
    }

    #[test]
    fn notes_from_before_versions_merge_as_before() {
        let mut base = synced("first");
        base.versions.clear();
        let mut theirs = base.edited("b", text("theirs"), t(1));
        theirs.versions.clear();
        let result = merge(&[base], &[theirs.for_upload()]);
        assert_eq!(texts(&result.notes), HashSet::from(["theirs".into()]));
        assert_eq!(result.conflicts, 0);
        assert!(!result.remote_needs_update);
    }
}

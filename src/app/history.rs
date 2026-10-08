//! Editor edit history: one inverse operation per action.
//!
//! Each entry names the piece it touched and holds both the value before and
//! the value after, so undo and redo are both derivable from one record. A
//! snapshot stack would be simpler and is wrong here: an edit is identified by
//! piece identity, and a snapshot only restores identity by accident, because
//! undoing "the piece before last" from a snapshot restores whichever piece now
//! sits at that index. An entry keyed on [`PieceUid`] stays correct after any
//! number of later edits.
//!
//! Identity rather than index throughout: placing or removing a piece renumbers
//! indices, so an index captured when a button was built no longer names the
//! piece the player acted on.

use retrackt_format::{PieceInstance, PieceUid, TrackDocument};

/// What one editor action did, in a form that can be walked either way.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// A piece was added at `at`.
    Added { at: usize, piece: PieceInstance },
    /// A piece was taken out of `at`.
    Removed { at: usize, piece: PieceInstance },
    /// One piece was replaced by another version of itself.
    Changed {
        uid: PieceUid,
        before: PieceInstance,
        after: PieceInstance,
    },
    /// The piece list was replaced wholesale: a load, a clear, or a paste.
    Replaced {
        before: Vec<PieceInstance>,
        after: Vec<PieceInstance>,
    },
    /// The document was renamed.
    Renamed { before: String, after: String },
}

impl Edit {
    /// Put the document back the way it was before this action.
    pub fn undo(&self, doc: &mut TrackDocument) {
        match self {
            Edit::Added { at, .. } => {
                if *at < doc.pieces.len() {
                    doc.pieces.remove(*at);
                }
            }
            Edit::Removed { at, piece } => insert_at(doc, *at, *piece),
            Edit::Changed { uid, before, .. } => replace(doc, *uid, *before),
            Edit::Replaced { before, .. } => doc.pieces = before.clone(),
            Edit::Renamed { before, .. } => doc.name = before.clone(),
        }
    }

    /// Put the document back the way it was after this action.
    pub fn redo(&self, doc: &mut TrackDocument) {
        match self {
            Edit::Added { at, piece } => insert_at(doc, *at, *piece),
            Edit::Removed { at, .. } => {
                if *at < doc.pieces.len() {
                    doc.pieces.remove(*at);
                }
            }
            Edit::Changed { uid, after, .. } => replace(doc, *uid, *after),
            Edit::Replaced { after, .. } => doc.pieces = after.clone(),
            Edit::Renamed { after, .. } => doc.name = after.clone(),
        }
    }
}

/// Insertion that clamps rather than panicking: an entry replayed against a
/// document whose length has since changed would otherwise abort the game.
fn insert_at(doc: &mut TrackDocument, at: usize, piece: PieceInstance) {
    let at = at.min(doc.pieces.len());
    doc.pieces.insert(at, piece);
}

/// A no-op rather than a panic when the piece is gone: an entry can outlive the
/// state it was made against if the document is replaced from outside the editor.
fn replace(doc: &mut TrackDocument, uid: PieceUid, piece: PieceInstance) {
    if let Some(slot) = doc.pieces.iter_mut().find(|p| p.uid == uid) {
        *slot = piece;
    }
}

/// Undo and redo stacks, bounded.
#[derive(Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

/// Depth of each stack. Bounded because an edit is a few dozen bytes but a
/// player who holds a key down can make thousands, and nothing here is worth
/// unbounded memory.
const MAX_DEPTH: usize = 200;

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an action that has already been applied to `doc`.
    ///
    /// Clears the redo stack, which is what every editor does and what the
    /// alternative makes impossible: redoing an edit whose intermediate state
    /// has been discarded reproduces a document nobody ever had.
    pub fn push(&mut self, edit: Edit) {
        self.undo.push(edit);
        if self.undo.len() > MAX_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Step one action back, returning the entry so the caller can rebuild
    /// whatever the document fed.
    pub fn undo(&mut self, doc: &mut TrackDocument) -> Option<Edit> {
        let edit = self.undo.pop()?;
        edit.undo(doc);
        self.redo.push(edit.clone());
        Some(edit)
    }

    /// Step one action forward, returning the entry for the same reason.
    pub fn redo(&mut self, doc: &mut TrackDocument) -> Option<Edit> {
        let edit = self.redo.pop()?;
        edit.redo(doc);
        self.undo.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Forget everything. Used when the document is replaced from outside the
    /// editor, where no entry in the stack refers to the current document.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// An entry for a piece edit, or `None` when nothing actually changed.
///
/// A param nudge already at its limit must not push an entry: undoing it would
/// land on the same state and the button would appear to do nothing.
pub fn changed(uid: PieceUid, before: PieceInstance, after: PieceInstance) -> Option<Edit> {
    (before != after).then_some(Edit::Changed {
        uid,
        before,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::{PieceId, demo_track};

    fn piece(uid: u32, z: i16) -> PieceInstance {
        PieceInstance::new(PieceId::Straight, [0, 0, z]).with_uid(PieceUid(uid))
    }

    fn three() -> TrackDocument {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![piece(1, 0), piece(2, 4), piece(3, 8)];
        doc
    }

    fn uids(doc: &TrackDocument) -> Vec<u32> {
        doc.pieces.iter().map(|p| p.uid.0).collect()
    }

    #[test]
    fn undoing_an_add_puts_the_piece_back_where_it_went_in() {
        let mut doc = three();
        let mut h = History::new();
        let at = 1;
        doc.pieces.insert(at, piece(9, 20));
        h.push(Edit::Added {
            at,
            piece: piece(9, 20),
        });

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.pieces, three().pieces);
    }

    #[test]
    fn undoing_a_remove_restores_the_piece_at_its_old_position() {
        let mut doc = three();
        let mut h = History::new();
        let at = 1;
        let removed = doc.pieces.remove(at);
        h.push(Edit::Removed {
            at,
            piece: removed,
        });

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.pieces, three().pieces);
        assert_eq!(
            uids(&doc),
            vec![1, 2, 3],
            "the restored piece must be the one that was taken out"
        );
    }

    #[test]
    fn undoing_a_change_restores_every_field_not_just_the_edited_one() {
        let mut doc = three();
        let mut h = History::new();
        let before = doc.pieces[1];
        doc.pieces[1].yaw = 2;
        doc.pieces[1].params = retrackt_format::PieceParams::default().length(7);
        h.push(changed(before.uid, before, doc.pieces[1]).expect("it changed"));

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.pieces[1], before);
    }

    #[test]
    fn redo_of_a_change_restores_the_edited_value() {
        // The other half of the record: an entry holding only the pre-edit value
        // can undo but cannot redo, and redo would silently do nothing.
        let mut doc = three();
        let mut h = History::new();
        let before = doc.pieces[1];
        let mut after = before;
        after.yaw = 3;
        doc.pieces[1] = after;
        h.push(changed(before.uid, before, after).expect("it changed"));

        h.undo(&mut doc).expect("an edit to undo");
        h.redo(&mut doc).expect("an edit to redo");
        assert_eq!(doc.pieces[1], after, "redo must re-apply, not no-op");
    }

    #[test]
    fn undo_walks_back_through_several_actions() {
        let mut doc = three();
        let mut h = History::new();
        for n in 0..4u32 {
            let at = doc.pieces.len();
            doc.pieces.push(piece(10 + n, 40));
            h.push(Edit::Added {
                at,
                piece: piece(10 + n, 40),
            });
        }
        assert_eq!(doc.pieces.len(), 7);

        for expected in [6, 5, 4, 3] {
            h.undo(&mut doc).expect("an edit to undo");
            assert_eq!(doc.pieces.len(), expected);
        }
        assert_eq!(doc.pieces, three().pieces);
        assert!(!h.can_undo());
    }

    #[test]
    fn redo_replays_what_undo_took_back() {
        let mut doc = three();
        let mut h = History::new();
        let removed = doc.pieces.remove(0);
        h.push(Edit::Removed {
            at: 0,
            piece: removed,
        });

        h.undo(&mut doc).expect("an edit to undo");
        assert!(h.can_redo());
        h.redo(&mut doc).expect("an edit to redo");
        assert_eq!(uids(&doc), vec![2, 3], "the removal must happen again");
    }

    #[test]
    fn redo_of_a_whole_document_swap_reinstates_the_new_pieces() {
        let mut doc = three();
        let mut h = History::new();
        let original = doc.pieces.clone();
        doc.pieces = demo_track().pieces;
        h.push(Edit::Replaced {
            before: original.clone(),
            after: doc.pieces.clone(),
        });

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.pieces, original);
        h.redo(&mut doc).expect("an edit to redo");
        assert_eq!(doc.pieces, demo_track().pieces);
    }

    #[test]
    fn a_rename_undoes_and_redoes() {
        let mut doc = three();
        let mut h = History::new();
        let before = doc.name.clone();
        doc.name = "Renamed".into();
        h.push(Edit::Renamed {
            before: before.clone(),
            after: "Renamed".into(),
        });

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.name, before);
        h.redo(&mut doc).expect("an edit to redo");
        assert_eq!(doc.name, "Renamed");
    }

    #[test]
    fn a_new_action_discards_the_redo_stack() {
        let mut doc = three();
        let mut h = History::new();
        let removed = doc.pieces.remove(0);
        h.push(Edit::Removed {
            at: 0,
            piece: removed,
        });
        h.undo(&mut doc).expect("an edit to undo");
        assert!(h.can_redo());

        doc.pieces.push(piece(9, 30));
        h.push(Edit::Added {
            at: 3,
            piece: piece(9, 30),
        });
        assert!(!h.can_redo(), "redoing would skip the discarded state");
    }

    #[test]
    fn an_entry_names_a_piece_so_later_edits_cannot_redirect_it() {
        // The property snapshots get wrong: the removed piece was at index 1, and
        // a later insertion shifts index 1 onto a different piece. An entry keyed
        // on identity still lands correctly.
        let mut doc = three();
        let mut h = History::new();
        let removed = doc.pieces.remove(1);
        h.push(Edit::Removed {
            at: 1,
            piece: removed,
        });

        doc.pieces.insert(1, piece(7, 60));
        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(uids(&doc), vec![1, 2, 7, 3]);
    }

    #[test]
    fn an_edit_that_changed_nothing_is_not_recorded() {
        let p = piece(1, 0);
        assert_eq!(changed(p.uid, p, p), None);
        let moved = PieceInstance::new(PieceId::CurveRight90, [0, 0, 0]).with_uid(PieceUid(1));
        assert!(changed(p.uid, p, moved).is_some());
    }

    #[test]
    fn an_entry_replayed_against_a_changed_document_does_not_panic() {
        // Recorded at index 3, then the document shrinks to one piece: undoing
        // must clamp rather than index out of the vector.
        let mut doc = three();
        let mut h = History::new();
        doc.pieces.insert(3, piece(9, 90));
        h.push(Edit::Added {
            at: 3,
            piece: piece(9, 90),
        });
        doc.pieces.clear();
        doc.pieces.push(piece(1, 0));

        h.undo(&mut doc).expect("an edit to undo");
        assert_eq!(doc.pieces.len(), 1);

        // And a change whose piece has since been removed.
        let mut doc = three();
        let mut h = History::new();
        let before = doc.pieces[1];
        let mut after = before;
        after.yaw = 1;
        doc.pieces[1] = after;
        h.push(changed(before.uid, before, after).expect("it changed"));
        doc.pieces.clear();
        h.undo(&mut doc).expect("an edit to undo");
        assert!(doc.pieces.is_empty());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut doc = TrackDocument::empty();
        let mut h = History::new();
        for n in 0..(MAX_DEPTH + 50) {
            let at = doc.pieces.len();
            doc.pieces.push(piece(n as u32 + 1, n as i16));
            h.push(Edit::Added {
                at,
                piece: piece(n as u32 + 1, n as i16),
            });
        }
        let mut undone = 0;
        while h.undo(&mut doc).is_some() {
            undone += 1;
        }
        assert_eq!(undone, MAX_DEPTH, "older edits must have been dropped");
    }

    #[test]
    fn clearing_forgets_both_directions() {
        let mut doc = three();
        let mut h = History::new();
        let removed = doc.pieces.remove(0);
        h.push(Edit::Removed {
            at: 0,
            piece: removed,
        });
        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
        assert!(h.undo(&mut doc).is_none());
    }
}

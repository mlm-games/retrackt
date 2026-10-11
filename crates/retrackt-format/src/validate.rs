//! What is wrong with a track, in the order it should be fixed.
//!
//! Every check here is a property of the document alone: no simulation, no
//! renderer, no storage. That is what lets the editor list the problems while
//! the track is still being built, and what lets a refused race say why
//! instead of reporting a spawn that would not settle.

use std::collections::{BTreeMap, BTreeSet};

use crate::demo::occupied_world;
use crate::piece::{PieceId, catalog_by_id};
use crate::track::{MAX_PIECES, PieceUid, TrackDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The track cannot be raced as it stands.
    Error,
    /// The track races, but something in it is unlikely to be what was meant.
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Errors first, then warnings, each group in the order the checks ran.
pub fn validate(doc: &TrackDocument) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    if doc.pieces.len() > MAX_PIECES {
        out.push(Diagnostic::error(format!(
            "{} pieces, over the {MAX_PIECES} limit",
            doc.pieces.len()
        )));
        return out;
    }
    if doc.pieces.is_empty() {
        out.push(Diagnostic::error("No pieces placed."));
        return out;
    }

    let starts: Vec<_> = doc
        .pieces
        .iter()
        .filter(|p| catalog_by_id(p.id).is_start)
        .collect();
    let finishes: Vec<_> = doc
        .pieces
        .iter()
        .filter(|p| catalog_by_id(p.id).is_finish)
        .collect();
    if starts.is_empty() {
        out.push(Diagnostic::error(
            "No Start piece: there is nowhere to begin.",
        ));
    }
    // Several Start pieces are a way of trying a second line out beside the
    // first, and several Finish lines are how a circuit gets alternatives. The
    // run uses one of each, so the choice has to be a rule rather than a
    // refusal: last placed Start, first Finish reached.
    if starts.len() > 1 {
        out.push(Diagnostic::warning(format!(
            "{} Start pieces: the last one placed is used.",
            starts.len()
        )));
    }
    if finishes.is_empty() {
        out.push(Diagnostic::error("No Finish piece: the run has no end."));
    }
    if finishes.len() > 1 {
        out.push(Diagnostic::warning(format!(
            "{} Finish lines: the run ends at the first one reached.",
            finishes.len()
        )));
    }
    // The chain checks below reason about one continuous road from the Start
    // The checks below read the connected stretch of road, so a document with no
    // Start or no Finish at all is reported on its own terms and stops there.
    // Several of either is not: the rules above pick one of each and the road is
    // followed from the one picked.
    if starts.is_empty() || finishes.is_empty() {
        return out;
    }

    // Whether the car can reach a Finish is not decidable from the document,
    // and that is the honest position rather than a gap in the checking. Pieces
    // are placed independently, so there is no single road to walk: the car may
    // fly a gap, take a branch that rejoins further on, or drive a line the
    // editor cannot see the shape of. Every version of this check that tried to
    // answer it by walking ports reported a jump as an unreachable Finish, which
    // is the opposite of what a jump is.
    //
    // What is decidable is that the gates are standing somewhere sensible, and
    // that nothing is obviously dropped in the wrong place. Both are warnings:
    // a track being built is full of pieces that are not joined up yet.
    let gates: Vec<_> = doc
        .pieces
        .iter()
        .filter(|p| catalog_by_id(p.id).is_checkpoint)
        .collect();
    for inst in &gates {
        if !doc.meets_any(inst.uid) {
            out.push(Diagnostic::warning(format!(
                "{} meets no other piece: the car can only reach it by driving \
                 over the same cells twice.",
                inst.id.label()
            )));
        }
    }

    // Pieces built beside the run to try a line later are the normal state of a
    // track being edited, so this is a warning and names the count rather than
    // the pieces.
    let unjoined = doc.unjoined();
    match unjoined.len() {
        0 => {}
        1 => out.push(Diagnostic::warning(
            "1 piece is standing free. It still races, but the car only reaches \
             it by leaving the road and coming back.",
        )),
        n => out.push(Diagnostic::warning(format!(
            "{n} pieces are standing free. They still race, but the car only \
             reaches them by leaving the road and coming back.",
        ))),
    }

    // Two pieces in one cell still build, and the sim resolves the overlap, but
    // the road the player sees is not the road the car drives.
    let mut claimed: BTreeMap<[i16; 3], PieceUid> = BTreeMap::new();
    let mut overlapping: BTreeSet<PieceUid> = BTreeSet::new();
    for inst in &doc.pieces {
        // A loop's ring legitimately shares its entry approach with the road
        // feeding it, so overlapping is judged on road layout only.
        if inst.id == PieceId::Loop {
            continue;
        }
        for cell in occupied_world(doc, inst) {
            if let Some(prev) = claimed.insert(cell, inst.uid) {
                overlapping.insert(prev);
                overlapping.insert(inst.uid);
            }
        }
    }
    if !overlapping.is_empty() {
        let mut names: Vec<&str> = overlapping
            .iter()
            .filter_map(|uid| doc.piece(*uid))
            .map(|p| p.id.label())
            .collect();
        names.sort_unstable();
        names.dedup();
        // A long list stops being readable, and the first few name the cause.
        let shown = names.len().min(4);
        let mut message = format!("Pieces share cells: {}", names[..shown].join(", "));
        if names.len() > shown {
            message.push_str(&format!(" and {} more", names.len() - shown));
        }
        out.push(Diagnostic::warning(format!("{message}.")));
    }

    if !doc.pieces.iter().any(|p| catalog_by_id(p.id).is_checkpoint) {
        out.push(Diagnostic::warning(
            "No checkpoints: the run is one unbroken line to the Finish.",
        ));
    }

    if doc.spawn_xyz.is_some() {
        out.push(Diagnostic::warning(
            "An explicit spawn point overrides where the Start piece puts the car.",
        ));
    }

    out.sort_by_key(|d| d.severity == Severity::Warning);
    out
}

/// True when the track cannot be raced, i.e. at least one error.
pub fn is_unraceable(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceParams;
    use crate::track::PieceInstance;
    use crate::{ChainBuilder, demo_track};

    fn errors(doc: &TrackDocument) -> Vec<String> {
        validate(doc)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message)
            .collect()
    }

    fn warnings(doc: &TrackDocument) -> Vec<String> {
        validate(doc)
            .into_iter()
            .filter(|d| d.severity == Severity::Warning)
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn a_built_in_track_is_raceable() {
        for doc in crate::builtin_tracks() {
            let found = errors(&doc);
            assert!(found.is_empty(), "{}: {found:?}", doc.name);
            assert!(!is_unraceable(&validate(&doc)));
        }
    }

    #[test]
    fn a_built_in_track_reports_no_warnings_either() {
        for doc in crate::builtin_tracks() {
            let found = warnings(&doc);
            assert!(found.is_empty(), "{}: {found:?}", doc.name);
        }
    }

    #[test]
    fn an_empty_document_names_what_is_missing() {
        let doc = TrackDocument::empty();
        let found = errors(&doc);
        assert_eq!(found, vec!["No pieces placed."]);
    }

    #[test]
    fn a_document_without_a_start_or_a_finish_says_so() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1))];
        let found = errors(&doc);
        assert!(found.contains(&"No Start piece: there is nowhere to begin.".to_string()));
        assert!(found.contains(&"No Finish piece: the run has no end.".to_string()));
    }

    #[test]
    fn a_second_start_is_allowed_and_the_last_placed_one_wins() {
        // Two complete lines, built as two chains and pushed into one document:
        // the second is a variant of the first placed well away from it, which
        // is what a second Start is for.
        let mut first = ChainBuilder::new("Line one");
        first.step(PieceId::Start, PieceParams::default().length(2));
        first.step(PieceId::Straight, PieceParams::default().length(2));
        first.step(PieceId::Finish, PieceParams::default().length(2));
        let mut second = ChainBuilder::new("Line two");
        second.step(PieceId::Start, PieceParams::default().length(2));
        second.step(PieceId::Straight, PieceParams::default().length(2));
        second.step(PieceId::Finish, PieceParams::default().length(2));

        let mut doc = first.build();
        for mut piece in second.build().pieces {
            piece.cell = [
                piece.cell[0].saturating_add(200),
                piece.cell[1],
                piece.cell[2].saturating_add(200),
            ];
            // Dropped so `normalize_uids` hands the second line fresh identities
            // above the first line's, which is what "last placed" means here.
            piece.uid = PieceUid(0);
            doc.pieces.push(piece);
        }
        doc.normalize_uids();

        assert!(
            errors(&doc).is_empty(),
            "a second start is a way of trying another line out: {:?}",
            errors(&doc)
        );
        assert!(
            warnings(&doc)
                .iter()
                .any(|m| m.contains("the last one placed is used")),
            "{:?}",
            warnings(&doc)
        );

        let last_placed = doc
            .pieces
            .iter()
            .filter(|p| catalog_by_id(p.id).is_start)
            .max_by_key(|p| p.uid)
            .expect("a start");
        assert_eq!(doc.active_start().map(|p| p.uid), Some(last_placed.uid));
        // The route begins where the car is put, or the run would be timed
        // against gates on a line it never drove.
        assert_eq!(doc.chain().first(), Some(&last_placed.uid));
        let route = doc.chain();
        assert!(
            route.len() >= 3,
            "the second line is a whole line: {route:?}"
        );
        assert!(
            catalog_by_id(doc.piece(*route.last().unwrap()).unwrap().id).is_finish,
            "and it ends at its own Finish: {route:?}"
        );
        // Spawn is the last placed start, which is 200 cells from the first.
        assert!(doc.spawn().0.x > 100.0, "spawn: {:?}", doc.spawn());
    }

    #[test]
    fn a_second_finish_is_allowed_rather_than_refused() {
        let mut b = ChainBuilder::new("Two finishes");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::Straight, PieceParams::default().length(2));
        b.step(PieceId::Finish, PieceParams::default().length(2));
        let mut doc = b.build();
        let uid = doc.next_piece_uid();
        doc.pieces.push(
            PieceInstance::new(PieceId::Finish, [60, 0, 60])
                .with_uid(uid)
                .with_params(PieceParams::default().length(2)),
        );
        doc.normalize_uids();

        assert!(
            errors(&doc).is_empty(),
            "a second finish is how a circuit offers another line home: {:?}",
            errors(&doc)
        );
        assert!(
            warnings(&doc)
                .iter()
                .any(|m| m.contains("the first one reached")),
            "{:?}",
            warnings(&doc)
        );
    }

    /// A Finish standing on its own is reported as free-standing, not refused.
    ///
    /// The test this replaces required a chain that stopped before the Finish to
    /// be an error naming where it stopped. That was the chain model talking: it
    /// assumed the road from the Start is the road the car drives, so a Finish
    /// the walk could not reach was a Finish the car could not reach. With pieces
    /// placed independently that inference is unsound, and the very same shape
    /// is what a jump looks like from the join walk, so the Finish is legal and
    /// what it gets is a warning.
    #[test]
    fn a_finish_standing_apart_from_the_road_is_reported_not_refused() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(2)),
            PieceInstance::new(PieceId::Finish, [40, 0, 40]).with_uid(PieceUid(3)),
        ];
        doc.normalize_uids();

        assert!(
            errors(&doc).is_empty(),
            "a track with a Start and a Finish is raceable whatever joins them: {:?}",
            errors(&doc)
        );
        assert!(
            warnings(&doc).iter().any(|m| m.contains("standing free")),
            "but the free-standing piece is reported: {:?}",
            warnings(&doc)
        );
    }

    /// The one thing free placement cannot make true is a run with no gates at
    /// all: there is nothing to pass, so nothing is timed.
    #[test]
    fn a_track_with_no_finish_says_so() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(2)),
        ];
        doc.normalize_uids();
        let found = errors(&doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("No Finish piece"), "{found:?}");
    }

    /// A gate the car cannot reach is a warning, never a refusal.
    ///
    /// The test this replaces parked a checkpoint at [200, 0, 200] and required
    /// an error, on the reasoning that a gate off the chain is a gate the car
    /// can never pass. That reasoning assumed the chain is the road, and a gate
    /// standing free is now simply a gate the author has not built a road to
    /// yet. It is worth saying out loud; it is not worth refusing the track over.
    #[test]
    fn a_gate_the_car_cannot_reach_warns_without_refusing() {
        let mut doc = demo_track();
        let uid = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::Checkpoint, [200, 0, 200]).with_uid(uid));
        doc.normalize_uids();

        assert!(
            errors(&doc).is_empty(),
            "the track still races: {:?}",
            errors(&doc)
        );
        assert!(
            warnings(&doc)
                .iter()
                .any(|m| m.contains("meets no other piece")),
            "but the stranded gate is reported: {:?}",
            warnings(&doc)
        );
    }

    /// A piece long enough to own a cell that is not one of its own port cells.
    ///
    /// `occupied_cells` deliberately drops port cells, because neighbours share
    /// their join cell and counting it would flag every connected track as
    /// overlapping. A one-cell straight is *only* its two port cells, so it owns
    /// nothing and cannot collide with anything.
    fn long_straight(uid: PieceUid, cell: [i16; 3]) -> PieceInstance {
        PieceInstance::new(PieceId::Straight, cell)
            .with_uid(uid)
            .with_params(PieceParams::default().length(4))
    }

    #[test]
    fn two_pieces_in_one_cell_warn_without_refusing_the_track() {
        let mut doc = demo_track();
        // Two copies of one piece, on top of each other. A curve is the only kind
        // with a road body at all: a straight's centreline is just its two port
        // cells, so it owns nothing and can never collide with anything, and
        // overlap is judged on occupied cells. The fixture has to use a piece that
        // has some or it is not testing anything.
        let victim = *doc
            .pieces
            .iter()
            .find(|p| {
                !crate::geometry::piece_shape(p.id, &p.params, doc.cell_size)
                    .occupied
                    .is_empty()
            })
            .expect("the demo circuit has a piece with a road body");
        let uid = doc.next_piece_uid();
        doc.pieces.push(victim.with_uid(uid));
        doc.normalize_uids();

        let found = validate(&doc);
        assert!(
            errors(&doc).is_empty(),
            "overlap is not fatal: {:?}",
            errors(&doc)
        );
        assert!(
            warnings(&doc).iter().any(|m| m.contains("share cells")),
            "{:?}",
            warnings(&doc)
        );
        assert!(!is_unraceable(&found));
    }

    #[test]
    fn a_detached_branch_is_a_warning_not_an_error() {
        let mut doc = demo_track();
        let uid = doc.next_piece_uid();
        doc.pieces.push(long_straight(uid, [0, 0, 60]));
        doc.normalize_uids();
        assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
        assert!(
            warnings(&doc).iter().any(|m| m.contains("standing free")),
            "{:?}",
            warnings(&doc)
        );
    }

    #[test]
    fn a_track_with_no_checkpoints_warns() {
        let mut b = ChainBuilder::new("No gates");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::Straight, PieceParams::default().length(2));
        b.step(PieceId::Finish, PieceParams::default().length(2));
        let doc = b.build();
        assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
        assert!(
            warnings(&doc).iter().any(|m| m.contains("No checkpoints")),
            "{:?}",
            warnings(&doc)
        );
    }

    #[test]
    fn an_oversized_document_is_refused_before_anything_expensive_runs() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Straight, [0, 0, 0]); MAX_PIECES + 1];
        assert_eq!(validate(&doc).len(), 1);
        assert_eq!(errors(&doc).len(), 1);
    }

    #[test]
    fn errors_are_listed_before_warnings() {
        let mut doc = demo_track();
        let victim = doc.pieces[3].cell;
        let uid = doc.next_piece_uid();
        doc.pieces.push(long_straight(uid, victim));
        doc.pieces
            .push(long_straight(doc.next_piece_uid(), [0, 0, 60]));
        doc.normalize_uids();

        let found = validate(&doc);
        let first_warning = found
            .iter()
            .position(|d| d.severity == Severity::Warning)
            .expect("the fixture produces a warning");
        assert!(
            found[..first_warning]
                .iter()
                .all(|d| d.severity == Severity::Error)
        );
    }
}

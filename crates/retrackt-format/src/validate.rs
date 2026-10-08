//! What is wrong with a track, in the order it should be fixed.
//!
//! Every check here is a property of the document alone: no simulation, no
//! renderer, no storage. That is what lets the editor list the problems while
//! the track is still being built, and what lets a refused race say why
//! instead of reporting a spawn that would not settle.

use std::collections::{BTreeMap, BTreeSet};

use crate::demo::occupied_world;
use crate::geometry::piece_shape;
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
    if starts.len() > 1 {
        out.push(Diagnostic::error(format!(
            "{} Start pieces: a run begins in one place.",
            starts.len()
        )));
    }
    if finishes.is_empty() {
        out.push(Diagnostic::error("No Finish piece: the run has no end."));
    }
    if finishes.len() > 1 {
        out.push(Diagnostic::error(format!(
            "{} Finish pieces: only the last would count.",
            finishes.len()
        )));
    }
    // Every check below reasons about one continuous chain between the two, so
    // a document missing either end is reported on its own terms and stops here.
    if starts.len() != 1 || finishes.len() != 1 {
        return out;
    }

    let route = doc.route();
    if route.is_empty() {
        out.push(Diagnostic::error("The Start piece has no road leaving it."));
        return out;
    }
    let on_route: BTreeSet<PieceUid> = route.iter().copied().collect();
    let last = *route.last().expect("route is non-empty");
    let last_id = doc
        .piece(last)
        .expect("a route uid names a piece in the document")
        .id;
    let reaches_finish = catalog_by_id(last_id).is_finish;
    if !reaches_finish {
        out.push(Diagnostic::error(format!(
            "The route dead-ends on {} before reaching the Finish.",
            last_id.label()
        )));
    }

    if reaches_finish {
        // Only meaningful once the chain is intact: after a break, every
        // remaining gate reads as unreachable and the list would be noise.
        for inst in &doc.pieces {
            let def = catalog_by_id(inst.id);
            if (def.is_checkpoint || def.is_finish) && !on_route.contains(&inst.uid) {
                out.push(Diagnostic::error(format!(
                    "{} is not on the route, so the car can never reach it.",
                    inst.id.label()
                )));
            }
        }
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

    for inst in &doc.pieces {
        let connects = !piece_shape(inst.id, &inst.params, doc.cell_size)
            .ports
            .is_empty();
        if connects && !on_route.contains(&inst.uid) {
            out.push(Diagnostic::warning(format!(
                "{} is off the route: the car never reaches it.",
                inst.id.label()
            )));
        }
    }

    if reaches_finish && !doc.pieces.iter().any(|p| catalog_by_id(p.id).is_checkpoint) {
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
    fn two_start_pieces_are_refused_rather_than_one_silently_winning() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Start, [0, 0, 4]).with_uid(PieceUid(2)),
        ];
        assert!(errors(&doc).contains(&"2 Start pieces: a run begins in one place.".to_string()));
    }

    #[test]
    fn a_chain_that_stops_short_of_the_finish_is_an_error() {
        let mut b = ChainBuilder::new("Half a track");
        b.step(PieceId::Start, PieceParams::default());
        b.step(PieceId::Straight, PieceParams::default());
        let doc = b.build();
        let found = errors(&doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("No Finish piece"), "{found:?}");
    }

    #[test]
    fn a_chain_that_stops_before_the_finish_names_where_it_stopped() {
        // Built by hand: the chain runs out before the finish is attached, which
        // a port-following builder cannot express.
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(2)),
            PieceInstance::new(PieceId::Finish, [40, 0, 40]).with_uid(PieceUid(3)),
        ];
        doc.normalize_uids();
        let found = errors(&doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("dead-ends") && found[0].contains("Straight"),
            "{found:?}"
        );
        // The unreachable Finish is *not* also reported: once the chain is known
        // to be broken, every gate past the break reads as unreachable, and
        // listing them would bury the one fact worth acting on.
        assert!(
            !found.iter().any(|m| m.contains("not on the route")),
            "a broken chain must report the break, not its consequences: {found:?}"
        );
    }

    #[test]
    fn a_gate_off_the_route_is_refused() {
        let mut doc = demo_track();
        // A second finish, parked where no road reaches it.
        let uid = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::Checkpoint, [200, 0, 200]).with_uid(uid));
        doc.normalize_uids();
        assert!(
            errors(&doc).iter().any(|m| m.contains("not on the route")),
            "an unreachable gate must be reported, not ignored"
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
    fn a_dead_end_branch_is_a_warning_not_an_error() {
        let mut doc = demo_track();
        let uid = doc.next_piece_uid();
        doc.pieces.push(long_straight(uid, [0, 0, 60]));
        doc.normalize_uids();
        assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
        assert!(
            warnings(&doc).iter().any(|m| m.contains("off the route")),
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

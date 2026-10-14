//! Editor placement: where the cursor is, what it would drop, and how a dropped
//! piece finds its neighbours.
//!
//! Placement used to be one rule only — chain onto the last piece — which left
//! no way to place anywhere else and no way to see what a placement would do
//! before committing to it. A cursor plus snapping gives both, and keeps the
//! chaining behaviour as the snap case rather than as a separate path.

use retrackt_format::piece::{
    GridDir, PieceId, PieceParams, catalog_by_id, rotate_local_dir, rotate_local_xz,
};
use retrackt_format::{PieceInstance, TrackDocument, piece_shape};

/// Where the next piece will land, and whether it snapped to something.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Anchor cell of the piece to be placed.
    pub anchor: [i16; 3],
    pub yaw: u8,
    /// True when the placement locked onto an open connector port rather than
    /// sitting where the cursor happens to be.
    pub snapped: bool,
}

/// Yaw that points a piece's local `+Z`, its direction of travel, along `dir`.
fn yaw_for_direction(dir: GridDir) -> u8 {
    match dir {
        GridDir::PosZ => 0,
        GridDir::PosX => 1,
        GridDir::NegZ => 2,
        GridDir::NegX => 3,
        // Vertical connectors are not something the editor places along: a piece
        // that climbs or loops handles its own elevation through its shape.
        _ => 0,
    }
}

/// Where `id` would go if dropped at `cell`.
///
/// With snapping on, this searches for an open *exit* port within `reach` cells
/// of the cursor and mates the new piece's entry against it, so building a run
/// of road is one action per piece. With snapping off, or with nothing in
/// reach, the piece lands where the cursor is, at `yaw`. That is the other
/// half of free placement, and the only way to put a piece on an exact cell.
pub fn placement_at(
    doc: &TrackDocument,
    id: PieceId,
    cell: [i16; 3],
    reach: i32,
    yaw: u8,
    snap: bool,
) -> Placement {
    let cell_size = doc.cell_size;
    let entry = piece_shape(id, &PieceParams::default(), cell_size)
        .ports
        .first()
        .map(|p| p.cell)
        .unwrap_or([0, 0, 0]);

    let mate = snap.then(|| open_exit_near(doc, cell, reach)).flatten();
    let (target, yaw) = match mate {
        Some((point, dir)) => (point, yaw_for_direction(dir)),
        None => (cell, yaw % 4),
    };
    let offset = rotate_local_xz(entry, yaw);
    Placement {
        anchor: [
            target[0] - offset[0],
            target[1] - offset[1],
            target[2] - offset[2],
        ],
        yaw,
        snapped: mate.is_some(),
    }
}

/// Every cell of the block a piece fills, turned by `yaw`, nearest cell first.
///
/// The piece's own extent rather than a fixed block size, which is what makes the
/// toggle mean the same thing for every piece: a straight fills its own length,
/// a block part fills its own area. Turned rather than measured again, so a
/// stamped block comes out the shape the piece is being placed at rather than the
/// shape it was authored in.
///
/// Empty for a piece with no occupied cells at all, which the caller reads as
/// "place one", because a decor piece that owns nothing cannot own a block.
pub fn stamp_cells(id: PieceId, doc: &TrackDocument, yaw: u8) -> Vec<[i16; 3]> {
    let shape = piece_shape(id, &PieceParams::default(), doc.cell_size);
    let Some(first) = shape.occupied.first().copied() else {
        return Vec::new();
    };
    let (lo, hi) = shape.occupied.iter().fold((first, first), |(lo, hi), c| {
        (
            [lo[0].min(c[0]), lo[1].min(c[1]), lo[2].min(c[2])],
            [hi[0].max(c[0]), hi[1].max(c[1]), hi[2].max(c[2])],
        )
    });
    let mut cells = Vec::new();
    for y in lo[1]..=hi[1] {
        for z in lo[2]..=hi[2] {
            for x in lo[0]..=hi[0] {
                cells.push(rotate_local_xz([x - lo[0], y - lo[1], z - lo[2]], yaw));
            }
        }
    }
    cells
}

/// Nearest open exit port to `cell`, within `reach` cells in every axis.
///
/// An exit is open when no entry already mates with it head-on. Sorting by
/// distance and then by uid means the choice does not depend on the order
/// pieces sit in the file.
fn open_exit_near(doc: &TrackDocument, cell: [i16; 3], reach: i32) -> Option<([i16; 3], GridDir)> {
    let mut best: Option<((i32, u32), ([i16; 3], GridDir))> = None;
    for (uid, port, index) in doc.all_ports() {
        // Port 0 is the entry, so it faces the way the piece is entered. Exits
        // are the ones a follower has to meet.
        if index != 1 {
            continue;
        }
        let distance = (0..3)
            .map(|a| (i32::from(port.point[a]) - i32::from(cell[a])).abs())
            .max()
            .unwrap_or(i32::MAX);
        if distance > reach {
            continue;
        }
        // `joins` reports `None` for a Finish because the gate is an end, not
        // because its exit is free. Treating that as an open port would let
        // every placement near the finish snap onto the end of the track.
        if doc
            .piece(uid)
            .is_some_and(|inst| catalog_by_id(inst.id).is_finish)
        {
            continue;
        }
        if doc.joins(uid).is_some() {
            // Something already leaves from here.
            continue;
        }
        let key = (distance, uid.0);
        if best.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
            best = Some((key, (port.point, port.outward)));
        }
    }
    best.map(|(_, mate)| mate)
}

/// The cell a piece's exit port sits in, in world grid space.
pub fn exit_cell(doc: &TrackDocument, inst: &PieceInstance) -> [i16; 3] {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    let local = shape.ports.last().map(|p| p.cell).unwrap_or([0, 0, 0]);
    let w = rotate_local_xz(local, inst.yaw);
    [
        inst.cell[0] + w[0],
        inst.cell[1] + w[1],
        inst.cell[2] + w[2],
    ]
}

/// The piece the editor should start from: the last one on the connected
/// stretch of road, so a fresh cursor sits where the next piece would have gone
/// anyway.
///
/// Falls back to the highest uid when nothing connects, which is the piece most
/// recently placed and therefore the one the player was last working on.
pub fn cursor_home(doc: &TrackDocument) -> [i16; 3] {
    let last_connected = doc
        .chain()
        .last()
        .and_then(|uid| doc.piece(*uid))
        .filter(|p| !piece_shape(p.id, &p.params, doc.cell_size).ports.is_empty());
    let piece = last_connected.or_else(|| {
        doc.pieces
            .iter()
            .filter(|p| !piece_shape(p.id, &p.params, doc.cell_size).ports.is_empty())
            .max_by_key(|p| p.uid)
    });
    match piece {
        Some(p) => exit_cell(doc, p),
        None => [0, 0, 0],
    }
}

/// Nudge `cell` by one grid step, staying inside the addressable range.
///
/// Saturating rather than wrapping: a cursor that wrapped from 32767 to -32768
/// would teleport the player to the far corner of the world.
pub fn step_cell(cell: [i16; 3], delta: [i16; 3]) -> [i16; 3] {
    let mut out = [0i16; 3];
    for a in 0..3 {
        out[a] = cell[a].saturating_add(delta[a]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::{PieceUid, demo::ChainBuilder};

    /// Two chained straights. The second is anchored wherever the first one
    /// leaves, rather than at a cell number written by hand, so the fixture
    /// stays connected whatever a straight happens to be sized at.
    fn straight_doc() -> TrackDocument {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1))];
        doc.normalize_uids();
        let first = doc.pieces[0];
        let exit = exit_cell(&doc, &first);
        doc.pieces
            .push(PieceInstance::new(PieceId::Straight, exit).with_uid(doc.next_piece_uid()));
        doc.normalize_uids();
        assert_eq!(
            doc.joins(PieceUid(1)),
            Some(PieceUid(2)),
            "fixture must actually be connected"
        );
        doc
    }

    /// A curve leaving along +X, so a snapped placement must turn to match.
    fn turning_doc() -> TrackDocument {
        let mut b = ChainBuilder::new("turn");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::CurveRight90, PieceParams::default().radius(2));
        b.build()
    }

    #[test]
    fn a_cursor_near_an_open_exit_snaps_the_piece_onto_it() {
        let doc = straight_doc();
        // The open exit is one past the second piece's end.
        let open = exit_cell(&doc, doc.pieces.last().unwrap());
        let near = step_cell(open, [0, 0, 1]);
        let p = placement_at(&doc, PieceId::Straight, near, 1, 0, true);
        assert!(p.snapped, "an exit within reach must be snapped to");
        assert_eq!(p.yaw, 0, "the chain runs along +Z");
        assert_eq!(p.anchor, open, "entry lands on the open exit");
    }

    /// Heading a piece leaves in, read off its exit port.
    fn exit_direction(doc: &TrackDocument, inst: &PieceInstance) -> GridDir {
        piece_shape(inst.id, &inst.params, doc.cell_size)
            .ports
            .last()
            .map(|p| rotate_local_dir(p.outward, inst.yaw))
            .unwrap_or(GridDir::PosZ)
    }

    #[test]
    fn snapping_turns_the_piece_to_face_its_neighbour() {
        let doc = turning_doc();
        let curve = doc
            .pieces
            .iter()
            .find(|p| p.id == PieceId::CurveRight90)
            .expect("the curve is there");
        let open = exit_cell(&doc, curve);
        assert_eq!(
            exit_direction(&doc, curve),
            GridDir::PosX,
            "it leaves along +X"
        );

        let placed = placement_at(&doc, PieceId::Straight, open, 0, 0, true);
        assert!(placed.snapped, "placement: {placed:?}");
        assert_eq!(placed.yaw, 1, "must continue along +X, not +Z");
    }

    #[test]
    fn nothing_in_reach_places_the_entry_on_the_cursor() {
        let doc = straight_doc();
        let p = placement_at(&doc, PieceId::Straight, [40, 0, 40], 1, 0, true);
        assert!(!p.snapped, "a cursor far from the road must not snap");
        assert_eq!(p.anchor, [40, 0, 40]);
        assert_eq!(p.yaw, 0);
    }

    #[test]
    fn a_snapped_piece_lands_its_entry_on_the_open_exit() {
        // The general property behind the anchor arithmetic: whatever the piece,
        // its entry port is what meets the port it snapped to.
        for id in [
            PieceId::Straight,
            PieceId::CurveRight90,
            PieceId::Checkpoint,
            PieceId::RampUp,
            PieceId::Loop,
        ] {
            let doc = turning_doc();
            let curve = doc
                .pieces
                .iter()
                .find(|p| p.id == PieceId::CurveRight90)
                .expect("the curve is there");
            let open = exit_cell(&doc, curve);
            let p = placement_at(&doc, id, open, 0, 0, true);
            let shape = piece_shape(id, &PieceParams::default(), doc.cell_size);
            let entry = rotate_local_xz(shape.ports[0].cell, p.yaw);
            assert_eq!(
                [
                    p.anchor[0] + entry[0],
                    p.anchor[1] + entry[1],
                    p.anchor[2] + entry[2]
                ],
                open,
                "{id:?} must meet the port it snapped to"
            );
        }
    }

    #[test]
    fn an_exit_already_taken_is_not_snapped_to() {
        let doc = straight_doc();
        // The first piece's exit is already met by the second piece's entry.
        let first_exit = exit_cell(&doc, &doc.pieces[0]);
        let p = placement_at(&doc, PieceId::Straight, first_exit, 0, 0, true);
        assert!(!p.snapped, "must not double up on a connected port");
    }

    #[test]
    fn the_nearest_open_exit_wins_over_a_further_one() {
        // Three chained straights: open exits are the second's and the third's.
        let mut doc = straight_doc();
        let third = PieceInstance::new(
            PieceId::Straight,
            exit_cell(&doc, doc.pieces.last().unwrap()),
        )
        .with_uid(doc.next_piece_uid());
        doc.pieces.push(third);
        doc.normalize_uids();
        // Two cells past the third piece: the nearest open exit is the third's.
        let target = exit_cell(&doc, doc.pieces.last().unwrap());
        let near = step_cell(target, [0, 0, 1]);
        let p = placement_at(&doc, PieceId::Straight, near, 2, 0, true);
        assert!(p.snapped, "placement: {p:?}");
        assert_eq!(p.anchor, target, "the nearest open exit is the one to join");
    }

    #[test]
    fn the_cursor_starts_where_the_next_piece_would_have_gone() {
        let mut b = ChainBuilder::new("home");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::Straight, PieceParams::default().length(2));
        let doc = b.build();

        let home = cursor_home(&doc);
        let last = *doc.chain().last().expect("a route");
        assert_eq!(home, exit_cell(&doc, doc.piece(last).unwrap()));
        // And placing there snaps, so the two paths agree.
        assert!(
            placement_at(&doc, PieceId::Checkpoint, home, 1, 0, true).snapped,
            "the home cell must be a snapping position"
        );
    }

    #[test]
    fn an_unconnected_document_homes_on_the_newest_piece() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [50, 0, 50]).with_uid(PieceUid(2)),
        ];
        doc.normalize_uids();
        let last = doc.pieces.last().unwrap();
        assert_eq!(cursor_home(&doc), exit_cell(&doc, last));
    }

    #[test]
    fn an_empty_document_homes_on_the_origin() {
        assert_eq!(cursor_home(&TrackDocument::empty()), [0, 0, 0]);
    }

    #[test]
    fn cursor_steps_saturate_instead_of_wrapping() {
        assert_eq!(step_cell([0, 0, 0], [0, 0, 1]), [0, 0, 1]);
        assert_eq!(step_cell([i16::MAX, 0, 0], [1, 0, 0]), [i16::MAX, 0, 0]);
        assert_eq!(step_cell([i16::MIN, 0, 0], [-1, 0, 0]), [i16::MIN, 0, 0]);
    }

    #[test]
    fn a_piece_with_no_ports_lands_on_the_cursor_itself() {
        let doc = straight_doc();
        let p = placement_at(&doc, PieceId::Wall, [7, 0, 7], 4, 0, true);
        assert_eq!(p.anchor, [7, 0, 7], "a prop has no entry to align");
        assert!(!p.snapped);
    }

    /// Snapping off is what makes free placement free: the cursor is the anchor,
    /// not a reach for a nearby connector.
    #[test]
    fn snapping_off_places_the_piece_exactly_on_the_cursor() {
        let mut b = ChainBuilder::new("open");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::Straight, PieceParams::default().length(2));
        let doc = b.build();

        // With snapping on, the tail of the road is the snap target.
        let snapped = placement_at(&doc, PieceId::Straight, [0, 0, 4], 1, 0, true);
        assert!(snapped.snapped, "there is an open exit to join");
        assert_eq!(snapped.anchor, [0, 0, 4]);

        // With it off, the same cursor and the same world give somewhere else.
        let free = placement_at(&doc, PieceId::Straight, [0, 0, 4], 1, 2, false);
        assert!(!free.snapped, "snapping off must not snap");
        assert_eq!(free.anchor, [0, 0, 4], "the cursor is the anchor");
        assert_eq!(free.yaw, 2, "and the armed heading is the one used");

        // A piece whose entry port is offset from its anchor is still anchored on
        // the cursor, so the piece lands where the pointer is rather than beside it.
        let ramp = placement_at(&doc, PieceId::RampUp, [0, 0, 9], 1, 0, false);
        assert!(!ramp.snapped);
        assert_eq!(ramp.anchor, [0, 0, 9]);
    }

    #[test]
    fn an_armed_heading_is_used_when_nothing_is_joined_to() {
        let mut b = ChainBuilder::new("open");
        b.step(PieceId::Start, PieceParams::default().length(2));
        let doc = b.build();

        // Far from anything, with snapping off: the heading is the armed one.
        assert_eq!(
            placement_at(&doc, PieceId::Straight, [20, 0, 20], 1, 0, false).yaw,
            0
        );
        assert_eq!(
            placement_at(&doc, PieceId::Straight, [20, 0, 20], 1, 1, false).yaw,
            1
        );
        assert_eq!(
            placement_at(&doc, PieceId::Straight, [20, 0, 20], 1, 3, false).yaw,
            3
        );
        // And it wraps rather than panicking.
        assert_eq!(
            placement_at(&doc, PieceId::Straight, [20, 0, 20], 1, 5, false).yaw,
            1
        );
    }

    /// A join decides the heading, because a piece head-on against its neighbour
    /// cannot also be facing somewhere else. That is the one thing that overrides
    /// the armed heading, and it only happens while snapping is on.
    #[test]
    fn a_snapped_join_overrides_the_armed_heading() {
        let doc = turning_doc();
        let curve = doc
            .pieces
            .iter()
            .find(|p| p.id == PieceId::CurveRight90)
            .expect("the curve is there");
        let open = exit_cell(&doc, curve);

        let placed = placement_at(&doc, PieceId::Straight, open, 0, 3, true);
        assert!(placed.snapped, "it found the join");
        assert_eq!(placed.yaw, 1, "the join's heading wins over the armed one");
    }

    #[test]
    fn exit_direction_follows_the_piece_own_yaw() {
        let doc = straight_doc();
        let mut inst = doc.pieces[0];
        assert_eq!(exit_direction(&doc, &inst), GridDir::PosZ);
        inst.yaw = 1;
        assert_eq!(exit_direction(&doc, &inst), GridDir::PosX);
    }

    #[test]
    fn a_closed_circuit_offers_nowhere_to_snap_to() {
        // The demo circuit ends in a Finish, and by the rule in `open_exit_near` a
        // Finish is not an open port — so on a closed circuit every exit is spoken
        // for and a placement must report that it is not snapping. Claiming a snap
        // it did not get would put the piece somewhere the player did not choose.
        let doc = retrackt_format::demo_track();
        let placement = placement_at(&doc, PieceId::Checkpoint, cursor_home(&doc), 1, 0, true);
        assert!(
            !placement.snapped,
            "a closed circuit has no open exit, so nothing may snap"
        );
    }

    #[test]
    fn the_tail_of_an_open_route_is_somewhere_to_build() {
        // The demo circuit with its Finish taken off the end: the piece that used
        // to feed it now has nothing leaving from it, which is exactly the open
        // port the editor homes the cursor on.
        let mut doc = retrackt_format::demo_track();
        let finish = doc
            .pieces
            .iter()
            .position(|p| catalog_by_id(p.id).is_finish)
            .expect("the demo circuit has a finish");
        doc.pieces.remove(finish);

        let placement = placement_at(&doc, PieceId::Checkpoint, cursor_home(&doc), 1, 0, true);
        assert!(
            placement.snapped,
            "the tail of an open route is somewhere to build"
        );
    }

    #[test]
    fn a_finish_is_not_treated_as_an_open_port() {
        // `next_piece` ends the route at a Finish, so testing openness by it alone
        // would offer the end of the track as somewhere to build.
        let doc = retrackt_format::demo_track();
        let finish = doc
            .pieces
            .iter()
            .find(|p| catalog_by_id(p.id).is_finish)
            .expect("the demo circuit has a finish");
        let at_finish = exit_cell(&doc, finish);
        assert_eq!(doc.joins(finish.uid), None, "the route ends there");

        let p = placement_at(&doc, PieceId::Straight, at_finish, 0, 0, true);
        assert!(
            !p.snapped || p.anchor != at_finish,
            "a piece must not be dropped onto the Finish gate"
        );
    }

    /// A block is filled by the piece's own extent, so the toggle means the same
    /// thing for every piece rather than stamping a fixed square of whatever
    /// happens to be armed — and it is the same block at every heading, only
    /// turned, so a block does not change size when the piece is rotated.
    #[test]
    fn a_stamp_is_the_pieces_own_extent_turned_by_its_heading() {
        let doc = retrackt_format::demo_track();
        let mut checked = 0;
        for def in catalog() {
            let shape = piece_shape(def.id, &PieceParams::default(), doc.cell_size);
            if shape.occupied.is_empty() {
                continue;
            }
            let lo = shape.occupied.iter().fold([i16::MAX; 3], |lo, c| {
                [lo[0].min(c[0]), lo[1].min(c[1]), lo[2].min(c[2])]
            });
            let hi = shape.occupied.iter().fold([i16::MIN; 3], |hi, c| {
                [hi[0].max(c[0]), hi[1].max(c[1]), hi[2].max(c[2])]
            });
            let extent = (hi[0] - lo[0] + 1) as usize
                * (hi[1] - lo[1] + 1) as usize
                * (hi[2] - lo[2] + 1) as usize;
            for yaw in 0..4u8 {
                assert_eq!(
                    stamp_cells(def.id, &doc, yaw).len(),
                    extent,
                    "{} at yaw {yaw} filled a number of cells other than the {extent} its own extent spans",
                    def.id.label()
                );
            }
            checked += 1;
        }
        assert!(checked > 8, "only {checked} pieces were checked");
    }

    /// A piece that owns no cells owns no block, so filling one places nothing
    /// rather than placing it once and calling that a block.
    #[test]
    fn a_piece_with_no_occupied_cells_fills_nothing() {
        let doc = retrackt_format::demo_track();
        let mut checked = 0;
        for def in catalog() {
            let shape = piece_shape(def.id, &PieceParams::default(), doc.cell_size);
            if shape.occupied.is_empty() {
                assert!(
                    stamp_cells(def.id, &doc, 0).is_empty(),
                    "{} owns no cells and should fill no block",
                    def.id.label()
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "the catalogue has no cell-less piece to check");
    }
}

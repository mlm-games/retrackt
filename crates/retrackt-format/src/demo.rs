//! Build a demo circuit.
//!
//! Pieces are placed by *following ports* rather than by hard-coded cells:
//! each new piece is anchored wherever the previous piece's exit landed.
//! Placing by hand is what produces off-by-one-cell chains that look right
//! and silently fail to connect.

use crate::FORMAT_VERSION;
use crate::geometry::piece_shape;
use crate::piece::{GridDir, PieceId, PieceParams, rotate_local_dir, rotate_local_xz};
use crate::track::{PieceInstance, TrackDocument};

/// Places pieces into a document, chaining each one onto the previous.
pub struct ChainBuilder {
    doc: TrackDocument,
    cursor: [i16; 3],
    yaw: u8,
}

impl ChainBuilder {
    pub fn new(name: &str) -> Self {
        let mut doc = TrackDocument::empty();
        doc.name = name.into();
        doc.author = "builtin".into();
        Self {
            doc,
            cursor: [0, 0, 0],
            yaw: 0,
        }
    }

    pub fn cursor(&self) -> [i16; 3] {
        self.cursor
    }

    /// Append a piece whose *entry* port lands exactly on the current cursor,
    /// then advance to its exit; anchoring on the origin splits asymmetric pieces.
    pub fn step(&mut self, id: PieceId, params: PieceParams) {
        let shape = piece_shape(id, &params, self.doc.cell_size);
        let (Some(entry_local), Some(exit_local)) = (shape.ports.first(), shape.ports.last())
        else {
            // Props such as walls have no ports; place them but do not chain.
            let uid = self.doc.next_piece_uid();
            self.doc.pieces.push(
                PieceInstance::new(id, self.cursor)
                    .with_uid(uid)
                    .with_yaw(self.yaw)
                    .with_params(params),
            );
            return;
        };

        let entry_world = rotate_local_xz(entry_local.cell, self.yaw);
        let exit_world = rotate_local_xz(exit_local.cell, self.yaw);
        let exit_dir = rotate_local_dir(exit_local.outward, self.yaw);

        // Anchor so that entry + anchor == cursor.
        let anchor = [
            self.cursor[0] - entry_world[0],
            self.cursor[1] - entry_world[1],
            self.cursor[2] - entry_world[2],
        ];

        let uid = self.doc.next_piece_uid();
        self.doc.pieces.push(
            PieceInstance::new(id, anchor)
                .with_uid(uid)
                .with_yaw(self.yaw)
                .with_params(params),
        );

        self.cursor = [
            anchor[0] + exit_world[0],
            anchor[1] + exit_world[1],
            anchor[2] + exit_world[2],
        ];
        // The next piece keeps travelling the same way.
        self.yaw = yaw_for_direction(exit_dir);
    }

    pub fn build(mut self) -> TrackDocument {
        self.doc.normalize_uids();
        self.doc.format = FORMAT_VERSION;
        self.doc
    }

    /// Place a piece at an explicit cell and heading, bypassing the cursor:
    /// a loop exits where it entered, so its follower would land on its anchor.
    fn step_at(&mut self, cell: [i16; 3], yaw: u8, id: PieceId, params: PieceParams) {
        self.cursor = cell;
        self.yaw = yaw;
        self.step(id, params);
    }
}

/// Yaw that points a piece's local `+Z` (its direction of travel) along `dir`
/// — the exit direction itself, un-inverted: inverting folds every track back.
fn yaw_for_direction(dir: GridDir) -> u8 {
    match dir {
        GridDir::PosZ => 0,
        GridDir::PosX => 1,
        GridDir::NegZ => 2,
        GridDir::NegX => 3,
        // Vertical connectors are not used by the built-in tracks.
        _ => 0,
    }
}

/// A closed circuit: two long straights joined by right-hand curves, with a
/// boost pad, checkpoints and a finish; exercises zone and yawed pieces.
pub fn demo_track() -> TrackDocument {
    use PieceId::*;

    let mut b = ChainBuilder::new("Demo Circuit");
    b.step(Start, PieceParams::default().length(2));
    b.step(Straight, PieceParams::default());
    b.step(Boost, PieceParams::default());
    b.step(Checkpoint, PieceParams::default());
    b.step(Straight, PieceParams::default().length(2));
    b.step(CurveRight90, PieceParams::default().radius(2));
    b.step(Straight, PieceParams::default().length(2));
    b.step(Boost, PieceParams::default());
    b.step(Checkpoint, PieceParams::default());
    b.step(Straight, PieceParams::default());
    b.step(CurveRight90, PieceParams::default().radius(2));
    b.step(Straight, PieceParams::default().length(2));
    b.step(Checkpoint, PieceParams::default());
    b.step(CurveRight90, PieceParams::default().radius(2));
    b.step(Straight, PieceParams::default().length(2));
    b.step(Finish, PieceParams::default().length(2));
    b.build()
}

/// A stunt track: climb, bank, descend, boost, then a loop as the finale.
pub fn stunt_track() -> TrackDocument {
    use PieceId::*;

    let mut b = ChainBuilder::new("Stunt Time Trial");
    b.step(Start, PieceParams::default().length(2));
    b.step(Straight, PieceParams::default().length(2));
    b.step(Boost, PieceParams::default());
    b.step(RampUp, PieceParams::default().length(2));
    b.step(Straight, PieceParams::default());
    b.step(BankRight, PieceParams::default().bank(25));
    b.step(BankRight, PieceParams::default().bank(-25));
    b.step(Boost, PieceParams::default());
    b.step(RampDown, PieceParams::default().length(2));
    b.step(Checkpoint, PieceParams::default());
    b.step(Straight, PieceParams::default());

    // A loop needs real entry speed, so it gets a long boost run-up.
    b.step(Boost, PieceParams::default().length(2));

    // The loop is the finale: it exits on its own anchor cell, so a follower
    // would sit inside its reserved cylinder and drive through its far wall.
    b.step(Loop, PieceParams::default().radius(3));
    b.step_at(b.cursor, 0, Finish, PieceParams::default());
    b.build()
}

/// All built-in tracks, in menu order.
pub fn builtin_tracks() -> Vec<TrackDocument> {
    vec![demo_track(), stunt_track()]
}

/// World-space centre of a piece's drive surface, for spawning and HUD maps.
pub fn piece_centre(doc: &TrackDocument, inst: &PieceInstance) -> glam::Vec3 {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    let mid = shape.center.pos.len() / 2;
    let local = shape
        .center
        .pos
        .get(mid)
        .copied()
        .unwrap_or(glam::Vec3::ZERO);
    let origin = glam::Vec3::from(doc.cell_origin(inst.cell));
    let yaw = inst.yaw as f32 * std::f32::consts::FRAC_PI_2;
    origin + glam::Quat::from_rotation_y(yaw) * local
}

/// Local grid cells a placed piece occupies in world space.
pub fn occupied_world(doc: &TrackDocument, inst: &PieceInstance) -> Vec<[i16; 3]> {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    shape
        .occupied
        .iter()
        .map(|c| {
            let w = rotate_local_xz(*c, inst.yaw);
            [
                inst.cell[0] + w[0],
                inst.cell[1] + w[1],
                inst.cell[2] + w[2],
            ]
        })
        .collect()
}

/// World grid cells a placed piece reserves, in world space.
pub fn reserved_world(doc: &TrackDocument, inst: &PieceInstance) -> Vec<[i16; 3]> {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    shape
        .reserve_cells()
        .map(|c| {
            let w = rotate_local_xz(c, inst.yaw);
            [
                inst.cell[0] + w[0],
                inst.cell[1] + w[1],
                inst.cell[2] + w[2],
            ]
        })
        .collect()
}

/// World cell of a piece's exit port.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::PieceUid;

    /// Walk the whole route and require every step to be a real head-on join;
    /// Start's entry and Finish's exit stay open, and so may dead-end branches.
    fn assert_fully_connected(doc: &TrackDocument) {
        let route = doc.route();
        assert!(
            route.len() >= 6,
            "{}: route is too short: {:?}",
            doc.name,
            route
        );

        for (i, uid) in route.iter().enumerate() {
            let inst = doc.piece(*uid).expect("route piece exists");
            let is_finish = crate::piece::catalog_by_id(inst.id).is_finish;
            match doc.next_piece(*uid) {
                Some(_) => continue,
                None => assert!(
                    is_finish,
                    "{}: piece {:?} (#{i}) is a dead end mid-route",
                    doc.name, uid.0
                ),
            }
        }

        let last = *route.last().expect("non-empty route");
        assert!(
            crate::piece::catalog_by_id(doc.piece(last).unwrap().id).is_finish,
            "{}: route must end on the finish",
            doc.name
        );
        assert!(
            route.iter().all(|u| doc.next_piece(*u) != Some(route[0])),
            "{}: route must not loop back on itself",
            doc.name
        );
    }

    #[test]
    fn demo_circuit_is_fully_connected() {
        let doc = demo_track();
        assert_fully_connected(&doc);
    }

    #[test]
    fn stunt_track_is_fully_connected() {
        let doc = stunt_track();
        assert_fully_connected(&doc);
    }

    #[test]
    fn builtin_tracks_route_from_start_to_finish() {
        for doc in builtin_tracks() {
            let route = doc.route();

            let first = doc.piece(route[0]).expect("route start exists");
            assert!(crate::piece::catalog_by_id(first.id).is_start);
            let last = doc.piece(route[route.len() - 1]).expect("route end exists");
            assert!(
                crate::piece::catalog_by_id(last.id).is_finish,
                "{}: route ends on {:?}",
                doc.name,
                last.id
            );
        }
    }

    #[test]
    fn no_two_builtin_pieces_share_an_occupied_cell() {
        for doc in builtin_tracks() {
            let mut seen: std::collections::BTreeMap<[i16; 3], PieceUid> = Default::default();
            for inst in &doc.pieces {
                // A loop's ring legitimately shares its entry/exit approach cells
                // with the feeding road; excluding loops tests road layout only.
                if inst.id == PieceId::Loop {
                    continue;
                }
                for cell in occupied_world(&doc, inst) {
                    if let Some(prev) = seen.insert(cell, inst.uid) {
                        panic!(
                            "{}: {:?} and {:?} both occupy {:?}",
                            doc.name, prev.0, inst.uid.0, cell
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_loop_exits_on_its_own_anchor_cell() {
        // The loop returns to where it started: anything anchored at that cell
        // overlaps it, which is why the stunt track ends on the loop.
        let shape = piece_shape(PieceId::Loop, &PieceParams::default().radius(3), 4.0);
        assert_eq!(shape.ports[0].cell, shape.ports[1].cell);
        assert_eq!(shape.ports[0].outward, GridDir::NegZ);
        assert_eq!(shape.ports[1].outward, GridDir::PosZ);
    }

    #[test]
    fn chain_following_lands_pieces_on_consecutive_cells() {
        let doc = demo_track();
        let route = doc.route();
        for pair in route.windows(2) {
            let a = doc.piece(pair[0]).unwrap();
            let b = doc.piece(pair[1]).unwrap();
            assert_eq!(
                exit_cell(&doc, a),
                b.cell,
                "{} does not feed {:?}",
                a.uid.0,
                b.uid.0
            );
        }
    }

    #[test]
    fn stunt_track_contains_the_stunt_pieces() {
        let doc = stunt_track();
        for want in [
            PieceId::Loop,
            PieceId::RampUp,
            PieceId::RampDown,
            PieceId::BankRight,
            PieceId::Boost,
        ] {
            assert!(
                doc.pieces.iter().any(|p| p.id == want),
                "stunt track is missing {want:?}"
            );
        }
    }
}

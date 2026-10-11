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

    /// Move the cursor `cells` along the current heading without placing
    /// anything, leaving that much road missing.
    ///
    /// This is what makes a jump: place a ramp, leave a gap, place the landing.
    /// The gap is the absence of road rather than a piece that draws one, so it
    /// costs nothing to author and the car is airborne over it because there is
    /// genuinely nothing under the wheels.
    pub fn gap(&mut self, cells: i16) {
        let dir = rotate_local_xz([0, 0, cells], self.yaw);
        self.cursor = [
            self.cursor[0].saturating_add(dir[0]),
            self.cursor[1].saturating_add(dir[1]),
            self.cursor[2].saturating_add(dir[2]),
        ];
    }

    /// Drop the cursor `cells` lower without placing anything.
    ///
    /// The second half of a jump, and the half that is easy to forget. A launch
    /// is level where the ramp left it, and level launches only ever arrive
    /// lower, so the landing has to sit below the lip. `gap` alone advances the
    /// cursor forward and leaves its height alone, which puts the landing level
    /// with the takeoff and makes the gap impossible for any car at any speed.
    pub fn drop(&mut self, cells: i16) {
        self.cursor[1] = self.cursor[1].saturating_sub(cells);
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
    // A bank each way. These are two pieces because they lean opposite ways,
    // which is the whole difference between them; the same piece twice with
    // opposite bank angles is what this used to be, back when BankLeft was a
    // name for BankRight.
    b.step(BankRight, PieceParams::default().bank(25));
    b.step(BankLeft, PieceParams::default().bank(25));
    b.step(Boost, PieceParams::default());
    b.step(RampDown, PieceParams::default().length(2));
    b.step(Checkpoint, PieceParams::default());
    b.step(Straight, PieceParams::default());

    // A chicane in one piece, then a jump built the way free placement builds
    // one: a ramp up to launch off, and then nothing at all for a few cells
    // before the landing. The gap is the absence of road, not a piece that
    // draws one.
    b.step(SCurve, PieceParams::default().radius(2));
    b.step(Straight, PieceParams::default());
    b.step(RampUp, PieceParams::default().length(2).slope(45));
    b.step(Straight, PieceParams::default());
    // Leave air, then pick the road up *below* where the car left it. The drop
    // is the jump: a level launch across a gap always arrives lower than it
    // started, which is a consequence of gravity and not of how fast the car is
    // going. Building the landing at the lip's height leaves the car falling
    // through the road before it gets there.
    b.gap(3);
    b.drop(2);
    b.step(Straight, PieceParams::default().length(2));

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
    use crate::geometry::from_grid;
    use crate::piece::catalog_by_id;
    use crate::track::PieceUid;

    /// Walk the connected stretch and require every step within it to be a real
    /// head-on join; Start's entry and Finish's exit stay open.
    ///
    /// The stretch may be shorter than the track: a built-in circuit with a jump
    /// in it is two runs of road with a gap between, which is what a gap is for.
    /// So the chain is walked to wherever it stops, and the pieces past the break
    /// are checked the only way they can be, by confirming the run as a whole
    /// still begins at a Start and ends at a Finish.
    fn assert_well_formed(doc: &TrackDocument) {
        let connected = doc.chain();
        assert!(
            connected.len() >= 6,
            "{}: connected road is too short: {:?}",
            doc.name,
            connected
        );

        for (i, uid) in connected.iter().enumerate() {
            let inst = doc.piece(*uid).expect("chain piece exists");
            let is_finish = crate::piece::catalog_by_id(inst.id).is_finish;
            match doc.joins(*uid) {
                Some(_) => continue,
                None => assert!(
                    is_finish || doc.chain_stops_at_air(*uid),
                    "{}: piece {:?} (#{i}) is a dead end in mid air",
                    doc.name,
                    uid.0
                ),
            }
        }

        assert!(
            connected
                .iter()
                .all(|u| doc.joins(*u) != Some(connected[0])),
            "{}: the road must not loop back on itself",
            doc.name
        );

        // The run as a whole: something to start from and something to end at.
        // A jump splits the road in two without making either half the track.
        let starts = doc
            .pieces
            .iter()
            .filter(|p| crate::piece::catalog_by_id(p.id).is_start);
        let finishes = doc
            .pieces
            .iter()
            .filter(|p| crate::piece::catalog_by_id(p.id).is_finish);
        assert_eq!(starts.count(), 1, "{}: exactly one Start", doc.name);
        assert_eq!(finishes.count(), 1, "{}: exactly one Finish", doc.name);
    }

    #[test]
    fn demo_circuit_is_one_unbroken_road() {
        let doc = demo_track();
        assert_well_formed(&doc);
        // With no gap in it, the whole circuit is one connected run.
        assert_eq!(
            doc.chain().len(),
            doc.pieces.len(),
            "{}: every piece is on the connected road",
            doc.name
        );
    }

    #[test]
    fn stunt_track_is_well_formed_across_its_jump() {
        let doc = stunt_track();
        assert_well_formed(&doc);
        // The jump is the interesting part: the connected road stops short of
        // the whole track, and the piece it stops on ends in open air rather
        // than against a neighbour that failed to mate.
        let chain = doc.chain();
        assert!(
            chain.len() < doc.pieces.len(),
            "{}: the jump must break the connected road",
            doc.name
        );
        let last = *chain.last().expect("non-empty");
        assert!(
            doc.chain_stops_at_air(last),
            "{}: the break must be a gap the car flies, not a missing piece",
            doc.name
        );
    }

    #[test]
    fn builtin_tracks_begin_at_a_start_and_end_at_a_finish() {
        for doc in builtin_tracks() {
            let order = doc.race_order();
            let first = doc.piece(order[0]).expect("a piece");
            let last = doc.piece(order[order.len() - 1]).expect("a piece");
            assert!(
                crate::piece::catalog_by_id(first.id).is_start,
                "{}: the run begins at {:?}",
                doc.name,
                first.id
            );
            assert!(
                crate::piece::catalog_by_id(last.id).is_finish,
                "{}: the run ends at {:?}",
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
        let route = doc.chain();
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
            PieceId::BankLeft,
            PieceId::SCurve,
            PieceId::Boost,
        ] {
            assert!(
                doc.pieces.iter().any(|p| p.id == want),
                "stunt track is missing {want:?}"
            );
        }
    }

    /// The jump is a gap in the road, and the landing sits below the takeoff.
    ///
    /// The rule that makes a jump flyable at all: a car leaves a level lip at
    /// the height the ramp put it, and level launches only ever arrive lower. A
    /// landing built level with the lip is a gap no car can cross at any speed,
    /// and that mistake is invisible in the editor because nothing is placed
    /// there at all, which is exactly what makes it worth a test.
    #[test]
    fn the_jump_lands_lower_than_it_takes_off() {
        let doc = stunt_track();
        // The ramp is the takeoff and the piece after the gap the landing.
        // The jump's ramp is the one that climbs the highest. The track has two:
        // a gentle climb early on, and the launch ramp at 45 degrees.
        let ramp = doc
            .pieces
            .iter()
            .filter(|p| p.id == PieceId::RampUp)
            .max_by_key(|p| p.params.slope_deg.unwrap_or(0))
            .expect("the jump's launch ramp");
        let takeoff = ramp;
        // The landing is the first piece anchored past the lip's exit. The flat
        // run sitting on top of the ramp is not it: that piece is part of the
        // takeoff, and comparing the lip against it is a test that can only ever
        // pass.
        let lip_exit_z = piece_shape(takeoff.id, &takeoff.params, doc.cell_size)
            .ports
            .last()
            .map(|port| {
                let w = rotate_local_xz(port.cell, takeoff.yaw);
                takeoff.cell[2] + w[2]
            })
            .unwrap_or(takeoff.cell[2]);
        let landing = doc
            .pieces
            .iter()
            .filter(|p| {
                p.uid > takeoff.uid && p.cell[2] >= lip_exit_z + 2 && !catalog_by_id(p.id).is_finish
            })
            .min_by_key(|p| p.uid)
            .expect("road after the gap");

        // The car leaves at the end of the lip and arrives on top of the landing,
        // so those are the two heights to compare. Reading the exit port of the
        // lip and the entry port of the landing is what puts both at the road
        // rather than at each piece's anchor, which for a slope is its foot.
        let lip_exit = piece_shape(takeoff.id, &takeoff.params, doc.cell_size)
            .ports
            .last()
            .map(|p| {
                let w = rotate_local_xz(p.cell, takeoff.yaw);
                from_grid(
                    [
                        takeoff.cell[0] + w[0],
                        takeoff.cell[1] + w[1],
                        takeoff.cell[2] + w[2],
                    ],
                    doc.cell_size,
                )
                .y
            })
            .expect("a ramp has an exit");
        // The landing is a level piece, so its road is its anchor's height plus
        // the height of its own centreline, which for level road is zero.
        let landing_top = from_grid(landing.cell, doc.cell_size).y
            + piece_shape(landing.id, &landing.params, doc.cell_size)
                .center
                .pos
                .iter()
                .map(|p| p.y)
                .fold(f32::MIN, f32::max);
        assert!(
            landing_top < lip_exit - 1.0,
            "the landing (y={landing_top:.1}) must sit below the lip (y={lip_exit:.1}) \\
             by enough to fly to, or the gap cannot be crossed"
        );
        // And the cells in between hold no road at all, measured from the lip's
        // exit rather than the ramp's anchor, which is its foot.
        for step in 1..3 {
            let cell = [takeoff.cell[0], takeoff.cell[1], lip_exit_z + step];
            let occupied = doc.pieces.iter().any(|inst| {
                piece_shape(inst.id, &inst.params, doc.cell_size)
                    .reserve_cells()
                    .any(|c| {
                        let w = rotate_local_xz(c, inst.yaw);
                        [
                            inst.cell[0] + w[0],
                            inst.cell[1] + w[1],
                            inst.cell[2] + w[2],
                        ] == cell
                    })
            });
            assert!(!occupied, "no piece may occupy {cell:?} inside the gap");
        }
    }
}

#[cfg(test)]
mod scurve_probe {
    use crate::geometry::piece_shape;
    use crate::piece::{PieceId, PieceParams};

    #[test]
    fn scurve_geometry() {
        for r in 1..=4u8 {
            let s = piece_shape(PieceId::SCurve, &PieceParams::default().radius(r), 4.0);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &s.center.pos {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            println!(
                "r={r} span x {:.1}..{:.1} z {:.1}..{:.1} ports {:?}",
                lo[0],
                hi[0],
                lo[2],
                hi[2],
                s.ports
                    .iter()
                    .map(|p| (p.cell, p.outward))
                    .collect::<Vec<_>>()
            );
        }
    }
}

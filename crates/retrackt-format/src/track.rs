//! Track documents: an ordered set of placed pieces plus spawn and rules.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::geometry::from_grid;
use crate::piece::{GridDir, PieceId, PieceParams, rotate_local_xz};

/// Bumped whenever the on-wire shape of [`TrackDocument`] or [`PieceInstance`]
/// changes. Share codes carry it so old tracks are rejected, not misread.
pub const FORMAT_VERSION: u32 = 1;

/// Ceiling on pieces in one document. Every piece builds a mesh, and the
/// simulation builds a second triangle soup from it, so an unbounded document
/// turns a small pasted or saved file into unbounded work at load.
pub const MAX_PIECES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum TrackError {
    #[error("malformed track: {0}")]
    Ron(#[from] ron::error::SpannedError),
    #[error("track has {count} pieces, over the {max} limit")]
    TooManyPieces { count: usize, max: usize },
}

/// Stable per-piece identity. Never a vector index: inserting a piece
/// renumbers indices and would silently corrupt saves, checkpoints and undo.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct PieceUid(pub u32);

/// One placed piece.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PieceInstance {
    #[serde(default)]
    pub uid: PieceUid,
    pub id: PieceId,
    /// Anchor cell. The piece's entry port sits on this cell.
    pub cell: [i16; 3],
    /// Quarter turns about +Y.
    #[serde(default)]
    pub yaw: u8,
    #[serde(default)]
    pub params: PieceParams,
}

impl PieceInstance {
    pub fn new(id: PieceId, cell: [i16; 3]) -> Self {
        Self {
            uid: PieceUid(0),
            id,
            cell,
            yaw: 0,
            params: PieceParams::default(),
        }
    }

    pub fn with_uid(mut self, uid: PieceUid) -> Self {
        self.uid = uid;
        self
    }

    pub fn with_yaw(mut self, yaw: u8) -> Self {
        self.yaw = yaw % 4;
        self
    }

    pub fn with_params(mut self, params: PieceParams) -> Self {
        self.params = params;
        self
    }

    /// World transform of this instance: rotate about the anchor, *then* move
    /// to it; rotation-first would throw yawed pieces clear of their track.
    pub fn world_matrix(&self, cell_size: f32) -> glam::Mat4 {
        let yaw = self.yaw as f32 * std::f32::consts::FRAC_PI_2;
        glam::Mat4::from_translation(from_grid(self.cell, cell_size))
            * glam::Mat4::from_rotation_y(yaw)
    }
}

/// A whole track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackDocument {
    pub format: u32,
    pub name: String,
    pub author: String,
    /// Cell edge length in metres.
    pub cell_size: f32,
    pub pieces: Vec<PieceInstance>,
    /// World-space spawn, overriding the first Start piece.
    #[serde(default)]
    pub spawn_xyz: Option<[f32; 3]>,
    #[serde(default)]
    pub spawn_yaw_deg: f32,
}

impl Default for TrackDocument {
    fn default() -> Self {
        Self::empty()
    }
}

impl TrackDocument {
    pub fn empty() -> Self {
        Self {
            format: FORMAT_VERSION,
            name: "untitled".into(),
            author: "anonymous".into(),
            cell_size: crate::DEFAULT_CELL_SIZE,
            pieces: Vec::new(),
            spawn_xyz: None,
            spawn_yaw_deg: 0.0,
        }
    }

    /// Give every piece a distinct UID and stamp the current format. UIDs that
    /// are already set are kept, so a saved track keeps its identity; unset and
    /// repeated ones are handed fresh values. Safe to call repeatedly.
    pub fn normalize_uids(&mut self) {
        // The uids already in the document, and the ones handed out below. The
        // two must be tracked apart: deciding which piece keeps an identity
        // consumes it, so `assigned` only ever grows and is what a fresh value
        // is checked against.
        let present: BTreeSet<u32> = self
            .pieces
            .iter()
            .map(|p| p.uid.0)
            .filter(|u| *u != 0)
            .collect();
        let mut assigned: BTreeSet<u32> = BTreeSet::new();
        // Fresh ids start above the highest one claimed, so a document carrying
        // an enormous uid costs no walk across the gap below it. A saturated
        // maximum leaves no room above, so start from the floor instead.
        let mut next = match present.iter().next_back() {
            Some(&u32::MAX) | None => 1,
            Some(&top) => top + 1,
        };
        for p in &mut self.pieces {
            // The first piece holding an identity keeps it; an unset slot or a
            // repeat is reissued below.
            if p.uid.0 != 0 && !assigned.contains(&p.uid.0) {
                assigned.insert(p.uid.0);
                continue;
            }
            while assigned.contains(&next) {
                next = next.saturating_add(1);
            }
            p.uid = PieceUid(next);
            assigned.insert(next);
            next = next.saturating_add(1);
        }
        self.format = FORMAT_VERSION;
    }

    /// Refuse a document too large to build a world from.
    pub fn check_piece_count(&self) -> Result<(), TrackError> {
        if self.pieces.len() > MAX_PIECES {
            return Err(TrackError::TooManyPieces {
                count: self.pieces.len(),
                max: MAX_PIECES,
            });
        }
        Ok(())
    }

    pub fn next_piece_uid(&self) -> PieceUid {
        PieceUid(
            self.pieces
                .iter()
                .map(|p| p.uid.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        )
    }

    /// World-space origin of a grid cell.
    pub fn cell_origin(&self, cell: [i16; 3]) -> [f32; 3] {
        from_grid(cell, self.cell_size).to_array()
    }

    pub fn piece(&self, uid: PieceUid) -> Option<&PieceInstance> {
        self.pieces.iter().find(|p| p.uid == uid)
    }

    /// The piece occupying `cell`, if any.
    pub fn piece_at(&self, cell: [i16; 3]) -> Option<&PieceInstance> {
        self.pieces.iter().find(|p| {
            let shape = crate::geometry::piece_shape(p.id, &p.params, self.cell_size);
            shape.reserve_cells().any(|c| {
                let w = rotate_local_xz(c, p.yaw);
                [p.cell[0] + w[0], p.cell[1] + w[1], p.cell[2] + w[2]] == cell
            })
        })
    }

    /// World spawn point and heading. Falls back to the Start piece.
    ///
    /// Several Start pieces are allowed and the last one placed is the one the
    /// run uses: building a second start beside a track being edited is how a
    /// variant of a circuit gets tried, and refusing the document for it would
    /// throw the work away. "Last placed" is the highest uid, not the last slot
    /// in the file. The array is an unordered set, and reading it as an order
    /// made the spawn depend on how the document happened to be saved.
    pub fn spawn(&self) -> (glam::Vec3, f32) {
        if let Some(xyz) = self.spawn_xyz {
            return (glam::Vec3::from(xyz), self.spawn_yaw_deg.to_radians());
        }
        match self.active_start() {
            Some(start) => {
                let shape = crate::geometry::piece_shape(start.id, &start.params, self.cell_size);
                let entry = shape.ports.first().map(|p| p.cell).unwrap_or([0, 0, 0]);
                let local = from_grid(rotate_local_xz(entry, start.yaw), self.cell_size);
                let origin = from_grid(start.cell, self.cell_size);
                let world = origin + local;
                let heading = start.yaw as f32 * std::f32::consts::FRAC_PI_2;
                // Ahead along the piece's own direction of travel: a world +Z
                // offset would set a yawed start piece's car down beside the road.
                let ahead = glam::Vec3::new(heading.sin(), 0.0, heading.cos()) * 2.0;
                (world + ahead, heading)
            }
            None => (glam::Vec3::new(0.0, 1.0, 0.0), 0.0),
        }
    }

    /// The Start piece the run uses: the last one placed.
    ///
    /// Highest uid, because a placement is handed the next uid above everything
    /// already in the document. That makes it the same answer on every load,
    /// where the position in the piece array is not.
    pub fn active_start(&self) -> Option<&PieceInstance> {
        self.pieces
            .iter()
            .filter(|p| crate::piece::catalog_by_id(p.id).is_start)
            .max_by_key(|p| p.uid)
    }

    /// Every checkpoint, in the order the car has to pass them.
    ///
    /// The order is a piece's own `checkpoint_order`, and a piece that sets none
    /// keeps the position it was placed at. Placement order is the fallback
    /// because the road cannot supply one any more: pieces are placed
    /// independently, so there is no single line the checkpoints sit along and
    /// "where it is on the track" has no answer. Uid is that placement order,
    /// and it survives a save, so an unnumbered gate stays where its author left
    /// it.
    ///
    /// A tie on the number is broken in favour of the piece that set it.
    /// Otherwise handing a gate the number it already sits at would change
    /// nothing, and "make this the first gate" would be a control that does
    /// nothing at all. Within one number, placement order breaks the tie, so a
    /// set of checkpoints sharing a number is still ordered predictably.
    pub fn checkpoint_sequence(&self) -> Vec<&PieceInstance> {
        let checkpoints: Vec<&PieceInstance> = self
            .race_order()
            .iter()
            .filter_map(|uid| self.piece(*uid))
            .filter(|p| crate::piece::catalog_by_id(p.id).is_checkpoint)
            .collect();
        let mut keyed: Vec<((u32, u8, usize), &PieceInstance)> = checkpoints
            .iter()
            .enumerate()
            .map(|(nth, p)| {
                // Explicit first, then the position it was placed at for
                // everything that never set one. The numbering starts above the
                // explicit range so an unnumbered gate cannot jump ahead of a
                // numbered one.
                let (order, set) = match p.params.checkpoint_order {
                    Some(order) => (u32::from(order), 0),
                    None => (
                        u32::from(u8::MAX)
                            .saturating_add(1)
                            .saturating_add(nth as u32),
                        1,
                    ),
                };
                ((order, set, nth), *p)
            })
            .collect();
        keyed.sort_by_key(|(key, _)| *key);
        keyed.into_iter().map(|(_, p)| p).collect()
    }

    /// Start, finish and checkpoint pieces, in race order.
    pub fn zones(&self) -> Vec<&PieceInstance> {
        self.race_order()
            .into_iter()
            .filter_map(|uid| self.piece(uid))
            .filter(|p| {
                let def = crate::piece::catalog_by_id(p.id);
                def.is_start || def.is_finish || def.is_checkpoint || def.is_boost
            })
            .collect()
    }

    /// Every piece, in the order a car meets them.
    ///
    /// Uid, which is placement order: `normalize_uids` hands each piece the
    /// next free number above everything already present, so a number says when
    /// a piece was dropped in and nothing else. That is the whole order there
    /// is, and it is a property of the document rather than of the file, so a
    /// shuffled or reordered array is still the same track.
    ///
    /// Deduplicated because this feeds the collision build, and a document
    /// carrying a repeated uid would otherwise name one piece twice. Every load
    /// path runs `normalize_uids` first, but the world is built from whatever
    /// document it is handed.
    pub fn race_order(&self) -> Vec<PieceUid> {
        let mut ordered: Vec<PieceUid> = self.pieces.iter().map(|p| p.uid).collect();
        ordered.sort_unstable();
        ordered.dedup();
        ordered
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, TrackError> {
        let mut doc: Self = ron::from_str(text)?;
        doc.check_piece_count()?;
        doc.normalize_uids();
        Ok(doc)
    }
}

/// A port in world grid space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorldPort {
    pub point: [i16; 3],
    pub outward: GridDir,
}

impl TrackDocument {
    /// Every connector port, in world grid space.
    pub fn world_ports(&self) -> Vec<(PieceUid, WorldPort)> {
        let mut out = Vec::new();
        for p in &self.pieces {
            let shape = crate::geometry::piece_shape(p.id, &p.params, self.cell_size);
            for port in shape.ports {
                let w = rotate_local_xz(port.cell, p.yaw);
                out.push((
                    p.uid,
                    WorldPort {
                        point: [p.cell[0] + w[0], p.cell[1] + w[1], p.cell[2] + w[2]],
                        outward: crate::piece::rotate_local_dir(port.outward, p.yaw),
                    },
                ));
            }
        }
        out
    }

    /// The piece whose entry meets `uid`'s exit head-on, if one does.
    ///
    /// A *suggestion*, not a fact about the track. Pieces are placed
    /// independently, so two of them meeting is something the author arranged
    /// rather than something the format requires, and a track may have any
    /// number of such pairs, none of them, or a chain that doubles back. The
    /// editor uses this to offer a join and the validator uses it to report one,
    /// but neither treats it as the road the car drives.
    pub fn joins(&self, uid: PieceUid) -> Option<PieceUid> {
        let ports = self.all_ports();
        // Every port past the entry is an exit: a junction's side arms join
        // like its through road, with the through road first in port order so
        // the chain prefers straight on.
        let exits: Vec<WorldPort> = ports
            .iter()
            .filter(|(u, _, i)| *u == uid && *i >= 1)
            .map(|(_, p, _)| *p)
            .collect();

        for exit in &exits {
            let mates = |port: &WorldPort| {
                port.point == exit.point && port.outward == exit.outward.opposite()
            };
            // Uid order, not the order `all_ports` happened to walk the piece
            // array in: a tie-break reading array order would let shuffling the
            // file change which neighbour is offered while the fingerprint
            // stayed identical.
            let mut candidates: Vec<PieceUid> = ports
                .iter()
                .filter(|(_, port, i)| *i == 0 && mates(port))
                .map(|(other_uid, _, _)| *other_uid)
                .filter(|u| *u != uid)
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            if let Some(inst) = self.piece(uid)
                && crate::piece::catalog_by_id(inst.id).is_finish
            {
                // A finish is a gate, not a road: nothing continues beyond it,
                // so offering a join there would invite building past the line.
                return None;
            }

            match candidates.len() {
                0 => {}
                1 => return Some(candidates[0]),
                // Contested join: two pieces share this cell. Prefer the one
                // that continues the current heading, then the lowest uid.
                _ => {
                    let heading = self.exit_heading(uid);
                    return candidates
                        .iter()
                        .copied()
                        .find(|c| Some(self.entry_heading(*c)) == heading)
                        .or(candidates.first().copied());
                }
            }
        }
        None
    }

    /// Every port as `(owner, port, 0 = entry | 1 = exit)`.
    pub fn all_ports(&self) -> Vec<(PieceUid, WorldPort, usize)> {
        let mut out = Vec::new();
        for p in &self.pieces {
            let shape = crate::geometry::piece_shape(p.id, &p.params, self.cell_size);
            for (i, port) in shape.ports.iter().enumerate() {
                let w = rotate_local_xz(port.cell, p.yaw);
                out.push((
                    p.uid,
                    WorldPort {
                        point: [p.cell[0] + w[0], p.cell[1] + w[1], p.cell[2] + w[2]],
                        outward: crate::piece::rotate_local_dir(port.outward, p.yaw),
                    },
                    i,
                ));
            }
        }
        out
    }

    /// Direction a piece travels as it leaves, from its exit tangent.
    fn exit_heading(&self, uid: PieceUid) -> Option<crate::piece::GridDir> {
        let inst = self.piece(uid)?;
        let shape = crate::geometry::piece_shape(inst.id, &inst.params, self.cell_size);
        shape
            .ports
            .last()
            .map(|p| crate::piece::rotate_local_dir(p.outward, inst.yaw))
    }

    /// Direction a piece travels as it enters, from its entry tangent.
    fn entry_heading(&self, uid: PieceUid) -> crate::piece::GridDir {
        let Some(inst) = self.piece(uid) else {
            return crate::piece::GridDir::PosZ;
        };
        let shape = crate::geometry::piece_shape(inst.id, &inst.params, self.cell_size);
        shape
            .ports
            .first()
            .map(|p| crate::piece::rotate_local_dir(p.outward.opposite(), inst.yaw))
            .unwrap_or(crate::piece::GridDir::PosZ)
    }

    /// Whether the road stops because there is nothing there, rather than
    /// because a piece is missing.
    ///
    /// The two are the same to the join walk and very different to the player. A
    /// piece ending in open air, with road resuming further along the same line,
    /// is a jump: the author left room to fly across and the track is complete.
    /// A piece ending against another piece that does not meet its port is a
    /// mistake, and so is one ending in air with nothing beyond it at all.
    pub fn chain_stops_at_air(&self, uid: PieceUid) -> bool {
        /// How far ahead a landing may plausibly be, in cells.
        const REACH: i32 = 6;
        let Some(inst) = self.piece(uid) else {
            return false;
        };
        let shape = crate::geometry::piece_shape(inst.id, &inst.params, self.cell_size);
        let Some(exit) = shape.ports.last() else {
            return false;
        };
        let exit_cell = {
            let w = rotate_local_xz(exit.cell, inst.yaw);
            [
                inst.cell[0] + w[0],
                inst.cell[1] + w[1],
                inst.cell[2] + w[2],
            ]
        };
        let ahead = crate::piece::rotate_local_dir(exit.outward, inst.yaw);
        let cell_at = |step: i32| -> Option<[i16; 3]> {
            let step = i16::try_from(step).ok()?;
            match ahead {
                crate::piece::GridDir::PosZ => {
                    Some([exit_cell[0], exit_cell[1], exit_cell[2] + step])
                }
                crate::piece::GridDir::NegZ => {
                    Some([exit_cell[0], exit_cell[1], exit_cell[2] - step])
                }
                crate::piece::GridDir::PosX => {
                    Some([exit_cell[0] + step, exit_cell[1], exit_cell[2]])
                }
                crate::piece::GridDir::NegX => {
                    Some([exit_cell[0] - step, exit_cell[1], exit_cell[2]])
                }
                // A gap is measured along the road, and a vertical exit has none.
                _ => None,
            }
        };
        let occupied = |probe: [i16; 3]| {
            self.pieces.iter().any(|other| {
                if other.uid == uid {
                    return false;
                }
                let other_shape =
                    crate::geometry::piece_shape(other.id, &other.params, self.cell_size);
                if other_shape.ports.is_empty() {
                    return false;
                }
                other_shape.reserve_cells().any(|c| {
                    let w = rotate_local_xz(c, other.yaw);
                    [
                        other.cell[0] + w[0],
                        other.cell[1] + w[1],
                        other.cell[2] + w[2],
                    ] == probe
                })
            })
        };

        // Road has to be missing at the very next cell for this to be a gap. A
        // piece butted up against this one that failed to mate is a different
        // fault, and reporting it as a jump would hide it.
        let Some(first) = cell_at(1) else {
            return false;
        };
        if occupied(first) {
            return false;
        }
        // And something has to be there to land on, within reach. Otherwise the
        // road simply stops, which is an unfinished track rather than a jump.
        (2..=REACH).any(|step| cell_at(step).is_some_and(&occupied))
    }

    /// The run of pieces joined end to end from the Start the run uses,
    /// following exits to entries and stopping at the first break.
    ///
    /// This is the connected stretch of road, which is what the editor snaps
    /// along and what the validator reports on. It is deliberately not the
    /// track: pieces placed apart still race, and a gap the car flies across is
    /// road the walk cannot cross. A track with no joins at all returns just the
    /// Start, which is a truthful answer rather than a failure.
    pub fn chain(&self) -> Vec<PieceUid> {
        let Some(start) = self.active_start() else {
            return Vec::new();
        };
        let mut out = vec![start.uid];
        // Set alongside the list: the loop below asks "have I been here" once per
        // step, and scanning the whole chain each time made this quadratic on
        // the long tracks it exists to order.
        let mut seen: BTreeSet<PieceUid> = BTreeSet::new();
        seen.insert(start.uid);
        let mut cur = start.uid;
        let mut guard = 0;
        while let Some(next) = self.joins(cur) {
            // Already visited: the run has closed, so it ends here.
            if !seen.insert(next) {
                break;
            }
            out.push(next);
            cur = next;
            guard += 1;
            if guard > self.pieces.len() {
                break;
            }
        }
        out
    }

    /// Pieces whose ports meet no other piece at all.
    ///
    /// A free-standing piece is legal now, so this is a warning's subject
    /// rather than a defect. It is what separates "the author built a second
    /// line to try later" from "the author dropped a piece in the wrong place",
    /// which is the only reason to mention it. Props are excluded: a wall joins
    /// nothing by nature, and reporting every one of them would bury the piece
    /// that matters.
    pub fn unjoined(&self) -> Vec<PieceUid> {
        self.pieces
            .iter()
            .filter(|inst| {
                let shape = crate::geometry::piece_shape(inst.id, &inst.params, self.cell_size);
                !shape.ports.is_empty() && !self.meets_any(inst.uid)
            })
            .map(|inst| inst.uid)
            .collect()
    }

    /// Whether `uid` has any road meeting either of its ports.
    ///
    /// Cell adjacency rather than a mating exit and entry: two pieces sharing a
    /// cell drive into each other whichever way they face, so that is what makes
    /// a piece reachable. A gap one cell wide is not adjacency, which is the
    /// point: the car crosses it in the air, not on the road.
    pub fn meets_any(&self, uid: PieceUid) -> bool {
        let Some(inst) = self.piece(uid) else {
            return false;
        };
        let shape = crate::geometry::piece_shape(inst.id, &inst.params, self.cell_size);
        if shape.ports.is_empty() {
            return false;
        }
        let own: BTreeSet<[i16; 3]> = shape
            .reserve_cells()
            .map(|c| {
                let w = rotate_local_xz(c, inst.yaw);
                [
                    inst.cell[0] + w[0],
                    inst.cell[1] + w[1],
                    inst.cell[2] + w[2],
                ]
            })
            .collect();
        self.pieces.iter().any(|other| {
            if other.uid == uid {
                return false;
            }
            let other_shape = crate::geometry::piece_shape(other.id, &other.params, self.cell_size);
            if other_shape.ports.is_empty() {
                return false;
            }
            other_shape.reserve_cells().any(|c| {
                let w = rotate_local_xz(c, other.yaw);
                own.contains(&[
                    other.cell[0] + w[0],
                    other.cell[1] + w[1],
                    other.cell[2] + w[2],
                ])
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::ChainBuilder;
    use crate::fingerprint::gameplay_fingerprint;
    use crate::piece::PieceId;
    use crate::piece::catalog_by_id;

    #[test]
    fn demo_track_chains_from_start_to_finish() {
        let doc = crate::demo::demo_track();
        let route = doc.chain();
        assert!(route.len() >= 8, "route too short: {:?}", route);
        let first = doc.piece(route[0]).unwrap();
        assert!(crate::piece::catalog_by_id(first.id).is_start);
        let last = doc.piece(route[route.len() - 1]).unwrap();
        assert!(crate::piece::catalog_by_id(last.id).is_finish);
    }

    #[test]
    fn demo_track_has_checkpoints_and_a_connected_route() {
        let doc = crate::demo::demo_track();
        assert!(
            doc.pieces
                .iter()
                .any(|p| crate::piece::catalog_by_id(p.id).is_checkpoint),
            "demo track needs at least one checkpoint"
        );
        for pair in doc.chain().windows(2) {
            assert_eq!(
                doc.joins(pair[0]),
                Some(pair[1]),
                "route claims {:?} follows {:?}",
                pair[1],
                pair[0]
            );
        }
    }

    #[test]
    fn a_side_road_joins_a_junction_mouth_head_on() {
        // A straight butted onto the east mouth must read as the junction's
        // continuation, the same way one butted onto the exit does.
        let mut doc = TrackDocument::empty();
        let t = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::TJunction, [0, 0, 0]).with_uid(t));
        let side = doc.next_piece_uid();
        // East mouth is local (2, 0, 2); yaw 1 turns the straight to run +X
        // away from it, entry facing back down the mouth.
        doc.pieces.push(
            PieceInstance::new(PieceId::Straight, [2, 0, 2])
                .with_uid(side)
                .with_yaw(1),
        );
        doc.normalize_uids();
        assert_eq!(doc.joins(t), Some(side));
        assert!(doc.meets_any(side));
    }

    #[test]
    fn the_chain_prefers_straight_on_at_a_junction() {
        // Through exit and side mouth both continued: the route follows the
        // through road, because a junction that routes every drive onto its
        // arm would make the main line unnavigable.
        let mut doc = TrackDocument::empty();
        let start = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::Start, [0, 0, -4]).with_uid(start));
        let t = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::TJunction, [0, 0, 0]).with_uid(t));
        let ahead = doc.next_piece_uid();
        doc.pieces
            .push(PieceInstance::new(PieceId::Straight, [0, 0, 4]).with_uid(ahead));
        let side = doc.next_piece_uid();
        doc.pieces.push(
            PieceInstance::new(PieceId::Straight, [2, 0, 2])
                .with_uid(side)
                .with_yaw(1),
        );
        doc.normalize_uids();
        assert_eq!(doc.joins(t), Some(ahead));
        assert_eq!(doc.chain(), vec![start, t, ahead]);
    }

    #[test]
    fn a_piece_exit_never_mates_with_another_exit() {
        // Matching on direction alone would also let an entry satisfy an
        // entry, inventing a route; matching by port index prevents it.
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1))];
        doc.normalize_uids();
        // The second piece goes wherever the first one leaves, so the pair is
        // joined whatever a straight happens to be sized at.
        let exit = crate::geometry::from_grid(
            crate::geometry::piece_shape(PieceId::Straight, &PieceParams::default(), doc.cell_size)
                .ports
                .last()
                .map(|p| {
                    let w = crate::piece::rotate_local_xz(p.cell, 0);
                    w
                })
                .unwrap_or([1, 0, 0]),
            doc.cell_size,
        );
        let exit_cell = crate::geometry::to_grid(exit, doc.cell_size);
        doc.pieces
            .push(PieceInstance::new(PieceId::Straight, exit_cell).with_uid(doc.next_piece_uid()));
        doc.normalize_uids();
        assert_eq!(doc.joins(PieceUid(1)), Some(PieceUid(2)));
        assert_eq!(doc.joins(PieceUid(2)), None, "tail is a dead end");
    }

    #[test]
    fn spawn_falls_back_to_the_start_piece() {
        let doc = crate::demo::demo_track();
        let (pos, heading) = doc.spawn();
        assert!(pos.distance(glam::Vec3::new(0.0, 0.0, 0.0)) < 12.0);
        assert!(heading.abs() < 1e-6);
    }

    #[test]
    fn explicit_spawn_overrides_the_start_piece() {
        let mut doc = crate::demo_track();
        doc.spawn_xyz = Some([10.0, 3.0, -4.0]);
        doc.spawn_yaw_deg = 90.0;
        let (pos, heading) = doc.spawn();
        assert_eq!(pos, glam::Vec3::new(10.0, 3.0, -4.0));
        assert!((heading - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn uids_are_assigned_once_and_are_stable() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]),
            PieceInstance::new(PieceId::Straight, [0, 0, 1]),
        ];
        doc.normalize_uids();
        assert_eq!(doc.pieces[0].uid, PieceUid(1));
        assert_eq!(doc.pieces[1].uid, PieceUid(2));
        let before = doc.pieces.clone();
        doc.normalize_uids();
        assert_eq!(doc.pieces, before, "re-normalising must not change uids");
    }

    #[test]
    fn an_oversized_document_is_refused_at_parse() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Straight, [0, 0, 0]); MAX_PIECES];
        assert!(
            doc.check_piece_count().is_ok(),
            "the limit itself is allowed"
        );
        doc.pieces
            .push(PieceInstance::new(PieceId::Straight, [0, 0, 1]));
        assert!(doc.check_piece_count().is_err());
        assert!(TrackDocument::from_ron(&doc.to_ron().unwrap()).is_err());
    }

    #[test]
    fn repeated_uids_are_reissued_and_a_saved_track_keeps_its_identities() {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(7)),
            PieceInstance::new(PieceId::Straight, [0, 0, 1]).with_uid(PieceUid(7)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 3]).with_uid(PieceUid(2)),
            PieceInstance::new(PieceId::Straight, [0, 0, 4]),
        ];
        doc.normalize_uids();

        let uids: Vec<u32> = doc.pieces.iter().map(|p| p.uid.0).collect();
        assert_eq!(uids[0], 7, "the first holder keeps its identity");
        assert_eq!(uids[2], 1);
        assert_eq!(uids[3], 2);
        let mut distinct = uids.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            uids.len(),
            "no identity may repeat: {uids:?}"
        );

        let before = doc.pieces.clone();
        doc.normalize_uids();
        assert_eq!(doc.pieces, before, "re-normalising must not change uids");
    }

    /// A start, three checkpoints and a finish, all on one chain. Checkpoints
    /// are one cell long by default, so each sits where the road passes it.
    fn gated_doc() -> TrackDocument {
        let mut b = ChainBuilder::new("gates");
        b.step(PieceId::Start, PieceParams::default().length(2));
        b.step(PieceId::Checkpoint, PieceParams::default());
        b.step(PieceId::Checkpoint, PieceParams::default());
        b.step(PieceId::Checkpoint, PieceParams::default());
        b.step(PieceId::Finish, PieceParams::default().length(2));
        b.build()
    }

    /// Uids of the checkpoints, in the order the document says the car must pass
    /// them.
    fn sequence(doc: &TrackDocument) -> Vec<PieceUid> {
        doc.checkpoint_sequence().iter().map(|p| p.uid).collect()
    }

    /// Road position of each checkpoint, so a reordering is visible as a set of
    /// uids that is not the order the chain has them in.
    fn road_order(doc: &TrackDocument) -> Vec<PieceUid> {
        doc.pieces
            .iter()
            .filter(|p| catalog_by_id(p.id).is_checkpoint)
            .map(|p| p.uid)
            .collect()
    }

    #[test]
    fn a_document_that_sets_no_order_keeps_the_order_the_road_gives() {
        let doc = gated_doc();
        assert_eq!(
            sequence(&doc),
            road_order(&doc),
            "the field is new: a track that never set it must not be reordered"
        );
    }

    #[test]
    fn an_explicit_order_moves_a_checkpoint_up_along_the_finish() {
        let mut doc = gated_doc();
        // The last gate on the road is taken first.
        let last = *road_order(&doc).last().expect("three checkpoints");
        let slot = doc.pieces.iter_mut().find(|p| p.uid == last).unwrap();
        slot.params = slot.params.order(1);

        let wanted = sequence(&doc);
        assert_eq!(wanted.first(), Some(&last), "order 1 is passed first");
        assert_eq!(wanted.len(), 3);
        assert_eq!(
            wanted,
            vec![last, road_order(&doc)[0], road_order(&doc)[1]],
            "the others keep the road order behind it"
        );
    }

    #[test]
    fn checkpoints_sharing_an_order_keep_their_placement_order() {
        let mut doc = gated_doc();
        // Two gates on the same number: either may be the one passed, so the
        // tie is the order they were placed in rather than the order the file
        // happens to list them.
        let road = road_order(&doc);
        for uid in [road[0], road[2]] {
            let slot = doc.pieces.iter_mut().find(|p| p.uid == uid).unwrap();
            slot.params = slot.params.order(3);
        }

        assert_eq!(
            sequence(&doc),
            vec![road[0], road[2], road[1]],
            "the pair sharing a number leads, in placement order, and the \
             unnumbered gate follows"
        );
    }

    /// A gate with no number of its own sits after every gate that has one.
    ///
    /// It used to keep its place on the road instead, which is what the test this
    /// replaces checked: an unnumbered gate took the number its position on the
    /// road gave it, and an explicit number had to fight with that. The road
    /// cannot supply a number any more, since pieces are placed independently,
    /// so the only order left is the order they were placed in. Ranking those
    /// below every explicit number is what keeps "put this gate first" meaning
    /// first, rather than first among the gates that did not say.
    #[test]
    fn an_unnumbered_gate_follows_every_numbered_one() {
        let mut doc = gated_doc();
        let road = road_order(&doc);
        // Number the last gate 1, so it must lead.
        let slot = doc.pieces.iter_mut().find(|p| p.uid == road[2]).unwrap();
        slot.params = slot.params.order(1);

        assert_eq!(
            sequence(&doc).first(),
            Some(&road[2]),
            "an explicit order wins over every unnumbered gate"
        );
        assert_eq!(sequence(&doc).len(), 3, "no gate is dropped either way");
    }

    #[test]
    fn reordering_the_gates_changes_the_fingerprint() {
        let plain = gated_doc();
        let mut reordered = plain.clone();
        let last = *road_order(&reordered).last().expect("three checkpoints");
        let slot = reordered.pieces.iter_mut().find(|p| p.uid == last).unwrap();
        slot.params = slot.params.order(1);
        assert_ne!(
            gameplay_fingerprint(&plain),
            gameplay_fingerprint(&reordered),
            "a different gate sequence is a different run"
        );
    }

    #[test]
    fn a_saturated_uid_does_not_walk_the_whole_range() {
        // A hostile document carrying u32::MAX must not cost a scan up to it.
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(u32::MAX)),
            PieceInstance::new(PieceId::Straight, [0, 0, 1]).with_uid(PieceUid(u32::MAX)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]),
        ];
        doc.normalize_uids();
        assert_eq!(doc.pieces[0].uid, PieceUid(u32::MAX));
        assert_ne!(doc.pieces[1].uid, doc.pieces[2].uid);
    }

    #[test]
    fn ron_round_trips() {
        let doc = crate::demo::demo_track();
        let text = doc.to_ron().unwrap();
        let back = TrackDocument::from_ron(&text).unwrap();
        assert_eq!(back.pieces.len(), doc.pieces.len());
        assert_eq!(back.cell_size, doc.cell_size);
        assert_eq!(back.name, doc.name);
    }

    #[test]
    fn yawed_pieces_occupy_rotated_cells() {
        let mut doc = TrackDocument::empty();
        let straight = PieceInstance::new(PieceId::Straight, [0, 0, 0])
            .with_uid(PieceUid(1))
            .with_params(PieceParams::default().length(2));
        doc.pieces.push(straight);
        // Unyawed it occupies z 0..2 at x=0.
        assert!(doc.piece_at([0, 0, 2]).is_some());
        // Yawed 90 degrees it runs along +X from the anchor instead.
        doc.pieces[0].yaw = 1;
        assert!(doc.piece_at([2, 0, 0]).is_some());
        assert!(doc.piece_at([0, 0, 2]).is_none());
    }

    #[test]
    fn world_matrix_puts_local_origin_on_the_anchor() {
        // The transform must not swing the piece away from its own cell.
        for yaw in 0..4 {
            let inst = PieceInstance::new(PieceId::Straight, [3, 2, 5]).with_yaw(yaw);
            let m = inst.world_matrix(4.0);
            let origin = m.transform_point3(glam::Vec3::ZERO);
            let expected = from_grid([3, 2, 5], 4.0);
            assert!(
                origin.distance(expected) < 1e-4,
                "yaw {yaw}: origin {origin:?} should be {expected:?}"
            );
        }
    }

    #[test]
    fn world_matrix_yaws_around_the_anchor_not_the_world_origin() {
        let inst = PieceInstance::new(PieceId::Straight, [3, 0, 5]).with_yaw(1);
        let m = inst.world_matrix(4.0);
        // Local +Z must map to world +X, still based at the anchor.
        let a = m.transform_point3(glam::Vec3::ZERO);
        let b = m.transform_point3(glam::Vec3::Z * 4.0);
        assert!((b - a - glam::Vec3::X * 4.0).length() < 1e-4);
    }

    #[test]
    fn to_and_from_grid_round_trip() {
        use crate::geometry::to_grid;
        for c in [[0, 0, 0], [3, -2, 7], [-5, 1, -1]] {
            let p = from_grid(c, 4.0);
            assert_eq!(to_grid(p, 4.0), c);
        }
    }

    #[test]
    fn race_order_follows_the_road_not_the_file() {
        let doc = crate::demo::demo_track();
        let route = doc.chain();
        // Every piece the chain reaches comes first, in chain order.
        let order = doc.race_order();
        assert_eq!(&order[..route.len()], route.as_slice());
        assert_eq!(order.len(), doc.pieces.len(), "no piece may be dropped");
    }

    /// Race order is the order the pieces were placed in, even when that
    /// disagrees with the road.
    ///
    /// The test this replaces required the opposite: it renumbered a built-in
    /// circuit so its uids ran against the chain, and insisted race order came
    /// back along the road anyway. That was correct while race order was a walk
    /// from the Start, and it is the assumption free placement removes. Two
    /// documents whose pieces are numbered differently are two runs, and saying
    /// so is the honest answer rather than a way of quietly preferring one.
    #[test]
    fn race_order_follows_placement_even_against_the_road() {
        let mut doc = crate::demo::demo_track();
        let chain = doc.chain();
        let reversed: Vec<u32> = chain.iter().rev().map(|u| u.0).collect();
        for piece in &mut doc.pieces {
            let rank = chain.iter().position(|u| *u == piece.uid);
            let Some(rank) = rank.and_then(|r| reversed.get(r)).copied() else {
                continue;
            };
            piece.uid = PieceUid(rank);
        }
        doc.normalize_uids();

        // Compared against the chain, which the numbering deliberately contradicts.
        // Recomputed after renumbering: the walk above named pieces by their old
        // uids, and comparing against those would compare two different labels.
        let order: Vec<u32> = doc.race_order().iter().map(|u| u.0).collect();
        let road: Vec<u32> = doc.chain().iter().map(|u| u.0).collect();
        assert_ne!(
            order, road,
            "the fixture must number the pieces against the road, or it proves nothing"
        );
        assert_eq!(order.len(), doc.pieces.len(), "no piece may be dropped");
        // And ascending, because placement order is the uid order.
        let mut ascending = order.clone();
        ascending.sort_unstable();
        assert_eq!(order, ascending, "race order is placement order");
    }

    #[test]
    fn race_order_includes_pieces_the_road_never_reaches() {
        let mut doc = crate::demo::demo_track();
        let uid = doc.next_piece_uid();
        let orphan = PieceInstance::new(PieceId::Straight, [500, 0, 500]).with_uid(uid);
        doc.pieces.push(orphan);
        doc.normalize_uids();

        let order = doc.race_order();
        assert_eq!(order.len(), doc.pieces.len());
        assert!(
            order.contains(&orphan.uid),
            "a detached piece is still a piece"
        );
        assert!(
            !doc.chain().contains(&orphan.uid),
            "and it is still not part of the chain"
        );
    }

    #[test]
    fn shuffling_the_file_does_not_change_race_order() {
        // The property the fingerprint already promised and the world now relies
        // on: an identical track stored in a different order is the same track.
        let doc = crate::demo::demo_track();
        let mut shuffled = doc.clone();
        shuffled.pieces.reverse();
        assert_eq!(
            doc.race_order(),
            shuffled.race_order(),
            "piece array order must not reach race order"
        );
        assert_eq!(doc.chain(), shuffled.chain());
        assert_eq!(
            crate::fingerprint::gameplay_fingerprint(&doc),
            crate::fingerprint::gameplay_fingerprint(&shuffled)
        );
    }

    #[test]
    fn a_contested_join_resolves_the_same_either_way_round() {
        // Two straights whose entries both mate head-on with the same exit. The
        // tie-break used to read the piece array, so which one the chain picked
        // could change under a shuffle while the fingerprint stayed put — and
        // race order is derived from this chain.
        //
        // A Start is two cells long, so its exit is the third cell, not the
        // second. Both straights sit there and both leave into the same cell, so
        // the route is three long whichever one is taken.
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1))];
        doc.normalize_uids();
        // The Start piece's own exit, rather than a cell number written by hand.
        let start_exit = crate::geometry::to_grid(
            crate::geometry::from_grid(
                crate::geometry::piece_shape(
                    PieceId::Start,
                    &PieceParams::default(),
                    doc.cell_size,
                )
                .ports
                .last()
                .map(|p| crate::piece::rotate_local_xz(p.cell, 0))
                .unwrap_or([0, 0, 0]),
                doc.cell_size,
            ),
            doc.cell_size,
        );
        let straight = PieceParams::default();
        let straight_len = crate::geometry::to_grid(
            crate::geometry::from_grid(
                crate::geometry::piece_shape(PieceId::Straight, &straight, doc.cell_size)
                    .ports
                    .last()
                    .map(|p| crate::piece::rotate_local_xz(p.cell, 0))
                    .unwrap_or([1, 0, 0]),
                doc.cell_size,
            ),
            doc.cell_size,
        )[2];
        // Both straights sit on the same exit, so they contest it. The fourth
        // piece continues whichever one the walk takes.
        let after = [start_exit[0], start_exit[1], start_exit[2] + straight_len];
        for cell in [start_exit, start_exit, after] {
            doc.pieces
                .push(PieceInstance::new(PieceId::Straight, cell).with_uid(doc.next_piece_uid()));
        }
        doc.normalize_uids();
        let forward = doc.chain();
        assert!(
            forward.len() >= 3,
            "the fixture must actually present a contested join: {forward:?}"
        );
        doc.pieces.reverse();
        assert_eq!(
            forward,
            doc.chain(),
            "the chain may not depend on file order"
        );
    }

    #[test]
    fn zones_come_back_in_race_order() {
        let doc = crate::demo::demo_track();
        let zones: Vec<PieceUid> = doc.zones().iter().map(|p| p.uid).collect();
        let order = doc.race_order();
        let ranks: Vec<usize> = zones
            .iter()
            .map(|uid| {
                order
                    .iter()
                    .position(|u| u == uid)
                    .expect("zone is a piece")
            })
            .collect();
        let mut sorted = ranks.clone();
        sorted.sort_unstable();
        assert_eq!(ranks, sorted, "zones must not come back in file order");
    }
}

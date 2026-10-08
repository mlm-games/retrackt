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
        let present: BTreeSet<u32> = self.pieces.iter().map(|p| p.uid.0).filter(|u| *u != 0).collect();
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
    pub fn spawn(&self) -> (glam::Vec3, f32) {
        if let Some(xyz) = self.spawn_xyz {
            return (glam::Vec3::from(xyz), self.spawn_yaw_deg.to_radians());
        }
        match self
            .pieces
            .iter()
            .find(|p| crate::piece::catalog_by_id(p.id).is_start)
        {
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
    /// The connected chain from [`Self::route`] first, then whatever the chain
    /// does not reach, ordered by uid. Route order is a property of the geometry
    /// rather than of the file: the piece array is an unordered set whose order
    /// only ever reflected the order pieces happened to be created in, and every
    /// consumer that read it as race order inherited that.
    ///
    /// Pieces outside the chain are still returned, because a track being built
    /// has plenty of them and dropping them would make this a partial answer.
    /// Their relative order carries no meaning, so uid keeps it stable across a
    /// save and load.
    pub fn race_order(&self) -> Vec<PieceUid> {
        let route = self.route();
        let mut on_route: BTreeSet<PieceUid> = BTreeSet::new();
        let mut ordered = Vec::with_capacity(self.pieces.len());
        // Filtered rather than pushed: a document with a repeated uid would
        // otherwise name the same piece twice, and this feeds the collision
        // build. `normalize_uids` clears that on every load path, but the world
        // is built from whatever document it is handed.
        for uid in route {
            if on_route.insert(uid) {
                ordered.push(uid);
            }
        }
        let mut rest: Vec<_> = self
            .pieces
            .iter()
            .map(|p| p.uid)
            .filter(|uid| !on_route.contains(uid))
            .collect();
        rest.sort_unstable();
        rest.dedup();
        ordered.extend(rest);
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

    /// The piece that follows `uid` along the track: `uid`'s exit must be met
    /// head-on by an *entry*, index-matched so exits never satisfy exits; loops self-match.
    pub fn next_piece(&self, uid: PieceUid) -> Option<PieceUid> {
        let ports = self.all_ports();
        let exits: Vec<WorldPort> = ports
            .iter()
            .filter(|(u, _, i)| *u == uid && *i == 1)
            .map(|(_, p, _)| *p)
            .collect();

        for exit in &exits {
            let mates = |port: &WorldPort| {
                port.point == exit.point && port.outward == exit.outward.opposite()
            };
            // Uid order, not the order `all_ports` happened to walk the piece
            // array in: race order is derived from this chain, so a tie-break
            // that read array order would let shuffling the file change which
            // gates a car meets while its fingerprint stayed identical.
            let mut candidates: Vec<PieceUid> = ports
                .iter()
                .filter(|(_, port, i)| *i == 0 && mates(port))
                .map(|(other_uid, _, _)| *other_uid)
                .filter(|u| *u != uid)
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            // A loop's own entry sits on its exit, so it is a candidate for
            // continuing itself. It never is: a piece cannot follow itself.
            if let Some(inst) = self.piece(uid)
                && crate::piece::catalog_by_id(inst.id).is_finish
            {
                // A finish ends the route even if something is attached
                // beyond it.
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

    /// Ordered chain of piece UIDs from the Start piece, following exits to
    /// entries. Stops at the first dead end.
    pub fn route(&self) -> Vec<PieceUid> {
        // Lowest uid, not the first Start in the array: a shuffled file is the
        // same track, so which Start the chain begins at cannot depend on where
        // it happens to sit in the file.
        let Some(start) = self
            .pieces
            .iter()
            .filter(|p| crate::piece::catalog_by_id(p.id).is_start)
            .min_by_key(|p| p.uid)
        else {
            return Vec::new();
        };
        let mut out = vec![start.uid];
        // Set alongside the list: the loop below asks "have I been here" once per
        // step, and scanning the whole chain each time made route building
        // quadratic on the long tracks it exists to order.
        let mut seen: BTreeSet<PieceUid> = BTreeSet::new();
        seen.insert(start.uid);
        let mut cur = start.uid;
        let mut guard = 0;
        while let Some(next) = self.next_piece(cur) {
            // Already visited: the route has closed, so it ends here.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceId;

    #[test]
    fn demo_track_chains_from_start_to_finish() {
        let doc = crate::demo::demo_track();
        let route = doc.route();
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
        for pair in doc.route().windows(2) {
            assert_eq!(
                doc.next_piece(pair[0]),
                Some(pair[1]),
                "route claims {:?} follows {:?}",
                pair[1],
                pair[0]
            );
        }
    }

    #[test]
    fn a_piece_exit_never_mates_with_another_exit() {
        // Matching on direction alone would also let an entry satisfy an
        // entry, inventing a route; matching by port index prevents it.
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 1]).with_uid(PieceUid(2)),
        ];
        assert_eq!(doc.next_piece(PieceUid(1)), Some(PieceUid(2)));
        assert_eq!(doc.next_piece(PieceUid(2)), None, "tail is a dead end");
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
        assert!(doc.check_piece_count().is_ok(), "the limit itself is allowed");
        doc.pieces.push(PieceInstance::new(PieceId::Straight, [0, 0, 1]));
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
        assert_eq!(distinct.len(), uids.len(), "no identity may repeat: {uids:?}");

        let before = doc.pieces.clone();
        doc.normalize_uids();
        assert_eq!(doc.pieces, before, "re-normalising must not change uids");
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
        let route = doc.route();
        // Every piece the chain reaches comes first, in chain order.
        let order = doc.race_order();
        assert_eq!(&order[..route.len()], route.as_slice());
        assert_eq!(order.len(), doc.pieces.len(), "no piece may be dropped");
    }

    #[test]
    fn race_order_differs_from_uid_order_when_the_file_says_otherwise() {
        // The built-in circuits are built by chaining, so their uids happen to
        // run in race order and cannot show the difference. Hand them uids that
        // do not: this is what an edited or hand-written document looks like, and
        // it is the case the decoupling exists for.
        let mut doc = crate::demo::demo_track();
        let route = doc.route();
        let reversed: Vec<u32> = route.iter().rev().map(|u| u.0).collect();
        for piece in &mut doc.pieces {
            let rank = route.iter().position(|u| *u == piece.uid);
            let Some(rank) = rank.and_then(|r| reversed.get(r)).copied() else {
                continue;
            };
            piece.uid = PieceUid(rank);
        }
        doc.normalize_uids();

        // Compared against ascending uid, not against array order: the array is
        // still in chain order, so array order would coincide with race order
        // and prove nothing.
        let mut ascending: Vec<u32> = doc.pieces.iter().map(|p| p.uid.0).collect();
        ascending.sort_unstable();
        let order: Vec<u32> = doc.race_order().iter().map(|u| u.0).collect();
        assert_ne!(order, ascending, "race order must not read the uids");
        assert_eq!(order.len(), doc.pieces.len(), "no piece may be dropped");
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
        assert!(order.contains(&orphan.uid), "a detached piece is still a piece");
        assert!(
            !doc.route().contains(&orphan.uid),
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
        assert_eq!(doc.route(), shuffled.route());
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
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Start, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(2)),
            PieceInstance::new(PieceId::Straight, [0, 0, 2]).with_uid(PieceUid(3)),
        ];
        doc.normalize_uids();
        let forward = doc.route();
        assert!(
            forward.len() >= 3,
            "the fixture must actually present a contested join: {forward:?}"
        );
        doc.pieces.reverse();
        assert_eq!(forward, doc.route(), "the chain may not depend on file order");
    }

    #[test]
    fn zones_come_back_in_race_order() {
        let doc = crate::demo::demo_track();
        let zones: Vec<PieceUid> = doc.zones().iter().map(|p| p.uid).collect();
        let order = doc.race_order();
        let ranks: Vec<usize> = zones
            .iter()
            .map(|uid| order.iter().position(|u| u == uid).expect("zone is a piece"))
            .collect();
        let mut sorted = ranks.clone();
        sorted.sort_unstable();
        assert_eq!(ranks, sorted, "zones must not come back in file order");
    }
}

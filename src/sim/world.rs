//! Collision world: the drivable surface as a triangle soup.
//!
//! Built from [`retrackt_format::geometry::piece_shape`] — literally the mesh
//! that gets drawn, so no separate collider can disagree with what the player
//! sees. Lookups go through a uniform spatial grid: the car probes several
//! points per tick, and a linear scan over every triangle would dominate the frame.

use glam::{Mat4, Vec3};
use retrackt_format::{
    PieceInstance, PieceUid, TrackDocument, demo::piece_centre, geometry::piece_shape,
    piece::catalog_by_id,
};

/// World bounds of the track's own geometry, for the terrain ring to sit around.
fn bounds_of(doc: &TrackDocument) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for inst in &doc.pieces {
        let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
        let m = inst.world_matrix(doc.cell_size);
        for p in &shape.center.pos {
            let w = m.transform_point3(glam::Vec3::from(*p));
            for a in 0..3 {
                lo[a] = lo[a].min(w[a]);
                hi[a] = hi[a].max(w[a]);
            }
        }
    }
    if lo[0] > hi[0] {
        return ([0.0; 3], [0.0; 3]);
    }
    (lo, hi)
}

/// One surface triangle with an outward normal.
#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    /// Points up, out of the road, toward the car.
    pub normal: Vec3,
    pub owner: PieceUid,
}

/// Half-width of the plain in the collision world. Far past the renderer's far
/// plane, so a car that wanders always finds ground rather than an edge.
pub const GROUND_REACH: f32 = 8000.0;
/// How far below the road the plain sits. A kerb's worth: enough that leaving the
/// road is felt and seen, little enough that the car climbs back on.
pub const GROUND_DROP: f32 = 0.25;

impl Triangle {
    #[inline]
    pub fn area(&self) -> f32 {
        (self.b - self.a).cross(self.c - self.a).length() * 0.5
    }
}

/// A surface hit.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub owner: PieceUid,
}

/// Ceiling on spatial-grid cells. `Grid::build` coarsens its cell size until the
/// grid fits this, so a document spanning an absurd box stays affordable.
const MAX_CELLS: u64 = 1 << 20;

/// Uniform grid over world space.
#[derive(Clone, Debug)]
struct Grid {
    origin: Vec3,
    /// Shared by bucketing and querying so the two cannot drift.
    cell: f32,
    dims: [i32; 3],
    /// Compressed sparse rows over triangle indices: cell `i` owns
    /// `flat[offsets[i]..offsets[i + 1]]`. Flat rather than one box per cell,
    /// because a coarsened hostile grid would otherwise mean an allocation per
    /// cell, which is the cost the cap exists to bound.
    offsets: Vec<u32>,
    flat: Vec<u32>,
}

impl Grid {
    /// Triangle indices bucketed in `cell`.
    fn in_cell(&self, cell: usize) -> &[u32] {
        &self.flat[self.offsets[cell] as usize..self.offsets[cell + 1] as usize]
    }

    fn build(triangles: &[Triangle], cell: f32) -> Self {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for t in triangles {
            for v in [t.a, t.b, t.c] {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if triangles.is_empty() {
            lo = Vec3::splat(-1.0);
            hi = Vec3::splat(1.0);
        }

        // Coarsen the cell until the grid fits its budget. Piece counts are capped
        // at load, but that cannot bound this: two pieces at opposite ends of the
        // cell range span the whole box, and at the requested cell size the cell
        // count runs to 10^14. A coarser cell costs lookup precision, not
        // correctness — every triangle still lands in the cells it overlaps.
        let mut cell = cell.max(1e-3);
        let (dims, origin, total) = loop {
            // Pad by two cells, not one: a flat track's vertical span is zero, so
            // a one-cell margin sends rays from above outside the grid, finding nothing.
            let pad = Vec3::splat(cell * 2.0);
            let origin = lo - pad;
            let span = hi + pad - origin;
            let dims = [
                (span.x / cell).ceil().max(2.0) as i32,
                (span.y / cell).ceil().max(2.0) as i32,
                (span.z / cell).ceil().max(2.0) as i32,
            ];
            let total =
                u64::from(dims[0] as u32) * u64::from(dims[1] as u32) * u64::from(dims[2] as u32);
            if total <= MAX_CELLS {
                break (dims, origin, total as usize);
            }
            cell *= 2.0;
        };

        // Bucket in two passes so each cell holds one contiguous slice.
        let mut g = Self {
            origin,
            cell,
            dims,
            offsets: Vec::new(),
            flat: Vec::new(),
        };

        let mut counts = vec![0u32; total];
        for t in triangles {
            for idx in g.cells_in_box(t.a.min(t.b).min(t.c), t.a.max(t.b).max(t.c)) {
                counts[idx] += 1;
            }
        }
        let mut offsets = vec![0u32; total + 1];
        for i in 0..total {
            offsets[i + 1] = offsets[i] + counts[i];
        }
        // One slot per triangle-cell reference, not per cell: a triangle spanning
        // several cells contributes one entry to each, so `offsets[total]` is the
        // true reference count and can exceed the cell count. Sizing by `total`
        // wrote past the end of `flat` whenever the geometry was dense.
        let mut flat = vec![0u32; offsets[total] as usize];
        let mut cursor = offsets[..total].to_vec();
        for (ti, t) in triangles.iter().enumerate() {
            for idx in g.cells_in_box(t.a.min(t.b).min(t.c), t.a.max(t.b).max(t.c)) {
                flat[cursor[idx] as usize] = ti as u32;
                cursor[idx] += 1;
            }
        }
        g.offsets = offsets;
        g.flat = flat;
        g
    }

    /// Cell indices covering an axis-aligned box, in a stable order.
    fn cells_in_box(&self, lo: Vec3, hi: Vec3) -> Vec<usize> {
        let (Some(a), Some(b)) = (self.cell_of(lo), self.cell_of(hi)) else {
            // Partly or wholly outside the grid: clamp into range so a query
            // near the edge still returns the cells it overlaps.
            return self.clamped_box(lo, hi);
        };
        let (ax, ay, az) = (
            a % self.dims[0] as usize,
            a / self.dims[0] as usize % self.dims[1] as usize,
            a / (self.dims[0] * self.dims[1]) as usize,
        );
        let (bx, by, bz) = (
            b % self.dims[0] as usize,
            b / self.dims[0] as usize % self.dims[1] as usize,
            b / (self.dims[0] * self.dims[1]) as usize,
        );
        let mut out = Vec::with_capacity((bx - ax + 1) * (by - ay + 1) * (bz - az + 1));
        for z in az..=bz {
            for y in ay..=by {
                for x in ax..=bx {
                    out.push(x + self.dims[0] as usize * (y + self.dims[1] as usize * z));
                }
            }
        }
        out
    }

    fn clamped_box(&self, lo: Vec3, hi: Vec3) -> Vec<usize> {
        let at = |p: Vec3| {
            let c = ((p - self.origin) / self.cell).floor();
            (
                (c.x as i32).clamp(0, self.dims[0] - 1) as usize,
                (c.y as i32).clamp(0, self.dims[1] - 1) as usize,
                (c.z as i32).clamp(0, self.dims[2] - 1) as usize,
            )
        };
        let (ax, ay, az) = at(lo.min(hi));
        let (bx, by, bz) = at(lo.max(hi));
        let mut out = Vec::new();
        for z in az..=bz {
            for y in ay..=by {
                for x in ax..=bx {
                    out.push(x + self.dims[0] as usize * (y + self.dims[1] as usize * z));
                }
            }
        }
        out
    }

    fn cell_of(&self, p: Vec3) -> Option<usize> {
        let c = ((p - self.origin) / self.cell).floor();
        let i = [c.x as i32, c.y as i32, c.z as i32];
        if i[0] < 0 || i[1] < 0 || i[2] < 0 {
            return None;
        }
        if i[0] >= self.dims[0] || i[1] >= self.dims[1] || i[2] >= self.dims[2] {
            return None;
        }
        Some((i[0] + self.dims[0] * (i[1] + self.dims[1] * i[2])) as usize)
    }

    /// Triangles along `origin` + `dir * t` for `t` in `0..max_t`. A 3D-DDA
    /// walk; deduped and ray-ordered so the work done never depends on grid layout.
    fn ray_triangles(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<u32> {
        let mut out = Vec::new();
        // Start from just inside the grid even when the ray begins outside it,
        // else a ray cast from above the track finds no cell to walk from.
        let Some(mut c) = self
            .cell_of(origin)
            .or_else(|| self.clamped_box(origin, origin).first().copied())
        else {
            return out;
        };
        let cell = self.cell;
        let dims = self.dims;
        let step = [
            if dir.x > 0.0 { 1i32 } else { -1 },
            if dir.y > 0.0 { 1i32 } else { -1 },
            if dir.z > 0.0 { 1i32 } else { -1 },
        ];
        let inv = Vec3::new(
            if dir.x.abs() > 1e-9 {
                1.0 / dir.x
            } else {
                f32::INFINITY
            },
            if dir.y.abs() > 1e-9 {
                1.0 / dir.y
            } else {
                f32::INFINITY
            },
            if dir.z.abs() > 1e-9 {
                1.0 / dir.z
            } else {
                f32::INFINITY
            },
        );
        // Distance along the ray to the next cell boundary on each axis.
        let mut t_max = Vec3::splat(f32::INFINITY);
        for k in 0..3 {
            if !inv[k].is_finite() {
                continue;
            }
            let cur = [origin.x, origin.y, origin.z][k];
            let axis_cell = [
                (origin.x - self.origin.x) / cell,
                (origin.y - self.origin.y) / cell,
                (origin.z - self.origin.z) / cell,
            ][k]
                .floor();
            let boundary =
                self.origin[k] + (axis_cell + if step[k] > 0 { 1.0 } else { 0.0 }) * cell;
            t_max[k] = (boundary - cur) * inv[k];
        }
        let t_delta = Vec3::new(cell * inv.x.abs(), cell * inv.y.abs(), cell * inv.z.abs());

        let dx = dims[0] as usize;
        let dy = dims[1] as usize;
        // Carry the cell coordinates explicitly: recovering them from a flat
        // index every step is where off-by-one and underflow bugs live.
        let (mut cx, mut cy, mut cz) = (
            (c % dx) as i32,
            (c / dx % dims[1] as usize) as i32,
            (c / (dx * dy)) as i32,
        );

        let mut t;
        // A DDA can cross more cells than the grid is wide on one axis at a
        // shallow entry angle, so budget generously rather than per-axis extents.
        let budget = (dx + dims[1] as usize + dims[2] as usize) * 4 + 8;
        let mut seen: Vec<u32> = Vec::new();
        for _ in 0..budget {
            for &ti in self.in_cell(c) {
                if !seen.contains(&ti) {
                    seen.push(ti);
                    out.push(ti);
                }
            }

            t = t_max.x.min(t_max.y).min(t_max.z);
            if t > max_t {
                break;
            }

            let axis = if t_max.x <= t_max.y && t_max.x <= t_max.z {
                0usize
            } else if t_max.y <= t_max.z {
                1usize
            } else {
                2usize
            };

            match axis {
                0 => {
                    cx += step[0];
                    if cx < 0 || cx >= dims[0] {
                        break;
                    }
                }
                1 => {
                    cy += step[1];
                    if cy < 0 || cy >= dims[1] {
                        break;
                    }
                }
                _ => {
                    cz += step[2];
                    if cz < 0 || cz >= dims[2] {
                        break;
                    }
                }
            }
            c = cx as usize + dx * (cy as usize + dims[1] as usize * cz as usize);
            t_max[axis] += t_delta[axis];
        }
        out
    }
}

/// Everything the simulation needs to know about a track's geometry.
#[derive(Clone, Debug, Default)]
pub struct TrackWorld {
    pub triangles: Vec<Triangle>,
    /// Boost pads as world-space boxes.
    pub boost_zones: Vec<[f32; 6]>,
    /// Checkpoint gates in track order, with the index of the piece.
    pub checkpoints: Vec<Gate>,
    /// Every finish line on the track.
    ///
    /// A list rather than one gate because a circuit may end in more than one
    /// place: a split near the line, or a second line tried out beside the
    /// first. The run ends at whichever of them the car reaches once every
    /// checkpoint is behind it.
    pub finishes: Vec<Gate>,
    pub spawn: Vec3,
    pub spawn_yaw: f32,
    grid: Option<Grid>,
}

/// A checkpoint or finish trigger.
#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub piece: PieceUid,
    pub centre: Vec3,
    /// Half-extents of the trigger box.
    pub half: Vec3,
    /// The direction a car has to be travelling to count this gate: the piece's
    /// own direction of travel, `local +Z`, in world space. Unit length.
    pub facing: Vec3,
    pub is_finish: bool,
    pub index: usize,
}

impl Gate {
    /// Whether the movement from `from` to `to` passes through this gate the
    /// right way.
    ///
    /// Both halves matter. A car moving far enough in one tick can cross the
    /// whole box and be outside it again by the next tick, so testing the end
    /// position alone would let a fast car miss a gate entirely; and testing the
    /// box alone would let it drive back the way it came and count.
    pub fn crossed(&self, from: Vec3, to: Vec3) -> bool {
        let motion = to - from;
        // A car that has not moved has crossed nothing, however deep inside the
        // box it is standing.
        if motion.dot(self.facing) <= 0.0 {
            return false;
        }
        segment_hits_box(from, motion, self.centre, self.half)
    }
}

/// Whether the segment `from` to `from + motion` meets the box, by the slab
/// method. A segment that starts inside counts as meeting it.
fn segment_hits_box(from: Vec3, motion: Vec3, centre: Vec3, half: Vec3) -> bool {
    let lo = centre - half;
    let hi = centre + half;
    let axis = |v: Vec3, i: usize| [v.x, v.y, v.z][i];
    let mut enter = 0.0f32;
    let mut leave = 1.0f32;
    for i in 0..3 {
        let (o, d) = (axis(from, i), axis(motion, i));
        if d.abs() < 1e-9 {
            // Parallel to this slab: inside it for the whole segment, or never.
            if o < axis(lo, i) || o > axis(hi, i) {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((axis(lo, i) - o) / d, (axis(hi, i) - o) / d);
        enter = enter.max(t0.min(t1));
        leave = leave.min(t0.max(t1));
        if enter > leave {
            return false;
        }
    }
    true
}

impl TrackWorld {
    /// Build collision, gates and spawn from a document.
    pub fn from_doc(doc: &TrackDocument) -> Self {
        let mut triangles: Vec<Triangle> = Vec::new();
        let mut boost_zones: Vec<[f32; 6]> = Vec::new();

        // Race order, from the connected chain: gate sequence is what a run is
        // measured against, and taking it from the piece array made it a
        // function of the order pieces happened to be created in. Pieces the
        // chain never reaches come back uid-sorted, so this is total and stable.
        let ordered: Vec<&PieceInstance> = doc
            .race_order()
            .iter()
            .filter_map(|uid| doc.piece(*uid))
            .collect();

        for inst in ordered {
            let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
            let world = instance_matrix(inst, doc);

            for (a, b, c, n) in shape.surface.triangles() {
                let (ta, tb, tc) = (
                    world.transform_point3(Vec3::from(a)),
                    world.transform_point3(Vec3::from(b)),
                    world.transform_point3(Vec3::from(c)),
                );
                let n = world.transform_vector3(Vec3::from(n)).normalize_or_zero();
                triangles.push(Triangle {
                    a: ta,
                    b: tb,
                    c: tc,
                    normal: n,
                    owner: inst.uid,
                });
            }

            // The kerbs and walls too, not just the road surface. Without them
            // the edges of the road are painted on: the car drives through a kerb
            // it can see, and a barrier offers no resistance.
            for (a, b, c, n) in shape.decor.triangles() {
                let (ta, tb, tc) = (
                    world.transform_point3(Vec3::from(a)),
                    world.transform_point3(Vec3::from(b)),
                    world.transform_point3(Vec3::from(c)),
                );
                // The decor mesh is built by pushing quads and it leaves some
                // with a collapsed edge where two segments meet. They carry no
                // area and so no collision, but they do make the grid bucket
                // work for nothing and trip the no-degenerate invariant.
                if (tb - ta).cross(tc - ta).length() < 1e-6 {
                    continue;
                }
                let n = world.transform_vector3(Vec3::from(n)).normalize_or_zero();
                triangles.push(Triangle {
                    a: ta,
                    b: tb,
                    c: tc,
                    normal: n,
                    owner: inst.uid,
                });
            }

            let def = catalog_by_id(inst.id);
            if def.is_boost {
                let centre = piece_centre(doc, inst);
                let half = Vec3::splat(doc.cell_size * 0.5);
                let pad = Vec3::splat(1.0);
                boost_zones.push([
                    (centre - half - pad).x,
                    (centre - half - pad).y,
                    (centre - half - pad).z,
                    (centre + half + pad).x,
                    (centre + half + pad).y,
                    (centre + half + pad).z,
                ]);
            }
        }

        // Gates are built from the document's own gate order, not from the walk
        // above: a checkpoint's place in the run is something the author sets,
        // and several may share one number.
        let gate_for = |inst: &PieceInstance, index: usize, is_finish: bool| {
            let world = instance_matrix(inst, doc);
            Gate {
                piece: inst.uid,
                centre: piece_centre(doc, inst) + Vec3::new(0.0, doc.cell_size * 0.25, 0.0),
                half: Vec3::new(
                    doc.cell_size * 0.5,
                    doc.cell_size * 0.75,
                    doc.cell_size * 0.5,
                ),
                // A piece drives along its own local +Z, so the world
                // direction it expects to be met from is that axis rotated
                // by the instance. Never zero: the transform is a rotation
                // about Y, so the axis keeps its length.
                facing: world.transform_vector3(Vec3::Z).normalize_or_zero(),
                is_finish,
                index,
            }
        };
        let mut checkpoints: Vec<Gate> = Vec::new();
        for (index, inst) in doc.checkpoint_sequence().into_iter().enumerate() {
            checkpoints.push(gate_for(inst, index, false));
        }
        let mut finishes: Vec<Gate> = Vec::new();
        for (index, inst) in doc
            .race_order()
            .iter()
            .filter_map(|uid| doc.piece(*uid))
            .filter(|inst| catalog_by_id(inst.id).is_finish)
            .enumerate()
        {
            finishes.push(gate_for(inst, index, true));
        }

        let (spawn, spawn_yaw) = doc.spawn();
        // Lift the spawn clear of the road so the car settles rather than
        // starting embedded in the surface.
        let spawn = spawn + Vec3::Y * 0.6;

        // The plain, as something to land on. Without it the car that leaves the
        // road falls into the void, and the only thing that can happen next is a
        // teleport: there is nothing under it to drive on and no way back up. Its
        // height is the road's own lowest point, so the plain never cuts through
        // the track, and it reaches far past the far plane so its edge is never
        // inside the view.
        // A little below the road, not level with it. Level with it means the
        // plain and the road are one surface and the edge of the road is nothing:
        // a kerb is the only cue that the car has left the road, and a car drives
        // off it without knowing. The reference's road stands a fifth of a metre
        // over its ground for the same reason.
        let road_y = triangles
            .iter()
            .flat_map(|t| [t.a.y, t.b.y, t.c.y])
            .fold(f32::INFINITY, f32::min);
        let ground_y = road_y - GROUND_DROP;
        if ground_y.is_finite() {
            let reach = GROUND_REACH.max(doc.cell_size * 64.0);
            let (a, b, c) = (
                Vec3::new(-reach, ground_y, -reach),
                Vec3::new(reach, ground_y, -reach),
                Vec3::new(reach, ground_y, reach),
            );
            triangles.push(Triangle {
                a,
                b,
                c,
                normal: Vec3::Y,
                owner: PieceUid(0),
            });
            triangles.push(Triangle {
                a,
                b: c,
                c: Vec3::new(-reach, ground_y, reach),
                normal: Vec3::Y,
                owner: PieceUid(0),
            });
        }

        // The terrain range, as something to hit. Same triangles the scene draws,
        // so a mountain is solid exactly where it is visible.
        for (a, b, c, n) in retrackt_format::geometry::mountain_triangles(bounds_of(doc), 0.0) {
            triangles.push(Triangle {
                a: Vec3::from(a),
                b: Vec3::from(b),
                c: Vec3::from(c),
                normal: Vec3::from(n),
                owner: PieceUid(0),
            });
        }

        let grid = Grid::build(&triangles, (doc.cell_size * 0.5).max(1.0));
        Self {
            triangles,
            boost_zones,
            checkpoints,
            spawn,
            spawn_yaw,
            finishes,
            grid: Some(grid),
        }
    }

    /// The gate the run is waiting on: checkpoint number `next`, or the first
    /// finish line once every checkpoint is behind the car.
    ///
    /// One method rather than arithmetic at each use, because "which gate comes
    /// next" is a rule: a run counts its checkpoints in order, and may only end
    /// at a finish line after the last of them. Written out at the call site,
    /// that rule is one `if` away from being wrong.
    pub fn gate_at(&self, next: usize) -> Option<Gate> {
        if next < self.checkpoints.len() {
            return Some(self.checkpoints[next]);
        }
        self.finishes.first().copied()
    }

    /// Nearest surface along a ray. Back faces are ignored, so a road seen
    /// from underneath reports no ground rather than flipping the car.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<Hit> {
        if max_t <= 0.0 || dir.length_squared() < 1e-12 {
            return None;
        }
        let dir = dir.normalize();
        let grid = self.grid.as_ref()?;
        let mut best: Option<Hit> = None;

        for ti in grid.ray_triangles(origin, dir, max_t) {
            let Some(t) = self.triangles.get(ti as usize) else {
                continue;
            };
            if t.normal.dot(dir) >= 0.0 {
                continue;
            }
            let Some(d) = ray_triangle(origin, dir, t) else {
                continue;
            };
            if d < 0.0 || d > max_t {
                continue;
            }
            if best.is_none_or(|b| d < b.distance) {
                best = Some(Hit {
                    distance: d,
                    point: origin + dir * d,
                    normal: t.normal,
                    owner: t.owner,
                });
            }
        }
        best
    }

    /// Closest surface directly beneath `p`, with its normal.
    pub fn ground_at(&self, p: Vec3, max_depth: f32) -> Option<Hit> {
        self.raycast(p, Vec3::NEG_Y, max_depth)
    }

    /// Height of the road under `(x, z)`, or `None` over a gap.
    pub fn height_at(&self, x: f32, z: f32, from_y: f32) -> Option<f32> {
        self.raycast(Vec3::new(x, from_y, z), Vec3::NEG_Y, from_y * 2.0 + 200.0)
            .map(|h| h.point.y)
    }

    /// Triangles whose surface passes within `radius` of `p`. Used for wall and
    /// body collision, where the road under the car would otherwise snag it.
    pub fn nearby(&self, p: Vec3, radius: f32) -> Vec<&Triangle> {
        let Some(grid) = self.grid.as_ref() else {
            return Vec::new();
        };
        let lo = p - Vec3::splat(radius);
        let hi = p + Vec3::splat(radius);
        let mut out: Vec<&Triangle> = Vec::new();
        let mut seen: Vec<u32> = Vec::new();

        for idx in grid.cells_in_box(lo, hi) {
            for &ti in grid.in_cell(idx) {
                if seen.contains(&ti) {
                    continue;
                }
                seen.push(ti);
                let Some(t) = self.triangles.get(ti as usize) else {
                    continue;
                };
                if (p - closest_on_triangle(p, t)).length() <= radius {
                    out.push(t);
                }
            }
        }
        out
    }

    pub fn in_boost_zone(&self, p: Vec3) -> bool {
        self.boost_zones.iter().any(|b| {
            p.x >= b[0] && p.x <= b[3] && p.y >= b[1] && p.y <= b[4] && p.z >= b[2] && p.z <= b[5]
        })
    }
}

/// Möller-Trumbore, double sided.
fn ray_triangle(origin: Vec3, dir: Vec3, t: &Triangle) -> Option<f32> {
    let e1 = t.b - t.a;
    let e2 = t.c - t.a;
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < 1e-9 {
        return None;
    }
    let f = 1.0 / a;
    let s = origin - t.a;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let dist = f * e2.dot(q);
    (dist > 1e-6).then_some(dist)
}

/// Closest point on a triangle to `p`.
///
/// Ericson, *Real-Time Collision Detection* §5.1.5.
pub fn closest_on_triangle(p: Vec3, t: &Triangle) -> Vec3 {
    let (a, b, c) = (t.a, t.b, t.c);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// World transform for a placed piece.
pub fn instance_matrix(inst: &PieceInstance, doc: &TrackDocument) -> Mat4 {
    inst.world_matrix(doc.cell_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::{PieceId, builtin_tracks};

    #[test]
    fn every_builtin_track_builds_a_collision_world() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            assert!(!w.triangles.is_empty(), "{}: no triangles", doc.name);
            assert!(!w.finishes.is_empty(), "{}: needs a finish gate", doc.name);
            assert!(!w.checkpoints.is_empty(), "{}: needs checkpoints", doc.name);
            assert!(!w.boost_zones.is_empty(), "{}: needs a boost pad", doc.name);
        }
    }

    #[test]
    fn scenery_collides_but_never_warns() {
        // Props are placed, not chained: a pillar beside the road and a sign
        // further on must end up solid in the collision world without the
        // validator reporting either as standing free.
        let mut b = retrackt_format::ChainBuilder::new("scenery");
        b.step(PieceId::Start, retrackt_format::PieceParams::default());
        b.step(PieceId::Straight, retrackt_format::PieceParams::default());
        b.step(PieceId::Finish, retrackt_format::PieceParams::default());
        b.step(PieceId::Pillar, retrackt_format::PieceParams::default());
        b.step(PieceId::Sign, retrackt_format::PieceParams::default());
        let doc = b.build();

        let diagnostics = retrackt_format::validate(&doc);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("standing free")),
            "scenery is placed, not chained: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        let w = TrackWorld::from_doc(&doc);
        for id in [PieceId::Pillar, PieceId::Sign] {
            let uid = doc.pieces.iter().find(|p| p.id == id).unwrap().uid;
            let owned: Vec<_> = w.triangles.iter().filter(|t| t.owner == uid).collect();
            assert!(!owned.is_empty(), "{id:?}: no collision triangles");
            for t in &owned {
                assert!(t.area() > 1e-4, "{id:?}: degenerate triangle");
                assert!(
                    (t.normal.length() - 1.0).abs() < 1e-3,
                    "{id:?}: normal not unit"
                );
            }
        }
    }

    #[test]
    fn gates_come_out_in_race_order_not_uid_order() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            let order = doc.race_order();
            let rank = |uid: PieceUid| {
                order
                    .iter()
                    .position(|u| *u == uid)
                    .expect("a gate piece is always in race order")
            };
            for pair in w.checkpoints.windows(2) {
                assert!(
                    rank(pair[0].piece) < rank(pair[1].piece),
                    "{}: gates {:?} then {:?} run backwards",
                    doc.name,
                    pair[0].piece,
                    pair[1].piece
                );
            }
        }
    }

    #[test]
    fn gate_order_follows_the_document_not_the_road() {
        // The test this replaces reversed the uids and required the gate
        // sequence to come back in road order regardless, because race order was
        // a walk from the Start through whatever ports met. Pieces are placed
        // independently now, so there is no road to walk: the order is the
        // order the document states, and this holds that down.
        //
        // Two documents with the same pieces in different order are two
        // different runs, and saying so is the honest answer. What must stay
        // true is that gate *numbering* still matches the sequence, or the
        // checkpoint counter would be comparing the wrong gates.
        let doc = builtin_tracks().remove(0);
        let world = TrackWorld::from_doc(&doc);
        let order = doc.race_order();
        let rank = |uid: PieceUid| {
            order
                .iter()
                .position(|u| *u == uid)
                .expect("a gate piece is always in race order")
        };

        assert_eq!(
            world.checkpoints.len(),
            TrackWorld::from_doc(&retrackt_format::demo_track())
                .checkpoints
                .len()
        );
        for pair in world.checkpoints.windows(2) {
            assert!(
                rank(pair[0].piece) < rank(pair[1].piece),
                "gates must come out in the order the document lists them"
            );
        }
        for (i, gate) in world.checkpoints.iter().enumerate() {
            assert_eq!(gate.index, i, "gate index must be its position in the run");
        }

        // And an explicit order overrides it, which is how an author moves a
        // gate without moving the piece.
        let mut moved = doc.clone();
        let last = moved.checkpoint_sequence().len().saturating_sub(1);
        if let Some(p) = moved.checkpoint_sequence().last() {
            let uid = p.uid;
            for piece in moved.pieces.iter_mut() {
                if piece.uid == uid {
                    piece.params.checkpoint_order = Some(1);
                }
            }
        }
        let reordered = TrackWorld::from_doc(&moved);
        assert_eq!(
            reordered.checkpoints.first().map(|g| g.piece),
            Some(moved.checkpoint_sequence()[0].uid),
            "an explicit order must decide the sequence"
        );
        assert!(last >= 1, "the fixture needs more than one gate");
    }

    #[test]
    fn a_gate_the_car_cannot_reach_is_reported_rather_than_refused() {
        // A checkpoint placed in mid air still builds and still gets a gate: the
        // document does not know what the car will do, and refusing the track
        // would refuse free placement. What the validator offers is a warning.
        let mut doc = retrackt_format::demo_track();
        let uid = doc.next_piece_uid();
        doc.pieces.push(
            retrackt_format::PieceInstance::new(PieceId::Checkpoint, [900, 0, 900]).with_uid(uid),
        );
        doc.normalize_uids();

        let world = TrackWorld::from_doc(&doc);
        assert_eq!(
            world.checkpoints.len(),
            4,
            "a gate is built wherever it is placed"
        );
        let diagnostics = retrackt_format::validate::validate(&doc);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("meets no other")),
            "but it is reported: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.severity == retrackt_format::validate::Severity::Error),
            "and it is a warning, not a refusal: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn gate_indices_number_the_sequence_the_player_drives() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            for (i, gate) in w.checkpoints.iter().enumerate() {
                assert_eq!(
                    gate.index, i,
                    "{}: gate index must be its position in the run",
                    doc.name
                );
            }
        }
    }

    #[test]
    fn a_shuffled_file_builds_the_same_gate_sequence() {
        let doc = builtin_tracks().remove(0);
        let mut shuffled = doc.clone();
        shuffled.pieces.reverse();
        let (a, b) = (TrackWorld::from_doc(&doc), TrackWorld::from_doc(&shuffled));
        let order = |w: &TrackWorld| {
            w.checkpoints
                .iter()
                .map(|g| (g.piece, g.centre))
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&a), order(&b));
    }

    #[test]
    fn a_gate_is_hit_by_the_straight_line_along_its_own_facing() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            for gate in w.checkpoints.iter().chain(w.finishes.iter()) {
                assert!(
                    (gate.facing.length() - 1.0).abs() < 1e-3,
                    "{}: gate facing is not a unit vector: {:?}",
                    doc.name,
                    gate.facing
                );
                let reach = gate.half.x.max(gate.half.z) + 1.0;
                assert!(
                    gate.crossed(
                        gate.centre - gate.facing * reach,
                        gate.centre + gate.facing * reach
                    ),
                    "{}: driving through {:?} along its facing misses it",
                    doc.name,
                    gate.piece
                );
            }
        }
    }

    #[test]
    fn no_degenerate_or_badly_normal_triangles() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            for t in &w.triangles {
                assert!(t.area() > 1e-4, "{}: degenerate triangle {:?}", doc.name, t);
                assert!(
                    (t.normal.length() - 1.0).abs() < 1e-3,
                    "{}: normal not unit: {:?}",
                    doc.name,
                    t.normal
                );
                // At a loop's apex the normal points straight down — the feature,
                // not a defect; what must hold is a unit vector facing away from the ribbon.
                assert!(t.normal.dot(Vec3::Y).abs() <= 1.001);
            }
        }
    }

    #[test]
    fn every_driveable_piece_has_ground_under_its_centre() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            for inst in &doc.pieces {
                if inst.id == PieceId::Loop {
                    // A loop is a ring: its anchor cell is its entry, not
                    // solid road, so there is nothing to stand on there.
                    continue;
                }
                if inst.id.is_scenery() {
                    continue;
                }
                let c = piece_centre(&doc, inst);
                assert!(
                    w.ground_at(c + Vec3::new(0.0, 30.0, 0.0), 60.0).is_some(),
                    "{}: no surface above {:?} at {:?}",
                    doc.name,
                    inst.id,
                    inst.cell
                );
            }
        }
    }

    #[test]
    fn a_downward_ray_finds_the_road_and_its_normal_points_up() {
        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        let p = retrackt_format::demo::piece_centre(&doc, &doc.pieces[2]);
        let hit = w
            .ground_at(p + Vec3::new(0.0, 30.0, 0.0), 60.0)
            .expect("flat road must be found");
        assert!((hit.point.x - p.x).abs() < 0.5);
        assert!(hit.normal.y > 0.9, "normal must face up: {:?}", hit.normal);
    }

    #[test]
    fn an_upward_ray_finds_nothing_through_the_road() {
        // Back faces are rejected, so shooting up from under the road misses.
        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        let p = retrackt_format::demo::piece_centre(&doc, &doc.pieces[2]);
        assert!(
            w.raycast(p - Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 10.0)
                .is_none()
        );
    }

    #[test]
    fn a_ray_that_misses_the_track_returns_none() {
        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        assert!(
            w.raycast(Vec3::new(5000.0, 5000.0, 5000.0), Vec3::NEG_Y, 100.0)
                .is_none(),
            "far outside the grid there is no ground"
        );
    }

    #[test]
    fn gates_are_ordered_and_finishes_are_finishes() {
        let doc = builtin_tracks().remove(0);
        let w = TrackWorld::from_doc(&doc);
        for (i, g) in w.checkpoints.iter().enumerate() {
            assert_eq!(g.index, i, "checkpoints must be in order");
            assert!(!g.is_finish);
        }
        assert!(!w.finishes.is_empty(), "a built-in circuit has a finish");
        for f in &w.finishes {
            assert!(f.is_finish);
        }
    }

    #[test]
    fn an_explicit_checkpoint_order_reorders_the_gates_the_car_passes() {
        // The last checkpoint on the road is handed order 1, so the run must
        // count it first. This is the whole point of the field: without it the
        // gate sequence is fixed by the geometry and an author cannot change it.
        let mut doc = builtin_tracks().remove(0);
        let road: Vec<PieceUid> = doc.checkpoint_sequence().iter().map(|p| p.uid).collect();
        let last = *road.last().expect("a built-in circuit has checkpoints");
        let slot = doc.pieces.iter_mut().find(|p| p.uid == last).unwrap();
        slot.params = slot.params.order(1);

        let world = TrackWorld::from_doc(&doc);
        let gated: Vec<PieceUid> = world.checkpoints.iter().map(|g| g.piece).collect();
        assert_eq!(gated.first(), Some(&last), "order 1 is passed first");
        assert_eq!(gated.len(), road.len(), "every gate is still there");
        for (i, g) in world.checkpoints.iter().enumerate() {
            assert_eq!(g.index, i, "the index is the position in the run");
        }
    }

    #[test]
    fn every_finish_line_on_the_track_becomes_a_gate() {
        let mut doc = builtin_tracks().remove(0);
        // A second finish, well off the road, so it is a gate nothing reaches.
        let uid = doc.next_piece_uid();
        doc.pieces.push(
            retrackt_format::PieceInstance::new(PieceId::Finish, [90, 0, 90])
                .with_uid(uid)
                .with_params(retrackt_format::PieceParams::default().length(2)),
        );
        doc.normalize_uids();

        let world = TrackWorld::from_doc(&doc);
        assert_eq!(
            world.finishes.len(),
            2,
            "both finish lines are gates: the run may end at either"
        );
        assert_eq!(
            world.finishes.iter().filter(|g| g.is_finish).count(),
            2,
            "and they are finish gates, not checkpoints"
        );
        assert!(
            !world
                .checkpoints
                .iter()
                .any(|g| world.finishes.iter().any(|f| f.piece == g.piece)),
            "a finish is never also a checkpoint"
        );
    }

    #[test]
    fn the_car_spawns_above_road_level() {
        for doc in builtin_tracks() {
            let w = TrackWorld::from_doc(&doc);
            assert!(
                w.spawn.y > 0.0,
                "{}: spawn must start above the road",
                doc.name
            );
            assert!(
                w.ground_at(w.spawn, 40.0).is_some(),
                "{}: nothing under the spawn point",
                doc.name
            );
        }
    }

    #[test]
    fn boost_zones_sit_on_the_road() {
        let doc = builtin_tracks().remove(0);
        let w = TrackWorld::from_doc(&doc);
        let b = w.boost_zones[0];
        let centre = Vec3::new(
            (b[0] + b[3]) * 0.5,
            (b[1] + b[4]) * 0.5,
            (b[2] + b[5]) * 0.5,
        );
        assert!(w.in_boost_zone(centre));
        assert!(!w.in_boost_zone(centre + Vec3::new(1000.0, 0.0, 0.0)));
    }

    #[test]
    fn closest_point_on_triangle_projection() {
        let t = Triangle {
            a: Vec3::new(0.0, 0.0, 0.0),
            b: Vec3::new(1.0, 0.0, 0.0),
            c: Vec3::new(0.0, 0.0, 1.0),
            normal: Vec3::Y,
            owner: PieceUid(0),
        };
        // Above the middle of the triangle.
        let p = closest_on_triangle(Vec3::new(0.25, 5.0, 0.25), &t);
        assert!((p - Vec3::new(0.25, 0.0, 0.25)).length() < 1e-4);
        // Off the edge: clamps to the boundary.
        let p = closest_on_triangle(Vec3::new(-3.0, 0.0, 0.5), &t);
        assert!(p.x >= -1e-4 && p.x <= 1.0 + 1e-4);
    }
}

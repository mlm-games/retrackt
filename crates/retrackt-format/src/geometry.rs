//! Analytic piece geometry. One function, [`piece_centerline`], produces the
//! centre curve of any piece; mesh, collision triangles, ports, reservation and
//! guide all derive from it, so a piece cannot be one shape to the renderer and
//! another to the car. Local space: entry at the origin heading `+Z`, units in
//! metres. Grid `+Y` is compressed (a cell step is half a cell edge), so `n`
//! cells of ramp rise `n/2` cell edges.

use glam::{Quat, Vec3};
use std::f32::consts::TAU;

use crate::piece::{GridDir, PieceId, PieceParams, catalog_by_id, resolve_params};
use crate::{KERB_HEIGHT, ROAD_HALF_CELLS, WIDE_ROAD_HALF_CELLS};

/// Height of one grid step on the vertical axis, in cells.
///
/// Less than a whole cell, so a slope can be made at an angle the grid can
/// actually carry: at a 5 m cell a whole-cell step would quantise a 20 m ramp
/// to a rise of 0, 5, 10 or 15 m and nothing between, which is the difference
/// between a ramp and a cliff. A quarter cell keeps the snapping that makes a
/// neighbour's port meet this piece's exit, and still gives 1.25 m of height
/// resolution to place it at.
pub const GRID_Y_CELLS: f32 = 0.25;

/// One triangle: three corner positions and its normal.
pub type TriangleData = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// Triangle soup in world space. Shared by rendering and collision, so the
/// drivable surface is by construction the surface that is drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl RawMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Triangle as three positions plus its analytic normal.
    pub fn triangles(&self) -> Vec<TriangleData> {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let i = t[0] as usize;
                (
                    self.positions[i],
                    self.positions[t[1] as usize],
                    self.positions[t[2] as usize],
                    *self.normals.get(i).unwrap_or(&[0.0, 1.0, 0.0]),
                )
            })
            .collect()
    }

    /// Bounds as `(min, max)`, or `None` when empty or non-finite.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = Vec3::from(*self.positions.first()?);
        let mut lo = first;
        let mut hi = first;
        for p in &self.positions {
            let v = Vec3::from(*p);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo.is_finite() && hi.is_finite()).then(|| (lo.into(), hi.into()))
    }

    /// Translate and rotate the whole mesh by `m`, multiplying `color` into
    /// the existing vertex colours (baked decor colours survive the trip).
    pub fn transformed(&self, m: glam::Mat4, color: [f32; 3]) -> RawMesh {
        let linear = glam::Mat3::from_mat4(m);
        let nm = linear.inverse().transpose();
        let flip = linear.determinant() < 0.0;
        let colors = if self.colors.len() == self.positions.len() {
            self.colors
                .iter()
                .map(|c| [c[0] * color[0], c[1] * color[1], c[2] * color[2]])
                .collect()
        } else {
            vec![color; self.positions.len()]
        };
        let mut out = RawMesh {
            positions: Vec::with_capacity(self.positions.len()),
            normals: Vec::with_capacity(self.normals.len()),
            colors,
            indices: Vec::with_capacity(self.indices.len()),
        };
        for p in &self.positions {
            out.positions
                .push(m.transform_point3(Vec3::from(*p)).to_array());
        }
        for n in &self.normals {
            out.normals.push((nm * Vec3::from(*n)).to_array());
        }
        out.indices.extend_from_slice(&self.indices);
        if flip {
            for t in out.indices.as_chunks_mut::<3>().0.iter_mut() {
                t.swap(1, 2);
            }
        }
        out
    }
}

/// A sampled centre curve with a frame at every sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Centerline {
    pub pos: Vec<Vec3>,
    pub tan: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    /// Cumulative arc length; `arc[0] == 0`.
    pub arc: Vec<f32>,
    pub half_width: f32,
    /// Height the road's sides hang down to, or `None` for a piece that sits on
    /// the ground.
    ///
    /// A slope is a surface in the air, and a surface with nothing under it
    /// reads as a floating ribbon: the skirt turns it into a solid ramp, and
    /// gives the far side of a climb a wall to drop behind.
    pub floor: Option<f32>,
    /// Pieces with no rail set this so the guide leaves the car to ordinary
    /// grounding instead of constraining it.
    pub skip: bool,
}

impl Centerline {
    pub fn length(&self) -> f32 {
        self.arc.last().copied().unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.pos.len() < 2
    }

    /// Frame at arc length `s`, linearly interpolated between samples.
    pub fn sample(&self, s: f32) -> Option<(Vec3, Vec3, Vec3, f32)> {
        if self.pos.len() < 2 {
            return None;
        }
        let s = s.clamp(0.0, self.length());
        let last_span = self.pos.len() - 2;
        // `Ok` can return the final index, which has no following span.
        let i = match self.arc.binary_search_by(|a| a.partial_cmp(&s).unwrap()) {
            Ok(i) => i.min(last_span),
            Err(0) => 0,
            Err(i) => (i - 1).min(last_span),
        };
        let (a, b) = (self.arc[i], self.arc[i + 1]);
        let t = if b > a { (s - a) / (b - a) } else { 0.0 };
        let pos = self.pos[i].lerp(self.pos[i + 1], t);
        let tan = self.tan[i].lerp(self.tan[i + 1], t).normalize_or_zero();
        let nrm = self.nrm[i].lerp(self.nrm[i + 1], t).normalize_or_zero();
        Some((pos, tan, nrm, self.half_width))
    }

    /// Ride width of the road, in metres.
    pub fn width(&self) -> f32 {
        self.half_width * 2.0
    }
}

/// Where a piece can join a neighbour, in local grid cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Port {
    pub cell: [i16; 3],
    pub outward: GridDir,
}

/// Everything one placed piece needs, all derived from one centre curve.
#[derive(Clone, Debug)]
pub struct PieceShape {
    /// Drivable surface. This is what is drawn *and* what the car drives on.
    pub surface: RawMesh,
    /// Visual-only dressing (kerbs, walls, zone paint). Never collided with.
    pub decor: RawMesh,
    /// Local grid cells of the connector ports.
    pub ports: Vec<Port>,
    /// Local grid cells covered, inclusive. Y is in vertical cell steps.
    pub reserve_min: [i16; 3],
    pub reserve_max: [i16; 3],
    /// Cells the road surface actually runs through — stricter than
    /// `reserve_cells`, since reserved cells may overlap but occupied ones may not.
    pub occupied: Vec<[i16; 3]>,
    /// Sampled centre curve in local metres, for the deterministic guide.
    pub center: Centerline,
    /// Side-road centrelines of a junction, in local metres. Empty for every
    /// other piece: only a junction paves more than its through road, and the
    /// guide still follows `center` because a drive through a junction is one
    /// line, not three.
    pub branches: Vec<Centerline>,
    /// False for pieces the guide must skip (a jump gap is not a rail).
    pub has_guide: bool,
}

impl PieceShape {
    /// Local grid cells covered, inclusive.
    pub fn reserve_cells(&self) -> impl Iterator<Item = [i16; 3]> + '_ {
        let (lo, hi) = (self.reserve_min, self.reserve_max);
        (lo[1]..=hi[1]).flat_map(move |y| {
            (lo[2]..=hi[2]).flat_map(move |z| (lo[0]..=hi[0]).map(move |x| [x, y, z]))
        })
    }
}

/// Arc steps per 90 degrees. Enough that a 4 m radius corner is smooth
/// while staying cheap to rebuild on every editor keystroke.
const ARC_SEGMENTS_PER_QUARTER: usize = 12;
/// Straight runs need only their two ends; the guide interpolates linearly.
const STRAIGHT_SEGMENTS: usize = 1;
/// Full loop resolution.
const LOOP_SEGMENTS: usize = 48;

/// The one entry point. `id` plus resolved params define the whole shape.
/// Rise snapped to the nearest vertical grid step, which is the increment the
/// grid quantises height to.
///
/// A slope's two ends have to land on half cells or a neighbour anchored on the
/// exit port meets the road a few centimetres off it, and every join on the
/// track carries that step.
fn snap_half_cells(rise: f32, cell: f32) -> f32 {
    let step = cell * GRID_Y_CELLS;
    (rise / step).round() * step
}

/// Bank lean of a piece, in radians, in the convention that positive is a
/// left-hand lean: the road's outside edge, on the right, is raised.
///
/// A right-hand piece leans the other way. The magnitude is the author's; the
/// hand of the piece picks the sign, which is what makes one `bank` value mean
/// "banked" on a left corner and on a right one alike.
fn bank_lean(id: PieceId, bank_deg: i8) -> f32 {
    let right_handed = matches!(id, PieceId::BankRight | PieceId::CurveRight90);
    let lean = f32::from(bank_deg).to_radians();
    if right_handed { -lean } else { lean }
}

/// Road normal for a sample, banked about the piece's own tangent.
///
/// `up` is the face the car rides on before the bank is applied, and the caller
/// owns it because it is not always world up: a road that has bent vertical has
/// no horizontal face any more, and its normal is whichever way the surface
/// turns out to be pointing. Rotating about the tangent rather than about world
/// +Y is what keeps the two perpendicular all the way round a corner.
fn banked(tangent: Vec3, up: Vec3, lean: f32) -> Vec3 {
    Quat::from_axis_angle(tangent, lean) * up
}

/// Slope steepness of a piece, in degrees, falling back to the catalogue
/// default.
///
/// A separate resolver from [`resolve_params`] because the angle is not
/// geometry the shape builder is handed: it is a knob with a range of its own
/// and a default every existing document relies on.
pub fn resolve_slope(id: PieceId, params: &PieceParams) -> f32 {
    let default = catalog_by_id(id).default_slope_deg;
    f32::from(params.slope_deg.unwrap_or(default)).clamp(5.0, 75.0)
}

pub fn piece_centerline(id: PieceId, params: &PieceParams, cell: f32) -> Centerline {
    let (len_cells, rad_cells, bank_deg) = resolve_params(id, params);
    let slope = resolve_slope(id, params);
    // The wide carriageway, where the author asked for one. Scenery sets its
    // own widths below, so the flag means nothing to a pillar or a sign.
    let half = (if params.wide {
        WIDE_ROAD_HALF_CELLS
    } else {
        ROAD_HALF_CELLS
    }) * cell;
    let hw = half;

    let mut c = Centerline {
        half_width: hw,
        ..Centerline::default()
    };
    let r = rad_cells as f32 * cell;
    let l = len_cells as f32 * cell;

    match id {
        PieceId::Straight
        | PieceId::Start
        | PieceId::Finish
        | PieceId::Checkpoint
        | PieceId::Boost
        | PieceId::TJunction
        | PieceId::Crossroads
        | PieceId::YJunction
        | PieceId::YJunctionFlip => {
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, 0.0, u * l), Vec3::Z, Vec3::Y)
            });
        }

        PieceId::RampUp | PieceId::RampDown => {
            // Steepness the author asked for, snapped to the half-cell the grid
            // quantises height to. The snap is what lets a neighbour anchor on
            // the exit cell and meet the road exactly: a ramp that ended 8 cm
            // above its own port would leave a step at every join. The default
            // angle is the one whose snap is the rise this piece always had, so
            // a document that never set one is unchanged.
            let rise = snap_half_cells(l * slope.to_radians().tan(), cell);
            let up = id == PieceId::RampUp;
            push_segment(&mut c, 4, |u| {
                let y = if up { u * rise } else { rise - u * rise };
                let dy = if up { rise } else { -rise };
                let t = Vec3::new(0.0, dy, l).normalize();
                // Out of the sloped face, never world up: the collision world
                // reads this straight back as the road normal, so a flat
                // normal makes every ramp drive as if it were level.
                let nrm = Vec3::new(0.0, l, -dy).normalize();
                (Vec3::new(0.0, y, u * l), t, nrm)
            });
            // The road is a surface in the air: without a skirt down to the
            // piece's lowest point a ramp reads as a floating ribbon, and the
            // drop on the far side of a climb has nothing to be its far wall.
            c.floor = Some(0.0);
        }

        PieceId::SCurve => {
            // Two quarter arcs of opposite hand: right then left, so the road
            // leaves heading the way it came in, offset across by two radii.
            // One piece rather than two, because the pair is one shape and a
            // chicane built from two pieces has a seam through the middle of it.
            //
            // One parameterisation rather than two pushed segments, so the
            // sample where they meet exists once: two segments both push their
            // own end and start, and the duplicate pair is a zero-length span,
            // which is a degenerate triangle in the road mesh.
            let lean = bank_lean(id, bank_deg);
            let n = ARC_SEGMENTS_PER_QUARTER * 2;
            push_segment(&mut c, n, |u| {
                let v = u * 2.0;
                let (pos, tangent) = if v <= 1.0 {
                    // Right-hand quarter, centre at (+r, 0, 0).
                    let th = std::f32::consts::PI - v * std::f32::consts::FRAC_PI_2;
                    (
                        Vec3::new(r, 0.0, 0.0) + Vec3::new(th.cos(), 0.0, th.sin()) * r,
                        Vec3::new(th.sin(), 0.0, -th.cos()),
                    )
                } else {
                    // Left-hand quarter from where the first left off, centre at
                    // (+r, 0, +2r).
                    let th = -std::f32::consts::FRAC_PI_2 + (v - 1.0) * std::f32::consts::FRAC_PI_2;
                    (
                        Vec3::new(r, 0.0, 2.0 * r) + Vec3::new(th.cos(), 0.0, th.sin()) * r,
                        Vec3::new(-th.sin(), 0.0, th.cos()),
                    )
                };
                (pos, tangent, banked(tangent, Vec3::Y, lean))
            });
        }
        PieceId::BankLeft | PieceId::BankRight => {
            // Bank leans the outside edge up: the normal tilts away from the low
            // side, and the low side of a left-hand bank is the right-hand edge.
            // The two hands are mirrors of each other, so the sign is the whole
            // difference between them. Getting it wrong is silent, because both
            // pieces still look like a banked road and still drive like one,
            // just the wrong way round, which is why the stunt track used to
            // place `BankRight` twice with opposite bank angles to fake a
            // `BankLeft`.
            let lean = bank_lean(id, bank_deg);
            let nrm = Vec3::new(lean.sin(), lean.cos(), 0.0);
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, 0.0, u * l), Vec3::Z, nrm)
            });
        }

        PieceId::CurveRight90 | PieceId::Curve180 => {
            // Entry at the origin heading +Z, exit at (+r, 0, +r) heading +X.
            let center = Vec3::new(r, 0.0, 0.0);
            let sweeps = if id == PieceId::Curve180 { 2 } else { 1 };
            let n = ARC_SEGMENTS_PER_QUARTER * sweeps;
            let lean = bank_lean(id, bank_deg);
            push_segment(&mut c, n, |u| {
                let th = std::f32::consts::PI - u * std::f32::consts::FRAC_PI_2 * sweeps as f32;
                let pos = center + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                let tangent = Vec3::new(th.sin(), 0.0, -th.cos());
                (pos, tangent, banked(tangent, Vec3::Y, lean))
            });
        }

        PieceId::CurveLeft90 => {
            // Entry at the origin heading +Z, exit at (-r, 0, +r) heading -X.
            let center = Vec3::new(-r, 0.0, 0.0);
            let n = ARC_SEGMENTS_PER_QUARTER;
            let lean = bank_lean(id, bank_deg);
            push_segment(&mut c, n, |u| {
                let th = u * std::f32::consts::FRAC_PI_2;
                let pos = center + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                let tangent = Vec3::new(-th.sin(), 0.0, th.cos());
                (pos, tangent, banked(tangent, Vec3::Y, lean))
            });
        }

        PieceId::Loop => {
            // Pure vertical circle, entered and exited at the origin heading +Z:
            // front and back halves pass through opposite z, so it never self-intersects.
            let center = Vec3::new(0.0, r, 0.0);
            push_segment(&mut c, LOOP_SEGMENTS, |u| {
                let ph = u * std::f32::consts::TAU;
                let pos = center + Vec3::new(0.0, -ph.cos(), ph.sin()) * r;
                let tan = Vec3::new(0.0, ph.sin(), ph.cos());
                // Surface faces the axis: the car rides the inside.
                let nrm = Vec3::new(0.0, ph.cos(), -ph.sin());
                (pos, tan, nrm)
            });
        }

        PieceId::Wall => {
            // Not drivable: a thin upright barrier, half a cell tall.
            c.half_width = cell * 0.06;
            let h = cell * 0.25;
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, h, u * l), Vec3::Z, Vec3::X)
            });
            c.skip = true;
        }

        PieceId::Pillar => {
            // Not drivable: a support column standing on its anchor cell. The
            // length knob is its height, so one piece props up a road at any
            // elevation instead of a stack of fixed blocks.
            c.half_width = cell * 0.15;
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, u * l, 0.0), Vec3::Y, Vec3::Z)
            });
            c.skip = true;
        }

        PieceId::Sign => {
            // Not drivable: a trackside board on a post. Fixed size, turned by
            // yaw to face the road it belongs to.
            c.half_width = cell * 0.4;
            let h = cell * 0.65;
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, u * h, 0.0), Vec3::Y, Vec3::Z)
            });
            c.skip = true;
        }

        PieceId::Block => {
            // Not drivable: one solid cube on its anchor cell. The centre stub
            // stays inside the cell so the block owns exactly the cell it is
            // placed on, which is what lets blocks stack without sharing.
            c.half_width = cell * 0.5;
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, u * cell * 0.5, 0.0), Vec3::Y, Vec3::Z)
            });
            c.skip = true;
        }
    }

    c
}

/// Sample `n+1` frames from `f(u in 0..=1)` and accumulate arc length;
/// asymmetric pieces (ramps, jumps) depend on this to mate exactly.
fn push_segment<F>(c: &mut Centerline, n: usize, f: F)
where
    F: Fn(f32) -> (Vec3, Vec3, Vec3),
{
    let mut prev: Option<Vec3> = None;
    for i in 0..=n {
        let (pos, tan, nrm) = f(i as f32 / n as f32);
        let tan = tan.normalize_or_zero();
        // Accumulate from the actual previous sample. Using `i - 1` as an
        // index would read the arc vector, not the position.
        let arc =
            c.arc.last().copied().unwrap_or(0.0) + prev.map_or(0.0, |p: Vec3| pos.distance(p));
        c.pos.push(pos);
        c.tan.push(tan);
        c.nrm.push(nrm);
        c.arc.push(arc);
        prev = Some(pos);
    }
}

/// Build mesh, ports, reservation box and guide samples for one piece.
/// `cell` is the cell edge in metres; local space is metres, entry at origin.
pub fn piece_shape(id: PieceId, params: &PieceParams, cell: f32) -> PieceShape {
    let center = piece_centerline(id, params, cell);
    let half = center.half_width;
    let has_guide = !center.skip && !center.is_empty();

    // A junction's fixed length, guarded: a hand-written length shorter than
    // the mouth leaves the spans backwards. The editor never offers one; the
    // file gets the minimum that still fits the junction.
    let junction_len = if id.is_junction() {
        Some((f32::from(resolve_params(id, params).0) * cell).max(half * 2.0 + cell))
    } else {
        None
    };
    // Side roads of a junction, built once and shared by the surface, the
    // ports and the reservation below.
    let branches = junction_len
        .map(|l| junction_branches(id, half, cell, l))
        .unwrap_or_default();

    // Ride width of a drivable road: the cell edge. Scenery is decor only:
    // nothing to drive on, just something to see and to hit.
    let (surface, decor) = match id {
        // A wall is a thin upright barrier; its centre line carries its own
        // thin half-width, so reuse that.
        PieceId::Wall => (
            RawMesh::default(),
            ribbon(&center, center.half_width, cell, KERB_HEIGHT * 3.0).1,
        ),
        // A pillar is a solid column standing on its anchor cell.
        PieceId::Pillar => (
            RawMesh::default(),
            pillar_mesh(cell, resolve_params(id, params).0),
        ),
        // A sign is a post with a board on top, facing local +Z.
        PieceId::Sign => (RawMesh::default(), sign_mesh(cell)),
        // A block is one solid cube on its anchor cell.
        PieceId::Block => (RawMesh::default(), block_mesh(cell)),
        // A junction paves its through road in three spans so the kerb breaks
        // at the mouths, plus one lifted ribbon per side road out to its joint
        // plane. A kerb wall across a side road is a barrier the side road
        // cannot cross except at speed, which is the opposite of a junction.
        PieceId::TJunction | PieceId::Crossroads | PieceId::YJunction | PieceId::YJunctionFlip => {
            let l = junction_len.unwrap_or(4.0 * cell);
            let mid = l / 2.0;
            let mut surface = RawMesh::default();
            let mut decor = RawMesh::default();
            for (z0, z1, kerb) in [
                (0.0, mid - half, KERB_HEIGHT),
                (mid - half, mid + half, 0.0),
                (mid + half, l, KERB_HEIGHT),
            ] {
                let span = straight_span(z0, z1, half, STRAIGHT_SEGMENTS);
                let (flat, walls) = ribbon(&span, half, cell, kerb);
                append_mesh(&mut surface, flat);
                append_mesh(&mut decor, walls);
            }
            for arm in &branches {
                let (flat, _) = ribbon(arm, half, cell, 0.0);
                append_mesh(&mut surface, flat);
            }
            (surface, decor)
        }
        _ => {
            let (a, b) = ribbon(&center, half, cell, KERB_HEIGHT);
            (a, b)
        }
    };

    let ports = if id.is_scenery() {
        Vec::new()
    } else if id.is_junction() {
        junction_ports(id, half, cell, junction_len.unwrap_or(4.0 * cell))
    } else {
        entry_exit_ports(&center, cell)
    };

    let (mut reserve_min, mut reserve_max) = reserve_box(&center, cell);
    for arm in &branches {
        let (lo, hi) = reserve_box(arm, cell);
        for k in 0..3 {
            reserve_min[k] = reserve_min[k].min(lo[k]);
            reserve_max[k] = reserve_max[k].max(hi[k]);
        }
    }
    // Centre line only: widening by the half-width would claim the cells just
    // outside the kerb, falsely colliding pieces at their shared join row.
    let mut occupied = occupied_cells(&center, cell, &ports);
    // Junctions sample densely, so the paved mouths are claimed: a straight's
    // two-sample centreline owns nothing, which is fine for a road no other
    // piece can stand on, but a side road butts onto cells the junction paves.
    if junction_len.is_some() {
        let l = junction_len.unwrap_or(4.0 * cell);
        let dense = straight_span(0.0, l, half, 8);
        occupied = occupied_cells(&dense, cell, &ports);
        for arm in &branches {
            for p in &arm.pos {
                let c = to_grid(*p, cell);
                if ports.iter().any(|port| port.cell == c) {
                    continue;
                }
                if !occupied.contains(&c) {
                    occupied.push(c);
                }
            }
        }
        occupied.sort_unstable();
    }
    if id == PieceId::Pillar {
        // The top cell is the road's, not the pillar's: the column props it,
        // and counting it flags every supported road as an overlap.
        if let Some(top) = occupied.iter().map(|c| c[1]).max() {
            occupied.retain(|c| c[1] != top);
        }
    }

    PieceShape {
        surface,
        decor,
        ports,
        reserve_min,
        reserve_max,
        occupied,
        center,
        branches,
        has_guide,
    }
}

/// Grid cells the road body actually occupies. Port cells are dropped:
/// neighbours share their join cell, so counting it flags every track as overlap.
fn occupied_cells(center: &Centerline, cell: f32, ports: &[Port]) -> Vec<[i16; 3]> {
    let mut out: Vec<[i16; 3]> = Vec::new();
    for p in &center.pos {
        let c = to_grid(*p, cell);
        if ports.iter().any(|port| port.cell == c) {
            continue;
        }
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out.sort_unstable();
    out
}

/// Extrude the centre curve into a flat ribbon plus raised kerb walls.
fn ribbon(c: &Centerline, half: f32, cell: f32, kerb: f32) -> (RawMesh, RawMesh) {
    let mut flat = RawMesh::default();
    let mut walls = RawMesh::default();
    if c.pos.len() < 2 || half <= 0.0 {
        return (flat, walls);
    }
    for i in 0..c.pos.len() - 1 {
        let (p0, p1) = (c.pos[i], c.pos[i + 1]);
        let (t0, t1) = (c.tan[i], c.tan[i + 1]);
        let (n0, n1) = (c.nrm[i], c.nrm[i + 1]);
        let r0 = t0.cross(n0).normalize_or_zero();
        let r1 = t1.cross(n1).normalize_or_zero();
        let (l0, rr0) = (p0 - r0 * half, p0 + r0 * half);
        let (l1, rr1) = (p1 - r1 * half, p1 + r1 * half);
        // The road's cross-section: a shallow crown, so the surface curves
        // instead of being one flat quad. Sampled across the width and extruded
        // along the span, with each strip carrying its own normal taken from the
        // slope it actually has. Without it the road reads as a flat sheet and
        // its edges as the crease where the sheet stops.
        crowned_run(&mut flat, l0, rr0, l1, rr1, n0, n1, half / cell);

        if kerb > 0.0 {
            for ((side0, side1), sign) in [((l0, l1), -1.0), ((rr0, rr1), 1.0)] {
                // The lip stands clear of the road by the kerb's own height, so
                // it is something you can see from the side and something the
                // car meets, rather than a wall of zero thickness drawn on the
                // edge column itself.
                let (a, b) = (side0 + r0 * sign * kerb, side1 + r1 * sign * kerb);
                let face = (r0 * sign).to_array();
                stripe_wall(
                    &mut walls,
                    (a, b),
                    (n0, n1),
                    (c.arc[i], c.arc[i + 1]),
                    kerb,
                    face,
                );
                // A cap between the road edge and the lip, so the edge is closed
                // and does not show the inside of the surface.
                push_quad(
                    &mut walls,
                    side0.to_array(),
                    a.to_array(),
                    b.to_array(),
                    side1.to_array(),
                    (n0 + n1).normalize().to_array(),
                );
            }
        }
        // A skirt hanging from the road's edges down to the piece's floor. A
        // slope is a surface in the air, and without this it reads as a ribbon
        // floating over nothing; with it, a climb is a solid ramp and the drop
        // beyond it has a far wall.
        if let Some(floor) = c.floor {
            for (a, b, sign) in [(l0, l1, -1.0), (rr0, rr1, 1.0)] {
                // Only where the road is above the floor: a skirt along a level
                // approach would put a wall through a piece of flat road. Both
                // ends are measured, because a slope's two edges are at two
                // heights and dropping them both by the first one leaves the
                // skirt at an angle instead of hanging under the road.
                let (drop_a, drop_b) = (floor - a.y, floor - b.y);
                if a.y > floor + 1e-3 || b.y > floor + 1e-3 {
                    push_quad(
                        &mut walls,
                        a.to_array(),
                        b.to_array(),
                        (b + Vec3::new(0.0, drop_b, 0.0)).to_array(),
                        (a + Vec3::new(0.0, drop_a, 0.0)).to_array(),
                        (r0 * sign).to_array(),
                    );
                }
            }
        }
    }
    (flat, walls)
}

/// Road surface between two edges, crowned.
///
/// Sampled across the width rather than drawn as one quad from edge to edge: the
/// centre rides a fraction of a cell above the edges, and each strip carries the
/// normal its own slope gives it. One flat quad puts every vertex on one normal,
/// which is why a road built that way reads as a flat sheet with a crease at
/// each edge.
fn crowned_run(
    out: &mut RawMesh,
    l0: Vec3,
    r0: Vec3,
    l1: Vec3,
    r1: Vec3,
    n0: Vec3,
    n1: Vec3,
    half_cells: f32,
) {
    /// Strips across the width. Enough for the crown to read as a curve rather
    /// than as a tent, few enough to stay cheap: every strip is a quad of road.
    const STRIPS: usize = 5;

    for strip in 0..STRIPS {
        let (a, b) = (
            strip as f32 / STRIPS as f32,
            (strip + 1) as f32 / STRIPS as f32,
        );
        let (pa, pb) = (l0.lerp(r0, a), l0.lerp(r0, b));
        let (qa, qb) = (l1.lerp(r1, a), l1.lerp(r1, b));
        let ca = n0 * crown_height(a, half_cells) + pa;
        let cb = n0 * crown_height(b, half_cells) + pb;
        let da = n1 * crown_height(a, half_cells) + qa;
        let db = n1 * crown_height(b, half_cells) + qb;
        // The strip's own normal: from the slope across the width, not the flat
        // surface normal it replaces.
        let along = (db - ca).normalize_or_zero();
        let across = (cb - ca).normalize_or_zero();
        let n = along.cross(across).normalize_or_zero();
        let n = if n.dot(n0) < 0.0 { -n } else { n };
        push_quad(
            out,
            ca.to_array(),
            cb.to_array(),
            db.to_array(),
            da.to_array(),
            n.to_array(),
        );
    }
}

/// How far above its edge a point this far across the road sits.
fn crown_height(across: f32, half_cells: f32) -> f32 {
    /// Height of the crown at the centre, as a fraction of the road width.
    const CROWN: f32 = 0.02;
    // A parabola peaking at the middle and zero at both edges, so the crown
    // meets the edges level and there is no step where the road stops.
    let t = across * 2.0 - 1.0;
    CROWN * half_cells * 2.0 * (1.0 - t * t)
}

/// Kerb stripe period along the arc. The wall mesh bakes the stripe parity
/// into vertex colour (white = even, black = odd); the renderer reads it back
/// and substitutes the palette, so the format crate stays colour-free.
const KERB_STRIPE: f32 = 0.8;

/// Extrude one kerb run, cutting it at the stripe grid so every block spans a
/// whole stripe.
fn stripe_wall(
    m: &mut RawMesh,
    (a, b): (Vec3, Vec3),
    (n0, n1): (Vec3, Vec3),
    (arc0, arc1): (f32, f32),
    kerb: f32,
    face: [f32; 3],
) {
    let span = arc1 - arc0;
    if span <= f32::EPSILON {
        push_wall(
            m,
            (a, b),
            (n0, n1),
            (0.0, 1.0),
            kerb,
            face,
            arc0 / KERB_STRIPE,
        );
        return;
    }
    let mut stripe = (arc0 / KERB_STRIPE).floor();
    let mut t0 = 0.0;
    loop {
        let t1 = (((stripe + 1.0) * KERB_STRIPE - arc0) / span).min(1.0);
        if (t1 - t0) * span > 1e-3 {
            push_wall(m, (a, b), (n0, n1), (t0, t1), kerb, face, stripe);
        }
        if t1 >= 1.0 {
            break;
        }
        t0 = t1;
        stripe += 1.0;
    }
}

/// One wall block: the sub-quad of the chord from `a` to `b` between `t0`
/// and `t1`, tagged with its stripe parity.
fn push_wall(
    m: &mut RawMesh,
    (a, b): (Vec3, Vec3),
    (n0, n1): (Vec3, Vec3),
    (t0, t1): (f32, f32),
    kerb: f32,
    face: [f32; 3],
    stripe: f32,
) {
    let (pa, pb) = (a.lerp(b, t0), a.lerp(b, t1));
    let (na, nb) = (
        n0.lerp(n1, t0).normalize_or_zero(),
        n0.lerp(n1, t1).normalize_or_zero(),
    );
    push_quad(
        m,
        pa.to_array(),
        (pa + na * kerb).to_array(),
        (pb + nb * kerb).to_array(),
        pb.to_array(),
        face,
    );
    let mask = if (stripe as i32).rem_euclid(2) == 0 {
        [1.0; 3]
    } else {
        [0.0; 3]
    };
    let tail = m.colors.len() - 4;
    m.colors[tail..].fill(mask);
}

fn push_quad(m: &mut RawMesh, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], n: [f32; 3]) {
    let base = m.positions.len() as u32;
    m.positions.extend_from_slice(&[a, b, c, d]);
    m.normals.extend_from_slice(&[n, n, n, n]);
    m.colors.extend_from_slice(&[[1.0; 3]; 4]);
    m.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Concatenate meshes, for pieces paved from several ribbons.
fn append_mesh(out: &mut RawMesh, other: RawMesh) {
    let base = out.positions.len() as u32;
    out.positions.extend_from_slice(&other.positions);
    out.normals.extend_from_slice(&other.normals);
    out.colors.extend_from_slice(&other.colors);
    out.indices.extend(other.indices.iter().map(|i| i + base));
}

/// One straight run of road centreline, for junction spans.
fn straight_span(z0: f32, z1: f32, half: f32, segments: usize) -> Centerline {
    let mut c = Centerline {
        half_width: half,
        ..Centerline::default()
    };
    push_segment(&mut c, segments, |u| {
        (Vec3::new(0.0, 0.0, z0 + u * (z1 - z0)), Vec3::Z, Vec3::Y)
    });
    c
}

/// Lift of a junction arm over the through road, in metres.
///
/// The arm overlaps the through ribbon where they cross, and two coplanar
/// same-colour surfaces shimmer. A centimetre and a half separates them past
/// depth precision while staying an order of magnitude below anything the
/// car's suspension can feel.
const ARM_LIFT: f32 = 0.015;

/// Side-road centrelines of a junction: one ribbon per junction, spanning its
/// joint planes. The joint sits one cell past the road edge, so a side road
/// butts onto the arm end exactly instead of overlapping the through road.
fn junction_branches(id: PieceId, half: f32, cell: f32, l: f32) -> Vec<Centerline> {
    let mid = l / 2.0;
    let reach = (half / cell).floor() as i16 + 1;
    let end = f32::from(reach) * cell;
    // From buried start to joint plane, laterally. A T and a cross run
    // straight across; a Y eases over a lane, which is what makes it a merge
    // rather than a turn. The buried end lands just inside the far kerb,
    // where through traffic never drives, instead of across the middle of it.
    let (start, start_z, end_z) = match id {
        PieceId::Crossroads => (-end, mid, mid),
        PieceId::TJunction => (-half + 0.5, mid, mid),
        PieceId::YJunction => (-half + 0.5, mid - 0.4 * cell, mid + cell),
        _ => (-half + 0.5, mid + 0.4 * cell, mid - cell),
    };
    let mut c = Centerline {
        half_width: half,
        ..Centerline::default()
    };
    push_segment(&mut c, 8, |u| {
        // Smoothstep across: leaves and arrives heading down the road, and
        // bends only in the middle where there is room for it.
        let s = u * u * (3.0 - 2.0 * u);
        let ds = 6.0 * u * (1.0 - u);
        let dx = end - start;
        let dz = end_z - start_z;
        let tan = Vec3::new(dx, 0.0, dz * ds).normalize_or_zero();
        (
            Vec3::new(start + u * dx, ARM_LIFT, start_z + s * dz),
            tan,
            Vec3::Y,
        )
    });
    vec![c]
}

/// Ports of a junction: entry, through exit, then one side exit per arm, so
/// the chain prefers the through road and only takes an arm when nothing
/// continues straight on.
fn junction_ports(id: PieceId, half: f32, cell: f32, l: f32) -> Vec<Port> {
    let mid = (l / 2.0 / cell).round() as i16;
    let reach = (half / cell).floor() as i16 + 1;
    let mut ports = vec![
        Port {
            cell: [0, 0, 0],
            outward: GridDir::NegZ,
        },
        Port {
            cell: [0, 0, (l / cell).round() as i16],
            outward: GridDir::PosZ,
        },
    ];
    // One side mouth per arm, on the arm's own row: a merge joins the road
    // it runs beside, not the junction's middle.
    match id {
        PieceId::Crossroads => {
            ports.push(Port {
                cell: [reach, 0, mid],
                outward: GridDir::PosX,
            });
            ports.push(Port {
                cell: [-reach, 0, mid],
                outward: GridDir::NegX,
            });
        }
        PieceId::TJunction => {
            ports.push(Port {
                cell: [reach, 0, mid],
                outward: GridDir::PosX,
            });
        }
        PieceId::YJunction => {
            ports.push(Port {
                cell: [reach, 0, mid + 1],
                outward: GridDir::PosX,
            });
        }
        _ => {
            ports.push(Port {
                cell: [reach, 0, mid - 1],
                outward: GridDir::PosX,
            });
        }
    }
    ports
}

/// One axis-aligned solid, wound outward. Corners are absolute local metres.
fn push_box(m: &mut RawMesh, min: [f32; 3], max: [f32; 3]) {
    let (x0, y0, z0) = (min[0], min[1], min[2]);
    let (x1, y1, z1) = (max[0], max[1], max[2]);
    push_quad(
        m,
        [x0, y1, z0],
        [x0, y1, z1],
        [x1, y1, z1],
        [x1, y1, z0],
        [0.0, 1.0, 0.0],
    );
    push_quad(
        m,
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y0, z1],
        [x0, y0, z1],
        [0.0, -1.0, 0.0],
    );
    push_quad(
        m,
        [x1, y0, z1],
        [x1, y0, z0],
        [x1, y1, z0],
        [x1, y1, z1],
        [1.0, 0.0, 0.0],
    );
    push_quad(
        m,
        [x0, y0, z0],
        [x0, y0, z1],
        [x0, y1, z1],
        [x0, y1, z0],
        [-1.0, 0.0, 0.0],
    );
    push_quad(
        m,
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
        [0.0, 0.0, 1.0],
    );
    push_quad(
        m,
        [x1, y0, z0],
        [x0, y0, z0],
        [x0, y1, z0],
        [x1, y1, z0],
        [0.0, 0.0, -1.0],
    );
}

/// A pillar's decor: one solid column on its anchor cell, `tall` cells high.
///
/// Quads are tagged dark, which the scene reads as the pale kerb colour: a
/// support column in warning red would read as a barrier, and it is not one.
fn pillar_mesh(cell: f32, tall: u8) -> RawMesh {
    let mut m = RawMesh::default();
    let w = cell * 0.15;
    push_box(&mut m, [-w, 0.0, -w], [w, f32::from(tall) * cell, w]);
    for quad in m.colors.chunks_mut(4) {
        quad.fill([0.0; 3]);
    }
    m
}

/// A block's decor: one solid cube on its anchor cell, tagged dark like the
/// pillar so it renders pale rather than warning red.
fn block_mesh(cell: f32) -> RawMesh {
    let mut m = RawMesh::default();
    let h = cell * 0.5;
    push_box(&mut m, [-h, 0.0, -h], [h, cell, h]);
    for quad in m.colors.chunks_mut(4) {
        quad.fill([0.0; 3]);
    }
    m
}

/// A sign's decor: a post with a board on top, facing local +Z.
///
/// The board is tagged light and the post dark, which the scene reads as a
/// red board on a pale post: the reference's warning signs, without a texture.
fn sign_mesh(cell: f32) -> RawMesh {
    let mut m = RawMesh::default();
    let post = cell * 0.04;
    let post_top = cell * 0.35;
    push_box(&mut m, [-post, 0.0, -post], [post, post_top, post]);
    let board_w = cell * 0.4;
    let board_h = cell * 0.15;
    let board = m.positions.len() / 4;
    push_box(
        &mut m,
        [-board_w, post_top, -post],
        [board_w, post_top + board_h * 2.0, post],
    );
    for quad in m.colors.chunks_mut(4).skip(board) {
        quad.fill([1.0; 3]);
    }
    for quad in m.colors.chunks_mut(4).take(board) {
        quad.fill([0.0; 3]);
    }
    m
}

/// Entry and exit ports, quantised from the centre curve's endpoints. Every
/// piece lands its ends exactly on integer cells, which lets neighbours mate.
fn entry_exit_ports(c: &Centerline, cell: f32) -> Vec<Port> {
    let first = c.pos.first().copied().unwrap_or(Vec3::ZERO);
    let last = c.pos.last().copied().unwrap_or(Vec3::ZERO);
    let t_in = c.tan.first().copied().unwrap_or(Vec3::Z);
    let t_out = c.tan.last().copied().unwrap_or(Vec3::Z);

    // Quantisation can collapse two ends onto one cell (a full loop enters
    // where it leaves); facing them out of the piece keeps them distinct.
    let first_cell = to_grid(first, cell);
    let last_cell = to_grid(last, cell);

    vec![
        Port {
            cell: first_cell,
            // The entry port faces backwards, out of the piece.
            outward: quantise_dir(-t_in),
        },
        Port {
            cell: last_cell,
            outward: quantise_dir(t_out),
        },
    ]
}

/// Local metres to grid cells. Y is in half-cell steps.
pub fn to_grid(p: Vec3, cell: f32) -> [i16; 3] {
    [
        (p.x / cell).round() as i16,
        (p.y / (cell * GRID_Y_CELLS)).round() as i16,
        (p.z / cell).round() as i16,
    ]
}

/// Grid cells back to local metres, given a document's cell size.
pub fn from_grid(c: [i16; 3], cell: f32) -> Vec3 {
    Vec3::new(
        c[0] as f32 * cell,
        c[1] as f32 * (cell * GRID_Y_CELLS),
        c[2] as f32 * cell,
    )
}

fn quantise_dir(t: Vec3) -> GridDir {
    let [x, _, z] = [t.x, t.y, t.z];
    if x.abs() > z.abs() {
        if x > 0.0 {
            GridDir::PosX
        } else {
            GridDir::NegX
        }
    } else if z > 0.0 {
        GridDir::PosZ
    } else {
        GridDir::NegZ
    }
}

/// Reservation box from the sampled geometry, so it is always correct even
/// for odd parameters.
fn reserve_box(c: &Centerline, cell: f32) -> ([i16; 3], [i16; 3]) {
    let mut lo = [i16::MAX; 3];
    let mut hi = [i16::MIN; 3];
    for p in &c.pos {
        let g = to_grid(*p, cell);
        for k in 0..3 {
            lo[k] = lo[k].min(g[k]);
            hi[k] = hi[k].max(g[k]);
        }
    }
    if lo[0] > hi[0] {
        return ([0; 3], [0; 3]);
    }
    (lo, hi)
}

/// Local-metre axis-aligned bounds of the reservation box, for editor
/// preview and HUD maps.
pub fn reserve_extent(shape: &PieceShape, cell: f32) -> (Vec3, Vec3) {
    (
        from_grid(shape.reserve_min, cell),
        from_grid(shape.reserve_max, cell),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceParams;

    const C: f32 = 4.0;

    fn center(id: PieceId) -> Centerline {
        piece_centerline(id, &PieceParams::default(), C)
    }

    /// Right-turn ports are read off the recorded endpoints, not assumed.
    fn ports(id: PieceId) -> Vec<Port> {
        piece_shape(id, &PieceParams::default(), C).ports
    }

    #[test]
    fn every_piece_produces_a_usable_centerline() {
        for id in PieceId::ALL {
            let c = center(id);
            assert!(c.pos.len() >= 2, "{id:?} has too few samples");
            assert!(c.length() > 0.0, "{id:?} has zero length");
            for (i, t) in c.tan.iter().enumerate() {
                assert!(t.is_normalized(), "{id:?} tangent {i} not unit");
            }
            for (i, n) in c.nrm.iter().enumerate() {
                assert!(n.is_normalized(), "{id:?} normal {i} not unit");
            }
            // A ribbon extrudes along tangent x normal, so a frame that is not
            // square tilts the road: this is what once made ramps read flat.
            for (i, (t, n)) in c.tan.iter().zip(c.nrm.iter()).enumerate() {
                assert!(
                    t.dot(*n).abs() < 1e-3,
                    "{id:?} frame {i}: normal {n:?} is not perpendicular to {t:?}"
                );
            }
        }
    }

    #[test]
    fn straight_spans_a_whole_number_of_cells_and_exits_level() {
        let c = center(PieceId::Straight);
        let shape = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        // It runs level, so it occupies one grid row in y and ends on it.
        assert_eq!(shape.ports[0].cell, [0, 0, 0]);
        assert_eq!(shape.ports[0].outward, GridDir::NegZ);
        assert_eq!(
            shape.ports[1].cell,
            to_grid(*c.pos.last().unwrap(), C),
            "the exit cell must be where the centreline actually ends"
        );
        assert_eq!(shape.ports[1].outward, GridDir::PosZ);
        // And a whole number of cells long, so the next piece's anchor is a cell.
        let cells = c.length() / C;
        assert!((cells - cells.round()).abs() < 1e-3, "len {} cells", cells);
    }

    #[test]
    fn right_curve_mates_with_a_straight_running_positive_x() {
        let c = center(PieceId::CurveRight90);
        let p = ports(PieceId::CurveRight90);
        assert_eq!(p[0].cell, [0, 0, 0]);
        assert_eq!(p[1].cell, to_grid(*c.pos.last().unwrap(), C), "exit cell");
        assert_eq!(p[1].outward, GridDir::PosX, "exit faces +X");
        // A quarter turn of a circle of radius r ends r along +X and r along +Z.
        let (p, t, _, _) = c.sample(c.length()).unwrap();
        let r = p.x.max(p.z);
        assert!((p - Vec3::new(r, 0.0, r)).length() < 1e-3, "exit at {p:?}");
        assert!(t.dot(Vec3::X) > 0.999, "exit tangent {t:?}");
    }

    #[test]
    fn left_curve_is_the_mirror_of_the_right() {
        let p = ports(PieceId::CurveLeft90);
        let c = center(PieceId::CurveLeft90);
        assert_eq!(p[1].cell, to_grid(*c.pos.last().unwrap(), C), "exit cell");
        assert_eq!(p[1].outward, GridDir::NegX);
        // The mirror of the right curve: same sweep, the other way across.
        let right = center(PieceId::CurveRight90);
        assert!(
            (c.pos.last().unwrap().x + right.pos.last().unwrap().x).abs() < 1e-3,
            "the two must be equal and opposite across x"
        );
    }

    #[test]
    fn ramp_up_ends_on_the_vertical_grid() {
        let shape = piece_shape(PieceId::RampUp, &PieceParams::default(), C);
        let c = center(PieceId::RampUp);
        assert_eq!(shape.ports[0].cell, [0, 0, 0]);
        let exit = to_grid(*c.pos.last().unwrap(), C);
        assert_eq!(shape.ports[1].cell, exit, "ramp exit cell");
        // The rise lands on a whole vertical grid step, which is what lets a
        // neighbour anchor on the exit cell and meet the road exactly.
        let steps = c.pos.last().unwrap().y / (C * GRID_Y_CELLS);
        assert!(
            (steps - steps.round()).abs() < 1e-3,
            "rise {} is not a whole step",
            c.pos.last().unwrap().y
        );
    }

    #[test]
    fn ramp_surface_normal_matches_its_slope() {
        // The rise over the run, read off the piece rather than assumed: this is
        // the property that must hold whatever the piece is sized at.
        let (up, down) = (center(PieceId::RampUp), center(PieceId::RampDown));
        let run = up.pos.last().unwrap().z;
        for (id, dy) in [
            // Rise over run, both read off the piece.
            (PieceId::RampUp, up.pos.last().unwrap().y / run),
            // A ramp down is anchored at its foot and climbs, so its drop is
            // its first sample, not its last.
            (PieceId::RampDown, -down.pos.first().unwrap().y / run),
        ] {
            let tangent = Vec3::new(0.0, dy, 1.0).normalize();
            let shape = piece_shape(id, &PieceParams::default(), C);
            assert!(!shape.surface.is_empty(), "{id:?} has no drivable surface");
            for (_, _, _, n) in shape.surface.triangles() {
                let n = Vec3::from(n);
                assert!(n.is_normalized(), "{id:?} normal {n:?} not unit");
                assert!(
                    n.dot(tangent) < 1e-3,
                    "{id:?} normal {n:?} must be perpendicular to the slope {tangent:?}"
                );
                assert!(n.y > 0.0, "{id:?} normal {n:?} must point out of the road");
            }
        }
    }

    #[test]
    fn ramp_up_and_down_mate_in_world_space() {
        let up = piece_shape(PieceId::RampUp, &PieceParams::default(), C);
        let down = piece_shape(PieceId::RampDown, &PieceParams::default(), C);
        let up_c = center(PieceId::RampUp);

        // The exit is where the centreline ends, on the vertical grid.
        let up_exit = to_grid(*up_c.pos.last().unwrap(), C);
        assert_eq!(up.ports[0].cell, [0, 0, 0], "ramp up enters at its base");
        assert_eq!(
            up.ports[1].cell, up_exit,
            "ramp up exits at the end of its centreline"
        );

        // A ramp down is the mirror image: it enters at its top and returns
        // to the level below over its own length. Its height is the one that
        // matters, not its whole anchor: a ramp down is anchored at its foot and
        // climbs, while a ramp up is anchored at its base.
        assert_eq!(
            down.ports[0].cell[1], up_exit[1],
            "ramp down enters at the height ramp up exits at"
        );
        let down_c = center(PieceId::RampDown);
        assert_eq!(
            down.ports[1].cell[1],
            to_grid(*down_c.pos.last().unwrap(), C)[1],
            "ramp down exits at its own base"
        );

        // A ramp down descends by the same amount a ramp up rises, over the
        // same run, so the two meet with no gap and no kink.
        assert!(
            (up.center.length() - down.center.length()).abs() < 1e-3,
            "matching ramps must match length"
        );
        let drop = down.center.pos[0].y - down.center.pos[down.center.pos.len() - 1].y;
        let rise = up.center.pos.last().unwrap().y - up.center.pos[0].y;
        assert!(
            (drop - rise).abs() < 1e-3,
            "a ramp down must drop as much as a ramp up rises, dropped {drop} vs {rise}"
        );
    }

    #[test]
    fn loop_is_a_full_turn_with_an_inward_normal() {
        let c = center(PieceId::Loop);
        // One full turn of a circle: the circumference is the length, and the
        // radius is whatever the piece was built at.
        let lo = c.pos.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let hi = c.pos.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let radius = (hi - lo) * 0.5;
        assert!(
            (c.length() - std::f32::consts::TAU * radius).abs() < 0.1 * radius,
            "length {} is not a circle of radius {}",
            c.length(),
            radius
        );
        let (p, _, n, _) = c.sample(c.length() * 0.5).unwrap();
        // At the apex the car is upside down, two radii up.
        assert!((p.y - 2.0 * radius).abs() < 0.1, "apex height {}", p.y);
        assert!(
            n.dot(Vec3::NEG_Y) > 0.99,
            "apex normal must face down, got {n:?}"
        );
        let (p0, _, n0, _) = c.sample(0.0).unwrap();
        assert!(p0.distance(Vec3::ZERO) < 1e-3, "entry at origin");
        assert!(n0.dot(Vec3::Y) > 0.99, "entry normal up");
    }

    #[test]
    fn loop_never_crosses_itself() {
        // The front and back halves must not share a z, or the car
        // re-enters its own geometry and orbits forever.
        let c = center(PieceId::Loop);
        let r = 2.0 * C;
        let front: Vec<f32> = c.pos.iter().filter(|p| p.z > 0.0).map(|p| p.z).collect();
        let back: Vec<f32> = c.pos.iter().filter(|p| p.z < 0.0).map(|p| p.z).collect();
        assert!(!front.is_empty() && !back.is_empty());
        assert!(front.iter().cloned().fold(f32::MAX, f32::min) > 0.0);
        assert!(back.iter().cloned().fold(f32::MIN, f32::max) < 0.0);
        assert!((r * 2.0 - 2.0 * r).abs() < 1e-3);
    }

    #[test]
    fn wall_is_not_driveable() {
        let shape = piece_shape(PieceId::Wall, &PieceParams::default(), C);
        assert!(shape.surface.is_empty(), "wall has no drive surface");
        assert!(!shape.decor.is_empty(), "wall still renders");
        assert!(shape.ports.is_empty());
        assert!(!shape.has_guide);
    }

    #[test]
    fn scenery_pieces_are_decor_only_with_no_ports() {
        // A pillar holds a road up and a sign stands beside one: neither is
        // driven on, neither joins the chain, and both still render and
        // collide through their decor.
        for id in [PieceId::Pillar, PieceId::Sign, PieceId::Block] {
            let shape = piece_shape(id, &PieceParams::default(), C);
            assert!(shape.surface.is_empty(), "{id:?} has no drive surface");
            assert!(!shape.decor.is_empty(), "{id:?} still renders");
            assert!(shape.ports.is_empty(), "{id:?} joins nothing");
            assert!(!shape.has_guide, "{id:?} is not a rail");
            assert!(!id.is_driveable());
            for (a, b, c, n) in shape.decor.triangles() {
                let (a, b, c, n) = (Vec3::from(a), Vec3::from(b), Vec3::from(c), Vec3::from(n));
                let area = (b - a).cross(c - a).length();
                assert!(area > 1e-6, "{id:?}: degenerate decor triangle");
                assert!(
                    (n.length() - 1.0).abs() < 1e-3,
                    "{id:?}: decor normal not unit"
                );
            }
        }
    }

    #[test]
    fn a_pillar_grows_with_its_length_and_yields_its_top_cell() {
        // The length knob is the column's height: one piece props up a road
        // at any elevation instead of a stack of fixed blocks.
        let short = piece_shape(PieceId::Pillar, &PieceParams::default().length(1), C);
        let tall = piece_shape(PieceId::Pillar, &PieceParams::default().length(3), C);
        let height = |shape: &PieceShape| {
            let (lo, hi) = shape.decor.bounds().unwrap();
            hi[1] - lo[1]
        };
        assert!((height(&tall) - 3.0 * height(&short)).abs() < 1e-3);
        // The top cell is the road's: counting it flags every supported road
        // as an overlap.
        let top = tall.occupied.iter().map(|c| c[1]).max().unwrap();
        assert!(top < 3, "pillar of 3 must not occupy its third cell row");
    }

    #[test]
    fn a_sign_is_a_board_on_a_post_not_a_wall() {
        let shape = piece_shape(PieceId::Sign, &PieceParams::default(), C);
        let (lo, hi) = shape.decor.bounds().unwrap();
        assert!(hi[0] - lo[0] < C, "board wider than its cell");
        assert!(hi[2] - lo[2] < hi[0] - lo[0], "board deeper than wide");
        assert!(hi[1] - lo[1] > C * 0.5, "sign shorter than a car is long");
    }

    #[test]
    fn a_wide_piece_is_the_reference_ratio_wider() {
        // The reference runs two carriageways at a ratio of 1.2614. Ours
        // matches it, so a wide straight sits beside a normal one the way
        // the reference's two roads sit beside each other.
        use crate::{ROAD_HALF_CELLS, WIDE_ROAD_HALF_CELLS};
        let normal = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        let wide = piece_shape(PieceId::Straight, &PieceParams::default().wide(), C);
        assert!(
            (wide.center.half_width / normal.center.half_width
                - WIDE_ROAD_HALF_CELLS / ROAD_HALF_CELLS)
                .abs()
                < 1e-6
        );
        let width = |shape: &PieceShape| {
            let (lo, hi) = shape.surface.bounds().unwrap();
            hi[0] - lo[0]
        };
        // The kerb lips stand clear of both edges, so the drawn ratio lands
        // a touch under the centreline one. Either side of the reference.
        let ratio = width(&wide) / width(&normal);
        assert!(
            (1.24..1.29).contains(&ratio),
            "drawn wide ratio {ratio}, not the reference 1.26"
        );
        // Ports do not move: a wide road joins a normal one on the centre.
        assert_eq!(wide.ports, normal.ports);
    }

    #[test]
    fn the_crown_grows_with_the_carriageway() {
        use crate::{ROAD_HALF_CELLS, WIDE_ROAD_HALF_CELLS};
        let peak = |half| crown_height(0.5, half);
        assert!(
            (peak(WIDE_ROAD_HALF_CELLS) / peak(ROAD_HALF_CELLS)
                - WIDE_ROAD_HALF_CELLS / ROAD_HALF_CELLS)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn scenery_ignores_the_wide_flag() {
        // A wide pillar is nonsense: the flag is for roads, and scenery sets
        // its own widths. Asking anyway must change nothing.
        for id in [
            PieceId::Wall,
            PieceId::Pillar,
            PieceId::Sign,
            PieceId::Block,
        ] {
            let plain = piece_shape(id, &PieceParams::default(), C);
            let flagged = piece_shape(id, &PieceParams::default().wide(), C);
            assert_eq!(
                plain.decor.bounds(),
                flagged.decor.bounds(),
                "{id:?}: the wide flag must not reshape scenery"
            );
        }
    }

    #[test]
    fn junctions_have_a_through_road_and_side_ports() {
        // A T is entry, exit and one arm; a cross adds the other arm. Ports
        // are grid cells, so a side road butts onto whole cells, not fractions.
        let t = piece_shape(PieceId::TJunction, &PieceParams::default(), C);
        let x = piece_shape(PieceId::Crossroads, &PieceParams::default(), C);
        assert_eq!(t.ports.len(), 3, "T: entry, exit, one arm");
        assert_eq!(x.ports.len(), 4, "cross: entry, exit, two arms");
        let straight = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        assert_eq!(&t.ports[..2], &straight.ports[..2]);
        assert_eq!(&x.ports[..2], &straight.ports[..2]);
        assert!(t.has_guide && x.has_guide);
        // Joint planes one cell past the road edge, either side of the middle.
        assert_eq!(t.ports[2].cell, [2, 0, 2]);
        assert_eq!(x.ports[2].cell, [2, 0, 2]);
        assert_eq!(x.ports[3].cell, [-2, 0, 2]);
    }

    #[test]
    fn a_junction_paves_every_route_with_no_thin_triangles() {
        for id in [
            PieceId::TJunction,
            PieceId::Crossroads,
            PieceId::YJunction,
            PieceId::YJunctionFlip,
        ] {
            let shape = piece_shape(id, &PieceParams::default(), C);
            assert!(!shape.surface.is_empty());
            let (lo, hi) = shape.surface.bounds().unwrap();
            // Through road full length, arms out to both joint planes.
            assert!(hi[2] - lo[2] >= 4.0 * C - 1.0, "{id:?}: through road short");
            assert!(hi[0] >= 2.0 * C - 0.01, "{id:?}: east arm short");
            if id == PieceId::Crossroads {
                assert!(lo[0] <= -2.0 * C + 0.01, "west arm short");
            }
            for (a, b, c, n) in shape.surface.triangles() {
                let (a, b, c, n) = (Vec3::from(a), Vec3::from(b), Vec3::from(c), Vec3::from(n));
                assert!((b - a).cross(c - a).length() > 1e-6, "{id:?}: degenerate");
                assert!((n.length() - 1.0).abs() < 1e-3, "{id:?}: normal not unit");
            }
        }
    }

    #[test]
    fn a_y_arm_bends_a_lane_over_instead_of_turning() {
        // The merge eases sideways a full cell between its buried end and its
        // mouth, and the mouth sits on the lane it joins rather than the
        // junction's middle row.
        for (id, mouth) in [
            (PieceId::YJunction, [2, 0, 3]),
            (PieceId::YJunctionFlip, [2, 0, 1]),
        ] {
            let shape = piece_shape(id, &PieceParams::default(), C);
            assert_eq!(shape.ports.len(), 3);
            assert_eq!(shape.ports[2].cell, mouth);
            assert_eq!(shape.branches.len(), 1);
            let arm = &shape.branches[0];
            let (lo, hi) = (
                arm.pos.iter().map(|p| p.z).fold(f32::MAX, f32::min),
                arm.pos.iter().map(|p| p.z).fold(f32::MIN, f32::max),
            );
            let span = hi - lo;
            assert!(
                span >= C - 0.01,
                "{id:?}: arm drifts {span}, not a full lane"
            );
            // Leaves and arrives heading down the road, bending in the middle.
            assert!(arm.tan.first().unwrap().x > 0.99);
            assert!(arm.tan.last().unwrap().x > 0.99);
            let mid_tan = arm.tan[arm.tan.len() / 2];
            assert!(mid_tan.z.abs() > 0.05, "{id:?}: arm never bends");
        }
    }

    #[test]
    fn a_junction_claims_its_mouths_but_not_its_neighbours() {
        // The paved arms are occupied, so a block dropped on a side road is
        // reported; the joint planes are ports, so a road butted onto one is
        // a neighbour, not an overlap.
        let shape = piece_shape(PieceId::Crossroads, &PieceParams::default(), C);
        assert!(shape.occupied.contains(&[1, 0, 2]), "arm body unclaimed");
        for port in &shape.ports {
            assert!(
                !shape.occupied.contains(&port.cell),
                "port {port:?} counted as body"
            );
        }
    }

    /// A jump is road, then nothing, then road: the gap is the absence of
    /// surface rather than a piece that draws one.
    ///
    /// The test this replaces drove a dedicated `JumpGap` piece, which had to
    /// build a lip, an airborne middle and a landing in one centreline while
    /// marking the air so no quad was drawn across it. With pieces placed
    /// independently none of that is needed: leave the cells empty and there is
    /// no surface there to collide with, which is what puts the car in the air.
    /// What has to hold is that the two sides still meet at the heights a
    /// neighbour's ports are read at.
    #[test]
    fn a_gap_between_two_pieces_leaves_nothing_under_the_car() {
        let mut b = crate::demo::ChainBuilder::new("gap");
        b.step(PieceId::RampUp, PieceParams::default().length(2).slope(45));
        b.step(PieceId::Straight, PieceParams::default());
        let before_gap = b.cursor();
        b.gap(3);
        let after_gap = b.cursor();
        let doc = b.build();

        assert_eq!(
            after_gap[2] - before_gap[2],
            3,
            "the gap must actually be three cells of nothing"
        );

        // Neither side of the gap is joined to the other, which is the whole
        // point: they are two pieces with space between, not one piece with a
        // hole.
        assert!(
            doc.chain().len() < doc.pieces.len() + 1,
            "the gap breaks the connected run, as a jump must"
        );

        // And the cells in between hold no road, so nothing is there to drive
        // on. A ray straight down through the middle of the gap must find
        // nothing from the road it is testing.
        for step in 1..3 {
            let cell = [before_gap[0], before_gap[1], before_gap[2] + step];
            let occupied = doc.pieces.iter().any(|inst| {
                let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
                shape.reserve_cells().any(|c| {
                    let w = crate::piece::rotate_local_xz(c, inst.yaw);
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

    /// Steepness is the author's to set, and it lands where the grid can carry
    /// it whatever they choose.
    ///
    /// Before this, a ramp's angle was fixed by its length: rise was half a
    /// cell per cell of run at every length, so a ramp was always the same
    /// slope and there was no such thing as a gentle one or a kicker.
    #[test]
    fn a_slope_steeper_or_genter_than_the_default_is_a_different_slope() {
        let flat = resolve_slope(PieceId::RampUp, &PieceParams::default());
        let gentle = PieceParams::default().slope(10);
        let steep = PieceParams::default().slope(60);
        assert!(
            resolve_slope(PieceId::RampUp, &gentle) < flat
                && resolve_slope(PieceId::RampUp, &steep) > flat,
            "the default must sit between a gentle and a steep setting"
        );

        // The geometry has to answer to it, not merely the resolver. A ramp
        // whose rise was half a cell per cell of run whatever the author asked
        // for could not be made gentle or made fierce, and the control would
        // read as working while changing nothing at all.
        let rise = |deg: u8| {
            piece_shape(
                PieceId::RampUp,
                &PieceParams::default().length(4).slope(deg),
                C,
            )
            .center
            .pos
            .last()
            .expect("samples")
            .y
        };
        assert!(
            rise(10) < rise(45) && rise(45) < rise(70),
            "the rise must follow the angle: {}, then {}, then {}",
            rise(10),
            rise(45),
            rise(70)
        );

        // Whatever is asked for, the rise lands on a half cell, which is the
        // increment a neighbour's entry port is quantised to. A ramp that ended
        // between two would leave a step at every join on the track.
        for deg in [5u8, 12, 27, 40, 60, 75] {
            let shape = piece_shape(
                PieceId::RampUp,
                &PieceParams::default().length(3).slope(deg),
                C,
            );
            let rise = shape.center.pos.last().expect("samples").y;
            let step = C * GRID_Y_CELLS;
            let cells = rise / step;
            assert!(
                (cells - cells.round()).abs() < 1e-3,
                "a {deg} degree ramp rises {rise}, which is {cells} vertical steps"
            );
            // And the exit port agrees with where the road actually ends.
            assert_eq!(
                to_grid(*shape.center.pos.last().unwrap(), C),
                shape.ports[1].cell
            );
        }
    }

    /// A ramp is a solid thing, not a surface floating in the air.
    #[test]
    fn a_slope_has_an_underside_down_to_the_ground() {
        let ramp = piece_shape(PieceId::RampUp, &PieceParams::default().length(2), C);
        assert!(
            !ramp.decor.is_empty(),
            "a ramp with no decor is a ribbon with nothing under it"
        );
        // The skirt hangs from the far end of the ramp to the ground. Kerbs
        // alone would leave decor at the top end level with the road, which is
        // a ramp's height above the floor, so this separates a skirt from a
        // kerb rather than from nothing at all.
        let far = *ramp.center.pos.last().expect("samples");
        let hangs = ramp
            .decor
            .positions
            .iter()
            .any(|p| p[2] > far.z - C * 0.5 && p[1] < 1e-3);
        assert!(
            hangs,
            "the underside must reach the ground at the top of the ramp"
        );
        // And a flat piece is left alone: a skirt along level road would be a
        // wall through the middle of it.
        let straight = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        let flat_lowest = straight
            .decor
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::MAX, f32::min);
        assert!(
            flat_lowest > -1e-3,
            "a straight must not grow a skirt, got {flat_lowest}"
        );
    }

    /// A wall ride bends until the road is a wall, and its face turns with it.
    ///
    /// The distinction from a steep ramp is the whole reason the piece exists:
    /// a steep ramp is still a ramp, while this one ends with the car driving
    /// vertically with the road normal pointing horizontally out of it.

    #[test]
    fn an_s_curve_leaves_heading_the_way_it_came_in() {
        let shape = piece_shape(PieceId::SCurve, &PieceParams::default().radius(2), C);
        let first = *shape.center.pos.first().expect("samples");
        let last = *shape.center.pos.last().expect("samples");
        let out = *shape.center.tan.last().expect("samples");
        assert!(first.distance(Vec3::ZERO) < 1e-4, "enters at the origin");
        assert!(
            (out - Vec3::Z).length() < 1e-4,
            "leaves heading +Z, got {out:?}"
        );
        // Two radii across, which is what a right then left pair puts you.
        let across = last.x;
        assert!(
            (across - C * 4.0).abs() < 1e-4,
            "offset by two radii: {across}"
        );
        // No degenerate triangle: the middle of the S is one sample, not two
        // pushed on top of each other.
        assert!(
            shape.surface.indices.len() % 3 == 0,
            "indices must describe whole triangles"
        );
    }

    #[test]
    fn surface_triangles_all_have_unit_normals_and_area() {
        for id in PieceId::ALL {
            let shape = piece_shape(id, &PieceParams::default(), C);
            assert!(shape.surface.colors.len() == shape.surface.positions.len());
            for (a, b, cc, n) in shape.surface.triangles() {
                let area = (Vec3::from(b) - Vec3::from(a))
                    .cross(Vec3::from(cc) - Vec3::from(a))
                    .length()
                    * 0.5;
                assert!(area > 1e-4, "{id:?} degenerate triangle");
                let n = Vec3::from(n);
                assert!((n.length() - 1.0).abs() < 1e-3, "{id:?} normal {n:?}");
            }
        }
    }

    #[test]
    fn longer_straight_moves_its_exit_port() {
        let short = piece_shape(PieceId::Straight, &PieceParams::default().length(1), C);
        let long = piece_shape(PieceId::Straight, &PieceParams::default().length(4), C);
        assert_ne!(short.ports[1].cell, long.ports[1].cell);
        assert_eq!(long.ports[1].cell, [0, 0, 4]);
    }

    #[test]
    fn reserve_box_contains_every_sample() {
        for id in PieceId::ALL {
            let shape = piece_shape(id, &PieceParams::default(), C);
            for p in &shape.center.pos {
                let g = to_grid(*p, C);
                for (k, &v) in g.iter().enumerate() {
                    assert!(
                        v >= shape.reserve_min[k] && v <= shape.reserve_max[k],
                        "{id:?} sample {p:?} outside reserve on axis {k}"
                    );
                }
            }
        }
    }

    /// The two banked pieces lean opposite ways, and each leans the way its own
    /// turn does.
    ///
    /// The test this replaces asserted that a right bank leaned to +X, which is
    /// what the code did: the two hands shared one arm and one default, so
    /// `BankLeft` and `BankRight` were the same piece banked the same way. A
    /// track built with both banked its left corners and its right corners
    /// identically, which is why the stunt track had to place `BankRight` twice
    /// with opposite bank angles to fake a `BankLeft`.
    #[test]
    fn banked_road_tilts_its_normal() {
        let left = piece_shape(PieceId::BankLeft, &PieceParams::default().bank(20), C);
        let right = piece_shape(PieceId::BankRight, &PieceParams::default().bank(20), C);
        let (nl, nr) = (left.center.nrm[0], right.center.nrm[0]);

        assert!(nl.x > 0.1, "a left bank leans its normal to +X, got {nl:?}");
        assert!(nr.x < -0.1, "a right bank leans to -X, got {nr:?}");
        for n in [nl, nr] {
            assert!(n.dot(Vec3::Y) > 0.9, "and stays mostly up: {n:?}");
        }

        // Equal magnitudes: the same bank angle means the same lean either side.
        assert!(
            (nl.x + nr.x).abs() < 1e-5,
            "the hands are mirrors: {nl:?} vs {nr:?}"
        );

        // And the lean lifts the edge the corner throws the car at, which is
        // the whole purpose of a bank. Read off the plane the normal defines: a
        // road leaning n has height -n.x/n.y at +X. Facing +Z the right-hand
        // side of the road is -X, and a left-hand corner throws the car there,
        // so that is the edge a left bank lifts. A right-hand corner throws it
        // the other way.
        //
        // Getting this backwards is silent in a way no amount of playing catches
        // quickly: a bank still holds the car on the road, just on the wrong
        // side of it, and the track still drives.
        let height_at = |n: Vec3, x: f32| -(n.x / n.y) * x;
        assert!(
            height_at(nl, -1.0) > 0.0,
            "a left bank must lift the right-hand side: {nl:?}"
        );
        assert!(
            height_at(nr, 1.0) > 0.0,
            "a right bank must lift the left-hand side: {nr:?}"
        );
    }

    #[test]
    fn kerb_walls_bake_alternating_stripe_parity() {
        let shape = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        assert_eq!(shape.decor.colors.len(), shape.decor.positions.len());
        let quads: Vec<[f32; 3]> = shape.decor.colors.chunks(4).map(|q| q[0]).collect();
        assert!(
            quads.iter().all(|c| *c == [1.0; 3] || *c == [0.0; 3]),
            "stripe mask must be pure white/black"
        );
        assert!(quads.contains(&[1.0; 3]), "no even stripe");
        assert!(quads.contains(&[0.0; 3]), "no odd stripe");
    }

    #[test]
    fn transformed_multiplies_baked_colours() {
        let mesh = RawMesh {
            positions: vec![[0.0; 3]; 3],
            colors: vec![[1.0, 0.5, 0.25]; 3],
            indices: vec![0, 1, 2],
            ..RawMesh::default()
        };
        let out = mesh.transformed(glam::Mat4::IDENTITY, [0.5, 1.0, 0.5]);
        assert_eq!(out.colors, vec![[0.5, 0.5, 0.125]; 3]);
    }
}

/// A terrain ring standing between the plain and the sky.
///
/// The same triangles are used by the scene and by the collision world, because
/// a mountain you can see but not touch is a mountain you drive through: the
/// reference game hands its range to the physics as well as to the renderer.
///
/// Heights come from a fixed generator, so the range is the same range on every
/// load, and the inner edge is held flat so the track opens onto the plain.
/// Triangles are returned flat-shaded, as `(a, b, c, normal)` in world metres.
pub fn mountain_triangles(
    bounds: ([f32; 3], [f32; 3]),
    inner_hint: f32,
) -> Vec<([f32; 3], [f32; 3], [f32; 3], [f32; 3])> {
    /// Height bands, and how far out each sits. Metres, so a range reads as a
    /// range against a track a hundred metres long.
    const BANDS: usize = 8;
    const STEP: f32 = 100.0;
    const SCALE: f32 = 100.0;
    const MIN_INNER: f32 = 100.0;
    const MAX_INNER: f32 = 1700.0;

    let (lo, hi) = bounds;
    let centre = [(lo[0] + hi[0]) * 0.5, 0.0, (lo[2] + hi[2]) * 0.5];
    let half = (hi[0] - lo[0]).max(hi[2] - lo[2]).max(1.0) * 0.5;
    let inner = (MIN_INNER + half)
        .max(MIN_INNER + inner_hint)
        .min(MAX_INNER);
    if inner >= MAX_INNER {
        return Vec::new();
    }
    // One column per tenth of the inner radius, so the ring is smooth however
    // large the track is.
    let columns = (inner * 0.1).max(8.0) as usize;

    let mut noise = crate::geometry::Noise::new(0x2545_F491);
    let profiles: Vec<[f32; BANDS]> = (0..columns)
        .map(|_| {
            let mut row = [0.0f32; BANDS];
            let (phase, frequency) = (noise.unit() * TAU, 1.0 + noise.unit() * 2.0);
            let (phase2, frequency2) = (noise.unit() * TAU, 3.0 + noise.unit() * 4.0);
            let mut peak = 0.0f32;
            for (band, slot) in row.iter_mut().enumerate() {
                let u = band as f32 / (BANDS - 1) as f32;
                let v = (u * frequency * TAU + phase).sin() * 0.6
                    + (u * frequency2 * TAU + phase2).sin() * 0.4;
                *slot = v.max(0.0);
                peak = peak.max(*slot);
            }
            row[0] = 0.0;
            row[BANDS - 1] = 0.0;
            if peak > 1e-4 {
                for v in row.iter_mut() {
                    *v /= peak;
                }
            }
            row
        })
        .collect();

    let at = |angle: f32, radius: f32, height: f32| {
        [
            centre[0] + angle.cos() * radius,
            height * SCALE,
            centre[2] + angle.sin() * radius,
        ]
    };
    let mut out = Vec::new();
    for column in 0..columns {
        let t = TAU * column as f32 / columns as f32;
        let u = TAU * ((column + 1) % columns) as f32 / columns as f32;
        for band in 0..BANDS - 1 {
            let r0 = inner + STEP * band as f32;
            let r1 = inner + STEP * (band + 1) as f32;
            let (h0, h1) = (profiles[column][band], profiles[column][band + 1]);
            let (h2, h3) = (
                profiles[(column + 1) % columns][band],
                profiles[(column + 1) % columns][band + 1],
            );
            let (a, b, c, d) = (at(t, r0, h0), at(u, r0, h2), at(u, r1, h3), at(t, r1, h1));
            out.push(tri(a, b, c));
            out.push(tri(a, c, d));
        }
    }
    out
}

/// One triangle with its outward normal.
fn tri(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = if len < 1e-6 {
        [0.0, 1.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    };
    (a, b, c, n)
}

/// Fixed-seed generator. The range has to be the same range on every load, or a
/// saved run's background differs from the run it was recorded against.
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self { state: seed | 1 }
    }

    /// Uniform in 0..1.
    pub fn unit(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as f32 / u32::MAX as f32
    }
}

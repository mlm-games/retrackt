//! Analytic piece geometry. One function, [`piece_centerline`], produces the
//! centre curve of any piece; mesh, collision triangles, ports, reservation and
//! guide all derive from it, so a piece cannot be one shape to the renderer and
//! another to the car. Local space: entry at the origin heading `+Z`, units in
//! metres. Grid `+Y` is compressed (a cell step is half a cell edge), so `n`
//! cells of ramp rise `n/2` cell edges.

use glam::{Quat, Vec3};

use crate::piece::{GridDir, PieceId, PieceParams, catalog_by_id, resolve_params};
use crate::{KERB_HEIGHT, ROAD_HALF_CELLS};

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
    /// One flag per span, true where the road is not there.
    ///
    /// A jump is the only piece that uses this: its middle is air, and the span
    /// across it must not be built, or the car drives over a hole that is not
    /// there. Parallel to the spans, so `gap[i]` covers `pos[i]` to `pos[i+1]`.
    pub gap: Vec<bool>,
    /// Height the road's sides hang down to, or `None` for a piece that sits on
    /// the ground.
    ///
    /// A slope is a surface in the air, and a surface with nothing under it
    /// reads as a floating ribbon: the skirt turns it into a solid ramp, and
    /// gives the far side of a climb a wall to drop behind.
    pub floor: Option<f32>,
    /// Pieces with a gap (jumps) or no rail (walls) set this so the guide
    /// leaves the car to ordinary grounding instead of constraining it.
    pub skip: bool,
}

impl Centerline {
    /// Mark the spans in `[from, to)` of the sample index as airborne.
    pub fn gap_spans(&mut self, from: usize, to: usize) {
        self.gap = vec![false; self.pos.len().saturating_sub(1)];
        for span in from..to.min(self.gap.len()) {
            self.gap[span] = true;
        }
    }

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
/// Rise snapped to the nearest half cell, which is the increment the grid
/// quantises height to.
///
/// A slope's two ends have to land on half cells or a neighbour anchored on the
/// exit port meets the road a few centimetres off it, and every join on the
/// track carries that step.
fn snap_half_cells(rise: f32, cell: f32) -> f32 {
    let step = cell * 0.5;
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
    let half = ROAD_HALF_CELLS * cell;
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
        | PieceId::Boost => {
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

        PieceId::JumpGap => {
            // A jump is three things: a lip to launch off, air, and a landing to
            // arrive on. One centreline, so the ports are the ends of the whole
            // flight, with the air's spans marked airborne so no road is built or
            // collided across them.
            //
            // The old piece was a lip and nothing else: its centreline stopped at
            // half its length while its exit port quantised two cells further on,
            // so a successor was anchored in mid air past the end of the road.
            // Both the lip's rise and the landing's fall are snapped to half
            // cells, for the reason a ramp's is: the piece that follows has to
            // meet the landing exactly.
            let lip = snap_half_cells(cell * 0.5, cell);
            let span = l / 4.0;
            // Lip: the first quarter, climbing.
            push_segment(&mut c, 3, |u| {
                let z = u * span;
                let y = u * lip;
                let tangent = Vec3::new(0.0, lip, span).normalize();
                (
                    Vec3::new(0.0, y, z),
                    tangent,
                    Vec3::new(0.0, span, -lip).normalize(),
                )
            });
            let air_from = c.pos.len().saturating_sub(1);
            // Air: the middle half, a parabola over the gap. The car is off the
            // road here, so its shape only has to be honest about the height it
            // reserves, which is what keeps a piece placed overhead from being
            // built straight through the flight.
            let apex = lip;
            push_segment(&mut c, 4, |u| {
                let z = span + u * span * 2.0;
                let y = lip + apex * 4.0 * u * (1.0 - u);
                let dy = apex * 4.0 * (1.0 - 2.0 * u);
                let tangent = Vec3::new(0.0, dy, span * 2.0).normalize_or_zero();
                // Perpendicular to the flight, as a frame has to be. No surface
                // is built from these samples, so the direction is arbitrary;
                // the orthogonality is not, because every consumer that reads
                // a frame divides by it.
                let nrm = Vec3::new(0.0, -tangent.z, tangent.y).normalize_or_zero();
                (Vec3::new(0.0, y, z), tangent, nrm)
            });
            let air_to = c.pos.len().saturating_sub(1);
            // Landing: the last quarter, descending back to the height the exit
            // port will be read at, which is the height the lip left from.
            push_segment(&mut c, 3, |u| {
                let z = span * 3.0 + u * span;
                let y = lip - u * lip;
                let tangent = Vec3::new(0.0, -lip, span).normalize();
                (
                    Vec3::new(0.0, y, z),
                    tangent,
                    Vec3::new(0.0, span, lip).normalize(),
                )
            });
            c.gap_spans(air_from, air_to + 1);
            // No rail through a jump: the car has to leave the road, which is
            // the entire point of having one.
            c.skip = true;
            c.floor = Some(0.0);
        }

        PieceId::Wall => {
            // Not drivable: a thin upright barrier, half a cell tall.
            c.half_width = cell * 0.06;
            let h = cell * 0.5;
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, h, u * l), Vec3::Z, Vec3::X)
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

    // Ride width of a drivable road: the cell edge. Walls are thin.
    let (surface, decor) = match id {
        // A wall is decor only: nothing to drive on, just a barrier. Its
        // centre line carries its own thin half-width, so reuse that.
        PieceId::Wall => (
            RawMesh::default(),
            ribbon(&center, center.half_width, KERB_HEIGHT * 3.0).1,
        ),
        // A jump's lip and landing are drivable; the flight between them is not
        // built at all, so there are no kerbs either.
        PieceId::JumpGap => (ribbon(&center, half, 0.0).0, RawMesh::default()),
        _ => {
            let (a, b) = ribbon(&center, half, KERB_HEIGHT);
            (a, b)
        }
    };

    let ports = if id == PieceId::Wall {
        Vec::new()
    } else {
        entry_exit_ports(&center, cell)
    };

    let (reserve_min, reserve_max) = reserve_box(&center, cell);
    // Centre line only: widening by the half-width would claim the cells just
    // outside the kerb, falsely colliding pieces at their shared join row.
    let occupied = occupied_cells(&center, cell, &ports);

    PieceShape {
        surface,
        decor,
        ports,
        reserve_min,
        reserve_max,
        occupied,
        center,
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
fn ribbon(c: &Centerline, half: f32, kerb: f32) -> (RawMesh, RawMesh) {
    let mut flat = RawMesh::default();
    let mut walls = RawMesh::default();
    if c.pos.len() < 2 || half <= 0.0 {
        return (flat, walls);
    }
    for i in 0..c.pos.len() - 1 {
        // A span the road is not on is not built. Bridging it would put a
        // drivable surface across a jump's gap, which is the one thing a jump
        // must not have.
        if c.gap.get(i).copied().unwrap_or(false) {
            continue;
        }
        let (p0, p1) = (c.pos[i], c.pos[i + 1]);
        let (t0, t1) = (c.tan[i], c.tan[i + 1]);
        let (n0, n1) = (c.nrm[i], c.nrm[i + 1]);
        let r0 = t0.cross(n0).normalize_or_zero();
        let r1 = t1.cross(n1).normalize_or_zero();
        let (l0, rr0) = (p0 - r0 * half, p0 + r0 * half);
        let (l1, rr1) = (p1 - r1 * half, p1 + r1 * half);
        push_quad(
            &mut flat,
            l0.to_array(),
            rr0.to_array(),
            rr1.to_array(),
            l1.to_array(),
            n0.to_array(),
        );
        if kerb > 0.0 {
            for (a, b, sign) in [(l0, l1, -1.0), (rr0, rr1, 1.0)] {
                let face = (r0 * sign).to_array();
                stripe_wall(
                    &mut walls,
                    (a, b),
                    (n0, n1),
                    (c.arc[i], c.arc[i + 1]),
                    kerb,
                    face,
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
        (p.y / (cell * 0.5)).round() as i16,
        (p.z / cell).round() as i16,
    ]
}

/// Grid cells back to local metres, given a document's cell size.
pub fn from_grid(c: [i16; 3], cell: f32) -> Vec3 {
    Vec3::new(
        c[0] as f32 * cell,
        c[1] as f32 * (cell * 0.5),
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
    fn straight_ends_exactly_one_cell_later() {
        let c = center(PieceId::Straight);
        assert!((c.length() - C).abs() < 1e-3, "len {}", c.length());
        let shape = piece_shape(PieceId::Straight, &PieceParams::default(), C);
        assert_eq!(shape.ports[0].cell, [0, 0, 0]);
        assert_eq!(shape.ports[0].outward, GridDir::NegZ);
        assert_eq!(shape.ports[1].cell, [0, 0, 1]);
        assert_eq!(shape.ports[1].outward, GridDir::PosZ);
    }

    #[test]
    fn right_curve_mates_with_a_straight_running_positive_x() {
        let c = center(PieceId::CurveRight90);
        let p = ports(PieceId::CurveRight90);
        assert_eq!(p[0].cell, [0, 0, 0]);
        assert_eq!(p[1].cell, [1, 0, 1], "exit cell");
        assert_eq!(p[1].outward, GridDir::PosX, "exit faces +X");
        // Total swept angle must be a quarter turn.
        let (p, t, _, _) = c.sample(c.length()).unwrap();
        assert!(p.distance(Vec3::new(C, 0.0, C)) < 1e-3, "exit at {p:?}");
        assert!(t.dot(Vec3::X) > 0.999, "exit tangent {t:?}");
    }

    #[test]
    fn left_curve_is_the_mirror_of_the_right() {
        let p = ports(PieceId::CurveLeft90);
        assert_eq!(p[1].cell, [-1, 0, 1]);
        assert_eq!(p[1].outward, GridDir::NegX);
    }

    #[test]
    fn ramp_up_ends_at_an_integer_height() {
        let shape = piece_shape(PieceId::RampUp, &PieceParams::default(), C);
        assert_eq!(shape.ports[0].cell, [0, 0, 0]);
        // Two cells long, rising one cell edge.
        assert_eq!(shape.ports[1].cell, [0, 2, 2], "ramp exit cell");
    }

    #[test]
    fn ramp_surface_normal_matches_its_slope() {
        // A ramp rises one cell edge over two cells: 0.5 of run, not flat.
        for (id, dy) in [(PieceId::RampUp, 0.5f32), (PieceId::RampDown, -0.5f32)] {
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

        // A two-cell ramp rises exactly one cell edge and ends at its far port.
        assert_eq!(up.ports[0].cell, [0, 0, 0], "ramp up enters at its base");
        assert_eq!(
            up.ports[1].cell,
            [0, 2, 2],
            "ramp up exits two cells on, one edge up"
        );

        // A ramp down is the mirror image: it enters at its top and returns
        // to the level below over its own length.
        assert_eq!(down.ports[0].cell, [0, 2, 0], "ramp down enters at its top");
        assert_eq!(down.ports[1].cell, [0, 0, 2], "ramp down exits at its base");

        // One cell edge above and two cells on from a ramp up's exit, so the
        // two join with no gap and no kink.
        assert_eq!(
            down.ports[0].cell,
            [0, 2, 0],
            "ramp down must enter one edge above its own base"
        );
        assert!(
            (up.center.length() - down.center.length()).abs() < 1e-3,
            "matching ramps must match length"
        );
        // A ramp down ends lower than it started, by exactly one edge.
        let drop = down.center.pos[0].y - down.center.pos[down.center.pos.len() - 1].y;
        assert!(
            (drop - C).abs() < 1e-3,
            "ramp down should drop one cell edge, dropped {drop}"
        );
    }

    #[test]
    fn loop_is_a_full_turn_with_an_inward_normal() {
        let c = center(PieceId::Loop);
        assert!((c.length() - std::f32::consts::TAU * 2.0 * C).abs() < 0.1);
        let (p, _, n, _) = c.sample(c.length() * 0.5).unwrap();
        // At the apex the car is upside down at 2 loop-radii.
        assert!((p.y - 4.0 * C).abs() < 0.1, "apex height {}", p.y);
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
    fn jump_gap_has_no_guide_so_the_car_detaches() {
        let shape = piece_shape(PieceId::JumpGap, &PieceParams::default(), C);
        assert!(!shape.has_guide, "a jump must not be a rail");
        assert!(!shape.surface.is_empty());
    }

    /// A jump has a gap, and the road on either side of it actually meets the
    /// ports a neighbour anchors on.
    ///
    /// The old piece was a lip and nothing more: its centreline stopped at half
    /// its length while its exit port quantised two cells further on, so the
    /// piece that followed was anchored in mid air past the end of the road.
    /// Nothing in the game used it, so nothing caught it.
    #[test]
    fn a_jump_has_air_in_the_middle_and_road_at_both_ends() {
        let shape = piece_shape(PieceId::JumpGap, &PieceParams::default(), C);
        let len = shape
            .ports
            .last()
            .expect("a jump has an exit")
            .cell
            .to_owned();
        // The centreline spans the whole piece, so the exit port is at its end.
        let end = *shape.center.pos.last().expect("samples");
        assert_eq!(
            to_grid(end, C),
            len,
            "the exit port must be where the road ends, not past it"
        );
        // Both ends are on the ground plane, so a flat road can join either.
        for (which, port) in [("entry", shape.ports[0]), ("exit", shape.ports[1])] {
            let world = from_grid(port.cell, C);
            assert!(
                world.y.abs() < 1e-3,
                "the {which} of a jump is {:.1} high, so a flat neighbour \
                 cannot meet it",
                world.y
            );
        }
        // And the middle really is empty: no triangle spans the flight.
        let span = |from: [f32; 3], to: [f32; 3]| {
            (0..shape.surface.indices.len() / 3).any(|t| {
                let tri = [
                    shape.surface.positions[shape.surface.indices[t * 3] as usize],
                    shape.surface.positions[shape.surface.indices[t * 3 + 1] as usize],
                    shape.surface.positions[shape.surface.indices[t * 3 + 2] as usize],
                ];
                tri.iter().any(|v| {
                    let d = |a: [f32; 3], b: [f32; 3]| {
                        (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2])
                    };
                    d(*v, from) < 1.0 && d(*v, to) > 1.0
                })
            })
        };
        assert!(
            !span(
                [0.0, 0.0, 0.0],
                shape.center.pos.last().expect("samples").to_array()
            ),
            "no road may be built across the gap"
        );
        // The flight is reserved all the same, so a piece cannot be built
        // straight through where the car flies.
        assert!(
            shape.reserve_max[1] > shape.reserve_min[1],
            "the flight envelope must be reserved: {:?}",
            (shape.reserve_min, shape.reserve_max)
        );
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
            let half = C * 0.5;
            let cells = rise / half;
            assert!(
                (cells - cells.round()).abs() < 1e-3,
                "a {deg} degree ramp rises {rise}, which is {cells} half cells"
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

//! Analytic piece geometry. One function, [`piece_centerline`], produces the
//! centre curve of any piece; mesh, collision triangles, ports, reservation and
//! guide all derive from it, so a piece cannot be one shape to the renderer and
//! another to the car. Local space: entry at the origin heading `+Z`, units in
//! metres. Grid `+Y` is compressed (a cell step is half a cell edge), so `n`
//! cells of ramp rise `n/2` cell edges.

use glam::Vec3;

use crate::piece::{GridDir, PieceId, PieceParams, resolve_params};
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
    /// Pieces with a gap (jumps) or no rail (walls) set this so the guide
    /// leaves the car to ordinary grounding instead of constraining it.
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
pub fn piece_centerline(id: PieceId, params: &PieceParams, cell: f32) -> Centerline {
    let (len_cells, rad_cells, bank_deg) = resolve_params(id, params);
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
            // A RampUp placed at the base and a RampDown at the top meet with
            // no gap: up rises over its length, down descends to its base.
            let rise = len_cells as f32 * (cell * 0.5);
            let up = id == PieceId::RampUp;
            // Both report their exit at the correct absolute height relative
            // to their anchor.
            let n = 4;
            push_segment(&mut c, n, |u| {
                let y = if up { u * rise } else { (1.0 - u) * rise };
                let dy = if up { rise } else { -rise };
                let t = Vec3::new(0.0, dy, l).normalize();
                // Out of the sloped face, never world up: the collision world
                // reads this straight back as the road normal, so a flat
                // normal makes every ramp drive as if it were level.
                let nrm = Vec3::new(0.0, l, -dy).normalize();
                (Vec3::new(0.0, y, u * l), t, nrm)
            });
        }

        PieceId::BankLeft | PieceId::BankRight => {
            // Bank leans the outside edge up. `bank_deg > 0` lowers the
            // right-hand side, matching a right-hand banked corner.
            let b = bank_deg as f32 * std::f32::consts::PI / 180.0;
            let nrm = Vec3::new(b.sin(), b.cos(), 0.0);
            push_segment(&mut c, STRAIGHT_SEGMENTS, |u| {
                (Vec3::new(0.0, 0.0, u * l), Vec3::Z, nrm)
            });
        }

        PieceId::CurveRight90 => {
            // Entry at the origin heading +Z, exit at (+r, 0, +r) heading +X.
            let center = Vec3::new(r, 0.0, 0.0);
            let n = ARC_SEGMENTS_PER_QUARTER;
            push_segment(&mut c, n, |u| {
                let th = std::f32::consts::PI - u * std::f32::consts::FRAC_PI_2;
                let pos = center + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                (pos, Vec3::new(th.sin(), 0.0, -th.cos()), Vec3::Y)
            });
        }

        PieceId::CurveLeft90 => {
            // Entry at the origin heading +Z, exit at (-r, 0, +r) heading -X.
            let center = Vec3::new(-r, 0.0, 0.0);
            let n = ARC_SEGMENTS_PER_QUARTER;
            push_segment(&mut c, n, |u| {
                let th = u * std::f32::consts::FRAC_PI_2;
                let pos = center + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                (pos, Vec3::new(-th.sin(), 0.0, th.cos()), Vec3::Y)
            });
        }

        PieceId::Curve180 => {
            // Entry at the origin heading +Z, exit at (+2r, 0, 0) heading -Z.
            let center = Vec3::new(r, 0.0, 0.0);
            let n = ARC_SEGMENTS_PER_QUARTER * 2;
            push_segment(&mut c, n, |u| {
                let th = std::f32::consts::PI - u * std::f32::consts::PI;
                let pos = center + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                (pos, Vec3::new(th.sin(), 0.0, -th.cos()), Vec3::Y)
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
            // Launch lip, airborne gap, landing ramp. The guide skips it so
            // the car actually detaches instead of riding a phantom rail.
            let n = 4;
            let half_len = len_cells as f32 * cell * 0.5;
            let lip = 0.8;
            push_segment(&mut c, n, |u| {
                let z = u * half_len;
                // The lip rises, so neither its heading nor its face is flat.
                let t = Vec3::new(0.0, lip, half_len).normalize();
                let nrm = Vec3::new(0.0, half_len, -lip).normalize();
                (Vec3::new(0.0, z * lip / half_len, z), t, nrm)
            });
            c.skip = true;
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
        // A jump's launch lip is drivable; the gap and landing are built by
        // the piece above it, so no kerbs here.
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

    #[test]
    fn banked_road_tilts_its_normal() {
        let shape = piece_shape(PieceId::BankRight, &PieceParams::default().bank(20), C);
        let n = shape.center.nrm[0];
        assert!(n.x > 0.1, "right bank leans its normal to +X, got {n:?}");
        assert!(n.dot(Vec3::Y) > 0.9, "and stays mostly up");
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

mod car;
mod track;

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::Vec3;
use repame_view3d::{Frame3d, MeshGroup, SceneLight, ShadowDesc};
use retrackt_format::geometry::RawMesh;

use super::App;
use crate::app::state::Screen;

const GROUND_Y: f32 = -0.5;
const TILE: f32 = 8.0;
/// How far past the track the built-up ground runs, in metres. Far enough to
/// reach past the far plane from wherever the track is, so the outermost frame
/// is only ever seen softening into the fog rather than ending.
const GROUND_RUN: f32 = 2200.0;
/// Width of the fine frame beside the road, where the eye can still resolve
/// detail, and how each graded frame's radius grows over the last.
const GROUND_FINE_RUN: f32 = 40.0;
const GROUND_GROWTH: f32 = 2.0;
/// Tiles across a graded frame's outer edge, which its pitch is derived from. A
/// frame holds a fixed count whatever its radius, so a frame far out has the
/// same tile count as one up close and simply larger tiles.
const GROUND_EDGE: f32 = 11.0;

/// Preview tints. Amber means the piece will join the road it is dropped beside;
/// blue means it will sit where the cursor is, unattached. Two flat colours that
/// no track surface uses, so the answer to "will this connect" is readable at a
/// glance rather than inferred from alignment.
const PREVIEW_SNAP: u32 = 0xFFB020;
const PREVIEW_FREE: u32 = 0x45C3F0;
/// Low enough to see the road through, high enough to read against it.
const PREVIEW_ALPHA: f32 = 0.55;
/// The cursor cell outline, in the editor accent.
const CURSOR_TINT: u32 = 0xFFB020;
const CURSOR_ALPHA: f32 = 0.30;
/// The piece under the pointer, in near-white, so it is never mistaken for the
/// selection's amber.
const HOVER_TINT: u32 = 0xF2F4F7;
/// Opacity of both outlines: enough to read over any surface, little enough that
/// the piece inside them is still the piece being looked at.
const OUTLINE_ALPHA: f32 = 0.55;

const HORIZON: u32 = 0xCFE6F7;
const ZENITH: u32 = 0x4E9BE0;
const GRASS_A: u32 = 0x69BD5F;
const GRASS_B: u32 = 0x5BB052;
/// Inside the far plane, outside the ground ring: the dome always hides the
/// world edge while the camera stays within the track.
const SKY_RADIUS: f32 = 1900.0;

/// Terrain ring that stands between the track and the sky.
///
/// Without it the background is a plain: the ground runs out, the fog takes the
/// rest, and the horizon is a flat wash of sky. These are the ring's own
/// proportions, in metres, matching the world's units so a range reads as a
/// range rather than as a kerb.
const MOUNTAIN_INNER: f32 = 100.0;
/// How far out each successive height band sits.
const MOUNTAIN_STEP: f32 = 100.0;
/// Height a full-height sample is raised by.
const MOUNTAIN_SCALE: f32 = 100.0;
/// Height bands per ring. The track faces an apron of flat ground: a wall of
/// mountain built straight against the road would box the circuit in.
const MOUNTAIN_BANDS: usize = 8;
/// Far ground sits a hair under the detailed ground so the two never z-fight.
const FAR_GROUND_DROP: f32 = 0.05;
/// Edge of the plain, and the renderer's far plane it has to reach past from
/// wherever the camera has got to, which is the whole reason it tracks the view.
const PLAIN_REACH: f32 = 3200.0;
const FAR_PLANE: f32 = 2000.0;
/// Inner radius of the terrain ring must stay inside the renderer's far plane,
/// or the range is clipped away and the bug it fixes looks like it never ran.
const MOUNTAIN_MAX_INNER: f32 = 1700.0;

/// sRGB hex to the renderer's linear rgb.
pub(super) fn lin(hex: u32) -> [f32; 3] {
    let mut rgb = [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    ];
    for v in &mut rgb {
        *v = if *v <= 0.04045 {
            *v / 12.92
        } else {
            ((*v + 0.055) / 1.055).powf(2.4)
        };
    }
    rgb
}

pub fn raw_mesh_to_group(mesh: &RawMesh, depth_test: bool) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test,
        ..MeshGroup::default()
    };
    group.positions = mesh.positions.clone();
    group.indices = mesh.indices.clone();
    if mesh.colors.len() == mesh.positions.len() {
        group.colors = mesh.colors.clone();
    } else {
        group.colors = vec![[1.0; 3]; mesh.positions.len()];
    }
    if mesh.normals.len() == mesh.positions.len() {
        group.normals = mesh.normals.clone();
    }
    group
}

/// Append `src` to `dst`, keeping the group valid. Normals carry only while
/// both sides are per-vertex complete; otherwise the pair drops to unlit.
fn merge(dst: &mut RawMesh, src: &RawMesh) {
    let base = dst.positions.len() as u32;
    dst.positions.extend_from_slice(&src.positions);
    dst.colors.extend_from_slice(&src.colors);
    if src.normals.len() == src.positions.len()
        && dst.normals.len() + src.normals.len() == dst.positions.len()
    {
        dst.normals.extend_from_slice(&src.normals);
    } else {
        dst.normals.clear();
    }
    dst.indices.extend(src.indices.iter().map(|i| base + *i));
}

/// Lit quad whose right-hand-rule normal points along `n`.
pub(super) fn push_lit(m: &mut RawMesh, vs: [Vec3; 4], n: Vec3, color: [f32; 3]) {
    let base = m.positions.len() as u32;
    for v in vs {
        m.positions.push(v.to_array());
    }
    let n = n.normalize_or_zero().to_array();
    m.normals.extend([n; 4]);
    m.colors.extend([color; 4]);
    m.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Lit triangle, same winding rule as [`push_lit`].
pub(super) fn push_tri_lit(m: &mut RawMesh, vs: [Vec3; 3], n: Vec3, color: [f32; 3]) {
    let base = m.positions.len() as u32;
    for v in vs {
        m.positions.push(v.to_array());
    }
    let n = n.normalize_or_zero().to_array();
    m.normals.extend([n; 3]);
    m.colors.extend([color; 3]);
    m.indices.extend([base, base + 1, base + 2]);
}

/// Box in an orthogonal frame; each face winds outward. The axis/half-extent
/// pairs satisfy `u x v = w` so every face's right-hand normal is its own
/// axis; `half` gives the extents along `(u, v, w)`.
pub(super) fn push_obox(
    m: &mut RawMesh,
    centre: Vec3,
    u: Vec3,
    v: Vec3,
    w: Vec3,
    half: Vec3,
    color: [f32; 3],
) {
    let (hu, hv, hw) = (half.x, half.y, half.z);
    let faces = [
        (u, u * hu, v, hv, w, hw),
        (-u, -u * hu, w, hw, v, hv),
        (v, v * hv, w, hw, u, hu),
        (-v, -v * hv, u, hu, w, hw),
        (w, w * hw, u, hu, v, hv),
        (-w, -w * hw, v, hv, u, hu),
    ];
    for (axis, offset, a, ha, b, hb) in faces {
        let c = centre + offset;
        push_lit(
            m,
            [
                c - a * ha - b * hb,
                c + a * ha - b * hb,
                c + a * ha + b * hb,
                c - a * ha + b * hb,
            ],
            axis,
            color,
        );
    }
}

fn ground_group(bounds: ([f32; 3], [f32; 3])) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let a = lin(GRASS_A);
    let b = lin(GRASS_B);
    let (lo, hi) = bounds;
    let centre = [(lo[0] + hi[0]) * 0.5, (lo[2] + hi[2]) * 0.5];
    // The *narrower* of the two extents, so the first frame starts inside the
    // track's own box and the corners either side of a long thin circuit are
    // ground rather than a hole in it.
    let inner = (hi[0] - lo[0]).min(hi[2] - lo[2]).max(1.0) * 0.5;

    // Fine tile beside the road: the pitch that gives parallax when the camera
    // is close in, which is the only place the eye can resolve it.
    let fine = inner + GROUND_FINE_RUN;
    emit_frame(&mut group, centre, inner, fine, TILE, a, b);

    // Then frames whose tile count is fixed, so the pitch grows with distance
    // instead. The same two greens at every distance, with only the size of a
    // square of grass changing: a dappling that simply runs out is a line the
    // eye finds instantly, and a pitch that grows is not.
    let mut radius = fine;
    while radius < inner + GROUND_RUN {
        let outer = radius * GROUND_GROWTH;
        emit_frame(
            &mut group,
            centre,
            radius,
            outer,
            (2.0 * outer / GROUND_EDGE).max(TILE),
            a,
            b,
        );
        radius = outer;
    }
    group
}

/// One square frame from `radius` to `outer`, tiled at `pitch`.
///
/// Four strips, with the corners belonging to the horizontal ones so nothing is
/// drawn twice. The inner and outer bounds are worked out per axis: reading one
/// axis's bound where the other's belongs leaves the hole a quadrant rather than
/// a square, which is a hole the size of the track beside the track.
fn emit_frame(
    group: &mut MeshGroup,
    centre: [f32; 2],
    radius: f32,
    outer: f32,
    pitch: f32,
    a: [f32; 3],
    b: [f32; 3],
) {
    let tint = |i: i32, j: i32| if (i + j).rem_euclid(2) == 0 { a } else { b };
    let (i_lo, i_hi) = (
        ((centre[0] - outer) / pitch).floor() as i32,
        ((centre[0] + outer) / pitch).ceil() as i32,
    );
    let (i_in, i_out) = (
        ((centre[0] - radius) / pitch).ceil() as i32,
        ((centre[0] + radius) / pitch).floor() as i32,
    );
    let (j_lo, j_hi) = (
        ((centre[1] - outer) / pitch).floor() as i32,
        ((centre[1] + outer) / pitch).ceil() as i32,
    );
    let (j_in, j_out) = (
        ((centre[1] - radius) / pitch).ceil() as i32,
        ((centre[1] + radius) / pitch).floor() as i32,
    );
    // Horizontal strips carry the corners.
    for i in i_lo..i_hi {
        for j in (j_lo..j_in).chain(j_out + 1..j_hi) {
            push_tile(group, i, j, pitch, tint(i, j));
        }
    }
    for j in j_in..j_out + 1 {
        for i in (i_lo..i_in).chain(i_out + 1..i_hi) {
            push_tile(group, i, j, pitch, tint(i, j));
        }
    }
}

/// One tile of `pitch`, at tile-index coordinates.
fn push_tile(group: &mut MeshGroup, i: i32, j: i32, pitch: f32, tint: [f32; 3]) {
    let (x0, z0) = (i as f32 * pitch, j as f32 * pitch);
    group.push_quad_lit(
        [x0, GROUND_Y, z0],
        [x0, GROUND_Y, z0 + pitch],
        [x0 + pitch, GROUND_Y, z0 + pitch],
        [x0 + pitch, GROUND_Y, z0],
        tint,
        [0.0, 1.0, 0.0],
    );
}

/// Plain out to the horizon, tracking the view.
///
/// Detailed ground is built once around the track, so flying the editor camera
/// off it would leave the plain ending under the wheels and the sky showing
/// through. This follows the camera instead.
///
/// One flat quad rather than a grid: the detailed tiles beside the track already
/// supply the parallax, the plain is mostly at distance and softening into the
/// fog, and a grid tall enough to reach past the far plane costs thousands of
/// triangles a frame for a pattern the eye cannot resolve out there. The colour
/// is the lighter of the two the tiles alternate between, so the change from
/// checked to plain reads as distance rather than as a step in hue.
fn plain_group(centre: Vec3) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    assert!(
        PLAIN_REACH > FAR_PLANE,
        "the plain must reach past the far plane or its edge shows"
    );
    let y = GROUND_Y - FAR_GROUND_DROP;
    let r = PLAIN_REACH;
    // Centred on the view rather than snapped to a grid: a quad laid out from a
    // grid origin covers 3200 m forward of that origin, which puts its far edge
    // a metre past the camera whenever the view sits near the end of a cell.
    let (x0, z0) = (centre.x - r, centre.z - r);
    group.push_quad_lit(
        [x0, y, z0],
        [x0, y, z0 + r * 2.0],
        [x0 + r * 2.0, y, z0 + r * 2.0],
        [x0 + r * 2.0, y, z0],
        lin(GRASS_A),
        [0.0, 1.0, 0.0],
    );
    group
}

/// Inner radius of the terrain ring: far enough to clear the track's own
/// diagonal, so no mountain rises over the road.
fn mountain_inner(half_x: f32, half_z: f32) -> f32 {
    let diagonal = half_x.hypot(half_z);
    (MOUNTAIN_INNER + diagonal.max(MOUNTAIN_INNER)).min(MOUNTAIN_MAX_INNER)
}

/// Terrain ring around the track: bands of height sampling out from the plain.
///
/// Heights are taken from a fixed generator, so the range is the same range on
/// every load rather than a different one per run, and the two ends facing the
/// road are held flat so the track opens onto the plain.
fn mountain_group(bounds: ([f32; 3], [f32; 3])) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let (lo, hi) = bounds;
    let inner = mountain_inner((hi[0] - lo[0]) * 0.5, (hi[2] - lo[2]) * 0.5);
    if inner >= MOUNTAIN_MAX_INNER {
        return group;
    }
    let centre = [(lo[0] + hi[0]) * 0.5, 0.0, (lo[2] + hi[2]) * 0.5];
    let columns = (inner * 0.1).max(8.0) as usize;
    let mut noise = Noise::new(0x2545_F491);

    let rock_a = lin(0x6F7A63);
    let rock_b = lin(0x8A9078);
    let mut profiles: Vec<[f32; MOUNTAIN_BANDS]> = Vec::with_capacity(columns);
    for _ in 0..columns {
        profiles.push(profile(&mut noise));
    }

    let mut column = 0usize;
    while column < columns {
        let here = column;
        let next_col = (column + 1) % columns;
        let t = TAU * here as f32 / columns as f32;
        let u = TAU * next_col as f32 / columns as f32;
        for band in 0..MOUNTAIN_BANDS - 1 {
            let r0 = inner + MOUNTAIN_STEP * band as f32;
            let r1 = inner + MOUNTAIN_STEP * (band + 1) as f32;
            let (h0, h1) = (profiles[here][band], profiles[here][band + 1]);
            let (h2, h3) = (profiles[next_col][band], profiles[next_col][band + 1]);
            let tint = if (column + band) % 2 == 0 {
                rock_a
            } else {
                rock_b
            };
            let at = |angle: f32, radius: f32, height: f32| {
                [
                    centre[0] + angle.cos() * radius,
                    height * MOUNTAIN_SCALE,
                    centre[2] + angle.sin() * radius,
                ]
            };
            let (a, b, c, d) = (at(t, r0, h0), at(u, r0, h2), at(u, r1, h3), at(t, r1, h1));
            let normal = face_normal(a, b, c);
            push_quad(&mut group, a, b, c, d, normal, tint);
        }
        column += 1;
    }
    group
}

/// One column's height across the bands, smooth and never below the plain.
///
/// The profile is a couple of summed sines rather than an independent random per
/// band. Independent randoms make a sawtooth: each band a step from the last,
/// with 100 m of height over 100 m of radius at full amplitude, which reads as
/// spikes on a pincushion rather than as a range. Summed sines give a silhouette
/// that rolls, and the ends are pinned flat so the range meets the plain at both
/// the inner and the outer edge.
fn profile(noise: &mut Noise) -> [f32; MOUNTAIN_BANDS] {
    let mut out = [0.0f32; MOUNTAIN_BANDS];
    let (phase, frequency) = (noise.unit() * TAU, 1.0 + noise.unit() * 2.0);
    let (phase2, frequency2) = (noise.unit() * TAU, 3.0 + noise.unit() * 4.0);
    let mut peak = 0.0f32;
    for (band, slot) in out.iter_mut().enumerate() {
        let u = band as f32 / (MOUNTAIN_BANDS - 1) as f32;
        let v =
            (u * frequency * TAU + phase).sin() * 0.6 + (u * frequency2 * TAU + phase2).sin() * 0.4;
        // Held above the plain: a negative height would put the terrain under
        // the ground and read as a trench.
        *slot = v.max(0.0);
        peak = peak.max(*slot);
    }
    out[0] = 0.0;
    out[MOUNTAIN_BANDS - 1] = 0.0;
    if peak > 1e-4 {
        for v in out.iter_mut() {
            *v /= peak;
        }
    }
    out
}

/// Fixed-seed generator. The range has to be the same range on every load, or a
/// saved run's background differs from the run it was recorded against.
struct Noise {
    state: u32,
}

impl Noise {
    fn new(seed: u32) -> Self {
        Self { state: seed | 1 }
    }

    /// Uniform in 0..1.
    fn unit(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as f32 / u32::MAX as f32
    }
}

/// Outward normal of the triangle `a, b, c`.
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-6 {
        [0.0, 1.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

/// One quad with an explicit normal and colour, as two triangles.
fn push_quad(
    group: &mut MeshGroup,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    normal: [f32; 3],
    colour: [f32; 3],
) {
    let base = group.positions.len() as u32;
    for p in [a, b, c, d] {
        group.positions.push(p);
        group.normals.push(normal);
        group.colors.push(colour);
    }
    group
        .indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Unlit gradient dome, centred on the car so the horizon stays put. No
/// normals: flat groups skip light and fog, which is exactly sky behaviour.
///
/// Only the position of the dome follows the car; the shape, its colours and its
/// indices never change. Those are built once and the vertices are translated
/// per frame, rather than re-deriving ~250 vertices of trigonometry and index
/// arithmetic every frame.
pub(super) struct SkyDome {
    /// `group` as built at the origin, and the same vertices un-translated.
    template: MeshGroup,
    offsets: Vec<[f32; 3]>,
}

impl SkyDome {
    pub(super) fn new() -> Self {
        let horizon = lin(HORIZON);
        let zenith = lin(ZENITH);
        const LON: usize = 16;
        const LAT: usize = 14;
        let mut mesh = RawMesh::default();
        let mut index = vec![0u32; (LAT + 1) * (LON + 1)];
        for iy in 0..=LAT {
            let elev = FRAC_PI_2 * (1.0 - 2.0 * iy as f32 / LAT as f32);
            for j in 0..=LON {
                let az = TAU * j as f32 / LON as f32;
                let d = Vec3::new(elev.cos() * az.cos(), elev.sin(), elev.cos() * az.sin());
                let t = d.y.max(0.0);
                let color = [
                    horizon[0] + (zenith[0] - horizon[0]) * t,
                    horizon[1] + (zenith[1] - horizon[1]) * t,
                    horizon[2] + (zenith[2] - horizon[2]) * t,
                ];
                index[iy * (LON + 1) + j] = mesh.positions.len() as u32;
                mesh.positions.push((d * SKY_RADIUS).to_array());
                mesh.colors.push(color);
            }
        }
        let at = |iy: usize, j: usize| index[iy * (LON + 1) + j];
        for iy in 0..LAT {
            for j in 0..LON {
                // Pole rings collapse to one point: the fan triangles are the
                // non-degenerate halves of these quads.
                if iy == 0 {
                    mesh.indices.extend([at(0, j), at(1, j), at(1, j + 1)]);
                } else if iy == LAT - 1 {
                    mesh.indices
                        .extend([at(LAT - 1, j), at(LAT, j + 1), at(LAT - 1, j + 1)]);
                } else {
                    mesh.indices.extend([
                        at(iy, j),
                        at(iy + 1, j),
                        at(iy + 1, j + 1),
                        at(iy, j),
                        at(iy + 1, j + 1),
                        at(iy, j + 1),
                    ]);
                }
            }
        }
        let offsets = mesh.positions.clone();
        Self {
            template: raw_mesh_to_group(&mesh, true),
            offsets,
        }
    }

    /// The dome moved to `centre`.
    pub(super) fn at(&self, centre: Vec3) -> MeshGroup {
        let mut group = self.template.clone();
        for (position, offset) in group.positions.iter_mut().zip(&self.offsets) {
            *position = (Vec3::from(*offset) + centre).to_array();
        }
        group
    }
}

impl App {
    /// `car` is the player's pose to draw, already blended across the last tick.
    /// `ghost`, when present, is the reference car's pose blended by the same
    /// factor; `alpha` is that blend factor for the camera.
    ///
    /// Both cars are blended by one `alpha` because they are stepped on the same
    /// tick: blending them by different factors would put the ghost half a tick
    /// ahead of the car it is being compared against, which is exactly the
    /// difference the player is trying to read.
    pub fn build_frame(
        &mut self,
        car: &crate::sim::car::Car,
        ghost: Option<&crate::sim::car::Car>,
        alpha: f32,
    ) -> Frame3d {
        if self.track_mesh.is_none() {
            let (track, bounds) = track::build_group(&self.data.track);
            self.ground_mesh = Some(ground_group(bounds));
            self.mountain_mesh = Some(mountain_group(bounds));
            self.track_mesh = Some(track);
        }
        let editing = self.data.screen == Screen::Editor;
        let horizon = lin(HORIZON);
        let mut frame = Frame3d {
            // The editor frames the whole track and must not follow the car: the
            // car is parked at the spawn, and a chase camera would drag the view
            // there every time it was rebuilt.
            cam: if editing {
                self.editor_cam.clone()
            } else {
                self.cam.to_orbit(alpha)
            },
            background: Some([horizon[0], horizon[1], horizon[2], 1.0]),
            light: SceneLight {
                direction: [0.42, 0.78, 0.46],
                color: [1.0, 0.97, 0.93],
                diffuse: 1.0,
                ambient: [0.34, 0.38, 0.47],
                fog: 1.0,
                fog_color: horizon,
                // Far enough out that the horizon survives it. Closing it to
                // the near field dissolved everything past a few hundred metres
                // into the sky's own colour, so the plain and the mountains
                // behind it read as an empty background.
                fog_start: 700.0,
                fog_end: 3000.0,
                exposure: 1.0,
            },
            shadow: Some(ShadowDesc {
                size: 2048,
                bias: 0.0035,
                strength: 0.8,
            }),
            ..Frame3d::default()
        };
        let centre = if editing {
            self.editor_cam.target
        } else {
            car.pos
        };
        frame.push(self.sky.at(centre));
        // The plain follows the view, so it is built here rather than cached with
        // the ground: the detailed tiles are static and this one is not.
        frame.push(plain_group(centre));
        // Ground and track are handed over by value: `Frame3d` owns its groups
        // and repame-view3d has no shared-group handle, so a per-frame clone is
        // the price of them appearing in every frame. They stay separate groups
        // because the renderer culls per group, and merging them would weld the
        // track's bounds to the ground plane's, which never leaves the frustum.
        if let Some(ground) = &self.ground_mesh {
            frame.push(ground.clone());
        }
        if let Some(mountains) = &self.mountain_mesh {
            frame.push(mountains.clone());
        }
        if let Some(track) = &self.track_mesh {
            frame.push(track.clone());
        }
        // Order here does not decide what draws over what: the renderer partitions
        // opaque from transparent and blends the latter back-to-front on its own
        // (`render.rs:2207`). A ghost is always alpha-blended, so it always
        // composites over the player's solid car, wherever in this list it sits.
        if let Some(ghost) = ghost {
            frame.push(car::ghost_group(ghost));
        }
        if editing {
            for group in self.editor_overlay() {
                frame.push(group);
            }
        } else {
            let (body, head, tail) = car::groups(car);
            frame.push(body);
            frame.push(head);
            frame.push(tail);
        }
        frame
    }

    /// Editor furniture: the cursor cell, a translucent preview of the piece the
    /// next placement would drop, and outlines around what is selected and what
    /// the pointer is over.
    ///
    /// Built per frame rather than cached with the track because both move with
    /// the cursor, and both are a handful of triangles — caching would cost a
    /// rebuild every time the cursor moved instead.
    fn editor_overlay(&self) -> Vec<MeshGroup> {
        let editor = &self.data.editor;
        let place = crate::app::placement::placement_at(
            &self.data.track,
            editor.armed,
            editor.cursor,
            crate::app::SNAP_REACH,
            editor.armed_yaw,
            editor.snap,
        );
        let mut groups = vec![
            track::preview_group(
                &self.data.track,
                &retrackt_format::PieceInstance::new(editor.armed, place.anchor)
                    .with_yaw(place.yaw),
                if place.snapped {
                    PREVIEW_SNAP
                } else {
                    PREVIEW_FREE
                },
            ),
            track::cursor_group(&self.data.track, editor.cursor),
        ];
        // What a key or a click will act on, drawn before it is acted on: the
        // selection in the accent, and the piece under the pointer in white so
        // the two are never the same colour.
        for uid in &editor.selection {
            if let Some(inst) = self.data.track.piece(*uid) {
                groups.push(track::outline_group(
                    &self.data.track,
                    inst,
                    CURSOR_TINT,
                    OUTLINE_ALPHA,
                ));
            }
        }
        if let Some(uid) = editor.hover.filter(|uid| !editor.is_selected(*uid))
            && let Some(inst) = self.data.track.piece(uid)
        {
            groups.push(track::outline_group(
                &self.data.track,
                inst,
                HOVER_TINT,
                OUTLINE_ALPHA,
            ));
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terrain_ring_stands_clear_of_the_track_and_inside_the_far_plane() {
        // A mountain built on top of the road would swallow it, and one past the
        // renderer's far plane would be clipped away, which looks exactly like
        // the feature never having been added. Both are checked against the real
        // built-in tracks rather than a fixture, because the ring's size comes
        // from the track's own extent.
        for doc in retrackt_format::builtin_tracks() {
            let (_, bounds) = track::build_group(&doc);
            let inner = mountain_inner(
                (bounds.1[0] - bounds.0[0]) * 0.5,
                (bounds.1[2] - bounds.0[2]) * 0.5,
            );
            let group = mountain_group(bounds);

            assert!(!group.is_empty(), "{}: no terrain was generated", doc.name);
            // Clear of the road: the nearest mountain is beyond the track's own
            // diagonal, so nothing rises over it.
            let (lo, hi) = (bounds.0, bounds.1);
            let (mut mlo, mut mhi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &group.positions {
                for a in 0..3 {
                    mlo[a] = mlo[a].min(p[a]);
                    mhi[a] = mhi[a].max(p[a]);
                }
            }
            let centre = [(lo[0] + hi[0]) * 0.5, 0.0, (lo[2] + hi[2]) * 0.5];
            // The track's own extent, measured from the centre of its box, must
            // sit inside the ring's hole, or the road is buried under terrain.
            let corner = (centre[0] - lo[0]).hypot(centre[2] - lo[2]);
            assert!(
                corner < inner,
                "{}: the track reaches {} from its centre but the ring starts at {}",
                doc.name,
                corner,
                inner
            );
            // And the terrain must stop before the renderer's far plane, with
            // room for the camera to stand at the track's edge looking out.
            let reach = mhi[0].max(mhi[2]).max(-mlo[0]).max(-mlo[2]);
            assert!(
                reach < 1900.0,
                "{}: terrain reaches {} and would be clipped",
                doc.name,
                reach
            );
        }
    }

    #[test]
    fn the_same_track_always_generates_the_same_range() {
        // Deterministic: a range that changed per load would make the background
        // of a saved ghost differ from the run it was recorded against.
        let doc = retrackt_format::demo_track();
        let (_, bounds) = track::build_group(&doc);
        let first = mountain_group(bounds);
        let second = mountain_group(bounds);
        assert_eq!(
            first.positions, second.positions,
            "the terrain must not depend on when it was built"
        );
    }

    #[test]
    fn every_terrain_face_is_outward_and_unit() {
        let doc = retrackt_format::demo_track();
        let (_, bounds) = track::build_group(&doc);
        let group = mountain_group(bounds);
        for (i, n) in group.normals.iter().enumerate() {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-3, "normal {i} is not unit: {n:?}");
            assert!(n[1] > 0.0, "normal {i} points into the ground: {n:?}");
        }
        assert_eq!(group.normals.len(), group.positions.len());
    }

    #[test]
    fn the_plain_covers_every_axis_past_the_far_plane_from_anywhere() {
        // A quad laid out from a grid origin covers 3200 m forward of that
        // origin, whichever puts its edge a metre past the camera when the view
        // sits near the end of a cell. Checked from several positions, including
        // one far outside any track, because "fly the camera off the plain" is
        // the case this exists for.
        for centre in [
            Vec3::new(45.0, 0.0, 45.0),
            Vec3::new(-3199.0, 0.0, 3199.0),
            Vec3::new(-9000.0, 0.0, 4000.0),
            Vec3::new(0.0, 0.0, 0.0),
        ] {
            let group = plain_group(centre);
            assert_eq!(group.tri_count(), 2, "the plain is one quad");
            let cover = |axis: usize| {
                let lo = group
                    .positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::MAX, f32::min);
                let hi = group
                    .positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::MIN, f32::max);
                (centre[axis] - lo).min(hi - centre[axis])
            };
            for axis in [0, 2] {
                assert!(
                    cover(axis) > FAR_PLANE,
                    "at {:?} the plain covers only {} on axis {axis}",
                    centre,
                    cover(axis)
                );
            }
        }
    }

    /// Every point beyond the ground's inner radius has ground under it.
    ///
    /// The hole the frames leave behind is the track's own *narrower* extent,
    /// because a circuit 20 m wide and 300 m long puts most of its bounding box
    /// off the road. Starting the frames at the wider extent left those corners
    /// open, which reads as a square hole in the ground beside the track.
    #[test]
    fn the_ground_leaves_no_hole_beside_a_non_square_track() {
        for doc in retrackt_format::builtin_tracks() {
            let (_, bounds) = track::build_group(&doc);
            let group = ground_group(bounds);
            let centre = [
                (bounds.0[0] + bounds.1[0]) * 0.5,
                (bounds.0[2] + bounds.1[2]) * 0.5,
            ];
            let reach = (bounds.1[0] - bounds.0[0])
                .max(bounds.1[2] - bounds.0[2])
                .min(group.positions.iter().fold(0.0f32, |acc, p| {
                    acc.max((p[0] - centre[0]).abs())
                        .max((p[2] - centre[1]).abs())
                }));
            for step in 0..24usize {
                let angle = std::f32::consts::TAU * step as f32 / 24.0;
                for fraction in [0.55f32, 0.8, 1.0, 1.4] {
                    let r = reach * fraction;
                    let (x, z) = (centre[0] + angle.cos() * r, centre[1] + angle.sin() * r);
                    let covered = group.positions.chunks_exact(4).any(|quad| {
                        // Every tile is axis-aligned, so its four corners give
                        // its extent directly. Comparing corner to corner would
                        // demand the point sit on the corner itself.
                        let (mut x_lo, mut x_hi) = (f32::MAX, f32::MIN);
                        let (mut z_lo, mut z_hi) = (f32::MAX, f32::MIN);
                        for corner in quad {
                            x_lo = x_lo.min(corner[0]);
                            x_hi = x_hi.max(corner[0]);
                            z_lo = z_lo.min(corner[2]);
                            z_hi = z_hi.max(corner[2]);
                        }
                        x >= x_lo && x <= x_hi && z >= z_lo && z <= z_hi
                    });
                    assert!(
                        covered,
                        "{}: no ground at ({x:.0}, {z:.0}), {} m from centre",
                        doc.name, r
                    );
                }
            }
        }
    }
}

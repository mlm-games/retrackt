mod car;
mod track;

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::Vec3;
use repame_view3d::render::TonemapMethod;
use repame_view3d::{Frame3d, MeshGroup, SceneLight, ShadowDesc};
use retrackt_format::geometry::RawMesh;
use retrackt_format::{ENVIRONMENT_DESERT, ENVIRONMENT_SUMMER, ENVIRONMENT_WINTER};

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

/// Far ground sits a hair under the detailed ground so the two never z-fight.
const FAR_GROUND_DROP: f32 = 0.05;
/// Edge of the plain, and the renderer's far plane it has to reach past from
/// wherever the camera has got to, which is the whole reason it tracks the view.
const PLAIN_REACH: f32 = 3200.0;
const FAR_PLANE: f32 = 2000.0;

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

/// Everything a track's environment decides about how it is drawn.
///
/// One value derived in one place, because the sky, the ground and the light are
/// otherwise three separate reads of the same document and drift apart the moment
/// one of them grows a new knob.
struct Atmosphere {
    /// Which of the environments these colours belong to, so the meshes that bake
    /// them in know when they are stale.
    environment: u8,
    horizon: [f32; 3],
    zenith: [f32; 3],
    ground_a: [f32; 3],
    ground_b: [f32; 3],
    ambient: [f32; 3],
    /// Unit vector toward the sun.
    sun: [f32; 3],
    /// The colour the sun throws, which separates a desert afternoon from a
    /// winter one as much as the sky above them does.
    sun_color: [f32; 3],
}

/// The atmosphere a track is drawn in, from the environment it names and how far
/// round the compass its sun sits.
///
/// Summer is the fallthrough rather than a listed arm: it is the value a
/// document from before environments existed reads as, and it is the sky the
/// renderer was first drawn for, so a track that has never heard of the setting
/// opens looking exactly as it was saved. Elevation is per environment rather
/// than per track, because what a summer noon and a winter noon share is where
/// the sun is, not how high it hangs.
fn atmosphere(environment: u8, sun_direction_deg: u16) -> Atmosphere {
    let (mut at, elevation_deg) = match environment {
        ENVIRONMENT_DESERT => (
            Atmosphere {
                environment,
                horizon: lin(0xEBD3AE),
                zenith: lin(0x5B93CE),
                ground_a: lin(0xD8B978),
                ground_b: lin(0xC9A868),
                ambient: [0.40, 0.36, 0.32],
                sun: [0.0; 3],
                sun_color: [1.0, 0.94, 0.82],
            },
            62.0,
        ),
        ENVIRONMENT_WINTER => (
            Atmosphere {
                environment,
                horizon: lin(0xDCE6EF),
                zenith: lin(0x7FA6C9),
                ground_a: lin(0xE8EEF4),
                ground_b: lin(0xD6E0EA),
                ambient: [0.46, 0.50, 0.58],
                sun: [0.0; 3],
                sun_color: [0.96, 0.97, 1.0],
            },
            27.0,
        ),
        _ => (
            Atmosphere {
                environment: ENVIRONMENT_SUMMER,
                horizon: lin(HORIZON),
                zenith: lin(ZENITH),
                ground_a: lin(GRASS_A),
                ground_b: lin(GRASS_B),
                ambient: [0.34, 0.38, 0.47],
                sun: [0.0; 3],
                sun_color: [1.0, 0.97, 0.93],
            },
            51.0,
        ),
    };
    // Wrapped rather than folded into the slider: a bearing of 360 is the same
    // bearing as 0, and taking it modulo keeps `sin` and `cos` off the seam.
    let bearing = f32::from(sun_direction_deg % 360) * (PI / 180.0);
    let elevation = elevation_deg * (PI / 180.0);
    at.sun = [
        elevation.cos() * bearing.cos(),
        elevation.sin(),
        elevation.cos() * bearing.sin(),
    ];
    at
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

/// Shadow map edge, in texels, for one graphics preset.
///
/// The steps are doublings: a shadow map's cost is quadratic in its edge, so a
/// middle step is a setting nobody can see the difference of.
fn shadow_map_size(quality: u8) -> u32 {
    match quality {
        0 => 512,
        1 => 1024,
        3 => 4096,
        _ => 2048,
    }
}

fn ground_group(bounds: ([f32; 3], [f32; 3]), at: &Atmosphere) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let a = at.ground_a;
    let b = at.ground_b;
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
fn plain_group(centre: Vec3, at: &Atmosphere) -> MeshGroup {
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
        at.ground_a,
        [0.0, 1.0, 0.0],
    );
    group
}

/// Terrain ring around the track, drawn.
///
/// The triangles come from the format crate so the scene and the collision
/// world agree on where the range is: a mountain you can see but not touch is a
/// mountain you drive through.
fn mountain_group(bounds: ([f32; 3], [f32; 3])) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let rock_a = lin(0x6F7A63);
    let rock_b = lin(0x8A9078);
    for (i, (a, b, c, n)) in retrackt_format::geometry::mountain_triangles(bounds, 0.0)
        .into_iter()
        .enumerate()
    {
        let tint = if i % 2 == 0 { rock_a } else { rock_b };
        // A triangle, not a quad: the generator returns triangles already, and
        // collapsing one into a quad leaves a degenerate second face whose
        // normal points the wrong way.
        group.push_tri_lit(a, b, c, tint, n);
    }
    group
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
    /// How far each vertex sits between horizon and zenith, kept from the build
    /// so recolouring does not have to work it out from the positions again.
    heights: Vec<f32>,
    /// The environment the tints are currently drawn for.
    environment: u8,
}

impl SkyDome {
    pub(super) fn new() -> Self {
        // Built in summer and recoloured by `set_atmosphere` if the track it ends
        // up drawing is not, so the vertex colours are stated once, in the one
        // place the environments are.
        let at = atmosphere(ENVIRONMENT_SUMMER, 0);
        let horizon = at.horizon;
        let zenith = at.zenith;
        const LON: usize = 16;
        const LAT: usize = 14;
        let mut mesh = RawMesh::default();
        let mut heights = Vec::new();
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
                heights.push(t);
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
            heights,
            environment: ENVIRONMENT_SUMMER,
        }
    }

    /// Recolour the dome for a new environment, and report whether it did.
    ///
    /// Only the tints move: the shape, and the heights they are mixed by, are the
    /// same for every environment, so this is the whole of what a preset costs
    /// and it costs nothing per frame once the preset is set.
    fn set_atmosphere(&mut self, at: &Atmosphere) -> bool {
        if self.environment == at.environment {
            return false;
        }
        for (color, &height) in self.template.colors.iter_mut().zip(&self.heights) {
            *color = [
                at.horizon[0] + (at.zenith[0] - at.horizon[0]) * height,
                at.horizon[1] + (at.zenith[1] - at.horizon[1]) * height,
                at.horizon[2] + (at.zenith[2] - at.horizon[2]) * height,
            ];
        }
        self.environment = at.environment;
        true
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
        let at = atmosphere(
            self.data.track.environment,
            self.data.track.sun_direction_deg,
        );
        // The ground and the dome hold their colours as vertex tints and are built
        // once, so a change of environment puts both back through their builders.
        // The track itself is only rebuilt when it is the thing that changed: the
        // group is built alongside to get the bounds the ground is sized from, and
        // thrown away when there was already a track mesh to keep.
        let restyled = self.sky.set_atmosphere(&at);
        if self.track_mesh.is_none() || restyled {
            let (track, bounds) = track::build_group(&self.data.track);
            self.ground_mesh = Some(ground_group(bounds, &at));
            self.mountain_mesh = Some(mountain_group(bounds));
            if self.track_mesh.is_none() {
                self.track_mesh = Some(track);
            }
        }
        let editing = self.data.screen == Screen::Editor;
        let horizon = at.horizon;
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
                direction: at.sun,
                color: at.sun_color,
                diffuse: 1.0,
                ambient: at.ambient,
                fog: if self.data.settings.fog { 1.0 } else { 0.0 },
                fog_color: horizon,
                // Far enough out that the horizon survives it. Closing it to
                // the near field dissolved everything past a few hundred metres
                // into the sky's own colour, so the plain and the mountains
                // behind it read as an empty background.
                fog_start: 700.0,
                fog_end: 3000.0,
                exposure: 1.0,
                tonemap: TonemapMethod::Reinhard,
            },
            shadow: self.data.settings.shadows.then(|| ShadowDesc {
                size: shadow_map_size(self.data.settings.quality),
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
        frame.push(plain_group(centre, &at));
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
    fn road_edges_read_as_concrete_and_marked_lines() {
        crate::report_failure_where_it_happens();
        // The reference's barrier is poured concrete in two tones, with a white
        // line on the left of the road and a red one on the right. Grey barriers
        // and the two-colour edge lines are what make a road read as road work
        // rather than as a kerb-striped toy track.
        let doc = retrackt_format::demo_track();
        let (group, _) = track::build_group(&doc);

        let (light, dark) = (lin(track::BARRIER_LIGHT), lin(track::BARRIER_DARK));
        let (white, red) = (lin(track::EDGE_WHITE), lin(track::EDGE_RED));
        let near = |c: [f32; 3], t: [f32; 3]| {
            (c[0] - t[0]).abs() < 0.2 && (c[1] - t[1]).abs() < 0.2 && (c[2] - t[2]).abs() < 0.2
        };

        let mut light_used = false;
        let mut dark_used = false;
        let mut white_used = false;
        let mut red_used = false;
        for c in &group.colors {
            if near(*c, light) {
                light_used = true;
            }
            if near(*c, dark) {
                dark_used = true;
            }
            if near(*c, white) {
                white_used = true;
            }
            if near(*c, red) {
                red_used = true;
            }
        }
        assert!(light_used, "no pale concrete in the barrier");
        assert!(dark_used, "no dark concrete in the barrier");
        assert!(white_used, "no white edge line");
        assert!(red_used, "no red edge line");
    }

    #[test]
    fn the_terrain_ring_stands_clear_of_the_track_and_inside_the_far_plane() {
        crate::report_failure_where_it_happens();
        // A mountain built on top of the road would swallow it, and one past the
        // renderer's far plane would be clipped away, which looks exactly like
        // the feature never having been added. Both are checked against the real
        // built-in tracks rather than a fixture, because the ring's size comes
        // from the track's own extent.
        for doc in retrackt_format::builtin_tracks() {
            let (_, bounds) = track::build_group(&doc);
            let group = mountain_group(bounds);

            assert!(!group.is_empty(), "{}: no terrain was generated", doc.name);
            // Measured off the triangles actually built, rather than restating
            // the generator's rule: the near edge of the ring is its lowest
            // radius from the track's centre.
            let (lo, hi0) = (bounds.0, bounds.1);
            let centre = [(lo[0] + hi0[0]) * 0.5, 0.0, (lo[2] + hi0[2]) * 0.5];
            let inner = group
                .positions
                .iter()
                .map(|p| (p[0] - centre[0]).hypot(p[2] - centre[2]))
                .fold(f32::MAX, f32::min);
            // Clear of the road: the nearest mountain is beyond the track's own
            // diagonal, so nothing rises over it.
            let (mut mlo, mut mhi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &group.positions {
                for a in 0..3 {
                    mlo[a] = mlo[a].min(p[a]);
                    mhi[a] = mhi[a].max(p[a]);
                }
            }

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
        crate::report_failure_where_it_happens();
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
        crate::report_failure_where_it_happens();
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
        crate::report_failure_where_it_happens();
        // A quad laid out from a grid origin covers 3200 m forward of that
        // origin, whichever puts its edge a metre past the camera when the view
        // sits near the end of a cell. Checked from several positions, including
        // one far outside any track, because "fly the camera off the plain" is
        // the case this exists for.
        // The quad's reach does not depend on what it is coloured, so this asks
        // with the default atmosphere rather than naming a track.
        let at = atmosphere(ENVIRONMENT_SUMMER, 0);
        for centre in [
            Vec3::new(45.0, 0.0, 45.0),
            Vec3::new(-3199.0, 0.0, 3199.0),
            Vec3::new(-9000.0, 0.0, 4000.0),
            Vec3::new(0.0, 0.0, 0.0),
        ] {
            let group = plain_group(centre, &at);
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
        crate::report_failure_where_it_happens();
        for doc in retrackt_format::builtin_tracks() {
            let (_, bounds) = track::build_group(&doc);
            let group = ground_group(bounds, &atmosphere(doc.environment, doc.sun_direction_deg));
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

    /// A track saved before a document could name an environment opens under the
    /// sky it was saved with. The bearing is the one the light was hard-coded at,
    /// so this is the difference between an old track looking unchanged and every
    /// track ever saved looking subtly wrong the day this setting was added.
    #[test]
    fn a_track_that_names_no_environment_opens_in_the_sky_it_was_saved_under() {
        crate::report_failure_where_it_happens();
        let sun = atmosphere(
            ENVIRONMENT_SUMMER,
            retrackt_format::DEFAULT_SUN_DIRECTION_DEG,
        )
        .sun;
        for (got, want) in sun.iter().zip([0.42, 0.78, 0.46]) {
            assert!(
                (got - want).abs() < 0.02,
                "the default sun is {sun:?}, which is not the light tracks were saved under"
            );
        }
    }

    /// Two environments that draw the same sky are one environment twice, and the
    /// editor would be offering a choice it cannot honour.
    #[test]
    fn every_environment_is_a_different_sky() {
        crate::report_failure_where_it_happens();
        let horizons: Vec<[f32; 3]> = retrackt_format::ENVIRONMENTS
            .iter()
            .map(|(id, _)| atmosphere(*id, 0).horizon)
            .collect();
        for (at, horizon) in horizons.iter().enumerate() {
            for other in horizons.iter().skip(at + 1) {
                assert_ne!(horizon, other, "two environments draw the same horizon");
            }
        }
    }
}

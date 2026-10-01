mod car;
mod track;

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::Vec3;
use repame_view3d::{Frame3d, MeshGroup, SceneLight, ShadowDesc};
use retrackt_format::geometry::RawMesh;

use super::App;
use super::schedule::CarRes;

const GROUND_Y: f32 = -0.5;
const TILE: f32 = 8.0;
const GROUND_TILES: i32 = 64;
const GROUND_MARGIN: f32 = 64.0;

const HORIZON: u32 = 0xCFE6F7;
const ZENITH: u32 = 0x4E9BE0;
const GRASS_A: u32 = 0x69BD5F;
const GRASS_B: u32 = 0x5BB052;
/// Inside the far plane, outside the ground ring: the dome always hides the
/// world edge while the camera stays within the track.
const SKY_RADIUS: f32 = 900.0;

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
    for z in tile_range(lo[2], hi[2]) {
        for x in tile_range(lo[0], hi[0]) {
            let tint = if (x + z).rem_euclid(2) == 0 { a } else { b };
            let x0 = x as f32 * TILE;
            let z0 = z as f32 * TILE;
            group.push_quad_lit(
                [x0, GROUND_Y, z0],
                [x0, GROUND_Y, z0 + TILE],
                [x0 + TILE, GROUND_Y, z0 + TILE],
                [x0 + TILE, GROUND_Y, z0],
                tint,
                [0.0, 1.0, 0.0],
            );
        }
    }
    group
}

fn tile_range(lo: f32, hi: f32) -> std::ops::Range<i32> {
    let start = (((lo - GROUND_MARGIN) / TILE).floor() as i32).clamp(-GROUND_TILES, GROUND_TILES);
    let end = (((hi + GROUND_MARGIN) / TILE).ceil() as i32)
        .clamp(-GROUND_TILES, GROUND_TILES)
        .max(start + 1);
    start..end
}

/// Unlit gradient dome, centred on the car so the horizon stays put. No
/// normals: flat groups skip light and fog, which is exactly sky behaviour.
fn sky_group(centre: Vec3) -> MeshGroup {
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
            mesh.positions.push((centre + d * SKY_RADIUS).to_array());
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
    raw_mesh_to_group(&mesh, true)
}

impl App {
    pub fn build_frame(&mut self) -> Frame3d {
        if self.track_mesh.is_none() {
            let (track, bounds) = track::build_group(&self.data.track);
            self.ground_mesh = Some(ground_group(bounds));
            self.track_mesh = Some(track);
        }
        let horizon = lin(HORIZON);
        let mut frame = Frame3d {
            cam: self.cam.to_orbit(),
            background: Some([horizon[0], horizon[1], horizon[2], 1.0]),
            light: SceneLight {
                direction: [0.42, 0.78, 0.46],
                color: [1.0, 0.97, 0.93],
                diffuse: 1.0,
                ambient: [0.34, 0.38, 0.47],
                fog: 1.0,
                fog_color: horizon,
                fog_start: 140.0,
                fog_end: 430.0,
                exposure: 1.0,
            },
            shadow: Some(ShadowDesc {
                size: 2048,
                bias: 0.0035,
                strength: 0.8,
            }),
            ..Frame3d::default()
        };
        let car = self.sim.world.resource::<CarRes>().0;
        frame.push(sky_group(car.pos));
        if let Some(ground) = &self.ground_mesh {
            frame.push(ground.clone());
        }
        if let Some(track) = &self.track_mesh {
            frame.push(track.clone());
        }
        let (body, head, tail) = car::groups(&car);
        frame.push(body);
        frame.push(head);
        frame.push(tail);
        frame
    }
}

use glam::Vec3;
use repame_view3d::{Frame3d, MeshGroup};
use retrackt_format::geometry::RawMesh;
use retrackt_format::{TrackDocument, piece_shape};

use crate::sim::car::{Car, body_matrix};

use super::App;
use super::schedule::CarRes;

const ROAD_TINT: [f32; 3] = [0.30, 0.32, 0.36];
const DECOR_TINT: [f32; 3] = [0.86, 0.54, 0.14];
const CAR_TINT: [f32; 3] = [0.80, 0.16, 0.13];
const SKY: [f32; 4] = [0.05, 0.07, 0.11, 1.0];
const GROUND_Y: f32 = -0.5;
const TILE: f32 = 8.0;
const GROUND_TILES: i32 = 64;
const GROUND_MARGIN: f32 = 64.0;

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

fn build_track_groups(doc: &TrackDocument) -> (MeshGroup, MeshGroup) {
    let mut merged = RawMesh::default();
    for inst in &doc.pieces {
        let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
        let xform = inst.world_matrix(doc.cell_size);
        merge(&mut merged, &shape.surface.transformed(xform, ROAD_TINT));
        merge(&mut merged, &shape.decor.transformed(xform, DECOR_TINT));
    }
    let bounds = merged
        .bounds()
        .unwrap_or(([-32.0, 0.0, -32.0], [32.0, 0.0, 32.0]));
    (ground_group(bounds), raw_mesh_to_group(&merged, true))
}

fn ground_group(bounds: ([f32; 3], [f32; 3])) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let (lo, hi) = bounds;
    for z in tile_range(lo[2], hi[2]) {
        for x in tile_range(lo[0], hi[0]) {
            let tint = if (x + z).rem_euclid(2) == 0 {
                [0.17, 0.23, 0.15]
            } else {
                [0.13, 0.18, 0.12]
            };
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

/// The car as six oriented quads. `push_box` is axis-aligned only, so the
/// faces are transformed through the body matrix by hand.
fn car_group(car: &Car) -> MeshGroup {
    let mut group = MeshGroup {
        depth_test: true,
        ..MeshGroup::default()
    };
    let body = body_matrix(car);
    let project = |v: Vec3| {
        let p = body * v.extend(1.0);
        [p.x, p.y, p.z]
    };
    let rotate = |v: Vec3| (car.orient * v).to_array();
    let quad = |group: &mut MeshGroup, a: Vec3, b: Vec3, c: Vec3, d: Vec3, n: Vec3| {
        group.push_quad_lit(
            project(a),
            project(b),
            project(c),
            project(d),
            CAR_TINT,
            rotate(n),
        );
    };
    let (hx, hy, hz) = (0.8, 0.45, 1.3);
    quad(
        &mut group,
        Vec3::new(-hx, hy, -hz),
        Vec3::new(-hx, hy, hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(hx, hy, -hz),
        Vec3::Y,
    );
    quad(
        &mut group,
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(hx, -hy, hz),
        Vec3::X,
    );
    quad(
        &mut group,
        Vec3::new(-hx, -hy, hz),
        Vec3::new(-hx, hy, hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(-hx, -hy, -hz),
        Vec3::NEG_X,
    );
    quad(
        &mut group,
        Vec3::new(hx, -hy, hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(-hx, hy, hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::Z,
    );
    quad(
        &mut group,
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::NEG_Z,
    );
    quad(
        &mut group,
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, -hy, hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::NEG_Y,
    );
    group
}

impl App {
    pub fn build_frame(&mut self) -> Frame3d {
        if self.track_mesh.is_none() {
            let (ground, track) = build_track_groups(&self.data.track);
            self.ground_mesh = Some(ground);
            self.track_mesh = Some(track);
        }
        let mut frame = Frame3d {
            cam: self.cam.to_orbit(),
            background: Some(SKY),
            ..Frame3d::default()
        };
        if let Some(ground) = &self.ground_mesh {
            frame.push(ground.clone());
        }
        if let Some(track) = &self.track_mesh {
            frame.push(track.clone());
        }
        let car = self.sim.world.resource::<CarRes>().0;
        frame.push(car_group(&car));
        frame
    }
}

use glam::Vec3;
use repame_view3d::MeshGroup;
use retrackt_format::geometry::{Centerline, RawMesh, reserve_extent};
use retrackt_format::piece::catalog_by_id;
use retrackt_format::{PieceId, PieceInstance, TrackDocument, piece_shape};

use super::{
    CURSOR_ALPHA, CURSOR_TINT, PREVIEW_ALPHA, lin, merge, push_lit, push_obox, push_tri_lit,
    raw_mesh_to_group,
};

const ROAD: u32 = 0x9BA1A9;
const EDGE_LINE: u32 = 0xF2F4F7;
const KERB_A: u32 = 0xE5473C;
const KERB_B: u32 = 0xF2F4F7;
const CHECK_DARK: u32 = 0x1B1E24;
const CHECK_LIGHT: u32 = 0xF2F4F7;
const BOOST: u32 = 0xFFD23F;
const ARCH_DARK: u32 = 0x2E333B;
const ARCH_CYAN: u32 = 0x45C3F0;
const ARCH_ORANGE: u32 = 0xFFB020;

pub(super) fn build_group(doc: &TrackDocument) -> (MeshGroup, ([f32; 3], [f32; 3])) {
    let mut merged = RawMesh::default();
    let road = lin(ROAD);
    for inst in &doc.pieces {
        let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
        let xform = inst.world_matrix(doc.cell_size);
        merge(&mut merged, &shape.surface.transformed(xform, road));

        let mut decor = shape.decor.transformed(xform, [1.0; 3]);
        kerb_palette(&mut decor);
        merge(&mut merged, &decor);

        let mut paint = RawMesh::default();
        piece_paint(inst.id, doc.cell_size, &shape.center, &mut paint);
        if !paint.is_empty() {
            merge(&mut merged, &paint.transformed(xform, [1.0; 3]));
        }
    }
    let bounds = merged
        .bounds()
        .unwrap_or(([-32.0, 0.0, -32.0], [32.0, 0.0, 32.0]));
    (raw_mesh_to_group(&merged, true), bounds)
}

/// One placed piece, drawn flat and translucent: the preview of what a
/// placement would drop.
///
/// Built from the same `piece_shape` as the real mesh, so the preview is the
/// piece rather than an approximation of it — a preview drawn any other way
/// would be wrong exactly where the shape is unusual, which is the case the
/// player is checking it for.
pub(super) fn preview_group(doc: &TrackDocument, inst: &PieceInstance, tint: u32) -> MeshGroup {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    let xform = inst.world_matrix(doc.cell_size);
    let mut mesh = shape.surface.transformed(xform, lin(tint));
    let mut group = raw_mesh_to_group(&mesh, true);
    // One flat albedo and no lighting: a shaded preview competes with the road
    // it is being judged against.
    group.colors.fill(lin(tint));
    group.normals.clear();
    group.transparent = true;
    group.alpha = PREVIEW_ALPHA;
    group.material.roughness = 1.0;
    group.material.emissive = [0.10, 0.10, 0.10];
    group
}

/// Flat outline of one grid cell, so the cursor is findable on a wide track.
///
/// A wireframe rather than a filled quad: a filled cell hides the road under it,
/// and the cursor has to be readable *over* the piece it is about to join.
pub(super) fn cursor_group(doc: &TrackDocument, cell: [i16; 3]) -> MeshGroup {
    let centre = Vec3::from(doc.cell_origin(cell));
    let half = doc.cell_size * 0.5;
    // Thin enough to read as an outline, thick enough not to disappear at the
    // distances the editor camera sits at.
    let t = (doc.cell_size * 0.06).max(0.12);
    let color = lin(CURSOR_TINT);
    let mut mesh = RawMesh::default();
    push_ground_outline(
        &mut mesh,
        (centre.x - half, centre.z - half),
        (centre.x + half, centre.z + half),
        centre.y,
        t,
        color,
    );

    let mut group = raw_mesh_to_group(&mesh, true);
    group.normals.clear();
    group.transparent = true;
    group.alpha = CURSOR_ALPHA;
    // Depth test on: the cursor sits on the road surface, and drawing through it
    // would put an outline on top of scenery the piece is behind.
    group.depth_test = true;
    group
}

/// Outline around one placed piece, for what the pointer is over and what is
/// selected.
///
/// Around the piece's reservation box rather than its mesh, so the shape is a
/// rectangle the eye can read at a glance and the cost is four edges whatever
/// the piece is. Without this the piece under the pointer is invisible, and
/// taking it with a key is a guess.
pub(super) fn outline_group(
    doc: &TrackDocument,
    inst: &PieceInstance,
    tint: u32,
    alpha: f32,
) -> MeshGroup {
    let shape = piece_shape(inst.id, &inst.params, doc.cell_size);
    let (lo, hi) = reserve_extent(&shape, doc.cell_size);
    let world = inst.world_matrix(doc.cell_size);
    let mut lo_corner = Vec3::splat(f32::MAX);
    let mut hi_corner = Vec3::splat(f32::MIN);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let corner = world.transform_point3(Vec3::new(x, y, z));
                lo_corner = lo_corner.min(corner);
                hi_corner = hi_corner.max(corner);
            }
        }
    }
    let t = (doc.cell_size * 0.08).max(0.14);
    let mut mesh = RawMesh::default();
    push_ground_outline(
        &mut mesh,
        (lo_corner.x, lo_corner.z),
        (hi_corner.x, hi_corner.z),
        lo_corner.y,
        t,
        lin(tint),
    );
    let mut group = raw_mesh_to_group(&mesh, true);
    group.normals.clear();
    group.transparent = true;
    group.alpha = alpha;
    group.depth_test = true;
    group
}

/// A flat wireframe rectangle on the ground plane, as a band of two quads per
/// edge: one facing up, one facing out.
///
/// Drawn as quads rather than lines because the scene has no line primitive, and
/// at a thickness in world units rather than in pixels, so an outline stays
/// readable at the range the editor camera works at.
fn push_ground_outline(
    m: &mut RawMesh,
    lo: (f32, f32),
    hi: (f32, f32),
    y: f32,
    t: f32,
    color: [f32; 3],
) {
    let at = |p: (f32, f32), d: f32| Vec3::new(p.0, y + d, p.1);
    let corners = [(lo.0, lo.1), (hi.0, lo.1), (hi.0, hi.1), (lo.0, hi.1)];
    for edge in 0..4 {
        let (a, b) = (corners[edge], corners[(edge + 1) % 4]);
        let along = Vec3::new(b.0 - a.0, 0.0, b.1 - a.1);
        if along.length_squared() < 1e-12 {
            continue;
        }
        let inward = Vec3::new(along.z, 0.0, -along.x).normalize();
        // The band runs along the edge, inset by the thickness, so it lies inside
        // the rectangle rather than straddling it.
        push_lit(
            m,
            [
                at(a, t),
                at(b, t),
                at((b.0 - inward.x * t, b.1 - inward.z * t), t),
                at((a.0 - inward.x * t, a.1 - inward.z * t), t),
            ],
            Vec3::Y,
            color,
        );
        push_lit(m, [at(a, t), at(a, -t), at(b, -t), at(b, t)], inward, color);
    }
}

/// Replaces the stripe mask (white/black per quad) baked into decor colours
/// with the kerb palette.
fn kerb_palette(decor: &mut RawMesh) {
    let (a, b) = (lin(KERB_A), lin(KERB_B));
    for quad in decor.colors.chunks_mut(4) {
        let color = if quad[0][0] >= 0.5 { a } else { b };
        quad.fill(color);
    }
}

fn piece_paint(id: PieceId, cell: f32, c: &Centerline, out: &mut RawMesh) {
    if c.is_empty() || id == PieceId::Wall {
        return;
    }
    edge_lines(c, out);
    let def = catalog_by_id(id);
    let mid = c.length() * 0.5;
    let (dark, light) = (lin(CHECK_DARK), lin(CHECK_LIGHT));
    if def.is_start {
        band(c, 0.55, 2, 10, light, dark, out);
        arch(c, cell, 0.55, Bar::Solid(lin(ARCH_ORANGE)), out);
    } else if def.is_finish {
        band(c, mid, 2, 10, light, dark, out);
        arch(c, cell, mid, Bar::Striped, out);
    } else if def.is_checkpoint {
        band(c, mid, 1, 1, lin(ARCH_CYAN), lin(ARCH_CYAN), out);
        arch(c, cell, mid, Bar::Solid(lin(ARCH_CYAN)), out);
    } else if def.is_boost {
        boost_arrows(c, out);
    }
}

/// White line just inside each road edge, lifted off the surface to beat
/// depth precision at fog range.
fn edge_lines(c: &Centerline, out: &mut RawMesh) {
    const GAP: f32 = 0.10;
    const WIDTH: f32 = 0.16;
    const LIFT: f32 = 0.02;
    let color = lin(EDGE_LINE);
    for i in 0..c.pos.len() - 1 {
        let (p0, p1) = (c.pos[i], c.pos[i + 1]);
        let (n0, n1) = (c.nrm[i], c.nrm[i + 1]);
        let (r0, r1) = (c.tan[i].cross(n0), c.tan[i + 1].cross(n1));
        for s in [-1.0, 1.0] {
            let (u, v) = (s * (c.half_width - GAP), s * (c.half_width - GAP - WIDTH));
            let (lo, hi) = (u.min(v), u.max(v));
            push_lit(
                out,
                [
                    p0 + r0 * lo + n0 * LIFT,
                    p0 + r0 * hi + n0 * LIFT,
                    p1 + r1 * hi + n1 * LIFT,
                    p1 + r1 * lo + n1 * LIFT,
                ],
                n0,
                color,
            );
        }
    }
}

/// Painted rectangle across the road at arc `s`, checkerboarded by
/// `rows x cols`. One row and one column reads as a solid band.
fn band(
    c: &Centerline,
    s: f32,
    rows: usize,
    cols: usize,
    a: [f32; 3],
    b: [f32; 3],
    out: &mut RawMesh,
) {
    const DEPTH: f32 = 0.9;
    const LIFT: f32 = 0.025;
    let row_d = DEPTH / rows as f32;
    let s0 = (s - DEPTH * 0.5).max(0.0);
    for row in 0..rows {
        let near = c.sample(s0 + row as f32 * row_d);
        let far = c.sample(s0 + (row + 1) as f32 * row_d);
        let (Some((p0, t0, n0, hw)), Some((p1, t1, n1, _))) = (near, far) else {
            return;
        };
        let (r0, r1) = (t0.cross(n0), t1.cross(n1));
        for col in 0..cols {
            let w = 2.0 * hw / cols as f32;
            let u0 = -hw + col as f32 * w;
            let color = if (row + col) % 2 == 0 { a } else { b };
            push_lit(
                out,
                [
                    p0 + r0 * u0 + n0 * LIFT,
                    p0 + r0 * (u0 + w) + n0 * LIFT,
                    p1 + r1 * (u0 + w) + n1 * LIFT,
                    p1 + r1 * u0 + n1 * LIFT,
                ],
                n0,
                color,
            );
        }
    }
}

/// Forward arrows down the boost pad.
fn boost_arrows(c: &Centerline, out: &mut RawMesh) {
    const LIFT: f32 = 0.03;
    let len = c.length();
    if len < 2.0 {
        return;
    }
    let color = lin(BOOST);
    let count = ((len / 1.7) as usize).max(2);
    for i in 0..count {
        let s = len * (i as f32 + 0.5) / count as f32;
        let Some((p, t, n, hw)) = c.sample(s) else {
            continue;
        };
        let r = t.cross(n);
        let span = hw * 0.45;
        push_tri_lit(
            out,
            [
                p - r * span - t * 0.45 + n * LIFT,
                p + r * span - t * 0.45 + n * LIFT,
                p + t * 0.55 + n * LIFT,
            ],
            n,
            color,
        );
    }
}

enum Bar {
    Solid([f32; 3]),
    /// Finish bars alternate two colours along the road direction.
    Striped,
}

/// Gate over the road: two posts sunk past the edge, one bar across the top.
/// Post feet drop below the road so a ground-level piece stands in the grass.
fn arch(c: &Centerline, cell: f32, s: f32, bar: Bar, out: &mut RawMesh) {
    let Some((p, t, n, hw)) = c.sample(s) else {
        return;
    };
    let r = t.cross(n);
    let h = cell * 0.95;
    let lat = hw + 0.42;
    let foot = -0.7;
    // Posts bury into the bar but must not share its planes: identical
    // coplanar faces z-fight where the two boxes overlap.
    let post_top = h - 0.04;
    let post = lin(ARCH_DARK);
    for side in [-1.0, 1.0] {
        push_obox(
            out,
            p + r * (side * lat) + n * ((post_top + foot) * 0.5),
            r,
            t,
            n,
            Vec3::new(0.16, 0.13, (post_top - foot) * 0.5),
            post,
        );
    }
    let bar_h = 0.36;
    match bar {
        Bar::Solid(color) => push_obox(
            out,
            p + n * (h - bar_h * 0.5),
            r,
            t,
            n,
            Vec3::new(hw + 0.62, 0.16, bar_h * 0.5),
            color,
        ),
        Bar::Striped => {
            let cols = 10;
            let full = hw + 0.62;
            let w = 2.0 * full / cols as f32;
            for col in 0..cols {
                let off = -full + w * (col as f32 + 0.5);
                let color = lin(if col % 2 == 0 {
                    CHECK_LIGHT
                } else {
                    CHECK_DARK
                });
                push_obox(
                    out,
                    p + r * off + n * (h - bar_h * 0.5),
                    r,
                    t,
                    n,
                    Vec3::new(w * 0.5, 0.16, bar_h * 0.5),
                    color,
                );
            }
        }
    }
}

use glam::Vec3;
use repame_view3d::MeshGroup;
use retrackt_format::geometry::{Centerline, RawMesh};
use retrackt_format::piece::catalog_by_id;
use retrackt_format::{PieceId, TrackDocument, piece_shape};

use super::{lin, merge, push_lit, push_obox, push_tri_lit, raw_mesh_to_group};

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
    let post = lin(ARCH_DARK);
    for side in [-1.0, 1.0] {
        push_obox(
            out,
            p + r * (side * lat) + n * ((h + foot) * 0.5),
            r,
            t,
            n,
            Vec3::new(0.16, 0.16, (h - foot) * 0.5),
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

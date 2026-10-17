use std::f32::consts::TAU;

use glam::{Quat, Vec3};
use repame_view3d::MeshGroup;
use retrackt_format::geometry::RawMesh;

use crate::sim::car::{Car, body_matrix};

use super::{lin, push_lit, push_obox, push_tri_lit, raw_mesh_to_group};

const GLASS: u32 = 0x1E2733;
const DARK: u32 = 0x23272E;
const UNDER: u32 = 0x1A1D23;
const TIRE: u32 = 0x14161A;
/// Ink for the rim's own details, a shade off the tyre so a spoke separates
/// from it without becoming a highlight.
const DARK_RIM: [f32; 3] = [0.055, 0.06, 0.07];
const HEAD: u32 = 0xEDEFF2;
const TAIL: u32 = 0xC2261C;

/// Ghost albedo: a cool cyan that no track surface or car panel uses, so a
/// translucent body is never mistaken for a piece of the scenery.
const GHOST_TINT: [f32; 3] = [0.16, 0.62, 0.86];
/// Low enough to read through the car's own shell, high enough to read against
/// both bright road and dark sky.
const GHOST_ALPHA: f32 = 0.42;

const RIDE: f32 = 0.55;
const WHEEL_R: f32 = 0.32;
const WHEEL_W: f32 = 0.12;
const TRACK: f32 = 0.80;
const AXLE: f32 = 1.25;

pub(super) fn groups(car: &Car) -> (MeshGroup, MeshGroup, MeshGroup) {
    let body_m = body_matrix(car);

    let mut shell = RawMesh::default();
    bodywork(&mut shell, car.paint, car.trim, car.pattern);
    wheels(&mut shell, car);
    let mut body = raw_mesh_to_group(&shell.transformed(body_m, [1.0; 3]), true);
    body.material.roughness = 0.5;

    let mut head = raw_mesh_to_group(&lamps(true).transformed(body_m, [1.0; 3]), true);
    head.material.emissive = [0.35, 0.34, 0.30];

    let mut tail = raw_mesh_to_group(&lamps(false).transformed(body_m, [1.0; 3]), true);
    tail.material.emissive = [0.45, 0.04, 0.03];

    (body, head, tail)
}

/// The ghost body: the same shell and wheels, drawn translucent and tinted.
/// Lamps are omitted, because a reference lap is not a car you can collide with
/// and a pair of lit headlights reads as a rival on the road.
pub(super) fn ghost_group(car: &Car) -> MeshGroup {
    let mut shell = RawMesh::default();
    bodywork(&mut shell, car.paint, car.trim, car.pattern);
    wheels(&mut shell, car);
    let mut group = raw_mesh_to_group(&shell.transformed(body_matrix(car), [1.0; 3]), true);
    // One flat albedo, not the car's own paint: the shell reads as a solid
    // silhouette rather than a translucent copy, which is what keeps a
    // half-transparent car from looking like a rendering fault.
    group.colors.fill(GHOST_TINT);
    // The blend pipeline disables depth writes (`render.rs:1533`), so the ghost
    // does not occlude its own far side. Depth *testing* stays on, so the track
    // still hides it — a reference visible through the scenery would be no help
    // at judging a line.
    group.transparent = true;
    group.alpha = GHOST_ALPHA;
    group.material.roughness = 1.0;
    group.material.emissive = [0.05, 0.12, 0.20];
    group
}

/// Everything in body space: +Z forward, +Y up, origin at the car centre,
/// which itself rides a height above the road.
fn bodywork(m: &mut RawMesh, paint: u8, trim: u8, pattern: u8) {
    let (hx, y0, y1, zf, zr) = (0.80, -0.45, 0.08, 1.32, -1.30);
    let v = Vec3::new;
    let body = lin(crate::app::theme::car_paint(paint));
    let under = lin(UNDER);
    let glass = lin(GLASS);
    let dark = lin(DARK);
    let stripe = lin(crate::app::theme::car_trim(trim));

    push_lit(
        m,
        [v(-hx, y0, zr), v(hx, y0, zr), v(hx, y0, zf), v(-hx, y0, zf)],
        Vec3::NEG_Y,
        under,
    );
    push_lit(
        m,
        [v(-hx, y1, zr), v(-hx, y1, zf), v(hx, y1, zf), v(hx, y1, zr)],
        Vec3::Y,
        body,
    );
    push_lit(
        m,
        [v(hx, y0, zr), v(hx, y1, zr), v(hx, y1, zf), v(hx, y0, zf)],
        Vec3::X,
        body,
    );
    push_lit(
        m,
        [
            v(-hx, y0, zf),
            v(-hx, y1, zf),
            v(-hx, y1, zr),
            v(-hx, y0, zr),
        ],
        Vec3::NEG_X,
        body,
    );
    push_lit(
        m,
        [v(hx, y0, zf), v(hx, y1, zf), v(-hx, y1, zf), v(-hx, y0, zf)],
        Vec3::Z,
        body,
    );
    push_lit(
        m,
        [v(-hx, y0, zr), v(-hx, y1, zr), v(hx, y1, zr), v(hx, y0, zr)],
        Vec3::NEG_Z,
        body,
    );

    // Cabin: dark glasshouse with a raked windshield, open at the bottom
    // where the deck already closes it.
    let (xc, yc, zb, zt, zbf) = (0.56, 0.46, -0.98, 0.06, 0.42);
    let rake = Vec3::new(0.0, 0.36, 0.38);
    push_lit(
        m,
        [
            v(-xc, y1, zbf),
            v(xc, y1, zbf),
            v(xc, yc, zt),
            v(-xc, yc, zt),
        ],
        rake,
        glass,
    );
    push_lit(
        m,
        [v(-xc, y1, zb), v(-xc, yc, zb), v(xc, yc, zb), v(xc, y1, zb)],
        Vec3::NEG_Z,
        glass,
    );
    push_lit(
        m,
        [v(xc, y1, zb), v(xc, yc, zb), v(xc, yc, zt), v(xc, y1, zbf)],
        Vec3::X,
        glass,
    );
    push_lit(
        m,
        [
            v(-xc, y1, zbf),
            v(-xc, yc, zt),
            v(-xc, yc, zb),
            v(-xc, y1, zb),
        ],
        Vec3::NEG_X,
        glass,
    );
    push_lit(
        m,
        [v(-xc, yc, zb), v(-xc, yc, zt), v(xc, yc, zt), v(xc, yc, zb)],
        Vec3::Y,
        glass,
    );

    // Every livery is the same three bands at a different offset and width, so
    // a livery can only change what the stripes look like: the geometry is
    // shared and only the numbers differ, which is what keeps a new one from
    // being a new set of quads nobody has looked at.
    let band = |m: &mut RawMesh, off: f32, half: f32| {
        let (x0, x1) = (off - half, off + half);
        for (y, z0, z1) in [
            (y1 + 0.01, zbf, zf),
            (yc + 0.01, zb, zt),
            (y1 + 0.01, zr, zb),
        ] {
            push_lit(
                m,
                [v(x0, y, z0), v(x0, y, z1), v(x1, y, z1), v(x1, y, z0)],
                Vec3::Y,
                stripe,
            );
        }
    };
    match pattern {
        // Plain: the body on its own.
        1 => {}
        // Twin: a narrow pair, the classic racing layout.
        2 => {
            band(m, -0.30, 0.055);
            band(m, 0.30, 0.055);
        }
        // Wide: one band across most of the deck.
        3 => band(m, 0.0, 0.30),
        // Centre, and anything a save names that the list no longer has.
        _ => band(m, 0.0, 0.09),
    }

    for side in [-1.0, 1.0] {
        push_obox(
            m,
            v(side * 0.60, 0.22, -1.14),
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            v(0.05, 0.14, 0.06),
            dark,
        );
    }
    push_obox(
        m,
        v(0.0, 0.40, -1.24),
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        v(0.86, 0.04, 0.12),
        dark,
    );
}

/// Four wheels on the sim's probe rectangle. Contact index order is
/// front-left, front-right, rear-left, rear-right; only the front pair
/// steers. Wheel bottom tracks the road exactly: distance below the body
/// is `RIDE - 0.9 * compression`.
fn wheels(m: &mut RawMesh, car: &Car) {
    let droop = if car.grounded { 0.0 } else { 0.08 };
    let tire = lin(TIRE);
    let rim = lin(crate::app::theme::car_trim(car.trim));
    for i in 0..4 {
        let lift = 0.9 * car.compression[i].clamp(0.0, 1.0);
        let y = -RIDE + WHEEL_R + lift - droop;
        let x = if i % 2 == 0 { -TRACK } else { TRACK };
        let z = if i < 2 { AXLE } else { -AXLE };
        let steer = if i < 2 { car.steer_angle } else { 0.0 };
        wheel(m, Vec3::new(x, y, z), steer, tire, rim, car.rims);
    }
}

/// Octagonal tyre: tread ring plus two rim discs, steered about its own centre
/// when `steer` is non-zero.
///
/// Every wheel style keeps the disc solid and adds to it. A style that made the
/// disc smaller or cut pieces out of it would open the wheel's interior to the
/// camera, and the tyre is a band rather than a bowl: there is nothing behind it
/// to see but the far wall, back-facing and culled.
fn wheel(m: &mut RawMesh, centre: Vec3, steer: f32, tire: [f32; 3], rim: [f32; 3], style: u8) {
    const SIDES: usize = 8;
    let rot = Quat::from_rotation_y(steer);
    let at = |p: Vec3| centre + rot * p;
    let n = |v: Vec3| rot * v;
    let w = Vec3::X * WHEEL_W;
    for k in 0..SIDES {
        let a0 = k as f32 / SIDES as f32 * TAU;
        let a1 = (k + 1) as f32 / SIDES as f32 * TAU;
        let p0 = Vec3::new(0.0, a0.cos(), a0.sin()) * WHEEL_R;
        let p1 = Vec3::new(0.0, a1.cos(), a1.sin()) * WHEEL_R;
        let mid = Vec3::new(0.0, ((a0 + a1) * 0.5).cos(), ((a0 + a1) * 0.5).sin());
        push_lit(
            m,
            [at(p0 - w), at(p1 - w), at(p1 + w), at(p0 + w)],
            n(mid),
            tire,
        );
        push_tri_lit(m, [at(w), at(w + p0), at(w + p1)], n(Vec3::X), rim);
        push_tri_lit(m, [at(-w), at(-w + p1), at(-w + p0)], n(Vec3::NEG_X), rim);

        // The style, laid on the disc a hair proud of it. Dark rather than the
        // trim colour so a spoke reads as a gap in a wheel rather than as a
        // lighter stripe painted on one.
        let proud = Vec3::X * 0.006;
        match style {
            1 => {
                // Spokes, every other vertex of the octagon, so five of them.
                if k % 2 == 0 {
                    let tip = (p0 + p1) * 0.42;
                    push_tri_lit(
                        m,
                        [
                            at(w + proud) + Vec3::ZERO,
                            at(w + proud + tip),
                            at(w + proud + tip * 0.72),
                        ],
                        n(Vec3::X),
                        DARK_RIM,
                    );
                    push_tri_lit(
                        m,
                        [
                            at(-w - proud),
                            at(-w - proud + tip * 0.72),
                            at(-w - proud + tip),
                        ],
                        n(Vec3::NEG_X),
                        DARK_RIM,
                    );
                }
            }
            2 => {
                // A ring, as the eight quads of a band between two radii.
                let inner = 0.62f32;
                let outer = 0.86f32;
                push_lit(
                    m,
                    [
                        at(w + proud + p0 * inner),
                        at(w + proud + p0 * outer),
                        at(w + proud + p1 * outer),
                        at(w + proud + p1 * inner),
                    ],
                    n(mid),
                    DARK_RIM,
                );
                push_lit(
                    m,
                    [
                        at(-w - proud + p0 * inner),
                        at(-w - proud + p1 * inner),
                        at(-w - proud + p1 * outer),
                        at(-w - proud + p0 * outer),
                    ],
                    n(mid),
                    DARK_RIM,
                );
            }
            _ => {}
        }
    }
}

fn lamps(front: bool) -> RawMesh {
    let mut m = RawMesh::default();
    let (y0, y1) = (-0.14, -0.02);
    if front {
        let z = 1.326;
        let color = lin(HEAD);
        for (x0, x1) in [(0.32, 0.70), (-0.70, -0.32)] {
            push_lit(
                &mut m,
                [
                    Vec3::new(x1, y0, z),
                    Vec3::new(x1, y1, z),
                    Vec3::new(x0, y1, z),
                    Vec3::new(x0, y0, z),
                ],
                Vec3::Z,
                color,
            );
        }
    } else {
        let z = -1.306;
        let color = lin(TAIL);
        for (x0, x1) in [(0.26, 0.64), (-0.64, -0.26)] {
            push_lit(
                &mut m,
                [
                    Vec3::new(x0, y0, z),
                    Vec3::new(x0, y1, z),
                    Vec3::new(x1, y1, z),
                    Vec3::new(x1, y0, z),
                ],
                Vec3::NEG_Z,
                color,
            );
        }
    }
    m
}

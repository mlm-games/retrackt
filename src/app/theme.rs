use repose_core::{Color, Dp, Sp, UnitExt};

pub fn background() -> Color {
    Color::from_rgb(10, 12, 16)
}

pub fn surface() -> Color {
    Color::from_rgb(24, 28, 36)
}

pub fn accent() -> Color {
    Color::from_rgb(255, 176, 32)
}

pub fn text() -> Color {
    Color::from_rgb(240, 243, 248)
}

pub fn text_dim() -> Color {
    Color::from_rgb(150, 158, 170)
}

pub fn danger() -> Color {
    Color::from_rgb(224, 72, 72)
}

/// Passed a check. Only used where a check passing is news, so it stays
/// distinct from the accent rather than reusing it.
pub fn ok() -> Color {
    Color::from_rgb(96, 208, 128)
}

// The on-screen stick, as layers of translucency rather than as a picture.
//
// Drawn from colours and corner radii so the paint system does the rounding: a
// disc is then exact at any pixel density, where a fixed-size sprite is either
// soft or blocky on a 2x or 3x screen. The stack reads as one control because the
// shadow and the knob rim are both the background colour at two opacities, which
// is what seats the knob inside the base rather than on top of it.

/// Under the base, showing as a rim around it.
pub fn touch_shadow() -> Color {
    background().with_alpha_f32(0.45)
}

/// The base the knob travels inside.
pub fn touch_base() -> Color {
    surface().with_alpha_f32(0.85)
}

/// The well the knob sits in, a shade darker than the base so travel reads.
pub fn touch_well() -> Color {
    background().with_alpha_f32(0.55)
}

/// The circle the knob travels to, visible at rest because it sits outside the
/// knob's own radius.
pub fn touch_ring() -> Color {
    text().with_alpha_f32(0.32)
}

/// The mark at the centre, so a resting stick reads as centred rather than off.
pub fn touch_centre() -> Color {
    text().with_alpha_f32(0.45)
}

/// Under the knob, as its rim.
pub fn touch_knob_rim() -> Color {
    background().with_alpha_f32(0.9)
}

pub fn touch_knob() -> Color {
    accent().with_alpha_f32(0.92)
}

pub fn dp(v: f32) -> Dp {
    v.dp()
}

pub fn sp(v: f32) -> Sp {
    v.sp()
}

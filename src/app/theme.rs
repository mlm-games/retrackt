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

pub fn dp(v: f32) -> Dp {
    v.dp()
}

pub fn sp(v: f32) -> Sp {
    v.sp()
}

use repose_core::{ImageHandle, Modifier, Px, View, px_to_dp};
use repose_ui::{Box, Image, Row, ViewExt, ZStack};

use crate::app::input::{STICK_BASE_DP, STICK_KNOB_DP};
use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::hud::race_hud;
use crate::ui::widgets::{controls_hint, ghost_btn, pusher};

pub fn race_ui(data: &AppData, actions: &ActionQueue, viewport: View) -> View {
    let controls = Box(Modifier::new()
        .absolute()
        .offset(None, None, Some(theme::dp(20.0)), Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(
        Row(Modifier::new().gap(theme::dp(12.0)))
            .child(ghost_btn("Restart", pusher(actions, UiAct::Restart)))
            .child(ghost_btn(
                "Quit to Title",
                pusher(actions, UiAct::RaceAbandoned),
            )),
    );

    let hints = Box(Modifier::new()
        .absolute()
        .offset(Some(theme::dp(20.0)), None, None, Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(controls_hint());

    let mut children = vec![viewport, race_hud(data), hints, controls];
    if let (Some(stick), Some(images)) = (data.stick, data.stick_images) {
        children.push(stick_layer(stick.anchor, images.0, STICK_BASE_DP));
        children.push(stick_layer(
            (stick.anchor.0 + stick.knob.0, stick.anchor.1 + stick.knob.1),
            images.1,
            STICK_KNOB_DP,
        ));
    }
    ZStack(Modifier::new().fill_max_size()).child(children)
}

fn stick_layer(center: (f32, f32), handle: ImageHandle, size_dp: f32) -> View {
    let size = theme::dp(size_dp);
    Image(
        Modifier::new()
            .absolute()
            .offset(
                Some(px_to_dp(Px(center.0)) - size / 2.0),
                Some(px_to_dp(Px(center.1)) - size / 2.0),
                None,
                None,
            )
            .width(size)
            .height(size)
            .hit_passthrough(),
        handle,
    )
}

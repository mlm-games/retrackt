use repose_core::{ImageHandle, Modifier, Px, View, px_to_dp};
use repose_ui::{Box, Column, Image, Row, Text, TextStyle, ViewExt, ZStack};

use crate::app::input::{STICK_BASE_DP, STICK_KNOB_DP};
use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::hud::race_hud;
use crate::ui::widgets::{accent_btn, controls_hint, ghost_btn, pusher};

pub fn race_ui(data: &AppData, actions: &ActionQueue, viewport: View) -> View {
    let mut controls_row = vec![ghost_btn("Restart", pusher(actions, UiAct::Restart))];
    if data.practice {
        controls_row.push(accent_btn(
            "Checkpoint",
            pusher(actions, UiAct::RecoverToCheckpoint),
        ));
    }
    controls_row.push(ghost_btn(
        "Quit to Title",
        pusher(actions, UiAct::RaceAbandoned),
    ));
    let controls = Box(Modifier::new()
        .absolute()
        .offset(None, None, Some(theme::dp(20.0)), Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(Row(Modifier::new().gap(theme::dp(12.0))).child(controls_row));

    let mut hints = controls_hint();
    if data.practice {
        hints = Column(Modifier::new().gap(theme::dp(4.0)))
            .child(hints)
            .child(
                Text("C returns you to the last checkpoint, keeping the clock")
                    .size(theme::sp(14.0))
                    .color(theme::accent())
                    .single_line(),
            );
    }
    let hints = Box(Modifier::new()
        .absolute()
        .offset(Some(theme::dp(20.0)), None, None, Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(hints);

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

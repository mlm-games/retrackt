use repose_core::{Modifier, View};
use repose_ui::{Box, Row, ViewExt, ZStack};

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

    ZStack(Modifier::new().fill_max_size()).child([viewport, race_hud(data), hints, controls])
}

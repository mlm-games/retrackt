pub mod hud;
pub mod library;
pub mod screens;
pub mod thumb;
pub mod widgets;

use repose_core::{Modifier, View};
use repose_ui::{Box, Row, Text, TextStyle, ViewExt, ZStack};

use crate::app::state::{AppData, Screen};
use crate::app::theme;
use crate::ui::widgets::danger_btn;

pub fn root_view(data: &AppData, actions: &crate::app::state::ActionQueue, viewport: View) -> View {
    let base = match data.screen {
        Screen::Title => over_viewport(viewport, screens::title::title_ui(data, actions)),
        Screen::Race => screens::race::race_ui(data, actions, viewport),
        Screen::Results => over_viewport(viewport, screens::results::results_ui(data, actions)),
        Screen::Editor => over_viewport(viewport, screens::editor::editor_ui(data, actions)),
        Screen::Ghosts => over_viewport(viewport, screens::ghosts::ghosts_ui(data, actions)),
    };
    match notice_bar(data, actions) {
        Some(bar) => ZStack(Modifier::new().fill_max_size()).child([base, bar]),
        None => base,
    }
}

fn over_viewport(viewport: View, overlay: View) -> View {
    ZStack(Modifier::new().fill_max_size()).child([viewport, overlay])
}

/// The runtime's last message, or nothing. Drawn over every screen rather than
/// inside one: the runtime sets `notice` for rejections raised from anywhere in
/// the frame — a refused race start, an unreadable tape, a rotated piece whose
/// chain broke — and a message bound to one screen would be invisible exactly
/// when it mattered.
fn notice_bar(data: &AppData, actions: &crate::app::state::ActionQueue) -> Option<View> {
    let text = data.notice.clone()?;
    let dismiss = danger_btn("Dismiss", {
        let queue = actions.clone();
        move || crate::app::state::push(&queue, crate::app::state::UiAct::DismissNotice)
    });
    Some(
        Box(Modifier::new()
            .absolute()
            .offset(None, None, Some(theme::dp(20.0)), Some(theme::dp(20.0)))
            .max_width(theme::dp(520.0))
            .background(theme::background().with_alpha_f32(0.9))
            .border(
                theme::dp(1.0),
                theme::danger().with_alpha_f32(0.6),
                theme::dp(8.0),
            )
            .clip_rounded(theme::dp(8.0)))
        .child(
            Row(Modifier::new().gap(theme::dp(12.0)).padding(theme::dp(10.0)))
                .child(
                    Box(Modifier::new().flex_grow(1.0)).child(
                        Text(text)
                            .size(theme::sp(15.0))
                            .color(theme::text())
                            .single_line(),
                    ),
                )
                .child(dismiss),
        ),
    )
}
pub mod hud;
pub mod library;
pub mod screens;
pub mod widgets;

use repose_core::{Modifier, View};
use repose_ui::{ViewExt, ZStack};

use crate::app::state::Screen;

pub fn root_view(
    data: &crate::app::state::AppData,
    actions: &crate::app::state::ActionQueue,
    viewport: repose_core::View,
) -> repose_core::View {
    match data.screen {
        Screen::Title => over_viewport(viewport, screens::title::title_ui(data, actions)),
        Screen::Race => screens::race::race_ui(data, actions, viewport),
        Screen::Results => over_viewport(viewport, screens::results::results_ui(data, actions)),
        Screen::Editor => over_viewport(viewport, screens::editor::editor_ui(data, actions)),
    }
}

fn over_viewport(viewport: View, overlay: View) -> View {
    ZStack(Modifier::new().fill_max_size()).child([viewport, overlay])
}

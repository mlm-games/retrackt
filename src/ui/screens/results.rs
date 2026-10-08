use repose_core::{Modifier, View};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Column, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::screens::ghosts::ghost_save_row;
use crate::ui::widgets::{
    badge, dim_text, fmt_ticks, ghost_btn, heading, menu_btn, panel, pusher, screen_backdrop,
};

pub fn results_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut children = vec![heading("Race Results")];
    let track = data.track.name.clone();
    children.push(dim_text(&track));

    match &data.last_result {
        Some(result) => {
            let new_record = match data.best {
                Some(best) => result.total_ticks < best,
                None => true,
            };
            if new_record {
                children.push(badge("NEW RECORD"));
            }
            children.push(
                Text(format!("Total  {}", fmt_ticks(result.total_ticks)))
                    .size(theme::sp(24.0))
                    .color(theme::accent())
                    .single_line(),
            );
            if let Some(best) = data.best {
                let best = fmt_ticks(best);
                children.push(dim_text(&format!("Previous best  {best}")));
            }
            if !result.splits.is_empty() {
                children.push(dim_text("Splits"));
                children.push(splits_body(&result.splits));
            }
        }
        None => children.push(dim_text("No result recorded")),
    }

    // Only offered when this run actually recorded a tape, which is the only
    // case where there is anything to keep.
    if data.last_result.as_ref().is_some_and(|r| r.replay.is_some()) {
        children.push(ghost_save_row(data, actions));
    }

    children.push(menu_btn("Restart", pusher(actions, UiAct::Restart)));
    children.push(ghost_btn(
        "Track Editor",
        pusher(actions, UiAct::OpenEditor),
    ));
    children.push(ghost_btn("Ghosts", pusher(actions, UiAct::OpenGhosts)));
    children.push(ghost_btn(
        "Quit to Title",
        pusher(actions, UiAct::QuitToTitle),
    ));

    screen_backdrop(panel("", children))
}

fn splits_body(splits: &[u32]) -> View {
    let rows: Vec<View> = splits
        .iter()
        .enumerate()
        .map(|(index, split)| dim_text(&format!("{}  {}", index + 1, fmt_ticks(*split))))
        .collect();
    if splits.len() > 5 {
        ScrollArea(
            Modifier::new()
                .width(theme::dp(300.0))
                .height(theme::dp(160.0)),
            remember_scroll_state("results.splits"),
            Column(Modifier::new().gap(theme::dp(4.0))).child(rows),
        )
    } else {
        Column(Modifier::new().gap(theme::dp(4.0))).child(rows)
    }
}

use repose_core::{Modifier, View};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Box, Column, Row, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::screens::ghosts::ghost_save_row;
use crate::ui::widgets::{
    badge, dim_text, fmt_delta, fmt_ticks, ghost_btn, heading, menu_btn, panel, practice_toggle,
    pusher, screen_backdrop,
};

pub fn results_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut children = vec![heading("Race Results")];
    let track = data.track.name.clone();
    children.push(dim_text(&track));

    match &data.last_result {
        Some(result) => {
            // A practice run records nothing, so it cannot set one.
            let new_record = !data.practice
                && match data.best {
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
            if let Some(delta) = data.split_delta {
                let label = if delta <= 0 { "AHEAD" } else { "BEHIND" };
                children.push(
                    Text(format!("{label}  {}", fmt_delta(delta)))
                        .size(theme::sp(20.0))
                        .color(if delta <= 0 {
                            theme::ok()
                        } else {
                            theme::danger()
                        })
                        .single_line(),
                );
            }
            if !result.splits.is_empty() {
                children.push(dim_text("Splits"));
                children.push(splits_body(&result.splits, data.ghost_splits.as_deref()));
            }
        }
        None => children.push(dim_text("No result recorded")),
    }

    // A practice run records nothing, so offering to keep its tape would name the
    // one run that is not a record.
    if !data.practice
        && data
            .last_result
            .as_ref()
            .is_some_and(|r| r.replay.is_some())
    {
        children.push(ghost_save_row(data, actions));
    }
    if data.practice {
        children.push(dim_text(
            "Practice: not recorded, and falls return to the last checkpoint",
        ));
    }

    children.push(menu_btn("Restart", pusher(actions, UiAct::Restart)));
    children.push(practice_toggle(data.practice, actions));
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

/// One row per checkpoint: the split, and the gap to the ghost's split there.
///
/// The ghost's own times are not printed beside them: two columns of numbers the
/// player has to subtract is the work the delta line already did.
fn splits_body(splits: &[u32], theirs: Option<&[u32]>) -> View {
    let rows: Vec<View> = splits
        .iter()
        .enumerate()
        .map(|(index, split)| {
            let delta = theirs.and_then(|theirs| theirs.get(index)).map(|t| {
                let d = i64::from(*split) - i64::from(*t);
                Text(fmt_delta(d))
                    .size(theme::sp(14.0))
                    .color(if d <= 0 { theme::ok() } else { theme::danger() })
                    .single_line()
            });
            Row(Modifier::new().gap(theme::dp(12.0)))
                .child(
                    Box(Modifier::new().width(theme::dp(28.0))).child(
                        Text(format!("{}", index + 1))
                            .size(theme::sp(14.0))
                            .color(theme::text())
                            .single_line(),
                    ),
                )
                .child(
                    Box(Modifier::new().width(theme::dp(130.0))).child(
                        Text(fmt_ticks(*split))
                            .size(theme::sp(14.0))
                            .color(theme::text())
                            .single_line(),
                    ),
                )
                .child(match delta {
                    Some(d) => d,
                    None => dim_text(""),
                })
        })
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

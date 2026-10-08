use std::cell::RefCell;
use std::rc::Rc;

// `TextFieldLineLimits` and `TextStyle` come from repose-core (the config value
// structs); the `TextStyle` trait that styles `Text` views is repose-ui's, and
// the two share a name. Both imports are needed and neither shadows the other.
use repose_core::{AlignItems, Modifier, TextFieldLineLimits, View, remember_with_key};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{
    BasicTextField, Box, Column, FlowRow, FlowRowConfig, Row, Text, TextFieldConfig,
    TextFieldState, TextStyle, ViewExt,
};

use crate::app::state::{ActionQueue, AppData, UiAct, push};
use crate::app::theme;
use crate::ui::widgets::{
    danger_btn, dim_text, fmt_ticks, ghost_btn, heading, hud_text, menu_btn, panel, pusher,
    screen_backdrop,
};

/// The kept-tape library: every tape the player has kept, across all tracks.
pub fn ghosts_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut children = vec![heading("Ghosts")];

    if data.ghosts.is_empty() {
        children.push(dim_text(
            "No ghosts yet. Every record is kept as one automatically, and a run can \
             be saved by name from the results screen.",
        ));
    } else {
        children.push(dim_text(&format!("{} kept", data.ghosts.len())));
        let mut rows = Vec::with_capacity(data.ghosts.len());
        for entry in &data.ghosts {
            let playable = entry.track == retrackt_format::gameplay_fingerprint(&data.track);
            let label = format!("{}  ·  {}", entry.name, fmt_ticks(entry.ticks));

            let remove = danger_btn("Delete", {
                let queue = actions.clone();
                let file = entry.file.clone();
                move || push(&queue, UiAct::DeleteGhost(file.clone()))
            });
            // Racing is offered only for the tape's own track: `race_ghost` refuses
            // anything else, and a button that can only error is worse than none.
            // Watching is gated the same way and for the same reason — playback
            // re-simulates the run against this track's geometry.
            let actions_row = if playable {
                Row(Modifier::new().gap(theme::dp(8.0)))
                    .child(ghost_btn(
                        "Race",
                        pusher(actions, UiAct::RaceGhost(entry.file.clone())),
                    ))
                    .child(ghost_btn(
                        "Watch",
                        pusher(actions, UiAct::WatchGhost(entry.file.clone())),
                    ))
                    .child(remove)
            } else {
                Row(Modifier::new().gap(theme::dp(8.0))).child(remove)
            };

            rows.push(
                Row(Modifier::new().gap(theme::dp(12.0)).align_items(AlignItems::CENTER))
                    .child(
                        Box(Modifier::new().width(theme::dp(260.0))).child(
                            Text(label)
                                .size(theme::sp(16.0))
                                .color(theme::text())
                                .single_line(),
                        ),
                    )
                    .child(actions_row),
            );
            if !playable {
                rows.push(dim_text("  recorded on another track"));
            }
        }
        children.push(ScrollArea(
            Modifier::new()
                .width(theme::dp(560.0))
                .height(theme::dp(300.0)),
            remember_scroll_state("ghosts.list"),
            Column(Modifier::new().gap(theme::dp(8.0))).child(rows),
        ));
    }

    children.push(menu_btn("Close", pusher(actions, UiAct::CloseGhosts)));

    screen_backdrop(panel("", children))
}

/// Name field and save button on the results screen. Its own function, and its
/// own keyed slot, so composing it cannot disturb the slots the rest of the UI
/// holds: a text field in the middle of a conditional subtree would otherwise
/// renumber everything after it.
pub fn ghost_save_row(data: &AppData, actions: &ActionQueue) -> View {
    let state: Rc<RefCell<TextFieldState>> = remember_with_key("ghosts.name", {
        || RefCell::new(TextFieldState::new())
    });
    // The field is the source of truth while it has focus; the draft on
    // `AppData` is what survives leaving the screen.
    if state.borrow().text != data.ghost_draft {
        state.borrow_mut().text = data.ghost_draft.clone();
    }

    let field = BasicTextField(
        state.clone(),
        Modifier::new().width(theme::dp(220.0)),
        "Ghost name",
        TextFieldConfig {
            line_limits: TextFieldLineLimits::SingleLine,
            on_change: Some(Rc::new({
                let queue = actions.clone();
                move |text: String| push(&queue, UiAct::SetGhostDraft(text))
            }) as Rc<dyn Fn(String)>),
            ..TextFieldConfig::default()
        },
    );

    // Read at press time, not captured here: the closure is built once per
    // composition, so a name typed afterwards would never reach it.
    let save = {
        let queue = actions.clone();
        let field_state = state;
        move || {
            let name = field_state.borrow().text.trim().to_string();
            push(&queue, UiAct::SaveGhost(name))
        }
    };

    FlowRow(
        Modifier::new().gap(theme::dp(10.0)).align_items(AlignItems::CENTER),
        FlowRowConfig::default(),
    )
    .child([
        hud_text("Keep this run as a ghost"),
        field,
        ghost_btn("Save Ghost", save),
    ])
}
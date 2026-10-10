use std::cell::RefCell;
use std::rc::Rc;

// `TextFieldLineLimits` and `TextStyle` come from repose-core (the config value
// structs); the `TextStyle` trait that styles `Text` views is repose-ui's, and
// the two share a name. Both imports are needed and neither shadows the other.
use repose_core::{AlignItems, Modifier, TextFieldLineLimits, View, remember_with_key};
use repose_ui::{
    BasicTextField, Box, Column, FlowRow, FlowRowConfig, Row, Text, TextFieldConfig,
    TextFieldState, TextStyle, ViewExt,
};

use crate::app::state::{ActionQueue, AppData, UiAct, push};
use crate::app::theme;
use crate::ui::fit;
use crate::ui::widgets::{
    danger_btn, dim_text, fmt_ticks, ghost_btn, heading, hud_text, list_panel, menu_btn, pusher,
    screen_backdrop,
};

/// The kept-tape library: every tape the player has kept, across all tracks.
pub fn ghosts_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut children = vec![heading("Ghosts")];

    if data.ghosts.is_empty() {
        children.push(
            Text(
                "No ghosts yet. Every record is kept as one automatically, and a run can \
                 be saved by name from the results screen.",
            )
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .max_lines(4),
        );
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
            let action_count = if playable { 3 } else { 1 };

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
                // Wrapping, because the name and three buttons together are wider
                // than the list is: a plain row would run the last button off the
                // edge with no way to reach it.
                FlowRow(
                    Modifier::new()
                        .gap(theme::dp(12.0))
                        .align_items(AlignItems::CENTER),
                    FlowRowConfig::default(),
                )
                .child(
                    Box(Modifier::new().width(theme::dp(fit::label_width(action_count)))).child(
                        Text(label)
                            .size(theme::sp(16.0))
                            .color(theme::text())
                            .single_line()
                            .overflow_ellipsize(),
                    ),
                )
                .child(actions_row),
            );
            if !playable {
                rows.push(dim_text("  recorded on another track"));
            }
        }
        // No scroller of its own. The panel this list sits in already scrolls its
        // body, and a second one under the same pointer takes the wheel from it.
        //
        // Its old XY scroller is the part that matters: an XY scroller leaves its
        // content at its own width so it can grow sideways, which gave a row of a
        // name and three buttons nothing to wrap against. It came out 718 dp wide
        // inside a 520 dp panel, and the last button needed a horizontal scrollbar to
        // reach. The panel's own scroller bounds the width to the panel, so the row
        // wraps instead of breaking out of it.
        children.push(Column(Modifier::new().gap(theme::dp(8.0)).fill_max_width()).child(rows));
    }

    children.push(menu_btn("Close", pusher(actions, UiAct::CloseGhosts)));

    screen_backdrop(list_panel("", children))
}

/// Name field and save button on the results screen. Its own function, and its
/// own keyed slot, so composing it cannot disturb the slots the rest of the UI
/// holds: a text field in the middle of a conditional subtree would otherwise
/// renumber everything after it.
pub fn ghost_save_row(data: &AppData, actions: &ActionQueue) -> View {
    let state: Rc<RefCell<TextFieldState>> =
        remember_with_key("ghosts.name", || RefCell::new(TextFieldState::new()));
    // The field is the source of truth while it has focus; the draft on
    // `AppData` is what survives leaving the screen.
    if state.borrow().text != data.ghost_draft {
        state.borrow_mut().text = data.ghost_draft.clone();
    }

    let field = BasicTextField(
        state.clone(),
        Modifier::new().fill_max_width(),
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

    // The caption takes its own line and the field shares one with the button, rather
    // than all three sharing one: a caption, a fixed-width field and a button are wider
    // together than the panel is on a phone, so they wrapped to three lines and left the
    // button below the panel's own height, where scrolling was the only way to reach it.
    // The field takes the width that is left instead of asking for a fixed share of it,
    // so the row and the column both fill the panel: the results panel centres its
    // children, which leaves a row sized to its own content with nothing for the field's
    // share to grow into. It came out at the width of its own hint with most of the
    // panel beside it unused.
    Column(Modifier::new().gap(theme::dp(10.0)).fill_max_width())
        .child(hud_text("Keep this run as a ghost"))
        .child(
            Row(Modifier::new()
                .gap(theme::dp(10.0))
                .align_items(AlignItems::CENTER)
                .fill_max_width())
            .child(Box(Modifier::new().weight(1.0)).child(field))
            .child(ghost_btn("Save Ghost", save)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fit::tests::WINDOW;
    use repose_core::{
        SceneNode, calculate_window_size_class, set_window_container_size,
        set_window_size_class_default,
    };
    use repose_ui::layout_and_paint;
    use std::collections::HashMap;

    /// A library long enough that the panel has to scroll to reach its own Close
    /// button.
    ///
    /// The list used to have a scroller of its own, nested inside the panel's. That
    /// nested XY scroller left its content at its own width so it could grow
    /// sideways, which gave the rows nothing to wrap against: a row of a name and
    /// three buttons came out 718 dp wide inside a 520 dp panel, and the last button
    /// needed a horizontal scrollbar to reach. Two entries fit without the panel ever
    /// scrolling, so the scroll path is the part that was never exercised.
    #[test]
    fn a_long_library_wraps_its_rows_and_still_scrolls() {
        let sizes = [
            (1600.0f32, 1000.0f32),
            (1280.0, 800.0),
            (1024.0, 600.0),
            (360.0, 780.0),
        ];
        for (w, h) in sizes {
            let _turn = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            set_window_container_size(w, h);
            set_window_size_class_default(calculate_window_size_class(w as u32, h as u32, 1.0));

            let mut data = AppData::default();
            let fp = retrackt_format::gameplay_fingerprint(&data.track);
            data.ghosts = (0..14)
                .map(|i| crate::save::GhostEntry {
                    name: format!("Aurora Sprint {i}"),
                    file: format!("g{i}.ghost"),
                    track: fp.clone(),
                    physics: retrackt_format::fingerprint::TrackFingerprint::default(),
                    ticks: 90_000 + i as u32 * 1_000,
                })
                .collect();

            let (scene, hits, _) = layout_and_paint(
                &ghosts_ui(&data, &Rc::new(std::cell::RefCell::new(Vec::new()))),
                (w as u32, h as u32),
                &HashMap::new(),
                &Default::default(),
                None,
            );

            // The panel is the widest bordered box, and it scrolls its own body, so
            // the list has to be reachable by scrolling rather than by overflowing.
            let panel = scene
                .nodes
                .iter()
                .filter_map(|n| match n {
                    SceneNode::Border { rect, .. } if rect.w > 200.0 => Some(*rect),
                    _ => None,
                })
                .max_by_key(|r| (r.h * r.w) as i64)
                .expect("a panel");
            let inner = (panel.x + 20.0, panel.x + panel.w - 20.0);

            let scrolls = hits.iter().any(|r| {
                r.on_scroll.is_some()
                    && r.rect.x >= panel.x - 1.0
                    && r.rect.x + r.rect.w <= panel.x + panel.w + 1.0
            });
            assert!(
                scrolls,
                "at {w:.0}x{h:.0} the panel does not scroll, so a long library \
                 cannot reach its own Close button"
            );

            // Every row's controls stay inside the panel, and the row wraps rather
            // than growing sideways out of it.
            for n in &scene.nodes {
                if let SceneNode::Text { rect, text, .. } = n {
                    if text.as_ref() == "Race" || text.as_ref() == "Delete" {
                        assert!(
                            rect.x >= inner.0 - 0.5 && rect.x + rect.w <= inner.1 + 0.5,
                            "at {w:.0}x{h:.0} {text:?} runs from {:.0} to {:.0}, \
                             outside the panel's {:.0}..{:.0}",
                            rect.x,
                            rect.x + rect.w,
                            inner.0,
                            inner.1
                        );
                    }
                }
            }

            // The last row is below the fold rather than beside the first: a library
            // this size has to need scrolling to reach the end of it.
            let folded = scene.nodes.iter().any(|n| match n {
                SceneNode::Text { rect, .. } if rect.y > panel.y + panel.h => true,
                _ => false,
            });
            assert!(
                folded,
                "at {w:.0}x{h:.0} every row fits, so the panel's scroller was never \
                 exercised by this test"
            );
        }
    }
}

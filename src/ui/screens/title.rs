use repose_core::{AlignItems, JustifyContent, Modifier, View};
use repose_ui::scroll::{ScrollAreaXY, remember_scroll_state_xy};
use repose_ui::{Box, Column, FlowRow, FlowRowConfig, Row, Spacer, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, TrackRef, UiAct};
use crate::app::theme;
use crate::ui::fit;
use crate::ui::thumb;
use crate::ui::widgets::{
    CONTROLS, PANEL_INSET_DP, chip_btn, dim_text, fmt_ticks, ghost_btn, menu_btn, panel_w, pusher,
};

/// Edge of a library thumbnail, in dp.
const THUMB_DP: f32 = 56.0;

/// Width of the wordmark-and-actions column, in dp. The menu reads as a console
/// because its controls share one bounded measure; left to the window they ran edge
/// to edge and read as three unrelated bars.
const MENU_DP: f32 = 340.0;
/// Width of the controls panel's key column, in dp. Sized for the longest entry in
/// `CONTROLS` plus room to breathe: at 120 the key column clipped `RB / LB`.
const KEY_DP: f32 = 150.0;

pub fn title_ui(data: &AppData, actions: &ActionQueue) -> View {
    let builtins = retrackt_format::builtin_tracks();
    let thumbs = data.editor.thumbnails;

    // Side by side when there is room for both, stacked when there is not: the
    // library and the controls panel together want about 780, which is more than a
    // phone held upright has. On a narrow or a short window the library takes the
    // whole row and the controls sit below it.
    let stacked = fit::is_narrow() || fit::is_short();
    let library_width = if stacked {
        fit::panel_width(460.0)
    } else {
        fit::fit(420.0, fit::EDGE_DP * 2.0 + 340.0)
    };
    // The name column is what the library has left once its thumbnail, count and
    // Load button are placed. Derived from the panel rather than chosen beside it:
    // at a fixed 240 the row wanted 476 of a 380 dp box and Load wrapped onto a
    // line of its own under every track.
    let name_dp = (library_width - PANEL_INSET_DP.0 * 2.0 - THUMB_DP - 130.0).max(80.0);

    let mut tracks: Vec<View> = crate::ui::library::with_saved(|saved| {
        let mut rows: Vec<View> = Vec::new();
        let mut any = false;
        for track in &builtins {
            // A saved track of the same name stands in for the built-in here. The
            // name is the only handle on either copy, so listing both would give
            // one label two meanings, and loading the player's own track is the
            // one they mean. The built-in is still listed, and loaded, by the
            // editor.
            if saved.iter().any(|doc| doc.name == track.name) {
                continue;
            }
            any = true;
            rows.push(track_row(
                thumb::thumbnail(track, THUMB_DP),
                track,
                TrackRef::Builtin(track.name.clone()),
                actions,
                thumbs,
                name_dp,
            ));
        }

        rows.push(dim_text("Your tracks"));
        if saved.is_empty() {
            rows.push(dim_text("No saved tracks yet"));
        }
        for doc in saved {
            any = true;
            rows.push(track_row(
                thumb::thumbnail(doc, THUMB_DP),
                doc,
                TrackRef::Saved(doc.name.clone()),
                actions,
                thumbs,
                name_dp,
            ));
        }
        if !any {
            rows.push(dim_text("No tracks available"));
        }
        rows
    });

    // No scroller of its own: the whole screen is one scroll view, and a list that
    // scrolled inside a panel inside it would take the wheel and keep it.
    let library = panel_w(
        "Tracks",
        theme::dp(library_width),
        vec![Column(Modifier::new().gap(theme::dp(6.0))).child(tracks)],
    );

    let controls = controls_panel(stacked);

    // One inset for the whole screen, and it lives in the fit rather than as a
    // padding on the body: a padding narrows the content box, while every width on
    // this screen is fitted against the window, so the two disagree and the children
    // overhang. The card's own padding sits inside its width for the same reason.
    let card_dp = fit::panel_width(MENU_DP + 40.0);
    let menu_w = card_dp - 40.0;

    // The track line sits inside the menu column rather than spanning the window:
    // it is about the current selection, and at full width it was the one element
    // glued to the left edge, aligned with nothing.
    let track_line = FlowRow(
        Modifier::new()
            .gap(theme::dp(10.0))
            .align_items(AlignItems::CENTER)
            // Centred per line, so a track name long enough to wrap does not leave
            // its own best time stranded at the left edge.
            .justify_content(JustifyContent::CENTER)
            .width(theme::dp(menu_w)),
        FlowRowConfig::default(),
    )
    .child(dim_text(&format!("Track: {}", data.track.name)))
    .child(match data.best {
        Some(best) => Text(format!("Best  {}", fmt_ticks(best)))
            .size(theme::sp(14.0))
            .color(theme::accent())
            .single_line(),
        None => dim_text("No best time yet"),
    });

    // Hierarchy, top to bottom: the wordmark, one primary destination, two
    // secondary ones, then the preferences — which are preferences, so they take
    // quiet chips on one line rather than a slot each in the action stack.
    //
    // On a backing card rather than straight onto the scene. The outlined controls
    // were being read against bright grass, and a card is what makes the column read
    // as one console rather than six controls floating over a track.
    let menu = Box(Modifier::new()
        .padding(theme::dp(20.0))
        .width(theme::dp(card_dp))
        .background(theme::surface().with_alpha_f32(0.72))
        .border(
            theme::dp(1.0),
            theme::text_dim().with_alpha_f32(0.28),
            theme::dp(14.0),
        )
        .clip_rounded(theme::dp(14.0)))
    .child(
        Column(
            Modifier::new()
                .gap(theme::dp(10.0))
                .align_items(AlignItems::CENTER)
                .width(theme::dp(menu_w)),
        )
        .child(
            Text("RETRACKT")
                .size(theme::sp(theme::WORDMARK_SP * fit::type_scale()))
                .font_family(theme::FONT_DISPLAY)
                .color(theme::accent())
                .single_line(),
        )
        .child(
            Text("time-trial time attack")
                .size(theme::sp(13.0))
                .color(theme::text_dim())
                .single_line(),
        )
        .child(Box(Modifier::new().height(theme::dp(10.0))).child(Spacer()))
        .child(menu_btn("Start Race", pusher(actions, UiAct::StartRace)))
        .child(ghost_btn(
            "Track Editor",
            pusher(actions, UiAct::OpenEditor),
        ))
        .child(ghost_btn("Ghosts", pusher(actions, UiAct::OpenGhosts)))
        .child(
            Row(Modifier::new()
                .gap(theme::dp(8.0))
                .align_items(AlignItems::CENTER))
            .child(practice_toggle_chip(data.practice, actions))
            .child(thumbnail_toggle(thumbs, actions)),
        )
        .child(Box(Modifier::new().height(theme::dp(6.0))).child(Spacer()))
        .child(track_line),
    );

    let panels = if stacked {
        Column(Modifier::new().gap(theme::dp(14.0))).child([library, controls])
    } else {
        Row(Modifier::new()
            .gap(theme::dp(16.0))
            .align_items(AlignItems::START))
        .child([library, controls])
    };

    let body = Column(
        Modifier::new()
            .gap(theme::dp(28.0))
            .align_items(AlignItems::CENTER)
            .fill_max_width()
            // A margin, not a padding: every width here is fitted against the
            // window, and a padding would narrow the content box out from under them.
            // A margin leaves the column's own box alone and only pushes the card off
            // the top and bottom edges, which is the breathing room it had lost when
            // the inset moved into `card_dp`.
            .margin_vertical(theme::dp(fit::EDGE_DP)),
    )
    .child([menu, panels]);

    // One scroller for the screen, so a short window can reach the bottom of the
    // menu rather than losing it. The backdrop centres and blocks input, but it is
    // not a scroll view, so without this the last buttons are simply off-screen.
    crate::ui::widgets::screen_backdrop(ScrollAreaXY(
        Modifier::new().fill_max_size(),
        remember_scroll_state_xy("title.root"),
        body,
    ))
}

/// Thumbnails cost a view per occupied cell, so the player who wants a compact
/// list turns them off. Persisted, because it is a preference rather than a mode.
fn thumbnail_toggle(thumbs: bool, actions: &ActionQueue) -> View {
    chip_btn(
        if thumbs {
            "Hide thumbnails"
        } else {
            "Show thumbnails"
        },
        pusher(actions, UiAct::SetThumbnails(!thumbs)),
    )
}

fn practice_toggle_chip(practice: bool, actions: &ActionQueue) -> View {
    chip_btn(
        if practice {
            "Practice: on"
        } else {
            "Practice: off"
        },
        pusher(actions, UiAct::SetPractice(!practice)),
    )
}

/// One library row: a thumbnail when one can be drawn, the name, a count, and
/// the button that loads it.
fn track_row(
    thumbnail: Option<View>,
    doc: &retrackt_format::TrackDocument,
    which: TrackRef,
    actions: &ActionQueue,
    thumbs: bool,
    name_dp: f32,
) -> View {
    let click = pusher(actions, UiAct::LoadTrack(which));
    let name = Box(Modifier::new().width(theme::dp(name_dp))).child(
        Text(doc.name.clone())
            .size(theme::sp(16.0))
            .color(theme::text())
            .single_line()
            .overflow_ellipsize(),
    );
    // Wrapping rather than a plain row: thumbnail, name, summary and button are
    // wider together than the library is, so a non-wrapping row would clip the
    // button off the right edge.
    let mut row = FlowRow(
        Modifier::new()
            .gap(theme::dp(10.0))
            .align_items(AlignItems::CENTER),
        FlowRowConfig::default(),
    );
    if thumbs {
        // A track with no geometry has nothing to draw, and an empty frame beside
        // its name would read as a rendering fault rather than as "no road yet".
        row = row.child(match thumbnail {
            Some(t) => t,
            None => Box(Modifier::new().width(theme::dp(THUMB_DP))),
        });
    }
    row.child(name)
        .child(dim_text(&thumb::summary(doc)))
        .child(chip_btn("Load", click))
}

fn controls_panel(stacked: bool) -> View {
    // The key column is as wide as the widest key and no wider: on a narrow window
    // every dp spent here is a dp the action does not have, and `restart /
    // checkpoint / quit` is the row that runs out first.
    let key_dp = if fit::is_narrow() { 104.0 } else { KEY_DP };
    // Wide enough for the key, the gap, the longest action and a little slack. The
    // slack is not padding: the two columns are fixed-width boxes in a row, and
    // sized to the sum exactly they lose the rounding and the key clips first.
    let panel_dp = if stacked {
        fit::panel_width(key_dp + 260.0)
    } else {
        fit::fit(key_dp + 260.0, fit::EDGE_DP * 2.0 + 460.0)
    };
    // Bound for the action text to ellipsize inside. Without a width of its own it
    // takes its full natural measure and hangs past the panel, which is how the
    // longest entry pushed the row 4 dp off a 320 dp screen.
    let action_dp = (panel_dp - PANEL_INSET_DP.0 * 2.0 - key_dp - 12.0).max(48.0);

    let mut rows: Vec<View> = Vec::new();
    for (key, action) in CONTROLS {
        rows.push(
            Row(Modifier::new().gap(theme::dp(12.0)))
                .child(
                    Box(Modifier::new().width(theme::dp(key_dp))).child(
                        Text(key)
                            .size(theme::sp(14.0))
                            .font_family(theme::FONT_DISPLAY)
                            .color(theme::accent())
                            .single_line()
                            .overflow_ellipsize(),
                    ),
                )
                .child(
                    Box(Modifier::new().width(theme::dp(action_dp))).child(
                        Text(action)
                            .size(theme::sp(14.0))
                            .color(theme::text_dim())
                            .single_line()
                            .overflow_ellipsize(),
                    ),
                ),
        );
    }
    panel_w(
        "Controls",
        theme::dp(panel_dp),
        vec![Column(Modifier::new().gap(theme::dp(10.0))).child(rows)],
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use repose_core::{
        SceneNode, calculate_window_size_class, set_window_container_size,
        set_window_size_class_default,
    };

    use super::*;
    use crate::ui::fit;

    /// Lays the title out at this window and reports what the test needs: the
    /// rightmost edge anything reaches, and each panel title's position.
    fn title_at(dp_w: f32, dp_h: f32) -> (f32, Vec<(String, f32)>) {
        let _turn = fit::tests::WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let view = title_ui(
            &AppData::default(),
            &Rc::new(RefCell::new(Vec::new())) as &ActionQueue,
        );
        let (scene, hits, _) = repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        assert_eq!(
            hits.iter().filter(|r| r.on_scroll.is_some()).count(),
            1,
            "{dp_w}x{dp_h}: more than one scroller, so one of them takes the wheel \
             and keeps it"
        );
        let right = scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Rect { rect, .. }
                | SceneNode::Border { rect, .. }
                | SceneNode::Text { rect, .. } => Some(rect.x + rect.w),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        let titles = ["Tracks", "Controls"]
            .iter()
            .filter_map(|want| {
                let y = scene.nodes.iter().find_map(|n| match n {
                    SceneNode::Text { rect, text, .. } if text.as_ref() == *want => Some(rect.y),
                    _ => None,
                });
                y.map(|y| ((*want).to_string(), y))
            })
            .collect();
        (right, titles)
    }

    #[test]
    fn nothing_on_the_title_screen_falls_off_the_side() {
        // The track line is a wrapping row, but a percentage width resolves to the
        // content width once an ancestor is itself sized to its content, which
        // unwrapped it again and pushed 13 dp past a 320 dp screen.
        for (w, h) in [
            (320.0f32, 568.0f32),
            (360.0, 780.0),
            (800.0, 360.0),
            (1024.0, 576.0),
            (1920.0, 1200.0),
        ] {
            let (right, _) = title_at(w, h);
            assert!(right <= w + 1.0, "{w}x{h}: content reaches {right:.0} dp");
        }
    }

    #[test]
    fn the_library_and_the_controls_share_a_row_only_when_there_is_room() {
        // The two panels were always in a column: the `stacked` flag chose their
        // sizes and nothing else, so the comment describing a row described an
        // arrangement the screen never took.
        for (w, h, side_by_side) in [
            (1024.0f32, 576.0f32, true),
            (1920.0, 1200.0, true),
            (800.0, 360.0, false),
            (360.0, 780.0, false),
        ] {
            let (_, titles) = title_at(w, h);
            let y = |name: &str| titles.iter().find(|t| t.0 == name).map(|t| t.1);
            let tracks = y("Tracks").unwrap_or_else(|| panic!("{w}x{h}: no library panel"));
            let controls = y("Controls").unwrap_or_else(|| panic!("{w}x{h}: no controls panel"));
            assert_eq!(
                (tracks - controls).abs() < 2.0,
                side_by_side,
                "{w}x{h}: library at y={tracks} and controls at y={controls}"
            );
        }
    }
}

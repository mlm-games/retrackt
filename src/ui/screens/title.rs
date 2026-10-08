use repose_core::{AlignItems, Modifier, View};
use repose_ui::scroll::{ScrollAreaXY, remember_scroll_state_xy};
use repose_ui::{Box, Column, FlowRow, FlowRowConfig, Row, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, TrackRef, UiAct};
use crate::app::theme;
use crate::ui::fit;
use crate::ui::thumb;
use crate::ui::widgets::{
    CONTROLS, dim_text, fmt_ticks, ghost_btn, menu_btn, panel_w, practice_toggle, pusher,
};

/// Edge of a library thumbnail, in dp.
const THUMB_DP: f32 = 56.0;

pub fn title_ui(data: &AppData, actions: &ActionQueue) -> View {
    let builtins = retrackt_format::builtin_tracks();
    let thumbs = data.editor.thumbnails;

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
            ));
        }
        if !any {
            rows.push(dim_text("No tracks available"));
        }
        rows
    });

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
    // No scroller of its own: the whole screen is one scroll view, and a list that
    // scrolled inside a panel inside it would take the wheel and keep it.
    let library = panel_w(
        "Tracks",
        theme::dp(library_width),
        vec![Column(Modifier::new().gap(theme::dp(6.0))).child(tracks)],
    );

    // Wrapping, because the track's name is player-supplied and unbounded: a plain
    // row pushed 13 dp past the right edge of a 320 dp screen, taking the rest of
    // the menu with it.
    let mut info = FlowRow(
        Modifier::new()
            .gap(theme::dp(12.0))
            .align_items(AlignItems::CENTER)
            // An explicit width, not `fill_max_width`: a percentage resolves to
            // the content width once an ancestor in the chain is itself sized to
            // its content, and a wrapping row bounded by its own content is a
            // single line again. The window is the bound that actually holds.
            .width(theme::dp(fit::width())),
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
    if data.track.pieces.is_empty() {
        info = info.child(dim_text("no pieces placed"));
    }

    let controls = controls_panel(stacked);

    let header = Column(
        Modifier::new()
            .gap(theme::dp(14.0))
            .align_items(AlignItems::CENTER)
            .fill_max_width(),
    )
    .child(
        Text("RETRACKT")
            .size(theme::sp(56.0 * fit::type_scale()))
            .color(theme::accent())
            .single_line(),
    )
    .child(dim_text("time-trial time attack"))
    .child(info)
    .child(menu_btn("Start Race", pusher(actions, UiAct::StartRace)))
    .child(practice_toggle(data.practice, actions))
    .child(menu_btn("Track Editor", pusher(actions, UiAct::OpenEditor)))
    .child(menu_btn("Ghosts", pusher(actions, UiAct::OpenGhosts)))
    .child(thumbnail_toggle(thumbs, actions));

    // The two panels are siblings, so side by side they are a row and stacked they
    // are a column. They were always a column: the widths were derived from a
    // `stacked` flag that chose panel sizes and nothing else, so the comment above
    // described an arrangement the screen never took.
    // `fill_max_width` all the way down, because a column sized to its content has no
    // width to wrap against: the track line below is a wrapping row, and a wrapping
    // row with no bound is a single line again.
    let body = if stacked {
        Column(Modifier::new().gap(theme::dp(14.0)).fill_max_width())
            .child([header, library, controls])
    } else {
        Column(Modifier::new().gap(theme::dp(16.0)).fill_max_width()).child([
            header,
            Row(Modifier::new()
                .gap(theme::dp(16.0))
                .align_items(AlignItems::START)
                .fill_max_width())
            .child([library, controls]),
        ])
    };

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
    ghost_btn(
        if thumbs {
            "Hide thumbnails"
        } else {
            "Show thumbnails"
        },
        pusher(actions, UiAct::SetThumbnails(!thumbs)),
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
) -> View {
    let click = pusher(actions, UiAct::LoadTrack(which));
    let name = Box(Modifier::new().width(theme::dp(fit::label_width(1)))).child(
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
        .child(ghost_btn("Load", click))
}

fn controls_panel(stacked: bool) -> View {
    let mut rows: Vec<View> = Vec::new();
    for (key, action) in CONTROLS {
        rows.push(
            Row(Modifier::new().gap(theme::dp(10.0)))
                .child(
                    Box(Modifier::new().width(theme::dp(120.0))).child(
                        Text(key)
                            .size(theme::sp(14.0))
                            .color(theme::accent())
                            .single_line(),
                    ),
                )
                .child(
                    Text(action)
                        .size(theme::sp(14.0))
                        .color(theme::text_dim())
                        .single_line(),
                ),
        );
    }
    panel_w(
        "Controls",
        theme::dp(if stacked {
            fit::panel_width(300.0)
        } else {
            fit::fit(300.0, fit::EDGE_DP * 2.0 + 460.0)
        }),
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

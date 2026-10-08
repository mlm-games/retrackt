use repose_core::{AlignItems, Modifier, View};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Box, Column, Row, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, TrackRef, UiAct};
use crate::app::theme;
use crate::ui::thumb;
use crate::ui::widgets::{
    CONTROLS, dim_text, fmt_ticks, ghost_btn, menu_btn, panel, practice_toggle, pusher,
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

    let library = panel(
        "Tracks",
        vec![ScrollArea(
            Modifier::new()
                .width(theme::dp(420.0))
                .height(theme::dp(240.0)),
            remember_scroll_state("title.library"),
            Column(Modifier::new().gap(theme::dp(6.0))).child(tracks),
        )],
    );

    let mut info = Row(Modifier::new().gap(theme::dp(12.0)))
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

    let columns = Row(Modifier::new()
        .gap(theme::dp(24.0))
        .align_items(AlignItems::START))
    .child(library)
    .child(controls_panel());

    let menu = Column(
        Modifier::new()
            .gap(theme::dp(14.0))
            .align_items(AlignItems::CENTER),
    )
    .child(
        Text("RETRACKT")
            .size(theme::sp(56.0))
            .color(theme::accent())
            .single_line(),
    )
    .child(dim_text("time-trial time attack"))
    .child(info)
    .child(menu_btn("Start Race", pusher(actions, UiAct::StartRace)))
    .child(practice_toggle(data.practice, actions))
    .child(menu_btn("Track Editor", pusher(actions, UiAct::OpenEditor)))
    .child(menu_btn("Ghosts", pusher(actions, UiAct::OpenGhosts)))
    .child(thumbnail_toggle(thumbs, actions))
    .child(columns);

    crate::ui::widgets::screen_backdrop(menu)
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
    let name = Box(Modifier::new().width(theme::dp(240.0))).child(
        Text(doc.name.clone())
            .size(theme::sp(16.0))
            .color(theme::text())
            .single_line(),
    );
    let mut row = Row(Modifier::new().gap(theme::dp(10.0)).align_items(AlignItems::CENTER));
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

fn controls_panel() -> View {
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
    panel(
        "Controls",
        vec![ScrollArea(
            Modifier::new()
                .width(theme::dp(300.0))
                .height(theme::dp(200.0)),
            remember_scroll_state("title.controls"),
            Column(Modifier::new().gap(theme::dp(10.0))).child(rows),
        )],
    )
}

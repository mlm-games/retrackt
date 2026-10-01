use repose_core::{AlignItems, Modifier, View};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Box, Column, Row, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::widgets::{
    CONTROLS, dim_text, fmt_time, ghost_btn, menu_btn, panel, pusher, screen_backdrop,
};

pub fn title_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut tracks: Vec<View> = Vec::new();
    let builtins = retrackt_format::builtin_tracks();
    let saved = crate::ui::library::saved_tracks();

    if builtins.is_empty() && saved.is_empty() {
        tracks.push(dim_text("No tracks available"));
    }
    for track in builtins {
        let label = track.name.clone();
        let click = pusher(actions, UiAct::LoadTrack(track.name));
        tracks.push(ghost_btn(&label, click));
    }
    tracks.push(dim_text("Your tracks"));
    if saved.is_empty() {
        tracks.push(dim_text("No saved tracks yet"));
    }
    for name in saved {
        let label = name.clone();
        let click = pusher(actions, UiAct::LoadTrack(name));
        tracks.push(ghost_btn(&label, click));
    }

    let library = panel(
        "Tracks",
        vec![ScrollArea(
            Modifier::new()
                .width(theme::dp(300.0))
                .height(theme::dp(200.0)),
            remember_scroll_state("title.library"),
            Column(Modifier::new().gap(theme::dp(6.0))).child(tracks),
        )],
    );

    let mut info = Row(Modifier::new().gap(theme::dp(12.0)))
        .child(dim_text(&format!("Track: {}", data.track.name)))
        .child(match data.best {
            Some(best) => Text(format!("Best  {}", fmt_time(best)))
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
    .child(menu_btn("Track Editor", pusher(actions, UiAct::OpenEditor)))
    .child(columns);

    screen_backdrop(menu)
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
    panel("Controls", rows)
}

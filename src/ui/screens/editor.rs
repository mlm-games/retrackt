use repose_core::{AlignItems, Alignment, Modifier, PaddingValues, View};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Box, Column, FlowRow, FlowRowConfig, Row, Text, TextStyle, ViewExt};
use retrackt_format::catalog;

use crate::app::state::{ActionQueue, AppData, TrackRef, UiAct, push};
use crate::app::theme;
use crate::ui::widgets::{
    danger_btn, dim_text, ghost_btn, heading, hud_text, menu_btn, panel, pusher, screen_backdrop,
};

pub fn editor_ui(data: &AppData, actions: &ActionQueue) -> View {
    let palette = panel("Palette", vec![palette_list(actions)]);
    let build = panel(
        &format!("Track · {}", data.track.name),
        build_children(data, actions),
    );

    let columns = Row(Modifier::new()
        .gap(theme::dp(20.0))
        .align_items(AlignItems::START))
    .child(palette)
    .child(build);

    let content = Column(
        Modifier::new()
            .gap(theme::dp(14.0))
            .padding_values(PaddingValues {
                top: theme::dp(12.0),
                ..PaddingValues::default()
            })
            .align_items(AlignItems::CENTER),
    )
    .child(heading("Track Editor"))
    .child(columns)
    .child(ghost_btn(
        "Close Editor",
        pusher(actions, UiAct::CloseEditor),
    ));

    screen_backdrop(ScrollArea(
        Modifier::new().fill_max_size(),
        remember_scroll_state("editor.screen"),
        Box(Modifier::new()
            .fill_max_width()
            .content_alignment_safe(Alignment::Center))
        .child(content),
    ))
}

fn palette_list(actions: &ActionQueue) -> View {
    let mut children: Vec<View> = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    for def in catalog() {
        let group = def.id.group();
        if groups.contains(&group) {
            continue;
        }
        groups.push(group);
        children.push(dim_text(group));
        for piece in catalog().iter().filter(|d| d.id.group() == group) {
            children.push(ghost_btn(
                piece.id.label(),
                pusher(actions, UiAct::PlacePiece(piece.id)),
            ));
        }
    }

    ScrollArea(
        Modifier::new()
            .width(theme::dp(230.0))
            .height(theme::dp(420.0)),
        remember_scroll_state("editor.palette"),
        Column(Modifier::new().gap(theme::dp(6.0))).child(children),
    )
}

fn build_children(data: &AppData, actions: &ActionQueue) -> Vec<View> {
    let mut children: Vec<View> = Vec::new();

    let count = data.track.pieces.len();
    children.push(dim_text(&format!("{count} pieces")));
    if crate::ui::library::persisted(&data.track) {
        children.push(
            Text("Saved to library")
                .size(theme::sp(14.0))
                .color(theme::accent())
                .single_line(),
        );
    } else {
        children.push(dim_text("Unsaved changes"));
    }

    let rows: Vec<View> = if data.track.pieces.is_empty() {
        vec![dim_text("No pieces placed")]
    } else {
        data.track
            .pieces
            .iter()
            .enumerate()
            .map(|(index, piece)| {
                let uid = piece.uid.0;
                hud_text(&format!(
                    "{}. {} [{} {} {}] #{uid}",
                    index + 1,
                    piece.id.label(),
                    piece.cell[0],
                    piece.cell[1],
                    piece.cell[2],
                ))
            })
            .collect()
    };
    children.push(ScrollArea(
        Modifier::new()
            .width(theme::dp(420.0))
            .height(theme::dp(200.0)),
        remember_scroll_state("editor.track"),
        Column(Modifier::new().gap(theme::dp(4.0))).child(rows),
    ));

    children.push(
        Row(Modifier::new().gap(theme::dp(10.0)))
            .child(ghost_btn(
                "Remove Last",
                pusher(actions, UiAct::RemoveLastPiece),
            ))
            .child(danger_btn(
                "Clear Track",
                pusher(actions, UiAct::ClearTrack),
            )),
    );

    let mut loaders: Vec<View> = Vec::new();
    for track in retrackt_format::builtin_tracks() {
        let label = format!("Load {}", track.name);
        let click = pusher(actions, UiAct::LoadTrack(TrackRef::Builtin(track.name)));
        loaders.push(ghost_btn(&label, click));
    }
    children.push(
        FlowRow(
            Modifier::new()
                .gap(theme::dp(10.0))
                .max_width(theme::dp(560.0)),
            FlowRowConfig::default(),
        )
        .child(save_btn(actions))
        .child(loaders),
    );

    children.push(menu_btn("Playtest", pusher(actions, UiAct::Playtest)));

    children
}

fn save_btn(actions: &ActionQueue) -> View {
    let queue = actions.clone();
    ghost_btn("Save", move || {
        crate::ui::library::invalidate();
        push(&queue, UiAct::SaveTrack);
    })
}

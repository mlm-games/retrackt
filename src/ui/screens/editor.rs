use std::cell::RefCell;
use std::rc::Rc;

// `TextFieldLineLimits` and `TextStyle` come from repose-core (the config value
// structs); the `TextStyle` trait that styles `Text` views is repose-ui's, and
// the two share a name. Both imports are needed and neither shadows the other.
use repose_core::{
    AlignItems, Modifier, TextFieldLineLimits, View, remember_with_key,
};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{
    BasicTextField, Box, Column, FlowRow, FlowRowConfig, Row, Text, TextFieldConfig,
    TextFieldState, TextStyle, ViewExt,
};
use retrackt_format::{PieceUid, catalog};

use crate::app::state::{ActionQueue, AppData, EditorData, TrackRef, UiAct, push};
use crate::app::theme;
use crate::app::ParamKind;
use crate::ui::widgets::{
    accent_btn, danger_btn, dim_text, disabled_btn, ghost_btn, heading, hud_text, menu_btn, panel,
    pusher,
};

/// Cursor steps for the six directional buttons.
const STEPS: [(&str, [i16; 3]); 6] = [
    ("-X", [-1, 0, 0]),
    ("+X", [1, 0, 0]),
    ("-Z", [0, 0, -1]),
    ("+Z", [0, 0, 1]),
    ("-Y", [0, -1, 0]),
    ("+Y", [0, 1, 0]),
];

/// The editor, laid out as two panels beside a live viewport.
///
/// Deliberately *not* `screen_backdrop`: that marks its subtree as an input
/// blocker, and the whole editor is about pointing at the track — dragging to
/// orbit, clicking the ground to move the cursor. A full-screen blocker over the
/// viewport would make every one of those gestures land on the panel instead. The
/// panels claim input over themselves; the gap between them passes it through.
pub fn editor_ui(data: &AppData, actions: &ActionQueue) -> View {
    let editor = &data.editor;

    let palette = panel("Palette", vec![palette_list(editor.armed, actions)]);
    let track = panel(
        &format!("Track · {}", data.track.name),
        build_children(data, actions),
    );

    let side_buttons = FlowRow(
        Modifier::new().gap(theme::dp(10.0)),
        FlowRowConfig::default(),
    )
    .child(menu_btn("Playtest", pusher(actions, UiAct::Playtest)))
    .child(ghost_btn(
        "Close Editor",
        pusher(actions, UiAct::CloseEditor),
    ))
    .child(heading("Track Editor"));

    // The row claims no input itself, so the empty middle falls through to the
    // viewport beneath. Each panel marks itself a blocker, which is what keeps a
    // drag that started over a button from orbiting the camera.
    Row(Modifier::new().fill_max_size().align_items(AlignItems::START)).child([
        Box(Modifier::new().input_blocker()).child(palette),
        Box(Modifier::new().flex_grow(1.0)),
        Box(Modifier::new().input_blocker()).child(ScrollArea(
            Modifier::new()
                .width(theme::dp(660.0))
                .height(theme::dp(700.0)),
            remember_scroll_state("editor.side"),
            Column(Modifier::new().gap(theme::dp(14.0)))
                .child([track, diagnostics_panel(editor), share_panel(editor, actions)])
                .child(side_buttons),
        )),
    ])
}

/// Palette rows. Clicking *arms* a piece rather than placing it, so the preview
/// can be inspected first; placement is its own button.
fn palette_list(armed: retrackt_format::PieceId, actions: &ActionQueue) -> View {
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
            let on_click = pusher(actions, UiAct::ArmPiece(piece.id));
            children.push(if piece.id == armed {
                accent_btn(piece.id.label(), on_click)
            } else {
                ghost_btn(piece.id.label(), on_click)
            });
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

/// One row per placed piece: its race position, identity and cell, then selection
/// and the edits that apply to it.
fn piece_rows(data: &AppData, actions: &ActionQueue) -> Vec<View> {
    if data.track.pieces.is_empty() {
        return vec![dim_text("No pieces placed")];
    }
    // Race position, so the list matches what a car meets rather than the order
    // the file happens to store.
    let order = data.track.race_order();
    let rank = |uid: PieceUid| {
        order
            .iter()
            .position(|u| *u == uid)
            .map(|i| i + 1)
            .unwrap_or(0)
    };

    data.track
        .pieces
        .iter()
        .map(|piece| {
            let uid = piece.uid;
            let selected = data.editor.is_selected(uid);
            let select = pusher(actions, UiAct::SelectPiece(uid));
            let toggle = pusher(actions, UiAct::ToggleSelect(uid));
            let (len, rad, bank) =
                retrackt_format::piece::resolve_params(piece.id, &piece.params);

            let nudge = |label: &'static str, kind: ParamKind, delta: i8| {
                let queue = actions.clone();
                ghost_btn(label, move || push(&queue, UiAct::AdjustParam(uid, kind, delta)))
            };

            // Radius means nothing for a piece the catalogue gives none, so the
            // control is withheld rather than offered and silently discarded.
            let radius = if rad == 0 {
                vec![dim_text("rad —")]
            } else {
                vec![
                    dim_text(&format!("rad {rad}")),
                    nudge("R-", ParamKind::Radius, -1),
                    nudge("R+", ParamKind::Radius, 1),
                ]
            };

            let head = Row(Modifier::new().gap(theme::dp(8.0)).align_items(AlignItems::CENTER))
                .child(
                    Box(Modifier::new().width(theme::dp(30.0))).child(
                        Text(format!("{}.", rank(uid)))
                            .size(theme::sp(14.0))
                            .color(theme::text_dim())
                            .single_line(),
                    ),
                )
                .child(hud_text(&format!(
                    "{} [{} {} {}] yaw {}",
                    piece.id.label(),
                    piece.cell[0],
                    piece.cell[1],
                    piece.cell[2],
                    piece.yaw,
                )))
                .child(if selected {
                    accent_btn("Sel", select)
                } else {
                    ghost_btn("Sel", select)
                })
                .child(ghost_btn("+Sel", toggle))
                .child(ghost_btn("Rotate", pusher(actions, UiAct::RotatePiece(uid))))
                .child(danger_btn("Remove", pusher(actions, UiAct::DeletePiece(uid))));

            Column(Modifier::new().gap(theme::dp(4.0))).child([
                head,
                FlowRow(
                    Modifier::new().gap(theme::dp(6.0)),
                    FlowRowConfig::default(),
                )
                .child(
                    [
                        dim_text(&format!("len {len}")),
                        nudge("L-", ParamKind::Length, -1),
                        nudge("L+", ParamKind::Length, 1),
                    ]
                    .into_iter()
                    .chain(radius)
                    .chain([
                        dim_text(&format!("bank {bank}°")),
                        nudge("B-", ParamKind::Bank, -5),
                        nudge("B+", ParamKind::Bank, 5),
                    ])
                    .collect::<Vec<_>>(),
                ),
            ])
        })
        .collect()
}

fn build_children(data: &AppData, actions: &ActionQueue) -> Vec<View> {
    let editor = &data.editor;
    let mut children: Vec<View> = Vec::new();
    let count = data.track.pieces.len();
    children.push(dim_text(&format!(
        "{count} pieces · {} selected",
        editor.selection.len()
    )));
    children.push(dim_text(if crate::ui::library::persisted(&data.track) {
        "Saved to library"
    } else {
        "Unsaved changes"
    }));

    children.push(cursor_row(editor, actions));

    children.push(ScrollArea(
        Modifier::new()
            .width(theme::dp(620.0))
            .height(theme::dp(200.0)),
        remember_scroll_state("editor.track"),
        Column(Modifier::new().gap(theme::dp(4.0))).child(piece_rows(data, actions)),
    ));

    children.push(FlowRow(
        Modifier::new()
            .gap(theme::dp(10.0))
            .max_width(theme::dp(640.0)),
        FlowRowConfig::default(),
    )
    .child(accent_btn("Place", pusher(actions, UiAct::PlaceArmed)))
    .child(history_btn("Undo", editor.can_undo, actions, UiAct::Undo))
    .child(history_btn("Redo", editor.can_redo, actions, UiAct::Redo))
    .child(ghost_btn("Select All", pusher(actions, UiAct::SelectAll)))
    .child(ghost_btn("Select Route", pusher(actions, UiAct::SelectRoute)))
    .child(ghost_btn("Clear Sel", pusher(actions, UiAct::SelectNone))));

    children.push(FlowRow(
        Modifier::new()
            .gap(theme::dp(10.0))
            .max_width(theme::dp(640.0)),
        FlowRowConfig::default(),
    )
    .child(ghost_btn(
        "Move to Cursor",
        pusher(actions, UiAct::MoveSelectionToCursor),
    ))
    .child(ghost_btn(
        "Rotate",
        pusher(actions, UiAct::RotateSelection(1)),
    ))
    .child(ghost_btn(
        "Rotate Back",
        pusher(actions, UiAct::RotateSelection(-1)),
    ))
    .child(ghost_btn("Copy", pusher(actions, UiAct::CopySelection)))
    .child(ghost_btn("Paste", pusher(actions, UiAct::PasteAtCursor)))
    .child(ghost_btn(
        "Duplicate",
        pusher(actions, UiAct::DuplicateSelection),
    ))
    .child(danger_btn(
        "Delete",
        pusher(actions, UiAct::DeleteSelection),
    )));

    // Retuning a whole selection is the point of selecting: lengthening every
    // straight on a circuit one button at a time is not an edit anyone makes.
    children.push(
        FlowRow(
            Modifier::new()
                .gap(theme::dp(6.0))
                .max_width(theme::dp(640.0)),
            FlowRowConfig::default(),
        )
        .child(dim_text("Selection"))
        .child(ghost_btn("len-", pusher(actions, UiAct::AdjustSelection(ParamKind::Length, -1))))
        .child(ghost_btn("len+", pusher(actions, UiAct::AdjustSelection(ParamKind::Length, 1))))
        .child(ghost_btn("rad-", pusher(actions, UiAct::AdjustSelection(ParamKind::Radius, -1))))
        .child(ghost_btn("rad+", pusher(actions, UiAct::AdjustSelection(ParamKind::Radius, 1))))
        .child(ghost_btn("bank-", pusher(actions, UiAct::AdjustSelection(ParamKind::Bank, -5))))
        .child(ghost_btn("bank+", pusher(actions, UiAct::AdjustSelection(ParamKind::Bank, 5)))),
    );

    children.push(dim_text(&format!(
        "Clipboard: {} piece{}",
        editor.clipboard.len(),
        if editor.clipboard.len() == 1 { "" } else { "s" }
    )));

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
                .max_width(theme::dp(640.0)),
            FlowRowConfig::default(),
        )
        .child(ghost_btn("Save", pusher(actions, UiAct::SaveTrack)))
        .child(ghost_btn(
            "Remove Last",
            pusher(actions, UiAct::RemoveLastPiece),
        ))
        .child(danger_btn(
            "Clear Track",
            pusher(actions, UiAct::ClearTrack),
        ))
        .child(loaders),
    );

    children
}

/// Cursor cell, its six directions, and whether the next placement connects.
fn cursor_row(editor: &EditorData, actions: &ActionQueue) -> View {
    let mut steps: Vec<View> = Vec::new();
    for (label, delta) in STEPS {
        steps.push(ghost_btn(
            label,
            pusher(actions, UiAct::MoveCursor(delta)),
        ));
    }
    Column(Modifier::new().gap(theme::dp(6.0)))
        .child(hud_text(&format!(
            "Cursor [{} {} {}]{}",
            editor.cursor[0],
            editor.cursor[1],
            editor.cursor[2],
            if editor.snapped {
                "  ·  will connect"
            } else {
                ""
            }
        )))
        .child(FlowRow(
            Modifier::new().gap(theme::dp(6.0)),
            FlowRowConfig::default(),
        )
        .child(steps))
}

/// Undo and redo. Disabled rather than hidden: a live button that does nothing is
/// the confusing case, and the player still needs to see the feature exists.
fn history_btn(
    label: &'static str,
    enabled: bool,
    actions: &ActionQueue,
    act: UiAct,
) -> View {
    if enabled {
        ghost_btn(label, pusher(actions, act))
    } else {
        disabled_btn(label)
    }
}

/// The track's problems, errors first. Shown always rather than only on failure
/// so a broken chain is visible while it is being made, not after a refused race.
fn diagnostics_panel(editor: &EditorData) -> View {
    if editor.diagnostics.is_empty() {
        return panel(
            "Checks",
            vec![Text("No problems found")
                .size(theme::sp(15.0))
                .color(theme::ok())
                .single_line()],
        );
    }
    let rows: Vec<View> = editor
        .diagnostics
        .iter()
        .map(|d| {
            let color = if d.severity == retrackt_format::Severity::Error {
                theme::danger()
            } else {
                theme::accent()
            };
            Text(d.message.clone())
                .size(theme::sp(15.0))
                .color(color)
                .single_line()
        })
        .collect();
    panel("Checks", rows)
}

/// Share-code export and import.
fn share_panel(editor: &EditorData, actions: &ActionQueue) -> View {
    let draft: Rc<RefCell<TextFieldState>> = remember_with_key("editor.code", {
        || RefCell::new(TextFieldState::new())
    });
    if draft.borrow().text != editor.code_draft {
        draft.borrow_mut().text = editor.code_draft.clone();
    }
    let field = BasicTextField(
        draft.clone(),
        Modifier::new().width(theme::dp(520.0)),
        "Paste a share code",
        TextFieldConfig {
            line_limits: TextFieldLineLimits::SingleLine,
            on_change: Some(Rc::new({
                let queue = actions.clone();
                move |text: String| push(&queue, UiAct::SetCodeDraft(text))
            }) as Rc<dyn Fn(String)>),
            ..TextFieldConfig::default()
        },
    );

    let mut children = vec![
        Row(Modifier::new().gap(theme::dp(10.0)).align_items(AlignItems::CENTER))
            .child(field)
            .child(ghost_btn("Load Code", pusher(actions, UiAct::LoadShareCode)))
            .child(accent_btn("Copy Code", pusher(actions, UiAct::CopyShareCode))),
    ];
    if !editor.code_out.is_empty() {
        // Also shown on screen: a platform with no working clipboard still leaves
        // the player able to read the code out and paste it somewhere.
        children.push(dim_text(&format!("Code: {}", editor.code_out)));
    }
    panel("Share", children)
}

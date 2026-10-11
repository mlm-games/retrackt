use std::cell::RefCell;
use std::rc::Rc;

// `TextFieldLineLimits` and `TextStyle` come from repose-core (the config value
// structs); the `TextStyle` trait that styles `Text` views is repose-ui's, and
// the two share a name. Both imports are needed and neither shadows the other.
use repose_core::{
    AlignItems, Dp, Modifier, PaddingValues, TextFieldLineLimits, View, remember_with_key,
};
use repose_material::material3::{ButtonConfig, ButtonDefaults};
use repose_ui::scroll::{ScrollAreaXY, remember_scroll_state_xy};
use repose_ui::{
    BasicTextField, Box, Center, Column, FlowRow, FlowRowConfig, Row, Text, TextFieldConfig,
    TextFieldState, TextStyle, ViewExt, ZStack,
};
use retrackt_format::{PieceUid, catalog};

use crate::app::ParamKind;
use crate::app::state::{ActionQueue, AppData, EditorData, TrackRef, UiAct, push};
use crate::app::theme;
use crate::ui::dims;
use crate::ui::widgets::{
    PANEL_INSET_DP, dim_text, heading, hud_text, list_panel_w, outlined_at, pusher,
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

/// Geometry every editor control shares, so the four variants differ in colour
/// only. The height is the 44 dp touch minimum rather than the material default
/// of 40, and the padding is the library's text-button padding rather than its
/// 24 dp button padding: the editor's labels run three to fourteen characters,
/// and 48 dp of padding around a four-character label is what stopped four
/// controls sharing a line at any window size.
fn ctrl_config() -> ButtonConfig {
    ButtonConfig {
        height: dims::CTRL_H,
        shape_radius: dims::CTRL_R,
        content_padding: Some(ButtonDefaults::TEXT_CONTENT_PADDING),
        ..ButtonConfig::default()
    }
}

/// A control in a row of controls, floored at the width the panel's content
/// width allows.
fn ctrl(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined_at(label, dims::ctrl_w(), ctrl_config(), on_click)
}

/// The emphasised variant: the armed piece, the place button.
fn ctrl_on(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined_at(
        label,
        dims::ctrl_w(),
        ButtonConfig {
            container_color: Some(theme::accent()),
            content_color: Some(theme::background()),
            ..ctrl_config()
        },
        on_click,
    )
}

fn ctrl_danger(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined_at(
        label,
        dims::ctrl_w(),
        ButtonConfig {
            content_color: Some(theme::danger()),
            border: Some((theme::dp(1.0), theme::danger(), dims::CTRL_R)),
            ..ctrl_config()
        },
        on_click,
    )
}

/// A control that is present but not live, for undo and redo with nothing to
/// step. Visible rather than hidden so the feature is discoverable, and inert
/// rather than silent so the player learns why.
fn ctrl_off(label: &str) -> View {
    outlined_at(
        label,
        dims::ctrl_w(),
        ButtonConfig {
            enabled: false,
            content_color: Some(theme::text_dim()),
            ..ctrl_config()
        },
        || {},
    )
}

/// A row of controls, allowed to wrap.
///
/// Centred rather than stretched, because the rows mix controls with text of other
/// heights — a group name, a rank, a heading — and stretching leaves the short one
/// sitting on the row's baseline rather than in the middle of it. Balanced rather
/// than packed greedily, so a row of six resolves as three and three instead of
/// five and one: greedy packing is what makes a wrapped row read as an accident
/// rather than a decision, and every row here is a cloud of equal-shaped controls.
fn ctrl_row(children: Vec<View>) -> View {
    FlowRow(
        Modifier::new()
            .gap(dims::SPACE_ROW)
            .max_width(dims::content_w())
            .align_items(AlignItems::CENTER),
        FlowRowConfig {
            balanced: true,
            ..FlowRowConfig::default()
        },
    )
    .child(children)
}

/// The editor, laid out as two panels beside a live viewport.
///
/// Deliberately *not* `screen_backdrop`: that marks its subtree as an input
/// blocker, and the whole editor is about pointing at the track — dragging to
/// orbit, clicking the ground to move the cursor. A full-screen blocker over the
/// viewport would make every one of those gestures land on the panel instead. The
/// panels claim input over themselves; the gap between them passes it through.
pub fn editor_ui(data: &AppData, actions: &ActionQueue) -> View {
    let editor = &data.editor;

    // Three panes fit side by side on a desktop and do not fit on a phone held
    // upright: the palette and the side column together want about 900, which is
    // more than twice the width of a compact screen. Narrow, so they stack in one
    // column and the whole thing scrolls, which is the only arrangement in which
    // the viewport underneath is still reachable by dragging it.
    let stacked = dims::stacked();
    // Side by side the palette sits in its own column beside the viewport, so it
    // needs its own scroller to reach pieces below the fold. Stacked it is inside
    // the panel column's scroller, and a nested one would take the wheel from it.
    let palette_body = palette_list(editor.armed, actions, stacked);
    let palette = list_panel_w(
        "Palette",
        dims::palette_w(),
        vec![if stacked {
            palette_body
        } else {
            ScrollAreaXY(
                // Side by side the palette is a *sibling* of the column's scroller
                // rather than inside it, so nothing above it bounds its height. An
                // explicit one is what keeps its list scrolling instead of running
                // the panel past the bottom of the window, and it fills that height
                // rather than asking for a comfortable 520 of it, which left a third
                // of a tall window with a dead margin under the last piece.
                Modifier::new().fill_max_width().fill_max_height(),
                remember_scroll_state_xy("editor.palette"),
                palette_body,
            )
        }],
    );
    let track = list_panel_w(
        &format!("Track · {}", data.track.name),
        dims::panel_w(),
        build_children(data, actions),
    );

    // The same inset the panels above it take, so this row starts where their content
    // starts rather than at the column's own edge: a control 20 dp to the left of
    // every other control in the column reads as a different column.
    let side_buttons = Box(Modifier::new().padding_values(PaddingValues {
        left: PANEL_INSET_DP,
        right: PANEL_INSET_DP,
        top: Dp::ZERO,
        bottom: Dp::ZERO,
    }))
    .child(ctrl_row(vec![
        outlined_at(
            "Playtest",
            dims::ctrl_w(),
            ButtonConfig {
                container_color: Some(theme::accent()),
                content_color: Some(theme::background()),
                ..ctrl_config()
            },
            pusher(actions, UiAct::Playtest),
        ),
        ctrl("Close Editor", pusher(actions, UiAct::CloseEditor)),
        heading("Track Editor"),
    ]));

    let track_column = Column(Modifier::new().gap(dims::SPACE_SECTION)).child([
        track,
        diagnostics_panel(editor),
        share_panel(editor, actions),
        side_buttons,
    ]);

    // The tools a touch player has no key for, over the bottom-left corner of the
    // track: the height being edited and the quarter turn to give the selection.
    let dock = editor_dock(data, actions);

    // The row claims no input itself, so the empty middle falls through to the
    // viewport beneath. Each panel marks itself a blocker, which is what keeps a
    // drag that started over a button from orbiting the camera.
    if stacked {
        // One scroller, over the panels only. Not the whole screen: a scroller that
        // fills the window has the wheel over its entire area, so the track behind
        // it can never be zoomed and only becomes reachable by scrolling past the
        // end of the panels. The strip below is left to the viewport, which is
        // where orbit, pan and the wheel-zoom live.
        //
        // The palette leads, because without it there is nothing to place: arming a
        // piece is the first thing the editor is for, and it was in the tree only
        // in the side-by-side branch, so on a handset the editor could not place
        // anything at all.
        let side_column =
            Column(Modifier::new().gap(dims::SPACE_SECTION)).child([palette, track_column]);
        // The dock is stacked above the strip rather than inside the column: the
        // strip is scrollable, and a tool that scrolled away with the panels is a
        // tool that has to be scrolled back to.
        ZStack(Modifier::new().fill_max_size()).child([
            Column(Modifier::new().fill_max_size()).child([
                Box(Modifier::new().input_blocker().weight(1.0)).child(ScrollAreaXY(
                    Modifier::new().fill_max_size(),
                    remember_scroll_state_xy("editor.stack"),
                    side_column,
                )),
                Box(Modifier::new().fill_max_height_frac(dims::viewport_frac())),
            ]),
            dock,
        ])
    } else {
        let (palette_w, side_w) = dims::split().expect("not stacked means a split");
        ZStack(Modifier::new().fill_max_size()).child([
            Row(Modifier::new().fill_max_size()).child([
                Box(Modifier::new()
                    .input_blocker()
                    .width(palette_w)
                    .fill_max_height())
                .child(palette),
                Box(Modifier::new().flex_grow(1.0)),
                // `fill_max_height` on the panel box, not just on the scroller: the row
                // does not stretch its children, so without it the scroller takes its
                // height from the content and grows past the bottom of the window. A
                // scroller taller than the window has nothing to scroll, the viewport
                // is the whole of it, and it swallows the wheel for that whole height.
                Box(Modifier::new()
                    .input_blocker()
                    .width(side_w)
                    .fill_max_height())
                .child(ScrollAreaXY(
                    Modifier::new().fill_max_size(),
                    remember_scroll_state_xy("editor.side"),
                    track_column,
                )),
            ]),
            dock,
        ])
    }
}

/// The hand tools, in the bottom-left corner over the track.
///
/// A touch player has no key for any of these, and they are the two things a
/// placement is made of: how high up, and which way round. Drawn over the track
/// because that is where the piece they are acting on is.
fn editor_dock(data: &AppData, actions: &ActionQueue) -> View {
    let editor = &data.editor;
    let inset = crate::ui::fit::chrome_inset();
    Box(Modifier::new()
        .absolute()
        .offset(Some(dims::dock_left()), None, None, Some(theme::dp(inset)))
        .background(theme::surface().with_alpha_f32(0.92))
        .border(
            theme::dp(1.0),
            theme::text_dim().with_alpha_f32(0.4),
            theme::dp(12.0),
        )
        .clip_rounded(theme::dp(12.0))
        // Claims the pointers over itself, so a tap aimed at a tool cannot also
        // drop a piece under it.
        .input_blocker())
    .child(
        Row(Modifier::new()
            .gap(dims::SPACE_ROW)
            .align_items(AlignItems::CENTER)
            .padding(dims::SPACE_ROW))
        .child([
            Column(Modifier::new().gap(theme::dp(2.0))).child([
                Text("Height")
                    .size(theme::sp(11.0))
                    .color(theme::text_dim())
                    .single_line(),
                Text(format!("{}", editor.cursor[1]))
                    .size(theme::sp(18.0))
                    .color(theme::text())
                    .font_family(theme::FONT_DISPLAY)
                    .single_line(),
            ]),
            tool_tile("-", actions, UiAct::MoveCursor([0, -1, 0])),
            tool_tile("+", actions, UiAct::MoveCursor([0, 1, 0])),
            tool_tile("Rotate", actions, UiAct::RotateSelection(1)),
        ]),
    )
}

/// One square tool in the dock: a keyboard shortcut made reachable by touch.
fn tool_tile(label: &str, actions: &ActionQueue, act: UiAct) -> View {
    outlined_at(
        label,
        dims::tool_w(),
        ButtonConfig {
            height: dims::tool_w(),
            shape_radius: dims::CTRL_R,
            content_padding: Some(ButtonDefaults::TEXT_CONTENT_PADDING),
            ..ButtonConfig::default()
        },
        pusher(actions, act),
    )
}

/// Palette rows. Clicking *arms* a piece rather than placing it, so the preview
/// can be inspected first; placement is its own gesture.
///
/// Tiles of a drawing and a short name rather than a button per piece: fifteen
/// full-width buttons is a wall of text to read through, and the shape of a piece
/// is the thing that says what it is. Arming is all a tile ever does, so the tile
/// carries no more than that.
fn palette_list(armed: retrackt_format::PieceId, actions: &ActionQueue, stacked: bool) -> View {
    let mut tiles: Vec<View> = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    for def in catalog() {
        let group = def.id.group();
        if groups.contains(&group) {
            continue;
        }
        groups.push(group);
        tiles.push(group_header(group));
        let row = FlowRow(
            Modifier::new().gap(dims::SPACE_ROW).fill_max_width(),
            FlowRowConfig {
                balanced: true,
                ..FlowRowConfig::default()
            },
        )
        .child(
            catalog()
                .iter()
                .filter(|d| d.id.group() == group)
                .map(|piece| {
                    piece_tile(
                        piece.id,
                        piece.id.label(),
                        piece.id == armed,
                        actions,
                        UiAct::ArmPiece(piece.id),
                    )
                })
                .collect::<Vec<_>>(),
        );
        tiles.push(row);
    }

    if stacked {
        // Stacked, the palette has the full width of the column, so the groups
        // flow across it instead of down a narrow column that would put the last
        // group of them below the fold.
        return FlowRow(
            Modifier::new().gap(dims::SPACE_ROW),
            FlowRowConfig {
                balanced: true,
                ..FlowRowConfig::default()
            },
        )
        .child(tiles);
    }

    // No scroller: the palette panel is already inside the column's scroller in the
    // stacked layout, and in the side-by-side layout it is a sibling of that
    // scroller rather than inside it. Either way a third scroller under the same
    // pointer only takes the wheel away from the one that matters.
    Column(Modifier::new().gap(dims::SPACE_ROW)).child(tiles)
}

/// One palette tile: the piece's footprint, its short name, and the accent fill
/// while it is the armed piece.
fn piece_tile(
    id: retrackt_format::PieceId,
    label: &str,
    armed: bool,
    actions: &ActionQueue,
    act: UiAct,
) -> View {
    let icon = crate::ui::thumb::piece_icon(
        id,
        &retrackt_format::PieceParams::default(),
        retrackt_format::DEFAULT_CELL_SIZE,
        dims::tile_icon_h().0,
    )
    .unwrap_or_else(|| {
        Box(Modifier::new()
            .width(dims::tile_icon_h())
            .height(dims::tile_icon_h()))
    });
    let mut face = Modifier::new()
        .width(dims::tile_w())
        .padding(theme::dp(4.0))
        .background(theme::surface())
        .border(
            theme::dp(1.0),
            if armed {
                theme::accent()
            } else {
                theme::text_dim().with_alpha_f32(0.4)
            },
            theme::dp(10.0),
        )
        .clip_rounded(theme::dp(10.0))
        .on_click(pusher(actions, act));
    if armed {
        face = face.background(theme::accent().with_alpha_f32(0.18));
    }
    Box(face).child(
        Column(
            Modifier::new()
                .gap(theme::dp(2.0))
                .align_items(AlignItems::CENTER),
        )
        .child([
            Center(Modifier::new()).child(icon),
            Text(short_name(label))
                .size(theme::sp(11.0))
                .color(theme::text_dim())
                .single_line()
                .overflow_ellipsize(),
        ]),
    )
}

/// The label a tile carries: the catalogue name trimmed to what fits under an
/// icon. "Curve Right 90" is three words for a corner.
fn short_name(label: &str) -> &str {
    match label {
        "Curve Right" => "Curve R",
        "Curve Left" => "Curve L",
        "Curve 180" => "U-turn",
        "Ramp Up" => "Ramp up",
        "Ramp Down" => "Ramp dn",
        "Checkpoint" => "Check pt",
        "JumpGap" | "Jump" => "Jump",
        other => other,
    }
}

/// A palette group heading, with the rule that separates it from the tiles above.
fn group_header(group: &str) -> View {
    Column(Modifier::new().gap(theme::dp(4.0))).child([
        Text(group)
            .size(theme::sp(13.0))
            .color(theme::accent())
            .single_line(),
        Box(Modifier::new()
            .height(theme::dp(1.0))
            .fill_max_width()
            .background(theme::text_dim().with_alpha_f32(0.3))),
    ])
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
    // The gate number of each checkpoint, from one walk of the sequence rather
    // than one per row: the sequence is derived from the whole track, so asking
    // for it per piece would make the list quadratic in the number of pieces.
    let gates: std::collections::HashMap<PieceUid, usize> = data
        .track
        .checkpoint_sequence()
        .iter()
        .enumerate()
        .map(|(nth, piece)| (piece.uid, nth + 1))
        .collect();

    data.track
        .pieces
        .iter()
        .map(|piece| {
            let uid = piece.uid;
            let selected = data.editor.is_selected(uid);
            let select = pusher(actions, UiAct::SelectPiece(uid));
            let toggle = pusher(actions, UiAct::ToggleSelect(uid));
            let (len, rad, bank) = retrackt_format::piece::resolve_params(piece.id, &piece.params);
            let slope = retrackt_format::geometry::resolve_slope(piece.id, &piece.params);

            let nudge = |label: &'static str, kind: ParamKind, delta: i8| {
                let queue = actions.clone();
                ctrl(label, move || {
                    push(&queue, UiAct::AdjustParam(uid, kind, delta))
                })
            };

            // Radius means nothing for a piece the catalogue gives none, so the
            // control is withheld rather than offered and silently discarded.
            let radius = if !piece.id.has_radius() {
                Vec::new()
            } else {
                vec![
                    dim_text(&format!("rad {rad}")),
                    nudge("R-", ParamKind::Radius, -1),
                    nudge("R+", ParamKind::Radius, 1),
                ]
            };

            // Three bands rather than one wrapped row: the identity gets a line to
            // itself, the four edits get a line of their own, and the parameters a
            // third. As one row the identity and four controls had to share 440 dp,
            // which is 464 dp of content at the floor — so the row wrapped the same
            // ragged way at every window size instead of responding to any of them.
            let length = if piece.id.has_length() {
                vec![
                    dim_text(&format!("len {len}")),
                    nudge("L-", ParamKind::Length, -1),
                    nudge("L+", ParamKind::Length, 1),
                ]
            } else {
                Vec::new()
            };
            let bank = if piece.id.is_slope() {
                // A slope leans across the road like a bank, and the same
                // control carries it.
                vec![
                    dim_text(&format!("bank {bank}°")),
                    nudge("B-", ParamKind::Bank, -5),
                    nudge("B+", ParamKind::Bank, 5),
                ]
            } else {
                Vec::new()
            };
            // Steepness is the one control that has nothing to say about a flat
            // piece, and everything about a slope.
            let steepness = if piece.id.is_slope() {
                vec![
                    dim_text(&format!("slope {}°", slope.round() as i32)),
                    nudge("S-", ParamKind::Slope, -5),
                    nudge("S+", ParamKind::Slope, 5),
                ]
            } else {
                Vec::new()
            };
            let params = length
                .into_iter()
                .chain(radius)
                .chain(bank)
                .chain(steepness);
            // A checkpoint carries one more value, and it is the one that decides
            // the run: the order the gates come in. Shown on the piece that owns it
            // rather than in a panel of its own, because it is a property of that
            // one gate and of nothing else on the track.
            let params: Vec<View> = if retrackt_format::piece::catalog_by_id(piece.id).is_checkpoint
            {
                params
                    .chain([
                        dim_text(&format!("gate {}", gates.get(&uid).copied().unwrap_or(0))),
                        nudge("G-", ParamKind::Order, -1),
                        nudge("G+", ParamKind::Order, 1),
                    ])
                    .collect()
            } else {
                params.collect()
            };

            Column(Modifier::new().gap(dims::SPACE_LIST)).child([
                Row(Modifier::new()
                    .gap(dims::SPACE_ROW)
                    .align_items(AlignItems::CENTER))
                .child(
                    Box(Modifier::new().width(dims::RANK_W)).child(
                        Text(format!("{}.", rank(uid)))
                            .size(theme::sp(14.0))
                            .color(theme::text_dim())
                            .single_line(),
                    ),
                )
                .child(
                    // Capped, not stretched: a single-line label measures as
                    // its own full length however narrow the box is, so left
                    // uncapped it hands the row a minimum width wider than the
                    // panel and the piece list grows a sideways scroll.
                    Box(Modifier::new().fill_max_width().max_width(dims::label_w())).child(
                        hud_text(&format!(
                            "{} [{} {} {}] yaw {}",
                            piece.id.label(),
                            piece.cell[0],
                            piece.cell[1],
                            piece.cell[2],
                            piece.yaw,
                        ))
                        .overflow_ellipsize(),
                    ),
                ),
                ctrl_row(vec![
                    if selected {
                        ctrl_on("Sel", select)
                    } else {
                        ctrl("Sel", select)
                    },
                    ctrl("+Sel", toggle),
                    ctrl("Rotate", pusher(actions, UiAct::RotatePiece(uid))),
                    ctrl_danger("Remove", pusher(actions, UiAct::DeletePiece(uid))),
                ]),
                ctrl_row(params),
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
    // What the pointer is over, which is what [G] takes. Shown rather than left
    // silent: a key that selects something the player cannot see is a coin toss.
    let hover = match editor.hover.and_then(|uid| data.track.piece(uid)) {
        Some(piece) => piece.id.label().to_string(),
        None => "nothing".into(),
    };
    children.push(dim_text(&format!("Pointer over: {hover}")));
    children.push(dim_text(if crate::ui::library::persisted(&data.track) {
        "Saved to library"
    } else {
        "Unsaved changes"
    }));

    children.push(cursor_row(editor, actions));

    // No scroller of its own: this list is inside the column's scroller already, and
    // a second one under the same pointer takes the wheel and keeps it.
    children.push(unscrolled(piece_rows(data, actions)));

    children.push(ctrl_row(vec![
        ctrl_on("Place", pusher(actions, UiAct::PlaceArmed)),
        history_btn("Undo", editor.can_undo, actions, UiAct::Undo),
        history_btn("Redo", editor.can_redo, actions, UiAct::Redo),
        ctrl("Select All", pusher(actions, UiAct::SelectAll)),
        ctrl("Select Route", pusher(actions, UiAct::SelectRoute)),
        ctrl("Clear Sel", pusher(actions, UiAct::SelectNone)),
    ]));

    children.push(ctrl_row(vec![
        ctrl(
            "Move to Cursor",
            pusher(actions, UiAct::MoveSelectionToCursor),
        ),
        ctrl("Rotate", pusher(actions, UiAct::RotateSelection(1))),
        ctrl("Rotate Back", pusher(actions, UiAct::RotateSelection(-1))),
        ctrl("Copy", pusher(actions, UiAct::CopySelection)),
        ctrl("Paste", pusher(actions, UiAct::PasteAtCursor)),
        ctrl("Duplicate", pusher(actions, UiAct::DuplicateSelection)),
        ctrl_danger("Delete", pusher(actions, UiAct::DeleteSelection)),
    ]));

    // Retuning a whole selection is the point of selecting: lengthening every
    // straight on a circuit one button at a time is not an edit anyone makes.
    children.push(ctrl_row(vec![
        dim_text("Selection"),
        ctrl(
            "len-",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Length, -1)),
        ),
        ctrl(
            "len+",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Length, 1)),
        ),
        ctrl(
            "rad-",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Radius, -1)),
        ),
        ctrl(
            "rad+",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Radius, 1)),
        ),
        ctrl(
            "bank-",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Bank, -5)),
        ),
        ctrl(
            "bank+",
            pusher(actions, UiAct::AdjustSelection(ParamKind::Bank, 5)),
        ),
    ]));

    children.push(dim_text(&format!(
        "Clipboard: {} piece{}",
        editor.clipboard.len(),
        if editor.clipboard.len() == 1 { "" } else { "s" }
    )));

    // The editor's own keys, so they can be found on the screen they work on.
    // The camera keys in particular are meaningless without this: three of them
    // sit under the left hand where the race puts the throttle.
    children.push(controls_panel());

    let mut loaders: Vec<View> = Vec::new();
    for track in retrackt_format::builtin_tracks() {
        let label = format!("Load {}", track.name);
        let click = pusher(actions, UiAct::LoadTrack(TrackRef::Builtin(track.name)));
        loaders.push(ctrl(&label, click));
    }
    let mut row = vec![
        ctrl("Save", pusher(actions, UiAct::SaveTrack)),
        ctrl("Remove Last", pusher(actions, UiAct::RemoveLastPiece)),
        ctrl_danger("Clear Track", pusher(actions, UiAct::ClearTrack)),
    ];
    row.extend(loaders);
    children.push(ctrl_row(row));

    children
}

/// The editor's own controls, on the screen they belong to.
///
/// Written out rather than implied, because the editor's keys are not the race's:
/// the arrows and WASD move the camera here and the car there, and a player who
/// crosses between the two has nothing but this to tell them so.
fn controls_panel() -> View {
    let lines: [(&str, &str); 9] = [
        ("WASD / arrows", "move the camera"),
        ("Q / E", "turn the camera"),
        ("Wheel", "zoom"),
        ("Shift + wheel", "edit height"),
        ("Right drag", "move · middle or Ctrl + right: turn"),
        ("Click / tap", "place · on a piece, select it"),
        ("G", "select under the pointer"),
        ("R / Space", "rotate the selection"),
        ("X / Del", "delete the selection"),
    ];
    let rows: Vec<View> = lines
        .iter()
        .map(|(keys, what)| {
            Row(Modifier::new().gap(dims::SPACE_ROW).fill_max_width()).child([
                Box(Modifier::new().width(dims::KEY_W)).child(
                    Text(*keys)
                        .size(theme::sp(13.0))
                        .color(theme::accent())
                        .single_line(),
                ),
                Box(Modifier::new().flex_grow(1.0)).child(
                    Text(*what)
                        .size(theme::sp(13.0))
                        .color(theme::text_dim())
                        .single_line()
                        .overflow_ellipsize(),
                ),
            ])
        })
        .collect();
    list_panel_w("Controls", dims::panel_w(), rows)
}

/// Cursor cell, its six directions, and whether the next placement connects.
fn cursor_row(editor: &EditorData, actions: &ActionQueue) -> View {
    let mut steps: Vec<View> = Vec::new();
    for (label, delta) in STEPS {
        steps.push(ctrl(label, pusher(actions, UiAct::MoveCursor(delta))));
    }
    Column(Modifier::new().gap(dims::SPACE_LIST))
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
        .child(ctrl_row(steps))
}

/// Undo and redo. Disabled rather than hidden: a live button that does nothing is
/// the confusing case, and the player still needs to see the feature exists.
fn history_btn(label: &'static str, enabled: bool, actions: &ActionQueue, act: UiAct) -> View {
    if enabled {
        ctrl(label, pusher(actions, act))
    } else {
        ctrl_off(label)
    }
}

/// The track's problems, errors first. Shown always rather than only on failure
/// so a broken chain is visible while it is being made, not after a refused race.
fn diagnostics_panel(editor: &EditorData) -> View {
    if editor.diagnostics.is_empty() {
        return list_panel_w(
            "Checks",
            dims::panel_w(),
            vec![
                Text("No problems found")
                    .size(theme::sp(15.0))
                    .color(theme::ok())
                    .single_line(),
            ],
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
                .overflow_ellipsize()
        })
        .collect();
    list_panel_w("Checks", dims::panel_w(), rows)
}

/// Removes the scroller around a list that already sits inside one.
///
/// A wheel event is resolved against the innermost scroller under the pointer and
/// stops there. An inner list therefore holds the wheel for its whole width, and
/// where the list is shorter than its frame it holds it while having nowhere to
/// scroll — the column behind it never moves, and the track never sees the wheel
/// that would zoom it. One scroller per screen region: the column scrolls, and the
/// list is part of its content.
fn unscrolled(children: Vec<View>) -> View {
    Column(Modifier::new().gap(dims::SPACE_LIST)).child(children)
}

/// Share-code export and import.
fn share_panel(editor: &EditorData, actions: &ActionQueue) -> View {
    let draft: Rc<RefCell<TextFieldState>> =
        remember_with_key("editor.code", || RefCell::new(TextFieldState::new()));
    if draft.borrow().text != editor.code_draft {
        draft.borrow_mut().text = editor.code_draft.clone();
    }

    let mut children = vec![
        // Fills the width itself: `fill_max_width` on the field alone fills the
        // column, and a column sized to its content is only as wide as that content
        // . So the field came out at the width of its own hint with half the panel
        // beside it unused.
        Column(Modifier::new().gap(dims::SPACE_LIST).fill_max_width())
            // The field takes a line to itself rather than sharing one with two
            // buttons. Unbounded in a wrapping row it claimed a line anyway, and the
            // buttons below it became a second row for no reason the row could have
            // been asked for.
            .child(BasicTextField(
                draft.clone(),
                // The same base height as the controls beside it, or the form has
                // a field a few dp shorter than the buttons under it and reads as
                // two different controls.
                Modifier::new().fill_max_width().min_height(dims::CTRL_H),
                "Paste a share code",
                TextFieldConfig {
                    line_limits: TextFieldLineLimits::SingleLine,
                    on_change: Some(Rc::new({
                        let queue = actions.clone();
                        move |text: String| push(&queue, UiAct::SetCodeDraft(text))
                    }) as Rc<dyn Fn(String)>),
                    ..TextFieldConfig::default()
                },
            ))
            .child(ctrl_row(vec![
                ctrl("Load Code", pusher(actions, UiAct::LoadShareCode)),
                ctrl_on("Copy Code", pusher(actions, UiAct::CopyShareCode)),
            ])),
    ];
    if !editor.code_out.is_empty() {
        // Also shown on screen: a platform with no working clipboard still leaves
        // the player able to read the code out and paste it somewhere.
        children.push(dim_text(&format!("Code: {}", editor.code_out)));
    }
    list_panel_w("Share", dims::panel_w(), children)
}

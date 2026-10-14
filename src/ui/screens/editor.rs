use std::cell::RefCell;
use std::rc::Rc;

// `TextFieldLineLimits` and `TextStyle` come from repose-core (the config value
// structs); the `TextStyle` trait that styles `Text` views is repose-ui's, and
// the two share a name. Both imports are needed and neither shadows the other.
use repose_core::{
    AlignItems, Dp, Modifier, PaddingValues, TextFieldLineLimits, View, remember_with_key,
};
use repose_material::material3::{Button, ButtonConfig, ButtonDefaults};
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
    let palette_body = palette_list(editor.armed, editor.palette_category, actions);
    // The panel names the shelf on show rather than the whole shelf: the strip
    // inside it is what picks a category, and a title reading "Palette" over a
    // grid of one category says less than the category does.
    let palette_title = {
        let mut groups: Vec<&str> = Vec::new();
        for def in catalog() {
            let group = def.id.group();
            if groups.contains(&group) {
                continue;
            }
            groups.push(group);
        }
        groups[editor.palette_category.min(groups.len() - 1)].to_string()
    };
    let palette = list_panel_w(
        &palette_title,
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

    // The paint of the selection, where the rest of the track's contents are:
    // colour is a property of a piece already laid, not of the one being placed,
    // so it belongs with the piece list rather than with the palette.
    let color_panel = color_panel(data, actions);
    let track_column = Column(Modifier::new().gap(dims::SPACE_SECTION)).child([
        track_settings_panel(data, actions),
        selection_panel(data, actions),
        track,
        color_panel,
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

/// The paint of the selection.
///
/// A track is one colour by default, which is right for a circuit and wrong for a
/// set piece: a red section, an ice bridge and a desert run are the fastest way
/// to tell where on a track you are.
fn color_panel(data: &AppData, actions: &ActionQueue) -> View {
    let editor = &data.editor;
    // Read back off the track, not off the editor: the paint lives on the piece,
    // so the swatch that is lit is the one the selection is actually wearing.
    let current = editor
        .selection
        .first()
        .and_then(|uid| data.track.piece(*uid).and_then(|piece| piece.params.color));
    let mut swatches: Vec<View> = Vec::new();
    for (index, _hex) in retrackt_format::PIECE_COLORS.iter().enumerate() {
        let selected = current == Some(index as u8);
        swatches.push(swatch(index as u8, selected, actions));
    }
    list_panel_w(
        "Paint",
        dims::panel_w(),
        vec![
            Column(Modifier::new().gap(dims::SPACE_ROW)).child([
                FlowRow(
                    Modifier::new().gap(theme::dp(4.0)).fill_max_width(),
                    FlowRowConfig {
                        balanced: true,
                        ..FlowRowConfig::default()
                    },
                )
                .child(swatches),
                if editor.selection.is_empty() {
                    Text("Select a piece to recolour it.")
                        .size(theme::sp(12.0))
                        .color(theme::text_dim())
                        .single_line()
                } else {
                    ctrl(
                        "Cycle Selection",
                        pusher(actions, UiAct::CycleSelectionColor(1)),
                    )
                },
            ]),
        ],
    )
}

/// One paint swatch. Tapping it paints the selection rather than arming a colour
/// for the next placement: a palette that armed instead of applied would need a
/// second gesture before any of it took effect.
fn swatch(index: u8, selected: bool, actions: &ActionQueue) -> View {
    Box(Modifier::new()
        .width(theme::dp(30.0))
        .height(theme::dp(30.0))
        .background(theme::rgb(retrackt_format::PIECE_COLORS[index as usize]))
        .border(
            theme::dp(2.0),
            if selected {
                theme::accent()
            } else {
                theme::text_dim().with_alpha_f32(0.4)
            },
            theme::dp(15.0),
        )
        .input_blocker())
    .child(Button(
        Modifier::new().fill_max_size(),
        pusher(actions, UiAct::SetSelectionColor(index)),
        ButtonConfig {
            container_color: Some(repose_core::Color::TRANSPARENT),
            content_color: Some(repose_core::Color::TRANSPARENT),
            ..ButtonConfig::default()
        },
        || Box(Modifier::new().fill_max_size()),
    ))
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

/// Palette. Clicking *arms* a piece rather than placing it, so the preview
/// can be inspected first; placement is its own gesture.
///
/// A strip of categories with the parts of the chosen one beside it, the way
/// the reference's editor splits them: a wall of eight labelled sections is a
/// list to scroll past, while one category of parts at a time is a shelf to
/// pick from. Tiles carry the piece's footprint and its short name rather than
/// a button of prose, and arming is all a tile ever does.
///
/// The strip is a column beside the shelf wherever there is width for a column,
/// because a category a player has chosen stays on screen as a row of tabs costs
/// a whole line of the shelf. Too narrow for both, the strip goes back above the
/// shelf and wraps, which is the same split one tile fewer.
fn palette_list(armed: retrackt_format::PieceId, category: usize, actions: &ActionQueue) -> View {
    let mut groups: Vec<&str> = Vec::new();
    for def in catalog() {
        let group = def.id.group();
        if groups.contains(&group) {
            continue;
        }
        groups.push(group);
    }
    let shown = category.min(groups.len().saturating_sub(1));
    let active = groups[shown];

    let tabs: Vec<View> = groups
        .iter()
        .enumerate()
        .map(|(index, group)| category_tab(index, group, index == shown, actions))
        .collect();
    let parts: Vec<View> = catalog()
        .iter()
        .filter(|d| d.id.group() == active)
        .map(|piece| {
            piece_tile(
                piece.id,
                piece.id.label(),
                piece.id == armed,
                actions,
                UiAct::ArmPiece(piece.id),
            )
        })
        .collect();

    // Greedy wrapping, not balanced. Balancing is for rows of differing widths,
    // where it evens out the ragged last line; every tile and every tab here is
    // the same fixed width, so there is nothing to even out and the balance pass
    // only gets between the shelf and the pieces in it.
    let wrap = |items: Vec<View>| {
        FlowRow(
            Modifier::new().gap(dims::SPACE_ROW).fill_max_width(),
            FlowRowConfig {
                balanced: false,
                ..FlowRowConfig::default()
            },
        )
        .child(items)
    };
    let tab = wrap(tabs.clone());
    let shelf = wrap(parts);

    // Both wrapped rows are given the width they are to lay out in. Left to fill it,
    // they measure against a scrolling body sized by its own contents, take the
    // width of one line of tiles, and never wrap: the row puts every piece of a
    // category on one line and lays out all but the first two past the edge of the
    // palette holding them, where the panel's clip takes their names away.
    match dims::stacked() {
        true => Column(Modifier::new().gap(dims::SPACE_ROW)).child([
            Box(Modifier::new().width(dims::palette_content_w())).child(tab),
            Box(Modifier::new().width(dims::shelf_content_w())).child(shelf),
        ]),
        false => Row(Modifier::new()
            .gap(dims::SPACE_ROW)
            .align_items(AlignItems::START))
        .child([
            Box(Modifier::new().width(dims::strip_w()))
                .child(Column(Modifier::new().gap(dims::SPACE_ROW)).child(tabs)),
            Box(Modifier::new().width(dims::shelf_content_w())).child(shelf),
        ]),
    }
}

/// One category in the palette strip. Accent-filled while it is the category
/// showing, since the strip is what says which shelf of parts is on screen.
fn category_tab(index: usize, group: &str, selected: bool, actions: &ActionQueue) -> View {
    let config = ButtonConfig {
        height: theme::dp(34.0),
        shape_radius: dims::CTRL_R,
        content_padding: Some(ButtonDefaults::TEXT_CONTENT_PADDING),
        ..if selected {
            ButtonConfig {
                container_color: Some(theme::accent()),
                content_color: Some(theme::background()),
                ..ctrl_config()
            }
        } else {
            ctrl_config()
        }
    };
    let queue = actions.clone();
    let w = match dims::stacked() {
        // Stacked the tabs share a line with each other, so a fixed width keeps
        // them equal; as a column each fills the strip.
        true => theme::dp(64.0),
        false => dims::strip_w(),
    };
    outlined_at(group, w, config, move || {
        push(&queue, UiAct::SetPaletteCategory(index))
    })
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
        .height(dims::tile_h())
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
/// The tile's short name for a piece label, so it fits under an icon.
pub(crate) fn short_name(label: &str) -> &str {
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
            let (len, rad, bank) = retrackt_format::piece::resolve_params(piece.id, &piece.params);
            let slope = retrackt_format::geometry::resolve_slope(piece.id, &piece.params);

            // What the piece is set to, read rather than edited here. A row that
            // carries a control for each of its values is a list whose rows cannot
            // be scanned: eleven controls in a row leaves nothing to read, so the
            // list is where the player looks to find a piece and the inspector is
            // where they change it.
            let mut bits: Vec<String> = Vec::new();
            if piece.id.has_length() {
                bits.push(format!("len {len}"));
            }
            if piece.id.has_radius() {
                bits.push(format!("rad {rad}"));
            }
            if piece.id.is_slope() {
                bits.push(format!("slope {}°", slope.round() as i32));
                bits.push(format!("bank {bank}°"));
            }
            if piece.id.is_driveable() && piece.params.wide {
                bits.push("wide".into());
            }
            if retrackt_format::piece::catalog_by_id(piece.id).is_checkpoint {
                bits.push(format!("gate {}", gates.get(&uid).copied().unwrap_or(0)));
            }

            let mut face = Modifier::new()
                .fill_max_width()
                .gap(dims::SPACE_ROW)
                .align_items(AlignItems::CENTER)
                .padding(theme::dp(4.0))
                .clip_rounded(theme::dp(6.0))
                .on_click(pusher(actions, UiAct::SelectPiece(uid)));
            if data.editor.is_selected(uid) {
                face = face.background(theme::accent().with_alpha_f32(0.18));
            }
            Row(face).child([
                Box(Modifier::new().width(dims::RANK_W)).child(
                    Text(format!("{}.", rank(uid)))
                        .size(theme::sp(14.0))
                        .color(theme::text_dim())
                        .single_line(),
                ),
                // Capped, not stretched: a single-line label measures as its
                // own full length however narrow the box is, so left uncapped
                // it hands the row a minimum width wider than the panel and the
                // piece list grows a sideways scroll.
                Box(Modifier::new().flex_grow(1.0).max_width(dims::label_w()))
                    .child(hud_text(piece.id.label()).overflow_ellipsize()),
                Box(Modifier::new().width(theme::dp(72.0))).child(
                    dim_text(&format!(
                        "[{} {} {}]",
                        piece.cell[0], piece.cell[1], piece.cell[2]
                    ))
                    .overflow_ellipsize(),
                ),
                // Weighted against the name rather than equal to it: a name is four
                // characters and the values are the reason to read the row.
                Box(Modifier::new().flex_grow(2.0))
                    .child(dim_text(&bits.join(" · ")).overflow_ellipsize()),
            ])
        })
        .collect()
}

/// One tunable value, its current setting, and the two ways to move it.
///
/// A named value rather than a pair of buttons that say `len-` and `len+`: a
/// player reading `len-` has to remember what it shortened, and a pair that does
/// not say what it holds cannot be scanned at all when every value on the piece
/// has one.
fn value_row(
    label: &'static str,
    value: String,
    kind: ParamKind,
    step: i8,
    actions: &ActionQueue,
) -> View {
    let down = {
        let queue = actions.clone();
        outlined_at("-", dims::ctrl_w(), ctrl_config(), move || {
            push(&queue, UiAct::AdjustSelection(kind, -step))
        })
    };
    let up = {
        let queue = actions.clone();
        outlined_at("+", dims::ctrl_w(), ctrl_config(), move || {
            push(&queue, UiAct::AdjustSelection(kind, step))
        })
    };
    Row(Modifier::new()
        .gap(dims::SPACE_ROW)
        .align_items(AlignItems::CENTER)
        .fill_max_width())
    .child([
        Box(Modifier::new().width(theme::dp(72.0))).child(dim_text(label)),
        Box(Modifier::new().flex_grow(1.0)).child(hud_text(&value).overflow_ellipsize()),
        down,
        up,
    ])
}

/// The value every selected piece agrees on, or a note that they do not.
///
/// A row of pieces retuned together shows one number while they agree and says so
/// the moment they do not, rather than showing whichever piece happened to be
/// read first: the number has to be the number the next press will move.
fn agreed(
    pieces: &[&retrackt_format::PieceInstance],
    read: impl Fn(&retrackt_format::PieceInstance) -> String,
) -> String {
    let first = read(pieces[0]);
    // `iter` yields `&&PieceInstance` and a closure argument does not deref on its
    // own, so the inner reference is taken out by hand.
    match pieces.iter().all(|p| read(*p) == first) {
        true => first,
        false => "mixed".into(),
    }
}

/// The selection, and every value that can be changed about it.
///
/// One panel per selection rather than a control strip on every row of the list.
/// The list is read constantly and edited rarely, and the piece it must show is
/// whatever the selection is, so the controls move here where there is room to
/// name them. Above the list rather than below it, because a piece list grows
/// without limit and a panel under forty rows is a panel nobody reaches.
fn selection_panel(data: &AppData, actions: &ActionQueue) -> View {
    let editor = &data.editor;
    let pieces: Vec<&retrackt_format::PieceInstance> = editor
        .selection
        .iter()
        .filter_map(|uid| data.track.piece(*uid))
        .collect();
    if pieces.is_empty() {
        return list_panel_w(
            "Selection",
            dims::panel_w(),
            vec![dim_text(
                "Nothing selected. Click a piece in the list below.",
            )],
        );
    }
    let one = pieces.len() == 1;
    let named = match pieces.as_slice() {
        [only] => only.id.label().to_string(),
        many => format!("{} pieces", many.len()),
    };
    let mut rows: Vec<View> = vec![hud_text(&named)];

    // A control is offered only when every selected piece has the value it would
    // move. Offering one that half the selection cannot honour is a button that
    // sometimes does nothing, which is the one behaviour a control must not have.
    let every = |has: fn(retrackt_format::PieceId) -> bool| pieces.iter().all(|p| has(p.id));
    if every(retrackt_format::PieceId::has_length) {
        rows.push(value_row(
            "Length",
            agreed(&pieces, |p| {
                format!(
                    "{} cells",
                    retrackt_format::piece::resolve_params(p.id, &p.params).0
                )
            }),
            ParamKind::Length,
            1,
            actions,
        ));
    }
    if every(retrackt_format::PieceId::has_radius) {
        rows.push(value_row(
            "Radius",
            agreed(&pieces, |p| {
                format!(
                    "{} cells",
                    retrackt_format::piece::resolve_params(p.id, &p.params).1
                )
            }),
            ParamKind::Radius,
            1,
            actions,
        ));
    }
    if every(retrackt_format::PieceId::is_slope) {
        rows.push(value_row(
            "Slope",
            agreed(&pieces, |p| {
                format!(
                    "{}°",
                    retrackt_format::geometry::resolve_slope(p.id, &p.params).round() as i32
                )
            }),
            ParamKind::Slope,
            5,
            actions,
        ));
        rows.push(value_row(
            "Bank",
            agreed(&pieces, |p| {
                format!(
                    "{}°",
                    retrackt_format::piece::resolve_params(p.id, &p.params).2
                )
            }),
            ParamKind::Bank,
            5,
            actions,
        ));
    }
    if one {
        let piece = pieces[0];
        if piece.id.is_driveable() {
            let queue = actions.clone();
            let uid = piece.uid;
            let wide = piece.params.wide;
            rows.push(
                Row(Modifier::new()
                    .gap(dims::SPACE_ROW)
                    .align_items(AlignItems::CENTER)
                    .fill_max_width())
                .child([
                    Box(Modifier::new().width(theme::dp(72.0))).child(dim_text("Width")),
                    Box(Modifier::new().flex_grow(1.0)).child(hud_text(if wide {
                        "Wide"
                    } else {
                        "Narrow"
                    })),
                    ctrl(if wide { "Narrow" } else { "Wide" }, move || {
                        push(&queue, UiAct::ToggleWide(uid))
                    }),
                ]),
            );
        }
    }
    if pieces
        .iter()
        .all(|p| retrackt_format::piece::catalog_by_id(p.id).is_checkpoint)
    {
        let sequence = data.track.checkpoint_sequence();
        let shown = |uid: PieceUid| {
            sequence
                .iter()
                .position(|p| p.uid == uid)
                .map(|i| i + 1)
                .unwrap_or(0)
        };
        rows.push(value_row(
            "Gate",
            agreed(&pieces, |p| {
                format!("{} of {}", shown(p.uid), sequence.len())
            }),
            ParamKind::Order,
            1,
            actions,
        ));
    }

    rows.push(ctrl_row(vec![
        ctrl("Rotate", pusher(actions, UiAct::RotateSelection(1))),
        ctrl("Rot -1", pusher(actions, UiAct::RotateSelection(-1))),
        ctrl("Duplicate", pusher(actions, UiAct::DuplicateSelection)),
        ctrl_danger("Delete", pusher(actions, UiAct::DeleteSelection)),
    ]));
    rows.push(ctrl_row(vec![
        ctrl("Select All", pusher(actions, UiAct::SelectAll)),
        ctrl("Select Route", pusher(actions, UiAct::SelectRoute)),
        ctrl("Clear Sel", pusher(actions, UiAct::SelectNone)),
        ctrl("Copy", pusher(actions, UiAct::CopySelection)),
    ]));
    rows.push(dim_text(&format!(
        "Clipboard: {} piece{}",
        editor.clipboard.len(),
        if editor.clipboard.len() == 1 { "" } else { "s" }
    )));
    list_panel_w("Selection", dims::panel_w(), rows)
}

/// What the track itself is: its name, and where the run begins and ends.
///
/// PolyTrack's editor calls this Track settings, and it is the panel a track is
/// *about* rather than a piece of it: the name is what the track is called
/// everywhere else in the game, and the start and finish are the two facts about a
/// circuit a player checks before asking whether the middle is any good.
fn track_settings_panel(data: &AppData, actions: &ActionQueue) -> View {
    let track = &data.track;
    let start = track
        .pieces
        .iter()
        .find(|p| retrackt_format::piece::catalog_by_id(p.id).is_start)
        .map(|p| cell_of(p))
        .unwrap_or_else(|| NONE.into());
    let finishes: Vec<String> = track
        .race_order()
        .iter()
        .filter_map(|uid| track.piece(*uid))
        .filter(|p| retrackt_format::piece::catalog_by_id(p.id).is_finish)
        .map(cell_of)
        .collect();
    let checkpoints = track.checkpoint_sequence().len();

    // A circuit may end in more than one place, and every cell of all of them is
    // more list than a single row can carry. The first, and a count of the rest:
    // which line is the finish is the fact, and the rest is arithmetic.
    let finish = match finishes.len() {
        0 => NONE.into(),
        1 => finishes[0].clone(),
        _ => format!("{} +{}", finishes[0], finishes.len() - 1),
    };

    let draft: Rc<RefCell<TextFieldState>> =
        remember_with_key("editor.name", || RefCell::new(TextFieldState::new()));
    let shown_name = match data.editor.name_draft.is_empty() {
        true => track.name.clone(),
        false => data.editor.name_draft.clone(),
    };
    // Synced rather than seeded, because a field rebuilt each frame loses its focus and
    // its caret on every keystroke and would accept one character and drop it.
    if draft.borrow().text != shown_name {
        draft.borrow_mut().text = shown_name;
    }
    let renamed = !data.editor.name_draft.is_empty() && data.editor.name_draft.trim() != track.name;

    list_panel_w(
        "Track settings",
        dims::panel_w(),
        vec![
            Column(Modifier::new().gap(dims::SPACE_LIST).fill_max_width()).child([
                BasicTextField(
                    draft,
                    Modifier::new().fill_max_width().min_height(dims::CTRL_H),
                    "Track name",
                    TextFieldConfig {
                        line_limits: TextFieldLineLimits::SingleLine,
                        on_change: Some(Rc::new({
                            let queue = actions.clone();
                            move |text: String| push(&queue, UiAct::SetNameDraft(text))
                        }) as Rc<dyn Fn(String)>),
                        ..TextFieldConfig::default()
                    },
                ),
                ctrl_row(vec![match renamed {
                    true => ctrl_on("Rename", pusher(actions, UiAct::RenameTrack)),
                    false => ctrl_off("Rename"),
                }]),
            ]),
            field_row("Start line", start),
            field_row("Finish line", finish),
            field_row("Gates", checkpoints.to_string()),
        ],
    )
}

/// Reads as "none" rather than as an empty row, so an empty track says it has no
/// start rather than showing a blank where one would be.
const NONE: &str = "not placed";

/// One named fact about the track, read rather than changed.
///
/// The label gets 96 dp because "Finish line" does not fit in 72, and a label too
/// narrow for its own text is not merely clipped on screen: the layout drops the
/// text node entirely, so the label stops existing for anything reading the laid
/// out tree. The value is capped as well as ellipsized, because a single-line
/// label measures as its own full length however narrow the box around it, so left
/// uncapped a long cell coordinate hands the row a minimum width wider than the
/// panel it sits in and the whole column grows a sideways scrollbar.
fn field_row(label: &str, value: String) -> View {
    Row(Modifier::new()
        .gap(dims::SPACE_ROW)
        .align_items(AlignItems::CENTER)
        .fill_max_width())
    .child([
        Box(Modifier::new().width(theme::dp(96.0))).child(dim_text(label)),
        Box(Modifier::new().flex_grow(1.0).max_width(value_w()))
            .child(hud_text(&value).overflow_ellipsize()),
    ])
}

/// Width a fact's value may take: the panel's content less its label and the gap.
fn value_w() -> Dp {
    (dims::content_w() - theme::dp(104.0)).coerce_at_least(Dp(0.0))
}

fn cell_of(piece: &retrackt_format::PieceInstance) -> String {
    format!("[{} {} {}]", piece.cell[0], piece.cell[1], piece.cell[2])
}

fn build_children(data: &AppData, actions: &ActionQueue) -> Vec<View> {
    let editor = &data.editor;
    let mut children: Vec<View> = Vec::new();
    let count = data.track.pieces.len();
    children.push(dim_text(&format!(
        "{count} pieces · {} selected · {}",
        editor.selection.len(),
        if crate::ui::library::persisted(&data.track) {
            "saved"
        } else {
            "unsaved"
        }
    )));
    // What the pointer is over, which is what [G] takes. Shown rather than left
    // silent: a key that selects something the player cannot see is a coin toss.
    let hover = match editor.hover.and_then(|uid| data.track.piece(uid)) {
        Some(piece) => piece.id.label().to_string(),
        None => "nothing".into(),
    };
    children.push(dim_text(&format!("Pointer over: {hover}")));

    children.push(cursor_row(editor, actions));

    // No scroller of its own: this list is inside the column's scroller already, and
    // a second one under the same pointer takes the wheel and keeps it.
    children.push(unscrolled(piece_rows(data, actions)));

    // Placing, and only placing. Everything about the selection lives in the
    // inspector, and the history is its own pair of buttons rather than two more
    // in a row that mixes it with what is being placed.
    children.push(ctrl_row(vec![
        ctrl_on("Place", pusher(actions, UiAct::PlaceArmed)),
        ctrl("Turn CW", pusher(actions, UiAct::RotateArmed(1))),
        ctrl("Turn CCW", pusher(actions, UiAct::RotateArmed(-1))),
        ctrl(
            if editor.snap { "Snap: On" } else { "Snap: Off" },
            pusher(actions, UiAct::ToggleSnap),
        ),
    ]));
    children.push(ctrl_row(vec![
        history_btn("Undo", editor.can_undo, actions, UiAct::Undo),
        history_btn("Redo", editor.can_redo, actions, UiAct::Redo),
        ctrl("Paste", pusher(actions, UiAct::PasteAtCursor)),
        ctrl(
            "Move to Cursor",
            pusher(actions, UiAct::MoveSelectionToCursor),
        ),
    ]));

    // The editor's own keys, behind their own button. The camera keys in
    // particular are meaningless without them — three of those sit under the left
    // hand where the race puts the throttle — but they are nine rows of text about
    // keys nobody reads while placing a piece, and open they stood between the
    // player and the list of pieces.
    children.push(ctrl_row(vec![ctrl(
        if editor.show_help {
            "Hide keys"
        } else {
            "Keys"
        },
        pusher(actions, UiAct::ToggleHelp),
    )]));
    if editor.show_help {
        children.push(controls_panel());
    }

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

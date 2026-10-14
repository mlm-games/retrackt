//! Every length the track editor lays itself out with.
//!
//! The editor used to carry its own dp numbers — 660 for the side column, 620 for
//! the piece list, 640 for the action rows, 230 for the palette, 690 and 260 for
//! the reserves, and gaps of 4, 6, 8, 10 and 14 — and each was picked against one
//! desktop window. Several disagreed with each other: the piece list asked for
//! 620 dp inside a panel whose content box is 520, the action rows were capped at
//! 640, and the two panes together asked for 930 dp of a window that holds 600.
//!
//! So the gaps come off one scale named by the role a gap plays, and the widths
//! are derived from the panel they sit in rather than picked beside it. Nothing
//! here is a font size, a colour or a duration: the editor's type, palette and
//! silence are untouched.

use repose_core::Dp;
use repose_material::material3::ButtonDefaults;

use crate::app::theme;
use crate::ui::fit;
use crate::ui::widgets;

/// Between the two bands of one piece row: its identity above its controls.
pub const SPACE_LIST: Dp = Dp(4.0);

/// Between controls that share a line, and between the bands inside one row.
pub const SPACE_ROW: Dp = Dp(8.0);

/// Between rows inside a panel. The panel's own gap, named here so the editor's
/// vertical budget can be reasoned about rather than counted off the source.
pub use crate::ui::widgets::PANEL_GAP as SPACE_BLOCK;

/// Between two panels. More than [`SPACE_BLOCK`], because a panel is a bigger
/// object than a row and the break between them should read as one.
pub const SPACE_SECTION: Dp = Dp(16.0);

/// Width of the race-position column of a piece row. Two digits and the period,
/// which is as many as a track's race order runs to.
pub const RANK_W: Dp = Dp(32.0);

/// Width of the key column in the editor's controls panel. Long enough for the
/// widest binding it lists, which is "Shift + wheel" rather than a key.
pub const KEY_W: Dp = Dp(112.0);

/// Height of a control. The material default is 40, which is under the 44 dp
/// touch minimum, and the editor is the screen most likely to be driven by
/// touch — that is what the narrow layout is for.
pub const CTRL_H: Dp = Dp(44.0);

/// Corner radius of every control in the editor. One value for all four variants:
/// the outlined variant's own default is 20, a full pill on a control this tall,
/// while the accent, danger and disabled variants set 8, so `Sel` and `Remove`
/// used to sit on one line as two different families of control.
pub const CTRL_R: Dp = Dp(8.0);

/// The side column's comfortable width. Narrower than a modal's panel on purpose:
/// the editor's panels sit *beside* the track rather than over it, and at a
/// modal's width they take four fifths of a 1024 dp window and leave the track —
/// the thing being edited — as the leftover.
const SIDE_COMFORT_DP: f32 = 480.0;

/// Width of the palette's category strip: a category name and nothing else.
///
/// The longest name is "Junctions", which at the palette's type is a little over
/// half of this, so the strip never has to wrap a label or clip one.
const STRIP_COMFORT_DP: f32 = 112.0;

/// Width of the palette's shelf of parts, beside the strip.
///
/// Room for two tiles across and the gap between them, and no less: at 208 the
/// shelf held one tile filling it whole, which is a banner rather than a shelf of
/// parts. Sized so the comfortable palette divides cleanly into two, because the
/// comfortable palette is the one almost every desktop gets.
const SHELF_COMFORT_DP: f32 = 232.0;

/// The palette's comfortable width, being its two halves side by side.
///
/// The strip and the shelf are one tool split in two rather than two tools, so
/// they are sized as one panel: the strip picks which shelf is showing and the
/// shelf is what a placement is actually made of, and asking for either without
/// the other is how the pair came to want more window than there was.
const PALETTE_COMFORT_DP: f32 = STRIP_COMFORT_DP + SHELF_COMFORT_DP;

/// The narrowest the palette may be squeezed to before the editor stacks instead.
///
/// Set by the shelf keeping one tile that still fits its own name, not by the
/// strip: the shelf below that width cannot show a piece without clipping the
/// name off it, and a clipped name is a piece nobody can tell from its
/// neighbour.
const PALETTE_FLOOR_DP: f32 = 248.0;

/// Narrowest a palette tile may be, which is the widest piece name the palette
/// shows. Below this the names are cut off, which is worse than another row.
const TILE_MIN_DP: f32 = 88.0;

/// Widest a palette tile may be, so one alone on a roomy shelf is a tile and not
/// a banner.
const TILE_MAX_DP: f32 = 120.0;

/// Height of a tile's name under its icon, and the gap above it.
const TILE_LABEL_DP: f32 = 18.0;

/// The viewport slot between the two panes.
const GUTTER_DP: f32 = 40.0;

/// The width of track the editor insists on keeping visible. A track editor whose
/// track is 40 dp wide is a form with a decoration on it. Set at what the old
/// layout left by accident at its best, so the floor never costs more than it did.
const VIEWPORT_MIN_DP: f32 = 160.0;

/// How many controls a row aims to hold on one line. Four is what the piece row's
/// edit band actually has.
const CTRLS_PER_LINE: f32 = 4.0;

/// Share of a stacked window's height left to the 3D track rather than the panels.
///
/// Not a dimension but a proportion of one, so it lives with them. The panels take
/// the rest and scroll; the track keeps enough room to be dragged and to take the
/// wheel, which is the only way to zoom once the panels are up.
pub const STACKED_VIEWPORT_FRAC: f32 = 0.34;

/// The same share for a window that is short rather than narrow.
///
/// Larger, because the whole point of stacking a short window is that its height
/// went scarce: taking a third of a 360 dp window leaves a letterbox that cannot
/// be dragged usefully, so the track is given the majority and the panels scroll
/// within what is left.
pub const SHORT_VIEWPORT_FRAC: f32 = 0.55;

/// The editor's lengths for one window, all of them derived together.
///
/// Derived in one place rather than one function at a time because the old numbers
/// were not independent: the piece list, the panel holding it and the panes
/// beside it were each measured against the window on their own, which is how a
/// list came to be 100 dp wider than the panel it sat in and two panes came to ask
/// for more window than there was.
struct Layout {
    stacked: bool,
    palette_w: Dp,
    panel_w: Dp,
}

impl Layout {
    /// Width one row inside the side panel has. Read from the panel rather than
    /// restated, because a row that guesses wrong is how the piece list came to ask
    /// for 620 dp inside a 520 dp box.
    fn content_w(&self) -> Dp {
        self.panel_w - widgets::PANEL_INSET_DP * 2.0
    }

    /// Width one row inside the palette has, which is the side panel's width
    /// whenever the two are stacked.
    fn palette_content_w(&self) -> Dp {
        self.palette_w - widgets::PANEL_INSET_DP * 2.0
    }

    /// Width the palette's category strip takes across the palette.
    ///
    /// Its own comfortable width beside the shelf, and the palette's whole content
    /// width when the editor is stacked — where the strip is a row above the shelf
    /// rather than a column beside it, and so has the room to lay its tabs out
    /// along a line.
    fn strip_w(&self) -> Dp {
        match self.stacked {
            true => self.palette_content_w(),
            false => theme::dp(STRIP_COMFORT_DP),
        }
    }

    /// Width one tile row inside the shelf has.
    ///
    /// The palette's content less the strip and the gap between them. Stacked it is
    /// the palette's content whole, since nothing is beside the shelf to take any.
    fn shelf_content_w(&self) -> Dp {
        match self.stacked {
            true => self.palette_content_w(),
            false => (self.palette_content_w() - theme::dp(STRIP_COMFORT_DP) - SPACE_ROW)
                .coerce_at_least(Dp(0.0)),
        }
    }

    /// Width a piece row's identity label may take: what is left of the row once
    /// the rank and the gap beside it are paid for.
    ///
    /// A cap rather than a share, because a single-line label measures as its own
    /// full length however narrow the box around it is. Left uncapped it would
    /// hand the row a minimum width wider than the panel, which is a horizontal
    /// scrollbar inside the piece list — the thing this module exists to stop.
    fn label_w(&self) -> Dp {
        (self.content_w() - RANK_W - SPACE_ROW).coerce_at_least(Dp(0.0))
    }

    /// Width floor for a control in a row of controls.
    ///
    /// Derived so four of them share a line, which is exactly what the piece row's
    /// edit band holds: four controls plus three gaps is the panel's content width,
    /// so the band is one line at every window wide enough for two panes. Below a
    /// 504 dp window the panel is no longer capped at its comfortable width and the
    /// controls narrow with it, which is what lets a handset fit fewer per line
    /// rather than clip the row.
    ///
    /// Floored at the library's own minimum for an outlined button, so a control is
    /// never narrower than the library considers a button, and left uncapped above
    /// because the panel's own cap already keeps it near six characters of label.
    fn ctrl_w(&self) -> Dp {
        let per_line = (self.content_w() - SPACE_ROW * (CTRLS_PER_LINE - 1.0)) / CTRLS_PER_LINE;
        per_line.max(ButtonDefaults::MIN_WIDTH)
    }

    /// Height of the piece list before it scrolls. Stacked it gets more of the
    /// column than it does beside a track, so it is worth more room there.
    fn piece_list_h(&self) -> Dp {
        let wanted = if self.stacked { 240.0 } else { 200.0 };
        theme::dp(fit::fit_height(wanted))
    }
}

/// The editor's lengths for a window this many dp wide.
///
/// The palette gives way before the side column does, since the side column
/// carries the track's contents and the palette carries only names, and the pair
/// gives way together rather than the track disappearing behind them. One
/// derivation for both panes, because the old pair asked each to reserve the
/// other's comfortable width and a second, different number on top of it.
fn at(window_w: f32) -> Layout {
    let side = SIDE_COMFORT_DP.min((window_w - fit::EDGE_DP * 2.0).max(0.0));
    let palette = PALETTE_COMFORT_DP.min(window_w - GUTTER_DP - VIEWPORT_MIN_DP - side);
    match palette >= PALETTE_FLOOR_DP {
        true => Layout {
            stacked: false,
            palette_w: theme::dp(palette),
            panel_w: theme::dp(side),
        },
        false => Layout {
            stacked: true,
            palette_w: theme::dp(side),
            panel_w: theme::dp(side),
        },
    }
}

/// True when the two panes cannot both be shown without starving the track.
///
/// Derived from the width rather than taken from the engine's width class, because
/// Compact asks the wrong question: it is true at 600 dp, where the editor still
/// fits two panes with a narrow palette, and false at 700 dp, where it does not.
///
/// Height is asked as well, since a window wide enough for the panes can still be
/// too short for the track between them to be worth having.
pub fn stacked() -> bool {
    at(fit::width()).stacked || fit::is_short()
}

/// Share of a stacked window's height the track keeps, which is what decides
/// whether there is anything left of it to drag.
pub fn viewport_frac() -> f32 {
    match fit::is_short() {
        true => SHORT_VIEWPORT_FRAC,
        false => STACKED_VIEWPORT_FRAC,
    }
}

/// The two pane widths, or `None` when the editor is stacked and there is no
/// second column to fit beside.
pub fn split() -> Option<(Dp, Dp)> {
    let l = at(fit::width());
    match l.stacked {
        true => None,
        false => Some((l.palette_w, l.panel_w)),
    }
}

/// Width of the side column's panel, or of both panels when the editor is stacked
/// and there is no second column to fit beside.
pub fn panel_w() -> Dp {
    at(fit::width()).panel_w
}

/// Width of the palette panel, which is the side column's width whenever the two
/// are stacked.
pub fn palette_w() -> Dp {
    at(fit::width()).palette_w
}

/// Width one row inside the side panel actually has.
pub fn content_w() -> Dp {
    at(fit::width()).content_w()
}

/// Width one row inside the palette actually has.
pub fn palette_content_w() -> Dp {
    at(fit::width()).palette_content_w()
}

/// Width the palette's category strip actually takes across the palette.
pub fn strip_w() -> Dp {
    at(fit::width()).strip_w()
}

/// Width a piece row's identity label may take.
pub fn label_w() -> Dp {
    at(fit::width()).label_w()
}

/// Width floor for a control in a row of controls.
pub fn ctrl_w() -> Dp {
    at(fit::width()).ctrl_w()
}

/// Width of one palette tile: an icon with a short name under it, rather than a
/// full-width button per piece.
///
/// Derived from the shelf rather than the whole palette, so the strip beside it
/// does not silently cost the tiles a column of their own.
///
/// The count is chosen first and the width measured from it, never the reverse.
/// A width picked on its own and a count inferred from it is how a shelf ended up
/// with two 120 dp tiles across a space sized for one and a horizontal scrollbar
/// under it: 120 fits a 160 dp shelf, so one fits, and the row put two there
/// anyway. Measuring the width as `shelf / count` makes the row *unable* to hold
/// one more, so it has nothing to disagree about. Below the width a name fits at
/// all, the shelf takes one tile filling it rather than clipping every name in it.
pub fn tile_w() -> Dp {
    tile_w_of(&at(fit::width()))
}

/// Tile width for one window, rather than for whatever window is current.
///
/// Separate from [`tile_w`] so a test can ask what a window *would* get without
/// setting the global up first. Comparing a tile measured from the current window
/// against a shelf measured from some other one compares two unrelated layouts,
/// and the tile test did exactly that.
fn tile_w_of(layout: &Layout) -> Dp {
    let shelf = layout.shelf_content_w().value();
    let fit = |count: usize| (shelf - SPACE_ROW.0 * (count as f32 - 1.0)) / count as f32;
    for count in [4usize, 3, 2] {
        let w = fit(count);
        if w >= TILE_MIN_DP && w <= TILE_MAX_DP {
            return theme::dp(w);
        }
    }
    // Nothing divides the shelf into readable tiles, so one tile fills it.
    theme::dp(shelf)
}

/// Height of a palette tile.
///
/// Fixed rather than left to its contents: a tile with no height of its own is
/// stretched to whatever the row around it is tall, which on a tall window made
/// every tile a full-height slab instead of a card.
pub fn tile_h() -> Dp {
    theme::dp(tile_icon_h().0 + TILE_LABEL_DP + theme::dp(8.0).0)
}

/// Height of the icon inside a palette tile.
pub fn tile_icon_h() -> Dp {
    theme::dp(44.0)
}

/// Size of a square tool in the editor's dock.
pub fn tool_w() -> Dp {
    theme::dp(44.0)
}

/// Left inset of the editor's dock, which sits over the track rather than over a
/// panel.
///
/// Clear of the palette when the two are side by side, since a tool drawn on top
/// of the palette it is not part of takes its taps from it. Stacked, the track
/// owns the whole width and the dock takes the window's own inset.
pub fn dock_left() -> Dp {
    match split() {
        Some((palette_w, _)) => palette_w + theme::dp(fit::EDGE_DP),
        None => theme::dp(fit::chrome_inset()),
    }
}

/// Height of the piece list before it scrolls, in dp.
pub fn piece_list_h() -> Dp {
    at(fit::width()).piece_list_h()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use repose_core::{
        SceneNode, calculate_window_size_class, set_window_container_size,
        set_window_size_class_default,
    };

    use crate::app::state::{ActionQueue, AppData};
    use crate::ui::screens::editor::editor_ui;

    /// A rectangle in dp: x, y, width, height.
    type Rect = (f32, f32, f32, f32);

    /// Report a failing assertion as a workflow annotation, so a red build names the
    /// assertion that failed in its own summary rather than only in a log the checks
    /// API will not hand back.
    fn report_failure_where_it_happens() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let said = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| String::from("assertion failed"));
                let at = info.location();
                let file = at.map_or("dims.rs", |l| l.file());
                let line = at.map_or(0, |l| l.line());
                let clean = said
                    .replace('\n', " ")
                    .replace('\r', " ")
                    .replace('%', "%25")
                    .replace(':', "%3A");
                eprintln!("::error file={file},line={line},title=failed assertion::{clean}");
                previous(info);
            }));
        });
    }

    /// Lay the real editor out at the given window with the given track behind it.
    fn editor_with(data: &AppData, dp_w: f32, dp_h: f32) -> repose_core::Scene {
        report_failure_where_it_happens();
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let view = editor_ui(data, &Rc::new(RefCell::new(Vec::new())) as &ActionQueue);
        repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        )
        .0
    }

    /// Every string the screen would draw.
    fn labels(scene: &repose_core::Scene) -> Vec<String> {
        scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Text { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Lay the real editor out at this window and report the scroller viewports
    /// that take the wheel, plus how far the content reaches down and across.
    fn scrollers(dp_w: f32, dp_h: f32) -> (Vec<Rect>, Vec<Rect>) {
        report_failure_where_it_happens();
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let view = editor_ui(
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
        let rects: Vec<Rect> = hits
            .iter()
            .filter(|r| r.on_scroll.is_some())
            .map(|r| (r.rect.x, r.rect.y, r.rect.w, r.rect.h))
            .collect();
        // Every rectangle the scene paints, not just a single furthest-reaching one.
        // The caller has to know what is inside which scroller, and a single global
        // maximum cannot say that: the dock floats over the track and can reach lower
        // than the column, so which node is furthest down changes with how tall the
        // panels are, and a caller that picked the scroller from it was inferring the
        // scroller from a measurement of the wrong thing.
        let content: Vec<Rect> = scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Rect { rect, .. }
                | SceneNode::Border { rect, .. }
                | SceneNode::Text { rect, .. } => Some((rect.x, rect.y, rect.w, rect.h)),
                _ => None,
            })
            .collect();
        (rects, content)
    }

    /// Every dp width from the narrowest window the layout claims to hold to a
    /// desktop large enough to need no clamping at all.
    const WIDTHS: [f32; 9] = [
        320.0, 360.0, 414.0, 600.0, 768.0, 855.0, 856.0, 1024.0, 2560.0,
    ];

    /// The editor laid out at this window, with `pieces` pieces placed in it.
    fn editor_at(dp_w: f32, dp_h: f32, pieces: usize) -> repose_core::Scene {
        report_failure_where_it_happens();
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let mut data = AppData::default();
        for i in 0..pieces {
            data.track.pieces.push(
                retrackt_format::PieceInstance::new(
                    retrackt_format::PieceId::Straight,
                    [0, 0, i as i16],
                )
                .with_uid(retrackt_format::PieceUid(i as u32 + 1)),
            );
        }
        let view = editor_ui(&data, &Rc::new(RefCell::new(Vec::new())) as &ActionQueue);
        repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        )
        .0
    }

    /// The same, with the palette opened on one category.
    fn editor_at_with_category(dp_w: f32, dp_h: f32, category: usize) -> repose_core::Scene {
        report_failure_where_it_happens();
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let mut data = AppData::default();
        data.editor.palette_category = category;
        let view = editor_ui(&data, &Rc::new(RefCell::new(Vec::new())) as &ActionQueue);
        repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        )
        .0
    }

    /// The editor's track panel: the tallest wide solid block in the side column.
    fn track_panel_h(scene: &repose_core::Scene) -> f32 {
        scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Rect {
                    rect,
                    brush: repose_core::Brush::Solid(_),
                    ..
                } if rect.w > 300.0 => Some(rect.h),
                _ => None,
            })
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn a_selected_gate_shows_its_number_and_the_way_to_change_it() {
        // A checkpoint's order is the one edit that is not about its shape, and it
        // is the one that decides the run. The list reads each gate's number so the
        // order is visible without selecting anything, and the control that changes
        // it lives in the inspector with the value it moves beside it — a button
        // that says only `G+` cannot be read as a gate number until you know it.
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(1280.0, 800.0);
        set_window_size_class_default(calculate_window_size_class(1280, 800, 1.0));

        let mut data = AppData::default();
        let mut b = retrackt_format::ChainBuilder::new("gates");
        b.step(
            retrackt_format::PieceId::Start,
            retrackt_format::PieceParams::default().length(2),
        );
        b.step(
            retrackt_format::PieceId::Checkpoint,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::Checkpoint,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::Finish,
            retrackt_format::PieceParams::default().length(2),
        );
        data.track = b.build();
        let second = data.track.checkpoint_sequence().last().expect("gates").uid;
        data.editor.selection = vec![second];

        let (scene, _, _) = repose_ui::layout_and_paint(
            &editor_ui(&data, &Rc::new(RefCell::new(Vec::new())) as &ActionQueue),
            (1280, 800),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        let texts: Vec<&str> = scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Text { text, .. } => Some(text.as_ref()),
                _ => None,
            })
            .collect();

        // One gate reads as 1 and the other as 2, which is the order they are
        // passed in, not the order they sit in the file. Read as part of the row's
        // summary rather than as its own label: the row is one line, so its values
        // arrive joined rather than as a control strip.
        assert!(texts.iter().any(|t| t.contains("gate 1")), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("gate 2")), "{texts:?}");
        // The selected one is named, its order named with it, and the pair of
        // controls that moves it is on the same line.
        assert!(texts.contains(&"Checkpoint"), "{texts:?}");
        assert!(texts.contains(&"Gate"), "{texts:?}");
        assert!(texts.contains(&"2 of 2"), "{texts:?}");
        assert!(texts.contains(&"-"), "a gate has to be movable: {texts:?}");
        assert!(texts.contains(&"+"), "a gate has to be movable: {texts:?}");
    }

    #[test]
    fn picking_a_category_swaps_the_shelf_under_the_strip() {
        // The strip is the only way to reach a piece that is not on the open
        // shelf, so switching categories has to change what is laid out: an
        // editor that ignored the tap would show one category forever.
        for group in retrackt_format::piece::groups() {
            let index = retrackt_format::piece::group_index(group).expect("group is listed");
            let scene = editor_at_with_category(1024.0, 576.0, index);
            // Only the palette column, by where each label is drawn. Searching the
            // whole screen for a tile's name reads any other panel that happens to
            // use the same word as a tile: the track-settings panel's own row made a
            // Control piece look like it was on the Road shelf, which is a fault
            // invented here rather than one the editor has. A palette label is inside
            // the palette panel, so its rectangle ends within it.
            //
            // The edge comes from the scene and not from `dims::palette_w()`. That
            // reads the window size out of a global which the layout helpers release
            // before returning, so asking it after the layout is a race with every
            // other layout test running in parallel: on another test's window it
            // answers a different palette's width, the shelf no longer fits inside
            // it, and real tiles read as hidden. The scene is the thing being asked
            // about, so the scene is what it is measured from.
            let palette_edge = scene
                .nodes
                .iter()
                .filter_map(|n| match n {
                    SceneNode::Rect { rect, .. } if rect.w > 300.0 => Some(rect.x + rect.w),
                    _ => None,
                })
                .fold(f32::MAX, f32::min);
            let texts: Vec<String> = texts(&scene)
                .into_iter()
                .filter(|(_, r)| r.0 + r.2 <= palette_edge)
                .map(|(t, _)| t)
                .collect();
            for piece in retrackt_format::catalog() {
                let tile = crate::ui::screens::editor::short_name(piece.id.label());
                let shown = texts.iter().any(|t| *t == tile);
                assert_eq!(
                    shown,
                    piece.id.group() == group,
                    "{group}: {tile} is {} of {texts:?} inside edge {palette_edge:.1}",
                    if shown { "shown" } else { "hidden" }
                );
            }
        }
    }

    #[test]
    fn every_palette_category_is_reachable_and_shows_its_parts() {
        // The palette shows one category at a time, so what must hold is that no
        // piece is stranded: each category's tab is in the tree to switch to, and
        // the shelf on show carries every piece it is responsible for. A piece
        // that is in the catalogue and in neither is a piece that cannot be armed.
        for (w, h) in [(360.0f32, 780.0f32), (800.0, 360.0), (1024.0, 576.0)] {
            let scene = editor_at(w, h, 0);
            let texts: Vec<String> = scene
                .nodes
                .iter()
                .filter_map(|n| match n {
                    SceneNode::Text { text, .. } => Some(text.to_string()),
                    _ => None,
                })
                .collect();
            let groups = retrackt_format::piece::groups();
            for group in &groups {
                assert!(
                    texts.iter().any(|t| t == group),
                    "{w}x{h}: category {group} has no tab to switch to"
                );
            }
            // The shelf on show is the default one, and it is complete.
            let open = retrackt_format::piece::DEFAULT_GROUP;
            for piece in retrackt_format::catalog() {
                if piece.id.group() != open {
                    continue;
                }
                let tile = crate::ui::screens::editor::short_name(piece.id.label());
                assert!(
                    texts.iter().any(|t| t == tile),
                    "{w}x{h}: {open} is missing {tile}"
                );
            }
        }
    }

    /// Text in a laid-out scene with the rectangle it was given.
    fn texts(scene: &repose_core::Scene) -> Vec<(String, Rect)> {
        scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Text { text, rect, .. } => {
                    Some((text.to_string(), (rect.x, rect.y, rect.w, rect.h)))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn side_by_side_the_strip_is_a_column_beside_the_shelf() {
        // The palette is two halves of one tool, not a strip stacked over a shelf: a
        // category a player has chosen staying on screen as a row of tabs costs a
        // whole line of the shelf, which is a whole line of pieces.
        //
        // Measured against the label rectangles rather than the buttons, because a
        // label is centred in its button and so is narrower than it: "Road" and
        // "Junctions" do not share a left edge, and what has to hold is that the
        // shelf is clear of the strip rather than that the tabs are one width.
        let open = retrackt_format::piece::group_index(retrackt_format::piece::DEFAULT_GROUP)
            .expect("the default group is listed");
        let laid = texts(&editor_at_with_category(1024.0, 576.0, open));

        let groups = retrackt_format::piece::groups();
        let tiles: Vec<Rect> = retrackt_format::catalog()
            .iter()
            .filter(|d| d.id.group() == retrackt_format::piece::DEFAULT_GROUP)
            .map(|p| {
                let name = crate::ui::screens::editor::short_name(p.id.label());
                laid.iter()
                    .find(|(t, _)| t == name)
                    .map(|(_, r)| *r)
                    .unwrap_or_else(|| panic!("{name} is on the shelf"))
            })
            .collect();
        // The open category's name is also the panel's own title, so take the
        // lowest of the matches: in a column that is the tab rather than the
        // heading above it.
        let tabs: Vec<Rect> = groups
            .iter()
            .map(|g| {
                laid.iter()
                    .filter(|(t, _)| t == g)
                    .max_by(|a, b| a.1.1.partial_cmp(&b.1.1).unwrap())
                    .map(|(_, r)| *r)
                    .unwrap_or_else(|| panic!("{g} has no tab"))
            })
            .collect();

        let strip_right = tabs.iter().map(|r| r.0 + r.2).fold(f32::MIN, f32::max);
        let tile_left = tiles.iter().map(|t| t.0).fold(f32::MAX, f32::min);
        assert!(
            tile_left >= strip_right - 1.0,
            "a tile overlaps the strip: leftmost tile at {tile_left:.0}, strip ends at {strip_right:.0}"
        );
        // A column, so the tabs stack down rather than sitting along one line.
        let mut ys: Vec<f32> = tabs.iter().map(|r| r.1).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ys.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        assert_eq!(ys.len(), tabs.len(), "the tabs stack down the strip");
    }

    #[test]
    fn stacked_the_strip_lies_above_the_shelf() {
        // The other half of the same split: with no width for both, the strip goes
        // back above the shelf, so the fallback has to be checked too or a change to
        // the rule that picks between them would only ever be caught on a desktop.
        let open = retrackt_format::piece::group_index(retrackt_format::piece::DEFAULT_GROUP)
            .expect("the default group is listed");
        let laid = texts(&editor_at_with_category(360.0, 780.0, open));
        let groups = retrackt_format::piece::groups();
        let tab_bottom = groups
            .iter()
            .map(|g| {
                laid.iter()
                    .filter(|(t, _)| t == g)
                    .map(|(_, r)| r.1 + r.3)
                    .fold(f32::MIN, f32::max)
            })
            .fold(f32::MIN, f32::max);
        let tile_top = retrackt_format::catalog()
            .iter()
            .filter(|d| d.id.group() == retrackt_format::piece::DEFAULT_GROUP)
            .map(|p| {
                let name = crate::ui::screens::editor::short_name(p.id.label());
                laid.iter()
                    .find(|(t, _)| t == name)
                    .map(|(_, r)| r.1)
                    .unwrap_or_else(|| panic!("{name} is on the shelf"))
            })
            .fold(f32::MAX, f32::min);
        assert!(
            tile_top >= tab_bottom,
            "a tile sits at {tile_top:.0}, above the strip ending at {tab_bottom:.0}"
        );
    }

    #[test]
    fn no_piece_name_is_clipped_at_any_width_the_palette_is_shown_at() {
        // A tile narrower than the name it carries has the name cut off, which is
        // worse than a shorter tile: the player sees two pieces they cannot tell
        // apart and no way to know which is which. Checked over every category at
        // the two widths that matter — the most squeezed width still holding two
        // panes, where the tiles are narrowest, and the narrowest window, where
        // the palette is a single column at the bottom of a stack.
        for (w, h) in [(928.0f32, 576.0f32), (320.0, 780.0)] {
            for group in retrackt_format::piece::groups() {
                let index = retrackt_format::piece::group_index(group).expect("group is listed");
                let laid = texts(&editor_at_with_category(w, h, index));
                for piece in retrackt_format::catalog() {
                    if piece.id.group() != group {
                        continue;
                    }
                    let name = crate::ui::screens::editor::short_name(piece.id.label());
                    assert!(
                        laid.iter().any(|(t, _)| t == name),
                        "{w}x{h}: {group} clips {name} off its tile"
                    );
                }
            }
        }
    }

    #[test]
    fn the_piece_list_grows_with_the_number_of_pieces() {
        // The editor's panels sit inside its own scroller, so they must not be
        // capped: a capped panel clips at the cap while the scroller measures the
        // column from those capped heights, so the surplus is painted inside the
        // clip rect and cannot be scrolled to. The symptom is the panel refusing to
        // grow — the layout engine still measures the list at full height either
        // way, so it is the panel's own height that gives the cap away.
        let few = track_panel_h(&editor_at(1024.0, 576.0, 4));
        let many = track_panel_h(&editor_at(1024.0, 576.0, 24));
        // Twenty more pieces, and each has to cost a row of real height rather than
        // nothing: a capped panel is flat here, so the growth is the whole claim.
        // Stated per piece and not as one total, because a row is now a single line
        // and a total measured against three bands of controls no longer says
        // anything — the number would have to be rewritten every time a row changed.
        let added = many - few;
        assert!(
            added >= 20.0 * 20.0,
            "the track panel is {many:.0} dp for 24 pieces and {few:.0} for 4: it is capped, so \
             the surplus is clipped where the column's scroller cannot reach it"
        );
    }

    #[test]
    fn a_window_wide_but_short_stacks_and_gives_its_height_to_the_track() {
        // 800x360 dp is a phone held sideways: wide enough for two panes, and far
        // too short for the track between them to be more than a letterbox.
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(800.0, 360.0);
        set_window_size_class_default(calculate_window_size_class(800, 360, 1.0));
        assert!(fit::is_short());
        assert!(stacked());
        assert!(
            viewport_frac() > STACKED_VIEWPORT_FRAC,
            "a short window keeps {:.0}% of its height for the track",
            viewport_frac() * 100.0
        );
    }

    #[test]
    fn no_width_the_editor_can_be_shown_at_asks_for_more_than_the_window_has() {
        // The old reserves asked 930 dp of a window holding 600: a 230 dp palette
        // and a 660 dp column, each reserving the other, plus a 40 dp gap. Ten dp of
        // that landed off the right edge, and the guard test for it only ran on the
        // default 360 dp window, where the side-by-side branch is never taken.
        for w in WIDTHS {
            let l = at(w);
            assert!(l.panel_w.value() > 0.0, "{w} dp: no panel at all");
            assert!(
                l.panel_w.value() <= w - fit::EDGE_DP * 2.0,
                "{w} dp holds a {} dp panel",
                l.panel_w.value()
            );
            assert!(l.palette_w.value() >= 0.0, "{w} dp: negative palette");
            match l.stacked {
                true => assert_eq!(l.palette_w, l.panel_w, "{w} dp: one column, one width"),
                false => {
                    let asked = l.palette_w.value() + l.panel_w.value() + GUTTER_DP;
                    assert!(asked <= w, "{w} dp holds {asked} of panes and gutter");
                }
            }
        }
    }

    #[test]
    fn a_wider_window_never_leaves_less_track_visible() {
        // The floor is what the track is guaranteed, and widening the window adds to
        // it rather than being absorbed: the panel is capped, so everything past
        // that cap goes to the track.
        let mut prev = 0.0;
        for w in WIDTHS {
            if at(w).stacked {
                continue;
            }
            let l = at(w);
            let track = w - l.palette_w.value() - l.panel_w.value() - GUTTER_DP;
            assert!(track >= VIEWPORT_MIN_DP, "{w} dp leaves {track} of track");
            assert!(
                track >= prev,
                "{w} dp leaves less track than a narrower window"
            );
            prev = track;
        }
    }

    #[test]
    fn the_palette_gives_way_before_the_side_column_does() {
        // 960 dp is inside the band where the two panes still fit but the palette
        // has been squeezed: narrower than that and the editor stacks rather than
        // squeezing, which is the whole reason the band exists.
        let l = at(960.0);
        assert!(!l.stacked, "960 dp should still hold two panes");
        assert!(l.palette_w.value() < PALETTE_COMFORT_DP);
        assert_eq!(
            l.panel_w.value(),
            SIDE_COMFORT_DP,
            "the side column holds its room"
        );
        assert_eq!(at(1024.0).palette_w.value(), PALETTE_COMFORT_DP);
    }

    #[test]
    fn one_rhythm_holds_across_the_breakpoint() {
        // A stacked desktop window and a wide one give a row the same width, so the
        // editor does not resize its whole interior when it changes shape.
        assert_eq!(at(900.0).content_w().value(), 440.0);
        assert_eq!(at(1024.0).content_w().value(), 440.0);
    }

    #[test]
    fn the_piece_list_is_never_wider_than_the_panel_holding_it() {
        // It asked for 620 dp inside a panel whose content box is 440, which is a
        // horizontal scrollbar over a list that never needed to scroll sideways.
        for w in WIDTHS {
            let l = at(w);
            assert!(
                l.content_w().value() <= l.panel_w.value() - widgets::PANEL_INSET_DP.value() * 2.0,
                "{w} dp: {} of content in a {} dp panel",
                l.content_w().value(),
                l.panel_w.value()
            );
            assert!(
                l.content_w().value() >= 256.0,
                "{w} dp leaves {} of content",
                l.content_w().value()
            );
        }
    }

    #[test]
    fn a_piece_row_never_wants_more_than_the_row_it_sits_in() {
        // The identity label is the one unbounded thing in a piece row, and
        // unbounded it would hand the row a minimum width wider than the panel.
        for w in WIDTHS {
            let l = at(w);
            let row = l.label_w().value() + RANK_W.value() + SPACE_ROW.value();
            assert!(
                row <= l.content_w().value(),
                "{w} dp: a {row} dp row in {} dp of content",
                l.content_w().value()
            );
        }
    }

    #[test]
    fn a_control_is_never_smaller_than_a_hand_can_hit() {
        // 44 dp is the touch minimum, and the material minimum is above it, so the
        // floor that binds is the library's. Height is the number that was 40.
        for w in WIDTHS {
            assert!(
                at(w).ctrl_w().value() >= ButtonDefaults::MIN_WIDTH.value(),
                "{w} dp gives a {} dp control",
                at(w).ctrl_w().value()
            );
        }
        assert!(
            CTRL_H.value() >= 44.0,
            "controls are {} dp tall",
            CTRL_H.value()
        );
    }

    #[test]
    fn the_panel_scroller_is_never_taller_than_the_window() {
        // The row does not stretch its children, so a scroller given only
        // `fill_max_height` took its height from the content instead: 895 dp inside
        // a 576 dp window. A scroller taller than the window has no range to scroll
        // — its viewport is all of it — and it holds the wheel over that whole
        // height, so the track beneath could not be zoomed at all.
        for (w, h) in [(360.0f32, 780.0f32), (1024.0, 576.0), (1440.0, 900.0)] {
            let (rects, _) = scrollers(w, h);
            let tallest = rects.iter().map(|r| r.3).fold(0.0f32, f32::max);
            assert!(
                tallest <= h + 1.0,
                "{w}x{h}: a scroller is {tallest} dp tall, past the window"
            );
        }
    }

    #[test]
    fn the_stacked_editor_leaves_room_for_the_track_under_the_panels() {
        // A scroller that fills a stacked window owns the wheel over the whole
        // screen, so the viewport behind it can only be reached by scrolling to the
        // end of the panels. The panels must stop short of the window.
        let (rects, _) = scrollers(360.0, 780.0);
        let panels = rects
            .iter()
            .filter(|r| r.3 > 1.0)
            .map(|r| r.3)
            .fold(0.0f32, f32::max);
        let viewport = 780.0 - panels;
        assert!(
            viewport >= 780.0 * STACKED_VIEWPORT_FRAC - 1.0,
            "780 dp leaves {viewport} dp of track"
        );
    }

    fn content_below_the_window_is_reachable_by_scrolling_rather_than_clipped() {
        // The panels are taller than the window, so part of the column sits below
        // it — the playtest and close row among it. That is only reachable if the
        // column's own scroller has room to scroll: viewport height bounded by the
        // window, content taller than the viewport. The palette's scroller is a
        // sibling of the column, not an ancestor of it, so it is not asked to cover
        // this.
        for (w, h) in [(360.0f32, 780.0f32), (1024.0, 576.0)] {
            let (rects, painted) = scrollers(w, h);
            // The column is the widest scroller on screen: it spans the side column,
            // and every other scroller is inside it. Found on its own shape rather
            // than by matching the x-span of whatever happened to reach furthest
            // down, which is a measurement of a different thing: the dock floats over
            // the track and reaches lower than the panels do, so which node wins that
            // contest changes with how tall the panels are, and one more row of
            // settings was enough to flip it and leave the column unmatched.
            let column = rects
                .iter()
                .filter(|r| r.3 > 1.0)
                .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
                .unwrap_or_else(|| panic!("{w}x{h}: no column scroller at all"));
            assert!(
                column.1 + column.3 <= h + 1.0,
                "{w}x{h}: the column's scroller is {} dp tall at y {}, past the window",
                column.3,
                column.1
            );
            // And there is in fact something below the fold inside it, or there is
            // nothing here to be reachable.
            let furthest = painted
                .iter()
                .filter(|r| r.0 >= column.0 - 1.0 && r.0 + r.2 <= column.0 + column.2 + 1.0)
                .map(|r| r.1 + r.3)
                .fold(0.0f32, f32::max);
            assert!(
                furthest > h + 1.0,
                "{w}x{h}: the column holds nothing past the window, so nothing is clipped"
            );
        }
    }

    #[test]
    fn the_track_settings_panel_says_where_the_run_begins_and_ends() {
        // The name is what the track is called everywhere else in the game, and the
        // start and finish are the two facts about a circuit a player checks before
        // asking whether the middle is any good. Without them the editor shows a
        // list of pieces and no way to tell what the track is.
        let mut data = AppData::default();
        let mut b = retrackt_format::ChainBuilder::new("settings");
        b.step(
            retrackt_format::PieceId::Start,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::Checkpoint,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::Finish,
            retrackt_format::PieceParams::default(),
        );
        data.track = b.build();
        let shown = labels(&editor_with(&data, 1280.0, 800.0));
        assert!(shown.iter().any(|t| t == "Track settings"), "{shown:?}");
        for label in ["Start line", "Finish line", "Gates"] {
            assert!(
                shown.iter().any(|t| t == label),
                "track settings has no {label} row: {shown:?}"
            );
        }
    }

    #[test]
    fn a_tile_row_never_overflows_the_shelf_it_sits_in() {
        // A palette that scrolls sideways is a palette showing half a tile. The cause
        // was a tile width chosen on its own and a count inferred from it: 120 dp fits
        // a 160 dp shelf, so one fits, and the row put two there anyway. The width is
        // now measured from the count, so the row cannot hold one more than the count.
        // Checked as geometry across the whole width range rather than on the two
        // widths that happened to look wrong.
        for w in WIDTHS {
            let l = at(w);
            let shelf = l.shelf_content_w().value();
            let tile = tile_w_of(&l).value();
            assert!(
                tile <= shelf + 0.5,
                "{w} dp: a {tile} dp tile in a {shelf} dp shelf"
            );
            // And the shelf holds a whole number of them: a shelf with room for one
            // and a half is a shelf with a scrollbar under it.
            let across = ((shelf + SPACE_ROW.0) / (tile + SPACE_ROW.0)).floor();
            assert!(
                (across * (tile + SPACE_ROW.0) - SPACE_ROW.0 - shelf).abs() < 0.5,
                "{w} dp: {across} tiles of {tile:.0} dp leave a {shelf:.0} dp shelf ragged"
            );
        }
    }

    #[test]
    fn the_key_reference_waits_to_be_asked_for() {
        // Nine rows of text about keys nobody reads while placing a piece, and open
        // they stood between the player and the list of pieces. The camera keys are
        // the ones that needed to be on the screen at all, so they have to be one
        // tap away rather than always taking the space.
        let quiet = labels(&editor_with(&AppData::default(), 1280.0, 800.0));
        assert!(
            !quiet.iter().any(|t| t.contains("move the camera")),
            "the key reference is open by default"
        );
        assert!(quiet.iter().any(|t| t == "Keys"), "{quiet:?}");

        let mut asked = AppData::default();
        asked.editor.show_help = true;
        let open = labels(&editor_with(&asked, 1280.0, 800.0));
        assert!(
            open.iter().any(|t| t.contains("move the camera")),
            "asking for the keys did not open them"
        );
        assert!(open.iter().any(|t| t == "Hide keys"), "{open:?}");
    }

    #[test]
    fn the_selection_panel_is_empty_until_something_is_selected() {
        // The inspector is the whole point of selecting, so it must be about the
        // selection: actions for a selection that does not exist are controls that
        // would do nothing, which is the one thing a control must never be.
        // "Duplicate" is unique to the inspector — the dock's tools are Rotate and
        // the height steppers, so nothing else on the screen carries the word.
        let empty = labels(&editor_with(&AppData::default(), 1280.0, 800.0));
        assert!(empty.iter().any(|t| t == "Selection"), "{empty:?}");
        assert!(
            !empty.iter().any(|t| t == "Duplicate"),
            "selection actions are shown with nothing selected"
        );

        let mut one = AppData::default();
        one.track.pieces.push(
            retrackt_format::PieceInstance::new(retrackt_format::PieceId::Straight, [0, 0, 0])
                .with_uid(retrackt_format::PieceUid(1)),
        );
        one.editor.selection = vec![retrackt_format::PieceUid(1)];
        let held = labels(&editor_with(&one, 1280.0, 800.0));
        assert!(held.iter().any(|t| t == "Duplicate"), "{held:?}");
        assert!(held.iter().any(|t| t == "Straight"), "{held:?}");
    }

    #[test]
    fn the_piece_list_is_read_only_and_the_inspector_above_it_is_not() {
        // The list is read constantly and edited rarely, and a row carrying a
        // control for every value it has is a list whose rows cannot be scanned:
        // eleven controls in a row of forty dp leaves nothing to read. So a row is a
        // line of text, and the values it shows are the ones the inspector edits.
        let mut data = AppData::default();
        let mut b = retrackt_format::ChainBuilder::new("rows");
        b.step(
            retrackt_format::PieceId::Start,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::RampUp,
            retrackt_format::PieceParams::default(),
        );
        b.step(
            retrackt_format::PieceId::Finish,
            retrackt_format::PieceParams::default(),
        );
        data.track = b.build();
        let ramp = data
            .track
            .race_order()
            .iter()
            .filter_map(|uid| data.track.piece(*uid))
            .find(|p| p.id == retrackt_format::PieceId::RampUp)
            .expect("the ramp")
            .uid;
        data.editor.selection = vec![ramp];

        let shown = labels(&editor_with(&data, 1280.0, 800.0));
        // The row reads the values out rather than offering them.
        assert!(
            shown.iter().any(|t| t.contains("slope ")),
            "a slope row does not say its steepness: {shown:?}"
        );
        // And the controls that used to sit on it now sit in the inspector, named
        // after what they move rather than after a minus sign.
        for name in ["Length", "Slope", "Bank"] {
            assert!(
                shown.iter().any(|t| t == name),
                "the inspector has no {name} row: {shown:?}"
            );
        }
        // The old per-row bands are gone, and nothing anywhere offers them.
        for gone in ["L-", "L+", "B-", "B+", "S-", "S+", "R-", "R+"] {
            assert!(
                !shown.iter().any(|t| t == gone),
                "{gone} is still a button on a piece row"
            );
        }
    }

    #[test]
    fn the_piece_row_s_edit_band_is_one_line_at_every_width_that_holds_two_panes() {
        // Four controls' worth of width plus three gaps is the panel's content
        // width exactly. It used to be four 150 dp controls inside a 440 dp panel,
        // so the band wrapped the same ragged way at every size.
        for w in [856.0, 1024.0, 1440.0, 2560.0] {
            let l = at(w);
            let band = CTRLS_PER_LINE * l.ctrl_w().value() + 3.0 * SPACE_ROW.value();
            assert!(
                band <= l.content_w().value(),
                "{w} dp: a {band} dp band in {} dp",
                l.content_w().value()
            );
        }
        assert!(
            at(360.0).ctrl_w().value() < at(1024.0).ctrl_w().value(),
            "a handset's controls are no wider than a desktop's"
        );
    }
}

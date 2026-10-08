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

/// The palette's comfortable width: a piece name, its control's padding and
/// little else, since arming a piece is all a palette row ever does.
const PALETTE_COMFORT_DP: f32 = 224.0;

/// The narrowest the palette may be squeezed to before the editor stacks instead.
const PALETTE_FLOOR_DP: f32 = 176.0;

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
/// Derived rather than taken from the engine's width class, because Compact asks
/// the wrong question: it is true at 600 dp, where the editor still fits two panes
/// with a narrow palette, and false at 700 dp, where it does not.
pub fn stacked() -> bool {
    at(fit::width()).stacked
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

/// Width a piece row's identity label may take.
pub fn label_w() -> Dp {
    at(fit::width()).label_w()
}

/// Width floor for a control in a row of controls.
pub fn ctrl_w() -> Dp {
    at(fit::width()).ctrl_w()
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

    /// How far the content reaching furthest down reaches, and across what span.
    type Reach = (f32, f32, f32);

    /// Lay the real editor out at this window and report the scroller viewports
    /// that take the wheel, plus how far the content reaches down and across.
    fn scrollers(dp_w: f32, dp_h: f32) -> (Vec<Rect>, Reach) {
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
        // The furthest-reaching content, and the span it occupies across: that span
        // is what says which scroller, if any, can scroll to it.
        let mut over: Option<Reach> = None;
        for n in &scene.nodes {
            let rect = match n {
                SceneNode::Rect { rect, .. }
                | SceneNode::Border { rect, .. }
                | SceneNode::Text { rect, .. } => *rect,
                _ => continue,
            };
            let reaches = rect.y + rect.h;
            match over {
                Some(prev) if prev.0 >= reaches => {}
                _ => over = Some((reaches, rect.x, rect.x + rect.w)),
            }
        }
        (rects, over.unwrap_or((0.0, 0.0, 0.0)))
    }

    /// Every dp width from the narrowest window the layout claims to hold to a
    /// desktop large enough to need no clamping at all.
    const WIDTHS: [f32; 9] = [
        320.0, 360.0, 414.0, 600.0, 768.0, 855.0, 856.0, 1024.0, 2560.0,
    ];

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
        let l = at(880.0);
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

    #[test]
    fn content_below_the_window_is_reachable_by_scrolling_rather_than_clipped() {
        // The panels are taller than the window, so part of the column sits below
        // it — the playtest and close row among it. That is only reachable if a
        // scroller spans the column's width *and* has room to scroll: viewport
        // height bounded by the window, content taller than the viewport. The
        // palette's scroller is a sibling of the column, not an ancestor of it, so
        // it is not asked to cover this.
        for (w, h) in [(360.0f32, 780.0f32), (1024.0, 576.0)] {
            let (rects, (bottom, left, right)) = scrollers(w, h);
            if bottom <= h + 1.0 {
                continue;
            }
            let column = rects
                .iter()
                .find(|r| r.0 <= left + 1.0 && r.0 + r.2 >= right - 1.0 && r.3 > 1.0)
                .unwrap_or_else(|| {
                    panic!("{w}x{h}: content at x={left}..{right} is in no scroller")
                });
            assert!(
                column.3 <= h + 1.0,
                "{w}x{h}: the column's scroller is {} dp, past the window",
                column.3
            );
            // The viewport must be able to scroll the content's full height past its own
            // top: what lies `over` below the window is reachable when the
            // viewport can travel at least that far, which it can when the
            // content is no deeper than the viewport plus the shortfall.
            let content = bottom - column.1;
            assert!(
                content - column.3 >= bottom - h - 1.0,
                "{w}x{h}: content {content} dp in a {}-dp viewport cannot scroll \
                 {} dp past the window",
                column.3,
                bottom - h,
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

use repose_core::{Color, Dp, Modifier, Px, View, px_to_dp};
use repose_ui::{Box, Column, FlowRow, FlowRowConfig, Spacer, Text, TextStyle, ViewExt, ZStack};

use crate::app::input::{STICK_BASE_DP, STICK_KNOB_DP, STICK_TRAVEL_DP};
use crate::app::state::{ActionQueue, AppData, StickView, UiAct};
use crate::app::theme;
use crate::ui::fit;
use crate::ui::hud::race_hud;
use crate::ui::widgets::{accent_btn, controls_hint, ghost_btn, pusher};

pub fn race_ui(data: &AppData, actions: &ActionQueue, viewport: View) -> View {
    let mut controls_row = vec![ghost_btn("Restart", pusher(actions, UiAct::Restart))];
    if data.practice {
        controls_row.push(accent_btn(
            "Checkpoint",
            pusher(actions, UiAct::RecoverToCheckpoint),
        ));
    }
    controls_row.push(ghost_btn(
        "Quit to Title",
        pusher(actions, UiAct::RaceAbandoned),
    ));
    let control_count = controls_row.len();

    // Wrapping rather than a plain row: three buttons at their comfortable width
    // are wider together than a 320 dp screen, and a row that cannot wrap puts
    // "Quit to Title" past the right edge with no way to reach it mid-race.
    let control_row = FlowRow(
        Modifier::new().gap(theme::dp(12.0)),
        FlowRowConfig::default(),
    )
    .child(controls_row);

    let mut hints = controls_hint();
    if data.practice {
        hints = Column(Modifier::new().gap(theme::dp(4.0)))
            .child(hints)
            .child(
                Text("C returns you to the last checkpoint, keeping the clock")
                    .size(theme::sp(14.0))
                    .color(theme::accent())
                    .max_lines(3),
            );
    }

    // The bottom band. Side by side the hints sit left and the buttons right, each
    // an absolute box that is a *sibling* of the stack rather than a layer inside
    // another one: a ZStack places its layers by its own alignment, so an absolute
    // box nested in one loses its offsets and lands wherever the stack put it.
    //
    // Side by side only while the two actually fit. The test is their widths
    // together, not the window's class: `is_narrow` alone leaves a 900 dp window —
    // not compact — where a 520 dp hint block and three 150 dp buttons come to more
    // than the screen, and each was drawn over the other. Stacked, hints above
    // buttons in one bottom-anchored column, so the pair cannot collide at any width.
    // The two views go in that column bare: an absolute box nested in a column is out
    // of flow, so both would land at the same corner rather than one above the other.
    let hint_w = fit::fit(520.0, fit::chrome_inset() * 2.0);
    let buttons_w =
        fit::button_width() * control_count as f32 + 12.0 * (control_count as f32 - 1.0);
    let fits_side_by_side = hint_w + buttons_w + fit::chrome_inset() * 2.0 <= fit::width();
    let bottom = if fits_side_by_side {
        vec![
            Box(Modifier::new()
                .absolute()
                .offset(
                    Some(theme::dp(fit::chrome_inset())),
                    None,
                    None,
                    Some(theme::dp(fit::chrome_inset())),
                )
                .hit_passthrough())
            .child(hints),
            Box(Modifier::new()
                .absolute()
                .offset(
                    None,
                    None,
                    Some(theme::dp(fit::chrome_inset())),
                    Some(theme::dp(fit::chrome_inset())),
                )
                .hit_passthrough())
            .child(control_row),
        ]
    } else {
        vec![
            Box(Modifier::new()
                .absolute()
                .offset(
                    Some(theme::dp(fit::chrome_inset())),
                    None,
                    Some(theme::dp(fit::chrome_inset())),
                    Some(theme::dp(fit::chrome_inset())),
                )
                .hit_passthrough())
            .child(
                Column(Modifier::new().gap(theme::dp(10.0)))
                    .child(hints)
                    .child(control_row),
            ),
        ]
    };

    let mut children = vec![viewport, race_hud(data)];
    children.extend(bottom);
    if let Some(stick) = data.stick {
        children.push(stick_view(stick));
    }
    ZStack(Modifier::new().fill_max_size()).child(children)
}

/// The touch stick: a base the knob travels inside, and the knob.
///
/// Every layer is a coloured square with a corner radius of half its side, so the
/// paint system draws the circles. That keeps the whole control sharp at any
/// pixel density, which a fixed-size sprite cannot be on a 2x or 3x screen, and it
/// costs no image to decode.
///
/// Radii are in dp. The knob's travel is `STICK_TRAVEL_DP` and its own radius
/// `STICK_KNOB_DP / 2`, so at full deflection its outer edge lands on the base's
/// — the well and ring are sized just outside the knob for that reason, which is
/// what leaves the ring visible while the stick is at rest.
fn stick_view(stick: StickView) -> View {
    ZStack(Modifier::new().fill_max_size()).child(stick_layers(stick))
}

/// The layers, back to front: the base and the well it travels in, then the knob
/// over them.
fn stick_layers(stick: StickView) -> Vec<View> {
    let anchor = stick.anchor;
    let knob = (stick.anchor.0 + stick.knob.0, stick.anchor.1 + stick.knob.1);
    let knob_r = STICK_KNOB_DP / 2.0;
    let well_r = STICK_TRAVEL_DP + 4.0;

    vec![
        disc(anchor, STICK_BASE_DP / 2.0 + 4.0, theme::touch_shadow()),
        disc(anchor, STICK_BASE_DP / 2.0, theme::touch_base()),
        disc(anchor, well_r, theme::touch_well()),
        ring(anchor, well_r, theme::touch_ring()),
        disc(anchor, 6.0, theme::touch_centre()),
        disc(knob, knob_r + 3.0, theme::touch_knob_rim()),
        disc(knob, knob_r, theme::touch_knob()),
    ]
}

/// A filled circle: a square whose corner radius is half its side.
fn disc(center: (f32, f32), radius_dp: f32, color: Color) -> View {
    let size = theme::dp(radius_dp * 2.0);
    let (left, top, right, bottom) = at(center, radius_dp);
    Box(Modifier::new()
        .absolute()
        .offset(left, top, right, bottom)
        .width(size)
        .height(size)
        .background(color)
        .clip_rounded(size / 2.0)
        .hit_passthrough())
    .child(Spacer())
}

/// The same, hollow: a transparent square with a circular border, which reads as a
/// ring and costs one view less than a filled disc with a hole cut out of it.
fn ring(center: (f32, f32), radius_dp: f32, color: Color) -> View {
    let size = theme::dp(radius_dp * 2.0);
    let (left, top, right, bottom) = at(center, radius_dp);
    Box(Modifier::new()
        .absolute()
        .offset(left, top, right, bottom)
        .width(size)
        .height(size)
        .border(theme::dp(2.0), color, size / 2.0)
        .hit_passthrough())
    .child(Spacer())
}

/// Where a square of the given radius sits to put its middle on `center`, as the
/// `(left, top, right, bottom)` offsets an absolute box takes.
///
/// Each axis is converted from physical px and backed off by the radius to reach
/// the square's corner, in f32 rather than as arithmetic on `Dp` because this is
/// only ever a position, never a length to compare.
fn at(center: (f32, f32), radius_dp: f32) -> (Option<Dp>, Option<Dp>, Option<Dp>, Option<Dp>) {
    let edge = |px: f32| Some(Dp(px_to_dp(Px(px)).0 - radius_dp));
    (edge(center.0), edge(center.1), None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::STICK_TRAVEL_DP;
    use crate::ui::fit::tests::WINDOW;
    use repose_core::{
        SceneNode, calculate_window_size_class, set_window_container_size,
        set_window_size_class_default,
    };
    use repose_ui::layout_and_paint;
    use std::collections::HashMap;

    /// The discs only exist during a race under a finger, neither of which a test
    /// can produce, so at least check it composes and comes out the depth it should.
    #[test]
    fn the_stick_composes_without_a_touch() {
        let layers = stick_layers(StickView {
            anchor: (100.0, 200.0),
            knob: (0.0, 0.0),
        });
        assert_eq!(
            layers.len(),
            7,
            "shadow, base, well, ring, centre, knob rim, knob"
        );
    }

    /// Where the stick actually lands. `Modifier::offset` is
    /// `(left, top, right, bottom)`, and the argument order is not something the
    /// compiler can check: passing the touch's Y as the left offset put every disc
    /// horizontally wherever the finger was vertically, and left the vertical edge
    /// unset — so a touch near the bottom of a wide window drew the stick far off
    /// to the right, at the top of the screen.
    #[test]
    fn the_stick_is_drawn_over_the_finger() {
        for (w, h) in [(800.0f32, 600.0f32), (360.0, 780.0)] {
            let _turn = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            set_window_container_size(w, h);
            set_window_size_class_default(calculate_window_size_class(w as u32, h as u32, 1.0));
            for anchor in [(200.0f32, 400.0f32), (600.0, 200.0)] {
                let layers = stick_layers(StickView {
                    anchor,
                    knob: (0.0, 0.0),
                });
                let (scene, _, _) = layout_and_paint(
                    &ZStack(Modifier::new().fill_max_size()).child(layers),
                    (w as u32, h as u32),
                    &HashMap::new(),
                    &Default::default(),
                    None,
                );
                // The base disc is the widest square the stack draws.
                let base = scene
                    .nodes
                    .iter()
                    .filter_map(|n| match n {
                        SceneNode::Rect { rect, .. } if rect.w > 60.0 && rect.h > 60.0 => {
                            Some(*rect)
                        }
                        _ => None,
                    })
                    .max_by(|a, b| a.w.partial_cmp(&b.w).unwrap())
                    .expect("a base disc");
                let want_x = px_to_dp(Px(anchor.0)).0;
                let want_y = px_to_dp(Px(anchor.1)).0;
                let found_x = base.x + base.w / 2.0;
                let found_y = base.y + base.h / 2.0;
                assert!(
                    (found_x - want_x).abs() < 2.0 && (found_y - want_y).abs() < 2.0,
                    "at {w:.0}x{h:.0} a touch at px{anchor:?} drew the base at \
                     {found_x:.1},{found_y:.1} dp instead of {want_x:.1},{want_y:.1}"
                );
            }
        }
    }

    /// The ring has to sit outside the knob's own radius or it is invisible while the
    /// stick is at rest, which is the state the player sees most.
    #[test]
    fn the_ring_shows_around_a_resting_knob() {
        assert!(
            STICK_TRAVEL_DP + 4.0 > STICK_KNOB_DP / 2.0,
            "the ring is drawn inside the knob at rest and will never be seen"
        );
    }

    /// At full travel the knob's outer edge has to land on the base's, or the stick
    /// either overhangs its own rim or never looks fully deflected.
    #[test]
    fn a_fully_deflected_knob_fills_the_base() {
        assert_eq!(STICK_TRAVEL_DP + STICK_KNOB_DP / 2.0, STICK_BASE_DP / 2.0);
    }
}

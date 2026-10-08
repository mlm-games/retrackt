use repose_core::{Color, Dp, Modifier, Px, View, px_to_dp};
use repose_ui::{Box, Column, Row, Spacer, Text, TextStyle, ViewExt, ZStack};

use crate::app::input::{STICK_BASE_DP, STICK_KNOB_DP, STICK_TRAVEL_DP};
use crate::app::state::{ActionQueue, AppData, StickView, UiAct};
use crate::app::theme;
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
    let controls = Box(Modifier::new()
        .absolute()
        .offset(None, None, Some(theme::dp(20.0)), Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(Row(Modifier::new().gap(theme::dp(12.0))).child(controls_row));

    let mut hints = controls_hint();
    if data.practice {
        hints = Column(Modifier::new().gap(theme::dp(4.0)))
            .child(hints)
            .child(
                Text("C returns you to the last checkpoint, keeping the clock")
                    .size(theme::sp(14.0))
                    .color(theme::accent())
                    .single_line(),
            );
    }
    let hints = Box(Modifier::new()
        .absolute()
        .offset(Some(theme::dp(20.0)), None, None, Some(theme::dp(20.0)))
        .hit_passthrough())
    .child(hints);

    let mut children = vec![viewport, race_hud(data), hints, controls];
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
    let (top, bottom, left, right) = at(center, radius_dp);
    Box(
        Modifier::new()
            .absolute()
            .offset(top, bottom, left, right)
            .width(size)
            .height(size)
            .background(color)
            .clip_rounded(size / 2.0)
            .hit_passthrough(),
    )
    .child(Spacer())
}

/// The same, hollow: a transparent square with a circular border, which reads as a
/// ring and costs one view less than a filled disc with a hole cut out of it.
fn ring(center: (f32, f32), radius_dp: f32, color: Color) -> View {
    let size = theme::dp(radius_dp * 2.0);
    let (top, bottom, left, right) = at(center, radius_dp);
    Box(
        Modifier::new()
            .absolute()
            .offset(top, bottom, left, right)
            .width(size)
            .height(size)
            .border(theme::dp(2.0), color, size / 2.0)
            .hit_passthrough(),
    )
    .child(Spacer())
}

/// Where a square of the given radius sits to put its middle on `center`, as the
/// `(top, bottom, left, right)` offsets an absolute box takes.
///
/// Each axis is converted from physical px and backed off by the radius to reach
/// the square's corner, in f32 rather than as arithmetic on `Dp` because this is
/// only ever a position, never a length to compare.
fn at(center: (f32, f32), radius_dp: f32) -> (Option<Dp>, Option<Dp>, Option<Dp>, Option<Dp>) {
    let edge = |px: f32| Some(Dp(px_to_dp(Px(px)).0 - radius_dp));
    (edge(center.1), None, edge(center.0), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::STICK_TRAVEL_DP;

    /// The stick only exists during a race under a finger, neither of which a test
    /// can produce, so at least check it composes and comes out the depth it should.
    #[test]
    fn the_stick_composes_without_a_touch() {
        let layers = stick_layers(StickView {
            anchor: (100.0, 200.0),
            knob: (0.0, 0.0),
        });
        assert_eq!(layers.len(), 7, "shadow, base, well, ring, centre, knob rim, knob");
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

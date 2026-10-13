//! The garage: the car's paint.
//!
//! One thing to choose, from a handful of swatches, with the car drawn on screen
//! so the choice is judged on the car rather than on a colour chip. The paint is
//! a preference and is persisted like the volume: it rides with the game rather
//! than with a track or a run.

use repose_core::{Color, Modifier, View};
use repose_ui::{Box, FlowRow, FlowRowConfig, Row, Text, TextStyle, ViewExt, ZStack};

use crate::app::state::{ActionQueue, AppData, UiAct};
use crate::app::theme;
use crate::ui::widgets::{list_panel_w, outlined_at, pusher};
use repose_material::material3::ButtonConfig;

/// Size of one paint swatch, and the body colour it shows.
const SWATCH: f32 = 40.0;

/// The garage screen.
pub fn garage_ui(data: &AppData, actions: &ActionQueue) -> View {
    let mut swatches: Vec<View> = Vec::new();
    for (index, hex) in theme::CAR_PAINTS.iter().enumerate() {
        let selected = data.car_paint == index as u8;
        swatches.push(swatch(index as u8, *hex, selected, actions));
    }

    let panel = list_panel_w(
        "Garage",
        theme::dp(340.0),
        vec![
            Text("Body paint")
                .size(theme::sp(13.0))
                .color(theme::text_dim())
                .single_line(),
            FlowRow(
                Modifier::new().gap(theme::dp(10.0)).fill_max_width(),
                FlowRowConfig {
                    balanced: true,
                    ..FlowRowConfig::default()
                },
            )
            .child(swatches),
            Text("The car you build your next race in.")
                .size(theme::sp(12.0))
                .color(theme::text_dim())
                .single_line(),
            outlined_at(
                "Back",
                theme::dp(120.0),
                ButtonConfig::default(),
                pusher(actions, UiAct::CloseGarage),
            ),
        ],
    );

    // Drawn over the car, so the swatch chosen is the car being driven rather
    // than a chip beside a name.
    ZStack(Modifier::new().fill_max_size()).child([
        Box(Modifier::new().fill_max_size()).child(car_silhouette(data.car_paint)),
        Box(Modifier::new().absolute().offset(
            Some(theme::dp(20.0)),
            None,
            None,
            Some(theme::dp(20.0)),
        ))
        .child(panel),
    ])
}

/// One paint swatch. Tapping it paints the car: there is no second gesture,
/// because a palette that armed a colour would need one.
fn swatch(index: u8, hex: u32, selected: bool, actions: &ActionQueue) -> View {
    Box(Modifier::new()
        .width(theme::dp(SWATCH))
        .height(theme::dp(SWATCH))
        .background(theme::rgb(hex))
        .border(
            theme::dp(3.0),
            if selected {
                theme::accent()
            } else {
                theme::text_dim().with_alpha_f32(0.4)
            },
            theme::dp(SWATCH / 2.0),
        )
        .on_click(pusher(actions, UiAct::SetCarPaint(index))))
}

/// The car in its paint, drawn as the two bands it is made of: body and canopy.
/// Enough to judge a colour by, and no more.
fn car_silhouette(paint: u8) -> View {
    let body = theme::rgb(theme::car_paint(paint));
    Box(Modifier::new()
        .fill_max_size()
        .background(theme::background()))
    .child(
        Row(Modifier::new()
            .fill_max_size()
            .align_items(repose_core::AlignItems::CENTER))
        .child([Box(Modifier::new()
            .width(theme::dp(220.0))
            .height(theme::dp(90.0))
            .background(body)
            .border(theme::dp(2.0), Color::from_rgb(10, 12, 16), theme::dp(18.0))
            .clip_rounded(theme::dp(18.0)))
        .child(Box(Modifier::new()
            .width(theme::dp(110.0))
            .height(theme::dp(42.0))
            .background(theme::rgb(0x1E2733))
            .offset(
                Some(theme::dp(48.0)),
                None,
                None,
                Some(theme::dp(20.0)),
            )))]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{ActionQueue, AppData, UiAct};
    use repose_core::set_window_size_class_default;
    use repose_core::{SceneNode, calculate_window_size_class, set_window_container_size};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn garage_at(dp_w: f32, dp_h: f32, paint: u8) -> (repose_core::Scene, Vec<UiAct>) {
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let actions: ActionQueue = Rc::new(RefCell::new(Vec::new()));
        let mut data = AppData::default();
        data.screen = crate::app::state::Screen::Garage;
        data.car_paint = paint;
        let view = crate::ui::screens::garage::garage_ui(&data, &actions);
        let (scene, _, _) = repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        (scene, std::mem::take(&mut *actions.borrow_mut()))
    }

    /// Swatches the garage lays out, by the rects that are exactly swatch sized.
    fn swatch_rects(scene: &repose_core::Scene) -> Vec<repose_core::Rect> {
        scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                SceneNode::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .filter(|r| (r.w - 40.0).abs() < 1.0 && (r.h - 40.0).abs() < 1.0)
            .collect()
    }

    #[test]
    fn the_garage_offers_every_paint_and_none_elsewhere() {
        // A paint that cannot be picked is a paint nobody has, so the count of
        // swatches is the length of the palette at every size the garage shows.
        for (w, h) in [(360.0f32, 780.0), (1024.0, 768.0)] {
            let (scene, _) = garage_at(w, h, 0);
            let rects = swatch_rects(&scene);
            assert_eq!(
                rects.len(),
                crate::app::theme::CAR_PAINTS.len(),
                "{w}x{h}: one swatch per paint"
            );
            // They lay out side by side, not stacked: a wall of swatches on a
            // phone is a wall to scroll, and the palette never overflows.
            let ys: Vec<f32> = rects.iter().map(|r| r.y).collect();
            let rows = ys.iter().filter(|y| **y == ys[0]).count();
            assert!(rows >= 2, "{w}x{h}: swatches do not flow, {rows} per row");
        }
    }

    #[test]
    fn the_garage_stays_inside_the_window() {
        // Drawn over the car, so the panel must not hang off a phone.
        for (w, h) in [(360.0f32, 780.0), (800.0, 360.0), (1280.0, 800.0)] {
            let (scene, _) = garage_at(w, h, 3);
            let (mut right, mut bottom) = (0.0f32, 0.0f32);
            for n in &scene.nodes {
                let r = match n {
                    SceneNode::Rect { rect, .. }
                    | SceneNode::Border { rect, .. }
                    | SceneNode::Text { rect, .. } => *rect,
                    _ => continue,
                };
                right = right.max(r.x + r.w);
                bottom = bottom.max(r.y + r.h);
            }
            assert!(right <= w as f32 + 1.0, "{w}x{h}: {right} wide");
            assert!(bottom <= h as f32 + 1.0, "{w}x{h}: {bottom} tall");
        }
    }

    #[test]
    fn a_paint_out_of_range_falls_back_rather_than_panicking() {
        // A save can carry an index the palette no longer has.
        assert_eq!(
            crate::app::theme::car_paint(250),
            crate::app::theme::CAR_PAINTS[0]
        );
        let (scene, _) = garage_at(1024.0, 768.0, 250);
        assert!(!scene.nodes.is_empty(), "the garage must still draw");
    }
}

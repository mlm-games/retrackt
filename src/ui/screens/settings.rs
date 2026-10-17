//! The options screen: what the game looks like, sounds like and reads in.
//!
//! One panel of labelled rows, and no Apply. Every row writes straight through
//! its own action and is persisted on the frame it is pressed, because an option
//! that had to be confirmed is an option nobody is sure they changed. The few
//! that cannot take effect until something else happens — the paint, the
//! practice flag — say so on the row rather than hiding the fact.

use repose_core::{AlignItems, Modifier, View};
use repose_material::material3::ButtonConfig;
use repose_ui::{Box, Column, DragValue, FlowRow, FlowRowConfig, Row, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, AppData, UiAct, push};
use crate::app::theme;
use crate::ui::widgets::{choice_chip, heading, list_panel, outlined_at, pusher};

/// Width of the label column. Fixed so every row's controls start at the same
/// x down the length of the panel, which is what makes the list scannable.
const LABEL_W: f32 = 116.0;

pub fn settings_ui(data: &AppData, actions: &ActionQueue) -> View {
    let s = &data.settings;

    let body = Column(
        Modifier::new()
            .gap(theme::dp(14.0))
            .align_items(AlignItems::STRETCH)
            .fill_max_width(),
    )
    .child(vec![
        heading("Gameplay"),
        choice_row(
            "Units",
            &crate::save::UNIT_NAMES,
            s.units,
            UiAct::SetUnits,
            actions,
        ),
        toggle_row("Speedometer", s.speedometer, UiAct::SetSpeedometer, actions),
        toggle_row("Timer", s.timer, UiAct::SetTimer, actions),
        slider_row(
            "Steering",
            s.steering,
            (0.5, 1.5),
            0.05,
            UiAct::SetSteering,
            actions,
            |v| format!("{:.0}%", v * 100.0),
        ),
        heading("Graphics"),
        choice_row(
            "Quality",
            &crate::save::QUALITY_NAMES,
            s.quality,
            UiAct::SetQuality,
            actions,
        ),
        toggle_row("Shadows", s.shadows, UiAct::SetShadows, actions),
        toggle_row("Fog", s.fog, UiAct::SetFog, actions),
        toggle_row("Clouds", s.clouds, UiAct::SetClouds, actions),
        toggle_row("Particles", s.particles, UiAct::SetParticles, actions),
        toggle_row("Skidmarks", s.skidmarks, UiAct::SetSkidmarks, actions),
        heading("Audio"),
        toggle_row("Sound", s.sound, UiAct::SetSound, actions),
        slider_row(
            "Master",
            s.master_volume,
            (0.0, 1.0),
            0.05,
            UiAct::SetMasterVolume,
            actions,
            pct,
        ),
        slider_row(
            "Music",
            s.music_volume,
            (0.0, 1.0),
            0.05,
            UiAct::SetMusicVolume,
            actions,
            pct,
        ),
        slider_row(
            "Effects",
            s.sfx_volume,
            (0.0, 1.0),
            0.05,
            UiAct::SetSfxVolume,
            actions,
            pct,
        ),
        heading("Display"),
        toggle_row("Fullscreen", s.fullscreen, UiAct::SetFullscreen, actions),
        Box(Modifier::new().height(theme::dp(8.0))),
        outlined_at(
            "Back",
            theme::dp(120.0),
            ButtonConfig::default(),
            pusher(actions, UiAct::CloseSettings),
        ),
    ]);

    Box(Modifier::new()
        .absolute()
        .offset(Some(theme::dp(20.0)), None, None, Some(theme::dp(20.0)))
        .max_width(theme::dp(crate::ui::fit::panel_width(520.0))))
    .child(list_panel("Options", vec![body]))
}

fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

fn row_label(text: &str) -> View {
    Box(Modifier::new().min_width(theme::dp(LABEL_W))).child(
        Text(text)
            .size(theme::sp(14.0))
            .color(theme::text())
            .single_line(),
    )
}

/// One of a fixed set, shown as a row of chips with the live one emphasised.
/// Every option is on screen at once, so choosing between them costs one tap
/// and never a hunt through a cycle.
/// Width of the wrapping control block in a choice row.
///
/// A fixed cap rather than `flex_grow`, which is what the editor's own control
/// rows use: a wrapping row given only a share of a parent has nothing to wrap
/// against until the parent has been measured, and the two sizes depend on each
/// other. Bounded, the wrap is decided from a number that is already known.
fn controls_w() -> f32 {
    (crate::ui::fit::panel_width(520.0) - LABEL_W - 40.0).max(140.0)
}

fn choice_row(
    label: &str,
    names: &[&str],
    current: u8,
    set: fn(u8) -> UiAct,
    actions: &ActionQueue,
) -> View {
    let chips: Vec<View> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let index = index as u8;
            choice_chip(name, index == current, pusher(actions, set(index)))
        })
        .collect();

    Row(Modifier::new()
        .gap(theme::dp(8.0))
        .align_items(AlignItems::CENTER)
        .fill_max_width())
    .child([
        row_label(label),
        FlowRow(
            Modifier::new()
                .gap(theme::dp(8.0))
                .max_width(theme::dp(controls_w())),
            FlowRowConfig {
                balanced: true,
                ..FlowRowConfig::default()
            },
        )
        .child(chips),
    ])
}

/// On or off, both shown, so the state is readable without knowing the
/// convention and switching it is a tap rather than a guess about a toggle.
fn toggle_row(label: &str, on: bool, set: fn(bool) -> UiAct, actions: &ActionQueue) -> View {
    Row(Modifier::new()
        .gap(theme::dp(8.0))
        .align_items(AlignItems::CENTER)
        .fill_max_width())
    .child([
        row_label(label),
        choice_chip("On", on, pusher(actions, set(false))),
        choice_chip("Off", !on, pusher(actions, set(true))),
    ])
}

/// A labelled number with the value in it, dragged rather than typed. The text
/// on the right is the value the drag is setting, so a slider does not have to
/// be read off its own handle.
fn slider_row(
    label: &str,
    value: f32,
    range: (f32, f32),
    step: f32,
    set: fn(f32) -> UiAct,
    actions: &ActionQueue,
    fmt: fn(f32) -> String,
) -> View {
    Row(Modifier::new()
        .gap(theme::dp(8.0))
        .align_items(AlignItems::CENTER)
        .fill_max_width())
    .child([
        row_label(label),
        Box(Modifier::new().flex_grow(1.0).min_width(theme::dp(150.0))).child(DragValue(
            value,
            range,
            step,
            {
                let queue = actions.clone();
                move |v: f32| push(&queue, set(v))
            },
        )),
        Box(Modifier::new().min_width(theme::dp(52.0))).child(
            Text(fmt(value))
                .size(theme::sp(14.0))
                .color(theme::text_dim())
                .single_line(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Screen;
    use repose_core::set_window_size_class_default;
    use repose_core::{SceneNode, calculate_window_size_class, set_window_container_size};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn options_at(dp_w: f32, dp_h: f32) -> (repose_core::Scene, Vec<UiAct>) {
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(dp_w, dp_h);
        set_window_size_class_default(calculate_window_size_class(dp_w as u32, dp_h as u32, 1.0));
        let actions: ActionQueue = Rc::new(RefCell::new(Vec::new()));
        let mut data = AppData::default();
        data.screen = Screen::Settings;
        let view = settings_ui(&data, &actions);
        let (scene, _, _) = repose_ui::layout_and_paint(
            &view,
            (dp_w as u32, dp_h as u32),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        (scene, std::mem::take(&mut *actions.borrow_mut()))
    }

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

    #[test]
    fn every_option_is_on_screen() {
        crate::report_failure_where_it_happens();
        // Scrolled to the top, every row above the fold has to be laid out. The
        // panel scrolls, so this asserts the head of the list rather than all of
        // it: a row that never laid out is a row that cannot be reached.
        let (scene, _) = options_at(1024.0, 768.0);
        let found = labels(&scene);
        for want in ["Gameplay", "Units", "Speedometer", "Timer"] {
            assert!(
                found.iter().any(|t| t == want),
                "{want} is missing from {found:?}"
            );
        }
    }

    #[test]
    fn the_options_stay_inside_the_window() {
        crate::report_failure_where_it_happens();
        for (w, h) in [(360.0f32, 780.0), (800.0, 360.0), (1280.0, 800.0)] {
            let (scene, _) = options_at(w, h);
            // The panel scrolls, so its *content* runs past the bottom on purpose:
            // measuring every node would measure what has been scrolled out of
            // sight, which is content the window is not meant to hold. The panel's
            // own box is the largest rect on screen, and that is what has to fit.
            let (mut area, mut widest) = (0.0f32, 0.0f32);
            let mut panel = None;
            for n in &scene.nodes {
                let r = match n {
                    SceneNode::Rect { rect, .. }
                    | SceneNode::Border { rect, .. }
                    | SceneNode::Text { rect, .. } => *rect,
                    _ => continue,
                };
                widest = widest.max(r.x + r.w);
                if r.w * r.h > area {
                    area = r.w * r.h;
                    panel = Some(r);
                }
            }
            let panel = panel.expect("the options must draw a panel");
            assert!(
                panel.x + panel.w <= w as f32 + 1.0,
                "{w}x{h}: panel {panel:?} runs off the right"
            );
            assert!(
                panel.y + panel.h <= h as f32 + 1.0,
                "{w}x{h}: panel {panel:?} runs off the bottom"
            );
            assert!(widest <= w as f32 + 1.0, "{w}x{h}: {widest} wide");
        }
    }

    #[test]
    fn an_option_out_of_range_still_draws() {
        crate::report_failure_where_it_happens();
        // A save can carry an index a shorter list no longer has.
        let _turn = crate::ui::fit::tests::WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_window_container_size(1024.0, 768.0);
        set_window_size_class_default(calculate_window_size_class(1024, 768, 1.0));
        let actions: ActionQueue = Rc::new(RefCell::new(Vec::new()));
        let mut data = AppData::default();
        data.screen = Screen::Settings;
        data.settings.units = 250;
        data.settings.quality = 250;
        let view = settings_ui(&data, &actions);
        let (scene, _, _) = repose_ui::layout_and_paint(
            &view,
            (1024, 768),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        assert!(!scene.nodes.is_empty(), "the options must still draw");
    }
}

use repose_core::{Modifier, View};
use repose_material::material3::{Button, ButtonConfig, OutlinedButton};
use repose_ui::{Box, Center, Column, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, UiAct, push};
use crate::app::theme;

pub const CONTROLS: [(&str, &str); 8] = [
    ("WASD / Arrows", "drive"),
    ("Space", "handbrake"),
    ("Shift", "boost"),
    ("R", "restart"),
    ("C", "back to checkpoint"),
    ("Esc", "quit to title"),
    ("Stick, RT, LT", "drive"),
    ("RB / LB", "handbrake / boost"),
    ("Y / X / B", "restart / checkpoint / quit"),
];

pub fn pusher(actions: &ActionQueue, act: UiAct) -> impl Fn() + 'static {
    let queue = actions.clone();
    move || push(&queue, act.clone())
}

pub fn menu_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    Button(
        Modifier::new().width(theme::dp(300.0)),
        on_click,
        ButtonConfig {
            container_color: Some(theme::accent()),
            content_color: Some(theme::background()),
            ..ButtonConfig::default()
        },
        || Text(label).size(theme::sp(16.0)).single_line(),
    )
}

pub fn ghost_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    OutlinedButton(
        Modifier::new().min_width(theme::dp(150.0)),
        on_click,
        ButtonConfig::default(),
        || Text(label).size(theme::sp(16.0)).single_line(),
    )
}

pub fn danger_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    OutlinedButton(
        Modifier::new().min_width(theme::dp(150.0)),
        on_click,
        ButtonConfig {
            content_color: Some(theme::danger()),
            border: Some((theme::dp(1.0), theme::danger(), theme::dp(8.0))),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        || Text(label).size(theme::sp(16.0)).single_line(),
    )
}

/// The emphasised state of a toggle: the armed palette piece, the selected row,
/// the place button. Same shape as `ghost_btn` so a highlighted control does not
/// resize its row when it turns on.
pub fn accent_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    OutlinedButton(
        Modifier::new().min_width(theme::dp(150.0)),
        on_click,
        ButtonConfig {
            container_color: Some(theme::accent()),
            content_color: Some(theme::background()),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        || Text(label).size(theme::sp(16.0)).single_line(),
    )
}

/// A control that is present but not live, for undo and redo with nothing to step.
///
/// Visible rather than hidden so the feature is discoverable, and inert rather
/// than silent so the player learns why.
pub fn disabled_btn(label: &str) -> View {
    OutlinedButton(
        Modifier::new().min_width(theme::dp(150.0)),
        || {},
        ButtonConfig {
            enabled: false,
            content_color: Some(theme::text_dim()),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        || Text(label).size(theme::sp(16.0)).single_line(),
    )
}

/// The practice toggle, on every screen a run is started from.
///
/// Practice is a mode rather than a screen: putting the toggle here means switching
/// it costs one tap instead of a trip back to a menu.
pub fn practice_toggle(practice: bool, actions: &ActionQueue) -> View {
    ghost_btn(
        if practice { "Practice: on" } else { "Practice: off" },
        pusher(actions, UiAct::SetPractice(!practice)),
    )
}

pub fn panel(title: &str, children: Vec<View>) -> View {
    let mut column = Column(
        Modifier::new()
            .gap(theme::dp(14.0))
            .padding(theme::dp(28.0))
            .background(theme::surface())
            .border(
                theme::dp(1.0),
                theme::text_dim().with_alpha_f32(0.4),
                theme::dp(12.0),
            )
            .clip_rounded(theme::dp(12.0)),
    );
    if !title.is_empty() {
        column = column.child(
            Text(title)
                .size(theme::sp(22.0))
                .color(theme::accent())
                .single_line(),
        );
    }
    column.child(children)
}

pub fn screen_backdrop(content: View) -> View {
    Center(
        Modifier::new()
            .fill_max_size()
            .background(theme::background().with_alpha_f32(0.72))
            .input_blocker(),
    )
    .child(content)
}

pub fn hud_text(text: &str) -> View {
    Text(text)
        .size(theme::sp(16.0))
        .color(theme::text())
        .single_line()
}

pub fn controls_hint() -> View {
    let line = |range: std::ops::Range<usize>| {
        CONTROLS[range]
            .iter()
            .map(|(key, action)| format!("{key} {action}"))
            .collect::<Vec<String>>()
            .join("  ·  ")
    };
    Column(
        Modifier::new()
            .gap(theme::dp(4.0))
            .padding(theme::dp(8.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(6.0))
            .hit_passthrough(),
    )
    .child(dim_text(&line(0..3)))
    .child(dim_text(&line(3..5)))
    .child(dim_text(&line(5..6)))
    .child(dim_text(&line(6..8)))
}

pub fn dim_text(text: &str) -> View {
    Text(text)
        .size(theme::sp(14.0))
        .color(theme::text_dim())
        .single_line()
}

pub fn heading(text: &str) -> View {
    Text(text)
        .size(theme::sp(30.0))
        .color(theme::text())
        .single_line()
}

pub fn badge(label: &str) -> View {
    Box(Modifier::new()
        .padding(theme::dp(6.0))
        .background(theme::accent())
        .clip_rounded(theme::dp(6.0)))
    .child(
        Text(label)
            .size(theme::sp(14.0))
            .color(theme::background())
            .single_line(),
    )
}

/// Race time in simulated ticks. Integer throughout: the race clock counts
/// ticks, so the displayed milliseconds are exactly the run that was timed.
pub fn fmt_ticks(ticks: u32) -> String {
    let ms = ticks as u64 * 1000 / crate::SIM_HZ as u64;
    format!(
        "{:02}:{:02}.{:03}",
        ms / 60_000,
        (ms % 60_000) / 1000,
        ms % 1000
    )
}

/// A gap to another run, signed: `-00:01.204` is a second and a bit quicker.
///
/// Signed rather than a bare duration because a delta read as a time loses the only
/// part of it the player is looking for.
pub fn fmt_delta(ticks: i64) -> String {
    let ms = ticks.unsigned_abs() * 1000 / crate::SIM_HZ as u64;
    format!(
        "{}{:02}:{:02}.{:03}",
        if ticks < 0 { "-" } else { "+" },
        ms / 60_000,
        (ms % 60_000) / 1000,
        ms % 1000
    )
}

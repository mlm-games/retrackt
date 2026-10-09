use repose_core::{AlignItems, Dp, Modifier, View};
use repose_material::material3::{Button, ButtonConfig, OutlinedButton};
use repose_ui::scroll::{ScrollArea, remember_scroll_state};
use repose_ui::{Box, Center, Column, Text, TextStyle, ViewExt};

use crate::app::state::{ActionQueue, UiAct, push};
use crate::app::theme;
use crate::ui::fit;

/// The width a dialog is happy at on a window with room to spare, in dp. Every
/// modal panel is capped at this *and* at the window, which is what lets the rows
/// inside one wrap instead of running off the side.
const PANEL_WIDE_DP: f32 = 560.0;
/// Comfortable height for a panel that is mostly one list.
const PANEL_TALL_DP: f32 = 520.0;

/// Gap between a panel's own children.
///
/// Named rather than written at the call site because callers that lay their own
/// content out against a panel have to reason about its rhythm: the editor sizes
/// its rows from it.
pub const PANEL_GAP: Dp = Dp(14.0);

/// Horizontal inset a panel's content sits inside.
///
/// Exposed because a caller that builds a row *inside* a panel has to know how
/// wide it is — the panel's width less this on both sides. Restating the number
/// separately is how the piece list came to ask for 620 dp inside a 520 dp box.
pub const PANEL_INSET_DP: Dp = Dp(20.0);

pub const CONTROLS: [(&str, &str); 9] = [
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
    // Grows to fill its row rather than sitting at a fixed 300 dp, which on a
    // phone would be wider than the screen and on a desktop would leave the
    // panel's own margin as the only thing bounding it.
    Button(
        Modifier::new().fill_max_width(),
        on_click,
        ButtonConfig {
            container_color: Some(theme::accent()),
            content_color: Some(theme::background()),
            ..ButtonConfig::default()
        },
        || {
            Text(label)
                .size(theme::sp(theme::CONTROL_SP))
                .font_family(theme::FONT_DISPLAY)
                .single_line()
        },
    )
}

pub fn ghost_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined(label, ButtonConfig::default(), on_click)
}

/// Every outlined button, since they differ only in colour: same width rules,
/// so turning one into its accent or danger variant never resizes its row.
fn outlined(label: &str, config: ButtonConfig, on_click: impl Fn() + 'static) -> View {
    outlined_at(label, theme::dp(fit::button_width()), config, on_click)
}

/// An outlined button with the caller's own width floor.
///
/// The editor derives its control width from the width of the panel it sits in,
/// so that a fixed number of controls share a line at every window size — which
/// one app-wide floor cannot express. Same shape and same variants as every
/// other outlined button, so a control changing colour never changes geometry.
pub fn outlined_at(
    label: &str,
    min_width: Dp,
    config: ButtonConfig,
    on_click: impl Fn() + 'static,
) -> View {
    OutlinedButton(
        Modifier::new().min_width(min_width),
        on_click,
        config,
        || {
            Text(label)
                .size(theme::sp(theme::CONTROL_SP))
                .font_family(theme::FONT_DISPLAY)
                .single_line()
        },
    )
}

/// A small, quiet control for a row of settings rather than a destination.
///
/// Tertiary to `menu_btn` and `ghost_btn`: the practice toggle and the thumbnail
/// toggle are preferences, and giving them the same weight as Start Race is what
/// made the old title read as one undifferentiated stack.
pub fn chip_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    OutlinedButton(
        Modifier::new().min_width(theme::dp(if fit::is_narrow() { 84.0 } else { 124.0 })),
        on_click,
        ButtonConfig {
            content_color: Some(theme::text_dim()),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        || {
            Text(label)
                .size(theme::sp(13.0))
                .font_family(theme::FONT_DISPLAY)
                .single_line()
        },
    )
}

pub fn danger_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined(
        label,
        ButtonConfig {
            content_color: Some(theme::danger()),
            border: Some((theme::dp(1.0), theme::danger(), theme::dp(8.0))),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        on_click,
    )
}

/// The emphasised state of a toggle: the armed palette piece, the selected row,
/// the place button. Same shape as `ghost_btn` so a highlighted control does not
/// resize its row when it turns on.
pub fn accent_btn(label: &str, on_click: impl Fn() + 'static) -> View {
    outlined(
        label,
        ButtonConfig {
            container_color: Some(theme::accent()),
            content_color: Some(theme::background()),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        on_click,
    )
}

/// A control that is present but not live, for undo and redo with nothing to step.
///
/// Visible rather than hidden so the feature is discoverable, and inert rather
/// than silent so the player learns why.
pub fn disabled_btn(label: &str) -> View {
    outlined(
        label,
        ButtonConfig {
            enabled: false,
            content_color: Some(theme::text_dim()),
            shape_radius: theme::dp(8.0),
            ..ButtonConfig::default()
        },
        || {},
    )
}

/// The practice toggle, on every screen a run is started from.
///
/// Practice is a mode rather than a screen: putting the toggle here means switching
/// it costs one tap instead of a trip back to a menu.
pub fn practice_toggle(practice: bool, actions: &ActionQueue) -> View {
    ghost_btn(
        if practice {
            "Practice: on"
        } else {
            "Practice: off"
        },
        pusher(actions, UiAct::SetPractice(!practice)),
    )
}

/// Width of a modal panel, in dp.
///
/// Every dialog uses this rather than a fixed number. A fixed width is a
/// coordinate in a desktop window and nothing at all in a phone held upright,
/// where the panel is most of the screen or wider than all of it.
pub fn panel_width() -> repose_core::Dp {
    theme::dp(fit::panel_width(PANEL_WIDE_DP))
}

/// Height cap for a panel, in dp.
///
/// Capped rather than left to its content because a panel taller than the window
/// has no way to be scrolled — the backdrop it sits on is what receives input,
/// not a scroll view — so its bottom controls would simply be unreachable.
pub fn panel_height() -> repose_core::Dp {
    theme::dp(fit::panel_height(PANEL_TALL_DP))
}

/// A dialog panel.
///
/// `children` are centred, and the panel itself is capped at a comfortable width
/// *and* at the window. The cap is what lets the rows inside it wrap: without one,
/// a panel is exactly as wide as its widest line, so a long name or a long time
/// pushes the buttons off the side of a small screen instead of the line wrapping
/// inside it.
pub fn panel(title: &str, children: Vec<View>) -> View {
    panel_inner(title, panel_width(), children, AlignItems::CENTER)
}

/// A panel for a list screen, whose content is taller than it is wide.
///
/// Left-aligned, because a row of a list is a row of columns and centring each of
/// them separately would leave a ragged left edge down the length of it.
pub fn list_panel(title: &str, children: Vec<View>) -> View {
    panel_inner(title, panel_width(), children, AlignItems::STRETCH)
}

/// A panel at the caller's own width, for a screen that has to fit one beside
/// something else rather than over it.
///
/// Deliberately *not* capped at [`panel_height`], unlike [`panel`] and
/// [`list_panel`]. Those sit over a backdrop that takes input rather than over a
/// scroll view, so a cap is the only thing keeping their contents on screen. One
/// inside a screen's own scroller is the opposite: the panel clips at the cap while
/// the scroller measures the column from those capped heights, so it has no range
/// reaching what was clipped.
pub fn panel_w(title: &str, width: Dp, children: Vec<View>) -> View {
    panel_body(title, width, children, AlignItems::CENTER, false)
}

/// [`list_panel`] at the caller's own width, and likewise uncapped for the same
/// reason as [`panel_w`].
///
/// A panel wider than the column holding it scrolls sideways inside that column,
/// which on the editor's side column is a horizontal scrollbar over a list.
pub fn list_panel_w(title: &str, width: Dp, children: Vec<View>) -> View {
    panel_body(title, width, children, AlignItems::STRETCH, false)
}

fn panel_inner(title: &str, width: Dp, children: Vec<View>, align: AlignItems) -> View {
    panel_body(title, width, children, align, true)
}

fn panel_body(
    title: &str,
    width: Dp,
    children: Vec<View>,
    align: AlignItems,
    capped: bool,
) -> View {
    let mut body = Column(
        Modifier::new()
            .gap(PANEL_GAP)
            .align_items(align)
            .fill_max_width(),
    );
    if !title.is_empty() {
        body = body.child(
            Text(title)
                .size(theme::sp(theme::SECTION_SP))
                .font_family(theme::FONT_DISPLAY)
                .color(theme::accent())
                .single_line(),
        );
    }
    body = body.child(children);

    // The cap goes on the scroller, not on the column holding the content. A column
    // with a max height and more in it than fits has its children shrunk below
    // their own height, and they paint over each other rather than being cut off —
    // which is how Restart ended up sitting on top of Save Ghost. Scrolling the
    // body instead keeps every control at its own height and reachable.
    //
    // Vertical only, and that is load-bearing rather than a simplification:
    // `ScrollArea` fills its content to the viewport width, so a wrapping row
    // inside a panel has something to wrap against. `ScrollAreaXY` leaves the
    // content at its own width so it can grow sideways, which leaves a wrapping row
    // bounded by itself — one line, running off the edge, with a horizontal
    // scrollbar on a dialog that has no business having one.
    let content = if capped {
        ScrollArea(
            Modifier::new().fill_max_width().max_height(panel_height()),
            remember_scroll_state(format!("panel.{title}")),
            body,
        )
    } else {
        body
    };

    Box(Modifier::new()
        .padding(PANEL_INSET_DP)
        .width(width)
        .background(theme::surface())
        .border(
            theme::dp(1.0),
            theme::text_dim().with_alpha_f32(0.4),
            theme::dp(12.0),
        )
        .clip_rounded(theme::dp(12.0)))
    .child(content)
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
    // Wrapped, and bounded by the window: a single line of the whole run is wider
    // than a phone held upright, so at 360 dp the hint block ran 86 dp off the right
    // edge of the screen and underneath the race buttons at the same time. Capping
    // the width is what gives the wrapping something to wrap against.
    let width = fit::fit(520.0, fit::chrome_inset() * 2.0);
    Column(
        Modifier::new()
            .gap(theme::dp(4.0))
            .max_width(theme::dp(width))
            .padding(theme::dp(8.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(6.0))
            .hit_passthrough(),
    )
    .child(
        Text(line(0..4))
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .max_lines(3),
    )
    .child(
        Text(line(4..6))
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .max_lines(3),
    )
    .child(
        Text(line(6..7))
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .max_lines(3),
    )
    .child(
        Text(line(7..9))
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .max_lines(3),
    )
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
        .font_family(theme::FONT_DISPLAY)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fit::tests::WINDOW;
    use repose_core::{
        SceneNode, calculate_window_size_class, set_window_container_size,
        set_window_size_class_default,
    };
    use repose_ui::{BasicTextField, TextFieldConfig, TextFieldState};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// A field inside a panel fills the panel's content width.
    ///
    /// `fill_max_width` on the field alone filled the column wrapped around it, and a
    /// column sized to its own content is only as wide as that content — so the field
    /// came out at the width of its own hint with the rest of the panel beside it
    /// unused, at every window size. The panel body has to fill its own box for a
    /// field inside it to have anything to fill.
    #[test]
    fn a_field_fills_the_panel_it_sits_in() {
        for (w, h) in [(1280.0f32, 800.0f32), (1024.0, 600.0)] {
            let _turn = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            set_window_container_size(w, h);
            set_window_size_class_default(calculate_window_size_class(w as u32, h as u32, 1.0));

            let state: Rc<RefCell<TextFieldState>> = Rc::new(RefCell::new(TextFieldState::new()));
            let field = BasicTextField(
                state,
                Modifier::new().fill_max_width(),
                "A placeholder hint",
                TextFieldConfig {
                    line_limits: repose_core::TextFieldLineLimits::SingleLine,
                    ..TextFieldConfig::default()
                },
            );
            let (scene, _, _) = repose_ui::layout_and_paint(
                &list_panel("Fields", vec![field]),
                (w as u32, h as u32),
                &HashMap::new(),
                &Default::default(),
                None,
            );

            // The panel is the bordered box; the placeholder marks the field.
            let panel = scene
                .nodes
                .iter()
                .find_map(|n| match n {
                    SceneNode::Border { rect, .. } if rect.w > 200.0 => Some(*rect),
                    _ => None,
                })
                .expect("a panel border");
            let inner_edge = panel.x + panel.w - PANEL_INSET_DP.0;
            let content = panel.w - PANEL_INSET_DP.0 * 2.0;
            let hint = scene
                .nodes
                .iter()
                .find_map(|n| match n {
                    SceneNode::Text { rect, text, .. } if text.as_ref() == "A placeholder hint" => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .expect("the field's placeholder");
            assert!(
                hint.x + hint.w <= inner_edge + 0.5,
                "at {w:.0}x{h:.0} the field reaches {:.0} but the panel's inner edge is {inner_edge:.0}",
                hint.x + hint.w
            );
            assert!(
                hint.w > content * 0.5,
                "at {w:.0}x{h:.0} the field is {:.0} wide inside {content:.0} of panel",
                hint.w
            );
        }
    }
}

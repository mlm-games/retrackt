use repose_core::{AlignItems, Modifier, View};
use repose_ui::{Column, Text, TextStyle, ViewExt, ZStack};

use crate::app::state::AppData;
use crate::app::theme;
use crate::ui::fit;
use crate::ui::widgets::{fmt_delta, fmt_ticks};

/// Matches the ghost car's albedo, so the HUD line and the car on the track are
/// visibly the same thing.
fn ghost_ink() -> repose_core::Color {
    repose_core::Color::from_rgb(90, 178, 232)
}

/// The top-left card's own geometry: it sits in the corner at the window's
/// chrome inset, over whatever the track is doing behind it.
fn top_left(children: Vec<View>) -> View {
    Column(
        Modifier::new()
            .absolute()
            .offset(
                Some(theme::dp(fit::chrome_inset())),
                Some(theme::dp(fit::chrome_inset())),
                None,
                None,
            )
            .gap(theme::dp(4.0))
            .padding(theme::dp(12.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(8.0))
            .hit_passthrough(),
    )
    .child(children)
}

fn top_right(children: Vec<View>) -> View {
    Column(
        Modifier::new()
            .absolute()
            .offset(
                None,
                Some(theme::dp(fit::chrome_inset())),
                Some(theme::dp(fit::chrome_inset())),
                None,
            )
            .gap(theme::dp(4.0))
            .padding(theme::dp(12.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(8.0))
            .align_items(AlignItems::END)
            .hit_passthrough(),
    )
    .child(children)
}

pub fn race_hud(data: &AppData) -> View {
    let settings = &data.settings;
    let checkpoint = data.checkpoint;
    let checkpoint_count = data.checkpoint_count;

    // The clock and the speedometer are the two readouts a driver can turn off,
    // and both live in the same card. The card goes with them rather than
    // staying as an empty box: a HUD that hides things by drawing a frame
    // around nothing still covers the corner of the track it sits on.
    let mut left_children: Vec<View> = Vec::new();
    if settings.timer {
        left_children.push(
            Text(fmt_ticks(data.race_ticks))
                .size(theme::sp(36.0 * fit::type_scale()))
                .color(theme::text())
                .single_line(),
        );
    }
    if settings.speedometer {
        let (value, unit) = crate::save::speed_in_units(data.speed_kmh, settings.units);
        left_children.push(
            Text(format!("{value:.0}{unit}"))
                .size(theme::sp(22.0 * fit::type_scale()))
                .color(theme::accent())
                .single_line(),
        );
    }

    let mut right_children = vec![
        Text(format!("CP {checkpoint} / {checkpoint_count}"))
            .size(theme::sp(22.0))
            .color(theme::text())
            .single_line(),
    ];
    let best = match data.best {
        Some(best) => format!("Best {}", fmt_ticks(best)),
        None => "Best —".to_string(),
    };
    right_children.push(
        Text(best)
            .size(theme::sp(16.0))
            .color(theme::text_dim())
            .single_line(),
    );

    // What the run is being timed against. Without a label it reads as a second
    // clock bug rather than a target, so the ghost's own time is named.
    if let (Some(name), Some(ticks)) = (&data.ghost_name, data.ghost_ticks) {
        right_children.push(
            Text(format!("vs {name}  {}", fmt_ticks(ticks)))
                .size(theme::sp(16.0))
                .color(ghost_ink())
                .single_line(),
        );
    }

    // The live gap to that ghost. Ahead is the same green the rest of the UI uses
    // for something good; behind is the danger colour, so the number reads without
    // being parsed.
    if let Some(delta) = data.split_delta {
        right_children.push(
            Text(fmt_delta(delta))
                .size(theme::sp(26.0))
                .color(if delta <= 0 {
                    theme::ok()
                } else {
                    theme::danger()
                })
                .single_line(),
        );
    }

    if settings.practice {
        right_children.push(
            Text("PRACTICE")
                .size(theme::sp(14.0))
                .color(theme::accent())
                .single_line(),
        );
    }

    let mut children: Vec<View> = Vec::new();
    if !left_children.is_empty() {
        children.push(top_left(left_children));
    }
    children.push(top_right(right_children));

    ZStack(Modifier::new().fill_max_size().hit_passthrough()).child(children)
}

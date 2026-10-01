use repose_core::{AlignItems, Modifier, View};
use repose_ui::{Column, Text, TextStyle, ViewExt, ZStack};

use crate::app::state::AppData;
use crate::app::theme;
use crate::ui::widgets::fmt_time;

pub fn race_hud(data: &AppData) -> View {
    let time = fmt_time(data.race_time);
    let speed = data.speed_kmh;
    let checkpoint = data.checkpoint;
    let checkpoint_count = data.checkpoint_count;

    let left = Column(
        Modifier::new()
            .absolute()
            .offset(Some(theme::dp(20.0)), Some(theme::dp(20.0)), None, None)
            .gap(theme::dp(4.0))
            .padding(theme::dp(12.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(8.0))
            .hit_passthrough(),
    )
    .child(
        Text(time)
            .size(theme::sp(36.0))
            .color(theme::text())
            .single_line(),
    )
    .child(
        Text(format!("{speed:.0} km/h"))
            .size(theme::sp(22.0))
            .color(theme::accent())
            .single_line(),
    );

    let mut right_children = vec![
        Text(format!("CP {checkpoint} / {checkpoint_count}"))
            .size(theme::sp(22.0))
            .color(theme::text())
            .single_line(),
    ];
    let best = match data.best {
        Some(best) => format!("Best {}", fmt_time(best)),
        None => "Best —".to_string(),
    };
    right_children.push(
        Text(best)
            .size(theme::sp(16.0))
            .color(theme::text_dim())
            .single_line(),
    );

    let right = Column(
        Modifier::new()
            .absolute()
            .offset(None, Some(theme::dp(20.0)), Some(theme::dp(20.0)), None)
            .gap(theme::dp(4.0))
            .padding(theme::dp(12.0))
            .background(theme::background().with_alpha_f32(0.55))
            .clip_rounded(theme::dp(8.0))
            .align_items(AlignItems::END)
            .hit_passthrough(),
    )
    .child(right_children);

    ZStack(Modifier::new().fill_max_size().hit_passthrough()).child([left, right])
}

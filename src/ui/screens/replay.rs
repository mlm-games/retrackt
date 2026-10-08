use repose_core::{Modifier, View};
use repose_ui::{Box, Column, FlowRow, FlowRowConfig, Text, TextStyle, ViewExt};

use crate::app::state::{REPLAY_SPEEDS, ActionQueue, AppData, ReplayView, UiAct};
use crate::app::theme;
use crate::ui::fit;
use crate::ui::widgets::{
    accent_btn, dim_text, disabled_btn, fmt_ticks, ghost_btn, heading, hud_text, menu_btn, panel,
    pusher, screen_backdrop,
};

/// Width and height of the playhead bar, in dp. Thick enough to read as a bar at a
/// glance rather than as a rule the eye has to find.
const BAR_DP: f32 = 420.0;
const BAR_H_DP: f32 = 10.0;

/// A kept tape, playing back on the track it was recorded on.
pub fn replay_ui(data: &AppData, actions: &ActionQueue) -> View {
    let Some(view) = data.replay.as_ref() else {
        return screen_backdrop(panel(
            "",
            vec![
                heading("Replay"),
                dim_text("No tape is being watched."),
                menu_btn("Back to Ghosts", pusher(actions, UiAct::CloseReplay)),
            ],
        ));
    };

    let mut children = vec![heading("Replay")];

    // Single-line so the panel is not widened by an unbreakable name. Ellipsized
    // rather than clipped so the truncation is visible as such.
    children.push(
        Text(view.name.clone())
            .size(theme::sp(14.0))
            .color(theme::text_dim())
            .single_line()
            .overflow_ellipsize(),
    );

    // Position against the length, not the finish tick: a tape recorded past the
    // line has a finish time that is not where the recording stops, and a bar that
    // filled at the line would claim there was nothing after it.
    children.push(
        Text(format!("{}  /  {}", fmt_ticks(view.tick), fmt_ticks(view.len)))
            .size(theme::sp(24.0))
            .color(theme::accent())
            .single_line(),
    );
    // A tape with no finish tick was recorded but never crossed the line. Saying so
    // beats printing a zero the player has to interpret.
    children.push(match view.finish_tick {
        Some(finish) => dim_text(&format!(
            "Run finished at {}  ·  {}",
            fmt_ticks(finish),
            speed_label(view.speed)
        )),
        None => dim_text(&format!(
            "Run never finished  ·  {}",
            speed_label(view.speed)
        )),
    });
    children.push(playhead(view));
    children.push(transport(view, actions));
    // Kept inside the panel rather than laid out at its natural width, so the
    // wrapping button rows below it have a ceiling to wrap against instead of
    // the panel being widened by this sentence.
    children.push(
        Text(
            "Seeking re-runs the tape from the start line, so it lands on the exact \
             frame rather than an approximation of one.",
        )
        .size(theme::sp(14.0))
        .color(theme::text_dim())
        .max_lines(4),
    );
    // Wrapped, so the last one is reachable rather than sitting below the fold of a
    // panel that is already carrying the bar and the transport.
    children.push(
        FlowRow(
            Modifier::new().gap(theme::dp(8.0)),
            FlowRowConfig::default(),
        )
        .child(ghost_btn(
            "Back to Ghosts",
            pusher(actions, UiAct::CloseReplay),
        )),
    );

    screen_backdrop(panel("", children))
}

fn speed_label(speed: usize) -> String {
    format!("{:.2}x", REPLAY_SPEEDS[speed])
}

/// Play or pause, step, and the speed ladder.
fn transport(view: &ReplayView, actions: &ActionQueue) -> View {
    // Both rows wrap: four buttons of a 150 minimum come to 624, which is wider than
    // the panel's usable width once the playhead and the heading are accounted for.
    let playback = FlowRow(
        Modifier::new().gap(theme::dp(8.0)),
        FlowRowConfig::default(),
    )
    .child(accent_btn(
        if view.playing { "Pause" } else { "Play" },
        pusher(actions, UiAct::ReplayPlayPause),
    ))
    .child(ghost_btn(
        "Start",
        pusher(actions, UiAct::ReplayRestart),
    ))
    .child(ghost_btn(
        "-1s",
        pusher(actions, UiAct::ReplaySeek(-1)),
    ))
    .child(ghost_btn("+1s", pusher(actions, UiAct::ReplaySeek(1))));

    // Both ends of the ladder stop at the edge rather than stepping off it:
    // indexing past the speed table would panic, and a live button that does
    // nothing is worse than one that reads as unavailable.
    let speeds = FlowRow(
        Modifier::new().gap(theme::dp(8.0)),
        FlowRowConfig::default(),
    )
    .child(match view.speed.checked_sub(1) {
        Some(next) => ghost_btn(
            &format!("Slower ({})", speed_label(next)),
            pusher(actions, UiAct::ReplaySpeedStep(-1)),
        ),
        None => disabled_btn("Slower"),
    })
    .child(hud_text(&speed_label(view.speed)))
    .child(match REPLAY_SPEEDS.get(view.speed + 1) {
        Some(_) => ghost_btn(
            &format!("Faster ({})", speed_label(view.speed + 1)),
            pusher(actions, UiAct::ReplaySpeedStep(1)),
        ),
        None => disabled_btn("Faster"),
    });

    Column(Modifier::new().gap(theme::dp(8.0)))
        .child(playback)
        .child(speeds)
}

/// The playhead as a filled bar. Two nested boxes rather than a slider: repose-ui
/// has no progress widget, and a drag would re-simulate the run on every pointer
/// event, which is the one thing the seek is too expensive for.
/// The playhead bar tracks the panel's width. It used to be a fixed 420 dp, which
/// on a phone held upright is wider than the panel it sits in and pushes the
/// transport controls off the side.
fn playhead(view: &ReplayView) -> View {
    let filled = if view.len == 0 {
        0.0
    } else {
        (view.tick as f32 / view.len as f32).clamp(0.0, 1.0)
    };
    let bar = fit::fit(BAR_DP, 40.0);
    let track = Box(
        Modifier::new()
            .width(theme::dp(bar))
            .height(theme::dp(BAR_H_DP))
            .background(theme::text_dim().with_alpha_f32(0.3))
            .clip_rounded(theme::dp(BAR_H_DP / 2.0))
            .hit_passthrough(),
    );
    let fill = Box(
        Modifier::new()
            .width(theme::dp(bar * filled))
            .height(theme::dp(BAR_H_DP))
            .background(theme::accent())
            .clip_rounded(theme::dp(BAR_H_DP / 2.0))
            .hit_passthrough(),
    );
    // The fill goes inside the track rather than overlapping it, so it cannot spill
    // past the end however the corner rounding works out.
    track.child(fill)
}
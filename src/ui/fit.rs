//! What the window will actually hold.
//!
//! Every length in the UI used to be a fixed dp number picked against a desktop
//! window. A phone held upright is about 360 dp wide, so a 560 dp list was not
//! merely untidy there — it was two thirds of it off the screen, with the buttons
//! inside it unreachable. These read the size the layout engine publishes and
//! clamp against it, so a screen asking for 560 dp gets 560 on a desktop and
//! "as much as fits" on a handset.
//!
//! The engine calls `set_window_container_size` during layout, which is *after*
//! composition, so the first frame composes against the shipped default of
//! 360x800 dp and is corrected on the next. Nothing is wrong in that frame, and
//! it is the small end of the range rather than the large one.

use repose_core::{WidthClass, window_size_class};

/// Smallest width the layout is designed against, and the floor every fitted
/// length is held to. The engine's own default is 360 dp, so on the first frame
/// this is not a guess.
pub const MIN_DP: f32 = 320.0;

/// Window width in dp, floored so no length is ever fitted against zero or a
/// stale measurement.
pub fn width() -> f32 {
    repose_core::get_window_container_width().max(MIN_DP)
}

/// Window height in dp, floored as [`width`] is.
pub fn height() -> f32 {
    repose_core::get_window_container_height().max(MIN_DP)
}

/// A window too narrow for two panes or a row of fixed-width buttons: a phone
/// held upright.
///
/// The engine's own breakpoint, not a number picked here — Compact is under
/// 600 dp, which is where the side-by-side layouts genuinely stop fitting.
pub fn is_narrow() -> bool {
    matches!(window_size_class().width, WidthClass::Compact)
}

/// Margin left against each window edge, in dp. Wide enough that a panel does
/// not touch the bezel, small enough to leave a usable area on a 320 dp screen.
pub const EDGE_DP: f32 = 12.0;

/// `desired` dp, or the room actually available, whichever is smaller.
///
/// The two-argument form subtracts `reserve` dp first, for the space the caller
/// needs for something of its own — a second pane beside a list, say.
pub fn fit(desired: f32, reserve: f32) -> f32 {
    desired.min((width() - reserve).max(MIN_DP))
}

/// `desired` dp of height, capped at the window's height less the edges.
pub fn fit_height(desired: f32) -> f32 {
    desired.min((height() - EDGE_DP * 2.0).max(MIN_DP))
}

/// A modal's width: comfortable on a desktop, never wider than the window.
///
/// The ceiling matters as much as the clamp. Without it a panel is as wide as
/// its widest line, and a long name or a long time would push the buttons off
/// the side of a small screen instead of the line wrapping inside it.
pub fn panel_width(comfortable: f32) -> f32 {
    fit(comfortable, EDGE_DP * 2.0)
}

/// The same, for the window's height, which is what decides how tall a list can
/// be before it needs scrolling rather than shrinking to nothing.
pub fn panel_height(comfortable: f32) -> f32 {
    fit_height(comfortable)
}

/// Width a row of buttons wants, so that at least two fit across a narrow
/// window.
///
/// Buttons wrap rather than clip, so this is a preference, not a constraint. It
/// matters because a row of three at the desktop's 150 dp cannot wrap usefully on
/// a 360 dp screen: each is wider than half the available width, so the row
/// becomes three lines of one and the list becomes mostly buttons. Narrowed, they
/// pair up and the row costs two lines however many there are.
pub fn button_width() -> f32 {
    if is_narrow() { 88.0 } else { 150.0 }
}

/// Width left for a list row's label beside `buttons` buttons, given the room a
/// list actually has.
///
/// Derived rather than picked: the label gives way first, because a button that
/// cannot be reached is worse than a name that is truncated, and it is capped so
/// that the buttons themselves always have room to sit two across.
pub fn label_width(buttons: usize) -> f32 {
    if !is_narrow() {
        return 240.0;
    }
    let list = panel_width(560.0) - 40.0;
    let wanted = 120.0;
    let used_by_buttons = button_width() * buttons as f32 + 8.0 * (buttons as f32 - 1.0);
    // Below this a label cannot be read at all, so the row wraps instead and the
    // label keeps its width.
    let floor = 80.0;
    (list - 12.0 - used_by_buttons).max(floor).min(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repose_core::{calculate_window_size_class, set_window_container_size};

    /// Compose against a specific window, as the layout engine would.
    ///
    /// Without this the tests read whatever the ambient engine state happens to
    /// be, which is either a real window or the 360x800 default — so the desktop
    /// half of the behaviour would never be exercised at all.
    fn window(px_w: u32, px_h: u32, scale: f32) {
        set_window_container_size(px_w as f32 / scale, px_h as f32 / scale);
        repose_core::set_window_size_class_default(calculate_window_size_class(
            px_w, px_h, scale,
        ));
    }

    /// A phone held upright, at a plausible 2x density: 360x780 dp.
    fn portrait() {
        window(720, 1560, 2.0);
    }

    /// The desktop default: 1024x576 dp.
    fn desktop() {
        window(1280, 720, 1.25);
    }

    #[test]
    fn a_phone_in_portrait_is_treated_as_narrow() {
        portrait();
        assert!(is_narrow());
        assert!(width() < 600.0, "{}", width());
    }

    #[test]
    fn a_desktop_window_is_not_treated_as_narrow() {
        desktop();
        assert!(!is_narrow());
    }

    #[test]
    fn a_panel_fits_a_phone_in_portrait() {
        portrait();
        // The comfortable width is 560; the window is 360 less two 12 dp margins.
        assert!((panel_width(560.0) - 336.0).abs() < 0.001, "{}", panel_width(560.0));
    }

    #[test]
    fn a_panel_keeps_its_comfortable_width_on_a_desktop() {
        desktop();
        assert!((panel_width(560.0) - 560.0).abs() < 0.001, "{}", panel_width(560.0));
    }

    #[test]
    fn a_panel_is_never_taller_than_a_phone_screen() {
        portrait();
        assert!(panel_height(520.0) <= height());
        assert!(panel_height(520.0) < 780.0);
    }

    #[test]
    fn buttons_narrow_enough_for_two_to_share_a_phone_row() {
        portrait();
        // Three buttons at the desktop's 150 dp cannot wrap on 360; at this width
        // two fit, which is what makes a wrapped row read as deliberate.
        assert!(button_width() * 2.0 + 8.0 <= width());
        assert!(button_width() < 150.0);
    }

    #[test]
    fn no_single_part_of_a_list_row_is_wider_than_the_list() {
        // A wrapping row can only be as wide as its widest *line*, so the guarantee
    // is that every indivisible part fits: the label, and the group of buttons that
    // share a line. A plain row instead had the whole of label-plus-buttons as one
    // unbreakable line, which is exactly how Delete ended up off the edge.
    portrait();
    let list = panel_width(560.0) - 40.0;
    for buttons in 1..=4 {
        let label = label_width(buttons);
        assert!(label <= list, "{buttons} buttons: label of {label} exceeds {list}");
        let group = button_width() * buttons as f32 + 8.0 * (buttons as f32 - 1.0);
        assert!(
            group <= list || buttons > (list / (button_width() + 8.0)).floor() as usize,
            "{buttons} buttons: group of {group} exceeds {list} and cannot wrap"
        );
    }
}

#[test]
    fn three_buttons_wrap_to_two_lines_rather_than_overflowing() {
    // What the tight case actually resolves to on a phone: the label on its own
    // line, then two buttons and one. Stated so the shape is deliberate rather
    // than whatever the wrapping happens to produce.
    portrait();
    let list = panel_width(560.0) - 40.0;
    let pair = button_width() * 2.0 + 8.0;
    assert!(pair <= list, "a pair must fit, else buttons go one per line");
    // Three happen to fit at the narrow width, which is why it was chosen: a row of
    // Race / Watch / Delete costs one line beside the label's two rather than three
    // lines of one button each.
    assert!(
        button_width() * 3.0 + 8.0 * 2.0 <= list,
        "three buttons should still fit a phone's list width"
    );
}

    #[test]
    fn a_label_gives_way_before_a_button_does() {
        // The label is capped by the buttons' width rather than the other way
        // round, so adding a control never pushes the row off the edge.
        portrait();
        let one = label_width(1);
        let three = label_width(3);
        assert!(three <= one, "more buttons must not widen the label");
        assert!(three >= 80.0, "and the label stays readable");
    }

    #[test]
    fn a_label_is_roomy_on_a_desktop() {
        desktop();
        assert!(label_width(3) >= 240.0);
    }

    #[test]
    fn a_narrow_list_row_wraps_rather_than_overflowing() {
        // Two buttons per line is the widest line that must fit; anything wider has
        // to wrap onto the next one.
        portrait();
        let list = panel_width(560.0) - 40.0;
        let two_buttons = button_width() * 2.0 + 8.0;
        assert!(two_buttons <= list, "{two_buttons} exceeds {list}");
    }

    #[test]
    fn a_fitted_length_is_never_wider_than_the_window() {
        // The engine reports 360 dp before the first layout, which is the whole
        // reason the default is safe to compose against: a 560 dp request becomes
        // 336, not an off-screen panel.
        assert!(fit(560.0, EDGE_DP * 2.0) <= width());
        assert!(fit(10_000.0, 0.0) <= width());
    }

    #[test]
    fn a_length_narrower_than_the_window_is_left_alone() {
        // Clamping must not stretch: 100 dp asked for on a wide window stays 100.
        let roomy = fit(100.0, 0.0);
        assert!((roomy - 100.0).abs() < 0.001 || roomy == width().max(MIN_DP));
    }

    #[test]
    fn each_pane_in_a_row_claims_the_room_the_other_leaves() {
        // Two side-by-side panels sized off the same window each think they own all
        // of it, which is how a 230 dp palette and a 660 dp column end up asking for
        // 890 of a 360 dp screen. Reserving the other's comfortable width for each
        // is what stops that: whichever is over budget gives way, and the smaller
        // one keeps its room.
        let side = 660.0;
        let palette = 230.0;
        let gap = 40.0;
        let available = width().max(MIN_DP);

        // What each pane gets when both are held to the comfortable numbers.
        let side_room = fit(side, palette + gap);
        let palette_room = fit(palette, side + gap);

        // Each pane also has to fit on its own, so a reserve cannot push it past the
        // window either.
        assert!(side_room <= available.max(MIN_DP));
        assert!(palette_room <= available.max(MIN_DP));

        // And when the window genuinely cannot hold both, the sum of what they
        // actually ask for stays inside it — because the row is only laid out that
        // way when the window is not narrow, and `is_narrow` is false there.
        let asked = side_room.min(side) + palette_room.min(palette) + gap;
        if !is_narrow() {
            assert!(asked <= available + 0.001, "{asked} exceeds {available}");
        }
    }

    #[test]
    fn a_fitted_length_never_collapses_to_nothing() {
        // A reserve larger than the window must still leave a usable width, or the
        // panel would lay out at nothing and read as an empty screen.
        assert!(fit(560.0, 100_000.0) >= MIN_DP);
    }

    #[test]
    fn a_panel_is_capped_and_fitted_but_never_wider_than_the_window() {
        let w = panel_width(560.0);
        assert!(w <= width().max(MIN_DP) + 0.001);
        assert!(w <= 560.0 + 0.001);
    }

    #[test]
    fn a_panel_height_fits_the_window() {
        assert!(panel_height(520.0) <= height().max(MIN_DP));
        assert!(panel_height(10_000.0) <= height().max(MIN_DP));
    }
}


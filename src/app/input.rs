use game_utils_vehicle::VehicleInput;
use repame_input::GamepadState;
use repame_shell::{GamepadPoller, PadBank};
use repose_core::input::PhysicalKey;
use repose_core::runtime::Scheduler;
use repose_core::{Dp, dp_to_px};
use retrackt_format::PackedInput;

use crate::app::state::StickView;

/// Forward speed below which held brake falls through to reverse.
const REVERSE_ENGAGE: f32 = 1.0;

const STICK_DEADZONE: f32 = 0.15;

/// On-screen touch stick diameters in dp; the knob travels the gap between
/// them, and that same travel is what maps a drag to full deflection.
pub const STICK_BASE_DP: f32 = 160.0;
pub const STICK_KNOB_DP: f32 = 80.0;
pub const STICK_TRAVEL_DP: f32 = (STICK_BASE_DP - STICK_KNOB_DP) / 2.0;

const STICK_PEDAL: f32 = 0.25;

/// Face buttons and the D-pad reach us as rising edges only, so anything
/// held down binds to triggers, shoulders and the stick instead.
#[derive(Default)]
struct PadControls {
    steer: f32,
    throttle: bool,
    brake: bool,
    handbrake: bool,
    boost: bool,
    restart: bool,
    recover: bool,
    quit: bool,
}

fn pad_controls(pad: &GamepadState) -> PadControls {
    let x = pad.left_stick.x;
    PadControls {
        steer: if x.abs() >= STICK_DEADZONE {
            x.clamp(-1.0, 1.0)
        } else {
            0.0
        },
        throttle: pad.right_trigger_held,
        brake: pad.left_trigger_held,
        handbrake: pad.right_shoulder_held,
        boost: pad.left_shoulder_held,
        restart: pad.north_pressed,
        recover: pad.west_pressed,
        quit: pad.east_pressed,
    }
}

fn travel_px() -> f32 {
    dp_to_px(Dp(STICK_TRAVEL_DP)).0
}

fn clamp_to_circle(x: f32, y: f32, r: f32) -> (f32, f32) {
    let len = (x * x + y * y).sqrt();
    if len > r && len > 0.0 {
        (x * r / len, y * r / len)
    } else {
        (x, y)
    }
}

/// `(steer, throttle, brake)` from a knob offset in physical px. Screen y
/// grows downward, so up is negative.
fn stick_axis(knob: (f32, f32), travel: f32) -> (f32, bool, bool) {
    let (x, y) = knob;
    let steer = if travel > 0.0 {
        (x / travel).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let pedal = travel * STICK_PEDAL;
    (steer, y < -pedal, y > pedal)
}

/// Anchored at the first touch of a race and grabbed by whichever finger is
/// down; the knob returns to centre on release but the anchor stays put.
#[derive(Default)]
struct TouchStick {
    visible: bool,
    anchor: (f32, f32),
    finger: Option<u64>,
    knob: (f32, f32),
}

impl TouchStick {
    fn update(&mut self, sched: &Scheduler, racing: bool) {
        if !racing {
            return;
        }
        if let Some(id) = self.finger {
            match sched.touch_points.iter().find(|(tid, _, _)| *tid == id) {
                Some((_, x, y)) => {
                    let (dx, dy) = (x - self.anchor.0, y - self.anchor.1);
                    self.knob = clamp_to_circle(dx, dy, travel_px());
                }
                None => {
                    self.finger = None;
                    self.knob = (0.0, 0.0);
                }
            }
        } else if let Some((id, x, y)) = sched.touch_points.first() {
            self.anchor = (*x, *y);
            self.knob = (0.0, 0.0);
            self.finger = Some(*id);
            self.visible = true;
        }
    }
}

/// Cursor movement and one-shot editor commands, resolved from held keys.
///
/// Separate from the vehicle map because the editor reuses the arrow and WASD
/// keys: while the editor is up the car is parked, and every key here moves the
/// cursor or edits a piece instead.
#[derive(Default)]
struct EditorKeys {
    /// Cell step to apply this frame, summed over the axes held. Screen up is
    /// -Z and right is +X, so the axis and its sign live in one table and the two
    /// cannot disagree.
    step: [i16; 3],
    /// How long each axis has been held, for the repeat delay.
    held: [u32; 3],
    /// One-shot commands, all edge-latched. Level-triggered, a held Ctrl+C would
    /// copy once per frame and flood the notice bar.
    pressed: EditorCommand,
    down: EditorCommand,
}

/// The editor's one-shot commands. Public because the runtime reads them to turn
/// them into actions; the type is what crosses the module boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorCommand {
    pub place: bool,
    pub delete: bool,
    pub rotate: bool,
    pub duplicate: bool,
    pub copy: bool,
    pub paste: bool,
    pub undo: bool,
    pub redo: bool,
    pub select_all: bool,
    pub select_route: bool,
    pub select_none: bool,
}

impl EditorCommand {
    fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Frames a direction must be held before it repeats, and then how often.
///
/// Without a repeat, crossing a long track means one key press per cell; without
/// a delay the first repeat lands while the key is still going down.
const REPEAT_DELAY_FRAMES: u32 = 18;
const REPEAT_PERIOD_FRAMES: u32 = 4;

/// Axis and direction of each cursor key, as `(key, axis, step)`.
const CURSOR_KEYS: [(PhysicalKey, usize, i16); 8] = [
    (PhysicalKey::ArrowUp, 2, -1),
    (PhysicalKey::KeyW, 2, -1),
    (PhysicalKey::ArrowRight, 0, 1),
    (PhysicalKey::KeyD, 0, 1),
    (PhysicalKey::ArrowDown, 2, 1),
    (PhysicalKey::KeyS, 2, 1),
    (PhysicalKey::ArrowLeft, 0, -1),
    (PhysicalKey::KeyA, 0, -1),
];

impl EditorKeys {
    /// Resolve the held keys into this frame's commands.
    fn poll(&mut self, sched: &Scheduler) {
        let held = |key: PhysicalKey| sched.held_keys.contains(&key);
        let ctrl = held(PhysicalKey::ControlLeft) || held(PhysicalKey::ControlRight);
        let shift = held(PhysicalKey::ShiftLeft) || held(PhysicalKey::ShiftRight);

        self.step = [0; 3];
        for (key, axis, direction) in CURSOR_KEYS {
            // A modified key is not a cursor key. Ctrl+D duplicates and Shift+A
            // selects the route; neither should also slide the cursor, or holding
            // a modifier would quietly move the piece about to be placed.
            if (ctrl || shift) || !held(key) {
                self.held[axis] = 0;
                continue;
            }
            self.held[axis] += 1;
            let n = self.held[axis];
            let repeats = n >= REPEAT_DELAY_FRAMES
                && (n - REPEAT_DELAY_FRAMES) % REPEAT_PERIOD_FRAMES == 0;
            if n == 1 || repeats {
                self.step[axis] += direction;
            }
        }

        let now = EditorCommand {
            place: held(PhysicalKey::Enter) || held(PhysicalKey::NumpadEnter),
            delete: held(PhysicalKey::Delete) || held(PhysicalKey::Backspace),
            rotate: held(PhysicalKey::KeyR),
            duplicate: ctrl && held(PhysicalKey::KeyD),
            copy: ctrl && held(PhysicalKey::KeyC),
            paste: ctrl && held(PhysicalKey::KeyV),
            undo: ctrl && held(PhysicalKey::KeyZ),
            redo: ctrl && held(PhysicalKey::KeyY),
            select_all: ctrl && held(PhysicalKey::KeyA),
            select_route: shift && held(PhysicalKey::KeyA),
            select_none: held(PhysicalKey::Escape),
        };
        // Newly down this frame only. A command whose key is released clears the
        // latch, so it can be pressed again without an intervening release.
        self.pressed = EditorCommand {
            place: now.place && !self.down.place,
            delete: now.delete && !self.down.delete,
            rotate: now.rotate && !self.down.rotate,
            duplicate: now.duplicate && !self.down.duplicate,
            copy: now.copy && !self.down.copy,
            paste: now.paste && !self.down.paste,
            undo: now.undo && !self.down.undo,
            redo: now.redo && !self.down.redo,
            select_all: now.select_all && !self.down.select_all,
            select_route: now.select_route && !self.down.select_route,
            select_none: now.select_none && !self.down.select_none,
        };
        self.down = now;
    }

    /// Reset the repeat counters when the editor closes.
    ///
    /// `down` is deliberately kept. It is the record of which keys were held on
    /// the frame the editor was last polled, and clearing it would make a key
    /// still held at that moment read as a *fresh* press the next time the editor
    /// opens — a command fired by the player letting go of nothing.
    fn release(&mut self) {
        self.held = [0; 3];
        self.step = [0; 3];
        self.pressed = EditorCommand::default();
    }
}

pub struct InputState {
    /// Signed forward speed of the car, refreshed by the app each frame so
    /// `poll` can tell "stopped" from "still braking".
    pub forward_speed: f32,
    /// Edge-latched: true only on the frame the restart key went down.
    pub restart: bool,
    /// Edge-latched: true only on the frame the quit key went down.
    pub quit: bool,
    /// Edge-latched: true only on the frame the back-to-checkpoint key went down.
    /// Practice only: a timed run may not skip the road between gates.
    pub recover: bool,
    /// Editor keys, edge-latched and repeated while held.
    editor: EditorKeys,
    vehicle: VehicleInput,
    reverse_latched: bool,
    restart_latched: bool,
    recover_latched: bool,
    quit_latched: bool,
    poller: GamepadPoller,
    pads: PadBank,
    stick: TouchStick,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            forward_speed: 0.0,
            restart: false,
            quit: false,
            recover: false,
            editor: EditorKeys::default(),
            vehicle: VehicleInput::neutral(),
            reverse_latched: false,
            restart_latched: false,
            recover_latched: false,
            quit_latched: false,
            poller: GamepadPoller::new(),
            pads: PadBank::default(),
            stick: TouchStick::default(),
        }
    }

    pub fn poll(&mut self, sched: &Scheduler, racing: bool) {
        self.stick.update(sched, racing);
        let (touch_steer, touch_throttle, touch_brake) = stick_axis(self.stick.knob, travel_px());

        let events = self.poller.poll();
        self.pads.feed(events);
        let drained = self.pads.drain_with_id();
        let pad = drained
            .first()
            .map(|(_, state)| pad_controls(state))
            .unwrap_or_default();

        let held = |key: PhysicalKey| sched.held_keys.contains(&key);
        let throttle =
            held(PhysicalKey::KeyW) || held(PhysicalKey::ArrowUp) || pad.throttle || touch_throttle;
        let brake =
            held(PhysicalKey::KeyS) || held(PhysicalKey::ArrowDown) || pad.brake || touch_brake;
        let left = held(PhysicalKey::KeyA) || held(PhysicalKey::ArrowLeft);
        let right = held(PhysicalKey::KeyD) || held(PhysicalKey::ArrowRight);

        if !brake || throttle {
            self.reverse_latched = false;
        } else if !self.reverse_latched && self.forward_speed.abs() < REVERSE_ENGAGE {
            self.reverse_latched = true;
        }

        let mut v = VehicleInput::neutral();
        if throttle {
            v.throttle = 1.0;
        } else if self.reverse_latched {
            v.throttle = -1.0;
        } else if brake {
            v.brake = 1.0;
        }
        if touch_steer != 0.0 {
            v.steer = touch_steer;
        } else if pad.steer != 0.0 {
            v.steer = pad.steer;
        } else {
            match (left, right) {
                (true, false) => v.steer = -1.0,
                (false, true) => v.steer = 1.0,
                _ => {}
            }
        }
        if held(PhysicalKey::Space) || pad.handbrake {
            v.handbrake = 1.0;
        }
        if held(PhysicalKey::ShiftLeft) || held(PhysicalKey::ShiftRight) || pad.boost {
            v.boost = 1.0;
        }
        self.vehicle = v.clamped();

        let restart = held(PhysicalKey::KeyR) || pad.restart;
        self.restart = restart && !self.restart_latched;
        self.restart_latched = restart;

        let quit = held(PhysicalKey::Escape) || pad.quit;
        self.quit = quit && !self.quit_latched;
        self.quit_latched = quit;

        let recover = held(PhysicalKey::KeyC) || pad.recover;
        self.recover = recover && !self.recover_latched;
        self.recover_latched = recover;
    }

    /// Quantised form of this frame's intent: the value the simulation consumes
    /// and the tape stores, so neither can drift from the other.
    pub fn packed(&self) -> PackedInput {
        let v = self.vehicle;
        PackedInput::new(
            v.steer,
            v.throttle,
            v.brake,
            v.handbrake > 0.0,
            v.boost > 0.0,
            false,
        )
    }

    pub fn stick_view(&self) -> Option<StickView> {
        self.stick.visible.then_some(StickView {
            anchor: self.stick.anchor,
            knob: self.stick.knob,
        })
    }

    /// Editor commands for this frame, edge-latched and repeated as held.
    ///
    /// Only called while the editor is up. Every other frame calls
    /// [`Self::release_editor`] so a key held on the way out of the editor is
    /// not still "down" when it is next pressed.
    pub fn poll_editor(&mut self, sched: &Scheduler) -> EditorFrame {
        self.editor.poll(sched);
        EditorFrame {
            step: self.editor.step,
            pressed: self.editor.pressed,
        }
    }

    pub fn release_editor(&mut self) {
        self.editor.release();
    }
}

/// One frame of editor input.
#[derive(Clone, Copy, Debug, Default)]
pub struct EditorFrame {
    /// Cell step for the cursor, in grid axes.
    pub step: [i16; 3],
    /// Commands whose key went down this frame.
    pub pressed: EditorCommand,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repame_shell::PadBridge;
    use repose_core::input::GamepadAxis;

    fn pad(build: impl FnOnce(&mut PadBridge)) -> GamepadState {
        let mut bridge = PadBridge::default();
        build(&mut bridge);
        bridge.snapshot()
    }

    #[test]
    fn stick_sign_follows_vehicle_steer_convention() {
        let right = pad(|b| b.axis(GamepadAxis::LeftStickX, 0.8));
        assert_eq!(pad_controls(&right).steer, 0.8);

        let left = pad(|b| b.axis(GamepadAxis::LeftStickX, -0.8));
        assert_eq!(pad_controls(&left).steer, -0.8);
    }

    #[test]
    fn stick_deadzone_silences_resting_tilt() {
        let resting = pad(|b| b.axis(GamepadAxis::LeftStickX, 0.1));
        assert_eq!(pad_controls(&resting).steer, 0.0);
    }

    #[test]
    fn triggers_and_shoulders_drive_the_car() {
        let controls = pad(|b| {
            b.axis(GamepadAxis::RightTrigger, 0.9);
            b.axis(GamepadAxis::LeftTrigger, 0.9);
            b.button(repose_core::input::GamepadButton::RightShoulder, true);
            b.button(repose_core::input::GamepadButton::LeftShoulder, true);
        });
        let c = pad_controls(&controls);
        assert!(c.throttle && c.brake && c.handbrake && c.boost);
    }

    #[test]
    fn face_buttons_fire_restart_and_quit_edges() {
        let controls = pad(|b| {
            b.button(repose_core::input::GamepadButton::North, true);
            b.button(repose_core::input::GamepadButton::East, true);
        });
        let c = pad_controls(&controls);
        assert!(c.restart && c.quit);
    }

    #[test]
    fn touch_up_throttles_down_brakes_right_steers() {
        let (steer, throttle, brake) = stick_axis((0.0, -STICK_TRAVEL_DP), STICK_TRAVEL_DP);
        assert_eq!(steer, 0.0);
        assert!(throttle && !brake);

        let (_, throttle, brake) = stick_axis((0.0, STICK_TRAVEL_DP), STICK_TRAVEL_DP);
        assert!(!throttle && brake);

        let (steer, _, _) = stick_axis((STICK_TRAVEL_DP, 0.0), STICK_TRAVEL_DP);
        assert_eq!(steer, 1.0);
        let (steer, _, _) = stick_axis((-STICK_TRAVEL_DP, 0.0), STICK_TRAVEL_DP);
        assert_eq!(steer, -1.0);
    }

    #[test]
    fn touch_track_stays_put_and_releases_cleanly() {
        let mut sched = Scheduler::new();
        let mut stick = TouchStick::default();
        assert!(!stick.visible);

        sched.touch_points.push((7, 100.0, 200.0));
        stick.update(&sched, true);
        assert_eq!(stick.anchor, (100.0, 200.0));
        assert_eq!(stick.finger, Some(7));

        sched.touch_points = vec![(7, 10_000.0, 200.0)];
        stick.update(&sched, true);
        let travel = travel_px();
        assert!((stick.knob.0 - travel).abs() < 1e-3);
        assert_eq!(stick.knob.1, 0.0);

        sched.touch_points.clear();
        stick.update(&sched, true);
        assert_eq!(stick.finger, None);
        assert_eq!(stick.knob, (0.0, 0.0));
        assert!(stick.visible);
    }

    fn sched_with(keys: &[PhysicalKey]) -> Scheduler {
        let mut sched = Scheduler::new();
        sched.held_keys = keys.iter().copied().collect();
        sched
    }

    /// Hold `keys` for `frames` polls and collect every step produced.
    fn steps_while_held(keys: &[PhysicalKey], frames: u32) -> Vec<[i16; 3]> {
        let sched = sched_with(keys);
        let mut input = InputState::new();
        (0..frames).map(|_| input.poll_editor(&sched).step).collect()
    }

    #[test]
    fn a_direction_key_steps_once_then_repeats_after_a_delay() {
        let frames = steps_while_held(&[PhysicalKey::ArrowUp], 40);

        assert_eq!(frames[0], [0, 0, -1], "the press itself fires");
        // Silent until the delay has elapsed, counting from the frame *after* the
        // press: frame 0 already fired, so the quiet run is 1..DELAY.
        for (n, step) in frames.iter().enumerate().take(REPEAT_DELAY_FRAMES as usize).skip(1) {
            assert_eq!(*step, [0, 0, 0], "frame {n} must be inside the delay");
        }
        // Then it fires exactly once per period.
        assert_eq!(
            frames[REPEAT_DELAY_FRAMES as usize], [0, 0, -1],
            "the first repeat lands on the delay"
        );
        for n in 1..REPEAT_PERIOD_FRAMES {
            assert_eq!(
                frames[(REPEAT_DELAY_FRAMES + n) as usize],
                [0, 0, 0],
                "frame {n} of the period must be silent"
            );
        }
        assert_eq!(
            frames[(REPEAT_DELAY_FRAMES + REPEAT_PERIOD_FRAMES) as usize],
            [0, 0, -1],
            "and then it repeats"
        );
    }

    #[test]
    fn the_repeat_stays_on_its_period_rather_than_drifting() {
        let frames = steps_while_held(&[PhysicalKey::ArrowRight], 80);
        for (n, step) in frames.iter().enumerate() {
            let expected = n == 0
                || (n as u32 >= REPEAT_DELAY_FRAMES
                    && (n as u32 - REPEAT_DELAY_FRAMES) % REPEAT_PERIOD_FRAMES == 0);
            assert_eq!(
                *step == [1, 0, 0],
                expected,
                "frame {n}: step {:?} but expected a step = {expected}",
                step
            );
        }
    }

    #[test]
    fn releasing_a_direction_resets_its_repeat() {
        let mut input = InputState::new();
        for _ in 0..REPEAT_DELAY_FRAMES + 2 {
            input.poll_editor(&sched_with(&[PhysicalKey::ArrowRight]));
        }
        assert_eq!(input.poll_editor(&sched_with(&[])).step, [0, 0, 0]);
        assert_eq!(
            input.poll_editor(&sched_with(&[PhysicalKey::ArrowRight])).step,
            [1, 0, 0],
            "held again, it must fire at once rather than waiting out the delay"
        );
    }

    #[test]
    fn the_wasd_letters_mirror_the_arrow_keys() {
        // A separate input each time: the repeat counter is state, so the second
        // poll on the same instance would be a different frame, not a comparison.
        for (letter, arrow) in [
            (PhysicalKey::KeyW, PhysicalKey::ArrowUp),
            (PhysicalKey::KeyS, PhysicalKey::ArrowDown),
            (PhysicalKey::KeyA, PhysicalKey::ArrowLeft),
            (PhysicalKey::KeyD, PhysicalKey::ArrowRight),
        ] {
            assert_eq!(
                steps_while_held(&[letter], 1),
                steps_while_held(&[arrow], 1),
                "{letter:?} must move the cursor the way {arrow:?} does"
            );
        }
    }

    #[test]
    fn opposite_directions_cancel_rather_than_accelerate_the_cursor() {
        assert_eq!(
            steps_while_held(&[PhysicalKey::ArrowUp, PhysicalKey::ArrowDown], 1),
            vec![[0, 0, 0]],
            "a cursor cannot move two ways at once"
        );
    }

    #[test]
    fn a_command_fires_on_the_press_and_not_while_held() {
        let mut input = InputState::new();
        let sched = sched_with(&[PhysicalKey::ControlLeft, PhysicalKey::KeyC]);

        assert!(input.poll_editor(&sched).pressed.copy, "the press itself");
        for _ in 0..5 {
            assert!(
                !input.poll_editor(&sched).pressed.copy,
                "a held Ctrl+C must not copy once per frame"
            );
        }
        input.poll_editor(&sched_with(&[]));
        assert!(
            input.poll_editor(&sched).pressed.copy,
            "and again once released"
        );
    }

    #[test]
    fn a_modified_direction_key_does_not_move_the_cursor() {
        let mut input = InputState::new();
        let frame =
            input.poll_editor(&sched_with(&[PhysicalKey::ControlLeft, PhysicalKey::KeyD]));
        assert_eq!(
            frame.step, [0, 0, 0],
            "Ctrl+D duplicates; it is not cursor-right"
        );
        assert!(frame.pressed.duplicate);
    }

    #[test]
    fn shift_a_selects_the_route_and_plain_a_moves_the_cursor() {
        let shifted =
            steps_while_held(&[PhysicalKey::ShiftLeft, PhysicalKey::KeyA], 1);
        assert_eq!(shifted, vec![[0, 0, 0]], "a modified key is not a cursor key");

        let mut input = InputState::new();
        let frame = input.poll_editor(&sched_with(&[PhysicalKey::ShiftLeft, PhysicalKey::KeyA]));
        assert!(frame.pressed.select_route);
    }

    #[test]
    fn undo_and_redo_accept_either_control_side() {
        for ctrl in [PhysicalKey::ControlLeft, PhysicalKey::ControlRight] {
            let mut input = InputState::new();
            assert!(input.poll_editor(&sched_with(&[ctrl, PhysicalKey::KeyZ])).pressed.undo);
            assert!(input.poll_editor(&sched_with(&[ctrl, PhysicalKey::KeyY])).pressed.redo);
        }
    }

    #[test]
    fn leaving_the_editor_does_not_fire_a_key_that_was_still_held() {
        let mut input = InputState::new();
        let sched = sched_with(&[PhysicalKey::Enter]);
        assert!(input.poll_editor(&sched).pressed.place);

        input.release_editor();
        assert!(
            !input.poll_editor(&sched).pressed.place,
            "a key held while leaving the editor must not act on the next press"
        );
        // But releasing and pressing again does fire it.
        input.poll_editor(&sched_with(&[]));
        assert!(input.poll_editor(&sched).pressed.place);
    }

    #[test]
    fn leaving_the_editor_resets_the_repeat_so_a_held_key_does_not_run() {
        let mut input = InputState::new();
        let sched = sched_with(&[PhysicalKey::ArrowRight]);
        for _ in 0..REPEAT_DELAY_FRAMES + 2 {
            input.poll_editor(&sched);
        }
        input.release_editor();
        // Quiet, because the key was already down when the editor closed.
        assert_eq!(input.poll_editor(&sched).step, [0, 0, 0]);
    }

    #[test]
    fn nothing_is_pressed_when_no_editor_key_is_held() {
        let mut input = InputState::new();
        let frame = input.poll_editor(&sched_with(&[]));
        assert_eq!(frame.step, [0, 0, 0]);
        assert!(frame.pressed.is_empty());
    }
}

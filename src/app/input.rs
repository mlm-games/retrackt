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
/// down; the knob returns to centre on release.
///
/// Shown only while a finger holds it, so the stick is not left sitting on the
/// track between runs. The anchor outlives the touch — it is wherever the ring
/// had slid to — so the next touch continues from the last one's position rather
/// than jumping back to wherever the player happened to tap.
#[derive(Default)]
struct TouchStick {
    anchor: (f32, f32),
    finger: Option<u64>,
    knob: (f32, f32),
}

impl TouchStick {
    fn update(&mut self, sched: &Scheduler, racing: bool) {
        if !racing {
            // Dropped here rather than left as it was: a race that ends with a
            // finger still down would otherwise open the next one with a stick
            // already held, deflected to wherever that finger was.
            self.finger = None;
            self.knob = (0.0, 0.0);
            return;
        }
        if let Some(id) = self.finger {
            match sched.touch_points.iter().find(|(tid, _, _)| *tid == id) {
                Some((_, x, y)) => {
                    let (dx, dy) = (x - self.anchor.0, y - self.anchor.1);
                    let travel = travel_px();
                    let len = (dx * dx + dy * dy).sqrt();
                    // Past full travel the ring comes along with the finger instead
                    // of pinning to where the drag began, so a thumb that runs out
                    // of room keeps steering rather than jamming against a stop that
                    // never moves.
                    let (dx, dy) = if len > travel && len > 0.0 {
                        let k = travel / len;
                        self.anchor = (x - dx * k, y - dy * k);
                        (dx * k, dy * k)
                    } else {
                        (dx, dy)
                    };
                    self.knob = (dx, dy);
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
        }
    }
}

/// Cursor height and one-shot editor commands, resolved from held keys.
///
/// Separate from the vehicle map because the editor reuses the arrow and WASD
/// keys: while the editor is up the car is parked, and those keys move the
/// camera instead. The cursor itself follows the mouse; only its height is a
/// key press, because height is the one axis a ground-plane click knows nothing
/// about.
#[derive(Default)]
struct EditorKeys {
    /// Camera movement, as `(forward, right)` in -1..=1, for a key that is held
    /// rather than tapped.
    move_cam: [f32; 2],
    /// Camera turn, -1 left to +1 right, likewise continuous.
    turn_cam: f32,
    /// Cells of height to move this frame, after the repeat delay.
    step_y: i16,
    /// How long the height keys have been held, for the repeat delay.
    held_y: u32,
    /// Whether the height key was already down when the editor opened, and so
    /// is not a press. Cleared by a frame that releases it.
    muted_y: bool,
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
    /// Turn the armed piece before it is placed. Edge-latched like the rest of
    /// the one-shot commands: held down, it would spin the piece once per frame.
    pub turn_armed: bool,
    /// Put connector snapping on or off.
    pub toggle_snap: bool,
    pub duplicate: bool,
    pub copy: bool,
    pub paste: bool,
    pub undo: bool,
    pub redo: bool,
    pub select_all: bool,
    pub select_route: bool,
    pub select_none: bool,
    /// Take the piece under the pointer.
    pub select_hovered: bool,
    /// Raise or lower the edited height by one cell.
    pub height_up: bool,
    pub height_down: bool,
}

impl EditorCommand {
    fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Frames a key must be held before the height repeats, and then how often.
///
/// Without a repeat, climbing a long ramp is one key press per half cell; without
/// a delay the first repeat lands while the key is still going down.
const REPEAT_DELAY_FRAMES: u32 = 18;
const REPEAT_PERIOD_FRAMES: u32 = 4;

/// Camera movement keys, as `(key, axis, direction)`. Axis 0 runs forwards and
/// back, axis 1 left and right, both across the ground plane rather than the
/// screen: dragging the view round with the mouse must not silently turn the
/// keyboard's notion of "forwards" with it.
///
/// Arrows as well as WASD, because the arrows are where a hand sits when it is
/// steering with the mouse.
const CAMERA_KEYS: [(PhysicalKey, usize, f32); 8] = [
    (PhysicalKey::ArrowUp, 0, 1.0),
    (PhysicalKey::KeyW, 0, 1.0),
    (PhysicalKey::ArrowDown, 0, -1.0),
    (PhysicalKey::KeyS, 0, -1.0),
    (PhysicalKey::ArrowRight, 1, 1.0),
    (PhysicalKey::KeyD, 1, 1.0),
    (PhysicalKey::ArrowLeft, 1, -1.0),
    (PhysicalKey::KeyA, 1, -1.0),
];

/// Camera turn keys. Left is negative.
const CAMERA_TURN_KEYS: [(PhysicalKey, f32); 2] =
    [(PhysicalKey::KeyQ, -1.0), (PhysicalKey::KeyE, 1.0)];

/// The edited height, one cell per press. Up is the key under the left index
/// finger, down the one under the left pinky, so a hand on the movement keys
/// reaches both without leaving it.
const HEIGHT_KEYS: [(PhysicalKey, i16); 2] = [(PhysicalKey::KeyF, 1), (PhysicalKey::KeyZ, -1)];

impl EditorKeys {
    /// Resolve the held keys into this frame's commands.
    fn poll(&mut self, sched: &Scheduler) {
        let held = |key: PhysicalKey| sched.held_keys.contains(&key);
        let ctrl = held(PhysicalKey::ControlLeft) || held(PhysicalKey::ControlRight);
        let shift = held(PhysicalKey::ShiftLeft) || held(PhysicalKey::ShiftRight);
        // A modified key is not a movement key. Ctrl+D duplicates and Shift+A
        // selects the route; neither should also slide the camera, or holding a
        // modifier would quietly move the view out from under the edit.
        let plain = |key: PhysicalKey| held(key) && !ctrl && !shift;

        self.move_cam = [0.0; 2];
        for (axis, slot) in self.move_cam.iter_mut().enumerate() {
            // Summed rather than picked, so opposite directions cancel instead of
            // one of them silently winning.
            let direction: f32 = CAMERA_KEYS
                .iter()
                .filter(|(key, a, _)| *a == axis && plain(*key))
                .map(|(_, _, direction)| *direction)
                .sum();
            *slot = direction.clamp(-1.0, 1.0);
        }
        self.turn_cam = CAMERA_TURN_KEYS
            .iter()
            .filter(|(key, _)| plain(*key))
            .map(|(_, direction)| *direction)
            .sum::<f32>()
            .clamp(-1.0, 1.0);

        let height: i16 = HEIGHT_KEYS
            .iter()
            .filter(|(key, _)| plain(*key))
            .map(|(_, step)| *step)
            .sum();
        self.step_y(height);

        let now = EditorCommand {
            place: held(PhysicalKey::Enter) || held(PhysicalKey::NumpadEnter),
            delete: held(PhysicalKey::Delete)
                || held(PhysicalKey::Backspace)
                || held(PhysicalKey::KeyX),
            rotate: held(PhysicalKey::KeyR) || held(PhysicalKey::Space),
            // A piece being built rather than one already placed: T turns it,
            // because R and Space rotate the selection and every other key
            // near the hand belongs to the camera.
            turn_armed: held(PhysicalKey::KeyT),
            // V, not G: G already selects the piece under the pointer, and one
            // key doing both would make snapping toggle every time the player
            // picked something out of the track.
            toggle_snap: held(PhysicalKey::KeyV) && !ctrl,
            duplicate: ctrl && held(PhysicalKey::KeyD),
            copy: ctrl && held(PhysicalKey::KeyC),
            paste: ctrl && held(PhysicalKey::KeyV),
            undo: ctrl && held(PhysicalKey::KeyZ),
            redo: ctrl && held(PhysicalKey::KeyY),
            select_all: ctrl && held(PhysicalKey::KeyA),
            select_route: shift && held(PhysicalKey::KeyA),
            select_none: held(PhysicalKey::Escape),
            select_hovered: held(PhysicalKey::KeyG),
            height_up: height > 0,
            height_down: height < 0,
        };
        // The height commands carry the repeat, so a held key raises one cell,
        // waits, then repeats. Every other command here is edge-latched.
        let pressed = EditorCommand {
            place: now.place && !self.down.place,
            delete: now.delete && !self.down.delete,
            rotate: now.rotate && !self.down.rotate,
            turn_armed: now.turn_armed && !self.down.turn_armed,
            toggle_snap: now.toggle_snap && !self.down.toggle_snap,
            duplicate: now.duplicate && !self.down.duplicate,
            copy: now.copy && !self.down.copy,
            paste: now.paste && !self.down.paste,
            undo: now.undo && !self.down.undo,
            redo: now.redo && !self.down.redo,
            select_all: now.select_all && !self.down.select_all,
            select_route: now.select_route && !self.down.select_route,
            select_none: now.select_none && !self.down.select_none,
            select_hovered: now.select_hovered && !self.down.select_hovered,
            height_up: self.step_y > 0,
            height_down: self.step_y < 0,
        };
        self.pressed = pressed;
        self.down = now;
    }

    /// One cell of height this frame, with a delay before it repeats.
    fn step_y(&mut self, direction: i16) {
        self.step_y = 0;
        if direction == 0 {
            self.held_y = 0;
            self.muted_y = false;
            return;
        }
        if self.muted_y {
            // Already down when the editor opened or closed. Not a press, so it
            // neither steps nor starts the delay: releasing the key arms it.
            return;
        }
        self.held_y += 1;
        let n = self.held_y;
        // `n` counts the press frame as 1, so a delay of `DELAY` frames is
        // `DELAY` quiet frames *after* it and the first repeat is the frame
        // after those.
        let repeats =
            n > REPEAT_DELAY_FRAMES && (n - REPEAT_DELAY_FRAMES - 1) % REPEAT_PERIOD_FRAMES == 0;
        if n == 1 || repeats {
            self.step_y = direction;
        }
    }

    /// Reset the repeat counters when the editor closes.
    ///
    /// `down` is deliberately kept. It is the record of which keys were held on
    /// the frame the editor was last polled, and clearing it would make a key
    /// still held at that moment read as a *fresh* press the next time the editor
    /// opens — a command fired by the player letting go of nothing.
    /// The editor was entered or left: forget this frame.
    ///
    /// The height axis is muted as well as reset. Zeroing alone would make a key
    /// that is still physically down look like a fresh press on the next poll, so
    /// the height would step, and then start repeating, under a finger that
    /// never touched the editor. A muted axis does neither until a frame releases
    /// it.
    fn release(&mut self) {
        self.held_y = 0;
        self.muted_y = true;
        self.step_y = 0;
        self.move_cam = [0.0; 2];
        self.turn_cam = 0.0;
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

    /// The stick to draw, or nothing when no finger is holding it.
    ///
    /// Reads the finger rather than a separate visible flag: the flag was set on
    /// touch-down and never cleared, so the stick stayed on screen for the rest of
    /// the session. One source of truth cannot drift that way.
    pub fn stick_view(&self) -> Option<StickView> {
        self.stick.finger.is_some().then_some(StickView {
            anchor: self.stick.anchor,
            knob: self.stick.knob,
        })
    }

    /// Editor keys, read while the editor is up.
    pub fn poll_editor(&mut self, sched: &Scheduler) -> EditorFrame {
        self.editor.poll(sched);
        EditorFrame {
            move_cam: self.editor.move_cam,
            turn_cam: self.editor.turn_cam,
            height: self.editor.step_y,
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
    /// Camera movement across the ground plane, as `(forward, right)` in
    /// -1..=1. Continuous rather than repeated: a held key glides the view.
    pub move_cam: [f32; 2],
    /// Camera turn, -1 left to +1 right, likewise continuous.
    pub turn_cam: f32,
    /// Cells to move the edited height by this frame, after the repeat delay.
    pub height: i16,
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
        crate::report_failure_where_it_happens();
        let right = pad(|b| b.axis(GamepadAxis::LeftStickX, 0.8));
        assert_eq!(pad_controls(&right).steer, 0.8);

        let left = pad(|b| b.axis(GamepadAxis::LeftStickX, -0.8));
        assert_eq!(pad_controls(&left).steer, -0.8);
    }

    #[test]
    fn stick_deadzone_silences_resting_tilt() {
        crate::report_failure_where_it_happens();
        let resting = pad(|b| b.axis(GamepadAxis::LeftStickX, 0.1));
        assert_eq!(pad_controls(&resting).steer, 0.0);
    }

    #[test]
    fn triggers_and_shoulders_drive_the_car() {
        crate::report_failure_where_it_happens();
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
        crate::report_failure_where_it_happens();
        let controls = pad(|b| {
            b.button(repose_core::input::GamepadButton::North, true);
            b.button(repose_core::input::GamepadButton::East, true);
        });
        let c = pad_controls(&controls);
        assert!(c.restart && c.quit);
    }

    #[test]
    fn touch_up_throttles_down_brakes_right_steers() {
        crate::report_failure_where_it_happens();
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
    fn the_knob_recentres_when_the_finger_lifts() {
        crate::report_failure_where_it_happens();
        let mut sched = Scheduler::new();
        let mut stick = TouchStick::default();
        assert_eq!(stick.finger, None);

        sched.touch_points.push((7, 100.0, 200.0));
        stick.update(&sched, true);
        assert_eq!(stick.anchor, (100.0, 200.0));
        assert_eq!(stick.finger, Some(7));

        sched.touch_points = vec![(7, 130.0, 200.0)];
        stick.update(&sched, true);
        assert!(stick.knob.0 > 0.0, "a drag right steers right");

        // Released: the knob centred and the finger is let go.
        sched.touch_points.clear();
        stick.update(&sched, true);
        assert_eq!(stick.finger, None);
        assert_eq!(stick.knob, (0.0, 0.0));
    }

    #[test]
    fn the_ring_slides_with_a_finger_that_runs_past_full_travel() {
        crate::report_failure_where_it_happens();
        // Beyond full travel the ring comes along, so a thumb that reaches the edge
        // of the screen keeps steering instead of jamming against a fixed stop.
        let mut sched = Scheduler::new();
        let mut stick = TouchStick::default();
        let travel = travel_px();

        sched.touch_points.push((2, 100.0, 200.0));
        stick.update(&sched, true);

        // A drag well past the travel: the ring should have moved, and the knob
        // should still be exactly one travel from it.
        sched.touch_points = vec![(2, 100.0 + travel * 3.0, 200.0)];
        stick.update(&sched, true);
        assert!(
            stick.anchor.0 > 100.0 + travel,
            "the ring did not follow the finger: anchor {:?}",
            stick.anchor
        );
        let (dx, dy) = stick.knob;
        assert!(((dx * dx + dy * dy).sqrt() - travel).abs() < 1e-3);
        assert_eq!(dy, 0.0);
        // The knob sits between the ring and the finger, not at the finger.
        assert!(stick.knob.0 < 100.0 + travel * 3.0);
    }

    #[test]
    fn a_drag_within_travel_leaves_the_ring_where_it_was() {
        crate::report_failure_where_it_happens();
        let mut sched = Scheduler::new();
        let mut stick = TouchStick::default();
        let travel = travel_px();

        sched.touch_points.push((5, 100.0, 200.0));
        stick.update(&sched, true);

        sched.touch_points = vec![(5, 100.0 + travel * 0.5, 200.0)];
        stick.update(&sched, true);
        assert_eq!(
            stick.anchor,
            (100.0, 200.0),
            "a short drag must not drag the ring"
        );
        assert!((stick.knob.0 - travel * 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_knob_never_leaves_the_ring_however_far_the_finger_goes() {
        crate::report_failure_where_it_happens();
        // The knob is a position on the ring, not a free vector, so full deflection
        // is the ceiling however far the drag runs — and a drag short of it is
        // proportional rather than clamped.
        let mut sched = Scheduler::new();
        let mut stick = TouchStick::default();
        let travel = travel_px();

        sched.touch_points.push((8, 50.0, 50.0));
        stick.update(&sched, true);

        // Short of full travel, along one axis: the knob tracks the finger exactly.
        sched.touch_points = vec![(8, 50.0 + travel * 0.5, 50.0)];
        stick.update(&sched, true);
        assert!((stick.knob.0 - travel * 0.5).abs() < 1e-3);

        for reach in [travel * 1.5, travel * 3.0, 10_000.0, 1.0e6] {
            sched.touch_points = vec![(8, 50.0 + reach, 50.0 - reach)];
            stick.update(&sched, true);
            let (dx, dy) = stick.knob;
            let len = (dx * dx + dy * dy).sqrt();
            assert!(
                (len - travel).abs() < 1e-3,
                "reach {reach}: knob {len} vs travel {travel}"
            );
        }
    }

    #[test]
    fn the_stick_is_not_drawn_once_the_finger_lifts() {
        crate::report_failure_where_it_happens();
        // The stick used to be marked visible on touch-down and never unmarked, so
        // it stayed on screen for the rest of the session once used.
        let mut sched = Scheduler::new();
        let mut input = InputState::default();

        sched.touch_points.push((3, 120.0, 260.0));
        input.poll(&sched, true);
        assert!(input.stick_view().is_some(), "held: the stick is drawn");

        sched.touch_points.clear();
        input.poll(&sched, true);
        assert!(input.stick_view().is_none(), "released: the stick is gone");
    }

    #[test]
    fn the_stick_is_not_drawn_before_the_first_touch() {
        crate::report_failure_where_it_happens();
        let sched = Scheduler::new();
        let mut input = InputState::default();
        input.poll(&sched, true);
        assert!(input.stick_view().is_none());
    }

    #[test]
    fn a_race_ending_mid_touch_does_not_hand_a_held_stick_to_the_next_one() {
        crate::report_failure_where_it_happens();
        // Leaving the race screen with a finger down used to leave the stick held,
        // so the next race opened already deflected.
        let mut sched = Scheduler::new();
        let mut input = InputState::default();

        sched.touch_points.push((4, 200.0, 300.0));
        input.poll(&sched, true);
        assert!(input.stick_view().is_some());

        // Not racing any more, finger still reported down.
        input.poll(&sched, false);
        assert!(
            input.stick_view().is_none(),
            "a stick held as a race ends must not survive into the next"
        );

        // Racing again with that same finger still down: it re-anchors afresh rather
        // than resuming a deflection from the previous race.
        input.poll(&sched, true);
        let view = input.stick_view().expect("a fresh touch re-anchors");
        assert_eq!(view.knob, (0.0, 0.0));
        assert_eq!(view.anchor, (200.0, 300.0));
    }

    fn sched_with(keys: &[PhysicalKey]) -> Scheduler {
        let mut sched = Scheduler::new();
        sched.held_keys = keys.iter().copied().collect();
        sched
    }

    /// One press of `keys`, read from a fresh input so nothing is latched.
    fn poll_first(keys: &[PhysicalKey]) -> EditorCommand {
        let mut input = InputState::new();
        input.poll_editor(&sched_with(keys)).pressed
    }

    /// Hold `keys` for `frames` polls and collect every height step produced.
    fn steps_while_held(keys: &[PhysicalKey], frames: u32) -> Vec<i16> {
        let sched = sched_with(keys);
        let mut input = InputState::new();
        (0..frames)
            .map(|_| input.poll_editor(&sched).height)
            .collect()
    }

    /// One frame of camera axes with `keys` held.
    fn axes_while_held(keys: &[PhysicalKey]) -> EditorFrame {
        let mut input = InputState::new();
        input.poll_editor(&sched_with(keys))
    }

    #[test]
    fn a_height_key_steps_once_then_repeats_after_a_delay() {
        crate::report_failure_where_it_happens();
        let frames = steps_while_held(&[PhysicalKey::KeyF], 40);

        assert_eq!(frames[0], 1, "the press itself fires");
        // Silent until the delay has elapsed, counting from the frame *after* the
        // press: frame 0 already fired, so the quiet run is 1..DELAY.
        for (n, step) in frames
            .iter()
            .enumerate()
            .take(REPEAT_DELAY_FRAMES as usize)
            .skip(1)
        {
            assert_eq!(*step, 0, "frame {n} must be inside the delay");
        }
        // Then it fires exactly once per period.
        assert_eq!(frames[REPEAT_DELAY_FRAMES as usize], 1, "the first repeat");
        for n in 1..REPEAT_PERIOD_FRAMES {
            assert_eq!(
                frames[(REPEAT_DELAY_FRAMES + n) as usize],
                0,
                "frame {n} of the period must be silent"
            );
        }
        assert_eq!(
            frames[(REPEAT_DELAY_FRAMES + REPEAT_PERIOD_FRAMES) as usize],
            1,
            "and then it repeats"
        );
    }

    #[test]
    fn the_height_repeat_stays_on_its_period_rather_than_drifting() {
        crate::report_failure_where_it_happens();
        for key in [PhysicalKey::KeyF, PhysicalKey::KeyZ] {
            let sign = if key == PhysicalKey::KeyF { 1 } else { -1 };
            let frames = steps_while_held(&[key], 80);
            for (n, step) in frames.iter().enumerate() {
                let expected = n == 0
                    || (n as u32 >= REPEAT_DELAY_FRAMES
                        && (n as u32 - REPEAT_DELAY_FRAMES) % REPEAT_PERIOD_FRAMES == 0);
                assert_eq!(
                    *step == sign,
                    expected,
                    "frame {n}: step {step} but expected a step = {expected}"
                );
            }
        }
    }

    #[test]
    fn releasing_a_height_key_resets_its_repeat() {
        crate::report_failure_where_it_happens();
        let mut input = InputState::new();
        for _ in 0..REPEAT_DELAY_FRAMES + 2 {
            input.poll_editor(&sched_with(&[PhysicalKey::KeyF]));
        }
        assert_eq!(input.poll_editor(&sched_with(&[])).height, 0);
        assert_eq!(
            input.poll_editor(&sched_with(&[PhysicalKey::KeyF])).height,
            1,
            "held again, it must fire at once rather than waiting out the delay"
        );
    }

    #[test]
    fn the_movement_keys_move_the_camera_rather_than_the_cursor() {
        crate::report_failure_where_it_happens();
        // The cursor follows the mouse; the ground-plane keys belong to the view.
        let forward = axes_while_held(&[PhysicalKey::KeyW, PhysicalKey::ArrowUp]);
        assert_eq!(forward.move_cam, [1.0, 0.0]);
        assert_eq!(forward.height, 0, "movement never moves the edited height");

        let back = axes_while_held(&[PhysicalKey::KeyS, PhysicalKey::ArrowDown]);
        assert_eq!(back.move_cam, [-1.0, 0.0]);

        let left = axes_while_held(&[PhysicalKey::KeyA, PhysicalKey::ArrowLeft]);
        assert_eq!(left.move_cam, [0.0, -1.0]);

        let right = axes_while_held(&[PhysicalKey::KeyD, PhysicalKey::ArrowRight]);
        assert_eq!(right.move_cam, [0.0, 1.0]);
    }

    #[test]
    fn the_turn_keys_turn_left_and_right() {
        crate::report_failure_where_it_happens();
        assert_eq!(axes_while_held(&[PhysicalKey::KeyQ]).turn_cam, -1.0);
        assert_eq!(axes_while_held(&[PhysicalKey::KeyE]).turn_cam, 1.0);
        assert_eq!(
            axes_while_held(&[PhysicalKey::KeyQ, PhysicalKey::KeyE]).turn_cam,
            0.0,
            "opposite turns cancel"
        );
        assert_eq!(axes_while_held(&[]).turn_cam, 0.0);
    }

    #[test]
    fn a_held_movement_key_keeps_moving_the_camera_every_frame() {
        crate::report_failure_where_it_happens();
        // Continuous, not repeated: a camera glides, it does not step.
        let sched = sched_with(&[PhysicalKey::KeyW]);
        let mut input = InputState::new();
        for _ in 0..60 {
            assert_eq!(input.poll_editor(&sched).move_cam, [1.0, 0.0]);
        }
    }

    #[test]
    fn opposite_movement_cancel_rather_than_doubling_up() {
        crate::report_failure_where_it_happens();
        assert_eq!(
            axes_while_held(&[PhysicalKey::KeyW, PhysicalKey::KeyS]).move_cam,
            [0.0, 0.0]
        );
        assert_eq!(
            axes_while_held(&[PhysicalKey::KeyA, PhysicalKey::KeyD]).move_cam,
            [0.0, 0.0]
        );
        assert_eq!(
            axes_while_held(&[PhysicalKey::ArrowUp, PhysicalKey::ArrowDown]).move_cam,
            [0.0, 0.0]
        );
    }

    #[test]
    fn a_command_fires_on_the_press_and_not_while_held() {
        crate::report_failure_where_it_happens();
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
    fn a_modified_movement_key_does_not_move_the_camera() {
        crate::report_failure_where_it_happens();
        let mut input = InputState::new();
        let frame = input.poll_editor(&sched_with(&[PhysicalKey::ControlLeft, PhysicalKey::KeyD]));
        assert_eq!(
            frame.move_cam,
            [0.0, 0.0],
            "Ctrl+D duplicates; it is not camera-right"
        );
        assert!(frame.pressed.duplicate);
    }

    #[test]
    fn shift_a_selects_the_route_and_plain_a_moves_the_camera() {
        crate::report_failure_where_it_happens();
        let shifted = axes_while_held(&[PhysicalKey::ShiftLeft, PhysicalKey::KeyA]);
        assert_eq!(
            shifted.move_cam,
            [0.0, 0.0],
            "a modified key is not a movement key"
        );

        let mut input = InputState::new();
        let frame = input.poll_editor(&sched_with(&[PhysicalKey::ShiftLeft, PhysicalKey::KeyA]));
        assert!(frame.pressed.select_route);
    }

    #[test]
    fn undo_and_redo_accept_either_control_side() {
        crate::report_failure_where_it_happens();
        for ctrl in [PhysicalKey::ControlLeft, PhysicalKey::ControlRight] {
            let mut input = InputState::new();
            assert!(
                input
                    .poll_editor(&sched_with(&[ctrl, PhysicalKey::KeyZ]))
                    .pressed
                    .undo
            );
            assert!(
                input
                    .poll_editor(&sched_with(&[ctrl, PhysicalKey::KeyY]))
                    .pressed
                    .redo
            );
        }
    }

    #[test]
    fn either_of_rotate_or_space_turns_the_selection() {
        crate::report_failure_where_it_happens();
        for key in [PhysicalKey::KeyR, PhysicalKey::Space] {
            let mut input = InputState::new();
            assert!(
                input.poll_editor(&sched_with(&[key])).pressed.rotate,
                "{key:?} must rotate the selection"
            );
        }
    }

    /// Turning the armed piece is a different command from turning the
    /// selection, and the two do not collide: holding R would otherwise spin a
    /// piece that has not been placed yet while also turning one that has.
    #[test]
    fn the_armed_piece_is_turned_by_its_own_commands() {
        crate::report_failure_where_it_happens();
        assert!(
            poll_first(&[PhysicalKey::KeyT]).turn_armed,
            "T turns the armed piece"
        );
        assert!(
            poll_first(&[PhysicalKey::KeyR]).rotate,
            "and R still turns the selection"
        );
        assert!(
            !poll_first(&[PhysicalKey::KeyT]).rotate,
            "T must not also rotate the selection"
        );
        assert!(
            !poll_first(&[PhysicalKey::KeyR]).turn_armed,
            "and R must not also turn the armed piece"
        );
    }

    /// Snapping is toggled on V, not G: G already picks the piece under the
    /// pointer, and a key that did both would toggle snapping whenever the
    /// player reached into the track to select something.
    #[test]
    fn snapshotting_is_toggled_by_v() {
        crate::report_failure_where_it_happens();
        assert!(
            poll_first(&[PhysicalKey::KeyV]).toggle_snap,
            "V toggles snapping"
        );
        assert!(
            poll_first(&[PhysicalKey::KeyG]).select_hovered,
            "and G is still the hover pick"
        );
        assert!(
            !poll_first(&[PhysicalKey::KeyV]).turn_armed,
            "and it is not the armed turn"
        );
    }

    /// Edge-latched: held, a turn command fires once, not once per frame.
    #[test]
    fn turn_commands_fire_once_per_press() {
        crate::report_failure_where_it_happens();
        let mut input = InputState::new();
        let sched = sched_with(&[PhysicalKey::KeyT]);
        assert!(input.poll_editor(&sched).pressed.turn_armed);
        assert!(
            !input.poll_editor(&sched).pressed.turn_armed,
            "held, it must not repeat"
        );
    }

    #[test]
    fn either_delete_backspace_or_x_removes_the_selection() {
        crate::report_failure_where_it_happens();
        for key in [
            PhysicalKey::Delete,
            PhysicalKey::Backspace,
            PhysicalKey::KeyX,
        ] {
            let mut input = InputState::new();
            assert!(
                input.poll_editor(&sched_with(&[key])).pressed.delete,
                "{key:?} must delete the selection"
            );
        }
    }

    #[test]
    fn a_key_selects_the_piece_under_the_pointer() {
        crate::report_failure_where_it_happens();
        let mut input = InputState::new();
        assert!(
            input
                .poll_editor(&sched_with(&[PhysicalKey::KeyG]))
                .pressed
                .select_hovered
        );
    }

    #[test]
    fn leaving_the_editor_does_not_fire_a_key_that_was_still_held() {
        crate::report_failure_where_it_happens();
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
    fn leaving_the_editor_mutes_the_height_and_the_commands() {
        crate::report_failure_where_it_happens();
        // The camera keys are continuous, so a key still held after leaving and
        // re-entering keeps gliding, exactly as it should. What must not survive
        // the trip is anything edge-latched: a command fired by the player
        // letting go of nothing, or a height still mid-repeat.
        let mut input = InputState::new();
        let sched = sched_with(&[PhysicalKey::KeyW, PhysicalKey::KeyF]);
        for _ in 0..REPEAT_DELAY_FRAMES + 2 {
            input.poll_editor(&sched);
        }
        input.release_editor();
        let frame = input.poll_editor(&sched);
        assert_eq!(frame.height, 0, "a held key must not raise the height");
        assert!(
            frame.pressed.is_empty(),
            "nor fire any command: {:?}",
            frame.pressed
        );
        // Released and pressed again, it behaves like any other press.
        input.poll_editor(&sched_with(&[]));
        assert_eq!(input.poll_editor(&sched).height, 1);
    }

    #[test]
    fn nothing_is_pressed_when_no_editor_key_is_held() {
        crate::report_failure_where_it_happens();
        let mut input = InputState::new();
        let frame = input.poll_editor(&sched_with(&[]));
        assert_eq!(frame.move_cam, [0.0, 0.0]);
        assert_eq!(frame.turn_cam, 0.0);
        assert_eq!(frame.height, 0);
        assert!(frame.pressed.is_empty());
    }
}

use game_utils_vehicle::VehicleInput;
use repame_input::GamepadState;
use repame_shell::{GamepadPoller, PadBank};
use repose_core::input::PhysicalKey;
use repose_core::runtime::Scheduler;
use repose_core::{Dp, dp_to_px};

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

pub struct InputState {
    /// Signed forward speed of the car, refreshed by the app each frame so
    /// `poll` can tell "stopped" from "still braking".
    pub forward_speed: f32,
    /// Edge-latched: true only on the frame the restart key went down.
    pub restart: bool,
    /// Edge-latched: true only on the frame the quit key went down.
    pub quit: bool,
    vehicle: VehicleInput,
    reverse_latched: bool,
    restart_latched: bool,
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
            vehicle: VehicleInput::neutral(),
            reverse_latched: false,
            restart_latched: false,
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
    }

    pub fn vehicle_input(&self) -> VehicleInput {
        self.vehicle
    }

    pub fn stick_view(&self) -> Option<StickView> {
        self.stick.visible.then_some(StickView {
            anchor: self.stick.anchor,
            knob: self.stick.knob,
        })
    }
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
}

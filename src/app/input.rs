use game_utils_vehicle::VehicleInput;
use repose_core::input::PhysicalKey;
use repose_core::runtime::Scheduler;

/// Forward speed below which held brake falls through to reverse.
const REVERSE_ENGAGE: f32 = 1.0;

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
        }
    }

    pub fn poll(&mut self, sched: &Scheduler) {
        let held = |key: PhysicalKey| sched.held_keys.contains(&key);
        let throttle = held(PhysicalKey::KeyW) || held(PhysicalKey::ArrowUp);
        let brake = held(PhysicalKey::KeyS) || held(PhysicalKey::ArrowDown);
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
        match (left, right) {
            (true, false) => v.steer = -1.0,
            (false, true) => v.steer = 1.0,
            _ => {}
        }
        if held(PhysicalKey::Space) {
            v.handbrake = 1.0;
        }
        if held(PhysicalKey::ShiftLeft) || held(PhysicalKey::ShiftRight) {
            v.boost = 1.0;
        }
        self.vehicle = v.clamped();

        let restart = held(PhysicalKey::KeyR);
        self.restart = restart && !self.restart_latched;
        self.restart_latched = restart;

        let quit = held(PhysicalKey::Escape);
        self.quit = quit && !self.quit_latched;
        self.quit_latched = quit;
    }

    pub fn vehicle_input(&self) -> VehicleInput {
        self.vehicle
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

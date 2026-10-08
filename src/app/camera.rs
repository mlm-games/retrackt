use std::f32::consts::{PI, TAU};

use glam::Vec3;
use repame_view3d::OrbitCamera;

use crate::sim::car::Car;

const FOLLOW_RATE: f32 = 7.0;
const TURN_RATE: f32 = 5.0;
const PITCH: f32 = 0.32;
const BASE_DIST: f32 = 9.5;
const SPEED_STRETCH: f32 = 0.05;
const MAX_STRETCH: f32 = 3.5;

#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub fov_y_deg: f32,
    /// Where the camera stood at the previous frame. The simulation is
    /// fixed-step, so blending this with the current pose is what stops the view
    /// from stepping once per tick while the display runs faster.
    prev: ChasePose,
}

#[derive(Clone, Copy, Debug)]
struct ChasePose {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl ChaseCamera {
    pub fn new() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: PITCH,
            dist: BASE_DIST,
            fov_y_deg: 70.0,
            prev: ChasePose {
                target: Vec3::ZERO,
                yaw: 0.0,
                pitch: PITCH,
                dist: BASE_DIST,
            },
        }
    }

    /// Jump behind the car with no easing, for race start and respawns.
    pub fn snap(&mut self, car: &Car) {
        self.target = car.pos;
        if let Some(yaw) = yaw_behind(car) {
            self.yaw = yaw;
        }
        self.pitch = PITCH;
        self.dist = BASE_DIST;
        // A teleport must not be blended across, so history starts here.
        self.prev = self.pose();
    }

    pub fn update(&mut self, dt: f32, car: &Car) {
        self.prev = self.pose();
        let follow = 1.0 - (-FOLLOW_RATE * dt.max(0.0)).exp();
        self.target += (car.pos - self.target) * follow;
        if let Some(yaw) = yaw_behind(car) {
            self.yaw = lerp_angle(self.yaw, yaw, 1.0 - (-TURN_RATE * dt.max(0.0)).exp());
        }
        self.pitch = PITCH;
        let dist = BASE_DIST + (car.speed() * SPEED_STRETCH).min(MAX_STRETCH);
        self.dist += (dist - self.dist) * follow;
    }

    fn pose(&self) -> ChasePose {
        ChasePose {
            target: self.target,
            yaw: self.yaw,
            pitch: self.pitch,
            dist: self.dist,
        }
    }

    /// `alpha` is `Sim::alpha`: 0 at the tick just taken, 1 as the wall clock
    /// reaches the next one.
    pub fn to_orbit(&self, alpha: f32) -> OrbitCamera {
        let t = alpha.clamp(0.0, 1.0);
        let prev = self.prev;
        let blend = |a: f32, b: f32| a + (b - a) * t;
        OrbitCamera {
            target: prev.target.lerp(self.target, t),
            // Shortest arc, or the view spins the long way round at the wrap.
            yaw: lerp_angle(prev.yaw, self.yaw, t),
            pitch: blend(prev.pitch, self.pitch),
            dist: blend(prev.dist, self.dist),
            fov_y_deg: self.fov_y_deg,
        }
    }
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self::new()
    }
}

/// Yaw that puts the eye far side of the target from the car: `eye` reaches
/// along `(cos yaw, sin yaw)`, so yaw is the negated heading; else surface, else `None`.
fn yaw_behind(car: &Car) -> Option<f32> {
    for dir in [car.forward(), car.heading_dir] {
        let flat = Vec3::new(dir.x, 0.0, dir.z);
        if flat.length_squared() > 1e-4 {
            let flat = flat.normalize();
            return Some((-flat.z).atan2(-flat.x));
        }
    }
    None
}

/// Shortest-arc blend: raw lerp would take the long way around at the wrap.
fn lerp_angle(from: f32, to: f32, t: f32) -> f32 {
    let mut delta = (to - from).rem_euclid(TAU);
    if delta > PI {
        delta -= TAU;
    }
    from + delta * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    fn ndc_x(cam: &OrbitCamera, p: Vec3) -> f32 {
        let clip = cam.view_proj(16.0 / 9.0) * Vec4::new(p.x, p.y, p.z, 1.0);
        clip.x / clip.w
    }

    #[test]
    fn the_chase_camera_puts_the_cars_right_side_on_screen_right() {
        let car = Car::at_spawn(Vec3::ZERO, 0.0);
        let mut cam = ChaseCamera::new();
        cam.snap(&car);
        let cam = cam.to_orbit(1.0);

        let right = car.forward().cross(car.up());
        let on_screen_right = ndc_x(&cam, right * 5.0);
        let on_screen_left = ndc_x(&cam, -right * 5.0);
        assert!(
            on_screen_right > on_screen_left,
            "the car's right side must project to screen-right, got {on_screen_right} vs {on_screen_left}"
        );
    }
}

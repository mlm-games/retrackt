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
    /// The heading the last turn was measured from, so each frame's rotation is
    /// relative. `None` until the first frame after a snap.
    last_heading: Option<Vec3>,
    /// Where the heading has actually taken the camera, un-eased.
    ///
    /// Kept apart from `yaw` because easing toward a target built out of the
    /// already-lagged yaw compounds the lag instead of closing it: a car holding a
    /// long corner would have drifted steadily further behind. This is the true
    /// position, and `yaw` chases it.
    heading_yaw: f32,
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
            last_heading: None,
            heading_yaw: 0.0,
        }
    }

    /// Jump behind the car with no easing, for race start and respawns.
    pub fn snap(&mut self, car: &Car) {
        self.target = car.pos;
        if let Some(yaw) = yaw_behind(car) {
            self.yaw = yaw;
            self.heading_yaw = yaw;
        }
        self.pitch = PITCH;
        self.dist = BASE_DIST;
        // A teleport must not be blended across, so history starts here — and the
        // next frame measures its turn from where the car now points.
        self.last_heading = Some(car.heading_dir);
        self.prev = self.pose();
    }

    pub fn update(&mut self, dt: f32, car: &Car) {
        self.prev = self.pose();
        let follow = 1.0 - (-FOLLOW_RATE * dt.max(0.0)).exp();
        self.target += (car.pos - self.target) * follow;
        if let Some(turn) = self.last_heading.and_then(|from| turned(from, car.heading_dir)) {
            // Accumulated from the turns that actually happened, never read back off
            // the car: a heading has no readable direction while the car is vertical
            // on a loop wall, and the reading either side of that is a half turn out,
            // which left the camera in front of the car after one loop.
            //
            // Negated, because `yaw_behind` reads where the *eye* goes, which moves
            // the other way round from the heading it is read off. Left unwrapped:
            // every consumer takes the shortest arc to it, so wrapping it here would
            // only invent a full turn on the frame that crossed the boundary.
            self.heading_yaw -= turn;
            self.yaw = lerp_angle(
                self.yaw,
                self.heading_yaw,
                1.0 - (-TURN_RATE * dt.max(0.0)).exp(),
            );
        }
        self.last_heading = Some(car.heading_dir);
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

/// Signed turn from heading `from` to heading `to`, about world up.
///
/// Relative rather than absolute on purpose. Reading an azimuth off the car's
/// heading has no answer while the car is vertical on a loop wall, and the answer
/// it picks on the far side is a half turn out — which put the camera in front of
/// the car for the rest of the track after one loop. A rotation that actually
/// happened cannot be ambiguous, because there is no absolute direction in it to
/// be wrong about.
///
/// `None` for a half turn inside one frame: the axis is undefined, and this only
/// arrives if the car moved further than opposite in a single display frame.
fn turned(from: Vec3, to: Vec3) -> Option<f32> {
    let cos = from.dot(to);
    if cos < -0.9999 {
        return None;
    }
    Some(from.cross(to).y.atan2(cos))
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
    use glam::{Mat3, Quat, Vec4};

    fn ndc_x(cam: &OrbitCamera, p: Vec3) -> f32 {
        let clip = cam.view_proj(16.0 / 9.0) * Vec4::new(p.x, p.y, p.z, 1.0);
        clip.x / clip.w
    }

    /// Shortest distance between two angles, since the yaw accumulates and is not
    /// kept in any one branch.
    fn gap(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(TAU);
        d.min(TAU - d)
    }

    /// A car taken once round a vertical loop, as a frame-by-frame rotation of the
    /// body, and the yaw a camera following it ends up at.
    ///
    /// The heading runs from `+Z` at the bottom, up through vertical at the loop's
    /// equator, to `-Z` on the way back down.
    fn drive_a_loop(cam: &mut ChaseCamera) -> Car {
        let mut car = Car::at_spawn(Vec3::ZERO, 0.0);
        cam.snap(&car);
        for step in 1..=600 {
            let t = step as f32 * PI / 300.0;
            let fwd = Vec3::new(0.0, t.sin(), t.cos());
            // Up points at the loop's centre: above the car at the bottom, below it
            // at the apex. This is what makes the loop a loop.
            let up = Vec3::new(0.0, t.cos(), -t.sin());
            let right = up.cross(fwd);
            car.orient = Quat::from_mat3(&Mat3::from_cols(right, up, fwd).into());
            car.heading_dir = fwd;
            car.pos.y = 9.0 * (1.0 - t.cos());
            cam.update(1.0 / 60.0, &car);
        }
        car
    }

    #[test]
    fn a_camera_comes_out_the_far_side_of_a_loop() {
        // The car finishes the loop travelling the other way, so the eye has to end
        // up on the other side of it. It used to end up on the *same* side — in
        // front of the car — because the yaw was read afresh each frame off a
        // heading that has no readable direction while the car is vertical.
        let mut cam = ChaseCamera::new();
        let car = drive_a_loop(&mut cam);

        // `-Z` travel puts the eye at `+Z`, and vice versa: a half turn on.
        let want = yaw_behind(&Car {
            heading_dir: Vec3::NEG_Z,
            ..Car::at_spawn(Vec3::ZERO, 0.0)
        })
        .expect("a horizontal heading has a yaw");
        assert!(
            gap(cam.yaw, want) < 0.05,
            "after a loop the camera must be behind the car again: yaw {:.3}, want \
             {:.3}",
            cam.yaw,
            want
        );

        // And it keeps tracking from there, rather than being stuck on a value the
        // loop happened to leave it at. Held straight rather than turning, so what
        // is measured is whether it settles — not how far behind a slow corner it
        // trails, which is the easing doing its job either way.
        let mut car = car;
        for _ in 0..240 {
            cam.update(1.0 / 60.0, &car);
        }
        assert!(
            gap(cam.yaw, yaw_behind(&car).unwrap()) < 0.02,
            "the camera must settle on the car after a loop, not stay put"
        );
    }

    #[test]
    fn a_camera_never_snaps_across_a_loop() {
        // The turn is eased, so a whip reads as the camera spinning round. What must
        // not happen is a discontinuity: one frame carrying the view most of the way
        // round is a snap no easing can hide.
        let mut cam = ChaseCamera::new();
        let mut car = Car::at_spawn(Vec3::ZERO, 0.0);
        cam.snap(&car);
        let mut last = cam.yaw;
        for step in 1..=600 {
            let t = step as f32 * PI / 300.0;
            let fwd = Vec3::new(0.0, t.sin(), t.cos());
            let up = Vec3::new(0.0, t.cos(), -t.sin());
            car.orient = Quat::from_mat3(&Mat3::from_cols(up.cross(fwd), up, fwd).into());
            car.heading_dir = fwd;
            car.pos.y = 9.0 * (1.0 - t.cos());
            cam.update(1.0 / 60.0, &car);
            let jump = gap(cam.yaw, last);
            assert!(
                jump < 0.35,
                "frame {step} swung the view {jump:.3} rad in one go",
            );
            last = cam.yaw;
        }
    }

    /// The camera as it behaved before the turn became relative: read the heading
    /// afresh every frame and ease towards it.
    ///
    /// Kept as the reference the real camera is held against. On flat road the two
    /// compute the same target every frame, so they must agree; where they do not
    /// is exactly where the old reading was wrong.
    struct AbsoluteYaw {
        yaw: f32,
    }

    impl AbsoluteYaw {
        fn snap(&mut self, car: &Car) {
            self.yaw = yaw_behind(car).expect("a horizontal heading has a yaw");
        }

        fn update(&mut self, dt: f32, car: &Car) {
            let want = yaw_behind(car).expect("a horizontal heading has a yaw");
            self.yaw = lerp_angle(self.yaw, want, 1.0 - (-TURN_RATE * dt.max(0.0)).exp());
        }
    }

    #[test]
    fn a_camera_turning_on_flat_road_is_unchanged() {
        // Every corner has to look the same as it did. Run both readings over the
        // same inputs and hold them together, rather than picking a tolerance that
        // happens to fit: the two only ever differ where the absolute reading had
        // no answer to give.
        let mut car = Car::at_spawn(Vec3::ZERO, 0.0);
        let mut cam = ChaseCamera::new();
        cam.snap(&car);
        let mut old = AbsoluteYaw { yaw: cam.yaw };

        for step in 1..240 {
            // A steady turn, then straight again: both phases, and the change between.
            let yaw = if step < 120 {
                step as f32 * 0.01
            } else {
                1.19
            };
            car.orient = Quat::from_rotation_y(yaw);
            car.heading_dir = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            cam.update(1.0 / 60.0, &car);
            old.update(1.0 / 60.0, &car);
            let drift = gap(cam.yaw, old.yaw);
            assert!(
                drift < 1e-3,
                "flat-road tracking drifted from the old reading at frame {step}: \
                 {drift:.6} rad"
            );
        }

        // And it settles on the car rather than trailing it forever.
        assert!(
            gap(cam.yaw, yaw_behind(&car).unwrap()) < 0.01,
            "a settled camera must sit exactly behind the car"
        );
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

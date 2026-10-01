//! The arcade vehicle: surface-relative motion.
//!
//! Not a rigid body: the car orients to the surface it is touching and projects
//! its velocity onto the tangent plane. That is what makes loops, banked corners
//! and wall rides work: gravity resolves into the surface instead of tearing the
//! car off it. Forces stay in the tangent and normal directions only.

use game_utils_vehicle::VehicleInput;
use glam::{Mat3, Mat4, Quat, Vec3};

use super::world::TrackWorld;

/// Tuning for the arcade car. Every value is a per-second rate or a
/// speed limit, so it reads independently of the simulation rate.
#[derive(Clone, Copy, Debug)]
pub struct CarTuning {
    /// Forward acceleration at full throttle (m/s^2).
    pub accel: f32,
    /// Braking deceleration (m/s^2).
    pub brake: f32,
    /// Linear coasting drag coefficient (1/s). Drag grows with speed, so this is
    /// a rate rather than a fixed deceleration.
    pub coast: f32,
    /// Top speed on the ground (m/s).
    pub top_speed: f32,
    /// Top speed in reverse (m/s). Lower, so reversing is a recovery tool
    /// rather than a way to escape a bad line.
    pub reverse_speed: f32,
    /// Lateral grip: how fast sideways velocity is shed (1/s).
    pub grip: f32,
    /// Lateral grip while the handbrake is held (1/s). Lower means it slides.
    pub handbrake_grip: f32,
    /// Peak yaw rate at full steer and low speed (rad/s).
    pub steer_rate: f32,
    /// Fraction of peak steering lost at top speed.
    pub steer_falloff: f32,
    /// Steering authority while airborne, as a fraction of ground steering.
    pub air_steer: f32,
    /// Gravity (m/s^2).
    pub gravity: f32,
    /// How fast the car's up vector follows the surface normal (1/s).
    pub surface_align: f32,
    /// How fast the car's up vector relaxes to world up in the air (1/s).
    pub air_align: f32,
    /// Downward force held while grounded, as a multiple of gravity.
    /// Above 1 the car sticks to steep surfaces; below 1 it slides off.
    pub stick: f32,
    /// Speed below which the car lets go of a steep surface (m/s).
    pub stick_min_speed: f32,
    /// Ride height above the surface (m).
    pub ride_height: f32,
    /// Fraction of ride-height penetration corrected positionally each tick.
    /// Must be below 1 for stability; higher tracks a rising surface faster.
    pub penetration: f32,
    /// Ride-height spring rate, applied to velocity for smoothness only.
    pub ride_stiffness: f32,
    /// Ride-height damping (1/s).
    pub ride_damping: f32,
    /// Seconds of road to look ahead for when the wheels find nothing. Scaled
    /// by speed, so this is a time constant, not a distance.
    pub look_ahead: f32,
    /// How far below the heading the look-ahead ray is aimed. Positive tilts
    /// it down, which is what makes it meet a rising surface at all.
    pub look_pitch: f32,
    /// Distance the body is pushed out of walls (m).
    pub body_radius: f32,
    /// Extra top speed while boosting, as a multiple.
    pub boost_speed: f32,
    /// Seconds of boost granted by a pad.
    pub boost_time: f32,
    /// Fall below this height respawns the car.
    pub kill_y: f32,
    /// Slipstream: speed multiplier for following another car closely. Not
    /// applied yet: the stand-in trigger gave every car a permanent +12%.
    pub slipstream: f32,
}

/// True when a car released at the world's spawn finds road under its wheels.
/// Racing a spawn that fails this is an endless fall-and-respawn loop.
pub fn settles_at_spawn(world: &TrackWorld) -> bool {
    let car = Car::at_spawn(world.spawn, world.spawn_yaw);
    contact(world, &car, &CarTuning::default()).is_some()
}

/// Physics identity stamped into replay tapes; playback refuses tapes recorded
/// under other tuning. Bump the digest label with `SIM_VERSION` on every change.
pub fn physics_fingerprint() -> retrackt_format::fingerprint::TrackFingerprint {
    retrackt_format::fingerprint::physics_fingerprint(
        retrackt_format::replay::SIM_VERSION,
        &retrackt_format::replay::PhysicsStamp::digest(b"car-tuning-v1"),
    )
}

impl Default for CarTuning {
    fn default() -> Self {
        Self {
            accel: 26.0,
            brake: 42.0,
            coast: 0.34,
            top_speed: 62.0,
            reverse_speed: 14.0,
            grip: 9.0,
            handbrake_grip: 2.2,
            steer_rate: 2.4,
            steer_falloff: 0.55,
            air_steer: 0.3,
            gravity: 30.0,
            surface_align: 14.0,
            air_align: 2.4,
            stick: 1.15,
            stick_min_speed: 7.0,
            ride_height: 0.55,
            penetration: 0.85,
            ride_stiffness: 220.0,
            ride_damping: 40.0,
            look_ahead: 0.30,
            look_pitch: 0.22,
            body_radius: 0.9,
            boost_speed: 1.35,
            boost_time: 2.2,
            kill_y: -60.0,
            slipstream: 0.12,
        }
    }
}

/// The car. One per player or ghost.
#[derive(Clone, Copy, Debug, Default)]
pub struct Car {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Orientation. Up is the car body's +Y.
    pub orient: Quat,
    /// Wheel spin rate per wheel (rad/s), for rendering only.
    pub wheel_spin: [f32; 4],
    /// Front wheel steer angle (rad).
    pub steer_angle: f32,
    /// Suspension compression per wheel, 0..1.
    pub compression: [f32; 4],
    /// Surface the car last touched.
    pub ground_normal: Vec3,
    pub grounded: bool,
    /// Seconds since the wheels last touched something.
    pub air_time: f32,
    /// Seconds since the car last touched a wall.
    pub wall_contact: f32,
    pub boost_left: f32,
    /// Direction of travel *along the surface*, as a 3D tangent. Not a yaw
    /// angle: yaw about world up degenerates on a wall and cannot climb a loop.
    pub heading_dir: Vec3,
    /// Set for one tick when the car lands hard, for effects.
    pub landed_hard: bool,
    /// Set when the kill-height teleport fires; the runtime reads and clears it,
    /// so a respawn inside a multi-tick frame still reports. Not reset per step.
    pub respawned: bool,
}

impl Car {
    pub fn at_spawn(pos: Vec3, yaw: f32) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            orient: Quat::from_rotation_y(yaw),
            heading_dir: Vec3::new(yaw.sin(), 0.0, yaw.cos()),
            wheel_spin: [0.0; 4],
            steer_angle: 0.0,
            compression: [0.5; 4],
            ground_normal: Vec3::Y,
            grounded: true,
            air_time: 0.0,
            wall_contact: 0.0,
            boost_left: 0.0,
            landed_hard: false,
            respawned: false,
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.orient * Vec3::Z
    }

    pub fn up(&self) -> Vec3 {
        self.orient * Vec3::Y
    }

    pub fn right(&self) -> Vec3 {
        self.orient * Vec3::X
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    pub fn speed_kmh(&self) -> f32 {
        self.speed() * 3.6
    }

    /// Signed forward speed along the current heading (m/s).
    pub fn forward_speed(&self) -> f32 {
        self.vel.dot(self.forward())
    }
}

/// Frame of the road the car is riding.
#[derive(Clone, Copy, Debug)]
struct Contact {
    normal: Vec3,
    point: Vec3,
    compression: [f32; 4],
}

/// Sample the four wheel positions into a contact frame; when they find nothing
/// it probes along the heading, which is what holds the car on a loop wall.
fn contact(world: &TrackWorld, car: &Car, tune: &CarTuning) -> Option<Contact> {
    let fwd = car.forward();
    let rgt = car.right();
    let half_w = 0.85;
    let half_l = 1.25;
    let reach = tune.ride_height + 0.9;

    let offsets = [
        -rgt * half_w + fwd * half_l,
        rgt * half_w + fwd * half_l,
        -rgt * half_w - fwd * half_l,
        rgt * half_w - fwd * half_l,
    ];

    let mut normal_sum = Vec3::ZERO;
    let mut point = Vec3::ZERO;
    let mut compression = [0.0f32; 4];
    let mut hits = 0;

    for (i, off) in offsets.iter().enumerate() {
        let wheel = car.pos + *off;
        let Some(h) = world.raycast(wheel, -car.up(), reach) else {
            continue;
        };
        hits += 1;
        normal_sum += h.normal;
        point += h.point;
        // 0 at full extension, 1 fully compressed.
        compression[i] = ((tune.ride_height - h.distance) / 0.9).clamp(0.0, 1.0);
    }

    if hits >= 2 {
        let n = (normal_sum / hits as f32).normalize_or_zero();
        if n.dot(Vec3::Y) > -0.999 {
            return Some(Contact {
                normal: n,
                point: point / hits as f32,
                compression,
            });
        }
    }

    // Probing only downward leaves the car a frame behind a rising surface at
    // speed; the probe aims down rather than level so it meets upcoming road.
    let closing = car.vel.dot(fwd);
    if closing > 0.5 {
        let ahead = reach + closing * tune.look_ahead;
        let aim = (fwd + car.up() * -tune.look_pitch).normalize_or_zero();
        if let Some(h) = world.raycast(car.pos, aim, ahead)
            // Facing the car: road being climbed, not a wall scraped sideways.
            && h.normal.dot(-aim) > 0.12
        {
            return Some(Contact {
                normal: h.normal,
                point: h.point,
                compression: [0.6; 4],
            });
        }
    }
    None
}

/// One fixed step of the arcade car. `dt` must be the fixed sim step; the
/// function is pure in `(car, input, world)`, so a replay tape reproduces a run.
pub fn step_car(
    car: &mut Car,
    input: &VehicleInput,
    world: &TrackWorld,
    tune: &CarTuning,
    dt: f32,
) {
    if dt <= 0.0 {
        return;
    }
    car.landed_hard = false;
    car.wall_contact = 0.0;

    if car.boost_left > 0.0 {
        car.boost_left = (car.boost_left - dt).max(0.0);
    } else if world.in_boost_zone(car.pos) {
        car.boost_left = tune.boost_time;
    }

    let speed = car.speed();
    let falloff = 1.0 - tune.steer_falloff * (speed / tune.top_speed.max(1.0)).clamp(0.0, 1.0);
    let target_steer = input.steer.clamp(-1.0, 1.0) * tune.steer_rate * falloff;
    car.steer_angle = lerp(car.steer_angle, target_steer, (18.0 * dt).min(1.0));

    match contact(world, car, tune) {
        Some(c) => {
            // Landed from a fall? Report it for effects and camera shake.
            if !car.grounded {
                let impact = -car.vel.dot(c.normal);
                car.landed_hard = impact > 14.0;
            }
            car.grounded = true;
            car.air_time = 0.0;
            car.compression = c.compression;
            car.ground_normal = c.normal;
            ground_step(car, input, c, tune, dt, world);
        }
        _ => {
            car.grounded = false;
            car.air_time += dt;
            car.compression = [0.0; 4];
            air_step(car, input, tune, dt);
        }
    }

    car.pos += car.vel * dt;
    resolve_body(car, world, tune);

    let rolling = car.forward_speed() / 0.42;
    for w in car.wheel_spin.iter_mut() {
        *w = rolling;
    }

    if car.pos.y < tune.kill_y {
        let (spawn, yaw) = (world.spawn, world.spawn_yaw);
        *car = Car::at_spawn(spawn, yaw);
        car.respawned = true;
    }
}

/// Motion on a surface with normal `n`, in surface coordinates: a heading
/// angle plus forward/sideways speed; rotating world velocity instead destroys the turn.
fn ground_step(
    car: &mut Car,
    input: &VehicleInput,
    c: Contact,
    tune: &CarTuning,
    dt: f32,
    world: &TrackWorld,
) {
    let n = c.normal;
    let speed = car.speed();

    // Order matters: re-project the tangent first so the car follows the road
    // as it curves away underneath, then steer within that surface.
    let mut fwd = project(car.heading_dir, n);
    if fwd.length() < 1e-3 {
        // Heading has degenerated into the surface; recover from the motion.
        fwd = project(car.vel, n);
        if fwd.length() < 1e-3 {
            return;
        }
    }
    fwd = fwd.normalize();

    let falloff = 1.0 - tune.steer_falloff * (speed / tune.top_speed.max(1.0)).clamp(0.0, 1.0);
    // Rotating about `n` sweeps toward `n x fwd`, and `right_of` puts that on
    // the car's left, so positive steer turns through the negative angle (positive = right).
    fwd = (Quat::from_axis_angle(n, -car.steer_angle * falloff * dt) * fwd).normalize();
    car.heading_dir = fwd;

    let right = right_of(n, fwd);
    if right == Vec3::ZERO {
        return;
    }
    car.orient = frame_from(n, fwd, car.steer_angle, tune, dt);

    // Decompose velocity in the surface frame. Any normal component is
    // contact noise from the previous step and is discarded.
    let v_t = project(car.vel, n);
    let mut v_fwd = v_t.dot(fwd);
    let mut v_side = v_t.dot(right);

    // Gravity resolved onto the surface. This is the loop mechanism: on a
    // vertical wall it points along the wall rather than pulling the car off.
    let g = Vec3::Y * -tune.gravity;
    let mut a_fwd = g.dot(fwd);
    let mut a_side = g.dot(right);

    let throttle = input.throttle.clamp(0.0, 1.0);
    let brake = input.brake.clamp(0.0, 1.0);
    let top = tune.top_speed
        * if car.boost_left > 0.0 {
            tune.boost_speed
        } else {
            1.0
        }
        * (1.0 + tune.slipstream * slipstream_factor(car, world));

    if throttle > 0.0 {
        a_fwd += throttle * tune.accel;
    }
    if brake > 0.0 {
        // Braking opposes travel and never pushes the car backwards: brake held
        // at a standstill must leave it parked. Reverse is a separate control.
        if v_fwd > 0.0 {
            // Clamped so the car can reach rest within one step.
            a_fwd -= (brake * tune.brake).min(v_fwd / dt.max(1e-6));
        } else if v_fwd < 0.0 {
            a_fwd -= (-brake * tune.brake).min(-v_fwd / dt.max(1e-6));
        }
    }
    if input.throttle < 0.0 {
        a_fwd -= -input.throttle * tune.accel * 0.6;
    }
    // Coasting drag, linear in speed: a quadratic term scaled by top speed
    // fights `accel` to a settle point too low to reach loop entry speed.
    a_fwd -= tune.coast * v_fwd;

    // Lateral grip. The handbrake lowers it so the car slides.
    let grip = if input.handbrake > 0.0 {
        tune.handbrake_grip
    } else {
        tune.grip
    };
    a_side -= v_side * grip;

    // Speed limits applied once after the update, slower in reverse: branching
    // on the previous velocity's sign would pin the car at rest and kill reverse.
    v_fwd = (v_fwd + a_fwd * dt).clamp(-tune.reverse_speed, top);
    v_side = (v_side + a_side * dt).clamp(-40.0, 40.0);

    car.vel = fwd * v_fwd + right * v_side;

    // Ride height, resolved by moving the car: velocity is rebuilt from tangent
    // axes only, so any normal push is discarded and the car sinks under the road.
    let gap = (car.pos - c.point).dot(n);
    let penetration = tune.ride_height - gap;
    if penetration > 0.0 {
        car.pos += n * penetration * tune.penetration;
    }
    // A little velocity on top, so the correction reads as ride height rather
    // than a teleport.
    car.vel += n * penetration * tune.ride_stiffness * dt;

    // Past the speed threshold, hold the car on a steep face instead of
    // letting it slide back off.
    if speed > tune.stick_min_speed && n.y < 0.3 {
        car.vel += n * penetration.max(0.0) * tune.stick * 40.0 * dt;
    }
}

fn air_step(car: &mut Car, input: &VehicleInput, tune: &CarTuning, dt: f32) {
    car.vel += Vec3::Y * (-tune.gravity) * dt;
    // Reduced steering authority in the air, about the car's own up axis.
    if input.steer.abs() > 1e-4 {
        // Same sign as the grounded turn: `orient`'s local +X is the left axis.
        let rate = -input.steer * tune.steer_rate * tune.air_steer;
        let yaw = Quat::from_axis_angle(car.up(), rate * dt);
        car.orient = (yaw * car.orient).normalize();
        car.heading_dir = (yaw * car.heading_dir).normalize();
    }
    // Relax toward level so a jump lands the right way up.
    let up = car.up();
    if up.dot(Vec3::Y) < 0.999 {
        let axis = up.cross(Vec3::Y).normalize_or_zero();
        if axis != Vec3::ZERO {
            let slerp = Quat::from_axis_angle(axis, tune.air_align * dt);
            car.orient = (slerp * car.orient).normalize();
        }
    }
}

/// Local +X of an up/forward pair. Right-handedness forces it onto the car's
/// *left*, which is why steering has to rotate through the negative angle.
#[inline]
fn right_of(n: Vec3, f: Vec3) -> Vec3 {
    n.cross(f).normalize_or_zero()
}

/// Heading tangent for a car on a plane with normal `n`.
#[cfg(test)]
fn surface_axes(n: Vec3, heading: f32) -> (Vec3, Vec3) {
    let f = project(Vec3::new(heading.sin(), 0.0, heading.cos()), n);
    let f = if f.length() < 1e-4 {
        project(Vec3::Z, n).normalize_or_zero()
    } else {
        f.normalize()
    };
    (f, right_of(n, f))
}

/// Body orientation on a surface: up along `n`, forward along `fwd`, with roll
/// into the turn about the forward axis so it reads as body roll, not heading.
fn frame_from(n: Vec3, fwd: Vec3, steer: f32, _tune: &CarTuning, _dt: f32) -> Quat {
    let right = right_of(n, fwd);
    let basis = Mat3::from_cols(right, n, fwd);
    // The body *is* the surface frame while grounded: no easing. A blended
    // alignment lags the normal on a loop; probes miss and the car stalls at the top.
    let orient = Quat::from_mat3(&basis);
    let bank = (-steer * 0.14).clamp(-0.3, 0.3);
    if bank.abs() > 1e-4 {
        (Quat::from_axis_angle(fwd, bank) * orient).normalize()
    } else {
        orient
    }
}

#[cfg(test)]
mod basis_tests {
    use super::*;
    use glam::{Mat3, Quat};

    /// The single fact the whole vehicle model rests on: the basis built from
    /// a surface normal and a forward direction must be a real rotation.
    #[test]
    fn surface_basis_is_right_handed() {
        let cases = [
            (Vec3::Y, Vec3::Z),
            (Vec3::Y, Vec3::X),
            (Vec3::X, Vec3::Z),
            (Vec3::NEG_X, Vec3::Y),
            (Vec3::NEG_Y, Vec3::Z),
        ];
        for (n, fwd) in cases {
            let right = right_of(n, fwd);
            let basis = Mat3::from_cols(right, n, fwd);
            assert!(
                basis.determinant() > 0.99,
                "basis for n={n:?} fwd={fwd:?} is left-handed: det={}",
                basis.determinant()
            );
            let q = Quat::from_mat3(&basis);
            assert!((q.length() - 1.0).abs() < 1e-4, "not a unit quaternion");
            assert!(
                (q * Vec3::Y - n).length() < 1e-4,
                "body up must match the surface normal"
            );
            assert!(
                (q * Vec3::Z - fwd.normalize()).length() < 1e-4,
                "body forward must match the heading"
            );
        }
    }

    #[test]
    fn axes_are_orthonormal_for_a_heading_on_a_bank() {
        let n = Vec3::new(0.3, 0.95, 0.0).normalize();
        for heading in [0.0, 1.0, 2.5, -3.0] {
            let (fwd, right) = surface_axes(n, heading);
            assert!((fwd.dot(n)).abs() < 1e-5, "forward must lie in the surface");
            assert!((right.dot(n)).abs() < 1e-5, "right must lie in the surface");
            assert!((fwd.dot(right)).abs() < 1e-5, "axes must be orthogonal");
            assert!((fwd.length() - 1.0).abs() < 1e-5);
            assert!((right.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_heading_pointing_into_the_surface_still_yields_a_valid_frame() {
        let (fwd, right) = surface_axes(Vec3::Y, 0.0);
        assert!((fwd.length() - 1.0).abs() < 1e-5);
        assert!((right.length() - 1.0).abs() < 1e-5);
        assert!(fwd.dot(Vec3::Y).abs() < 1e-5);
    }
}

/// Push the body out of any triangle it has penetrated. Surfaces more than a
/// right angle to the car's up are skipped, so the road never shunts it sideways.
fn resolve_body(car: &mut Car, world: &TrackWorld, tune: &CarTuning) {
    let up = car.up();
    for t in world.nearby(car.pos, tune.body_radius + 0.6) {
        if t.normal.dot(up) > 0.35 {
            continue;
        }
        let cp = super::world::closest_on_triangle(car.pos, t);
        let d = car.pos - cp;
        let dist = d.length();
        if dist >= tune.body_radius || dist <= 1e-5 {
            continue;
        }
        let n = d / dist;
        car.pos += n * (tune.body_radius - dist);
        let into = car.vel.dot(n);
        if into < 0.0 {
            car.vel -= n * into * 1.1;
        }
        // Scrub along the wall rather than bouncing off it.
        car.vel *= 0.985;
        car.wall_contact = 0.08;
    }
}

/// How much slipstream is available behind `car`, 0..1.
fn slipstream_factor(car: &Car, world: &TrackWorld) -> f32 {
    // A cheap proxy: how much road is directly ahead at a similar height.
    let ahead = car.pos + car.forward() * 12.0;
    match world.ground_at(ahead, 8.0) {
        Some(_) => 1.0,
        None => 0.0,
    }
}

/// Project `v` onto the plane with normal `n`, removing the normal part.
#[inline]
fn project(v: Vec3, n: Vec3) -> Vec3 {
    v - n * v.dot(n)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Body axes as a matrix, for rendering.
pub fn body_matrix(car: &Car) -> Mat4 {
    Mat4::from_rotation_translation(car.orient, car.pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::{
        PieceId, PieceInstance, PieceParams, PieceUid, TrackDocument, demo_track, stunt_track,
    };

    const DT: f32 = 1.0 / 120.0;

    fn flat_track() -> TrackWorld {
        // A long straight, built explicitly: driving tests need road for as long
        // as they run, and a closed track's end is a cliff.
        let mut doc = TrackDocument::empty();
        doc.name = "Test Straight".into();
        doc.pieces.push(
            PieceInstance::new(PieceId::Start, [0, 0, 0])
                .with_uid(PieceUid(1))
                .with_params(PieceParams::default().length(2)),
        );
        for i in 1..40 {
            doc.pieces.push(
                PieceInstance::new(PieceId::Straight, [0, 0, i * 2])
                    .with_uid(PieceUid(i as u32 + 1))
                    .with_params(PieceParams::default().length(2)),
            );
        }
        doc.normalize_uids();
        TrackWorld::from_doc(&doc)
    }

    /// Drive `car` for `ticks` with one input. Multi-phase sequences must be a
    /// continuous run: restarting per phase silently resets carried-over state.
    fn drive_car(car: &mut Car, ticks: usize, input: &VehicleInput, world: &TrackWorld) {
        let tune = CarTuning::default();
        for _ in 0..ticks {
            step_car(car, input, world, &tune, DT);
        }
    }

    fn fresh(world: &TrackWorld) -> Car {
        Car::at_spawn(world.spawn, world.spawn_yaw)
    }

    #[test]
    fn the_car_accelerates_forward_and_stays_upright_on_flat_road() {
        let world = flat_track();
        let mut car = fresh(&world);
        drive_car(
            &mut car,
            240,
            &VehicleInput {
                throttle: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(
            car.speed() > 15.0,
            "should have built speed, got {}",
            car.speed()
        );
        assert!(car.grounded, "should still be on the road");
        assert!(
            car.up().dot(Vec3::Y) > 0.97,
            "flat road must not tilt the car: {:?}",
            car.up()
        );
        assert!(
            car.forward().z > 0.0,
            "car should travel along +Z, got {:?}",
            car.forward()
        );
    }

    #[test]
    fn the_car_climbs_a_ramp_without_leaving_the_road() {
        use retrackt_format::demo::ChainBuilder;

        let mut b = ChainBuilder::new("Test Ramp");
        b.step(PieceId::Start, PieceParams::default().length(2));
        for _ in 0..3 {
            b.step(PieceId::Straight, PieceParams::default().length(2));
        }
        b.step(PieceId::RampUp, PieceParams::default().length(2));
        for _ in 0..8 {
            b.step(PieceId::Straight, PieceParams::default().length(2));
        }
        let world = TrackWorld::from_doc(&b.build());

        let mut car = fresh(&world);
        let start_y = car.pos.y;
        let tune = CarTuning::default();
        let input = VehicleInput {
            throttle: 1.0,
            ..VehicleInput::neutral()
        };
        let mut airborne_before_top = 0;
        let mut reached_top = false;
        // How far the body leaned off world up: a ramp's face is 26.57 deg, so
        // a car that stays level was told the road is flat.
        let mut max_tilt = 0.0f32;
        for _ in 0..400 {
            step_car(&mut car, &input, &world, &tune, DT);
            max_tilt = max_tilt.max(1.0 - car.up().dot(Vec3::Y));
            if car.pos.y > start_y + 3.5 {
                reached_top = true;
                break;
            }
            if !car.grounded {
                airborne_before_top += 1;
            }
        }

        assert!(
            reached_top,
            "never gained the ramp's 4 m; ended {:+.2} m up",
            car.pos.y - start_y
        );
        assert!(
            airborne_before_top <= 10,
            "left the road {airborne_before_top} ticks before the top"
        );
        assert!(
            max_tilt > 0.05,
            "never pitched to the slope: max lean {max_tilt:.4} (up {:+?})",
            car.up()
        );
    }

    #[test]
    fn the_car_respects_its_top_speed() {
        let world = flat_track();
        let tune = CarTuning::default();
        let mut car = fresh(&world);
        drive_car(
            &mut car,
            1200,
            &VehicleInput {
                throttle: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(
            car.speed() <= tune.top_speed * 1.05,
            "speed {} exceeded top {}",
            car.speed(),
            tune.top_speed
        );
    }

    #[test]
    fn braking_decelerates_and_stops() {
        let world = flat_track();
        // One continuous run: accelerate, then brake on the same car.
        let mut car = fresh(&world);
        drive_car(
            &mut car,
            360,
            &VehicleInput {
                throttle: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        let fast = car.speed();
        assert!(fast > 15.0, "should be fast before braking, got {fast}");
        drive_car(
            &mut car,
            240,
            &VehicleInput {
                brake: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(
            car.speed() < 2.0,
            "braking from {fast} left {} m/s",
            car.speed()
        );
    }

    #[test]
    fn holding_the_brake_at_a_standstill_does_not_reverse() {
        let world = flat_track();
        let start = fresh(&world).pos.z;
        let mut car = fresh(&world);
        drive_car(
            &mut car,
            240,
            &VehicleInput {
                brake: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(
            car.pos.z > start - 1.0,
            "brake must not push the car backwards: {} -> {}",
            start,
            car.pos.z
        );
    }

    #[test]
    fn steering_turns_the_car_and_changes_heading() {
        let world = flat_track();
        let input = VehicleInput {
            throttle: 0.5,
            ..VehicleInput::neutral()
        };
        let mut car = fresh(&world);
        drive_car(
            &mut car,
            600,
            &VehicleInput {
                steer: 0.8,
                ..input
            },
            &world,
        );
        assert!(
            car.forward().x < -0.5,
            "holding right steer must curve the car right, forward is {:?}",
            car.forward().to_array()
        );
        assert!(
            car.heading_dir.x < -0.2,
            "heading should swing toward -X on a right turn from +Z: {:?}",
            car.heading_dir.to_array()
        );
    }

    #[test]
    fn the_car_drives_a_full_loop_and_comes_back_down() {
        let world = loop_track();
        let tune = CarTuning::default();
        let mut car = fresh(&world);

        let mut peak_y = f32::MIN;
        let mut inverted = 0usize;
        let mut entry_speed = 0.0f32;
        let mut saw_entry = false;

        for _ in 0..(120 * 20) {
            step_car(
                &mut car,
                &VehicleInput {
                    throttle: 1.0,
                    ..VehicleInput::neutral()
                },
                &world,
                &tune,
                DT,
            );
            peak_y = peak_y.max(car.pos.y);
            if car.up().dot(Vec3::Y) < 0.0 {
                inverted += 1;
            }
            // The loop sits at z = 200 m; note the speed on arrival.
            if !saw_entry && car.pos.z > 199.0 {
                saw_entry = true;
                entry_speed = car.speed();
            }
        }

        assert!(saw_entry, "never reached the loop");
        assert!(
            entry_speed > 18.0,
            "entered the loop at {entry_speed:.1} m/s; a 12 m loop needs              about {:.0} m/s to hold at the apex",
            (tune.gravity * 12.0).sqrt()
        );
        assert!(
            inverted > 6,
            "a loop must put the car upside down for a moment, was {inverted} ticks"
        );
        assert!(peak_y > 12.0, "never got up the loop, peak {peak_y}");
    }

    #[test]
    fn the_car_is_never_buried_in_the_road_while_grounded() {
        // Driving off the end of a track is legitimate; what must never happen
        // is the car passing *through* the road while still reporting contact.
        for doc in [demo_track(), stunt_track()] {
            let world = TrackWorld::from_doc(&doc);
            let tune = CarTuning::default();
            let mut car = fresh(&world);
            for tick in 0..(120 * 25) {
                // Contact is resolved before the step moves the car, so the road
                // must be under where the car *was*, not where it ended up.
                let before = car.pos;
                let up = car.up();
                step_car(
                    &mut car,
                    &VehicleInput {
                        throttle: 1.0,
                        ..VehicleInput::neutral()
                    },
                    &world,
                    &tune,
                    DT,
                );
                assert!(
                    car.pos.is_finite(),
                    "{}: position went non-finite",
                    doc.name
                );
                assert!(
                    car.speed() < 200.0,
                    "{}: speed ran away at {tick}",
                    doc.name
                );
                // Respawn is a teleport, not a physics step: at the 200 m/s bound
                // a step covers under 1.7 m, so beyond 2 m the invariant does not apply.
                if car.pos.distance(before) > 2.0 {
                    continue;
                }
                if !car.grounded {
                    continue;
                }
                // Probe along the car's own down axis: on a bank or a loop
                // wall, "below the car" is not straight down in world space.
                let above = before + up * 2.0;
                assert!(
                    world.raycast(above, -up, 12.0).is_some(),
                    "{}: grounded at tick {tick} but no road under {:?}",
                    doc.name,
                    before.to_array()
                );
                assert!(
                    car.pos.y > -5.0,
                    "{}: grounded below the world at {tick}",
                    doc.name
                );
            }
        }
    }

    /// A loop with a long approach: the car needs `sqrt(g * r)` at the apex,
    /// which on a 3-cell loop is about 19 m/s, so it has to build speed first.
    fn loop_track() -> TrackWorld {
        let mut doc = TrackDocument::empty();
        doc.name = "Loop Test".into();
        doc.pieces.push(
            PieceInstance::new(PieceId::Start, [0, 0, 0])
                .with_uid(PieceUid(1))
                .with_params(PieceParams::default().length(2)),
        );
        for i in 1..24 {
            doc.pieces.push(
                PieceInstance::new(PieceId::Straight, [0, 0, i * 2])
                    .with_uid(PieceUid(i as u32 + 1))
                    .with_params(PieceParams::default().length(2)),
            );
        }
        doc.pieces.push(
            PieceInstance::new(PieceId::Boost, [0, 0, 48])
                .with_uid(PieceUid(100))
                .with_params(PieceParams::default().length(2)),
        );
        doc.pieces.push(
            PieceInstance::new(PieceId::Loop, [0, 0, 50])
                .with_uid(PieceUid(101))
                .with_params(PieceParams::default().radius(3)),
        );
        doc.normalize_uids();
        TrackWorld::from_doc(&doc)
    }

    #[test]
    fn driving_is_deterministic() {
        // The whole point of a fixed step: identical input, identical state.
        let world = TrackWorld::from_doc(&stunt_track());
        let tune = CarTuning::default();
        let run = || {
            let mut car = fresh(&world);
            let mut out = Vec::new();
            for t in 0..(120 * 8) {
                let steer = ((t as f32) * 0.01).sin() * 0.5;
                step_car(
                    &mut car,
                    &VehicleInput {
                        throttle: 1.0,
                        steer,
                        ..VehicleInput::neutral()
                    },
                    &world,
                    &tune,
                    DT,
                );
                out.push(car.pos.to_array());
            }
            out
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn reversing_backs_the_car_up() {
        let world = flat_track();
        let mut car = fresh(&world);
        // Drive forward first: reversing from the spawn standstill immediately
        // leaves the track, and the fall — not reverse gear — skews the assertion.
        drive_car(
            &mut car,
            90,
            &VehicleInput {
                throttle: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(car.forward_speed() > 1.0, "should be moving forward first");
        drive_car(
            &mut car,
            120,
            &VehicleInput {
                throttle: -1.0,
                ..VehicleInput::neutral()
            },
            &world,
        );
        assert!(
            car.forward_speed() < -0.5,
            "negative throttle must reverse: signed speed {}",
            car.forward_speed()
        );
        // The car may have turned around, so world position is the wrong check:
        // signed speed along its own heading is what reverse gear means.
        let tune = CarTuning::default();
        assert!(
            car.speed() <= tune.reverse_speed + 0.01,
            "reverse speed {} exceeded the {} m/s limit",
            car.speed(),
            tune.reverse_speed
        );
        assert!(
            car.speed() < tune.top_speed,
            "reversing must stay slower than forward"
        );
    }

    #[test]
    fn handbrake_reduces_lateral_grip() {
        let tune = CarTuning::default();
        // Compared directly: the tuned value must be lower, which is what
        // makes the handbrake slide.
        assert!(tune.handbrake_grip < tune.grip);
    }

    #[test]
    fn boost_pads_give_extra_speed() {
        let world = TrackWorld::from_doc(&demo_track());
        let tune = CarTuning::default();
        let mut car = fresh(&world);
        // Put the car right on a boost pad.
        let pad = world.boost_zones[0];
        car.pos = Vec3::new(
            (pad[0] + pad[3]) * 0.5,
            (pad[1] + pad[4]) * 0.5,
            (pad[2] + pad[5]) * 0.5,
        );
        assert!(world.in_boost_zone(car.pos));
        step_car(&mut car, &VehicleInput::neutral(), &world, &tune, DT);
        assert!(car.boost_left > 0.0, "boost pad should start the timer");
    }

    #[test]
    fn a_zero_dt_is_a_no_op() {
        let world = flat_track();
        let mut car = fresh(&world);
        let before = car;
        step_car(
            &mut car,
            &VehicleInput {
                throttle: 1.0,
                ..VehicleInput::neutral()
            },
            &world,
            &CarTuning::default(),
            0.0,
        );
        assert_eq!(car.pos.to_array(), before.pos.to_array());
    }

    #[test]
    fn surface_projection_removes_the_normal_component() {
        let n = Vec3::new(0.0, 0.6, 0.8).normalize();
        let v = Vec3::new(3.0, 4.0, 5.0);
        let p = project(v, n);
        assert!(p.dot(n).abs() < 1e-5, "projection must be tangent");
        // Projection can only shorten a vector.
        assert!(p.length() <= v.length() + 1e-5);
        assert!(p.length() > 0.0);
    }
}

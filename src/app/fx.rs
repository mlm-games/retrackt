//! Skid marks and dust.
//!
//! Both are `repame-fx` entities living in the simulation's own world rather
//! than in the renderer. That is what makes them belong to the run: they age on
//! the tick they were laid on, a replay lays the marks its run laid, and a held
//! race stops its dust instead of leaving it drifting under the pause panel.
//!
//! Nothing here draws. The scene asks for the live entities once a frame and
//! turns them into groups, which is the only part that needs a camera.

use glam::Vec3;
use repame_fx::decals::{Decal, DecalDef, decal_groups, spawn_decal, step_decals_secs};
use repame_fx::effect::{EaseKind, Gradient};
use repame_fx::particles3d::{Particle3, particle_groups, step_particles3_secs};
use repame_sim::SimTime;
use repame_sim::bevy_ecs::{self, prelude::*};
use repame_view3d::MeshGroup;

use crate::app::schedule::{CarRes, TrackRes};
use crate::sim::car::Car;

/// Sideways speed at which the tyres start leaving a mark, in m/s. Below it the
/// car is cornering, and a corner taken cleanly is not something that marks the
/// road.
const SLIDE_MPS: f32 = 4.0;

/// How long a mark lasts, in seconds. Long enough to read the line taken
/// through a corner after the corner has been left behind.
const MARK_LIFE_SECS: f32 = 15.0;

/// Ticks between two marks. A slide lays a dotted line rather than a solid
/// band, and a frame that catches up several ticks cannot stack a pile of
/// decals on one spot.
const MARK_EVERY_TICKS: u32 = 3;

/// Half the track width, in metres: where the rear tyres are. Matches the
/// scene's own wheel placement.
const REAR_TRACK: f32 = 0.8;
const REAR_AXLE: f32 = 1.25;

/// Dust thrown by one hard landing.
const LANDING_PUFFS: usize = 10;

#[derive(Resource)]
pub struct FxRes {
    /// Whether marks are laid at all, from the options.
    pub marks: bool,
    /// Whether dust is thrown at all, from the options.
    pub dust: bool,
    /// Ticks since the last mark was laid.
    since_mark: u32,
    /// Whether the car was off the ground last tick.
    airborne: bool,
    /// Free-running tick count, used to seed the puff directions. The same run
    /// thrown at the same tick lands the same way, so a replay of it matches.
    seed: u32,
}

impl Default for FxRes {
    fn default() -> Self {
        Self {
            marks: true,
            dust: true,
            since_mark: 0,
            airborne: false,
            seed: 0x2545_F491,
        }
    }
}

impl FxRes {
    fn next(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

/// The car's speed across its own nose, in m/s. Zero when it is going straight,
/// whatever that happens to be over the ground: a car pointing where it is
/// going is not sliding, and one crossing a banked corner sideways is.
fn lateral_speed(car: &Car) -> f32 {
    let fwd = car.heading_dir;
    (car.vel - fwd * car.vel.dot(fwd)).length()
}

/// Lay a mark under each rear tyre while the car slides, and throw dust when it
/// lands hard.
fn emit_fx(mut commands: Commands, car: Res<CarRes>, world: Res<TrackRes>, mut fx: ResMut<FxRes>) {
    let car = &car.0;

    if fx.dust && car.landed_hard {
        puff(&mut commands, &mut fx, car.pos, &world.0);
    }

    let sliding = car.grounded && lateral_speed(car) > SLIDE_MPS;
    if !sliding {
        fx.since_mark = 0;
        fx.airborne = !car.grounded;
        return;
    }
    if fx.since_mark > 0 {
        fx.since_mark -= 1;
        fx.airborne = !car.grounded;
        return;
    }
    fx.since_mark = MARK_EVERY_TICKS;
    fx.airborne = !car.grounded;

    if !fx.marks {
        return;
    }
    let fwd = car.heading_dir;
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let yaw = fwd.x.atan2(fwd.z);
    for side in [-1.0f32, 1.0] {
        let at = car.pos - fwd * REAR_AXLE + right * (side * REAR_TRACK);
        spawn_decal(
            &mut commands,
            DecalDef {
                pos: at.to_array(),
                size_world: 0.55,
                tint: [0.04, 0.04, 0.05],
                life_secs: MARK_LIFE_SECS,
                // The last third of the life is the fade, so a mark does not
                // simply stop existing at the far end of a long corner.
                fade_secs: MARK_LIFE_SECS / 3.0,
                page: 0,
                uv_min: [0.0, 0.0],
                uv_max: [1.0, 1.0],
                yaw,
                // Clear of the road it is lying on, or it z-fights with it.
                lift: 0.02,
                grow: 0.0,
                life_ticks: 1,
                fade_ticks: 0,
            },
        );
    }
}

/// A ring of dust at one point, thrown outward and upward.
fn puff(commands: &mut Commands, fx: &mut FxRes, pos: Vec3, world: &crate::sim::world::TrackWorld) {
    // Grounded on the road under the car rather than at the car itself: a
    // landing is met at the wheels, and the car's own origin sits above them.
    let ground = world.ground_at(pos, 4.0).map(|hit| hit.point);
    let at = ground.unwrap_or(pos);
    for i in 0..LANDING_PUFFS {
        let r = fx.next();
        let theta = (r % 1024) as f32 / 1024.0 * std::f32::consts::TAU;
        let speed = 1.4 + (r >> 10 & 0xff) as f32 / 255.0 * 2.2;
        let lift = 1.0 + (r >> 18 & 0xff) as f32 / 255.0 * 1.6;
        commands.spawn((Particle3 {
            pos: at.to_array(),
            vel: [theta.cos() * speed, lift, theta.sin() * speed],
            age_secs: 0.0,
            life_secs: 0.55 + (i as f32 / LANDING_PUFFS as f32) * 0.25,
            size_world: 0.5,
            // Falls a little as it spreads, which is what makes it read as
            // something that was thrown rather than something that grew.
            gravity_pps2: -2.2,
            drag_per_sec: 2.4,
            gradient: Gradient::fade_out([0.62, 0.58, 0.5, 0.5]),
            ease: EaseKind::QuadOut,
            spawner: None,
            page: 0,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            age_ticks: 0,
            life_ticks: 60,
        },));
    }
}

fn step_marks(mut commands: Commands, mut decals: Query<(Entity, &mut Decal)>, time: Res<SimTime>) {
    step_decals_secs(&mut commands, &mut decals, time.delta_secs);
}

fn step_dust(
    mut commands: Commands,
    mut particles: Query<(Entity, &mut Particle3)>,
    time: Res<SimTime>,
) {
    step_particles3_secs(&mut commands, &mut particles, time.delta_secs);
}

/// The live marks and dust as groups the scene can draw, over nothing when the
/// options have turned them off.
pub fn groups(world: &mut World, eye: Vec3) -> Vec<MeshGroup> {
    let fx = world.resource::<FxRes>();
    let (want_marks, want_dust) = (fx.marks, fx.dust);

    let mut out = Vec::new();
    if want_marks {
        out.extend(decal_groups(world.query::<&Decal>().iter(world)));
    }
    if want_dust {
        out.extend(particle_groups(
            world.query::<&Particle3>().iter(world),
            eye,
        ));
    }
    out
}

/// How many marks and dust motes are alive. For tests: the effects are the only
/// thing in the world whose whole lifecycle is "appears, ages, goes", and the
/// count is the whole of what can be observed about them.
pub fn live(world: &mut World) -> (usize, usize) {
    let decals = world.query::<&Decal>().iter(world).count();
    let dust = world.query::<&Particle3>().iter(world).count();
    (decals, dust)
}

pub fn register(sim: &mut repame_sim::Sim) {
    sim.add_chained_systems((emit_fx, step_marks, step_dust));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::schedule;
    use crate::sim::world::TrackWorld;
    use retrackt_format::demo_track;

    /// A simulation with the effects systems in it and a car standing on the
    /// road at the start line.
    ///
    /// The car is left un-driven — nothing sets `ActiveRes` — so the velocity a
    /// test gives it survives: the whole subject here is what a car crossing
    /// the road sideways does, and driving it would be testing the tyres.
    fn rig() -> repame_sim::Sim {
        let mut sim = repame_sim::Sim::new(crate::SIM_STEP);
        schedule::register(&mut sim);
        schedule::insert_resources(&mut sim.world);
        sim.world.insert_resource(FxRes::default());
        register(&mut sim);

        let world = TrackWorld::from_doc(&demo_track());
        let mut car = Car::at_spawn(world.spawn, world.spawn_yaw, 0);
        car.pos = world.checkpoints[0].centre;
        sim.world.insert_resource(schedule::TrackRes(world));
        sim.world.insert_resource(schedule::CarRes(car));
        sim
    }

    fn slide(sim: &mut repame_sim::Sim, mps: f32) {
        let mut car = sim.world.resource_mut::<schedule::CarRes>();
        car.0.vel = car.0.heading_dir.cross(Vec3::Y) * mps;
    }

    fn run(sim: &mut repame_sim::Sim, ticks: u32) {
        for _ in 0..ticks {
            sim.step(crate::SIM_STEP);
        }
    }

    #[test]
    fn a_sliding_car_lays_marks_and_a_standing_one_does_not() {
        crate::report_failure_where_it_happens();
        let mut sim = rig();
        run(&mut sim, 12);
        assert_eq!(live(&mut sim.world).0, 0, "a car at rest leaves nothing");

        slide(&mut sim, 12.0);
        run(&mut sim, 12);
        let (marks, _) = live(&mut sim.world);
        assert!(
            marks > 0,
            "a car crossing the road leaves marks, got {marks}"
        );
    }

    #[test]
    fn a_car_going_straight_leaves_nothing() {
        crate::report_failure_where_it_happens();
        let mut sim = rig();
        {
            let mut car = sim.world.resource_mut::<schedule::CarRes>();
            car.0.vel = car.0.heading_dir * 30.0;
        }
        run(&mut sim, 24);
        assert_eq!(
            live(&mut sim.world).0,
            0,
            "speed along the nose is not a slide"
        );
    }

    #[test]
    fn the_marks_option_stops_them_at_the_source() {
        crate::report_failure_where_it_happens();
        let mut sim = rig();
        sim.world.resource_mut::<FxRes>().marks = false;
        slide(&mut sim, 12.0);
        run(&mut sim, 24);
        assert_eq!(live(&mut sim.world).0, 0, "off means no entities at all");
    }

    #[test]
    fn a_mark_goes_away_when_its_life_is_over() {
        crate::report_failure_where_it_happens();
        let mut sim = rig();
        slide(&mut sim, 12.0);
        run(&mut sim, 12);
        assert!(live(&mut sim.world).0 > 0, "something was laid");

        // Longer than the mark's own life, so what is left is what the slide
        // has gone on to lay rather than what it laid at the start.
        let ticks = (MARK_LIFE_SECS / crate::SIM_STEP.as_secs_f32()).ceil() as u32 + 8;
        sim.world.resource_mut::<schedule::CarRes>().0.vel = Vec3::ZERO;
        run(&mut sim, ticks);
        assert_eq!(live(&mut sim.world).0, 0, "marks do not outlive their life");
    }
}

use game_utils_vehicle::VehicleInput;
use repame_sim::SimTime;
use retrackt_format::{PackedInput, ReplayTape};
// The `bevy_ecs` binding is what `#[derive(Resource)]` resolves against:
// retrackt does not depend on bevy_ecs directly.
use repame_sim::bevy_ecs::{self, prelude::*};

use crate::session::RaceSession;
use crate::sim::car::{Car, CarTuning, step_car};
use crate::sim::world::TrackWorld;

#[derive(Resource)]
pub struct TrackRes(pub TrackWorld);

#[derive(Resource)]
pub struct CarRes(pub Car);

/// The canonical per-tick driver intent, quantised once at the edge of the
/// simulation. Both the car and the tape read this same value, so a run always
/// replays from exactly the input the live car consumed.
///
/// The intent is queued rather than held singly because one frame may run
/// several ticks after a hitch, and each of those ticks is a step the tape
/// records. `take` hands out one entry per tick, so a tick can never read
/// another tick's input.
#[derive(Resource)]
pub struct InputRes {
    /// Applied to the tick being simulated.
    current: PackedInput,
    /// Refilled by the runtime each frame, consumed one entry per tick.
    pending: std::collections::VecDeque<PackedInput>,
}

impl InputRes {
    pub fn new() -> Self {
        Self {
            current: PackedInput::default(),
            pending: Default::default(),
        }
    }

    /// Replace the queue with `n` copies of `packed`, enough for the most ticks
    /// one `step` can run. Only one input sample exists per frame, so every
    /// catch-up tick necessarily applies the intent held at the frame; the queue
    /// makes that explicit and gives each tick its own entry, rather than one
    /// value quietly applying to all of them.
    pub fn refill(&mut self, packed: PackedInput, n: usize) {
        self.current = packed;
        self.pending.clear();
        self.pending.extend(std::iter::repeat_n(packed, n));
    }

    /// The intent for this tick, advancing to the next queued entry.
    pub fn take(&mut self) -> PackedInput {
        if let Some(next) = self.pending.pop_front() {
            self.current = next;
        }
        self.current
    }

    /// What `take` most recently handed out, for the tape.
    pub fn current(&self) -> PackedInput {
        self.current
    }
}

#[derive(Resource)]
pub struct SessionRes(pub RaceSession);

#[derive(Resource)]
pub struct ActiveRes(pub bool);

#[derive(Resource)]
pub struct FinishedRes(pub bool);

/// The replay tape of the run in progress, closed once the finish tick is
/// recorded. Taken by the runtime when it builds the race result.
#[derive(Resource)]
pub struct TapeRes(pub Option<ReplayTape>);

pub fn insert_resources(world: &mut World) {
    world.insert_resource(TrackRes(TrackWorld::default()));
    world.insert_resource(CarRes(Car::default()));
    world.insert_resource(InputRes::new());
    world.insert_resource(SessionRes(RaceSession::new()));
    world.insert_resource(ActiveRes(false));
    world.insert_resource(FinishedRes(false));
    world.insert_resource(TapeRes(None));
}

/// Decode a packed tick back into the vehicle's own input type, clamped to the
/// same ranges the live poll produces.
fn decode(packed: PackedInput) -> VehicleInput {
    VehicleInput {
        steer: packed.steer_f32(),
        throttle: packed.throttle_f32(),
        brake: packed.brake_f32(),
        clutch: 0.0,
        handbrake: f32::from(u8::from(packed.handbrake())),
        boost: f32::from(u8::from(packed.boost())),
    }
    .clamped()
}

fn step_vehicle(
    time: Res<SimTime>,
    mut input: ResMut<InputRes>,
    track: Res<TrackRes>,
    active: Res<ActiveRes>,
    mut car: ResMut<CarRes>,
) {
    if !active.0 {
        return;
    }
    step_car(
        &mut car.0,
        &decode(input.take()),
        &track.0,
        &CarTuning::default(),
        time.delta_secs,
    );
}

fn advance_session(active: Res<ActiveRes>, car: Res<CarRes>, mut session: ResMut<SessionRes>) {
    if !active.0 {
        return;
    }
    session.0.tick(car.0.pos);
}

fn signal_finish(
    active: Res<ActiveRes>,
    session: Res<SessionRes>,
    mut finished: ResMut<FinishedRes>,
) {
    finished.0 = active.0 && session.0.finished();
}

/// One tape tick per simulated tick, holding exactly what `step_vehicle`
/// consumed this tick — `InputRes::current` is the entry `take` handed out, and
/// `take` runs first in the chain. The tick that crossed the finish gate closes
/// the tape.
fn record_replay(
    active: Res<ActiveRes>,
    input: Res<InputRes>,
    finished: Res<FinishedRes>,
    mut tape: ResMut<TapeRes>,
) {
    if !active.0 {
        return;
    }
    let Some(tape) = tape.0.as_mut() else {
        return;
    };
    if tape.header.finish_tick != 0 {
        return;
    }
    tape.push(input.current());
    if finished.0 {
        tape.header.finish_tick = tape.tick_len();
    }
}

/// One chain, ordered: car, session, finish flag, tape. Separate `add_*` calls
/// get no ordering, so order-sensitive systems live in this tuple.
pub fn register(sim: &mut repame_sim::Sim) {
    sim.add_chained_systems((step_vehicle, advance_session, signal_finish, record_replay).chain());
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use retrackt_format::{ReplayTape, demo_track, gameplay_fingerprint};

    #[test]
    fn each_tick_takes_its_own_entry_and_the_tape_records_that_entry() {
        let mut input = InputRes::new();
        let idle = PackedInput::new(0.0, 0.0, 0.0, false, false, false);
        let boost = PackedInput::new(0.0, 1.0, 0.0, false, true, false);

        input.refill(idle, 3);
        assert_eq!(input.take(), idle);
        assert_eq!(input.take(), idle);
        assert_eq!(input.take(), idle);
        assert_eq!(input.take(), idle, "an empty queue repeats, never blocks");

        input.refill(idle, 1);
        input.pending.push_back(boost);
        assert_eq!(input.take(), idle, "the next tick gets the queued order");
        assert_eq!(input.take(), boost);
        assert_eq!(
            input.current(),
            boost,
            "the tape reads what take handed out this tick"
        );
    }

    #[test]
    fn chain_records_one_tape_tick_per_sim_tick_and_closes_at_the_finish() {
        let mut sim = repame_sim::Sim::new(crate::SIM_STEP);
        register(&mut sim);
        insert_resources(&mut sim.world);

        let doc = demo_track();
        let world = TrackWorld::from_doc(&doc);
        sim.world.insert_resource(TrackRes(world));
        {
            let w = sim.world.resource::<TrackRes>().0.clone();
            let mut car = Car::at_spawn(w.spawn, w.spawn_yaw);
            car.pos = w.checkpoints[0].centre;
            sim.world.insert_resource(CarRes(car));
            sim.world.resource_mut::<SessionRes>().0.start(&w);
        }
        sim.world.insert_resource(TapeRes(Some(ReplayTape::new(
            gameplay_fingerprint(&doc),
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        ))));
        sim.world.insert_resource(InputRes::new());
        sim.world.insert_resource(ActiveRes(true));

        let mut ticks = 0u32;
        for _ in 0..2000 {
            let next = {
                let s = sim.world.resource::<SessionRes>();
                let w = sim.world.resource::<TrackRes>();
                let n = s.0.checkpoint();
                if n < w.0.checkpoints.len() {
                    w.0.checkpoints[n].centre
                } else {
                    w.0.finish.expect("demo has a finish").centre
                }
            };
            sim.world.resource_mut::<CarRes>().0.pos = next;
            sim.tick();
            ticks += 1;
            if sim.world.resource::<FinishedRes>().0 {
                break;
            }
        }
        assert!(
            sim.world.resource::<FinishedRes>().0,
            "teleporting through every gate must finish the run in {ticks} ticks"
        );

        let tape = sim.world.resource_mut::<TapeRes>().0.take().unwrap();
        assert_eq!(tape.tick_len(), ticks, "one tape tick per sim tick");
        assert_eq!(tape.header.finish_tick, ticks, "closed on the finish tick");
        assert_eq!(tape.header.sim_hz, crate::SIM_HZ as u16);
        assert_eq!(tape.header.track, gameplay_fingerprint(&doc));
        assert_eq!(tape.header.physics, crate::sim::car::physics_fingerprint());

        // The closed tape ignores further ticks.
        sim.world.insert_resource(TapeRes(Some(tape)));
        for _ in 0..10 {
            sim.tick();
        }
        let tape = sim.world.resource::<TapeRes>().0.as_ref().unwrap();
        assert_eq!(tape.tick_len(), ticks);

        // Inactive ticks never record.
        let mut sim2 = repame_sim::Sim::new(crate::SIM_STEP);
        register(&mut sim2);
        insert_resources(&mut sim2.world);
        sim2.world.insert_resource(TapeRes(Some(ReplayTape::new(
            gameplay_fingerprint(&doc),
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        ))));
        for _ in 0..10 {
            sim2.tick();
        }
        assert_eq!(
            sim2.world
                .resource::<TapeRes>()
                .0
                .as_ref()
                .unwrap()
                .tick_len(),
            0
        );
    }
}

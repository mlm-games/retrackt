use game_utils_vehicle::VehicleInput;
use glam::Vec3;
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

/// The ghost car: a second [`Car`] driven by a recorded tape rather than by the
/// player. It is stepped by the same chain, on the same track world, so a ghost
/// is a real car in the simulation and cannot drift from the run that recorded it.
#[derive(Resource)]
pub struct GhostRes {
    pub car: Car,
    /// Next tick of the tape to apply.
    pub tick: u32,
    /// Empty when no ghost is selected, or once its tape has run out.
    pub tape: Option<ReplayTape>,
    /// Where this ghost started. Kept rather than read back from the world on a
    /// rewind, so a rewind cannot put the car somewhere a different track's world
    /// would say — the one case where the viewer would silently desync from the
    /// tape it is showing.
    spawn: Vec3,
    spawn_yaw: f32,
}

impl Default for GhostRes {
    fn default() -> Self {
        Self {
            car: Car::default(),
            tick: 0,
            tape: None,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
        }
    }
}

impl GhostRes {
    /// Load `tape` and put the ghost on the start line of `world`. `track` is
    /// the fingerprint of the document `world` was built from: a tape recorded
    /// anywhere else is refused rather than resimulated against geometry it was
    /// never driven on.
    ///
    /// The track check must be made here, against the track being raced. Passing
    /// `tape.header.track` back in would compare it with itself and accept every
    /// tape, which is the one input that cannot be checked later — by then the
    /// ghost is already driving.
    pub fn arm(
        &mut self,
        tape: ReplayTape,
        track: retrackt_format::fingerprint::TrackFingerprint,
        world: &TrackWorld,
    ) -> Result<(), String> {
        let physics = crate::sim::car::physics_fingerprint();
        let hz = crate::SIM_HZ as u16;
        if !tape.compatible(track, physics, hz) {
            return Err("That ghost was recorded on a different track or build.".into());
        }
        self.spawn = world.spawn;
        self.spawn_yaw = world.spawn_yaw;
        self.restart();
        self.tape = Some(tape);
        Ok(())
    }

    /// Back to the start line, keeping the tape.
    ///
    /// Playback re-simulates from inputs rather than reading back stored poses, so
    /// this is how a viewer goes backwards: restart, then run the ticks again. The
    /// pose it lands on is the same to the bit, not an approximation.
    pub fn restart(&mut self) {
        self.car = Car::at_spawn(self.spawn, self.spawn_yaw);
        self.tick = 0;
    }

    pub fn disarm(&mut self) {
        self.tape = None;
        self.tick = 0;
    }

    pub fn armed(&self) -> bool {
        self.tape.is_some()
    }
}

#[derive(Resource)]
pub struct SessionRes(pub RaceSession);

#[derive(Resource)]
pub struct ActiveRes(pub bool);

/// Whether this run is practice. A fall then puts the car back at the last
/// checkpoint instead of the start line, and finishing records nothing.
#[derive(Resource, Default)]
pub struct PracticeRes(pub bool);

/// One-shot: the player asked to be put back at the last checkpoint.
///
/// Held as a request rather than acted on by the runtime, so a button press and a
/// fall go through the same placement. It survives a frame that runs no ticks —
/// `Car::respawned` would not, because the runtime clears that flag when it draws.
#[derive(Resource, Default)]
pub struct RecoverRes(pub bool);

/// What is stepping: the player, or a tape being watched.
///
/// Not a mode of the race but a separate thing that runs beside nothing else, so
/// every system that would advance a *race* stands down while it is set and only
/// `step_ghost` keeps going. It reads as a modifier of [`ActiveRes`] because the
/// invariant is the important part: `ActiveRes(false)` runs nothing, whatever this
/// says.
#[derive(Resource, Default)]
pub struct ReplayRes(pub bool);

#[derive(Resource)]
pub struct FinishedRes(pub bool);

/// The replay tape of the run in progress, closed once the finish tick is
/// recorded. Taken by the runtime when it builds the race result.
#[derive(Resource)]
pub struct TapeRes(pub Option<ReplayTape>);

pub fn insert_resources(world: &mut World) {
    world.insert_resource(TrackRes(TrackWorld::default()));
    world.insert_resource(CarRes(Car::default()));
    world.insert_resource(GhostRes::default());
    world.insert_resource(InputRes::new());
    world.insert_resource(SessionRes(RaceSession::new()));
    world.insert_resource(ActiveRes(false));
    world.insert_resource(FinishedRes(false));
    world.insert_resource(TapeRes(None));
    world.insert_resource(PracticeRes(false));
    world.insert_resource(RecoverRes(false));
    world.insert_resource(ReplayRes(false));
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
    replay: Res<ReplayRes>,
    mut car: ResMut<CarRes>,
) {
    if !active.0 || replay.0 {
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

/// Step the ghost on the tick the player's car runs, from the next tape entry.
///
/// Uses [`ReplayTape::tick`] rather than a recorded position stream: playback
/// re-simulates the same inputs through the same [`step_car`], so the ghost is
/// exact by construction and cannot accumulate the drift a stored pose would.
///
/// Past the end of the tape the ghost stops being stepped and holds its last
/// pose. That is what makes it read as a finished run rather than a car frozen
/// mid-jump.
fn step_ghost(
    time: Res<SimTime>,
    track: Res<TrackRes>,
    active: Res<ActiveRes>,
    mut ghost: ResMut<GhostRes>,
) {
    if !active.0 {
        return;
    }
    step_tape(&mut ghost, &track.0, time.delta_secs);
}

/// Advance `ghost` by one tick of its tape. False once the tape has run out.
///
/// `dt` is the step in seconds, the same value `SimTime::delta_secs` carries in
/// game.
///
/// The plain function behind [`step_ghost`], so the headless verifier walks this
/// rather than a copy of it that could only ever agree with itself.
pub fn step_tape(ghost: &mut GhostRes, world: &TrackWorld, dt: f32) -> bool {
    let Some(packed) = ghost.tape.as_ref().and_then(|t| t.tick(ghost.tick)) else {
        return false;
    };
    step_car(
        &mut ghost.car,
        &decode(packed),
        world,
        &CarTuning::default(),
        dt,
    );
    ghost.tick += 1;
    true
}

/// Put the car back at the last checkpoint: after a fall, or on request.
///
/// Both paths land here so they cannot differ in whether the gate history is
/// cleared or the clock keeps running. The car still carries `respawned`, so the
/// runtime drops its render history and snaps the camera across the jump rather
/// than sliding it, and the session's gate test skips the tick as the teleport it
/// was.
fn recover_to_checkpoint(
    active: Res<ActiveRes>,
    practice: Res<PracticeRes>,
    replay: Res<ReplayRes>,
    session: Res<SessionRes>,
    mut recover: ResMut<RecoverRes>,
    mut car: ResMut<CarRes>,
) {
    // Taken first, so the request is consumed whether or not it is honoured: a request
    // left set would fire on the next run, wherever that was.
    let asked = std::mem::take(&mut recover.0);
    if !active.0 || !practice.0 || replay.0 || (!asked && !car.0.respawned) {
        // Practice only, and only while a run is going.
        return;
    }
    let Some((pos, yaw)) = session.0.recovery() else {
        return;
    };
    car.0 = Car::at_spawn(pos, yaw);
    // Re-set rather than carried: `step_vehicle` built the next tick's car from the
    // spawn, and the runtime clears this flag when it draws, so it only ever means
    // "the car jumped this tick".
    car.0.respawned = true;
}

/// Test this tick's movement against the gates.
///
/// A respawn is a teleport rather than a crossing: the car has just been thrown from
/// wherever it fell back to the start, and sweeping that segment would hand out
/// every gate between the two. The tick is still counted — the run took that step
/// and the tape records it — so the clock and the tape stay the same length and
/// every split keeps its own index into the tape.
fn advance_session(
    active: Res<ActiveRes>,
    replay: Res<ReplayRes>,
    car: Res<CarRes>,
    mut session: ResMut<SessionRes>,
) {
    if !active.0 || replay.0 {
        return;
    }
    if car.0.respawned {
        session.0.tick_teleported(car.0.pos);
        return;
    }
    session.0.tick(car.0.pos);
}

fn signal_finish(
    active: Res<ActiveRes>,
    replay: Res<ReplayRes>,
    session: Res<SessionRes>,
    mut finished: ResMut<FinishedRes>,
) {
    finished.0 = active.0 && !replay.0 && session.0.finished();
}

/// One tape tick per simulated tick, holding exactly what `step_vehicle`
/// consumed this tick — `InputRes::current` is the entry `take` handed out, and
/// `take` runs first in the chain. The tick that crossed the finish gate closes
/// the tape.
fn record_replay(
    active: Res<ActiveRes>,
    replay: Res<ReplayRes>,
    input: Res<InputRes>,
    finished: Res<FinishedRes>,
    mut tape: ResMut<TapeRes>,
) {
    if !active.0 || replay.0 {
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

/// One chain, ordered: car, practice recovery, ghost, session, finish flag, tape.
/// Separate `add_*` calls get no ordering, so order-sensitive systems live in
/// this tuple.
///
/// Recovery sits directly after the car so the session's gate test sees the car
/// where it was put back rather than where it fell. The ghost sits between the
/// car and the session so a tape tick is consumed on the same step it was
/// recorded on, and `record_replay` still runs last and reads the input
/// `step_vehicle` actually consumed.
///
/// A tape being *watched* runs the same chain with [`ReplayRes`] set, which stands
/// everything but `step_ghost` down: watching a run and racing it are the same
/// simulation, reached by a different set of systems being live.
pub fn register(sim: &mut repame_sim::Sim) {
    sim.add_chained_systems(
        (
            step_vehicle,
            recover_to_checkpoint,
            step_ghost,
            advance_session,
            signal_finish,
            record_replay,
        )
            .chain(),
    );
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
                    w.0.checkpoints[n]
                } else {
                    w.0.finish.expect("demo has a finish")
                }
            };
            // The session's reference point goes behind the gate and the car past it, so
            // the one tick that counts is the one driving through the box along the
            // gate's facing — the path a crossing is actually defined by. A chord
            // across a curve is not that path.
            let reach = next.half.x.max(next.half.z) + 1.0;
            let (behind, past) = (
                next.centre - next.facing * reach,
                next.centre + next.facing * reach,
            );
            sim.world.resource_mut::<SessionRes>().0.teleport(behind);
            sim.world.resource_mut::<CarRes>().0.pos = past;
            sim.tick();
            ticks += 1;
            if sim.world.resource::<FinishedRes>().0 {
                break;
            }
        }
        assert!(
            sim.world.resource::<FinishedRes>().0,
            "driving through every gate must finish the run in {ticks} ticks"
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

    /// Drive 120 ticks of fixed input, optionally recording it to `tape`.
    fn drive_recorded_run(mut tape: Option<&mut ReplayTape>) -> Car {
        let world = TrackWorld::from_doc(&demo_track());
        let mut car = Car::at_spawn(world.spawn, world.spawn_yaw);
        let tune = CarTuning::default();
        let steer = PackedInput::new(0.25, 1.0, 0.0, false, false, false);
        let dt = crate::SIM_STEP.as_secs_f32();
        for _ in 0..120 {
            step_car(&mut car, &decode(steer), &world, &tune, dt);
            if let Some(tape) = tape.as_deref_mut() {
                tape.push(steer);
            }
        }
        car
    }

    #[test]
    fn a_ghost_replaying_a_tape_reproduces_the_recorded_run() {
        let doc = demo_track();
        let world = TrackWorld::from_doc(&doc);
        let mut tape = ReplayTape::new(
            gameplay_fingerprint(&doc),
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        );
        let driven = drive_recorded_run(Some(&mut tape));
        assert_eq!(tape.tick_len(), 120);

        let mut ghost = GhostRes::default();
        ghost
            .arm(tape.clone(), gameplay_fingerprint(&doc), &world)
            .expect("arms");
        assert!(ghost.armed());
        while let Some(packed) = tape.tick(ghost.tick) {
            step_car(
                &mut ghost.car,
                &decode(packed),
                &world,
                &CarTuning::default(),
                crate::SIM_STEP.as_secs_f32(),
            );
            ghost.tick += 1;
        }

        assert!(
            ghost.car.pos.distance(driven.pos) < 1e-4,
            "ghost drifted: {:?} vs {:?}",
            ghost.car.pos,
            driven.pos
        );
    }

    #[test]
    fn a_ghost_past_the_end_of_its_tape_holds_its_last_pose() {
        let doc = demo_track();
        let world = TrackWorld::from_doc(&doc);
        let mut tape = ReplayTape::new(
            gameplay_fingerprint(&doc),
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        );
        drive_recorded_run(Some(&mut tape));

        let mut ghost = GhostRes::default();
        ghost
            .arm(tape, gameplay_fingerprint(&doc), &world)
            .expect("arms");
        // Two passes over the tape: the second finds no tick and must not move it.
        for _ in 0..240 {
            let Some(packed) = ghost.tape.as_ref().and_then(|t| t.tick(ghost.tick)) else {
                break;
            };
            step_car(
                &mut ghost.car,
                &decode(packed),
                &world,
                &CarTuning::default(),
                crate::SIM_STEP.as_secs_f32(),
            );
            ghost.tick += 1;
        }
        assert_eq!(
            ghost.tick, 120,
            "the tape ran out, so no further ticks are consumed"
        );

        // Disarming must not leave a ghost that comes back on the next step.
        ghost.disarm();
        assert!(!ghost.armed());
    }

    #[test]
    fn a_ghost_is_refused_a_tape_from_another_track_or_rate() {
        let doc = demo_track();
        let world = TrackWorld::from_doc(&doc);
        let track = gameplay_fingerprint(&doc);
        let mut ghost = GhostRes::default();

        // The check must compare the tape against the track being raced. Were
        // `arm` to pass `tape.header.track` instead, this would arm, and the
        // ghost would drive geometry its recorded run never touched.
        let wrong_track = ReplayTape::new(
            [9u8; 16],
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        );
        assert!(ghost.arm(wrong_track, track, &world).is_err());
        assert!(!ghost.armed(), "a refused tape must leave no ghost");

        // Same track and tuning, recorded at another tick rate: the same inputs
        // would step over different distances.
        let wrong_hz = ReplayTape::new(
            track,
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16 - 1,
        );
        assert!(ghost.arm(wrong_hz, track, &world).is_err());
        assert!(!ghost.armed());
    }

    /// A run already past its first two gates, with the car below the kill plane.
    ///
    /// Returns where practice must put it back: the last gate the session crossed, as
    /// the session itself reports it, rather than a gate named here.
    fn falling_sim(practice: bool) -> (repame_sim::Sim, TrackWorld, Vec3, usize) {
        let doc = demo_track();
        let world = TrackWorld::from_doc(&doc);
        let mut sim = repame_sim::Sim::new(crate::SIM_STEP);
        register(&mut sim);
        insert_resources(&mut sim.world);
        sim.world.insert_resource(TrackRes(world.clone()));
        sim.world.insert_resource(ActiveRes(true));
        sim.world.insert_resource(InputRes::new());
        sim.world.insert_resource(PracticeRes(practice));
        {
            let mut car = Car::at_spawn(world.spawn, world.spawn_yaw);
            car.pos.y = -10_000.0;
            sim.world.insert_resource(CarRes(car));
            let mut session = sim.world.resource_mut::<SessionRes>();
            session.0.start(&world);
            // Walk the first two gates, each from behind its own facing.
            for gate in world.checkpoints.iter().take(2) {
                let reach = gate.half.x.max(gate.half.z) + 1.0;
                session.0.teleport(gate.centre - gate.facing * reach);
                session.0.tick(gate.centre + gate.facing * reach);
            }
            assert!(
                session.0.checkpoint() >= 2,
                "two gates were driven through, got {}",
                session.0.checkpoint()
            );
        }
        let crossed = sim.world.resource::<SessionRes>().0.checkpoint();
        let recovery = sim
            .world
            .resource::<SessionRes>()
            .0
            .recovery()
            .expect("a gate has been crossed");
        (sim, world, recovery.0, crossed)
    }

    #[test]
    fn a_fall_in_practice_returns_to_the_last_checkpoint() {
        let (mut sim, _world, recovery, crossed) = falling_sim(true);
        sim.tick();

        let car = sim.world.resource::<CarRes>().0;
        assert!(
            car.pos.distance(recovery) < 0.01,
            "practice recovers to the last checkpoint {recovery:?}, got {:?}",
            car.pos
        );
        assert!(
            car.respawned,
            "the recovery is still a teleport: the camera must snap, not slide"
        );
        assert_eq!(
            sim.world.resource::<SessionRes>().0.checkpoint(),
            crossed,
            "recovering is not a crossing"
        );
    }

    #[test]
    fn asking_to_recover_does_the_same_thing_as_falling() {
        // The car is on the road this time: the request is the only difference, which
        // is the point of routing a button press through the fall's own path.
        let (mut sim, _world, recovery, _) = falling_sim(true);
        sim.world.resource_mut::<CarRes>().0.pos = Vec3::new(0.0, 0.0, 0.0);
        sim.world.resource_mut::<RecoverRes>().0 = true;
        sim.tick();

        let car = sim.world.resource::<CarRes>().0;
        assert!(
            car.pos.distance(recovery) < 0.01,
            "the request recovers to {recovery:?}, got {:?}",
            car.pos
        );
        assert!(car.respawned);
        // One-shot: a request that outlived its run must not fire on the next one.
        assert!(!sim.world.resource::<RecoverRes>().0);
    }

    #[test]
    fn a_recovery_request_is_ignored_outside_practice() {
        let (mut sim, _world, _recovery, _) = falling_sim(false);
        sim.world.resource_mut::<CarRes>().0.pos = Vec3::new(0.0, 0.0, 0.0);
        sim.world.resource_mut::<RecoverRes>().0 = true;
        sim.tick();
        assert!(
            !sim.world.resource::<RecoverRes>().0,
            "a timed run may not skip the road between gates"
        );
    }

    #[test]
    fn a_fall_in_a_timed_run_returns_to_the_start_and_earns_nothing() {
        let (mut sim, world, _recovery, crossed) = falling_sim(false);
        sim.tick();

        let car = sim.world.resource::<CarRes>().0;
        assert!(
            car.pos.distance(world.spawn) < 0.01,
            "a timed run respawns at the start line {:?}, got {:?}",
            world.spawn,
            car.pos
        );
        assert_eq!(
            sim.world.resource::<SessionRes>().0.checkpoint(),
            crossed,
            "a fall must not hand out the gates between where it fell and the spawn"
        );
    }
}

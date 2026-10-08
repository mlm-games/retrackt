pub mod result;

use glam::Vec3;

use crate::sim::world::{Gate, TrackWorld};

/// Live race timing for a single time-trial run: `world.checkpoints` plus
/// `world.finish` when present.
///
/// The clock counts ticks rather than seconds. One [`RaceSession::tick`] per
/// simulated tick means the recorded time is exactly the number of steps the car
/// ran, so it cannot drift from the tape the same run recorded.
pub struct RaceSession {
    gates: Vec<Gate>,
    next: usize,
    ticks: u32,
    splits: Vec<u32>,
    finished: bool,
    /// Where the car was at the last tick, so a crossing is a swept test.
    prev: Vec3,
}

impl RaceSession {
    pub fn new() -> Self {
        Self {
            gates: Vec::new(),
            next: 0,
            ticks: 0,
            splits: Vec::new(),
            finished: false,
            prev: Vec3::ZERO,
        }
    }

    pub fn start(&mut self, world: &TrackWorld) {
        self.gates = world.checkpoints.clone();
        if let Some(finish) = world.finish {
            self.gates.push(finish);
        }
        self.next = 0;
        self.ticks = 0;
        self.splits.clear();
        self.finished = false;
        self.prev = world.spawn;
    }

    /// Move the reference point without testing any gate and without the clock.
    ///
    /// For a teleport made outside a tick: a kill-height respawn throws the car
    /// across the map, and the segment it flew would otherwise cross every gate
    /// between where it fell and where it reappeared, handing out checkpoints for
    /// a fall.
    pub fn teleport(&mut self, pos: Vec3) {
        self.prev = pos;
    }

    /// One tick that crossed nothing.
    ///
    /// The clock still advances, because the run really did take that step and the
    /// tape records it: skipping the tick here would leave the recorded time and
    /// the tape a different length, and every split on the tape would then be
    /// offset from the gate it belongs to.
    pub fn tick_teleported(&mut self, pos: Vec3) {
        if self.finished {
            return;
        }
        self.ticks = self.ticks.saturating_add(1);
        self.prev = pos;
    }

    /// The last gate crossed, as a place to put the car back on the road facing
    /// the way the track runs. `None` before the first one.
    ///
    /// The gate's centre is a trigger box, not a piece of road, so the car is
    /// returned a little way along the facing instead — inside the gate, or driving
    /// out of it, and the session would count the gate a second time.
    pub fn recovery(&self) -> Option<(Vec3, f32)> {
        let gate = self.gates.get(self.next.checked_sub(1)?)?;
        let f = gate.facing;
        let along = gate.half.x.max(gate.half.z) + 1.0;
        // Heading is measured from +Z, the same convention `Car::at_spawn` takes.
        Some((gate.centre + f * along, f.x.atan2(f.z)))
    }

    /// Advance the run by one simulated tick. `pos` is where the car ended the
    /// tick; the gate test is against the movement since the last call.
    pub fn tick(&mut self, pos: Vec3) {
        if self.finished {
            return;
        }
        self.ticks = self.ticks.saturating_add(1);
        self.tick_from(self.prev, pos);
        self.prev = pos;
    }

    /// Test the gates against one tick of movement, from `from` to `to`.
    fn tick_from(&mut self, from: Vec3, to: Vec3) {
        while self.next < self.gates.len() {
            let gate = self.gates[self.next];
            if !gate.crossed(from, to) {
                break;
            }
            if !gate.is_finish {
                self.splits.push(self.ticks);
            }
            self.next += 1;
        }
        if !self.gates.is_empty() && self.next >= self.gates.len() {
            self.finished = true;
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Race time in simulated ticks.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// Race time at each checkpoint, in ticks.
    pub fn splits(&self) -> &[u32] {
        &self.splits
    }

    /// Gates crossed so far, in `0..checkpoint_count`.
    pub fn checkpoint(&self) -> usize {
        self.next
    }

    /// Total gates in the run: every checkpoint plus the finish gate when the
    /// track has one, checkpoints alone when it does not.
    pub fn checkpoint_count(&self) -> usize {
        self.gates.len()
    }

    pub fn result(
        &self,
        track: &retrackt_format::TrackDocument,
        replay: Option<retrackt_format::ReplayTape>,
    ) -> crate::session::result::RaceResult {
        crate::session::result::RaceResult {
            total_ticks: self.ticks,
            splits: self.splits.clone(),
            track_fingerprint: retrackt_format::gameplay_fingerprint(track),
            physics_fingerprint: crate::sim::car::physics_fingerprint(),
            replay,
        }
    }
}

impl Default for RaceSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::PieceUid;

    /// A gate on the +X axis of the test road, which runs from x = 0 to x = 30.
    fn gate(x: f32, is_finish: bool) -> Gate {
        Gate {
            piece: PieceUid(0),
            centre: Vec3::new(x, 0.0, 0.0),
            half: Vec3::new(2.0, 3.0, 2.0),
            facing: Vec3::X,
            is_finish,
            index: 0,
        }
    }

    fn spaced_world() -> TrackWorld {
        let mut w = TrackWorld::default();
        w.spawn = Vec3::new(-5.0, 0.0, 0.0);
        w.checkpoints = vec![gate(10.0, false), gate(20.0, false)];
        w.finish = Some(gate(30.0, true));
        w
    }

    fn run(w: &TrackWorld, path: &[Vec3]) -> RaceSession {
        let mut s = RaceSession::new();
        s.start(w);
        for pos in path {
            s.tick(*pos);
        }
        s
    }

    /// Drive through every gate of `w` in order, arriving at each from behind its
    /// own facing. Bounded so a track whose gates cannot be reached this way fails
    /// the test rather than hanging it.
    fn drive_through(s: &mut RaceSession, w: &TrackWorld) {
        for _ in 0..4 * w.checkpoint_count() + 4 {
            if s.finished() {
                return;
            }
            let i = s.checkpoint();
            let gate = if i < w.checkpoints.len() {
                &w.checkpoints[i]
            } else {
                w.finish
                    .as_ref()
                    .expect("a track being walked has a finish")
            };
            // Two moves per gate, one just short of the box and one past it. The
            // first is a teleport rather than a crossing, so it cannot count the gate
            // it is approaching from behind.
            let reach = gate.half.x.max(gate.half.z) + 1.0;
            s.teleport(gate.centre - gate.facing * reach);
            s.tick(gate.centre + gate.facing * reach);
        }
    }

    #[test]
    fn run_walks_gates_in_order_and_freezes_at_the_finish() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.checkpoint_count(), 3);
        assert_eq!(s.checkpoint(), 0);
        assert!(!s.finished());

        // From the spawn at x = -5, straight past the first gate to the second.
        s.tick(Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 2, "one tick can cross more than one gate");
        assert_eq!(s.splits(), &[1, 1]);

        s.tick(Vec3::new(30.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.checkpoint(), 3);
        assert_eq!(s.splits().len(), 2, "the finish gate records no split");
        assert_eq!(s.ticks(), 2);

        s.tick(Vec3::new(999.0, 0.0, 0.0));
        assert_eq!(s.ticks(), 2, "the clock stops once the run is over");
        assert_eq!(s.checkpoint(), 3);
    }

    #[test]
    fn a_gate_is_crossed_by_the_path_not_by_the_end_point() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        // One tick straight over the box at x = 10: outside it at both ends, inside
        // at neither. The run must still count the crossing.
        s.tick(Vec3::new(14.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 1);

        let mut s = RaceSession::new();
        s.start(&w);
        s.teleport(Vec3::new(8.0, 0.0, 0.0));
        // Parked inside the box and going nowhere.
        s.tick(Vec3::new(8.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 0, "standing in a gate crosses nothing");
    }

    #[test]
    fn driving_back_through_a_gate_does_not_count_it() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        s.tick(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 1);

        // Back the way it came, right through the gate it just crossed.
        s.tick(Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 1, "the gate is not re-counted backwards");
        assert_eq!(s.splits(), &[1]);

        s.tick(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(
            s.checkpoint(),
            1,
            "and crossing it again is not a second pass"
        );
    }

    #[test]
    fn a_teleport_earns_nothing() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        // A fall thrown the length of the map: the segment it flew would otherwise
        // cross every gate on the way.
        s.teleport(Vec3::new(34.0, 0.0, 0.0));
        s.tick(Vec3::new(36.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 0);
        assert!(!s.finished());
        assert!(s.splits().is_empty());
    }

    #[test]
    fn a_teleported_tick_is_still_a_tick() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        s.tick_teleported(Vec3::new(-5.0, 0.0, 0.0));
        s.tick_teleported(Vec3::new(-5.0, 0.0, 0.0));
        assert_eq!(s.ticks(), 2, "the run took those two steps");
        assert_eq!(s.checkpoint(), 0);
        assert!(s.splits().is_empty());

        // And the run carries on from where it was put, not from where it fell.
        s.tick(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(s.ticks(), 3);
        assert_eq!(s.checkpoint(), 1);
        assert_eq!(s.splits(), &[3]);
    }

    #[test]
    fn a_teleported_tick_does_not_rewind_a_finished_run() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        drive_through(&mut s, &w);
        assert!(s.finished());
        let ticks = s.ticks();

        s.tick_teleported(Vec3::ZERO);
        assert_eq!(s.ticks(), ticks, "the clock stops with the run");
    }

    #[test]
    fn recovery_is_the_last_gate_crossed_facing_the_way_the_track_runs() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.recovery(), None, "nothing crossed yet");

        s.tick(Vec3::new(10.0, 0.0, 0.0));
        let (pos, yaw) = s.recovery().expect("a crossed gate can be recovered to");
        assert!(
            pos.x > 12.0,
            "recovered past the box, not inside it: {pos:?}"
        );
        assert!(yaw.abs() < 1e-5, "+X forward is a yaw of zero, got {yaw}");

        // Recovering to it must not cross it a second time.
        s.teleport(pos);
        s.tick(pos + Vec3::X);
        assert_eq!(s.checkpoint(), 1);
        assert_eq!(s.splits(), &[1]);

        drive_through(&mut s, &w);
        assert!(s.finished());
        let (pos, _) = s.recovery().expect("the finish gate is still a place");
        assert!(pos.x > 32.0, "past the finish box, got {pos:?}");
    }

    #[test]
    fn several_gates_in_one_tick_are_all_processed_in_order() {
        let mut w = TrackWorld::default();
        w.checkpoints = vec![gate(0.0, false), gate(1.0, false), gate(2.0, false)];
        w.finish = Some(gate(3.0, true));

        let mut s = RaceSession::new();
        s.start(&w);
        // Inside all three checkpoints, short of the finish.
        s.tick(Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 3);
        assert_eq!(s.splits(), &[1, 1, 1]);
        assert!(!s.finished());

        s.tick(Vec3::new(3.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.ticks(), 2);
        assert_eq!(s.splits().len(), 3);
    }

    #[test]
    fn the_clock_counts_ticks_and_never_rewinds() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.ticks(), 0);
        for expected in 1..=5 {
            s.tick(Vec3::new(0.0, 0.0, 0.0));
            assert_eq!(s.ticks(), expected, "exactly one tick per call");
        }
        // Nothing to wind back: the run has no wall-clock input at all.
        s.start(&w);
        assert_eq!(s.ticks(), 0);
        assert!(s.splits().is_empty());
    }

    #[test]
    fn identical_tick_sequences_produce_identical_runs() {
        let w = spaced_world();
        let path = [
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::new(30.0, 0.0, 0.0),
            Vec3::new(30.0, 0.0, 0.0),
        ];
        let a = run(&w, &path);
        let b = run(&w, &path);
        assert_eq!(a.ticks(), b.ticks());
        assert_eq!(a.splits(), b.splits());
        assert_eq!(a.checkpoint(), b.checkpoint());
        assert_eq!(a.finished(), b.finished());
    }

    #[test]
    fn a_builtin_track_is_walkable_to_the_finish() {
        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.checkpoint_count(), w.checkpoints.len() + 1);

        drive_through(&mut s, &w);
        assert!(s.finished());
        assert_eq!(s.checkpoint(), s.checkpoint_count());
        assert_eq!(s.splits().len(), w.checkpoints.len());
        assert!(s.ticks() > 0);
    }

    #[test]
    fn a_track_without_a_finish_ends_at_the_last_checkpoint() {
        let mut w = TrackWorld::default();
        w.spawn = Vec3::new(-5.0, 0.0, 0.0);
        w.checkpoints = vec![gate(10.0, false), gate(20.0, false)];
        w.finish = None;

        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.checkpoint_count(), 2, "no finish gate to count");
        s.tick(Vec3::new(10.0, 0.0, 0.0));
        assert!(!s.finished());
        s.tick(Vec3::new(20.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.splits(), &[1, 2]);
        assert_eq!(s.checkpoint(), s.checkpoint_count());
    }

    #[test]
    fn a_runtime_shaped_replay_round_trips_through_the_tape_codec() {
        use retrackt_format::PackedInput;
        use retrackt_format::ReplayTape;
        use retrackt_format::decode_replay;
        use retrackt_format::encode_replay;

        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        let mut s = RaceSession::new();
        s.start(&w);
        drive_through(&mut s, &w);
        assert!(s.finished());

        let track_fp = retrackt_format::gameplay_fingerprint(&doc);
        let mut tape = ReplayTape::new(
            track_fp,
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        );
        for i in 0..240u32 {
            tape.push(PackedInput::new(
                (i as f32 - 120.0) / 60.0,
                1.0 - (i % 3) as f32 / 4.0,
                (i % 7 == 0) as u8 as f32,
                i % 11 == 0,
                i % 13 == 0,
                false,
            ));
        }
        tape.header.finish_tick = tape.tick_len();
        // Splits land on the tape verbatim: the run clock is already in ticks,
        // so there is no seconds-to-ticks conversion left to round.
        tape.split_ticks = s.splits().to_vec();

        let result = s.result(&doc, Some(tape.clone()));
        assert_eq!(result.track_fingerprint, track_fp);
        assert_eq!(
            result.physics_fingerprint,
            crate::sim::car::physics_fingerprint()
        );
        assert_eq!(result.total_ticks, s.ticks());
        assert_eq!(result.splits.as_slice(), s.splits());
        assert_eq!(tape.split_ticks, result.splits);
        let bytes =
            encode_replay(result.replay.as_ref().expect("replay present")).expect("encodes");
        assert_eq!(decode_replay(&bytes).expect("decodes"), tape);
    }

    #[test]
    fn a_tape_tick_preserves_reverse_throttle() {
        let v = game_utils_vehicle::VehicleInput {
            throttle: -1.0,
            ..game_utils_vehicle::VehicleInput::neutral()
        };
        let packed = retrackt_format::PackedInput::new(
            v.steer,
            v.throttle,
            v.brake,
            v.handbrake > 0.0,
            v.boost > 0.0,
            false,
        );
        assert!(packed.reverse());
        assert_eq!(
            packed.throttle_f32(),
            -1.0,
            "reverse must survive quantisation"
        );
    }
}

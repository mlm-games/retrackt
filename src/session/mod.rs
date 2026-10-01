pub mod result;

use glam::Vec3;

use crate::sim::world::{Gate, TrackWorld};

/// Live race timing for a single time-trial run: `world.checkpoints` plus
/// `world.finish` when present; clock, splits and gate order follow the `dt` sequence.
pub struct RaceSession {
    gates: Vec<Gate>,
    next: usize,
    elapsed: f32,
    splits: Vec<f32>,
    finished: bool,
}

impl RaceSession {
    pub fn new() -> Self {
        Self {
            gates: Vec::new(),
            next: 0,
            elapsed: 0.0,
            splits: Vec::new(),
            finished: false,
        }
    }

    pub fn start(&mut self, world: &TrackWorld) {
        self.gates = world.checkpoints.clone();
        if let Some(finish) = world.finish {
            self.gates.push(finish);
        }
        self.next = 0;
        self.elapsed = 0.0;
        self.splits.clear();
        self.finished = false;
    }

    pub fn update(&mut self, dt: f32, pos: Vec3) {
        if self.finished {
            return;
        }
        self.elapsed += dt.max(0.0);
        while self.next < self.gates.len() {
            let gate = self.gates[self.next];
            if !gate.contains(pos) {
                break;
            }
            if !gate.is_finish {
                self.splits.push(self.elapsed);
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

    pub fn time(&self) -> f32 {
        self.elapsed
    }

    pub fn splits(&self) -> &[f32] {
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
            total_time: self.elapsed,
            best_lap: self.elapsed,
            splits: self.splits.clone(),
            track_fingerprint: retrackt_format::gameplay_fingerprint(track),
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

    fn gate(centre: [f32; 3], half: [f32; 3], is_finish: bool) -> Gate {
        Gate {
            piece: PieceUid(0),
            centre: Vec3::from(centre),
            half: Vec3::from(half),
            is_finish,
            index: 0,
        }
    }

    fn spaced_world() -> TrackWorld {
        let mut w = TrackWorld::default();
        w.checkpoints = vec![
            gate([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
            gate([10.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
        ];
        w.finish = Some(gate([20.0, 0.0, 0.0], [2.0, 2.0, 2.0], true));
        w
    }

    fn run(w: &TrackWorld, path: &[(f32, Vec3)]) -> RaceSession {
        let mut s = RaceSession::new();
        s.start(w);
        for (dt, pos) in path {
            s.update(*dt, *pos);
        }
        s
    }

    #[test]
    fn run_walks_gates_in_order_and_freezes_at_the_finish() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.checkpoint_count(), 3);
        assert_eq!(s.checkpoint(), 0);
        assert!(!s.finished());

        s.update(0.25, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 0, "gates must be passed in order");
        assert!(s.splits().is_empty());

        s.update(0.25, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 1);
        assert_eq!(s.splits(), &[0.5]);

        s.update(0.25, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 2);
        assert_eq!(s.splits(), &[0.5, 0.75]);

        s.update(0.25, Vec3::new(20.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.checkpoint(), 3);
        assert_eq!(s.splits().len(), 2, "the finish gate records no split");
        assert_eq!(s.time(), 1.0);

        s.update(5.0, Vec3::new(999.0, 0.0, 0.0));
        assert_eq!(s.time(), 1.0, "the clock stops once the run is over");
        assert_eq!(s.checkpoint(), 3);
    }

    #[test]
    fn several_gates_in_one_tick_are_all_processed_in_order() {
        let mut w = TrackWorld::default();
        w.checkpoints = vec![
            gate([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
            gate([1.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
            gate([2.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
        ];
        w.finish = Some(gate([3.0, 0.0, 0.0], [2.0, 2.0, 2.0], true));

        let mut s = RaceSession::new();
        s.start(&w);
        // Inside all three checkpoints, short of the finish.
        s.update(0.25, Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(s.checkpoint(), 3);
        assert_eq!(s.splits(), &[0.25, 0.25, 0.25]);
        assert!(!s.finished());

        s.update(0.25, Vec3::new(3.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.time(), 0.5);
        assert_eq!(s.splits().len(), 3);
    }

    #[test]
    fn identical_dt_sequences_produce_identical_runs() {
        let w = spaced_world();
        let path = [
            (0.25, Vec3::new(0.0, 0.0, 0.0)),
            (0.25, Vec3::new(10.0, 0.0, 0.0)),
            (0.25, Vec3::new(20.0, 0.0, 0.0)),
            (0.25, Vec3::new(20.0, 0.0, 0.0)),
        ];
        let a = run(&w, &path);
        let b = run(&w, &path);
        assert_eq!(a.time(), b.time());
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

        for _ in 0..600 {
            let next = s.checkpoint();
            let gate = if next < w.checkpoints.len() {
                w.checkpoints[next]
            } else {
                w.finish.expect("demo track has a finish gate")
            };
            s.update(1.0 / 120.0, gate.centre);
            if s.finished() {
                break;
            }
        }
        assert!(s.finished());
        assert_eq!(s.checkpoint(), s.checkpoint_count());
        assert_eq!(s.splits().len(), w.checkpoints.len());
        assert!(s.time() > 0.0);
    }

    #[test]
    fn a_track_without_a_finish_ends_at_the_last_checkpoint() {
        let mut w = TrackWorld::default();
        w.checkpoints = vec![
            gate([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
            gate([10.0, 0.0, 0.0], [2.0, 2.0, 2.0], false),
        ];
        w.finish = None;

        let mut s = RaceSession::new();
        s.start(&w);
        assert_eq!(s.checkpoint_count(), 2, "no finish gate to count");
        s.update(0.5, Vec3::new(0.0, 0.0, 0.0));
        assert!(!s.finished());
        s.update(0.5, Vec3::new(10.0, 0.0, 0.0));
        assert!(s.finished());
        assert_eq!(s.splits(), &[0.5, 1.0]);
        assert_eq!(s.checkpoint(), s.checkpoint_count());
    }

    #[test]
    fn negative_dt_does_not_rewind_the_clock() {
        let w = spaced_world();
        let mut s = RaceSession::new();
        s.start(&w);
        s.update(0.5, Vec3::new(0.0, 0.0, 0.0));
        s.update(-10.0, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(s.time(), 0.5, "the clock must never go backwards");
        assert_eq!(s.checkpoint(), 2, "gates still advance when dt is clamped");
    }

    #[test]
    fn a_runtime_shaped_replay_round_trips_through_the_tape_codec() {
        use retrackt_format::PackedInput;
        use retrackt_format::ReplayTape;
        use retrackt_format::decode_replay;
        use retrackt_format::encode_replay;
        use retrackt_format::fingerprint::physics_fingerprint;
        use retrackt_format::replay::{PhysicsStamp, SIM_VERSION};

        let doc = retrackt_format::demo_track();
        let w = TrackWorld::from_doc(&doc);
        let mut s = RaceSession::new();
        s.start(&w);
        for _ in 0..600 {
            if s.finished() {
                break;
            }
            let next = s.checkpoint();
            let gate = if next < w.checkpoints.len() {
                w.checkpoints[next]
            } else {
                w.finish.expect("demo track has a finish gate")
            };
            s.update(1.0 / 120.0, gate.centre);
        }
        assert!(s.finished());

        let track_fp = retrackt_format::gameplay_fingerprint(&doc);
        let physics_fp = physics_fingerprint(SIM_VERSION, &PhysicsStamp::digest(b"car-tuning-v1"));
        let mut tape = ReplayTape::new(track_fp, physics_fp, crate::SIM_HZ as u16);
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
        tape.split_ticks = s
            .splits()
            .iter()
            .map(|t| (t * crate::SIM_HZ as f32).round() as u32)
            .collect();

        let result = s.result(&doc, Some(tape.clone()));
        assert_eq!(result.track_fingerprint, track_fp);
        assert_eq!(result.splits.as_slice(), s.splits());
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

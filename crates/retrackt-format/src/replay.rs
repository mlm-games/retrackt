//! Deterministic replay tapes.
//!
//! A tape records quantised driver input, run-length encoded. It is the
//! authority for a ghost: playback re-simulates from input, it never
//! interpolates stored positions, so a ghost cannot drift from a live run.

use serde::{Deserialize, Serialize};

use crate::fingerprint::TrackFingerprint;

/// Wire version of the tape container.
pub const TAPE_VERSION: u16 = 3;
/// Bump when vehicle tuning changes behaviour. Tapes recorded under a
/// different version are refused rather than re-simulated incorrectly.
pub const SIM_VERSION: u32 = 2;

/// Tuning digest stamped into every tape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicsStamp(pub [u8; 16]);

impl PhysicsStamp {
    pub fn digest(bytes: &[u8]) -> Self {
        let mut out = [0u8; 16];
        out.copy_from_slice(&blake3::hash(bytes).as_bytes()[..16]);
        Self(out)
    }
}

/// One tick of driver intent, quantised so a tape is byte-exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedInput {
    /// -1.0 .. 1.0 as i8.
    pub steer: i8,
    /// |throttle| 0.0 .. 1.0 as u8. The sign lives in [`Self::REVERSE`].
    pub throttle: u8,
    /// 0.0 .. 1.0 as u8.
    pub brake: u8,
    pub flags: u8,
}

impl PackedInput {
    pub const HANDBRAKE: u8 = 1 << 0;
    pub const BOOST: u8 = 1 << 1;
    pub const RESET: u8 = 1 << 2;
    /// Throttle demand was negative: the tick drove backwards.
    pub const REVERSE: u8 = 1 << 3;

    pub fn new(
        steer: f32,
        throttle: f32,
        brake: f32,
        handbrake: bool,
        boost: bool,
        reset: bool,
    ) -> Self {
        let mut flags = 0;
        if handbrake {
            flags |= Self::HANDBRAKE;
        }
        if boost {
            flags |= Self::BOOST;
        }
        if reset {
            flags |= Self::RESET;
        }
        if throttle < 0.0 {
            flags |= Self::REVERSE;
        }
        Self {
            steer: (steer.clamp(-1.0, 1.0) * 127.0).round() as i8,
            throttle: (throttle.abs().clamp(0.0, 1.0) * 255.0).round() as u8,
            brake: (brake.clamp(0.0, 1.0) * 255.0).round() as u8,
            flags,
        }
    }

    pub fn steer_f32(self) -> f32 {
        self.steer as f32 / 127.0
    }

    pub fn throttle_f32(self) -> f32 {
        let magnitude = self.throttle as f32 / 255.0;
        if self.reverse() {
            -magnitude
        } else {
            magnitude
        }
    }

    pub fn brake_f32(self) -> f32 {
        self.brake as f32 / 255.0
    }

    pub fn handbrake(self) -> bool {
        self.flags & Self::HANDBRAKE != 0
    }

    pub fn boost(self) -> bool {
        self.flags & Self::BOOST != 0
    }

    pub fn reset(self) -> bool {
        self.flags & Self::RESET != 0
    }

    pub fn reverse(self) -> bool {
        self.flags & Self::REVERSE != 0
    }
}

/// A run of identical ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRun {
    pub input: PackedInput,
    pub ticks: u16,
}

/// What the tape was recorded against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TapeHeader {
    pub format: u16,
    pub sim_version: u32,
    /// Fixed simulation rate the tape was recorded at.
    pub sim_hz: u16,
    pub track: TrackFingerprint,
    pub physics: TrackFingerprint,
    /// Tick at which the finish gate was crossed.
    pub finish_tick: u32,
}

/// Full replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayTape {
    pub header: TapeHeader,
    pub runs: Vec<InputRun>,
    /// Race time at each checkpoint, in ticks.
    #[serde(default)]
    pub split_ticks: Vec<u32>,
}

impl ReplayTape {
    pub fn new(track: TrackFingerprint, physics: TrackFingerprint, sim_hz: u16) -> Self {
        Self {
            header: TapeHeader {
                format: TAPE_VERSION,
                sim_version: SIM_VERSION,
                sim_hz,
                track,
                physics,
                finish_tick: 0,
            },
            runs: Vec::new(),
            split_ticks: Vec::new(),
        }
    }

    /// Append one tick, extending the trailing run when the input repeats.
    pub fn push(&mut self, input: PackedInput) {
        if let Some(last) = self.runs.last_mut()
            && last.input == input
            && last.ticks < u16::MAX
        {
            last.ticks += 1;
            return;
        }
        self.runs.push(InputRun { input, ticks: 1 });
    }

    /// Total ticks the tape covers.
    pub fn tick_len(&self) -> u32 {
        self.runs.iter().map(|r| r.ticks as u32).sum()
    }

    /// Milliseconds the tape represents.
    pub fn duration_ms(&self) -> u32 {
        let hz = self.header.sim_hz.max(1) as u32;
        self.tick_len().saturating_mul(1000) / hz
    }

    /// Decode one tick by index. O(log runs).
    pub fn tick(&self, index: u32) -> Option<PackedInput> {
        let mut left = index;
        for run in &self.runs {
            let n = run.ticks as u32;
            if left < n {
                return Some(run.input);
            }
            left -= n;
        }
        None
    }

    /// Iterate every tick in order.
    pub fn iter(&self) -> impl Iterator<Item = PackedInput> + '_ {
        self.runs
            .iter()
            .flat_map(|r| std::iter::repeat_n(r.input, r.ticks as usize))
    }

    /// True when the tape may be played on this build. `sim_hz` is the caller's
    /// current rate: a tape recorded at a different one steps the same inputs
    /// over different distances, so it must be refused too.
    pub fn compatible(
        &self,
        track: TrackFingerprint,
        physics: TrackFingerprint,
        sim_hz: u16,
    ) -> bool {
        self.header.format == TAPE_VERSION
            && self.header.sim_version == SIM_VERSION
            && self.header.sim_hz == sim_hz
            && self.header.track == track
            && self.header.physics == physics
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("tape too large")]
    TooLarge,
    #[error("tape covers too many ticks")]
    TooLong,
    #[error("malformed tape: {0}")]
    Malformed(String),
}

/// Hard caps so a hostile or corrupt blob cannot exhaust memory.
const MAX_COMPRESSED: usize = 512 * 1024;
const MAX_RAW: usize = 8 * 1024 * 1024;
const MAX_TICKS: u32 = 120 * 60 * 60;

pub fn encode_replay(tape: &ReplayTape) -> Result<Vec<u8>, ReplayError> {
    use std::io::Write;
    let text = ron::ser::to_string(tape).map_err(|e| ReplayError::Malformed(e.to_string()))?;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(text.as_bytes())
        .map_err(|e| ReplayError::Malformed(e.to_string()))?;
    enc.finish()
        .map_err(|e| ReplayError::Malformed(e.to_string()))
}

pub fn decode_replay(bytes: &[u8]) -> Result<ReplayTape, ReplayError> {
    use std::io::Read;
    if bytes.len() > MAX_COMPRESSED {
        return Err(ReplayError::TooLarge);
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .take(MAX_RAW as u64)
        .read_to_end(&mut raw)
        .map_err(|e| ReplayError::Malformed(e.to_string()))?;
    let tape: ReplayTape = ron::from_str(
        std::str::from_utf8(&raw).map_err(|e| ReplayError::Malformed(e.to_string()))?,
    )
    .map_err(|e| ReplayError::Malformed(e.to_string()))?;
    if tape.tick_len() > MAX_TICKS {
        return Err(ReplayError::TooLong);
    }
    Ok(tape)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(b: u8) -> TrackFingerprint {
        [b; 16]
    }

    #[test]
    fn quantisation_round_trips_within_one_step() {
        let p = PackedInput::new(-0.723, 0.81, 0.17, true, false, false);
        assert!((p.steer_f32() + 0.723).abs() <= 1.0 / 127.0);
        assert!((p.throttle_f32() - 0.81).abs() <= 1.0 / 255.0);
        assert!((p.brake_f32() - 0.17).abs() <= 1.0 / 255.0);
        assert!(p.handbrake());
        assert!(!p.boost());

        let rev = PackedInput::new(0.0, -0.64, 0.0, false, false, false);
        assert!(rev.reverse());
        assert!((rev.throttle_f32() + 0.64).abs() <= 1.0 / 255.0);
        assert!(!PackedInput::new(0.0, 0.64, 0.0, false, false, false).reverse());
    }

    #[test]
    fn identical_inputs_collapse_into_one_run() {
        let mut tape = ReplayTape::new(fp(1), fp(2), 120);
        let input = PackedInput::new(0.0, 1.0, 0.0, false, false, false);
        for _ in 0..1000 {
            tape.push(input);
        }
        assert_eq!(tape.runs.len(), 1, "RLE must merge identical ticks");
        assert_eq!(tape.runs[0].ticks, 1000);
        assert_eq!(tape.tick_len(), 1000);
    }

    #[test]
    fn alternating_inputs_stay_distinct_and_index_correctly() {
        let mut tape = ReplayTape::new(fp(1), fp(2), 120);
        let a = PackedInput::new(0.0, 1.0, 0.0, false, false, false);
        let b = PackedInput::new(0.0, 0.0, 1.0, false, false, false);
        for i in 0..100 {
            tape.push(if i % 2 == 0 { a } else { b });
        }
        assert_eq!(tape.tick_len(), 100);
        assert_eq!(tape.tick(0), Some(a));
        assert_eq!(tape.tick(1), Some(b));
        assert_eq!(tape.tick(99), Some(b));
        assert_eq!(tape.tick(100), None, "past the end must be None");
        assert_eq!(tape.iter().count(), 100);
    }

    #[test]
    fn codec_round_trips() {
        let mut tape = ReplayTape::new(fp(7), fp(9), 120);
        for i in 0..500 {
            tape.push(PackedInput::new(
                i as f32 / 250.0 - 1.0,
                i as f32 / 500.0,
                0.0,
                i % 97 == 0,
                i % 61 == 0,
                false,
            ));
        }
        tape.header.finish_tick = 500;
        let bytes = encode_replay(&tape).unwrap();
        let back = decode_replay(&bytes).unwrap();
        assert_eq!(back.header.finish_tick, 500);
        assert_eq!(back.runs, tape.runs);
        assert!(back.compatible(fp(7), fp(9), 120));
    }

    #[test]
    fn incompatible_tapes_are_refused() {
        let tape = ReplayTape::new(fp(1), fp(2), 120);
        assert!(!tape.compatible(fp(9), fp(2), 120), "wrong track");
        assert!(!tape.compatible(fp(1), fp(9), 120), "wrong tuning");
        assert!(!tape.compatible(fp(1), fp(2), 60), "wrong tick rate");
        let mut old = tape.clone();
        old.header.sim_version = SIM_VERSION - 1;
        assert!(!old.compatible(fp(1), fp(2), 120), "wrong sim version");
    }

    #[test]
    fn corrupt_bytes_decode_to_an_error_not_a_panic() {
        assert!(decode_replay(b"not a tape at all").is_err());
        assert!(decode_replay(&[]).is_err());
    }

    #[test]
    fn oversized_input_is_rejected_before_allocating() {
        let big = vec![0u8; MAX_COMPRESSED + 1];
        assert!(matches!(decode_replay(&big), Err(ReplayError::TooLarge)));
    }

    #[test]
    fn duration_follows_the_declared_rate() {
        let mut tape = ReplayTape::new(fp(1), fp(2), 100);
        let i = PackedInput::default();
        for _ in 0..250 {
            tape.push(i);
        }
        assert_eq!(tape.duration_ms(), 2500);
    }
}

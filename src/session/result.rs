use retrackt_format::ReplayTape;
use retrackt_format::fingerprint::TrackFingerprint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct RaceResult {
    /// Race time in simulated ticks, matching `ReplayTape::split_ticks`.
    pub total_ticks: u32,
    /// Race time at each checkpoint, in ticks.
    pub splits: Vec<u32>,
    pub track_fingerprint: TrackFingerprint,
    /// Vehicle tuning the run was set under. A record only means something
    /// against the physics that produced it.
    pub physics_fingerprint: TrackFingerprint,
    pub replay: Option<ReplayTape>,
}

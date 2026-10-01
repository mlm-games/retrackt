use retrackt_format::ReplayTape;
use retrackt_format::fingerprint::TrackFingerprint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct RaceResult {
    pub total_time: f32,
    pub best_lap: f32,
    pub splits: Vec<f32>,
    pub track_fingerprint: TrackFingerprint,
    pub replay: Option<ReplayTape>,
}

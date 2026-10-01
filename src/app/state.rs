use std::cell::RefCell;
use std::rc::Rc;

use retrackt_format::TrackDocument;

use crate::save::Settings;
use crate::session::result::RaceResult;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Title,
    Race,
    Results,
    Editor,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiAct {
    StartRace,
    Restart,
    QuitToTitle,
    RaceFinished(RaceResult),
    RaceAbandoned,
    OpenEditor,
    CloseEditor,
    SaveTrack,
    LoadTrack(String),
    Playtest,
    PlacePiece(retrackt_format::PieceId),
    RemoveLastPiece,
    ClearTrack,
}

pub type ActionQueue = Rc<RefCell<Vec<UiAct>>>;

pub fn push(q: &ActionQueue, act: UiAct) {
    q.borrow_mut().push(act);
    repose_core::request_frame();
}

#[derive(Clone, Debug, Default)]
pub struct AppData {
    pub screen: Screen,
    pub track: TrackDocument,
    pub settings: Settings,
    pub last_result: Option<RaceResult>,
    /// Best recorded time on the current track, if one exists.
    pub best: Option<f32>,
    /// Live race readout, refreshed by the runtime every frame for the HUD.
    pub race_time: f32,
    pub speed_kmh: f32,
    pub checkpoint: usize,
    pub checkpoint_count: usize,
    /// Last runtime message for the UI to surface (a rejected race start,
    /// a failed save or load). Cleared when a race starts or the track changes.
    pub notice: Option<String>,
    pub actions: ActionQueue,
}

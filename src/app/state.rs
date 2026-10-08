use std::cell::RefCell;
use std::rc::Rc;

use repose_core::ImageHandle;
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
    Ghosts,
}

/// Which copy of a track a menu row refers to. Built-ins and saved files share
/// one namespace by name, so a name alone cannot say which one was clicked —
/// and resolving by name alone quietly returned the built-in, leaving a saved
/// track of the same name listed but unreachable.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackRef {
    Builtin(String),
    Saved(String),
}

impl TrackRef {
    pub fn name(&self) -> &str {
        match self {
            TrackRef::Builtin(name) | TrackRef::Saved(name) => name,
        }
    }
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
    LoadTrack(TrackRef),
    Playtest,
    PlacePiece(retrackt_format::PieceId),
    RemoveLastPiece,
    ClearTrack,
    /// Delete the piece with this identity. Uid rather than index: placing or
    /// removing a piece renumbers indices, so a button built this frame would
    /// delete a different piece by the time it was pressed.
    DeletePiece(retrackt_format::PieceUid),
    /// Quarter-turn the piece with this identity.
    RotatePiece(retrackt_format::PieceUid),
    /// Nudge one tunable value on the piece with this identity.
    AdjustParam(retrackt_format::PieceUid, crate::app::ParamKind, i8),
    OpenGhosts,
    CloseGhosts,
    /// Race a kept tape by its library file stem.
    RaceGhost(String),
    /// Keep the tape of the run that just finished, under this name.
    SaveGhost(String),
    /// The ghost-name field's current text. Its own action so the draft survives
    /// leaving and re-entering the results screen.
    SetGhostDraft(String),
    /// Clear the last runtime message.
    DismissNotice,
    /// Discard the kept tape with this file stem, and its file.
    DeleteGhost(String),
}

pub type ActionQueue = Rc<RefCell<Vec<UiAct>>>;

/// On-screen touch stick geometry, in physical px with a top-left origin.
#[derive(Clone, Copy, Debug)]
pub struct StickView {
    pub anchor: (f32, f32),
    pub knob: (f32, f32),
}

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
    /// The kept-tape library, refreshed by the runtime whenever it changes so
    /// the ghost screen never reads storage while a view is being built.
    pub ghosts: Vec<crate::save::GhostEntry>,
    /// Name of the ghost the current race is running against, for the HUD.
    pub ghost_name: Option<String>,
    /// Race time of that ghost, in ticks, so the HUD can show what is being
    /// chased without decoding a tape per frame.
    pub ghost_ticks: Option<u32>,
    /// Where the ghost library was opened from, so closing returns there rather
    /// than always dropping to the title.
    pub ghost_return: Screen,
    /// Draft text for naming a new ghost.
    pub ghost_draft: String,
    /// Best recorded time on the current track under the current vehicle
    /// tuning, in simulated ticks.
    pub best: Option<u32>,
    /// Live race readout, refreshed by the runtime every frame for the HUD.
    pub race_ticks: u32,
    pub speed_kmh: f32,
    pub checkpoint: usize,
    pub checkpoint_count: usize,
    /// Last runtime message for the UI to surface (a rejected race start,
    /// a failed save or load). Cleared when a race starts or the track changes.
    pub notice: Option<String>,
    pub actions: ActionQueue,
    /// Set once the player first touches during a race.
    pub stick: Option<StickView>,
    /// Base and knob textures, uploaded on the frame the stick first shows.
    pub stick_images: Option<(ImageHandle, ImageHandle)>,
}

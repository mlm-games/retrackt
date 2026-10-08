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
    /// Race the current track, and come back to the editor afterwards rather
    /// than to the title.
    Playtest,
    /// Arm a palette piece without placing it, so the preview and cursor have
    /// something to show. Placing is a separate action: arming on every click
    /// would make it impossible to look at a piece before committing to it.
    ArmPiece(retrackt_format::PieceId),
    /// Drop the armed piece at the cursor, snapped to a connector if one is near.
    PlaceArmed,
    /// Move the cursor by one grid cell.
    MoveCursor([i16; 3]),
    /// Put the cursor at an absolute cell, from a click in the viewport.
    SetCursor([i16; 3]),
    /// Put the cursor's X and Z, leaving its height alone. A ground-plane click
    /// knows nothing about Y, and taking it from the click would drop a piece
    /// placed above the terrain.
    SetCursorXZ(i16, i16),
    /// Editor camera orbit, pan and zoom, in viewport px and a wheel factor.
    EditorOrbit(f32, f32),
    EditorPan(f32, f32),
    EditorZoom(f32),
    /// Replace the selection with this one piece.
    SelectPiece(retrackt_format::PieceUid),
    /// Add or remove one piece from the selection.
    ToggleSelect(retrackt_format::PieceUid),
    SelectAll,
    SelectNone,
    /// Select every piece on the connected chain.
    SelectRoute,
    /// Delete the selected pieces. Unlike the other bulk actions this does *not*
    /// fall back to every piece when nothing is selected: one tap on Delete after
    /// clearing a selection would otherwise wipe the track.
    DeleteSelection,
    /// Copy the selection to the editor's clipboard.
    CopySelection,
    /// Drop the clipboard at the cursor.
    PasteAtCursor,
    /// Copy the selection and immediately paste it at the cursor.
    DuplicateSelection,
    /// Move the selection so its first piece's anchor lands on the cursor.
    MoveSelectionToCursor,
    /// Turn every selected piece a quarter turn.
    RotateSelection(i8),
    Undo,
    Redo,
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
    /// Apply one parameter nudge to every selected piece.
    AdjustSelection(crate::app::ParamKind, i8),
    /// Encode the current track as a share code and put it on the clipboard.
    CopyShareCode,
    /// Decode the editor's code field into the current track.
    LoadShareCode,
    /// The share-code field's current text.
    SetCodeDraft(String),
    /// Turn an autosaved per-track thumbnail on or off.
    SetThumbnails(bool),
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
    pub editor: EditorData,
}

/// Everything the editor screen needs that is not the track itself.
///
/// Held on `AppData` rather than in the runtime so composing the screen never
/// reaches into mutable state, and so the values survive the runtime rebuilding
/// the world underneath them.
#[derive(Clone, Debug)]
pub struct EditorData {
    /// Grid cell the next placement lands on or near.
    pub cursor: [i16; 3],
    /// Palette piece armed for placement.
    pub armed: retrackt_format::PieceId,
    /// Selected pieces, by identity. Empty means "every piece" for the reversible
    /// bulk actions — turning a whole track's curves is a thing players want — but
    /// deletion treats empty as "nothing", so a cleared selection cannot wipe the
    /// track by accident.
    pub selection: Vec<retrackt_format::PieceUid>,
    /// Pieces copied by the last Copy, ready to drop anywhere. Kept after a paste
    /// so the same block can be dropped again.
    pub clipboard: Vec<retrackt_format::PieceInstance>,
    /// Whether the next placement would lock onto a connector port.
    pub snapped: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Problems with the track, recomputed when it changes.
    pub diagnostics: Vec<retrackt_format::Diagnostic>,
    /// Share-code field contents, so the draft survives leaving the editor.
    pub code_draft: String,
    /// The last code this track exported to, shown so it can be re-copied.
    pub code_out: String,
    /// Whether the library draws a schematic of each track beside its name.
    pub thumbnails: bool,
}

impl Default for EditorData {
    fn default() -> Self {
        Self {
            cursor: [0, 0, 0],
            armed: retrackt_format::PieceId::Straight,
            selection: Vec::new(),
            clipboard: Vec::new(),
            snapped: false,
            can_undo: false,
            can_redo: false,
            diagnostics: Vec::new(),
            code_draft: String::new(),
            code_out: String::new(),
            thumbnails: true,
        }
    }
}

impl EditorData {
    /// The pieces an action applies to: the selection, or all of them.
    pub fn targets<'a>(&self, doc: &'a TrackDocument) -> Vec<&'a retrackt_format::PieceInstance> {
        if self.selection.is_empty() {
            return doc.pieces.iter().collect();
        }
        doc.pieces
            .iter()
            .filter(|p| self.selection.contains(&p.uid))
            .collect()
    }

    pub fn is_selected(&self, uid: retrackt_format::PieceUid) -> bool {
        self.selection.contains(&uid)
    }
}

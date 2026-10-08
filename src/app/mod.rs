pub mod camera;
pub mod history;
pub mod input;
pub mod placement;
pub mod scene;
pub mod schedule;
pub mod state;
pub mod theme;

use repame_sim::Sim;
use repame_view3d::{BatchDesc, GeomHandle, MeshGroup, Viewport3d};
use repose_core::{RenderContext, Scheduler, View};
use retrackt_format::{
    PieceInstance, ReplayTape, TrackDocument, builtin_tracks, demo_track, gameplay_fingerprint,
};
use web_time::Instant;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::sim::car::Car;
use crate::sim::world::TrackWorld;
use history::{Edit, History, changed};
use placement::placement_at;
use schedule::{
    ActiveRes, CarRes, FinishedRes, GhostRes, InputRes, SessionRes, TapeRes, TrackRes,
};
use state::{Screen, TrackRef, UiAct};

/// How many cells away a placement will reach for an open connector port.
pub const SNAP_REACH: i32 = 1;

pub struct App {
    pub data: state::AppData,
    sim: Sim,
    input: input::InputState,
    cam: camera::ChaseCamera,
    save: crate::save::SaveData,
    ground_mesh: Option<MeshGroup>,
    track_mesh: Option<MeshGroup>,
    sky: scene::SkyDome,
    /// Car pose at the previous frame, blended with the current one by
    /// `Sim::alpha`. `None` after a teleport, where there is nothing to blend
    /// from. The simulation keeps no history of its own.
    prev_car: Option<Car>,
    /// Same history for the ghost car. Separate from `prev_car` because the two
    /// are only ever blended against themselves: pairing a ghost pose with a
    /// player pose would draw one car at an interpolated point between them.
    prev_ghost: Option<Car>,
    last: Instant,
    race_done: bool,
    /// The tape the current race is racing against, kept here so a restart can
    /// re-arm the same ghost. Held alongside the sim rather than in it: the
    /// simulation only ever sees the one ghost it is driving this run.
    ghost: Option<SelectedGhost>,
    /// Held for the process lifetime: `paint` publishes the real window size
    /// into it; a fresh handle every frame would report the 1600x900 default.
    geom: GeomHandle,
    /// Editor undo/redo. Not in `AppData`: it is mutable runtime state, and the
    /// view only ever needs to know whether a button is live.
    history: History,
    /// Editor camera. Separate from the chase camera so the framing the player
    /// chose survives a playtest and a return.
    editor_cam: repame_view3d::OrbitCamera,
    /// What the editor looked like when Playtest was pressed, so returning from
    /// a playtest restores the view instead of dropping the player at a default.
    editor_return: Option<EditorView>,
}

/// Editor state to come back to after a playtest.
struct EditorView {
    camera: repame_view3d::OrbitCamera,
    /// Selected pieces, by identity, so the piece being worked on is still
    /// selected on return.
    selection: Vec<retrackt_format::PieceUid>,
}

/// Which of a piece's tunable values an editor button drives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Length,
    Radius,
    Bank,
}

/// Quarter turns about +Y that make a full turn. A piece's `yaw` is a count of
/// these, so the modulus belongs with the step rather than at each use.
pub const YAW_STEPS: u8 = 4;

/// The tape this race is chasing, and the identity the library knows it by.
/// Cloned on every `start_race` so the selection outlives the tape the sim is
/// driving, which `arm` consumes.
#[derive(Clone)]
struct SelectedGhost {
    /// File stem in the ghost library. Kept so deleting that row can clear the
    /// selection instead of leaving a race against a tape that is gone.
    file: String,
    name: String,
    tape: ReplayTape,
}

/// Catch-up ticks one frame may run. Bounded because every one of them reuses
/// the single input sample taken at the frame, so a larger budget buys nothing
/// and delays the player's next input by longer.
const MAX_CATCHUP_TICKS: u32 = 8;

impl App {
    pub fn new() -> Self {
        let mut save = crate::save::boot();
        let physics = crate::sim::car::physics_fingerprint();
        // Records under other tuning can no longer be read as records; drop them
        // now rather than carrying them for the rest of the session.
        save.records.prune(&physics);
        // Tapes under other tuning cannot be replayed either, and a row whose
        // file has been deleted outside the game would fail to load every time it
        // was picked.
        let files = crate::save::stored_ghost_files();
        save.ghosts.prune(&physics, &|file| {
            files.iter().any(|f| f == file)
        });
        let mut sim = Sim::new(crate::SIM_STEP);
        sim.max_steps = MAX_CATCHUP_TICKS;
        schedule::register(&mut sim);
        schedule::insert_resources(&mut sim.world);
        let data = state::AppData {
            settings: save.settings.clone(),
            ghosts: save.ghosts.all().to_vec(),
            ghost_draft: "My ghost".into(),
            editor: state::EditorData {
                thumbnails: save.settings.thumbnails,
                ..state::EditorData::default()
            },
            ..state::AppData::default()
        };
        let mut app = Self {
            data,
            sim,
            input: input::InputState::new(),
            cam: camera::ChaseCamera::new(),
            save,
            ground_mesh: None,
            track_mesh: None,
            sky: scene::SkyDome::new(),
            prev_car: None,
            prev_ghost: None,
            last: Instant::now(),
            race_done: false,
            ghost: None,
            geom: GeomHandle::new(),
            history: History::new(),
            editor_cam: editor_camera(),
            editor_return: None,
        };
        app.set_track(demo_track());
        app
    }

    pub fn view(&mut self, sched: &mut Scheduler, ctx: &RenderContext) -> View {
        repose_core::request_frame();
        let now = Instant::now();
        let dt = now
            .duration_since(self.last)
            .min(MAX_CATCHUP_TICKS * crate::SIM_STEP);
        self.last = now;

        self.drain_actions();

        let car = self.sim.world.resource::<CarRes>().0;
        self.input.forward_speed = car.forward_speed();
        self.input.poll(sched, self.data.screen == Screen::Race);
        self.data.stick = self.input.stick_view();
        if self.data.stick.is_some() && self.data.stick_images.is_none() {
            self.data.stick_images = Some((
                ctx.image_from_encoded(
                    include_bytes!("../../assets/touch/joystick_outer.png").to_vec(),
                    true,
                ),
                ctx.image_from_encoded(
                    include_bytes!("../../assets/touch/joystick_inner.png").to_vec(),
                    true,
                ),
            ));
        }
        // Queued for every tick this frame could run, so a hitch that catches up
        // several ticks still gives each one its own entry to consume.
        let packed = self.input.packed();
        let catchup = self.sim.max_steps as usize;
        self.sim.world.resource_mut::<InputRes>().refill(packed, catchup);

        // Editor keys are read here rather than in the view, because the cursor
        // they move lives on `AppData` and the placement preview is recomputed
        // from it. Polled only while the editor is up: the same arrows drive the
        // car, and a piece would slide one cell every time the player braked.
        //
        // Released on the frame the editor closes, so a key still held at that
        // moment is not read as a fresh press — or as a held key that starts
        // repeating — when the editor is opened again.
        if self.data.screen == Screen::Editor {
            self.poll_editor_keys(sched);
        } else {
            self.input.release_editor();
        }

        // Stepped only once this frame's intent is queued, so the car reacts to
        // the input the player just gave rather than to the previous frame's.
        self.sim.step(dt);

        // Consumed before the keys below so a same-frame restart cannot overwrite
        // a finish; only `start_race`/`leave_run` reset `race_done`.
        let finished_now = self.sim.world.resource::<FinishedRes>().0 && !self.race_done;
        if finished_now {
            self.race_done = true;
            let mut tape = self.sim.world.resource_mut::<TapeRes>().0.take();
            let result = {
                let session = self.sim.world.resource::<SessionRes>();
                if let Some(tape) = tape.as_mut() {
                    // Verbatim: the run clock already counts ticks.
                    tape.split_ticks = session.0.splits().to_vec();
                }
                session.0.result(&self.data.track, tape)
            };
            // Freeze the car where it crossed the line: the results screen
            // shows the scene behind itself.
            self.sim.world.insert_resource(ActiveRes(false));
            state::push(&self.data.actions, UiAct::RaceFinished(result));
        }

        if !finished_now {
            let screen = self.data.screen;
            if self.input.restart && matches!(screen, Screen::Race | Screen::Results) {
                self.start_race();
            }
            if self.input.quit && matches!(screen, Screen::Race | Screen::Results | Screen::Editor)
            {
                self.leave_run();
            }
        }

        // Blend the tick just taken against the one before it: the sim runs at a
        // fixed rate that need not divide the display's, so drawing only the
        // newest tick holds one pose per tick and judders between them.
        let alpha = self.sim.alpha();
        let car = self.sim.world.resource::<CarRes>().0;
        let draw_car = if car.respawned {
            // A kill-height respawn teleports the car. Blending across that jump
            // draws the car sliding across the map for one frame, so the pose is
            // drawn as it landed and the history is dropped.
            self.sim.world.resource_mut::<CarRes>().0.respawned = false;
            self.prev_car = None;
            self.cam.snap(&car);
            car
        } else {
            Car::render_lerp(&self.prev_car.unwrap_or(car), &car, alpha)
        };
        self.prev_car = Some(car);

        let session = self.sim.world.resource::<SessionRes>();
        self.data.race_ticks = session.0.ticks();
        self.data.speed_kmh = car.speed_kmh();
        self.data.checkpoint = session.0.checkpoint();
        self.data.checkpoint_count = session.0.checkpoint_count();

        // Eased toward the current tick, not the drawn one: `to_orbit` applies
        // `alpha` itself, so blending here as well would put the view a frame
        // behind the car it is following.
        self.cam.update(dt.as_secs_f32(), &car);

        // The ghost is drawn from its own history, dropped whenever it teleports for
        // the same reason the player's is. Read through a block so the borrow
        // ends before the flag is cleared: `Res` drops at scope end, and a
        // `resource_mut` inside its lifetime would be a second overlapping
        // borrow of the world.
        let (ghost_armed, ghost_car) = {
            let ghost = self.sim.world.resource::<GhostRes>();
            (ghost.armed(), ghost.car)
        };
        let draw_ghost = if !ghost_armed {
            self.prev_ghost = None;
            None
        } else if ghost_car.respawned {
            self.sim.world.resource_mut::<GhostRes>().car.respawned = false;
            self.prev_ghost = None;
            Some(ghost_car)
        } else {
            let drawn = Car::render_lerp(
                &self.prev_ghost.unwrap_or(ghost_car),
                &ghost_car,
                alpha,
            );
            self.prev_ghost = Some(ghost_car);
            Some(drawn)
        };
        let frame = self.build_frame(&draw_car, draw_ghost.as_ref(), alpha);
        let viewport = self.viewport(frame);
        crate::ui::root_view(&self.data, &self.data.actions, viewport)
    }

    /// The 3D viewport, wired to the editor camera while the editor is up.
    ///
    /// The event callback cannot borrow `self`, so it pushes onto the action
    /// queue and the work happens at the top of the next frame. One frame of
    /// latency on a camera drag is invisible; borrowing across the view build
    /// would not compile.
    fn viewport(&mut self, frame: repame_view3d::Frame3d) -> View {
        let queue = self.data.actions.clone();
        let editing = self.data.screen == Screen::Editor;
        let cell_size = self.data.track.cell_size;
        Viewport3d(
            frame,
            self.geom.clone(),
            "scene.main",
            BatchDesc::default(),
            move |ev| {
                if !editing {
                    return;
                }
                match ev {
                    repame_view3d::View3dEvent::Orbit { dx, dy } => {
                        state::push(&queue, UiAct::EditorOrbit(dx, dy))
                    }
                    repame_view3d::View3dEvent::Pan { dx, dy } => {
                        state::push(&queue, UiAct::EditorPan(dx, dy))
                    }
                    repame_view3d::View3dEvent::Zoom { factor } => {
                        state::push(&queue, UiAct::EditorZoom(factor))
                    }
                    // The ground plane is where the cursor lives. A click sets
                    // X and Z only: Y belongs to the cursor, so clicking beside a
                    // ramp cannot teleport a piece up it.
                    repame_view3d::View3dEvent::GroundClick { x, z } => {
                        let cell = retrackt_format::geometry::to_grid(
                            glam::Vec3::new(x, 0.0, z),
                            cell_size,
                        );
                        state::push(&queue, UiAct::SetCursorXZ(cell[0], cell[2]))
                    }
                    _ => {}
                }
            },
        )
    }

    fn drain_actions(&mut self) {
        let actions: Vec<UiAct> = {
            let mut queue = self.data.actions.borrow_mut();
            std::mem::take(&mut *queue)
        };
        for act in actions {
            self.apply_action(act);
        }
    }

    fn apply_action(&mut self, act: UiAct) {
        match act {
            UiAct::StartRace | UiAct::Restart => {
                self.start_race();
            }
            // A playtest remembers where the editor was looking, so returning
            // from the results screen lands back on the piece being worked on
            // rather than at the spawn with nothing selected.
            UiAct::Playtest => {
                self.editor_return = Some(EditorView {
                    camera: self.editor_cam,
                    selection: self.data.editor.selection.clone(),
                });
                self.start_race();
            }
            UiAct::QuitToTitle | UiAct::RaceAbandoned => self.leave_run(),
            UiAct::OpenEditor => self.open_editor(),
            UiAct::CloseEditor => {
                self.editor_return = None;
                self.data.screen = Screen::Title;
                // Results -> Editor -> Title must not keep showing the
                // pre-race best the record badge needed.
                self.refresh_best();
            }
            UiAct::RaceFinished(result) => {
                let (track, physics) = (result.track_fingerprint, result.physics_fingerprint);
                if self.save.records.insert(&track, &physics, result.total_ticks)
                    && let Err(e) = crate::save::save(&self.save)
                {
                    self.data.notice = Some(e);
                }
                // `data.best` stays at the pre-race value — the results screen
                // shows it as "previous best"; it refreshes on the next race start.
                self.data.last_result = Some(result);
                self.data.screen = Screen::Results;
                self.sim.world.insert_resource(ActiveRes(false));
            }
            UiAct::SaveTrack => self.save_track(),
            UiAct::OpenGhosts => {
                // Read here rather than in the view: the library is storage, and a
                // screen must not touch it while it is being composed.
                self.refresh_ghosts();
                self.data.ghost_return = self.data.screen;
                self.data.screen = Screen::Ghosts;
            }
            // Returns to wherever the library was opened from, so reaching it from the
            // results screen does not throw the player past their own result.
            UiAct::CloseGhosts => self.data.screen = self.data.ghost_return,
            UiAct::RaceGhost(file) => self.race_ghost(&file),
            UiAct::SaveGhost(name) => self.save_ghost(&name),
            UiAct::SetGhostDraft(name) => self.data.ghost_draft = name,
            UiAct::DismissNotice => self.data.notice = None,
            UiAct::DeleteGhost(file) => self.delete_ghost(&file),
            UiAct::LoadTrack(track) => self.load_track(&track),

            UiAct::ArmPiece(id) => {
                self.data.editor.armed = id;
                self.data.editor.snapped =
                    placement_at(&self.data.track, id, self.data.editor.cursor, SNAP_REACH)
                        .snapped;
            }
            UiAct::PlaceArmed => self.place_armed(),
            UiAct::MoveCursor(delta) => {
                let cell = placement::step_cell(self.data.editor.cursor, delta);
                self.set_cursor(cell);
            }
            UiAct::SetCursor(cell) => self.set_cursor(cell),
            UiAct::SetCursorXZ(x, z) => {
                let cell = self.data.editor.cursor;
                self.set_cursor([x, cell[1], z]);
            }
            UiAct::EditorOrbit(dx, dy) => self.editor_cam.orbit(dx, dy),
            UiAct::EditorPan(dx, dy) => self.editor_cam.pan(dx, dy),
            UiAct::EditorZoom(factor) => self.editor_cam.zoom(factor),
            UiAct::SelectPiece(uid) => self.data.editor.selection = vec![uid],
            UiAct::ToggleSelect(uid) => {
                let selection = &mut self.data.editor.selection;
                match selection.iter().position(|u| *u == uid) {
                    Some(i) => {
                        selection.remove(i);
                    }
                    None => selection.push(uid),
                }
            }
            UiAct::SelectAll => {
                self.data.editor.selection =
                    self.data.track.pieces.iter().map(|p| p.uid).collect();
            }
            UiAct::SelectNone => self.data.editor.selection.clear(),
            UiAct::SelectRoute => {
                self.data.editor.selection = self.data.track.route();
            }
            UiAct::DeleteSelection => self.delete_selection(),
            UiAct::CopySelection => self.copy_selection(),
            UiAct::PasteAtCursor => self.paste_at_cursor(),
            UiAct::DuplicateSelection => {
                self.copy_selection();
                self.paste_at_cursor();
            }
            UiAct::MoveSelectionToCursor => self.move_selection_to_cursor(),
            UiAct::RotateSelection(steps) => self.rotate_selection(steps),
            UiAct::Undo => self.undo(),
            UiAct::Redo => self.redo(),
            UiAct::AdjustSelection(kind, delta) => self.adjust_selection(kind, delta),
            UiAct::CopyShareCode => self.copy_share_code(),
            UiAct::LoadShareCode => self.load_share_code(),
            UiAct::SetCodeDraft(code) => self.data.editor.code_draft = code,
            UiAct::SetThumbnails(on) => {
                self.data.editor.thumbnails = on;
                // A setting, not a view preference: it belongs in the save file so
                // it survives a restart.
                self.save.settings.thumbnails = on;
                let _ = crate::save::save(&self.save);
            }

            UiAct::RemoveLastPiece => {
                if self.data.track.pieces.pop().is_some() {
                    self.track_changed();
                }
            }
            UiAct::DeletePiece(uid) => self.delete_piece(uid),
            UiAct::RotatePiece(uid) => self.rotate_piece(uid),
            UiAct::AdjustParam(uid, kind, delta) => self.adjust_param(uid, kind, delta),
            UiAct::ClearTrack => self.clear_track(),
        }
    }

    /// Start (or restart) a run. Returns false when the track cannot be raced or
    /// has no drivable spawn; nothing is reset then, and `data.notice` explains
    /// the rejection.
    fn start_race(&mut self) -> bool {
        // Checked here as well as in the editor: a track can reach this point
        // having been edited into an unraceable state, and a run that starts and
        // can never be completed is worse than a refusal with a reason.
        let diagnostics = retrackt_format::validate(&self.data.track);
        if let Some(first) = diagnostics
            .iter()
            .find(|d| d.severity == retrackt_format::Severity::Error)
        {
            self.data.notice = Some(format!("Cannot race this track: {}", first.message));
            return false;
        }
        let world = self.sim.world.resource::<TrackRes>().0.clone();
        if !crate::sim::car::settles_at_spawn(&world) {
            self.data.notice =
                Some("No drivable spawn: place the Start piece flat on the road.".into());
            return false;
        }
        self.data.notice = None;
        let car = Car::at_spawn(world.spawn, world.spawn_yaw);
        {
            let mut session = self.sim.world.resource_mut::<SessionRes>();
            session.0.start(&world);
            self.data.checkpoint_count = session.0.checkpoint_count();
        }
        self.sim.world.insert_resource(CarRes(car));
        self.sim.world.insert_resource(ActiveRes(true));
        self.sim.world.insert_resource(FinishedRes(false));
        self.sim.world.insert_resource(TapeRes(Some(ReplayTape::new(
            gameplay_fingerprint(&self.data.track),
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        ))));
        // Re-armed on every start, so a restart races the same ghost from the
        // start line rather than resuming one that has already finished.
        self.arm_ghost(&world);
        self.race_done = false;
        self.data.screen = Screen::Race;
        self.data.race_ticks = 0;
        self.data.speed_kmh = 0.0;
        self.data.checkpoint = 0;
        self.refresh_best();
        self.prev_car = None;
        self.prev_ghost = None;
        self.cam.snap(&car);
        true
    }

    /// Put the selected ghost on the start line, or clear it.
    fn arm_ghost(&mut self, world: &TrackWorld) {
        self.data.ghost_name = None;
        self.data.ghost_ticks = None;
        let Some(selected) = self.ghost.clone() else {
            self.sim.world.resource_mut::<GhostRes>().disarm();
            return;
        };
        // The fingerprint of the document `world` was built from, not the tape's own
        // header: that is the whole point of the check. The finish tick is read
        // before the tape is handed over, because `arm` takes it by value.
        let track = gameplay_fingerprint(&self.data.track);
        let ticks = selected.tape.header.finish_tick;
        match self
            .sim
            .world
            .resource_mut::<GhostRes>()
            .arm(selected.tape, track, world)
        {
            Ok(()) => {
                self.data.ghost_name = Some(selected.name);
                self.data.ghost_ticks = Some(ticks);
            }
            Err(e) => {
                // Drop it rather than racing an unarmed ghost: the HUD would
                // claim a target that is not on the track.
                self.ghost = None;
                self.sim.world.resource_mut::<GhostRes>().disarm();
                self.data.notice = Some(e);
            }
        }
    }

    /// Start a race against the kept tape with this file stem.
    fn race_ghost(&mut self, file: &str) {
        let Some(tape) = crate::save::load_ghost(file) else {
            self.data.notice = Some(format!("Could not read ghost \"{file}\""));
            return;
        };
        let name = self
            .save
            .ghosts
            .get(file)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| file.to_string());
        // Checked here as well as in `arm`, so a ghost from another track never starts
        // a race at all: `arm` would refuse it after the fact, leaving the player
        // in an untimed run reading an error about a button they just pressed.
        if tape.header.track != gameplay_fingerprint(&self.data.track) {
            self.data.notice = Some("That ghost was recorded on a different track.".into());
            return;
        }
        self.ghost = Some(SelectedGhost {
            file: file.to_string(),
            name,
            tape,
        });
        if !self.start_race() {
            self.ghost = None;
        }
    }

    /// Keep the tape of the run that just finished, under `name`.
    fn save_ghost(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.data.notice = Some("Give the ghost a name first.".into());
            return;
        }
        // Borrowed, never taken: the results screen still reads `last_result`
        // after this, and it is the only place the finished run's time lives.
        let Some(tape) = self.data.last_result.as_ref().and_then(|r| r.replay.clone())
        else {
            self.data.notice = Some(if self.data.last_result.is_some() {
                "That run recorded no tape.".into()
            } else {
                "Finish a race before saving a ghost.".into()
            });
            return;
        };
        match crate::save::save_ghost(&tape, name) {
            Ok(entry) => {
                let entry = match self.save.ghosts.insert(entry) {
                    Ok(()) => entry,
                    Err(e) => {
                        // The tape is on disk with nothing pointing at it.
                        crate::save::delete_ghost(&entry.file);
                        self.data.notice = Some(e);
                        return;
                    }
                };
                let _ = crate::save::save(&self.save);
                self.data.notice = Some(format!("Saved ghost \"{}\".", entry.name));
                self.refresh_ghosts();
            }
            Err(e) => self.data.notice = Some(format!("Save failed: {e}")),
        }
    }

    fn delete_ghost(&mut self, file: &str) {
        crate::save::delete_ghost(file);
        self.save.ghosts.remove(file);
        let _ = crate::save::save(&self.save);
        // A tape deleted mid-race must stop being chased, or the car keeps
        // running a reference the player can no longer see in the library.
        if self.ghost.as_ref().is_some_and(|g| g.file == file) {
            self.ghost = None;
            let world = self.sim.world.resource::<TrackRes>().0.clone();
            self.arm_ghost(&world);
        }
        self.refresh_ghosts();
    }

    fn refresh_ghosts(&mut self) {
        self.data.ghosts = self.save.ghosts.all().to_vec();
    }

    fn leave_run(&mut self) {
        self.sim.world.insert_resource(ActiveRes(false));
        self.sim.world.insert_resource(FinishedRes(false));
        // Leaving a race drops its ghost: the tape stays in the library, but a
        // title screen must not leave the sim replaying a car nobody is racing.
        self.ghost = None;
        self.sim.world.resource_mut::<GhostRes>().disarm();
        self.data.ghost_name = None;
        self.data.ghost_ticks = None;
        self.prev_ghost = None;
        self.race_done = false;
        // A playtest owes the player a return to where they were editing;
        // anything else goes to the title.
        if self.editor_return.is_some() {
            self.open_editor();
            return;
        }
        self.data.screen = Screen::Title;
        // Leaving Results may strand `data.best` at the pre-race value after
        // a record run; the title screen shows it.
        self.refresh_best();
    }

    /// Replace the document wholesale, from a load or a share code.
    ///
    /// The history is cleared: every entry in it names a piece of the *old*
    /// document, and undoing one would splice a piece from a different track
    /// into this one.
    fn set_track(&mut self, track: TrackDocument) {
        self.data.track = track;
        self.history.clear();
        self.data.editor.selection.clear();
        self.data.editor.clipboard.clear();
        self.set_cursor(placement::cursor_home(&self.data.track));
        self.track_changed();
    }

    fn open_editor(&mut self) {
        self.sim.world.insert_resource(ActiveRes(false));
        if let Some(saved) = self.editor_return.take() {
            // Coming back from a playtest: restore the view the player left,
            // rather than dropping them at a default framing.
            self.editor_cam = saved.camera;
            self.data.editor.selection = saved.selection;
        }
        self.data.screen = Screen::Editor;
        self.data.editor.diagnostics = retrackt_format::validate(&self.data.track);
        self.data.editor.can_undo = self.history.can_undo();
        self.data.editor.can_redo = self.history.can_redo();
        self.set_cursor(self.data.editor.cursor);
    }

    fn track_changed(&mut self) {
        self.data.notice = None;
        self.data.track.normalize_uids();
        // A normalised uid is the identity every editor action keys on, so a
        // selection captured before this call may now name a different piece.
        self.data
            .editor
            .selection
            .retain(|uid| self.data.track.pieces.iter().any(|p| p.uid == *uid));
        self.data.editor.diagnostics = retrackt_format::validate(&self.data.track);
        self.data.editor.can_undo = self.history.can_undo();
        self.data.editor.can_redo = self.history.can_redo();
        let world = TrackWorld::from_doc(&self.data.track);
        let car = Car::at_spawn(world.spawn, world.spawn_yaw);
        {
            let mut session = self.sim.world.resource_mut::<SessionRes>();
            session.0.start(&world);
        }
        self.sim.world.insert_resource(TrackRes(world));
        self.sim.world.insert_resource(CarRes(car));
        self.track_mesh = None;
        self.ground_mesh = None;
        self.prev_car = None;
        self.prev_ghost = None;
        // The ghost was recorded against the old geometry, so it can no longer be
        // played. `start_race` below re-arms whatever survived that check.
        self.ghost = None;
        self.sim.world.resource_mut::<GhostRes>().disarm();
        self.cam.snap(&car);
        self.refresh_best();
        // A race on the old geometry restarts on the new one; anywhere else the
        // sim idles until asked to run. `start_race` re-arms the ghost, and the
        // selection was just cleared above, so an edited track races untimed.
        if self.data.screen != Screen::Race || !self.start_race() {
            self.sim.world.insert_resource(ActiveRes(false));
            self.sim.world.insert_resource(FinishedRes(false));
            self.race_done = false;
        }
    }

    fn refresh_best(&mut self) {
        let track = gameplay_fingerprint(&self.data.track);
        self.data.best = self.save.records.get(&track, &crate::sim::car::physics_fingerprint());
    }

    fn save_track(&mut self) {
        match crate::save::save_track(&self.data.track) {
            Ok(()) => {
                self.data.notice = None;
                // The library screen caches the stored documents, and a track just
                // written is not in that cache.
                crate::ui::library::invalidate();
            }
            Err(e) => self.data.notice = Some(format!("Save failed: {e}")),
        }
    }

    fn load_track(&mut self, track: &TrackRef) {
        let name = track.name();
        let loaded = match track {
            TrackRef::Builtin(_) => builtin_tracks()
                .into_iter()
                .find(|d| d.name == name)
                .ok_or_else(|| format!("No built-in track named \"{name}\"")),
            TrackRef::Saved(_) => crate::save::load_track(name)
                .ok_or_else(|| format!("Could not load track \"{name}\"")),
        };
        match loaded {
            Some(doc) => self.set_track(doc),
            Err(e) => self.data.notice = Some(e),
        }
    }

    /// Turn one frame of editor keys into actions on the queue.
    ///
    /// Pushing rather than applying directly, because the cursor step and the
    /// commands are read from a single snapshot: applying the step first would
    /// recompute the placement preview before the command was seen.
    fn poll_editor_keys(&mut self, sched: &repose_core::runtime::Scheduler) {
        let frame = self.input.poll_editor(sched);
        let q = &self.data.actions;
        if frame.step != [0, 0, 0] {
            state::push(q, UiAct::MoveCursor(frame.step));
        }
        let pressed = frame.pressed;
        for act in [
            pressed.place.then_some(UiAct::PlaceArmed),
            pressed.delete.then_some(UiAct::DeleteSelection),
            pressed.rotate.then_some(UiAct::RotateSelection(1)),
            pressed.duplicate.then_some(UiAct::DuplicateSelection),
            pressed.copy.then_some(UiAct::CopySelection),
            pressed.paste.then_some(UiAct::PasteAtCursor),
            pressed.undo.then_some(UiAct::Undo),
            pressed.redo.then_some(UiAct::Redo),
            pressed.select_all.then_some(UiAct::SelectAll),
            pressed.select_route.then_some(UiAct::SelectRoute),
            pressed.select_none.then_some(UiAct::SelectNone),
        ]
        .into_iter()
        .flatten()
        {
            state::push(q, act);
        }
    }

    /// Move the editor cursor and refresh what it would drop.
    fn set_cursor(&mut self, cell: [i16; 3]) {
        self.data.editor.cursor = cell;
        self.data.editor.snapped =
            placement_at(&self.data.track, self.data.editor.armed, cell, SNAP_REACH).snapped;
    }

    /// Drop the armed piece at the cursor. Selected afterwards, so the parameter
    /// and rotate controls act on what was just placed.
    fn place_armed(&mut self) {
        let id = self.data.editor.armed;
        if self.data.track.pieces.len() >= retrackt_format::MAX_PIECES {
            self.data.notice = Some(format!(
                "This track already has the maximum of {} pieces.",
                retrackt_format::MAX_PIECES
            ));
            return;
        }
        let place = placement_at(&self.data.track, id, self.data.editor.cursor, SNAP_REACH);
        let uid = self.data.track.next_piece_uid();
        let piece = PieceInstance::new(id, place.anchor)
            .with_uid(uid)
            .with_yaw(place.yaw);
        let at = self.data.track.pieces.len();
        self.data.track.pieces.push(piece);
        self.history.push(Edit::Added { at, piece });
        self.data.editor.selection = vec![uid];
        // Following the cursor keeps a run of placements going without a click
        // per piece, which is what chaining onto the last exit used to do.
        self.set_cursor(placement::exit_cell(&self.data.track, &piece));
        self.track_changed();
    }

    fn undo(&mut self) {
        let Some(edit) = self.history.undo(&mut self.data.track) else {
            self.data.notice = Some("Nothing to undo.".into());
            return;
        };
        self.after_history(&edit, "Undid");
    }

    fn redo(&mut self) {
        let Some(edit) = self.history.redo(&mut self.data.track) else {
            self.data.notice = Some("Nothing to redo.".into());
            return;
        };
        self.after_history(&edit, "Redid");
    }

    /// Shared tail of undo and redo: the document moved, so everything derived
    /// from it has to be rebuilt.
    ///
    /// The selection is pruned rather than translated. A selection naming a
    /// piece that is not there would fall back to acting on *every* piece, which
    /// is the one way undo could destroy work.
    fn after_history(&mut self, edit: &Edit, verb: &str) {
        self.data
            .editor
            .selection
            .retain(|uid| self.data.track.pieces.iter().any(|p| p.uid == *uid));
        self.set_cursor(self.data.editor.cursor);
        self.track_changed();
        self.data.notice = Some(describe_edit(edit, verb));
    }

    /// The pieces a bulk action applies to: the selection, or all of them.
    ///
    /// "All" is the right default for the reversible actions — turning a whole
    /// track's curves is a thing players want — but never for deletion, which
    /// uses [`Self::selected`] instead.
    fn targets(&self) -> Vec<PieceInstance> {
        self.data
            .editor
            .targets(&self.data.track)
            .into_iter()
            .copied()
            .collect()
    }

    /// Just the selection, empty or not.
    ///
    /// Deletion reads this rather than `targets`. An empty selection falling back
    /// to "every piece" would mean one tap on Delete with nothing selected wipes
    /// the track, and the tap before it — the one that cleared the selection — is
    /// exactly what a player does after deciding they did not mean to select
    /// anything.
    fn selected(&self) -> Vec<PieceInstance> {
        self.data
            .track
            .pieces
            .iter()
            .filter(|p| self.data.editor.is_selected(p.uid))
            .copied()
            .collect()
    }

    fn delete_selection(&mut self) {
        let targets = self.selected();
        if targets.is_empty() {
            self.data.notice = Some("Nothing selected. Use Select Route or Select All.".into());
            return;
        }
        self.delete_pieces(targets.into_iter().map(|p| p.uid).collect());
    }

    fn delete_pieces(&mut self, uids: Vec<retrackt_format::PieceUid>) {
        let mut removed = 0;
        // Highest index first: removing one piece shifts everything after it
        // down, so walking downwards keeps each recorded index valid.
        let mut targets: Vec<(usize, PieceInstance)> = self
            .data
            .track
            .pieces
            .iter()
            .enumerate()
            .filter(|(_, p)| uids.contains(&p.uid))
            .map(|(i, p)| (i, *p))
            .collect();
        targets.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
        for (at, piece) in targets {
            self.data.track.pieces.remove(at);
            self.history.push(Edit::Removed { at, piece });
            removed += 1;
        }
        if removed == 0 {
            return;
        }
        self.data.editor.selection.clear();
        self.set_cursor(self.data.editor.cursor);
        self.track_changed();
        self.data.notice = Some(if removed == 1 {
            "Removed 1 piece. Undo puts it back.".into()
        } else {
            format!("Removed {removed} pieces. Undo puts them back.")
        });
    }

    fn clear_track(&mut self) {
        if self.data.track.pieces.is_empty() {
            return;
        }
        let before = self.data.track.pieces.clone();
        self.data.track.pieces.clear();
        self.history.push(Edit::Replaced {
            before,
            after: Vec::new(),
        });
        self.data.editor.selection.clear();
        self.set_cursor(self.data.editor.cursor);
        self.track_changed();
        self.data.notice = Some("Cleared the track. Undo brings it back.".into());
    }

    fn copy_selection(&mut self) {
        let clipboard = self.targets();
        if clipboard.is_empty() {
            return;
        }
        self.data.editor.clipboard = clipboard;
        self.data.notice = Some(format!(
            "Copied {} piece{}. Paste drops them at the cursor.",
            self.data.editor.clipboard.len(),
            if self.data.editor.clipboard.len() == 1 { "" } else { "s" }
        ));
    }

    /// Drop the clipboard so its first piece's entry port lands on the cursor.
    ///
    /// The whole block moves as one, so a copied run keeps its internal joins
    /// instead of arriving as a row of disconnected pieces.
    fn paste_at_cursor(&mut self) {
        let clipboard = std::mem::take(&mut self.data.editor.clipboard);
        if clipboard.is_empty() {
            self.data.notice = Some("Nothing copied yet.".into());
            return;
        }
        if self.data.track.pieces.len() + clipboard.len() > retrackt_format::MAX_PIECES {
            self.data.editor.clipboard = clipboard;
            self.data.notice =
                Some(format!("That would exceed the {} piece limit.", retrackt_format::MAX_PIECES));
            return;
        }
        let origin = clipboard
            .first()
            .map(|p| p.cell)
            .unwrap_or(self.data.editor.cursor);
        let cursor = self.data.editor.cursor;
        let mut placed = Vec::with_capacity(clipboard.len());
        for piece in &clipboard {
            let uid = self.data.track.next_piece_uid();
            let moved = PieceInstance {
                uid,
                // Widened before subtracting: two cells at opposite ends of the
                // grid overflow `i16`, and a debug build panics on that.
                cell: std::array::from_fn(|a| {
                    ((i32::from(piece.cell[a]) - i32::from(origin[a]) + i32::from(cursor[a]))
                        .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
                        as i16)
                }),
                ..*piece
            };
            placed.push(moved);
        }
        let before = self.data.track.pieces.clone();
        self.data.track.pieces.extend(placed.iter().copied());
        self.history.push(Edit::Replaced {
            before,
            after: self.data.track.pieces.clone(),
        });
        self.data.editor.selection = placed.iter().map(|p| p.uid).collect();
        // Restored, not consumed: the whole point of a clipboard is dropping the
        // same block twice, and the notice below says so.
        self.data.editor.clipboard = clipboard;
        self.set_cursor(cursor);
        self.track_changed();
        self.data.notice = Some(format!(
            "Pasted {} piece{}. The clipboard is kept, so it can be dropped again.",
            placed.len(),
            if placed.len() == 1 { "" } else { "s" }
        ));
    }

    fn move_selection_to_cursor(&mut self) {
        let targets = self.targets();
        let Some(first) = targets.first().copied() else {
            return;
        };
        let cursor = self.data.editor.cursor;
        // Widened for the same reason as the paste offset: a cursor at one end of
        // the grid and a piece at the other overflows `i16`.
        let delta: [i16; 3] = std::array::from_fn(|a| {
            (i32::from(cursor[a]) - i32::from(first.cell[a]))
                .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        });
        if delta == [0, 0, 0] {
            return;
        }
        self.move_pieces(&targets, delta);
        self.data.notice = Some(format!(
            "Moved {} piece{}. The road either side may no longer line up.",
            targets.len(),
            if targets.len() == 1 { "" } else { "s" }
        ));
    }

    /// Translate a set of pieces, as one undoable step.
    fn move_pieces(&mut self, pieces: &[PieceInstance], delta: [i16; 3]) {
        let mut count = 0;
        for piece in pieces {
            let Some(slot) = self.data.track.pieces.iter_mut().find(|p| p.uid == piece.uid)
            else {
                continue;
            };
            let before = *slot;
            let moved = placement::step_cell(before.cell, delta);
            if let Some(edit) = changed(before.uid, before, PieceInstance { cell: moved, ..before })
            {
                *slot = PieceInstance { cell: moved, ..before };
                self.history.push(edit);
                count += 1;
            }
        }
        if count > 0 {
            self.set_cursor(self.data.editor.cursor);
            self.track_changed();
        }
    }

    fn rotate_selection(&mut self, steps: i8) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let count = self.rotate_pieces(&targets, steps);
        if count > 0 {
            self.data.notice = Some(format!(
                "Turned {count} piece{}. Pieces after them may no longer be connected.",
                if count == 1 { "" } else { "s" }
            ));
        }
    }

    /// Quarter-turn a set of pieces, as one undoable step per piece.
    ///
    /// The ports move with the piece, so anything it was joined to stops lining
    /// up. That is surfaced through `notice` rather than repaired: re-joining
    /// would silently rewrite geometry the player placed on purpose.
    fn rotate_pieces(&mut self, pieces: &[PieceInstance], steps: i8) -> usize {
        let mut count = 0;
        for piece in pieces {
            let Some(slot) = self.data.track.pieces.iter_mut().find(|p| p.uid == piece.uid)
            else {
                continue;
            };
            let before = *slot;
            // Modulo on a signed step, so a negative count turns the other way
            // instead of wrapping to 255 quarter turns.
            let yaw = (before.yaw as i16 + i16::from(steps)).rem_euclid(YAW_STEPS as i16) as u8;
            if let Some(edit) = changed(before.uid, before, PieceInstance { yaw, ..before }) {
                *slot = PieceInstance { yaw, ..before };
                self.history.push(edit);
                count += 1;
            }
        }
        if count > 0 {
            self.set_cursor(self.data.editor.cursor);
            self.track_changed();
        }
        count
    }

    fn copy_share_code(&mut self) {
        match retrackt_format::export_code(&self.data.track) {
            Ok(code) => {
                self.data.editor.code_out = code.clone();
                // Best effort: the code is also shown on screen, so a platform
                // with no clipboard still leaves the player able to read it out.
                repose_core::clipboard::copy_to_clipboard(&code);
                self.data.notice =
                    Some("Share code copied to the clipboard, and shown below.".into());
            }
            Err(e) => self.data.notice = Some(format!("Could not encode this track: {e}")),
        }
    }

    fn load_share_code(&mut self) {
        let code = self.data.editor.code_draft.trim().to_string();
        if code.is_empty() {
            self.data.notice = Some("Paste a share code first.".into());
            return;
        }
        match retrackt_format::import_code(&code) {
            Ok(doc) => self.set_track(doc),
            Err(e) => self.data.notice = Some(format!("Could not read that code: {e}")),
        }
    }

    /// Delete one piece by identity. `uid` rather than an index: placing or
    /// removing a piece renumbers indices, so an index captured when a button was
    /// built no longer names the piece the player clicked.
    fn delete_piece(&mut self, uid: retrackt_format::PieceUid) {
        self.delete_pieces(vec![uid]);
    }

    /// Quarter-turn one piece in place. See [`Self::rotate_pieces`] for why the
    /// chain is not repaired.
    fn rotate_piece(&mut self, uid: retrackt_format::PieceUid) {
        let Some(piece) = self.data.track.piece(uid).copied() else {
            return;
        };
        self.rotate_pieces(&[piece], 1);
    }

    /// Length, radius or bank of one piece.
    ///
    /// Writes an explicit value rather than leaving the field `None`: `None`
    /// means "use the catalogue default", so a single nudge away from the
    /// default would be silently discarded the next time the two coincided.
    /// Length and radius are clamped to the same ranges `resolve_params`
    /// applies, so what the editor shows is what the shape builder will use.
    fn adjust_param(&mut self, uid: retrackt_format::PieceUid, kind: ParamKind, delta: i8) {
        self.adjust_pieces(&[uid], kind, delta);
    }

    fn adjust_selection(&mut self, kind: ParamKind, delta: i8) {
        let uids: Vec<_> = self.targets().into_iter().map(|p| p.uid).collect();
        self.adjust_pieces(&uids, kind, delta);
    }

    fn adjust_pieces(
        &mut self,
        uids: &[retrackt_format::PieceUid],
        kind: ParamKind,
        delta: i8,
    ) {
        let mut count = 0;
        for uid in uids {
            let Some(existing) = self.data.track.piece(*uid).copied() else {
                continue;
            };
            let (len, rad, bank) =
                retrackt_format::piece::resolve_params(existing.id, &existing.params);
            let mut params = existing.params;
            match kind {
                ParamKind::Length => {
                    params.length_cells = Some((len as i16 + i16::from(delta)).clamp(1, 16) as u8);
                }
                ParamKind::Radius => {
                    // Straight pieces have no radius; writing one would be ignored
                    // by the shape builder, so the edit is refused rather than
                    // silently dropped.
                    if rad == 0 {
                        if uids.len() == 1 {
                            self.data.notice =
                                Some(format!("{} has no radius.", existing.id.label()));
                        }
                        continue;
                    }
                    params.radius_cells = Some((rad as i16 + i16::from(delta)).clamp(1, 8) as u8);
                }
                ParamKind::Bank => {
                    params.bank_deg = Some((bank as i16 + i16::from(delta)).clamp(-45, 45) as i8);
                }
            }
            let after = PieceInstance { params, ..existing };
            if let Some(edit) = changed(existing.uid, existing, after) {
                if let Some(slot) = self.data.track.pieces.iter_mut().find(|p| p.uid == *uid) {
                    *slot = after;
                }
                self.history.push(edit);
                count += 1;
            }
        }
        if count > 0 {
            self.set_cursor(self.data.editor.cursor);
            self.track_changed();
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// A one-line account of an edit, for the notice bar.
fn describe_edit(edit: &Edit, verb: &str) -> String {
    match edit {
        Edit::Added { piece, .. } => format!("{verb} placing {}.", piece.id.label()),
        Edit::Removed { piece, .. } => format!("{verb} removing {}.", piece.id.label()),
        Edit::Changed { before, after } if before.id != after.id => {
            format!("{verb} changing {} to {}.", before.id.label(), after.id.label())
        }
        Edit::Changed { .. } => format!("{verb} editing a piece."),
        Edit::Replaced { before, after } => format!(
            "{verb} a change from {} to {} pieces.",
            before.len(),
            after.len()
        ),
        Edit::Renamed { before, after } => format!("{verb} renaming {before} to {after}."),
    }
}

/// The editor's opening view: framed on the whole track, looking down at it.
fn editor_camera() -> repame_view3d::OrbitCamera {
    repame_view3d::OrbitCamera {
        target: glam::Vec3::ZERO,
        yaw: -0.7,
        pitch: 0.85,
        dist: 110.0,
        fov_y_deg: 45.0,
    }
}

#[cfg(not(target_os = "android"))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    let mut app = App::new();
    #[cfg(not(target_arch = "wasm32"))]
    repame_shell::run_desktop("Retrackt", (1280, 720), move |sched, ctx| {
        app.view(sched, ctx)
    })
    .expect("retrackt failed to start");
    #[cfg(target_arch = "wasm32")]
    {
        let _ = repame_shell::run_web(move |sched, ctx| app.view(sched, ctx));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    if let Some(dir) = android_app.internal_data_path() {
        game_utils::set_android_data_dir(dir.join("files"));
    }
    let mut app = App::new();
    let _ = repame_shell::run_android(android_app, move |sched, ctx| app.view(sched, ctx));
}

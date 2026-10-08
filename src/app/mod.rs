pub mod camera;
pub mod input;
pub mod scene;
pub mod schedule;
pub mod state;
pub mod theme;

use repame_sim::Sim;
use repame_view3d::{BatchDesc, GeomHandle, MeshGroup, Viewport3d};
use repose_core::{RenderContext, Scheduler, View};
use retrackt_format::piece::{GridDir, rotate_local_dir};
use retrackt_format::{
    PieceId, PieceInstance, PieceParams, ReplayTape, TrackDocument, builtin_tracks, demo_track,
    gameplay_fingerprint, piece_shape, rotate_local_xz,
};
use web_time::Instant;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::sim::car::Car;
use crate::sim::world::TrackWorld;
use schedule::{
    ActiveRes, CarRes, FinishedRes, GhostRes, InputRes, SessionRes, TapeRes, TrackRes,
};
use state::{Screen, TrackRef, UiAct};

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
        let viewport = Viewport3d(
            frame,
            self.geom.clone(),
            "scene.main",
            BatchDesc::default(),
            |_ev| {},
        );
        crate::ui::root_view(&self.data, &self.data.actions, viewport)
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
            UiAct::StartRace | UiAct::Restart | UiAct::Playtest => {
                self.start_race();
            }
            UiAct::QuitToTitle | UiAct::RaceAbandoned => self.leave_run(),
            UiAct::OpenEditor => {
                self.sim.world.insert_resource(ActiveRes(false));
                self.data.screen = Screen::Editor;
            }
            // The editor's "Close" button must leave the editor; the literal
            // spec mapping (Editor) was a no-op.
            UiAct::CloseEditor => {
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
            UiAct::PlacePiece(id) => self.place_piece(id),
            UiAct::RemoveLastPiece => {
                if self.data.track.pieces.pop().is_some() {
                    self.track_changed();
                }
            }
            UiAct::DeletePiece(uid) => self.delete_piece(uid),
            UiAct::RotatePiece(uid) => self.rotate_piece(uid),
            UiAct::AdjustParam(uid, kind, delta) => self.adjust_param(uid, kind, delta),
            UiAct::ClearTrack => {
                if !self.data.track.pieces.is_empty() {
                    self.data.track.pieces.clear();
                    self.track_changed();
                }
            }
        }
    }

    /// Start (or restart) a run. Returns false when the track has no drivable
    /// spawn; nothing is reset then, and `data.notice` explains the rejection.
    fn start_race(&mut self) -> bool {
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
        self.data.screen = Screen::Title;
        // Leaving Results may strand `data.best` at the pre-race value after
        // a record run; the title screen shows it.
        self.refresh_best();
    }

    fn set_track(&mut self, track: TrackDocument) {
        self.data.track = track;
        self.track_changed();
    }

    fn track_changed(&mut self) {
        self.data.notice = None;
        self.data.track.normalize_uids();
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
            Ok(()) => self.data.notice = None,
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

    /// Where the editor's cursor sits: the end of the chain that has ports, or
    /// the origin for an empty document. Edits are anchored to it so the editor
    /// has one place to reason about instead of a cursor it can drift from.
    fn cursor(&self) -> [i16; 3] {
        let Some(prev) = self
            .data
            .track
            .pieces
            .iter()
            .rev()
            .find(|p| !piece_shape(p.id, &p.params, self.data.track.cell_size).ports.is_empty())
        else {
            return [0, 0, 0];
        };
        retrackt_format::demo::exit_cell(&self.data.track, prev)
    }

    fn place_piece(&mut self, id: PieceId) {
        let (anchor, yaw) = self.next_anchor(id);
        let uid = self.data.track.next_piece_uid();
        self.data
            .track
            .pieces
            .push(PieceInstance::new(id, anchor).with_uid(uid).with_yaw(yaw));
        self.track_changed();
    }

    /// Delete one piece by identity. `uid` rather than an index: placing or
    /// removing a piece renumbers indices, so an index captured when a button was
    /// built no longer names the piece the player clicked.
    fn delete_piece(&mut self, uid: retrackt_format::PieceUid) {
        let Some(index) = self.data.track.pieces.iter().position(|p| p.uid == uid) else {
            return;
        };
        let removed = self.data.track.pieces.remove(index);
        self.track_changed();
        self.data.notice = Some(format!("Removed {}", removed.id.label()));
    }

    /// Quarter-turn one piece in place. The ports move with it, so anything the
    /// piece was joined to stops lining up — `track_changed` rebuilds the world
    /// and `notice` says so, because a silently broken chain reads as the rotate
    /// button not working.
    fn rotate_piece(&mut self, uid: retrackt_format::PieceUid) {
        let Some(piece) = self.data.track.pieces.iter_mut().find(|p| p.uid == uid) else {
            return;
        };
        piece.yaw = (piece.yaw + 1) % YAW_STEPS;
        let label = piece.id.label();
        self.track_changed();
        self.data.notice = Some(format!(
            "Rotated {label}. Pieces after it may no longer be connected."
        ));
    }

    /// Length, radius or bank of one piece.
    ///
    /// Writes an explicit value rather than leaving the field `None`: `None`
    /// means "use the catalogue default", so a single nudge away from the
    /// default would be silently discarded the next time the two coincided.
    /// Length and radius are clamped to the same ranges `resolve_params`
    /// applies, so what the editor shows is what the shape builder will use.
    fn adjust_param(&mut self, uid: retrackt_format::PieceUid, kind: ParamKind, delta: i8) {
        let Some(piece) = self.data.track.pieces.iter().find(|p| p.uid == uid) else {
            return;
        };
        let (len, rad, bank) =
            retrackt_format::piece::resolve_params(piece.id, &piece.params);
        let Some(piece) = self.data.track.pieces.iter_mut().find(|p| p.uid == uid) else {
            return;
        };
        match kind {
            ParamKind::Length => {
                piece.params.length_cells = Some((len as i16 + delta as i16).clamp(1, 16) as u8);
            }
            ParamKind::Radius => {
                // Straight pieces have no radius; writing one would be ignored by
                // the shape builder, so the edit is refused rather than silently
                // dropped.
                if rad == 0 {
                    self.data.notice =
                        Some(format!("{} has no radius.", piece.id.label()));
                    return;
                }
                piece.params.radius_cells = Some((rad as i16 + delta as i16).clamp(1, 8) as u8);
            }
            ParamKind::Bank => {
                piece.params.bank_deg = Some((bank as i16 + delta as i16).clamp(-45, 45) as i8);
            }
        }
        self.track_changed();
    }

    /// Anchor for the next piece: its entry port lands on the open exit of
    /// the last piece that has ports, facing the way that piece left.
    fn next_anchor(&self, id: PieceId) -> ([i16; 3], u8) {
        let cell_size = self.data.track.cell_size;
        let mut cursor = [0i16; 3];
        let mut yaw = 0u8;
        if let Some(prev) = self
            .data
            .track
            .pieces
            .iter()
            .rev()
            .find(|p| !piece_shape(p.id, &p.params, cell_size).ports.is_empty())
        {
            let shape = piece_shape(prev.id, &prev.params, cell_size);
            let exit = shape.ports.last().expect("ports are non-empty");
            yaw = yaw_for_dir(rotate_local_dir(exit.outward, prev.yaw));
            cursor = retrackt_format::demo::exit_cell(&self.data.track, prev);
        }
        let entry = piece_shape(id, &PieceParams::default(), cell_size)
            .ports
            .first()
            .map(|p| p.cell)
            .unwrap_or([0, 0, 0]);
        let offset = rotate_local_xz(entry, yaw);
        (
            [
                cursor[0] - offset[0],
                cursor[1] - offset[1],
                cursor[2] - offset[2],
            ],
            yaw,
        )
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn yaw_for_dir(dir: GridDir) -> u8 {
    match dir {
        GridDir::PosZ => 0,
        GridDir::PosX => 1,
        GridDir::NegZ => 2,
        GridDir::NegX => 3,
        _ => 0,
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

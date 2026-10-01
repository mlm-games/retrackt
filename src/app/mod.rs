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
use schedule::{ActiveRes, CarRes, FinishedRes, InputRes, SessionRes, TapeRes, TrackRes};
use state::{Screen, UiAct};

pub struct App {
    pub data: state::AppData,
    sim: Sim,
    input: input::InputState,
    cam: camera::ChaseCamera,
    save: crate::save::SaveData,
    ground_mesh: Option<MeshGroup>,
    track_mesh: Option<MeshGroup>,
    last: Instant,
    race_done: bool,
    /// Held for the process lifetime: `paint` publishes the real window size
    /// into it; a fresh handle every frame would report the 1600x900 default.
    geom: GeomHandle,
}

impl App {
    pub fn new() -> Self {
        let save = crate::save::boot();
        let mut sim = Sim::new(crate::SIM_STEP);
        schedule::register(&mut sim);
        schedule::insert_resources(&mut sim.world);
        let data = state::AppData {
            settings: save.settings.clone(),
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
            last: Instant::now(),
            race_done: false,
            geom: GeomHandle::new(),
        };
        app.set_track(demo_track());
        app
    }

    pub fn view(&mut self, sched: &mut Scheduler, _ctx: &RenderContext) -> View {
        repose_core::request_frame();
        let now = Instant::now();
        let dt = now
            .duration_since(self.last)
            .min(web_time::Duration::from_secs_f32(0.25));
        self.last = now;

        self.sim.step(dt);

        self.drain_actions();

        let car = self.sim.world.resource::<CarRes>().0;
        self.input.forward_speed = car.forward_speed();
        self.input.poll(sched);
        self.sim
            .world
            .insert_resource(InputRes(self.input.vehicle_input()));

        // Consumed before the keys below so a same-frame restart cannot overwrite
        // a finish; only `start_race`/`leave_run` reset `race_done`.
        let finished_now = self.sim.world.resource::<FinishedRes>().0 && !self.race_done;
        if finished_now {
            self.race_done = true;
            let mut tape = self.sim.world.resource_mut::<TapeRes>().0.take();
            let result = {
                let session = self.sim.world.resource::<SessionRes>();
                if let Some(tape) = tape.as_mut() {
                    tape.split_ticks = session
                        .0
                        .splits()
                        .iter()
                        .map(|t| (t * crate::SIM_HZ as f32).round() as u32)
                        .collect();
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

        let car = self.sim.world.resource::<CarRes>().0;
        let session = self.sim.world.resource::<SessionRes>();
        self.data.race_time = session.0.time();
        self.data.speed_kmh = car.speed_kmh();
        self.data.checkpoint = session.0.checkpoint();
        self.data.checkpoint_count = session.0.checkpoint_count();

        // A kill-height respawn teleports the car; easing the camera across
        // that jump reads as a glitch, so it snaps instead.
        if car.respawned {
            self.sim.world.resource_mut::<CarRes>().0.respawned = false;
            self.cam.snap(&car);
        }
        self.cam.update(dt.as_secs_f32(), &car);
        let frame = self.build_frame();
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
                let fingerprint = result.track_fingerprint;
                if self.save.best_times.insert(&fingerprint, result.total_time)
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
            UiAct::LoadTrack(name) => self.load_track(&name),
            UiAct::PlacePiece(id) => self.place_piece(id),
            UiAct::RemoveLastPiece => {
                if self.data.track.pieces.pop().is_some() {
                    self.track_changed();
                }
            }
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
        self.race_done = false;
        self.data.screen = Screen::Race;
        self.data.race_time = 0.0;
        self.data.speed_kmh = 0.0;
        self.data.checkpoint = 0;
        self.refresh_best();
        self.cam.snap(&car);
        true
    }

    fn leave_run(&mut self) {
        self.sim.world.insert_resource(ActiveRes(false));
        self.sim.world.insert_resource(FinishedRes(false));
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
        self.cam.snap(&car);
        self.refresh_best();
        // A race on the old geometry restarts on the new one; anywhere else
        // the sim idles until asked to run.
        if self.data.screen != Screen::Race || !self.start_race() {
            self.sim.world.insert_resource(ActiveRes(false));
            self.sim.world.insert_resource(FinishedRes(false));
            self.race_done = false;
        }
    }

    fn refresh_best(&mut self) {
        let fingerprint = gameplay_fingerprint(&self.data.track);
        self.data.best = self.save.best_times.get(&fingerprint);
    }

    fn save_track(&mut self) {
        match crate::save::save_track(&self.data.track) {
            Ok(()) => self.data.notice = None,
            Err(e) => self.data.notice = Some(format!("Save failed: {e}")),
        }
    }

    fn load_track(&mut self, name: &str) {
        // Builtins win over a saved file of the same name, and
        // `save::load_track` reads the disk only, so the check stays first.
        if let Some(doc) = builtin_tracks().into_iter().find(|d| d.name == name) {
            self.set_track(doc);
            return;
        }
        match crate::save::load_track(name) {
            Some(doc) => self.set_track(doc),
            None => self.data.notice = Some(format!("Could not load track \"{name}\"")),
        }
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

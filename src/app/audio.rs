//! Sound.
//!
//! Every cue is synthesised at startup rather than shipped beside the binary.
//! `repame_audio` writes the WAV bytes and decodes them back through its own
//! bank, so the game has a voice without carrying an asset pipeline or a
//! licensing question. The tones are deliberately plain: a short burst for a
//! one-shot and a low tone for the engine, which is one long-lived voice kept
//! at a playback rate that follows the car.
//!
//! Nothing here is audible until the player asks for it: `sound` is off in the
//! options by default, and off stops the engine's voice rather than muting it.

use repame_audio::{Audio, AudioChannel, CueDef, Variation, synth_sine_wav};

/// Rate the cues are synthesised at. The mixer resamples, so this only has to
/// be a rate the sine is accurate at.
const SAMPLE_RATE: u32 = 44_100;

/// Speed the engine reaches its top pitch at, in km/h. Above it the rate holds:
/// a car that keeps rising in pitch stops reading as a car.
const ENGINE_TOP_KMH: f32 = 190.0;

/// Engine playback rate standing still and at [`ENGINE_TOP_KMH`].
const ENGINE_RATE_IDLE: f32 = 0.6;
const ENGINE_RATE_TOP: f32 = 2.0;

/// Gain of the engine voice within its channel, so a one-shot over it is still
/// audible.
const ENGINE_GAIN: f32 = 0.3;

const CLICK: &str = "ui.click";
const CHECKPOINT: &str = "race.checkpoint";
const FINISH: &str = "race.finish";
const LAND: &str = "race.land";
const ENGINE: &str = "race.engine";

pub struct Sound {
    audio: Audio,
    /// The engine's voice. `None` whenever the engine is not running, which is
    /// also how a silent device is told apart from a stopped engine.
    engine: Option<u64>,
    /// The rate the engine voice was last given, so a car holding a speed does
    /// not re-send the same number every frame.
    rate: f32,
    levels: [f32; 3],
    /// Whether sound is on at all. Off is not a master fader at zero: off stops
    /// the engine's voice and refuses to start another, where a silent master
    /// would leave a voice running that nobody can hear.
    enabled: bool,
    /// Checkpoints crossed as of the last frame, so a crossing is a change in a
    /// count rather than a call the racing code has to remember to make.
    checkpoint: usize,
    /// Whether the car was off the ground last frame.
    airborne: bool,
}

impl Sound {
    pub fn new() -> Self {
        let mut audio = Audio::try_init().unwrap_or_else(|_| Audio::noop());
        load_cues(&mut audio);
        let mut sound = Self {
            audio,
            engine: None,
            rate: 0.0,
            levels: [f32::NAN; 3],
            enabled: false,
            checkpoint: 0,
            airborne: false,
        };
        sound.apply(false, 1.0, 1.0, 1.0);
        sound
    }

    /// Whether a device is actually playing. False is the no-op backend, which
    /// is what a platform with no audio at all gets instead of a failure.
    pub fn is_live(&self) -> bool {
        self.audio.is_live()
    }

    /// Whether sound is on, and the three faders, in one call.
    ///
    /// One call rather than two because the first thing every option screen does
    /// is hand over all four, and a caller that set the faders without saying
    /// whether sound was on would leave the game audible or silent by accident.
    pub fn apply(&mut self, enabled: bool, master: f32, music: f32, sfx: f32) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.silence();
            }
        }
        if self.levels == [master, music, sfx] {
            return;
        }
        self.levels = [master, music, sfx];
        self.audio.set_channel(AudioChannel::Master, master);
        self.audio.set_channel(AudioChannel::Music, music);
        self.audio.set_channel(AudioChannel::Sfx, sfx);
    }

    /// One frame of sound.
    ///
    /// `running` is whether a race is being driven. The engine only has a voice
    /// while one is, so leaving a run stops it rather than leaving it idling
    /// under the menu it was abandoned to.
    pub fn update(
        &mut self,
        dt: f32,
        speed_kmh: f32,
        running: bool,
        checkpoint: usize,
        airborne: bool,
    ) {
        self.audio.update(dt);

        // The first checkpoint of a run is the start line being left behind, and
        // the count going from zero is the run beginning rather than a gate
        // being crossed. Only a rise from one gate to the next is a crossing.
        let crossed = checkpoint > self.checkpoint && self.checkpoint > 0;
        let landed = self.airborne && !airborne;
        // Tracked whether or not sound is on, so turning it on mid-run does not
        // fire the crossings that happened while it was off.
        self.checkpoint = checkpoint;
        self.airborne = airborne;

        if !self.enabled {
            return;
        }
        if crossed {
            self.audio.play(CHECKPOINT);
        }
        if landed {
            self.audio.play(LAND);
        }

        if !running {
            if let Some(voice) = self.engine.take() {
                self.audio.stop_voice(voice);
            }
            return;
        }
        if !self.audio.is_live() {
            return;
        }

        let rate = ENGINE_RATE_IDLE
            + (ENGINE_RATE_TOP - ENGINE_RATE_IDLE) * (speed_kmh / ENGINE_TOP_KMH).clamp(0.0, 1.0);
        match self.engine {
            None => {
                self.engine = self.audio.play_loop(ENGINE, ENGINE_GAIN, rate);
                self.rate = rate;
            }
            Some(voice) => {
                if (rate - self.rate).abs() > 0.01 {
                    self.audio.set_voice_rate(voice, rate);
                    self.rate = rate;
                }
            }
        }
    }

    pub fn finish(&mut self) {
        self.audio.play(FINISH);
    }

    pub fn click(&mut self) {
        self.audio.play(CLICK);
    }

    /// Stop everything that is still sounding. Called when a run ends, so a
    /// finish sting is not layered over an engine that never stopped.
    pub fn silence(&mut self) {
        self.engine = None;
        self.audio.silence();
    }
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

fn load_cues(audio: &mut Audio) {
    let ui = CueDef {
        bus: AudioChannel::Ui,
        gain: 0.5,
        cooldown_ms: 40,
        max_voices: 2,
        ..CueDef::default()
    };
    let _ = audio.load_cue(CLICK, ui, &[&synth_sine_wav(880.0, 0.05, SAMPLE_RATE)]);

    let one_shot = |gain: f32, voices: usize, wobble: f32| CueDef {
        bus: AudioChannel::Sfx,
        gain,
        max_voices: voices,
        pitch_wobble: wobble,
        variation: Variation::Random,
        ..CueDef::default()
    };
    let _ = audio.load_cue(
        CHECKPOINT,
        one_shot(0.5, 2, 0.02),
        &[&synth_sine_wav(1_320.0, 0.14, SAMPLE_RATE)],
    );
    let _ = audio.load_cue(
        FINISH,
        one_shot(0.6, 2, 0.0),
        &[&synth_sine_wav(1_760.0, 0.45, SAMPLE_RATE)],
    );
    let _ = audio.load_cue(
        LAND,
        one_shot(0.55, 3, 0.18),
        &[&synth_sine_wav(150.0, 0.16, SAMPLE_RATE)],
    );

    // The engine is the one cue whose shape matters rather than its tone: the
    // burst decays inside its own length, so looping it at a rate is a pulse
    // train, which is what an engine is. A single sustaining tone would loop
    // seamlessly and read as a test signal instead.
    let engine = CueDef {
        bus: AudioChannel::Sfx,
        gain: 1.0,
        max_voices: 1,
        ..CueDef::default()
    };
    let _ = audio.load_cue(ENGINE, engine, &[&synth_sine_wav(105.0, 0.22, SAMPLE_RATE)]);
}

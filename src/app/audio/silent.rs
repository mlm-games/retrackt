//! A `Sound` for the targets whose audio backend does not build yet.
//!
//! Every method the live backend has, answering nothing. The call sites are the
//! same on every target, so a build without a backend still has a sound object
//! to hold, hand levels to and drive per frame, and nothing has to know which
//! of the two it was given.

pub struct Sound;

impl Sound {
    pub fn new() -> Self {
        Self
    }

    pub fn is_live(&self) -> bool {
        false
    }

    pub fn apply(&mut self, _enabled: bool, _master: f32, _music: f32, _sfx: f32) {}

    pub fn update(
        &mut self,
        _dt: f32,
        _speed_kmh: f32,
        _running: bool,
        _checkpoint: usize,
        _airborne: bool,
    ) {
    }

    pub fn finish(&mut self) {}

    pub fn click(&mut self) {}

    pub fn silence(&mut self) {}
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

//! Sound.
//!
//! Every cue is synthesised at startup rather than shipped beside the binary.
//! `repame_audio` writes the WAV bytes and decodes them back through its own
//! bank, so the game has a voice without carrying an asset pipeline or a
//! licensing question. The tones are deliberately plain: a short burst for a
//! one-shot and a low tone for the engine, which is one long-lived voice kept
//! at a playback rate that follows the car.
//!
//! The backend is desktop-only for now. `repame-audio` reaches the speakers
//! through cpal, and cpal's wasm and Android backends do not build in this
//! project's CI; a build for either target gets the silent [`Sound`] below
//! instead. That is the lesser evil by a distance: a game that will not build
//! for a phone is worse than one that is quiet on it, and neither target had
//! sound before. Anything that reads as a finished mixed soundtrack has to wait
//! on those two backends building.

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod live;
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod silent;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use live::Sound;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub use silent::Sound;

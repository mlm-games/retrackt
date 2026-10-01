//! Headless fixed-step simulation.

pub mod car;
pub mod world;

pub use car::{Car, CarTuning, step_car};
pub use world::{Gate, TrackWorld};

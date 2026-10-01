//! retrackt: a time-trial racer on the repame + repose stack.

pub mod app;
pub mod save;
pub mod session;
pub mod sim;
pub mod ui;

/// Simulation rate. Fixed, so replays reproduce exactly.
pub const SIM_HZ: u32 = 120;

/// Fixed step duration.
pub const SIM_STEP: web_time::Duration =
    web_time::Duration::from_nanos(1_000_000_000 / SIM_HZ as u64);

//! retrackt: a time-trial racer on the repame + repose stack.

pub mod app;
pub mod save;
pub mod session;
pub mod sim;
pub mod ui;

/// Report a failing assertion as a workflow annotation.
///
/// A build reads workflow commands off its step's own output, and a check run's
/// annotations are the one place that comes back through the checks API — the
/// test logs behind them do not. Without this a red build says only that
/// something failed, and finding out what costs a whole build.
///
/// Called at the top of a test rather than once at load, because the harness runs
/// tests on many threads at once and a hook installed by whichever test happens to
/// start first is not installed in time for the one that panics first. The hook
/// itself is installed once however many tests ask.
#[cfg(test)]
pub fn report_failure_where_it_happens() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let said = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| String::from("assertion failed"));
            let at = info.location();
            let file = at.map_or("retrackt", |l| l.file());
            let line = at.map_or(0, |l| l.line());
            let clean = said
                .replace('\n', " ")
                .replace('\r', " ")
                .replace('%', "%25")
                .replace(':', "%3A");
            eprintln!("::error file={file},line={line},title=failed assertion::{clean}");
            previous(info);
        }));
    });
}

/// Simulation rate. Fixed, so replays reproduce exactly.
pub const SIM_HZ: u32 = 120;

/// Fixed step duration.
pub const SIM_STEP: web_time::Duration =
    web_time::Duration::from_nanos(1_000_000_000 / SIM_HZ as u64);

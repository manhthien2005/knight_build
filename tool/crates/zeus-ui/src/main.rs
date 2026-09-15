//! Native Windows account UI binary.

#![cfg_attr(windows, windows_subsystem = "windows")]
// On non-Windows targets `main` is an unsupported-platform stub, so the pure model, controller, and
// port reach no binary code path and are exercised only by this crate's tests. The Windows build
// carries no such allowance: it must pass `-D warnings` with every item genuinely reachable.
#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "the non-Windows binary is an unsupported-platform stub"
    )
)]

use std::process::ExitCode;

mod app;
mod model;
mod port;
mod windows;

#[cfg(windows)]
fn main() -> ExitCode {
    // The shell owns process startup: DPI awareness, common controls, the portable layout, the worker,
    // the window, and the message loop. A nonzero code means a bounded startup failure was reported.
    ExitCode::from(windows::shell_window::run())
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("zeus-ui runs on Windows only");
    ExitCode::FAILURE
}

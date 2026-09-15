#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{
    ProcessBirthId, RootExit, UnconfirmedWindowsProcess, WindowsContainedProcess,
    WindowsProcessError, WindowsSpawnFailure, spawn,
};

#[cfg(windows)]
#[allow(
    unused_imports,
    reason = "bounded error diagnostics remain crate-private for sibling lifecycle callers"
)]
pub(crate) use windows::{WindowsProcessErrorKind, WindowsProcessStage};

#[cfg(all(test, windows))]
pub(crate) use windows::test_support;

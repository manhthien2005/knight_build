//! Native Win32 shell for the account manager.
//!
//! The main thread performs no SQLite, crypto, runtime, process, sleep, or input work: it creates
//! controls, drains bounded event batches on a timer, and submits commands through the port.

pub mod controls;
#[cfg(windows)]
pub mod dialog_window;
pub mod dialogs;
pub mod icons;
pub mod layout;
pub mod metrics;
pub mod player_view;
#[cfg(windows)]
pub mod shell_window;

/// Timer cadence for draining worker events.
pub const EVENT_TIMER_MS: u32 = 50;

/// Timer id for the event drain.
pub const EVENT_TIMER_ID: usize = 1;

/// Window class name of the shell's top-level window.
pub const WINDOW_CLASS: &str = "ZeusAccountManagerWindow";

/// Main window caption.
pub const WINDOW_TITLE: &str = "Zeus HSO - Quản lý tài khoản";

/// Startup failure that must be reported before any window exists.
///
/// A pre-window failure has no place to draw a status line, so it uses a bounded message box and a
/// nonzero exit code instead of failing silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupFailure {
    /// The portable data root could not be opened or repaired.
    DataRoot,
    /// The pinned runtime could not be verified or relocated.
    PinnedRuntime,
    /// The worker thread could not be started.
    Worker,
    /// A Win32 registration or control-library call failed.
    Shell,
}

impl StartupFailure {
    /// Bounded Vietnamese copy for the pre-window message box.
    pub fn label(self) -> &'static str {
        match self {
            Self::DataRoot => {
                "Không mở được thư mục dữ liệu. Hãy chạy Zeus từ ổ đĩa cục bộ có quyền ghi."
            }
            Self::PinnedRuntime => "Không kiểm tra được bộ chạy game đã ghim.",
            Self::Worker => "Không khởi động được tiến trình quản lý.",
            Self::Shell => "Không khởi tạo được giao diện Windows.",
        }
    }

    pub fn title(self) -> &'static str {
        "Zeus HSO"
    }

    /// Process exit code. Never zero, so a launcher can detect the failure.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::DataRoot => 2,
            Self::PinnedRuntime => 3,
            Self::Worker => 4,
            Self::Shell => 5,
        }
    }
}

#[cfg(windows)]
pub use shell::{initialize_process_dpi, register_common_controls, show_startup_failure};

#[cfg(windows)]
mod shell {
    use windows_sys::Win32::UI::Controls::{
        ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
    };
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    use super::StartupFailure;

    /// Declares PerMonitorV2 awareness before the first HWND exists.
    ///
    /// Called before any window is created, because awareness cannot be changed afterwards and a late
    /// call would leave the table blurry on a rescaled monitor.
    pub fn initialize_process_dpi() -> bool {
        // SAFETY: the context constant is a valid opaque handle from windows-sys.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 }
    }

    /// Registers the list view and toolbar classes the shell needs.
    pub fn register_common_controls() -> bool {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
        };
        // SAFETY: `controls` is a correctly sized, fully initialized structure.
        unsafe { InitCommonControlsEx(&controls) != 0 }
    }

    /// Reports a pre-window startup failure through a bounded modal message box.
    pub fn show_startup_failure(failure: StartupFailure) {
        let text = to_wide(failure.label());
        let caption = to_wide(failure.title());
        // SAFETY: both buffers are NUL-terminated and outlive the call.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shell_startup_failures_are_bounded_and_nonzero() {
        let failures = [
            StartupFailure::DataRoot,
            StartupFailure::PinnedRuntime,
            StartupFailure::Worker,
            StartupFailure::Shell,
        ];
        let mut codes = Vec::new();
        for failure in failures {
            let label = failure.label();
            assert!(!label.is_empty(), "{failure:?} has no copy");
            // No backend identifier reaches a pre-window dialog.
            assert!(!label.contains('_'), "{failure:?} renders a backend token");
            for forbidden in ["sqlite", "hresult", "win32", "error code", "0x"] {
                assert!(
                    !label.to_lowercase().contains(forbidden),
                    "{failure:?} renders {forbidden}"
                );
            }
            // A launcher must be able to detect the failure.
            assert_ne!(failure.exit_code(), 0);
            codes.push(failure.exit_code());
        }
        // Each failure is distinguishable by exit code.
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), failures.len());
    }

    #[test]
    fn windows_shell_event_timer_matches_the_approved_cadence() {
        assert_eq!(EVENT_TIMER_MS, 50);
        assert_eq!(crate::model::MAX_EVENTS_PER_POLL, 32);
        // The window identity is stable so a second instance cannot silently register a variant.
        assert_eq!(WINDOW_CLASS, "ZeusAccountManagerWindow");
        assert!(!WINDOW_TITLE.is_empty());
    }
}

//! Stable accessible names and Vietnamese tooltips for every icon-only control.
//!
//! These are pure data so the complete icon inventory is assertable without creating a window. Every
//! icon-only control must have both a stable accessible name and a tooltip; a bare glyph is not
//! reachable by a screen reader.

/// Command ids. These are the control identifiers Win32 reports back in `WM_COMMAND`.
pub const CMD_ADD: u16 = 0x1001;
pub const CMD_RUN_SELECTED: u16 = 0x1002;
pub const CMD_STOP_SELECTED: u16 = 0x1003;
pub const CMD_DELETE_SELECTED: u16 = 0x1004;
pub const CMD_REFRESH: u16 = 0x1005;

/// Row action command ids.
pub const CMD_ROW_ACTION: u16 = 0x1101;
pub const CMD_ROW_EDIT: u16 = 0x1102;
pub const CMD_ROW_DELETE: u16 = 0x1103;
pub const CMD_ROW_SERVER: u16 = 0x1104;
/// Cycles one row's auto-attack mode: off, stand and fight, follow the target.
pub const CMD_ROW_AUTO: u16 = 0x1105;
/// Opens the settings dialog for one row.
pub const CMD_ROW_CONFIG: u16 = 0x1106;

/// One icon-only control's stable identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconCommand {
    pub id: u16,
    /// Stable accessible name. Never localized, so automation and assistive tech can rely on it.
    pub accessible_name: &'static str,
    /// Vietnamese tooltip shown to the operator.
    pub tooltip: &'static str,
}

/// The toolbar inventory in display order: Add, Run, Stop, Auto attack, Auto settings, Choose server, Edit, Delete, Refresh.
pub const TOOLBAR_COMMANDS: &[IconCommand] = &[
    IconCommand {
        id: CMD_ADD,
        accessible_name: "add_account",
        tooltip: "➕ Thêm Acc",
    },
    IconCommand {
        id: CMD_RUN_SELECTED,
        accessible_name: "run_selected",
        tooltip: "▶️ Chạy",
    },
    IconCommand {
        id: CMD_STOP_SELECTED,
        accessible_name: "stop_selected",
        tooltip: "⏹️ Dừng",
    },
    IconCommand {
        id: CMD_ROW_AUTO,
        accessible_name: "row_auto_attack",
        tooltip: "⚡ Tự đánh: Tắt",
    },
    IconCommand {
        id: CMD_ROW_CONFIG,
        accessible_name: "row_auto_settings",
        tooltip: "⚙️ Cài đặt Auto",
    },
    IconCommand {
        id: CMD_ROW_SERVER,
        accessible_name: "row_choose_server",
        tooltip: "🌐 Chọn Server",
    },
    IconCommand {
        id: CMD_ROW_EDIT,
        accessible_name: "row_edit",
        tooltip: "✏️ Sửa",
    },
    IconCommand {
        id: CMD_DELETE_SELECTED,
        accessible_name: "delete_selected",
        tooltip: "🗑️ Xóa",
    },
    IconCommand {
        id: CMD_REFRESH,
        accessible_name: "refresh_accounts",
        tooltip: "🔄 Tải lại",
    },
];

/// Empty row commands: all actions are unified on the single top toolbar.
pub const ROW_COMMANDS: &[IconCommand] = &[];

/// Tooltip for the contextual row action, which changes with the row's status.
pub fn row_action_tooltip(action: crate::model::UiRowAction) -> Option<&'static str> {
    match action {
        crate::model::UiRowAction::Run => Some("Chạy tài khoản"),
        crate::model::UiRowAction::Stop => Some("Dừng tài khoản"),
        crate::model::UiRowAction::RetryCleanup => Some("Dọn dẹp lại"),
        crate::model::UiRowAction::None => None,
    }
}

/// Loads the application brand icon at the requested square dimension (e.g. 16, 32, 48).
#[cfg(windows)]
pub fn load_app_icon(size: i32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    use std::path::PathBuf;
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HICON, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW,
    };
    use crate::windows::layout::PortableLayout;

    let icon_path = PortableLayout::from_current_executable()
        .map(|l| l.root.join("app_icon.ico"))
        .unwrap_or_else(|| PathBuf::from("app_icon.ico"));

    if !icon_path.exists() {
        let _ = std::fs::write(&icon_path, include_bytes!("../../resources/app_icon.ico"));
    }

    let wide_path: Vec<u16> = icon_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        LoadImageW(
            null_mut(),
            wide_path.as_ptr(),
            IMAGE_ICON,
            size,
            size,
            LR_LOADFROMFILE,
        ) as HICON
    }
}


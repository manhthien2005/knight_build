//! Modal Add, Edit, Delete, server, and settings dialogs.
//!
//! Built from plain child controls rather than a resource template, because M3.1 ships no resource
//! script. Each dialog runs a local message loop that calls the same bounded worker drain the main
//! timer uses, so worker progress never stalls behind a modal window.
//!
//! Geometry and fonts follow the window's DPI. The settings dialog is a dense fixed grid, so at 125%
//! or 150% a 96-DPI bitmap font and unscaled rectangles would clip controls with nothing to resize.

#![cfg(windows)]

use std::cell::RefCell;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CreateFontIndirectW, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject,
    GetStockObject, HDC, HFONT, InvalidateRect, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, EM_SETPASSWORDCHAR, WC_BUTTONW, WC_COMBOBOXW, WC_EDITW, WC_STATICW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_GROUPBOX, CB_ADDSTRING,
    CB_GETCURSEL, CB_GETITEMDATA, CB_GETLBTEXT, CB_GETLBTEXTLEN, CB_RESETCONTENT, CB_SETCURSEL,
    CB_SETITEMDATA, CBS_DROPDOWNLIST, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, ES_NUMBER, ES_PASSWORD, GetDlgCtrlID, GetDlgItem, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowRect, ICON_BIG, ICON_SMALL, MINMAXINFO, MSG,
    NONCLIENTMETRICSW, RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLORDLG, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_GETMINMAXINFO, WM_SETFONT, WM_SETICON, WM_SIZE, WNDCLASSEXW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_GROUP, WS_MAXIMIZEBOX, WS_SYSMENU, WS_TABSTOP,
    WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

use super::dialogs::{
    self, ConfigControl, ConfigGroup, ConfigRow, DELETE_PROMPT, DELETE_TITLE, FieldError,
    ID_CANCEL, ID_CONFIRM_TEXT, ID_OK, ID_PASSWORD, ID_SHOW_PASSWORD, ID_USERNAME,
};
use super::metrics::{ConfigMetrics, Rect, scale};
use crate::model::{
    BUFF_SLOTS, MATERIAL_SLOTS, NAV_TARGET_OPTIONS, UiAutoMode, UiControl, UiSavedSpot, UiSpot,
    UiSpotBook,
};

/// Combo-box notification for "the selection changed", in the high word of `WM_COMMAND`'s wparam.
///
/// Spelled out rather than imported: this crate's `windows-sys` feature set does not export it, and one
/// named constant beats a bare 1 at the comparison site.
const CBN_SELCHANGE: u16 = 1;

/// Window class of every modal dialog in this module.
const DIALOG_CLASS: &str = "ZeusAccountManagerDialog";

/// Child control ids for the message and error labels.
const ID_MESSAGE: isize = 0x2101;
const ID_ERROR: isize = 0x2199;

/// Mask character used whenever the password is hidden.
const PASSWORD_MASK: u16 = b'*' as u16;

/// Columns the settings dialog lays its groups out in: Combat, Recovery, Travel, Loot.
const CONFIG_COLUMNS: usize = 4;

/// Light mode color constants in BGR format for Win32 GDI.
const COLOR_WINDOW_BG: u32 = 0x00F5F2F0; // #F0F2F5
const COLOR_TEXT_SLATE: u32 = 0x003B291E; // #1E293B
const COLOR_TEXT_HEADER: u32 = 0x000F172A; // #0F172A
const COLOR_TEXT_MUTED: u32 = 0x008B7464; // #64748B
const COLOR_TEXT_ERROR: u32 = 0x002626DC; // #DC2626
const COLOR_WHITE: u32 = 0x00FFFFFF;

/// What the operator submitted.
pub(super) struct AccountSubmission {
    pub(super) username: String,
    /// Empty means "leave the stored password unchanged", which only Edit allows.
    pub(super) password: Vec<u16>,
}

/// Outcome of one modal dialog.
enum DialogOutcome {
    Pending,
    Cancelled,
    Submitted,
}

/// What the mount picker calls [`zeus_core::MOUNT_ANY`].
const MOUNT_ANY_LABEL: &str = "thú nào cũng được";

/// Per-dialog state owned by the window procedure.
struct DialogState {
    kind: DialogKind,
    outcome: DialogOutcome,
    username: String,
    password: Vec<u16>,
    /// Server index the picker starts on, then the one the operator chose.
    server_index: u8,
    /// Settings the config dialog opened on, then the ones the operator submitted.
    control: UiControl,
    /// Where the character is standing, for the config dialog's read-only spot line.
    ///
    /// `None` means no settled reading, which is exactly when arming would be refused, so the line
    /// says so instead of showing a position the operator could not actually farm.
    live: Option<UiSpot>,
    /// The spots already saved for the map the character is standing on.
    ///
    /// The dialog's own copy, seeded when it opens and edited as buttons act, so the picker shows what
    /// is stored rather than what the operator hoped they stored.
    saved: Vec<UiSavedSpot>,
    /// The mounts the character's bag holds, for the mount picker.
    ///
    /// Seeded when the dialog opens and never edited: it is what the client reported, not a setting.
    /// Empty means the character carries none, so the picker offers only "any".
    mounts: Vec<(u16, String)>,
    /// Book requests the operator pressed, submitted with the settings.
    spot_requests: Vec<SpotRequest>,
    /// DPI the controls were laid out for.
    dpi: u32,
    /// Font this dialog owns, deleted when it closes.
    font: HFONT,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    /// Username plus a required password.
    Add,
    /// Username plus an optional password.
    Edit,
    /// Typed confirmation word.
    Delete,
    /// One radio button per world in the client's server table.
    Server,
    /// Every setting the running client's attack and item modules read.
    Config,
}

thread_local! {
    /// The dialog runs on the UI thread only, so thread-local ownership avoids a lock.
    static DIALOG: std::cell::RefCell<Option<DialogState>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Registers the shared dialog class once per process.
fn register_dialog_class() -> bool {
    static mut REGISTERED: bool = false;
    // SAFETY: the UI thread is the only caller, so this static is never raced.
    unsafe {
        if REGISTERED {
            return true;
        }
    }
    let class_name = to_wide(DIALOG_CLASS);
    let icon_big = crate::windows::icons::load_app_icon(32);
    let icon_sm = crate::windows::icons::load_app_icon(16);
    let bg_brush = unsafe { CreateSolidBrush(COLOR_WINDOW_BG) };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(dialog_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        // SAFETY: null requests the current process image handle.
        hInstance: unsafe { GetModuleHandleW(null()) },
        hIcon: icon_big,
        hCursor: null_mut(),
        hbrBackground: bg_brush,
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: icon_sm,
    };
    // SAFETY: `class` is fully initialized and its string outlives the call.
    let registered = unsafe { RegisterClassExW(&class) != 0 };
    if registered {
        // SAFETY: single-threaded UI access, as above.
        unsafe {
            REGISTERED = true;
        }
    }
    registered
}


/// Shows the Add dialog, returning the submission or `None` when cancelled.
pub(super) fn prompt_add(parent: HWND, drain: &mut dyn FnMut()) -> Option<AccountSubmission> {
    let state = run_dialog(parent, opening(DialogKind::Add), drain)?;
    Some(AccountSubmission {
        username: state.username,
        password: state.password,
    })
}

/// Shows the Edit dialog pre-filled with the current username.
pub(super) fn prompt_edit(
    parent: HWND,
    username: &str,
    drain: &mut dyn FnMut(),
) -> Option<AccountSubmission> {
    let state = run_dialog(
        parent,
        DialogState {
            username: username.to_owned(),
            ..opening(DialogKind::Edit)
        },
        drain,
    )?;
    Some(AccountSubmission {
        username: state.username,
        password: state.password,
    })
}

/// Shows the destructive delete confirmation, requiring the exact confirmation word.
pub(super) fn confirm_delete(parent: HWND, drain: &mut dyn FnMut()) -> bool {
    run_dialog(parent, opening(DialogKind::Delete), drain).is_some()
}

/// Shows the server picker, returning the chosen index or `None` when cancelled.
pub(super) fn prompt_server(parent: HWND, current: u8, drain: &mut dyn FnMut()) -> Option<u8> {
    run_dialog(
        parent,
        DialogState {
            server_index: current,
            ..opening(DialogKind::Server)
        },
        drain,
    )
    .map(|state| state.server_index)
}

/// Shows the settings dialog, returning the edited settings or `None` when cancelled.
///
/// `current` is what the engine last confirmed, so the dialog opens on what the client is really
/// reading. `live` is where the character is standing, used for the read-only spot line and as the
/// default position when the operator has never chosen one.
pub(super) fn prompt_config(
    parent: HWND,
    current: UiControl,
    live: Option<UiSpot>,
    saved: Vec<UiSavedSpot>,
    mounts: Vec<(u16, String)>,
    drain: &mut dyn FnMut(),
) -> Option<ConfigSubmission> {
    run_dialog(
        parent,
        DialogState {
            control: current,
            live,
            saved,
            mounts,
            ..opening(DialogKind::Config)
        },
        drain,
    )
    .map(|state| ConfigSubmission {
        settings: state.control,
        spots: state.spot_requests,
    })
}

/// What the config dialog submitted: the settings, and anything it asked of the shared spot book.
///
/// Two fields because they go to different places. Settings belong to one account; the book is shared
/// by all of them, so a save cannot ride along inside `UiControl` without pretending to be per-account.
pub(super) struct ConfigSubmission {
    pub(super) settings: UiControl,
    /// Requests in the order the operator pressed them, so the last press wins where they conflict.
    pub(super) spots: Vec<SpotRequest>,
}

/// One change the operator asked of the shared spot book.
///
/// Only the book: pressing ĐI MAP or Dò bãi acts immediately through [`install_actions`], because both
/// are things the operator wants to happen while they are still looking at the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SpotRequest {
    /// Save where the character is standing under this name.
    Save { spot: UiSpot, name: String },
    /// Forget the named spot on this map.
    Clear { map_id: u16, name: String },
}

/// A dialog's opening state, before the operator has touched anything.
fn opening(kind: DialogKind) -> DialogState {
    DialogState {
        kind,
        outcome: DialogOutcome::Pending,
        username: String::new(),
        password: Vec::new(),
        saved: Vec::new(),
        spot_requests: Vec::new(),
        mounts: Vec::new(),
        server_index: 0,
        control: UiControl::default(),
        live: None,
        dpi: super::metrics::REFERENCE_DPI,
        font: null_mut(),
    }
}

fn run_dialog(parent: HWND, opening: DialogState, drain: &mut dyn FnMut()) -> Option<DialogState> {
    if !register_dialog_class() {
        return None;
    }
    let kind = opening.kind;
    // SAFETY: `parent` is this thread's live shell window.
    let dpi = unsafe { GetDpiForWindow(parent) };
    let font = dialog_font(dpi);
    DIALOG.with(|dialog| {
        *dialog.borrow_mut() = Some(DialogState {
            dpi,
            font,
            ..opening
        });
    });

    let class_name = to_wide(DIALOG_CLASS);
    let caption = to_wide(title_for(kind));
    let (style, ex_style) = if kind == DialogKind::Config {
        (
            WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX,
            0,
        )
    } else {
        (
            WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
            0,
        )
    };
    let (width, height) = window_size(kind, dpi, style, ex_style);
    let (x, y) = if !parent.is_null() {
        let mut parent_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: `parent` is a live window.
        if unsafe { GetWindowRect(parent, &mut parent_rect) != 0 } {
            let parent_w = parent_rect.right - parent_rect.left;
            let parent_h = parent_rect.bottom - parent_rect.top;
            let px = parent_rect.left + (parent_w - width) / 2;
            let py = parent_rect.top + (parent_h - height) / 2;
            (px.max(10), py.max(10))
        } else {
            (CW_USEDEFAULT, CW_USEDEFAULT)
        }
    } else {
        (CW_USEDEFAULT, CW_USEDEFAULT)
    };
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let window = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            caption.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            null_mut(),
        )
    };
    if window.is_null() {
        DIALOG.with(|dialog| *dialog.borrow_mut() = None);
        delete_dialog_font(font);
        return None;
    }

    let icon_big = crate::windows::icons::load_app_icon(32);
    let icon_sm = crate::windows::icons::load_app_icon(16);
    if !icon_big.is_null() {
        unsafe {
            SendMessageW(window, WM_SETICON, ICON_BIG as usize, icon_big as isize);
        }
    }
    if !icon_sm.is_null() {
        unsafe {
            SendMessageW(window, WM_SETICON, ICON_SMALL as usize, icon_sm as isize);
        }
    }

    build_controls(window);
    // The parent is disabled for the duration, which is what makes this modal.
    // SAFETY: `parent` is a live window owned by this thread.
    unsafe {
        EnableWindow(parent, 0);
        ShowWindow(window, SW_SHOW);
    }

    pump_dialog(window, drain);

    // SAFETY: re-enabling the parent before returning, on every path.
    unsafe {
        EnableWindow(parent, 1);
        SetFocus(parent);
    }

    let state = DIALOG.with(|dialog| dialog.borrow_mut().take());
    // The font outlived every control that used it; the window is already destroyed.
    delete_dialog_font(font);
    let state = state?;
    match state.outcome {
        DialogOutcome::Submitted => Some(state),
        _ => None,
    }
}

fn title_for(kind: DialogKind) -> &'static str {
    match kind {
        DialogKind::Add => dialogs::ADD_TITLE,
        DialogKind::Edit => dialogs::EDIT_TITLE,
        DialogKind::Delete => DELETE_TITLE,
        DialogKind::Server => dialogs::SERVER_TITLE,
        DialogKind::Config => dialogs::CONFIG_TITLE,
    }
}

/// Outer window size for one dialog kind, so the client area holds every control it places.
fn window_size(kind: DialogKind, dpi: u32, style: u32, ex_style: u32) -> (i32, i32) {
    let (client_width, client_height) = match kind {
        DialogKind::Config => config_metrics().client_size(CONFIG_COLUMNS),
        DialogKind::Server => (scale(440, dpi), scale(270, dpi)),
        DialogKind::Delete => (scale(440, dpi), scale(230, dpi)),
        DialogKind::Add | DialogKind::Edit => (scale(440, dpi), scale(260, dpi)),
    };
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    // SAFETY: `frame` is a live rect and the style bits are the ones the window is created with.
    let adjusted =
        unsafe { AdjustWindowRectExForDpi(&mut frame, style, 0, ex_style, dpi) != 0 };
    if !adjusted {
        // Without the frame extent the client would be short by the caption and borders; a fixed
        // allowance is wrong by a few pixels rather than by a whole caption bar.
        return (
            client_width + scale(16, dpi),
            client_height + scale(39, dpi),
        );
    }
    (frame.right - frame.left, frame.bottom - frame.top)
}

/// Settings grid geometry for the current dialog's DPI and the tallest group.
fn config_metrics() -> ConfigMetrics {
    let dpi = DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .map_or(super::metrics::REFERENCE_DPI, |state| state.dpi)
    });
    let rows = [
        ConfigGroup::Combat,
        ConfigGroup::Recovery,
        ConfigGroup::Travel,
        ConfigGroup::Loot,
    ]
    .into_iter()
    .map(|group| dialogs::config_rows(group).count())
    .max()
    .unwrap_or(1);
    ConfigMetrics::new(dpi, rows)
}


/// The message font Windows itself uses for dialogs, at one DPI.
///
/// `DEFAULT_GUI_FONT` is a fixed 96-DPI bitmap face, so it renders undersized and jagged on a scaled
/// display — and in a dense grid it also mismeasures every caption. The caller owns the handle.
fn dialog_font(dpi: u32) -> HFONT {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is correctly sized and the call writes only into it.
    let queried = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            size_of::<NONCLIENTMETRICSW>() as u32,
            (&raw mut metrics).cast(),
            0,
            dpi,
        ) != 0
    };
    if !queried {
        // SAFETY: a stock handle needs no cleanup; `delete_dialog_font` skips it.
        return unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT };
    }
    let mut font = metrics.lfMessageFont;
    font.lfQuality = CLEARTYPE_QUALITY;
    // SAFETY: `font` is a fully initialized LOGFONTW that outlives the call.
    let created = unsafe { CreateFontIndirectW(&font) };
    if created.is_null() {
        // SAFETY: as above, the stock fallback needs no cleanup.
        unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT }
    } else {
        created
    }
}

/// Deletes a dialog-owned font, leaving the stock fallback alone.
fn delete_dialog_font(font: HFONT) {
    if font.is_null() {
        return;
    }
    // SAFETY: a stock handle must never be deleted, so it is compared out first.
    unsafe {
        if font as isize != GetStockObject(DEFAULT_GUI_FONT) as isize {
            DeleteObject(font as _);
        }
    }
}

fn build_controls(window: HWND) {
    let Some((kind, username, server_index, control, live, saved, mounts, dpi, font)) = DIALOG
        .with(|dialog| {
            dialog.borrow().as_ref().map(|state| {
                (
                    state.kind,
                    state.username.clone(),
                    state.server_index,
                    state.control.clone(),
                    state.live,
                    state.saved.clone(),
                    state.mounts.clone(),
                    state.dpi,
                    state.font,
                )
            })
        })
    else {
        return;
    };
    // Every reference-pixel constant below goes through this, so one scaled layout serves 100%,
    // 125% and 150% instead of clipping captions on the two the operator is most likely to use.
    let at = |x: i32, y: i32, width: i32, height: i32| Placement {
        x: scale(x, dpi),
        y: scale(y, dpi),
        width: scale(width, dpi),
        height: scale(height, dpi),
    };
    let label = |text: &str, id: isize, x: i32, y: i32, width: i32| {
        let control = create_child(window, WC_STATICW, text, 0, at(x, y, width, 20), id);
        set_font(control, font);
        control
    };

    match kind {
        DialogKind::Config => build_config_controls(window, control, live, saved, mounts, font),
        DialogKind::Server => {
            label("🌐 HSO MANAGER — CHỌN MÁY CHỦ", ID_MESSAGE + 50, 20, 14, 400);
            label(dialogs::SERVER_PROMPT, ID_MESSAGE, 20, 36, 400);
            let group_box = create_child(
                window,
                WC_BUTTONW,
                "Danh sách máy chủ",
                BS_GROUPBOX as u32,
                at(16, 62, 408, 144),
                ID_MESSAGE + 52,
            );
            set_font(group_box, font);

            for (index, name) in zeus_core::SERVER_NAMES.iter().enumerate() {
                let column = index / 4;
                let row = index % 4;
                let mut style = BS_AUTORADIOBUTTON as u32 | WS_TABSTOP;
                if index == 0 {
                    style |= WS_GROUP;
                }
                let button = create_child(
                    window,
                    WC_BUTTONW,
                    name,
                    style,
                    at(32 + column as i32 * 200, 88 + row as i32 * 28, 180, 24),
                    dialogs::ID_SERVER_FIRST as isize + index as isize,
                );
                set_font(button, font);
                if index == usize::from(server_index) {
                    // SAFETY: `button` is a live child of this dialog.
                    unsafe {
                        SendMessageW(button, BM_SETCHECK, BST_CHECKED as usize, 0);
                    }
                }
            }
        }
        DialogKind::Delete => {
            label("⚠️ HSO MANAGER — XÁC NHẬN XÓA", ID_MESSAGE + 50, 20, 14, 400);
            label("Hành động này sẽ xóa vĩnh viễn tài khoản khỏi hệ thống.", ID_MESSAGE + 51, 20, 36, 400);
            label(DELETE_PROMPT, ID_MESSAGE, 20, 68, 400);
            let field = create_child(
                window,
                WC_EDITW,
                "",
                WS_BORDER | WS_TABSTOP,
                at(20, 94, 400, 28),
                ID_CONFIRM_TEXT as isize,
            );
            set_font(field, font);
        }
        _ => {
            // Add and Edit differ in the password label: Edit states that empty keeps the stored one.
            let fields = if kind == DialogKind::Edit {
                dialogs::EDIT_FIELDS
            } else {
                dialogs::ADD_FIELDS
            };
            let (title_text, sub_text) = if kind == DialogKind::Edit {
                ("⚡ HSO MANAGER — SỬA TÀI KHOẢN", "Cập nhật mật khẩu hoặc thông tin đăng nhập")
            } else {
                ("⚡ HSO MANAGER — THÊM TÀI KHOẢN", "Nhập thông tin tài khoản và mật khẩu đăng nhập")
            };
            label(title_text, ID_MESSAGE + 50, 20, 14, 400);
            label(sub_text, ID_MESSAGE + 51, 20, 36, 400);

            let group_box = create_child(
                window,
                WC_BUTTONW,
                "Thông tin tài khoản",
                BS_GROUPBOX as u32,
                at(16, 62, 408, 126),
                ID_MESSAGE + 52,
            );
            set_font(group_box, font);

            label(fields[0].label, ID_MESSAGE, 32, 88, 85);
            let username = create_child(
                window,
                WC_EDITW,
                &username,
                WS_BORDER | WS_TABSTOP,
                at(122, 84, 286, 26),
                ID_USERNAME as isize,
            );
            set_font(username, font);

            label(fields[1].label, ID_MESSAGE + 1, 32, 120, 85);
            let password = create_child(
                window,
                WC_EDITW,
                "",
                WS_BORDER | WS_TABSTOP | ES_PASSWORD as u32,
                at(122, 116, 286, 26),
                ID_PASSWORD as isize,
            );
            set_font(password, font);

            let reveal = create_child(
                window,
                WC_BUTTONW,
                dialogs::SHOW_PASSWORD_LABEL,
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                at(122, 150, 160, 20),
                ID_SHOW_PASSWORD as isize,
            );
            set_font(reveal, font);
        }
    }

    // The picker fills the area the error label would occupy, and it cannot fail validation, so it
    // gets no error line. The settings dialog placed its own from the grid.
    if !matches!(kind, DialogKind::Server | DialogKind::Config) {
        let error_y = match kind {
            DialogKind::Delete => 134,
            _ => 194,
        };
        let error = label("", ID_ERROR, 20, error_y, 400);
        set_font(error, font);
    }

    let (ok_label, cancel_label) = match kind {
        DialogKind::Add => (dialogs::ADD_BUTTONS[1].1, dialogs::ADD_BUTTONS[0].1),
        DialogKind::Edit => (dialogs::EDIT_BUTTONS[1].1, dialogs::EDIT_BUTTONS[0].1),
        DialogKind::Delete => (dialogs::DELETE_BUTTONS[1].1, dialogs::DELETE_BUTTONS[0].1),
        DialogKind::Server => (dialogs::SERVER_BUTTONS[1].1, dialogs::SERVER_BUTTONS[0].1),
        DialogKind::Config => (dialogs::CONFIG_BUTTONS[1].1, dialogs::CONFIG_BUTTONS[0].1),
    };
    let (cancel_at, ok_at) = if kind == DialogKind::Config {
        let metrics = config_metrics();
        (
            metrics.button_rect(CONFIG_COLUMNS, 0, 2).into(),
            metrics.button_rect(CONFIG_COLUMNS, 1, 2).into(),
        )
    } else {
        let btn_y = match kind {
            DialogKind::Delete => 174,
            DialogKind::Server => 220,
            _ => 218,
        };
        (at(220, btn_y, 96, 30), at(326, btn_y, 96, 30))
    };
    let cancel = create_child(
        window,
        WC_BUTTONW,
        cancel_label,
        WS_TABSTOP,
        cancel_at,
        ID_CANCEL as isize,
    );
    set_font(cancel, font);
    let ok = create_child(
        window,
        WC_BUTTONW,
        ok_label,
        WS_TABSTOP,
        ok_at,
        ID_OK as isize,
    );
    set_font(ok, font);
}

/// Builds the settings grid: one group box per column, then one row per setting.
fn build_config_controls(
    window: HWND,
    control: UiControl,
    live: Option<UiSpot>,
    saved: Vec<UiSavedSpot>,
    mounts: Vec<(u16, String)>,
    font: HFONT,
) {
    let metrics = config_metrics();
    let opening = ConfigOpening {
        control,
        live,
        saved,
        mounts,
        font,
    };
    for (column, group) in [
        ConfigGroup::Combat,
        ConfigGroup::Recovery,
        ConfigGroup::Travel,
        ConfigGroup::Loot,
    ]
    .into_iter()
    .enumerate()
    {
        let box_rect = metrics.group_rect(column);
        // A group box is a button style, not a container: its children are siblings placed inside it.
        let caption = create_child(
            window,
            WC_BUTTONW,
            group.caption(),
            BS_GROUPBOX as u32,
            box_rect.into(),
            ID_MESSAGE + column as isize,
        );
        set_font(caption, font);
        for (index, row) in dialogs::config_rows(group).enumerate() {
            build_config_row(window, metrics, column, index, row, opening.clone());
        }
    }
    let error = create_child(
        window,
        WC_STATICW,
        "",
        0,
        metrics.error_rect(CONFIG_COLUMNS).into(),
        ID_ERROR,
    );
    set_font(error, font);
}


/// What every settings control needs to open on its current value.
#[derive(Clone)]
struct ConfigOpening {
    control: UiControl,
    live: Option<UiSpot>,
    /// What the shared book already holds for the map the character is on.
    saved: Vec<UiSavedSpot>,
    mounts: Vec<(u16, String)>,
    font: HFONT,
}

/// Builds one settings row: its label when it needs one, then its control, set to the current value.
fn build_config_row(
    window: HWND,
    metrics: ConfigMetrics,
    column: usize,
    index: usize,
    row: &ConfigRow,
    opening: ConfigOpening,
) {
    let ConfigOpening {
        control,
        live,
        saved,
        mounts,
        font,
    } = opening;
    let id = row.id as isize;
    match row.control {
        ConfigControl::Check => {
            // A checkbox carries its own caption, so it spans the whole row and needs no label.
            let check = create_child(
                window,
                WC_BUTTONW,
                row.label,
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                metrics.check_rect(column, index).into(),
                id,
            );
            set_font(check, font);
            if config_flag(row.id, &control) {
                // SAFETY: `check` is a live child of this dialog.
                unsafe {
                    SendMessageW(check, BM_SETCHECK, BST_CHECKED as usize, 0);
                }
            }
        }
        ConfigControl::Action => {
            // A button carries its own caption, so it spans the group box width and needs no label.
            // Unlike every other kind this is not a value the dialog reads back: it asks the engine to
            // act now, and the readout row above shows what came of it.
            let button = create_child(
                window,
                WC_BUTTONW,
                row.label,
                WS_TABSTOP,
                metrics.check_rect(column, index).into(),
                id,
            );
            set_font(button, font);
        }
        ConfigControl::Readout => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            let value = create_child(
                window,
                WC_STATICW,
                &spot_readout(live),
                0,
                metrics.control_rect(column, index).into(),
                id,
            );
            set_font(value, font);
        }
        ConfigControl::Number { .. } => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            let field = create_child(
                window,
                WC_EDITW,
                &config_number(row.id, &control, live),
                WS_BORDER | WS_TABSTOP | ES_NUMBER as u32,
                metrics.control_rect(column, index).into(),
                id,
            );
            set_font(field, font);
        }
        ConfigControl::SpotChoice => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            // Sized for a full map's worth of spots rather than for the current count: the list grows
            // as the operator saves, and a combo sized to today clips tomorrow.
            let mut placement: Placement = metrics.control_rect(column, index).into();
            placement.height += placement.height * crate::model::MAX_SPOTS_PER_MAP;
            let combo = create_child(
                window,
                WC_COMBOBOXW,
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
                placement,
                id,
            );
            set_font(combo, font);
            // One rule for "this map's spots", the book's own, so the list built here and the list
            // rebuilt on every change can never disagree.
            let book = UiSpotBook {
                entries: saved.clone(),
            };
            let listed = chosen_map_of(&control, live).unwrap_or(u16::MAX);
            for saved in book.for_map(listed) {
                let text = to_wide(&saved.name);
                // SAFETY: `combo` is a live child and the buffer outlives the call.
                unsafe {
                    SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as isize);
                }
            }
            // The stored name, not a row index: the list is rebuilt whenever the book changes.
            let selected = book
                .for_map(listed)
                .into_iter()
                .position(|saved| saved.name == control.spot_name);
            // SAFETY: as above; -1 clears the selection, which is what "none chosen" means.
            unsafe {
                SendMessageW(combo, CB_SETCURSEL, selected.unwrap_or(usize::MAX), 0);
            }
        }
        ConfigControl::Text => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            let field = create_child(
                window,
                WC_EDITW,
                &control.spot_name,
                WS_TABSTOP,
                metrics.control_rect(column, index).into(),
                id,
            );
            set_font(field, font);
        }
        ConfigControl::Choice(options) => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            // A drop-down list's height argument sizes the list, not the closed control.
            // Cap at 6 items as requested so long lists scroll neatly instead of stretching across the screen.
            let mut placement: Placement = metrics.control_rect(column, index).into();
            let visible_items = options.len().min(6) as i32;
            placement.height += placement.height * visible_items;
            let combo = create_child(
                window,
                WC_COMBOBOXW,
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
                placement,
                id,
            );
            set_font(combo, font);
            const CB_SETMINVISIBLE: u32 = 0x1701;
            // SAFETY: `combo` is a live child window.
            unsafe {
                SendMessageW(combo, CB_SETMINVISIBLE, 6, 0);
            }
            for option in options {
                let text = to_wide(option);
                // SAFETY: `combo` is a live child and the buffer outlives the call.
                unsafe {
                    SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as isize);
                }
            }
            let selected = config_choice(row.id, &control) as usize;
            // SAFETY: as above; an out-of-range index is refused by the control, not undefined.
            unsafe {
                SendMessageW(combo, CB_SETCURSEL, selected, 0);
            }
        }
        ConfigControl::MountChoice => {
            let label = create_child(
                window,
                WC_STATICW,
                row.label,
                0,
                metrics.label_rect(column, index).into(),
                ID_MESSAGE + 200 + index as isize,
            );
            set_font(label, font);
            // "Any" plus every mount id the client knows — not only what is carried. Picking one
            // is a standing instruction ("ride this when it is to hand"), so an id absent from
            // the bag stays a valid choice and the mod simply finds nothing to ride yet.
            let mut placement: Placement = metrics.control_rect(column, index).into();
            placement.height += placement.height * (zeus_core::MOUNT_TEMPLATE_IDS.len() as i32 + 1);
            let combo = create_child(
                window,
                WC_COMBOBOXW,
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
                placement,
                id,
            );
            set_font(combo, font);
            // Index 0 is "any": the mod rides whatever turns up rather than holding out for one
            // id, which is what an account that carries nothing needs.
            //
            // Each entry carries its template id as item data rather than being identified by
            // its position, so the read-back does not depend on the order the list was built in.
            //
            // Names come from the bag when the account is carrying that mount, because those are
            // the server's own; otherwise the fallback label stands in.
            for (index, (mount_id, name)) in
                std::iter::once((zeus_core::MOUNT_ANY, MOUNT_ANY_LABEL.to_owned()))
                    .chain(
                        zeus_core::MOUNT_TEMPLATE_IDS
                            .iter()
                            .map(|id| (*id, crate::model::mount_label(*id, &mounts))),
                    )
                    .enumerate()
            {
                let wide = to_wide(&name);
                // SAFETY: `combo` is a live child and the buffer outlives the call.
                unsafe {
                    SendMessageW(combo, CB_ADDSTRING, 0, wide.as_ptr() as isize);
                    SendMessageW(combo, CB_SETITEMDATA, index, mount_id as isize);
                }
            }
            let selected = zeus_core::MOUNT_TEMPLATE_IDS
                .iter()
                .position(|id| *id == control.mount_template_id)
                .map_or(0, |found| found + 1);
            // SAFETY: as above; an out-of-range index is refused by the control.
            unsafe {
                SendMessageW(combo, CB_SETCURSEL, selected, 0);
            }
        }
    }
}

/// The value a checkbox row opens on.
fn config_flag(id: u16, control: &UiControl) -> bool {
    match id {
        dialogs::ID_CFG_HP_ON => control.hp_on,
        dialogs::ID_CFG_MP_ON => control.mp_on,
        dialogs::ID_CFG_MOUNT => control.mount,
        dialogs::ID_CFG_REVIVE_ON => control.revive_on,
        dialogs::ID_CFG_MEDAL => control.medal_dialog,
        dialogs::ID_CFG_DROPS_ON => control.materials_managed,
        dialogs::ID_CFG_RING => control.ring,
        dialogs::ID_CFG_SPOT_FARM => control.farm_on_arrival,
        // ---- ENHANCE ----
        dialogs::ID_CFG_ENHANCE_ON => control.enhance_on,
        // ---- end ENHANCE ----
        // ---- DUNGEON ----
        dialogs::ID_CFG_DUNGEON_ON => control.dungeon_on,
        // ---- end DUNGEON ----
        _ => {
            if let Some(slot) = buff_slot(id) {
                control.buffs[slot]
            } else {
                drop_slot(id)
                    .map(|slot| control.materials[slot])
                    .unwrap_or(false)
            }
        }
    }
}

/// The buff slot a checkbox id addresses, if it is one.
fn buff_slot(id: u16) -> Option<usize> {
    let slot = id.checked_sub(dialogs::ID_CFG_BUFF_FIRST)? as usize;
    (slot < BUFF_SLOTS).then_some(slot)
}

/// The material slot a checkbox id addresses, if it is one.
fn drop_slot(id: u16) -> Option<usize> {
    let slot = id.checked_sub(dialogs::ID_CFG_DROP_FIRST)? as usize;
    (slot < MATERIAL_SLOTS).then_some(slot)
}

/// The text a numeric row opens on.
///
/// A position the operator has never chosen defaults to where the character is standing, so the field
/// shows the spot arming would actually use rather than a zero that means nothing.
fn config_number(id: u16, control: &UiControl, live: Option<UiSpot>) -> String {
    match id {
        dialogs::ID_CFG_RADIUS => control.radius.to_string(),
        dialogs::ID_CFG_HP => control.hp_percent.to_string(),
        dialogs::ID_CFG_MP => control.mp_percent.to_string(),
        dialogs::ID_CFG_ZONE_PICK => control.zone_pick.to_string(),
        dialogs::ID_CFG_REVIVE_DELAY => control.revive_delay_seconds.to_string(),
        dialogs::ID_CFG_SPOT_X => control
            .spot
            .or(live)
            .map_or_else(|| "0".to_owned(), |spot| spot.pixel_x.to_string()),
        dialogs::ID_CFG_SPOT_Y => control
            .spot
            .or(live)
            .map_or_else(|| "0".to_owned(), |spot| spot.pixel_y.to_string()),
        _ => String::new(),
    }
}

/// The option index a picker row opens on.
fn config_choice(id: u16, control: &UiControl) -> u8 {
    match id {
        dialogs::ID_CFG_MODE => match control.mode {
            crate::model::UiAutoMode::Off => 0,
            crate::model::UiAutoMode::Stand => 1,
            crate::model::UiAutoMode::Move => 2,
        },
        dialogs::ID_CFG_REVIVE => control.revive,
        dialogs::ID_CFG_RANK => control.item_rank,
        dialogs::ID_CFG_MPHP => control.potion_pickup,
        dialogs::ID_CFG_GOLD => control.gold,
        dialogs::ID_CFG_ZONE_MODE => control.zone_mode,
        dialogs::ID_CFG_NAV_TARGET => control.nav_target,
        // ---- ENHANCE ----
        // The one picker whose index is not its value: the list starts at +1, so the level is one
        // above the index it opens on. `saturating_sub` rather than `-`: the setting is clamped into
        // range before the dialog opens, but a level below the list's first entry must show that
        // entry rather than wrap to 255 and then be refused by the control.
        dialogs::ID_CFG_ENHANCE_MAXLV => control
            .enhance_max_level
            .saturating_sub(crate::model::ENHANCE_LEVEL_MIN),
        dialogs::ID_CFG_ENHANCE_CHARM => control.enhance_charm,
        // ---- end ENHANCE ----
        // ---- DUNGEON ----
        // Both open on the picker index itself, like `nav_target` and `revive`: the index is what the
        // dialog stores and what `port.rs` translates through `DUNGEON_RUN_VALUES` and
        // `DUNGEON_SCHEDULE_VALUES`. Unlike `ENHANCE_MAXLV` there is no arithmetic here, because the
        // setting carries the index rather than a level the list starts above.
        dialogs::ID_CFG_DUNGEON_MAX => control.dungeon_max,
        dialogs::ID_CFG_DUNGEON_SCHED => control.dungeon_schedule,
        // ---- end DUNGEON ----
        _ => 0,
    }
}

/// The read-only line describing where the character is standing.
fn spot_readout(live: Option<UiSpot>) -> String {
    match live {
        // The same condition that refuses arming, said in the dialog rather than only in the status
        // line: without a settled reading there is no map to anchor on.
        None => "chưa vào game".to_owned(),
        Some(spot) if spot.zone < 0 => format!("bản đồ {}", spot.map_id),
        Some(spot) => format!("bản đồ {} — khu {}", spot.map_id, spot.zone),
    }
}

/// Child control placement, grouped so `create_child` stays within a readable argument count.
#[derive(Clone, Copy)]
struct Placement {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl From<Rect> for Placement {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

fn create_child(
    parent: HWND,
    class: *const u16,
    text: &str,
    style: u32,
    at: Placement,
    id: isize,
) -> HWND {
    let caption = to_wide(text);
    let class_name = copy_pcwstr(class);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            caption.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            at.x,
            at.y,
            at.width,
            at.height,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    }
}

fn set_font(control: HWND, font: *mut core::ffi::c_void) {
    if control.is_null() {
        return;
    }
    // SAFETY: `control` is a live child and `font` is a stock font handle.
    unsafe {
        SendMessageW(control, WM_SETFONT, font as usize, 1);
    }
}

/// Runs the dialog's local message loop, draining worker events between messages.
fn pump_dialog(window: HWND, drain: &mut dyn FnMut()) {
    let mut message = MSG {
        hwnd: null_mut(),
        message: 0,
        wParam: 0,
        lParam: 0,
        time: 0,
        pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
    };
    loop {
        let done = DIALOG.with(|dialog| {
            !matches!(
                dialog.borrow().as_ref().map(|state| &state.outcome),
                Some(DialogOutcome::Pending)
            )
        });
        if done {
            break;
        }
        // The same bounded drain the main timer uses, so a modal dialog cannot stall the worker.
        drain();
        // SAFETY: `message` is live for the duration of the loop.
        let received = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if received <= 0 {
            break;
        }
        // SAFETY: forwarding a received message.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // SAFETY: `window` is this thread's live dialog.
    unsafe {
        DestroyWindow(window);
    }
}

unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CTLCOLORDLG => {
            static mut BG_BRUSH: windows_sys::Win32::Graphics::Gdi::HBRUSH = null_mut();
            if unsafe { BG_BRUSH.is_null() } {
                unsafe {
                    BG_BRUSH = CreateSolidBrush(COLOR_WINDOW_BG);
                }
            }
            unsafe { BG_BRUSH as LRESULT }
        }
        WM_CTLCOLORSTATIC => {
            static mut BG_BRUSH: windows_sys::Win32::Graphics::Gdi::HBRUSH = null_mut();
            if unsafe { BG_BRUSH.is_null() } {
                unsafe {
                    BG_BRUSH = CreateSolidBrush(COLOR_WINDOW_BG);
                }
            }
            let hdc = wparam as HDC;
            unsafe {
                SetBkMode(hdc, TRANSPARENT as i32);
                let ctrl_id = GetDlgCtrlID(lparam as HWND);
                if ctrl_id == ID_ERROR as i32 {
                    SetTextColor(hdc, COLOR_TEXT_ERROR);
                } else if ctrl_id == (ID_MESSAGE + 50) as i32 {
                    SetTextColor(hdc, COLOR_TEXT_HEADER);
                } else if ctrl_id == (ID_MESSAGE + 51) as i32 {
                    SetTextColor(hdc, COLOR_TEXT_MUTED);
                } else {
                    SetTextColor(hdc, COLOR_TEXT_SLATE);
                }
                BG_BRUSH as LRESULT
            }
        }
        WM_CTLCOLOREDIT => {
            static mut WHITE_BRUSH: windows_sys::Win32::Graphics::Gdi::HBRUSH = null_mut();
            if unsafe { WHITE_BRUSH.is_null() } {
                unsafe {
                    WHITE_BRUSH = CreateSolidBrush(COLOR_WHITE);
                }
            }
            let hdc = wparam as HDC;
            unsafe {
                SetBkColor(hdc, COLOR_WHITE);
                SetTextColor(hdc, COLOR_TEXT_SLATE);
                WHITE_BRUSH as LRESULT
            }
        }
        WM_COMMAND => {
            let control = (wparam & 0xffff) as u16;
            if control == ID_CANCEL {
                finish(DialogOutcome::Cancelled);
                return 0;
            }
            if control == ID_OK {
                submit(window);
                return 0;
            }
            if control == ID_SHOW_PASSWORD {
                apply_password_reveal(window);
                return 0;
            }
            // Switching the farm mode acts at once: the operator moved a control that means "start
            // farming", and making them press Save afterwards is a second step for a decision already
            // made. ĐI MAP stays a button because walking is a distinct request, not a setting.
            if matches!(control, dialogs::ID_CFG_MODE | dialogs::ID_CFG_SPOT_PICK)
                && (wparam >> 16) as u16 == CBN_SELCHANGE
            {
                let settings = DIALOG.with(|dialog| {
                    let mut borrowed = dialog.borrow_mut();
                    let state = borrowed.as_mut()?;
                    if control == dialogs::ID_CFG_MODE {
                        state.control.mode = read_choice(
                            window,
                            dialogs::ID_CFG_MODE,
                            crate::model::ATTACK_MODE_OPTIONS.len(),
                        )
                        .and_then(UiAutoMode::from_index)
                        .unwrap_or(UiAutoMode::Off);
                    } else if let Some(name) = read_picked_spot_name(window) {
                        state.control.spot_name = name;
                    }
                    Some(state.control.clone())
                });
                if let Some(settings) = settings {
                    apply_now(settings);
                }
                return 0;
            }
            // The Map picker rebuilds the spot list as soon as it changes: a spot list that only
            // catches up when the dialog reopens shows the previous map's spots, which is exactly the
            // list of places the walk cannot reach.
            if control == dialogs::ID_CFG_NAV_TARGET && (wparam >> 16) as u16 == CBN_SELCHANGE {
                DIALOG.with(|dialog| {
                    let mut borrowed = dialog.borrow_mut();
                    if let Some(state) = borrowed.as_mut() {
                        state.control.nav_target = read_choice(
                            window,
                            dialogs::ID_CFG_NAV_TARGET,
                            NAV_TARGET_OPTIONS.len(),
                        )
                        .unwrap_or(0);
                        // The chosen spot belonged to the previous map, so it is no longer a choice.
                        state.control.spot_name = String::new();
                        refresh_spot_controls(window, state);
                    }
                });
                return 0;
            }
            if matches!(
                control,
                dialogs::ID_CFG_NAV_GO | dialogs::ID_CFG_SPOT_SAVE | dialogs::ID_CFG_SPOT_CLEAR
            ) {
                act_on_spot_button(window, control);
                return 0;
            }
            if control == dialogs::ID_CFG_TRAVEL_START {
                unsafe {
                    let chk = GetDlgItem(window, dialogs::ID_CFG_TRAVEL as i32);
                    if !chk.is_null() {
                        SendMessageW(chk, BM_SETCHECK, BST_CHECKED as usize, 0);
                    }
                }
                submit(window);
                return 0;
            }
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = lparam as *mut MINMAXINFO;
            if !mmi.is_null() {
                let kind = DIALOG.with(|d| d.borrow().as_ref().map(|s| s.kind));
                if let Some(kind) = kind {
                    let dpi = DIALOG.with(|d| d.borrow().as_ref().map_or(super::metrics::REFERENCE_DPI, |s| s.dpi));
                    let (style, ex_style) = if kind == DialogKind::Config {
                        (WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX, 0)
                    } else {
                        (WS_CAPTION | WS_SYSMENU | WS_THICKFRAME, 0)
                    };
                    let (min_w, min_h) = window_size(kind, dpi, style, ex_style);
                    unsafe {
                        (*mmi).ptMinTrackSize.x = min_w;
                        (*mmi).ptMinTrackSize.y = min_h;
                    }
                    return 0;
                }
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_SIZE => {
            let new_w = (lparam & 0xffff) as i32;
            let new_h = ((lparam >> 16) & 0xffff) as i32;
            let dpi = DIALOG.with(|d| d.borrow().as_ref().map_or(super::metrics::REFERENCE_DPI, |s| s.dpi));
            let btn_w = scale(96, dpi);
            let btn_h = scale(30, dpi);
            let pad = scale(16, dpi);
            let ok_btn = unsafe { GetDlgItem(window, ID_OK as i32) };
            let cancel_btn = unsafe { GetDlgItem(window, ID_CANCEL as i32) };
            if !ok_btn.is_null() && !cancel_btn.is_null() {
                let y = new_h - pad - btn_h;
                let ok_x = new_w - pad - btn_w;
                let cancel_x = ok_x - scale(10, dpi) - btn_w;
                unsafe {
                    SetWindowPos(cancel_btn, null_mut(), cancel_x, y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                    SetWindowPos(ok_btn, null_mut(), ok_x, y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            let err_lbl = unsafe { GetDlgItem(window, ID_ERROR as i32) };
            if !err_lbl.is_null() {
                let y = new_h - pad - btn_h + scale(4, dpi);
                unsafe {
                    SetWindowPos(err_lbl, null_mut(), pad, y, new_w - pad * 2 - btn_w * 2 - scale(20, dpi), btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            0
        }
        WM_CLOSE => {
            // Closing the frame is a cancel, never an implicit submit.
            finish(DialogOutcome::Cancelled);
            0
        }
        WM_DESTROY => 0,
        // SAFETY: forwarding an unhandled message.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Acts on one of the travel column's buttons, without closing the dialog.
///
/// Queued rather than sent here because the dialog owns no port: the shell submits the requests and the
/// settings together when it closes. The picker is rewritten immediately anyway — a button that visibly
/// does nothing until the dialog closes reads as a button that did not work.
fn act_on_spot_button(window: HWND, control: u16) {
    DIALOG.with(|dialog| {
        let mut borrowed = dialog.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return;
        };
        // Every one of these needs a map, and the book is keyed by map: without a reading there is
        // nothing a press could name.
        let Some(live) = state.live else {
            return;
        };
        match control {
            dialogs::ID_CFG_NAV_GO => {
                // Acts NOW, not when the dialog closes. Queued, it did nothing visible until the
                // operator pressed Save, which is what made Save look like the button that walks.
                state.control.nav_target =
                    read_choice(window, dialogs::ID_CFG_NAV_TARGET, NAV_TARGET_OPTIONS.len())
                        .unwrap_or(0);
                let map_id = state.control.nav_map();
                drop(borrowed);
                walk_now(map_id);
                return;
            }
            dialogs::ID_CFG_SPOT_SAVE => {
                let name = String::from_utf16_lossy(&read_text(
                    window,
                    dialogs::ID_CFG_SPOT_NAME as isize,
                ));
                let name = if name.trim().is_empty() {
                    // A save with no name still has to be findable later, so it gets a plain one
                    // rather than being refused: the operator can rename it in the same field.
                    format!("Bãi {},{}", live.pixel_x, live.pixel_y)
                } else {
                    name.trim().to_owned()
                };
                state
                    .saved
                    .retain(|saved| !(saved.spot.map_id == live.map_id && saved.name == name));
                state.saved.push(UiSavedSpot {
                    name: name.clone(),
                    spot: live,
                });
                state.control.spot_name = name.clone();
                // The picker lists the CHOSEN map, and a save records where the character is standing.
                // With another map named the new spot belongs to the live one, so the picker has to
                // point there or the operator sees their save vanish: it was filed on a map the list is
                // not showing. This is why saving appeared to need the dialog reopened.
                state.control.nav_target = crate::model::NAV_TARGET_IDS
                    .iter()
                    .position(|candidate| *candidate == live.map_id)
                    .unwrap_or(0) as u8;
                state
                    .spot_requests
                    .push(SpotRequest::Save { spot: live, name });
            }
            dialogs::ID_CFG_SPOT_CLEAR => {
                let name = state.control.spot_name.clone();
                if name.is_empty() {
                    return;
                }
                state
                    .saved
                    .retain(|saved| !(saved.spot.map_id == live.map_id && saved.name == name));
                state.control.spot_name = String::new();
                state.spot_requests.push(SpotRequest::Clear {
                    map_id: live.map_id,
                    name,
                });
            }
            _ => return,
        }
        refresh_spot_controls(window, state);
    });
}

/// The callback the shell installs so ĐI MAP can act while the dialog is open.
///
/// A dialog cannot reach the account port: it owns no state beyond its own controls, and passing the
/// whole shell in would let a modal window mutate anything. One closure is the smallest opening that
/// lets "go now" mean now.
type WalkAction = Box<dyn Fn(Option<u16>)>;
/// Applies the settings the dialog currently holds, without waiting for it to close.
type ApplyAction = Box<dyn Fn(UiControl)>;

thread_local! {
    static ACTIONS: RefCell<Option<(WalkAction, ApplyAction)>> = const { RefCell::new(None) };
}

/// Lends the dialog its two immediate actions, for as long as it is open.
pub(super) fn install_actions(walk: WalkAction, apply: ApplyAction) {
    ACTIONS.with(|actions| *actions.borrow_mut() = Some((walk, apply)));
}

/// Takes them back, so a closed dialog cannot act through a stale closure.
pub(super) fn clear_actions() {
    ACTIONS.with(|actions| *actions.borrow_mut() = None);
}

fn walk_now(map_id: Option<u16>) {
    ACTIONS.with(|actions| {
        if let Some((walk, _)) = actions.borrow().as_ref() {
            walk(map_id);
        }
    });
}

/// Writes the settings now, so switching the farm mode acts without a trip through Save.
fn apply_now(control: UiControl) {
    ACTIONS.with(|actions| {
        if let Some((_, apply)) = actions.borrow().as_ref() {
            apply(control);
        }
    });
}

/// The map whose spots the picker should list: the one named in the Map picker, else the one underfoot.
///
/// Named first because that is the map the walk is aimed at. Falling back to the live map keeps
/// "farm here" working without touching the Map picker at all.
fn chosen_map(state: &DialogState) -> Option<u16> {
    chosen_map_of(&state.control, state.live)
}

fn chosen_map_of(control: &UiControl, live: Option<UiSpot>) -> Option<u16> {
    control.nav_map().or_else(|| live.map(|spot| spot.map_id))
}

/// The name the spot picker has selected, or `None` when nothing is selected.
fn read_picked_spot_name(window: HWND) -> Option<String> {
    // SAFETY: the id belongs to a live child of this dialog; CB_ERR is -1 and means no selection.
    unsafe {
        let picker = GetDlgItem(window, dialogs::ID_CFG_SPOT_PICK as i32);
        if picker.is_null() {
            return None;
        }
        let selected = SendMessageW(picker, CB_GETCURSEL, 0, 0);
        if selected < 0 {
            return None;
        }
        let length = SendMessageW(picker, CB_GETLBTEXTLEN, selected as usize, 0);
        if length <= 0 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        SendMessageW(
            picker,
            CB_GETLBTEXT,
            selected as usize,
            buffer.as_mut_ptr() as isize,
        );
        buffer.truncate(length as usize);
        Some(String::from_utf16_lossy(&buffer))
    }
}

/// Rewrites the spot picker and the name field from the dialog's own copy of the book.
fn refresh_spot_controls(window: HWND, state: &DialogState) {
    // The map the picker names, not the one underfoot: the operator chooses where to go and then which
    // of THAT map's spots to farm. Filtering by the live map listed another map's spots, which is a
    // list of places the walk would never reach.
    let Some(map_id) = chosen_map(state) else {
        return;
    };
    let book = UiSpotBook {
        entries: state.saved.clone(),
    };
    let names: Vec<&str> = book
        .for_map(map_id)
        .into_iter()
        .map(|saved| saved.name.as_str())
        .collect();
    // SAFETY: both ids belong to live children of this dialog.
    unsafe {
        let picker = GetDlgItem(window, dialogs::ID_CFG_SPOT_PICK as i32);
        if !picker.is_null() {
            SendMessageW(picker, CB_RESETCONTENT, 0, 0);
            let mut selected = -1isize;
            for (index, name) in names.iter().enumerate() {
                let text = to_wide(name);
                SendMessageW(picker, CB_ADDSTRING, 0, text.as_ptr() as isize);
                if *name == state.control.spot_name {
                    selected = index as isize;
                }
            }
            SendMessageW(picker, CB_SETCURSEL, selected as usize, 0);
        }
        let field = GetDlgItem(window, dialogs::ID_CFG_SPOT_NAME as i32);
        if !field.is_null() {
            let text = to_wide(&state.control.spot_name);
            SetWindowTextW(field, text.as_ptr());
        }
        // A save can move the Map picker to the map it recorded on, so that control is rewritten too:
        // leaving it showing the previous map would put the two pickers into a state the book cannot
        // produce, with a spot list that belongs to neither.
        let map = GetDlgItem(window, dialogs::ID_CFG_NAV_TARGET as i32);
        if !map.is_null() {
            SendMessageW(map, CB_SETCURSEL, state.control.nav_target as usize, 0);
        }
    }
}

/// Applies the show/hide toggle to the password control.
///
/// `EM_SETPASSWORDCHAR` with `0` reveals the text and a non-zero character re-masks it; the value is
/// read back from the checkbox rather than tracked separately, so the two can never disagree.
fn apply_password_reveal(window: HWND) {
    // SAFETY: both ids belong to live children of this dialog.
    unsafe {
        let toggle = GetDlgItem(window, ID_SHOW_PASSWORD as i32);
        let password = GetDlgItem(window, ID_PASSWORD as i32);
        if toggle.is_null() || password.is_null() {
            return;
        }
        let revealed = SendMessageW(toggle, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        let mask = if revealed { 0 } else { PASSWORD_MASK as usize };
        SendMessageW(password, EM_SETPASSWORDCHAR, mask, 0);
        // The control keeps its own buffer; a repaint is required for the new mask to show.
        InvalidateRect(password, null(), 1);
    }
}

fn finish(outcome: DialogOutcome) {
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.outcome = outcome;
        }
    });
}

/// Validates the typed values locally and either records them or shows a bounded error.
fn submit(window: HWND) {
    let kind = DIALOG.with(|dialog| dialog.borrow().as_ref().map(|state| state.kind));
    let Some(kind) = kind else {
        return;
    };

    if kind == DialogKind::Config {
        match read_config(window) {
            Ok(control) => DIALOG.with(|dialog| {
                if let Some(state) = dialog.borrow_mut().as_mut() {
                    state.control = control;
                    state.outcome = DialogOutcome::Submitted;
                }
            }),
            Err(error) => show_error(window, error),
        }
        return;
    }

    if kind == DialogKind::Server {
        // Reads back which radio button is checked. The control id encodes the index, so no side
        // table can fall out of step with the buttons.
        let mut chosen = None;
        for index in 0..zeus_core::SERVER_NAMES.len() {
            // SAFETY: every id belongs to a live child of this dialog.
            let checked = unsafe {
                let control = GetDlgItem(window, dialogs::ID_SERVER_FIRST as i32 + index as i32);
                !control.is_null()
                    && SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
            };
            if checked {
                chosen = Some(index as u8);
                break;
            }
        }
        // A group always has exactly one checked button once one was preselected, so `None` cannot
        // normally happen; treating it as a cancel is safer than storing an arbitrary world.
        if let Some(server_index) = chosen {
            DIALOG.with(|dialog| {
                if let Some(state) = dialog.borrow_mut().as_mut() {
                    state.server_index = server_index;
                    state.outcome = DialogOutcome::Submitted;
                }
            });
        } else {
            finish(DialogOutcome::Cancelled);
        }
        return;
    }

    if kind == DialogKind::Delete {
        let typed = read_text(window, ID_CONFIRM_TEXT as isize);
        let typed = String::from_utf16_lossy(&typed);
        match dialogs::validate_delete_confirmation(&typed) {
            Ok(()) => finish(DialogOutcome::Submitted),
            Err(error) => show_error(window, error),
        }
        return;
    }

    let username = String::from_utf16_lossy(&read_text(window, ID_USERNAME as isize));
    if let Err(error) = dialogs::validate_username(&username) {
        show_error(window, error);
        return;
    }
    let mut password = read_text(window, ID_PASSWORD as isize);
    // Edit may leave the password untouched; Add always requires one.
    let password_required = kind == DialogKind::Add || !password.is_empty();
    if password_required && let Err(error) = dialogs::validate_password(&password) {
        // Clear the rejected value before returning, so it never lingers in this buffer.
        password.fill(0);
        show_error(window, error);
        return;
    }
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.username = username;
            state.password = password;
            state.outcome = DialogOutcome::Submitted;
        }
    });
}

/// Reads every settings control back, refusing the whole submission on the first bad field.
///
/// All or nothing on purpose: the file the client reads is replaced wholesale, so storing the fields
/// that happened to parse would write a settings set the operator never approved.
fn read_config(window: HWND) -> Result<UiControl, FieldError> {
    let (mut control, live) = DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .map_or((UiControl::default(), None), |state| {
                (state.control.clone(), state.live)
            })
    });
    // The chosen spot: the picker when the operator selected one, else whatever they typed. Read here
    // rather than per row because both controls name the same setting, and the picker wins — selecting
    // an existing spot is a choice, while the field is where a NEW name is written.
    control.spot_name = read_picked_spot_name(window).unwrap_or_else(|| {
        String::from_utf16_lossy(&read_text(window, dialogs::ID_CFG_SPOT_NAME as isize))
            .trim()
            .to_owned()
    });
    let mut spot_x = None;
    let mut spot_y = None;
    for row in dialogs::CONFIG_ROWS {
        match row.control {
            // None of these is a value read back from its control: a readout only shows, a button
            // already acted when it was pressed, and the picker and the name field are read together
            // below because a chosen name and a typed one are the same setting.
            ConfigControl::Readout
            | ConfigControl::Action
            | ConfigControl::SpotChoice
            | ConfigControl::Text => {}
            // The template id the operator picked, read from the entry's own item data rather than
            // from its position: the list is built from the bag, so index 2 is a different mount
            // once one is picked up. A missing selection keeps the current setting.
            ConfigControl::MountChoice => {
                if let Some(mount_id) = read_choice_data(window, row.id) {
                    control.mount_template_id = mount_id;
                }
            }
            ConfigControl::Check => {
                let checked = read_check(window, row.id);
                match row.id {
                    dialogs::ID_CFG_HP_ON => control.hp_on = checked,
                    dialogs::ID_CFG_MP_ON => control.mp_on = checked,
                    dialogs::ID_CFG_MOUNT => control.mount = checked,
                    dialogs::ID_CFG_MEDAL => control.medal_dialog = checked,
                    dialogs::ID_CFG_DROPS_ON => control.materials_managed = checked,
                    dialogs::ID_CFG_REVIVE_ON => control.revive_on = checked,
                    dialogs::ID_CFG_RING => control.ring = checked,
                    dialogs::ID_CFG_SPOT_FARM => control.farm_on_arrival = checked,
                    // ---- ENHANCE ----
                    dialogs::ID_CFG_ENHANCE_ON => control.enhance_on = checked,
                    // ---- end ENHANCE ----
                    // ---- DUNGEON ----
                    dialogs::ID_CFG_DUNGEON_ON => control.dungeon_on = checked,
                    // ---- end DUNGEON ----
                    _ => {
                        if let Some(slot) = buff_slot(row.id) {
                            control.buffs[slot] = checked;
                        } else if let Some(slot) = drop_slot(row.id) {
                            control.materials[slot] = checked;
                        }
                    }
                }
            }
            ConfigControl::Number { low, high } => {
                let typed = String::from_utf16_lossy(&read_text(window, row.id as isize));
                let value = dialogs::validate_number(&typed, low, high)?;
                match row.id {
                    dialogs::ID_CFG_RADIUS => control.radius = value as u16,
                    dialogs::ID_CFG_HP => control.hp_percent = value as u8,
                    dialogs::ID_CFG_MP => control.mp_percent = value as u8,
                    dialogs::ID_CFG_ZONE_PICK => control.zone_pick = value as u8,
                    dialogs::ID_CFG_REVIVE_DELAY => {
                        control.revive_delay_seconds = value as u16;
                    }
                    dialogs::ID_CFG_SPOT_X => spot_x = Some(value),
                    dialogs::ID_CFG_SPOT_Y => spot_y = Some(value),
                    _ => {}
                }
            }
            ConfigControl::Choice(options) => {
                // A drop-down list always has a selection once one was set, so `None` can only mean
                // the control is missing; keeping the current value beats storing the first option.
                let Some(selected) = read_choice(window, row.id, options.len()) else {
                    continue;
                };
                match row.id {
                    dialogs::ID_CFG_MODE => {
                        control.mode = match selected {
                            1 => crate::model::UiAutoMode::Stand,
                            2 => crate::model::UiAutoMode::Move,
                            _ => crate::model::UiAutoMode::Off,
                        }
                    }
                    dialogs::ID_CFG_REVIVE => control.revive = selected,
                    dialogs::ID_CFG_RANK => control.item_rank = selected,
                    dialogs::ID_CFG_MPHP => control.potion_pickup = selected,
                    dialogs::ID_CFG_GOLD => control.gold = selected,
                    dialogs::ID_CFG_ZONE_MODE => control.zone_mode = selected,
                    dialogs::ID_CFG_NAV_TARGET => control.nav_target = selected,
                    // ---- ENHANCE ----
                    // Back to a level from the index it was opened on. `read_choice` already bounded
                    // `selected` to the option count, so this cannot exceed the engine's ceiling, and
                    // `clamped` re-checks it on the way out.
                    dialogs::ID_CFG_ENHANCE_MAXLV => {
                        control.enhance_max_level =
                            selected.saturating_add(crate::model::ENHANCE_LEVEL_MIN)
                    }
                    dialogs::ID_CFG_ENHANCE_CHARM => control.enhance_charm = selected,
                    // ---- end ENHANCE ----
                    // ---- DUNGEON ----
                    dialogs::ID_CFG_DUNGEON_MAX => control.dungeon_max = selected,
                    dialogs::ID_CFG_DUNGEON_SCHED => control.dungeon_schedule = selected,
                    // ---- end DUNGEON ----
                    _ => {}
                }
            }
        }
    }
    // The map and the zone are never typed: the mod can only work the map its character stands on, so
    // they come from the reading and the operator only chooses where within it.
    if let (Some(pixel_x), Some(pixel_y)) = (spot_x, spot_y) {
        let anchor = control.spot.or(live);
        control.spot = anchor.map(|spot| UiSpot {
            pixel_x,
            pixel_y,
            ..spot
        });
    }
    Ok(control.clamped())
}

/// Whether one checkbox is checked.
fn read_check(window: HWND, id: u16) -> bool {
    // SAFETY: `window` is the live dialog; a missing child yields null and reads as unchecked.
    unsafe {
        let control = GetDlgItem(window, id as i32);
        !control.is_null() && SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
    }
}

/// The selected index of one drop-down list, or `None` when it has no selection.
fn read_choice(window: HWND, id: u16, options: usize) -> Option<u8> {
    // SAFETY: `window` is the live dialog; a missing child yields null.
    let selected = unsafe {
        let control = GetDlgItem(window, id as i32);
        if control.is_null() {
            return None;
        }
        SendMessageW(control, CB_GETCURSEL, 0, 0)
    };
    // CB_ERR is -1. An index past the options it was filled with cannot be trusted either.
    let selected = usize::try_from(selected).ok()?;
    (selected < options).then_some(selected as u8)
}

/// The item data behind a drop-down's selection, for lists whose options are not a fixed table.
///
/// Used by the mount picker: its entries come from the running client's bag, so what identifies a
/// choice is the template id stored with it, never the row it happens to sit on.
fn read_choice_data(window: HWND, id: u16) -> Option<u16> {
    // SAFETY: `window` is the live dialog; a missing child yields null.
    let data = unsafe {
        let control = GetDlgItem(window, id as i32);
        if control.is_null() {
            return None;
        }
        let selected = SendMessageW(control, CB_GETCURSEL, 0, 0);
        if selected < 0 {
            return None; // CB_ERR: nothing selected
        }
        SendMessageW(control, CB_GETITEMDATA, selected as usize, 0)
    };
    u16::try_from(data).ok()
}

fn show_error(window: HWND, error: FieldError) {
    let text = to_wide(error.label());
    // SAFETY: the label is a live child and the buffer outlives the call.
    unsafe {
        let control =
            windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(window, ID_ERROR as i32);
        if !control.is_null() {
            SetWindowTextW(control, text.as_ptr());
        }
    }
}

/// Reads one control's text as UTF-16 without assuming it is valid Unicode scalar text.
fn read_text(window: HWND, id: isize) -> Vec<u16> {
    // SAFETY: `window` is the live dialog; a missing child yields null and an empty result.
    unsafe {
        let control = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(window, id as i32);
        if control.is_null() {
            return Vec::new();
        }
        let length = GetWindowTextLengthW(control);
        if length <= 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(copied.max(0) as usize);
        buffer
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_pcwstr(value: *const u16) -> Vec<u16> {
    let mut buffer = Vec::new();
    let mut cursor = value;
    // SAFETY: `value` is a static NUL-terminated wide string from windows-sys.
    unsafe {
        while *cursor != 0 {
            buffer.push(*cursor);
            cursor = cursor.add(1);
        }
    }
    buffer.push(0);
    buffer
}

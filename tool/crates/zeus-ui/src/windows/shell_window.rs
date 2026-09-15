//! Top-level window, account table, toolbar, status line, and the message loop.
//!
//! The main thread only creates controls, drains bounded event batches on a timer, and submits
//! commands. It performs no database, crypto, runtime, process, sleep, or input work.

#![cfg(windows)]

use std::cell::{Cell, RefCell};
use std::num::NonZeroU64;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CreateFontIndirectW, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject,
    GetDC, GetStockObject, GetTextExtentPoint32W, HBRUSH, HFONT, ReleaseDC, SelectObject,
    SetBkColor, SetTextColor, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    LVCFMT_LEFT, LVCOLUMNW, LVIF_PARAM, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED,
    LVIS_STATEIMAGEMASK, LVITEMW, LVM_DELETEALLITEMS, LVM_GETITEMCOUNT, LVM_GETITEMSTATE,
    LVM_GETITEMW, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETCOLUMNWIDTH,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVN_ITEMCHANGED,
    LVNI_SELECTED, LVS_EX_CHECKBOXES, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_REPORT,
    LVS_SHOWSELALWAYS, NMHDR, WC_BUTTONW, WC_LISTVIEWW, WC_STATICW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetDlgItem, GetMessageW, ICON_BIG, ICON_SMALL, IDC_ARROW, IDYES, KillTimer, LoadCursorW, MB_ICONWARNING, MB_YESNO,
    MINMAXINFO, MSG, MessageBoxW, NONCLIENTMETRICSW, PostQuitMessage, RegisterClassExW,
    SPI_GETNONCLIENTMETRICS, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_GETMINMAXINFO, WM_NOTIFY, WM_SETFONT, WM_SETICON, WM_SIZE,
    WM_TIMER, WNDCLASSEXW, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};


use super::layout::PortableLayout;
use super::metrics::{self, ShellMetrics};
use super::{
    EVENT_TIMER_ID, EVENT_TIMER_MS, StartupFailure, WINDOW_CLASS, WINDOW_TITLE, controls,
    dialog_window, icons, initialize_process_dpi, player_view, register_common_controls,
    show_startup_failure,
};
use crate::app::{AccountApp, ClosePhase, ClosePrompt};
use crate::model::{RowKey, UiBootFailure, UiRowAction};
use crate::port::WorkerAccountPort;

/// Child control id of the account table.
const ID_TABLE: isize = 0x3001;
/// Child control id of the status line.
const ID_STATUS: isize = 0x3002;
/// Child control id of the character panel.
const ID_PLAYER: isize = 0x3003;
/// Child control id of the auto config summary panel.
const ID_PLAYER_CONFIG: isize = 0x3004;
/// Command rows above the table: the single unified modern toolbar.
const COMMAND_ROWS: usize = 1;
/// `SS_LEFT`: left-aligned static text that word-wraps and breaks on CRLF.
///
/// Spelled out here rather than imported: windows-sys keeps it behind a feature that would pull in the
/// whole system-services module for a constant whose value is zero, and naming it is what makes the
/// panel's reliance on CRLF line breaks legible.
const SS_LEFT: u32 = 0;
/// `SS_NOPREFIX`: prevents '&' from being treated as an accelerator mnemonic.
const SS_NOPREFIX: u32 = 0x0080;
/// `INDEXTOSTATEIMAGEMASK(2)`: the checked state image of a list view checkbox.
const INDEX_TO_CHECKED_STATE_IMAGE: u32 = 2 << 12;
/// Timer ticks between character polls.
///
/// The client rewrites its reading about once a second, so polling every 50 ms tick would re-read the
/// same file twenty times over. One second matches the writer and keeps the panel visibly live.
const PLAYER_POLL_TICKS: u32 = 1_000 / EVENT_TIMER_MS;

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Pure native Win32 Light Mode: clean, high contrast, zero CPU overhead, elegant Windows look.
const COLOR_WINDOW_BG: u32 = rgb(240, 242, 245);  // #F0F2F5 Clean soft off-white background
const COLOR_TABLE_BG: u32 = rgb(255, 255, 255);   // #FFFFFF Crisp pure white table
const COLOR_TABLE_TEXT: u32 = rgb(17, 24, 39);     // #111827 Deep charcoal text (highest contrast & readability)
const COLOR_PANEL_BG: u32 = rgb(255, 255, 255);   // #FFFFFF Crisp white card panel
const COLOR_PANEL_TEXT: u32 = rgb(15, 23, 42);    // #0F172A Deep slate text
const COLOR_STATUS_BG: u32 = rgb(226, 232, 240);  // #E2E8F0 Soft grey status bar
const COLOR_STATUS_TEXT: u32 = rgb(15, 23, 42);   // #0F172A Deep slate text

/// Owned UI state reachable from the window procedure.
struct ShellState {
    app: AccountApp<WorkerAccountPort>,
    table: HWND,
    status: HWND,
    /// The contextual row button, whose caption follows the selected row's status.
    row_action: HWND,
    /// The character panel, which follows the highlighted row rather than the checked ones.
    player: HWND,
    /// The auto config summary panel.
    player_config: HWND,
    /// The shell-owned UI font, rebuilt on every DPI change and deleted on destroy.
    font: HFONT,
    /// DPI the current layout was computed for.
    dpi: u32,
    /// Ticks since the last character poll.
    player_poll_ticks: u32,
    bg_brush: HBRUSH,
    panel_brush: HBRUSH,
    status_brush: HBRUSH,
}

impl Drop for ShellState {
    fn drop(&mut self) {
        if !self.bg_brush.is_null() {
            unsafe { DeleteObject(self.bg_brush as _) };
        }
        if !self.panel_brush.is_null() {
            unsafe { DeleteObject(self.panel_brush as _) };
        }
        if !self.status_brush.is_null() {
            unsafe { DeleteObject(self.status_brush as _) };
        }
    }
}

thread_local! {
    /// The window procedure runs on this thread only, so thread-local ownership is sufficient and
    /// avoids a lock on the UI path.
    static SHELL: RefCell<Option<ShellState>> = const { RefCell::new(None) };
}

thread_local! {
    /// Set while `repaint_table` drives the list view. Each insert and the bulk delete send
    /// `LVN_ITEMCHANGED` back into `window_proc` synchronously, and that handler borrows `SHELL`, so
    /// the notification is ignored for the duration of a repaint. Every caller refreshes the
    /// contextual caption after the repaint, so no selection state is lost.
    static REPAINTING: Cell<bool> = const { Cell::new(false) };
}

/// Runs the shell, returning the process exit code.
pub fn run() -> u8 {
    if !initialize_process_dpi() {
        show_startup_failure(StartupFailure::Shell);
        return StartupFailure::Shell.exit_code();
    }
    if !register_common_controls() {
        show_startup_failure(StartupFailure::Shell);
        return StartupFailure::Shell.exit_code();
    }
    let Some(layout) = PortableLayout::from_current_executable() else {
        show_startup_failure(StartupFailure::DataRoot);
        return StartupFailure::DataRoot.exit_code();
    };

    // All portable boot work happens on the worker thread; a failure surfaces as a startup event.
    let worker =
        match zeus_core::ManagerWorker::spawn_portable(&layout.data_root, &layout.runtime_root) {
            Ok(worker) => worker,
            Err(_) => {
                show_startup_failure(StartupFailure::Worker);
                return StartupFailure::Worker.exit_code();
            }
        };
    let app = AccountApp::new(WorkerAccountPort::new(worker));

    if !register_window_class() {
        show_startup_failure(StartupFailure::Shell);
        return StartupFailure::Shell.exit_code();
    }
    let window = create_main_window(app);
    if window.is_null() {
        show_startup_failure(StartupFailure::Shell);
        return StartupFailure::Shell.exit_code();
    }

    pump_messages();
    0
}

fn register_window_class() -> bool {
    let class_name = to_wide(WINDOW_CLASS);
    let icon_big = crate::windows::icons::load_app_icon(32);
    let icon_sm = crate::windows::icons::load_app_icon(16);
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        // SAFETY: passing null requests the handle of the current process image.
        hInstance: unsafe { GetModuleHandleW(null()) },
        hIcon: icon_big,
        // SAFETY: `IDC_ARROW` is a system cursor id, so the returned handle needs no cleanup.
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        // Modern dark slate background brush
        hbrBackground: unsafe { CreateSolidBrush(COLOR_WINDOW_BG) },
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: icon_sm,
    };
    // SAFETY: `class` is fully initialized and its string outlives the call.
    unsafe { RegisterClassExW(&class) != 0 }
}

fn create_main_window(app: AccountApp<WorkerAccountPort>) -> HWND {
    let bg_brush = unsafe { CreateSolidBrush(COLOR_WINDOW_BG) };
    let panel_brush = unsafe { CreateSolidBrush(COLOR_PANEL_BG) };
    let status_brush = unsafe { CreateSolidBrush(COLOR_STATUS_BG) };
    SHELL.with(|shell| {
        *shell.borrow_mut() = Some(ShellState {
            app,
            table: null_mut(),
            status: null_mut(),
            row_action: null_mut(),
            player: null_mut(),
            player_config: null_mut(),
            font: null_mut(),
            dpi: metrics::REFERENCE_DPI,
            player_poll_ticks: 0,
            bg_brush,
            panel_brush,
            status_brush,
        });
    });
    let class_name = to_wide(WINDOW_CLASS);
    let title = to_wide(WINDOW_TITLE);
    // Opening size, in reference pixels. Comfortably above the minimum client size so the table keeps
    // a usable width beside the character panel; `WM_GETMINMAXINFO` clamps any later resize.
    let (initial_width, initial_height) = (1_200, 680);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            initial_width,
            initial_height,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null_mut(),
        )
    };
    if !window.is_null() {
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
        // Shown immediately: spec section 6 requires the boot state to be visible as `Đang khởi động`

        // with every action disabled, not hidden behind a modal box.
        // SAFETY: `window` is a live top-level window this thread owns.
        unsafe {
            ShowWindow(window, SW_SHOW);
            UpdateWindow(window);
            SetForegroundWindow(window);
        }
    }
    window
}

fn pump_messages() {
    let mut message = MSG {
        hwnd: null_mut(),
        message: 0,
        wParam: 0,
        lParam: 0,
        time: 0,
        pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
    };
    // SAFETY: `message` is a live, fully initialized MSG for the duration of the loop.
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            let table = create_table(window);
            let status = create_status_line(window);
            let player = create_player_panel(window, ID_PLAYER);
            let player_config = create_player_panel(window, ID_PLAYER_CONFIG);
            let row_action = create_command_rows(window);
            // SAFETY: `window` is the window being created, so its DPI is queryable.
            let dpi = unsafe { GetDpiForWindow(window) };
            let font = create_shell_font(dpi);
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    state.table = table;
                    state.status = status;
                    state.player = player;
                    state.player_config = player_config;
                    state.row_action = row_action;
                    state.font = font;
                    state.dpi = dpi;
                }
            });
            apply_font(window, font);
            resize_to_client(window);
            // Paint the boot banner and disable every action before the first timer tick.
            refresh_status_line();
            refresh_player_panel();
            refresh_command_availability(window);
            // SAFETY: `window` is the window being created and the timer id is process-unique.
            unsafe {
                SetTimer(window, EVENT_TIMER_ID, EVENT_TIMER_MS, None);
            }
            0
        }
        WM_TIMER => {
            drain_worker_events(window);
            0
        }
        WM_COMMAND => {
            handle_command(window, (wparam & 0xffff) as u16);
            0
        }
        WM_NOTIFY => {
            // SAFETY: for WM_NOTIFY, lparam is a valid NMHDR for the lifetime of the message.
            let code = unsafe { (*(lparam as *const NMHDR)).code };
            // A checkbox toggle changes which action the contextual button offers, and a highlight
            // change re-points the character panel. Notifications that the repaint itself generates
            // are ignored, because the caller refreshes both once the repaint completes.
            if code == LVN_ITEMCHANGED && !REPAINTING.get() {
                refresh_row_action_caption(window);
                focus_player_row(window);
            }
            0
        }
        WM_SIZE => {
            resize_shell(window, lparam);
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as windows_sys::Win32::Graphics::Gdi::HDC;
            let control = lparam as HWND;
            let brush = SHELL.with(|shell| {
                let borrowed = shell.borrow();
                let Some(state) = borrowed.as_ref() else {
                    return null_mut();
                };
                if control == state.player || control == state.player_config {
                    unsafe {
                        SetTextColor(hdc, COLOR_PANEL_TEXT);
                        SetBkColor(hdc, COLOR_PANEL_BG);
                    }
                    state.panel_brush
                } else if control == state.status {
                    unsafe {
                        SetTextColor(hdc, COLOR_STATUS_TEXT);
                        SetBkColor(hdc, COLOR_STATUS_BG);
                    }
                    state.status_brush
                } else {
                    unsafe {
                        SetTextColor(hdc, COLOR_PANEL_TEXT);
                        SetBkColor(hdc, COLOR_WINDOW_BG);
                    }
                    state.bg_brush
                }
            });
            if !brush.is_null() {
                brush as isize
            } else {
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
        WM_DPICHANGED => {
            // The new DPI arrives in wparam and the suggested window rect in lparam; both are applied
            // before the children are re-measured, so the font matches the frame Windows just chose.
            let dpi = (wparam & 0xffff) as u32;
            let font = create_shell_font(dpi);
            let previous = SHELL.with(|shell| {
                shell.borrow_mut().as_mut().map(|state| {
                    let previous = state.font;
                    state.font = font;
                    state.dpi = dpi;
                    previous
                })
            });
            // SAFETY: for WM_DPICHANGED, lparam points to a valid RECT for the message's lifetime.
            unsafe {
                let suggested = *(lparam as *const RECT);
                SetWindowPos(
                    window,
                    null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            apply_font(window, font);
            resize_to_client(window);
            if let Some(previous) = previous {
                delete_shell_font(previous);
            }
            0
        }
        WM_GETMINMAXINFO => {
            // Clamped before the shell state exists, so the reference minimum is used during creation.
            let dpi = SHELL.with(|shell| shell.borrow().as_ref().map_or(0, |state| state.dpi));
            let (width, height) = ShellMetrics::new(dpi).min_client_size();
            // SAFETY: for WM_GETMINMAXINFO, lparam points to a writable MINMAXINFO.
            unsafe {
                (*(lparam as *mut MINMAXINFO)).ptMinTrackSize.x = width;
                (*(lparam as *mut MINMAXINFO)).ptMinTrackSize.y = height;
            }
            0
        }
        WM_CLOSE => {
            handle_close(window);
            0
        }
        WM_DESTROY => {
            // SAFETY: the timer was created for this window.
            unsafe {
                KillTimer(window, EVENT_TIMER_ID);
            }
            let font = SHELL.with(|shell| {
                let font = shell
                    .borrow()
                    .as_ref()
                    .map_or(null_mut(), |state| state.font);
                // Dropping the app closes the worker, which releases the Core instance lock.
                *shell.borrow_mut() = None;
                font
            });
            delete_shell_font(font);
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: forwarding an unhandled message to the default procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Creates the redacted status line.
///
/// It renders only bounded Vietnamese copy: the last error class, the close progress, or the account
/// count. No path, OS code, or backend identifier ever reaches it.
fn create_status_line(parent: HWND) -> HWND {
    let class = to_wide_from_pcwstr(WC_STATICW);
    let empty = to_wide("");
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX,
            0,
            0,
            0,
            0,
            parent,
            ID_STATUS as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    }
}

/// Builds the shell font for one DPI from the system's message font.
///
/// `DEFAULT_GUI_FONT` is a fixed 96-DPI bitmap face, so it renders undersized and jagged on a scaled
/// display. The message font Windows itself uses for dialogs is queried at the window's DPI instead,
/// and the caller owns the returned handle.
fn create_shell_font(dpi: u32) -> HFONT {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is correctly sized and initialized, and the call writes only into it.
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
        // SAFETY: a stock font handle needs no cleanup; `delete_shell_font` skips stock handles.
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

/// Measures one caption in the shell font, in device pixels.
fn measure_caption(window: HWND, font: HFONT, caption: &str) -> i32 {
    let text = to_wide(caption);
    let length = text.len().saturating_sub(1) as i32;
    // SAFETY: `window` is live, `font` is a valid font handle, and the DC is released on every path.
    unsafe {
        let dc = GetDC(window);
        if dc.is_null() {
            return 0;
        }
        let previous = SelectObject(dc, font as _);
        let mut size = SIZE { cx: 0, cy: 0 };
        let measured = GetTextExtentPoint32W(dc, text.as_ptr(), length, &mut size) != 0;
        SelectObject(dc, previous);
        ReleaseDC(window, dc);
        if measured { size.cx } else { 0 }
    }
}

/// Applies `font` to every child control of the shell.
fn apply_font(window: HWND, font: HFONT) {
    for id in [ID_TABLE, ID_STATUS, ID_PLAYER, ID_PLAYER_CONFIG] {
        // SAFETY: `window` is live; a missing child yields null, which `SendMessageW` rejects.
        unsafe {
            let control = GetDlgItem(window, id as i32);
            if !control.is_null() {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
    }
    for command in icons::TOOLBAR_COMMANDS.iter().chain(icons::ROW_COMMANDS) {
        // SAFETY: as above.
        unsafe {
            let control = GetDlgItem(window, command.id as i32);
            if !control.is_null() {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
    }
}

/// Deletes a shell-owned font, leaving a stock fallback handle alone.
fn delete_shell_font(font: HFONT) {
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

/// Creates a character/config panel.
///
/// A plain static, not an edit control: the panel is read-only prose, and an edit box would take focus
/// in the tab order and invite the operator to type into data the game owns. `SS_LEFT` word-wraps and
/// breaks on the CRLF the projection emits.
fn create_player_panel(parent: HWND, id: isize) -> HWND {
    let class = to_wide_from_pcwstr(WC_STATICW);
    let empty = to_wide("");
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
            0,
            0,
            0,
            0,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    }
}

/// Points the character panel at the highlighted row and repaints it.
///
/// Highlight rather than checkbox: clicking a row is how the operator asks to look at it, while the
/// checkboxes drive batch commands and are often left checked across several rows.
fn focus_player_row(window: HWND) {
    let highlighted = highlighted_row();
    let changed = SHELL.with(|shell| {
        shell
            .borrow_mut()
            .as_mut()
            .is_some_and(|state| state.app.focus_player_row(highlighted))
    });
    if changed {
        // The panel is cleared immediately so the previous account's character is never shown under a
        // newly selected row; the next poll fills it in.
        refresh_player_panel();
        request_player_reading();
        // The settings travel with the row too, so the dialog opens on this account's values rather
        // than on whatever the previously focused one had.
        request_control_reading();
        refresh_auto_caption(window);
    }
    let _ = window;
}

/// Submits one settings read for the focused row, ignoring a refusal.
///
/// A refused read is not reported: the dialog still opens, on the defaults, and the next focus change
/// asks again.
fn request_control_reading() {
    SHELL.with(|shell| {
        if let Some(state) = shell.borrow_mut().as_mut()
            && let Some(row) = state.app.player_row()
        {
            let _ = state.app.observe_control(row);
        }
    });
}

/// Reads the highlighted list view row, mapping it back to its hidden key.
fn highlighted_row() -> Option<RowKey> {
    let table = SHELL.with(|shell| shell.borrow().as_ref().map(|state| state.table));
    let table = table.filter(|table| !table.is_null())?;
    highlighted_row_key(table)
}

/// Submits one character poll for the focused row, ignoring a refusal.
///
/// A refused poll is not reported: it repeats every second, and the panel simply keeps the reading it
/// already has until the next attempt is admitted.
fn request_player_reading() {
    SHELL.with(|shell| {
        if let Some(state) = shell.borrow_mut().as_mut() {
            let _ = state.app.observe_focused_player();
        }
    });
}

/// Repaints the character info and auto configuration panels.
fn refresh_player_panel() {
    SHELL.with(|shell| {
        let borrowed = shell.borrow();
        let Some(state) = borrowed.as_ref() else {
            return;
        };
        let focused_row = state.app.player_row();
        let control = focused_row.map(|row| state.app.control(row));

        if !state.player.is_null() {
            let info_text = player_view::player_info_text(focused_row.is_some(), state.app.player());
            let buffer = to_wide(&info_text);
            // SAFETY: `player` is a live child and the buffer outlives the call.
            unsafe {
                SetWindowTextW(state.player, buffer.as_ptr());
            }
        }

        if !state.player_config.is_null() {
            let config_text = player_view::player_config_text(focused_row.is_some(), control.as_ref());
            let buffer = to_wide(&config_text);
            // SAFETY: `player_config` is a live child and the buffer outlives the call.
            unsafe {
                SetWindowTextW(state.player_config, buffer.as_ptr());
            }
        }
    });
}

/// Retitles the auto control from the mode the panel's row is in.
///
/// Read from the model rather than remembered by the button: a repaint or a row change must not
/// leave the caption claiming a mode that belongs to a different account.
fn refresh_auto_caption(window: HWND) {
    let caption = SHELL.with(|shell| {
        shell
            .borrow()
            .as_ref()
            .map_or("Tự đánh: tắt", |state| match state.app.player_row() {
                Some(row) => state.app.auto_mode(row).label(),
                None => "Tự đánh: tắt",
            })
    });
    let buffer = to_wide(caption);
    // SAFETY: `window` is this thread's live shell window; a missing child yields null, which
    // `SetWindowTextW` rejects harmlessly.
    unsafe {
        let control = GetDlgItem(window, icons::CMD_ROW_AUTO as i32);
        if !control.is_null() {
            SetWindowTextW(control, buffer.as_ptr());
        }
    }
}

#[repr(C)]
#[allow(non_snake_case)]
struct PROCESS_MEMORY_COUNTERS {
    cb: u32,
    PageFaultCount: u32,
    PeakWorkingSetSize: usize,
    WorkingSetSize: usize,
    QuotaPeakPagedPoolUsage: usize,
    QuotaPagedPoolUsage: usize,
    QuotaPeakNonPagedPoolUsage: usize,
    QuotaNonPagedPoolUsage: usize,
    PagefileUsage: usize,
    PeakPagefileUsage: usize,
}

unsafe extern "system" {
    fn K32GetProcessMemoryInfo(
        process: isize,
        counters: *mut PROCESS_MEMORY_COUNTERS,
        cb: u32,
    ) -> i32;
    fn GetCurrentProcess() -> isize;
}

/// Reads the real working-set memory of this process in megabytes.
fn current_process_ram_mb() -> Option<f64> {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle and counters is writable.
    let ok = unsafe {
        let handle = GetCurrentProcess();
        K32GetProcessMemoryInfo(handle, &mut counters, counters.cb)
    };
    if ok != 0 {
        Some(counters.WorkingSetSize as f64 / (1024.0 * 1024.0))
    } else {
        None
    }
}

/// Updates the status line from the current model.
fn refresh_status_line() {
    SHELL.with(|shell| {
        let borrowed = shell.borrow();
        let Some(state) = borrowed.as_ref().filter(|state| !state.status.is_null()) else {
            return;
        };
        let total_acc = state.app.rows().len();
        let running_acc = state.app.rows().iter().filter(|r| r.status.is_active()).count();
        let ram_info = current_process_ram_mb()
            .map(|mb| format!("  |  RAM: {:.1} MB", mb))
            .unwrap_or_default();
        let text = if let Some(failure) = state.app.boot_failure() {
            format!("🔴 Tool chưa sẵn sàng: {}", boot_guidance(failure))
        } else if state.app.model().worker_closed() {
            "🔴 Tool chưa sẵn sàng".to_owned()
        } else if let Some(progress) = state.app.close_phase().label() {
            format!("🟡 {progress}")
        } else if let Some(error) = state.app.model().last_error() {
            format!("⚠️ {}", error.label())
        } else if !state.app.model().worker_ready() {
            "🟡 Đang khởi động...".to_owned()
        } else {
            format!("🟢 Hoạt động: {running_acc}/{total_acc} tài khoản{ram_info}  |  ⚡ Sẵn sàng")
        };
        let buffer = to_wide(&text);
        // SAFETY: `status` is a live child and the buffer outlives the call.
        unsafe {
            SetWindowTextW(state.status, buffer.as_ptr());
        }
    });
}

/// Bounded Vietnamese guidance for a boot failure. No path, OS code, or backend token.
fn boot_guidance(failure: UiBootFailure) -> &'static str {
    match failure {
        UiBootFailure::DataRoot => StartupFailure::DataRoot.label(),
        UiBootFailure::PinnedRuntime => StartupFailure::PinnedRuntime.label(),
    }
}

/// Enables or disables every command control from the model's admission state.
///
/// Spec section 6: no partially ready worker exists, so during boot and after a boot failure every
/// action is disabled rather than allowed to fail one by one.
fn refresh_command_availability(window: HWND) {
    let enabled = SHELL.with(|shell| {
        shell.borrow().as_ref().is_some_and(|state| {
            state.app.model().worker_ready() && !state.app.model().worker_closed()
        })
    });
    for command in icons::TOOLBAR_COMMANDS.iter().chain(icons::ROW_COMMANDS) {
        // SAFETY: `window` is this thread's live shell window; a missing child yields null, which
        // `EnableWindow` rejects harmlessly.
        unsafe {
            let control = GetDlgItem(window, command.id as i32);
            if !control.is_null() {
                EnableWindow(control, i32::from(enabled));
            }
        }
    }
}

/// Creates one push button per command, in two rows: batch commands, then per-row commands.
///
/// Plain buttons rather than a toolbar control: they are keyboard reachable, expose their name to
/// assistive tech directly, and need no image list. Returns the contextual row button, whose caption
/// is re-derived from the selected row.
fn create_command_rows(parent: HWND) -> HWND {
    let mut contextual = null_mut();
    for command in icons::TOOLBAR_COMMANDS.iter().chain(icons::ROW_COMMANDS) {
        let caption = to_wide(command.tooltip);
        let class = to_wide_from_pcwstr(WC_BUTTONW);
        // Position and size are applied by `apply_layout`, which owns all geometry.
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let button = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                caption.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                0,
                0,
                0,
                0,
                parent,
                command.id as _,
                GetModuleHandleW(null()),
                null_mut(),
            )
        };
        if !button.is_null() && command.id == icons::CMD_ROW_ACTION {
            contextual = button;
        }
    }
    contextual
}

/// Places every child control for the window's current DPI and client size.
///
/// One button width is used for the whole grid, derived from the widest caption, so the two command
/// rows align and no caption is clipped at any scaling.
fn apply_layout(window: HWND, client_width: i32, client_height: i32) {
    let (font, table, status, player, player_config) = SHELL.with(|shell| {
        shell
            .borrow()
            .as_ref()
            .map_or((null_mut(), null_mut(), null_mut(), null_mut(), null_mut()), |state| {
                (state.font, state.table, state.status, state.player, state.player_config)
            })
    });
    if font.is_null() {
        return;
    }
    let dpi = SHELL.with(|shell| shell.borrow().as_ref().map_or(0, |state| state.dpi));
    let layout = ShellMetrics::new(dpi);
    let widest = icons::TOOLBAR_COMMANDS
        .iter()
        .chain(icons::ROW_COMMANDS)
        .map(|command| measure_caption(window, font, command.tooltip))
        .max()
        .unwrap_or(0);
    // One width for the whole grid so the two rows align, capped by the longer row so its last
    // button cannot land past the client edge.
    let columns = icons::TOOLBAR_COMMANDS.len().max(icons::ROW_COMMANDS.len());
    let button_width = layout.button_width(widest, columns, client_width);
    for (row, commands) in [icons::TOOLBAR_COMMANDS, icons::ROW_COMMANDS]
        .into_iter()
        .enumerate()
    {
        for (index, command) in commands.iter().enumerate() {
            let rect = layout.button_rect(row, index, button_width);
            // SAFETY: `window` is live; a missing child yields null, which `SetWindowPos` rejects.
            unsafe {
                let control = GetDlgItem(window, command.id as i32);
                if !control.is_null() {
                    SetWindowPos(
                        control,
                        null_mut(),
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
    if !table.is_null() {
        let rect = layout.table_rect(client_width, client_height, COMMAND_ROWS);
        // SAFETY: the table is a live child of `window`.
        unsafe {
            SetWindowPos(
                table,
                null_mut(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        // Auto-expand table columns proportionally to fill table_rect without horizontal scroll or empty trailing space.
        let check_w = metrics::scale(28, layout.dpi);
        let pad = metrics::scale(8, layout.dpi);
        let avail_w = (rect.width - check_w - pad).max(100);
        let total_base: i32 = 650;
        let w_acc = (avail_w * 200) / total_base;
        let w_svr = (avail_w * 140) / total_base;
        let w_sta = (avail_w * 150) / total_base;
        let w_lst = avail_w - w_acc - w_svr - w_sta;

        send(table, LVM_SETCOLUMNWIDTH, 0, check_w as isize);
        send(table, LVM_SETCOLUMNWIDTH, 1, w_acc as isize);
        send(table, LVM_SETCOLUMNWIDTH, 2, w_svr as isize);
        send(table, LVM_SETCOLUMNWIDTH, 3, w_sta as isize);
        send(table, LVM_SETCOLUMNWIDTH, 4, w_lst as isize);
    }
    if !status.is_null() {
        let rect = layout.status_rect(client_width, client_height);
        // SAFETY: the status line is a live child of `window`.
        unsafe {
            SetWindowPos(
                status,
                null_mut(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    if !player.is_null() {
        let rect = layout.player_info_rect(client_width, client_height, COMMAND_ROWS);
        // SAFETY: the character info card is a live child of `window`.
        unsafe {
            SetWindowPos(
                player,
                null_mut(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    if !player_config.is_null() {
        let rect = layout.player_config_rect(client_width, client_height, COMMAND_ROWS);
        // SAFETY: the auto config summary card is a live child of `window`.
        unsafe {
            SetWindowPos(
                player_config,
                null_mut(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Retitles the contextual row button from the selected row's status.
///
/// With no selection, or a row whose status offers nothing, it falls back to the generic row tooltip
/// so the button never displays a stale action.
fn refresh_row_action_caption(window: HWND) {
    // Selection is read first, because it borrows the shell state too.
    let selected = selected_rows(window);
    SHELL.with(|shell| {
        let borrowed = shell.borrow();
        let Some(state) = borrowed
            .as_ref()
            .filter(|state| !state.row_action.is_null())
        else {
            return;
        };
        let generic = icons::ROW_COMMANDS
            .iter()
            .find(|command| command.id == icons::CMD_ROW_ACTION)
            .map_or("", |command| command.tooltip);
        // Exactly one checked row has an unambiguous action; anything else keeps the generic caption.
        let caption = match selected.as_slice() {
            [row] => icons::row_action_tooltip(state.app.row_action(*row)).unwrap_or(generic),
            _ => generic,
        };
        let buffer = to_wide(caption);
        // SAFETY: `row_action` is a live child and the buffer outlives the call.
        unsafe {
            SetWindowTextW(state.row_action, buffer.as_ptr());
        }
    });
}

/// Routes one toolbar command to the controller.
fn handle_command(window: HWND, command: u16) {
    // The drain closure is handed to every modal dialog so worker progress continues while it is open.
    let mut drain = || drain_worker_events(window);
    match command {
        icons::CMD_ADD => {
            if let Some(submission) = dialog_window::prompt_add(window, &mut drain) {
                submit_import(window, submission);
            }
        }
        icons::CMD_REFRESH => {
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    let _ = state.app.refresh();
                }
            });
        }
        icons::CMD_RUN_SELECTED => {
            let selected = selected_rows(window);
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    let _ = state.app.run_rows(&selected);
                }
            });
        }
        icons::CMD_STOP_SELECTED => {
            let selected = selected_rows(window);
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    let _ = state.app.stop_rows(&selected);
                }
            });
        }
        icons::CMD_DELETE_SELECTED => {
            let selected = selected_rows(window);
            if selected.is_empty() {
                return;
            }
            // Destructive: the operator must type the confirmation word first.
            if dialog_window::confirm_delete(window, &mut drain) {
                SHELL.with(|shell| {
                    if let Some(state) = shell.borrow_mut().as_mut() {
                        let _ = state.app.delete_rows(&selected);
                    }
                });
            }
        }
        icons::CMD_ROW_EDIT => {
            let selected = selected_rows(window);
            let Some(row) = selected.first().copied() else {
                return;
            };
            let current = SHELL.with(|shell| {
                shell
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.app.model().row(row).cloned())
            });
            let Some(current) = current else {
                return;
            };
            if let Some(submission) =
                dialog_window::prompt_edit(window, &current.username, &mut drain)
            {
                SHELL.with(|shell| {
                    if let Some(state) = shell.borrow_mut().as_mut() {
                        // An empty password means "leave the stored one unchanged", which is why a
                        // rename never needs the vault key.
                        let replacement = if submission.password.is_empty() {
                            None
                        } else {
                            Some(submission.password)
                        };
                        let _ = state.app.update(
                            row,
                            current.revision,
                            &submission.username,
                            replacement,
                        );
                    }
                });
            }
        }
        icons::CMD_ROW_SERVER => {
            let selected = selected_rows(window);
            let Some(row) = selected.first().copied() else {
                return;
            };
            let current = SHELL.with(|shell| {
                shell
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.app.model().row(row).cloned())
            });
            let Some(current) = current else {
                return;
            };
            // The picker opens on the account's current world, so confirming without moving the
            // selection is a no-op rather than a silent reset to the first server.
            if let Some(server_index) =
                dialog_window::prompt_server(window, current.server_index, &mut drain)
            {
                SHELL.with(|shell| {
                    if let Some(state) = shell.borrow_mut().as_mut() {
                        let _ = state.app.set_server(row, server_index);
                    }
                });
            }
        }
        icons::CMD_ROW_AUTO => {
            // The panel's row, not a checked one: the spot has to come from the character the
            // operator is actually looking at.
            let Some(row) = SHELL.with(|shell| {
                shell
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.app.player_row())
            }) else {
                return;
            };
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    let _ = state.app.cycle_auto(row);
                }
            });
            refresh_auto_caption(window);
        }
        icons::CMD_ROW_CONFIG => {
            // The panel's row, like the auto control: the settings dialog shows where the character
            // is standing, and that only means something for the row being looked at.
            let Some(row) = SHELL.with(|shell| {
                shell
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.app.player_row())
            }) else {
                return;
            };
            // Opened on what the engine last confirmed, so a value it clamped is visible rather than
            // silently disagreeing with what the client reads.
            let (current, live, saved, mounts) = SHELL.with(|shell| {
                shell.borrow().as_ref().map_or(
                    (
                        crate::model::UiControl::default(),
                        None,
                        Vec::new(),
                        Vec::new(),
                    ),
                    |state| {
                        let live = state.app.live_spot(row);
                        let mut current = state.app.control(row);
                        // The WHOLE book, not one map's worth: the dialog filters it by whichever map
                        // its own picker names, and handing over only the live map's spots left every
                        // other map showing an empty list — the saved spots were there and simply never
                        // reached the control.
                        let saved = state.app.saved_spots();
                        // The engine stores where a spot is, the book stores what it is called, so the
                        // name is recovered rather than carried on the wire twice.
                        if let Some(spot) = current.spot
                            && let Some(name) = state.app.saved_spot_name(spot)
                        {
                            current.spot_name = name;
                        }
                        // The mounts the client last reported carrying, so the picker offers them
                        // by name. This row's reading only: the panel shows one character at a time.
                        let mounts = state
                            .app
                            .player()
                            .filter(|_| state.app.player_row() == Some(row))
                            .map_or_else(Vec::new, |info| info.mounts.clone());
                        (current, live, saved, mounts)
                    },
                )
            });
            // ĐI MAP and the farm switch act while the dialog is open, so both are lent to it as
            // closures. Queued until the dialog closed, they did nothing visible — which made Save look
            // like the button that walks, and like the button that starts farming.
            dialog_window::install_actions(
                Box::new(move |map_id| {
                    SHELL.with(|shell| {
                        if let Some(state) = shell.borrow_mut().as_mut() {
                            let _ = state.app.walk_to_map(row, map_id);
                        }
                    });
                }),
                Box::new(move |settings| {
                    SHELL.with(|shell| {
                        if let Some(state) = shell.borrow_mut().as_mut() {
                            let _ = state.app.apply_control(row, settings);
                        }
                    });
                    refresh_auto_caption(window);
                }),
            );
            let submission =
                dialog_window::prompt_config(window, current, live, saved, mounts, &mut drain);
            dialog_window::clear_actions();
            if let Some(submission) = submission {
                SHELL.with(|shell| {
                    if let Some(state) = shell.borrow_mut().as_mut() {
                        // The book first: a save the operator pressed should land even if the settings
                        // write is refused, because the two are separate requests to separate stores.
                        for request in submission.spots {
                            let _ = match request {
                                dialog_window::SpotRequest::Save { spot, name } => {
                                    state.app.save_spot(spot, name)
                                }
                                dialog_window::SpotRequest::Clear { map_id, name } => {
                                    state.app.clear_spot(map_id, name)
                                }
                            };
                        }
                        let _ = state.app.apply_control(row, submission.settings);
                    }
                });
                refresh_auto_caption(window);
            }
        }
        icons::CMD_ROW_DELETE => {
            let selected = selected_rows(window);
            let Some(row) = selected.first().copied() else {
                return;
            };
            // Destructive: the same typed confirmation as the bulk delete.
            if dialog_window::confirm_delete(window, &mut drain) {
                SHELL.with(|shell| {
                    if let Some(state) = shell.borrow_mut().as_mut() {
                        let _ = state.app.delete_rows(&[row]);
                    }
                });
            }
        }
        icons::CMD_ROW_ACTION => {
            let selected = selected_rows(window);
            let Some(row) = selected.first().copied() else {
                return;
            };
            SHELL.with(|shell| {
                if let Some(state) = shell.borrow_mut().as_mut() {
                    // One contextual action per row, chosen by its reconciled status.
                    match state.app.row_action(row) {
                        UiRowAction::Run => {
                            let _ = state.app.run_row(row);
                        }
                        UiRowAction::Stop => {
                            let _ = state.app.stop_row(row);
                        }
                        UiRowAction::RetryCleanup => {
                            let _ = state.app.retry_cleanup(row);
                        }
                        UiRowAction::None => {}
                    }
                }
            });
        }
        _ => {}
    }
    // Repaint immediately so a pending status appears without waiting for the next tick.
    refresh_table_from_model();
    refresh_row_action_caption(window);
}

/// Submits an import, clearing the temporary secret copy afterwards.
fn submit_import(window: HWND, mut submission: dialog_window::AccountSubmission) {
    let password = std::mem::take(&mut submission.password);
    SHELL.with(|shell| {
        if let Some(state) = shell.borrow_mut().as_mut() {
            let _ = state.app.import(&submission.username, password);
        }
    });
    let _ = window;
}

/// Reads the checked rows from the list view, mapping each back to its hidden key.
fn selected_rows(window: HWND) -> Vec<RowKey> {
    let table = SHELL.with(|shell| shell.borrow().as_ref().map(|state| state.table));
    let Some(table) = table.filter(|table| !table.is_null()) else {
        return Vec::new();
    };
    let _ = window;
    checked_row_keys(table)
}

/// The hidden key of one list view item, or `None` when the item cannot be read.
fn item_row_key(table: HWND, index: usize) -> Option<RowKey> {
    let mut item = empty_item(index);
    item.mask = LVIF_PARAM;
    if send(table, LVM_GETITEMW, 0, &mut item as *mut LVITEMW as isize) == 0 {
        return None;
    }
    NonZeroU64::new(item.lParam as u64).map(RowKey::new)
}

/// Keys of every checked row, in table order.
fn checked_row_keys(table: HWND) -> Vec<RowKey> {
    let count = send(table, LVM_GETITEMCOUNT, 0, 0);
    let mut rows = Vec::new();
    for index in 0..count.max(0) {
        // The state image index is 2 when the checkbox is checked.
        let state = send(
            table,
            LVM_GETITEMSTATE,
            index as usize,
            LVIS_STATEIMAGEMASK as isize,
        );
        if (state >> 12) != 2 {
            continue;
        }
        if let Some(key) = item_row_key(table, index as usize) {
            rows.push(key);
        }
    }
    rows
}

/// Key of the highlighted row, which is the one the character panel follows.
fn highlighted_row_key(table: HWND) -> Option<RowKey> {
    let index = send(table, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
    if index < 0 {
        return None;
    }
    item_row_key(table, index as usize)
}

/// A zeroed `LVITEMW` addressing one item's first column.
fn empty_item(index: usize) -> LVITEMW {
    LVITEMW {
        mask: 0,
        iItem: index as i32,
        iSubItem: 0,
        state: 0,
        stateMask: 0,
        pszText: null_mut(),
        cchTextMax: 0,
        iImage: 0,
        lParam: 0,
        iIndent: 0,
        iGroupId: 0,
        cColumns: 0,
        puColumns: null_mut(),
        piColFmt: null_mut(),
        iGroup: 0,
    }
}

/// Repaints the table from the current model without waiting for a worker event.
fn refresh_table_from_model() {
    SHELL.with(|shell| {
        if let Some(state) = shell.borrow().as_ref() {
            let rows: Vec<controls::RowCells> =
                state.app.rows().iter().map(controls::project_row).collect();
            repaint_table(state.table, &rows);
        }
    });
    refresh_status_line();
}

fn create_table(parent: HWND) -> HWND {
    let class_name = to_wide_from_pcwstr(WC_LISTVIEWW);
    let empty = to_wide("");
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let table = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS,
            0,
            0,
            0,
            0,
            parent,
            ID_TABLE as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    };
    if table.is_null() {
        return table;
    }
    // Checkbox selection, full-row selection, double buffer, and gridlines.
    const LVS_EX_GRIDLINES: u32 = 0x0000_0001;
    send(
        table,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        0,
        // Double buffering removes the flicker a full repaint would otherwise show.
        (LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_GRIDLINES) as isize,
    );
    // Dark theme for ListView: 0% CPU overhead
    const LVM_SETBKCOLOR: u32 = 0x1001;
    const LVM_SETTEXTCOLOR: u32 = 0x1024;
    const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
    send(table, LVM_SETBKCOLOR, 0, COLOR_TABLE_BG as isize);
    send(table, LVM_SETTEXTBKCOLOR, 0, COLOR_TABLE_BG as isize);
    send(table, LVM_SETTEXTCOLOR, 0, COLOR_TABLE_TEXT as isize);
    for column in controls::COLUMNS {
        let mut header = to_wide(column.header);
        let mut spec = LVCOLUMNW {
            mask: windows_sys::Win32::UI::Controls::LVCF_TEXT
                | windows_sys::Win32::UI::Controls::LVCF_WIDTH,
            fmt: LVCFMT_LEFT,
            cx: column.width,
            pszText: header.as_mut_ptr(),
            cchTextMax: 0,
            iSubItem: 0,
            iImage: 0,
            iOrder: 0,
            cxMin: 0,
            cxDefault: 0,
            cxIdeal: 0,
        };
        send(
            table,
            LVM_INSERTCOLUMNW,
            column.index,
            &mut spec as *mut LVCOLUMNW as isize,
        );
    }
    table
}

/// Re-places every child from the `WM_SIZE` client extent.
fn resize_shell(window: HWND, lparam: LPARAM) {
    let width = (lparam & 0xffff) as i32;
    let height = ((lparam >> 16) & 0xffff) as i32;
    apply_layout(window, width, height);
}

/// Re-places every child from the window's current client rect.
///
/// Used when the layout inputs changed without a `WM_SIZE`: creation and a DPI change.
fn resize_to_client(window: HWND) {
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `window` is this thread's live window and `client` outlives the call.
    if unsafe { GetClientRect(window, &mut client) } == 0 {
        return;
    }
    apply_layout(
        window,
        client.right - client.left,
        client.bottom - client.top,
    );
}

/// Drains one bounded batch and repaints when the table changed.
///
/// This is the single drain the timer and every dialog procedure share, so a modal dialog cannot
/// stall worker progress.
fn drain_worker_events(window: HWND) {
    let mut changed = false;
    let mut player_changed = false;
    let mut poll_player = false;
    SHELL.with(|shell| {
        let mut borrowed = shell.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return;
        };
        let player_before = state.app.player().cloned();
        changed = state.app.pump();
        player_changed = state.app.player().cloned() != player_before;
        // Portable boot finishes on the worker thread, so the first list can only be submitted once
        // its readiness event arrives.
        if state.app.refresh_when_ready() {
            changed = true;
        }
        if changed {
            let rows: Vec<controls::RowCells> =
                state.app.rows().iter().map(controls::project_row).collect();
            repaint_table(state.table, &rows);
        }
        // Counted here rather than driven by a second timer: one cadence keeps the poll aligned with
        // the drain that consumes its answer.
        state.player_poll_ticks = state.player_poll_ticks.saturating_add(1);
        if state.player_poll_ticks >= PLAYER_POLL_TICKS {
            state.player_poll_ticks = 0;
            poll_player = true;
        }
    });
    if changed {
        // The status line, the contextual caption, and command availability all read reconciled state.
        refresh_status_line();
        refresh_row_action_caption(window);
        refresh_command_availability(window);
        // A repaint reinserts every item and drops the highlight, so the panel's row is re-derived.
        focus_player_row(window);
    }
    if player_changed {
        refresh_player_panel();
    }
    if poll_player {
        request_player_reading();
    }
    // The close completes on whichever drain settles it: the last stop reply, or the shutdown result.
    destroy_when_close_completed(window);
}

fn repaint_table(table: HWND, rows: &[controls::RowCells]) {
    if table.is_null() {
        return;
    }
    // Deleting and inserting items sends LVN_ITEMCHANGED back into `window_proc` on this thread while
    // the caller still holds the shell state borrowed, so the handler is muted for the whole repaint.
    REPAINTING.set(true);
    if !rewrite_cells_in_place(table, rows) {
        rebuild_table(table, rows);
    }
    REPAINTING.set(false);
}

/// Rewrites every cell without touching the item set, or reports that the row set itself changed.
///
/// Preferred whenever the keys line up: a rebuild drops the highlight and every checkbox, so a status
/// arriving mid-selection would silently discard what the operator had chosen — and the once-a-second
/// character poll would do it repeatedly.
fn rewrite_cells_in_place(table: HWND, rows: &[controls::RowCells]) -> bool {
    let count = send(table, LVM_GETITEMCOUNT, 0, 0);
    if count < 0 || count as usize != rows.len() {
        return false;
    }
    for (index, row) in rows.iter().enumerate() {
        if item_row_key(table, index) != Some(row.key) {
            return false;
        }
    }
    for (index, row) in rows.iter().enumerate() {
        write_row_cells(table, index, row);
    }
    true
}

/// Rebuilds the item set, restoring the checks and the highlight for rows that still exist.
fn rebuild_table(table: HWND, rows: &[controls::RowCells]) {
    let checked = checked_row_keys(table);
    let highlighted = highlighted_row_key(table);
    send(table, LVM_DELETEALLITEMS, 0, 0);
    for (index, row) in rows.iter().enumerate() {
        // Column 0 is the unlabeled selection column; LVM_INSERTITEM rejects any other iSubItem.
        let mut item = empty_item(index);
        item.mask = LVIF_PARAM;
        // The row key routes actions and is never rendered as text.
        item.lParam = row.key.get().get() as isize;
        let inserted = send(
            table,
            LVM_INSERTITEMW,
            0,
            &mut item as *mut LVITEMW as isize,
        );
        if inserted < 0 {
            continue;
        }
        let index = inserted as usize;
        write_row_cells(table, index, row);
        if checked.contains(&row.key) {
            set_item_state(
                table,
                index,
                INDEX_TO_CHECKED_STATE_IMAGE,
                LVIS_STATEIMAGEMASK,
            );
        }
        if highlighted == Some(row.key) {
            set_item_state(
                table,
                index,
                LVIS_SELECTED | LVIS_FOCUSED,
                LVIS_SELECTED | LVIS_FOCUSED,
            );
        }
    }
}

fn write_row_cells(table: HWND, index: usize, row: &controls::RowCells) {
    set_cell(table, index, 1, &row.username);
    set_cell(table, index, 2, row.server);
    set_cell(table, index, 3, row.status);
    set_cell(table, index, 4, &row.last_run);
}

fn set_item_state(table: HWND, index: usize, state: u32, mask: u32) {
    let mut item = empty_item(index);
    item.state = state;
    item.stateMask = mask;
    send(
        table,
        LVM_SETITEMSTATE,
        index,
        &mut item as *mut LVITEMW as isize,
    );
}

fn set_cell(table: HWND, item: usize, column: i32, text: &str) {
    let mut buffer = to_wide(text);
    let mut cell = LVITEMW {
        mask: LVIF_TEXT,
        iItem: item as i32,
        iSubItem: column,
        state: 0,
        stateMask: 0,
        pszText: buffer.as_mut_ptr(),
        cchTextMax: 0,
        iImage: 0,
        lParam: 0,
        iIndent: 0,
        iGroupId: 0,
        cColumns: 0,
        puColumns: null_mut(),
        piColFmt: null_mut(),
        iGroup: 0,
    };
    send(
        table,
        LVM_SETITEMTEXTW,
        item,
        &mut cell as *mut LVITEMW as isize,
    );
}
/// Runs the confirmed close sequence instead of destroying the window immediately.
///
/// With retained work the operator is actually asked, and answering no cancels the close and leaves
/// every account running. There is deliberately no leave-running-and-exit option.
///
/// A second WM_CLOSE while one is already running is ignored rather than restarting the sequence: the
/// operator pressing the X again used to re-ask, re-submit, and re-arm the wait, which is why
/// answering Yes could leave the window open.
fn handle_close(window: HWND) {
    let prompt = SHELL.with(|shell| {
        shell.borrow_mut().as_mut().map(|state| {
            if state.app.close_in_flight() {
                None
            } else {
                Some(state.app.request_close())
            }
        })
    });
    let Some(prompt) = prompt else {
        // No state at all: the window is already torn down.
        // SAFETY: `window` is this thread's live top-level window.
        unsafe {
            DestroyWindow(window);
        }
        return;
    };
    // Already closing: the drain finishes it, so this press is deliberately inert.
    let Some(prompt) = prompt else {
        return;
    };
    if prompt == ClosePrompt::StopAndExitOrCancel && !confirm_stop_and_exit(window) {
        // Cancelled: nothing was submitted and the table is untouched.
        SHELL.with(|shell| {
            if let Some(state) = shell.borrow_mut().as_mut() {
                state.app.cancel_close();
            }
        });
        return;
    }
    if prompt == ClosePrompt::StopAndExitOrCancel {
        SHELL.with(|shell| {
            if let Some(state) = shell.borrow_mut().as_mut() {
                let _ = state.app.confirm_stop_and_exit();
            }
        });
    }
    refresh_table_from_model();
    // The close may already be complete — every stop refused, or the worker gone — and a completed
    // close produces no further event to destroy the window on.
    destroy_when_close_completed(window);
}

/// Destroys the window once the close sequence has reached `Closed`.
fn destroy_when_close_completed(window: HWND) {
    let closed = SHELL.with(|shell| {
        shell
            .borrow()
            .as_ref()
            .is_some_and(|state| state.app.close_phase() == ClosePhase::Closed)
    });
    if closed {
        // SAFETY: `window` is this thread's live top-level window.
        unsafe {
            DestroyWindow(window);
        }
    }
}

/// Asks the operator to confirm stopping every running account and exiting.
fn confirm_stop_and_exit(window: HWND) -> bool {
    let text = to_wide(
        "Vẫn còn tài khoản đang chạy. Dừng tất cả và thoát?\n\nChọn Không để tiếp tục chạy.",
    );
    let caption = to_wide(WINDOW_TITLE);
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    let answer = unsafe {
        MessageBoxW(
            window,
            text.as_ptr(),
            caption.as_ptr(),
            MB_YESNO | MB_ICONWARNING,
        )
    };
    answer == IDYES
}

fn send(window: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: `window` is a live control owned by this thread.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(window, message, wparam, lparam)
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_wide_from_pcwstr(value: *const u16) -> Vec<u16> {
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

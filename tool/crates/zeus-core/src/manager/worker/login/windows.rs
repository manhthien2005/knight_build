//! Exact Windows target qualification and the measured fixed submit script.
//!
//! The geometry and script layout are pure functions so they can be asserted literally without a
//! live game. Only the foreground/input driver touches Win32.
//!
//! The script is valid only for the pinned KO402 runtime and bundle, and the operator can neither
//! supply nor edit it.

use super::LoginFailureCode;

/// Reference client size measured on 2026-08-25 at DPI 120.
pub(super) const REFERENCE_CLIENT_WIDTH: i32 = 239;
pub(super) const REFERENCE_CLIENT_HEIGHT: i32 = 362;
pub(super) const REFERENCE_DPI: u32 = 120;
/// Normalized client size must land within this many pixels of the reference.
pub(super) const NORMALIZED_TOLERANCE: i32 = 2;

/// Measured reference click points, in reference client coordinates.
pub(super) const CLICK_ACCOUNT_MENU: (i32, i32) = (150, 210);
pub(super) const CLICK_USERNAME_FIELD: (i32, i32) = (150, 178);
pub(super) const CLICK_EDITOR_TEXT_AREA: (i32, i32) = (150, 178);
pub(super) const CLICK_PASSWORD_FIELD: (i32, i32) = (70, 200);

/// One step of the fixed script. The list is data so a test can assert it literally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginStage {
    /// Wait the configured stable-menu delay after qualification.
    WaitMenuSettle,
    /// Wait a screen transition.
    WaitScreenTransition,
    /// Wait for a MIDP editor to settle.
    WaitEditorSettle,
    /// Click one scaled reference point.
    Click(ClickTarget),
    /// One `SendInput` call carrying Ctrl+A then the username.
    SendUsername,
    /// One `SendInput` call carrying Ctrl+A then the complete password.
    SendPassword,
    /// Right soft command, committing an editor.
    SendCommandF2,
    /// Left soft command, submitting the form.
    SendCommandF1,
    /// Revalidate the login form after a transition.
    RevalidateForm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClickTarget {
    AccountMenu,
    UsernameField,
    EditorTextArea,
    PasswordField,
}

impl ClickTarget {
    /// Reference coordinate of this target, before scaling.
    pub(super) fn reference_point(self) -> (i32, i32) {
        match self {
            Self::AccountMenu => CLICK_ACCOUNT_MENU,
            Self::UsernameField => CLICK_USERNAME_FIELD,
            Self::EditorTextArea => CLICK_EDITOR_TEXT_AREA,
            Self::PasswordField => CLICK_PASSWORD_FIELD,
        }
    }
}

/// The measured stage sequence, in spec section 15 order.
///
/// Every stage that sends input is preceded by foreground acquisition and revalidation at execution
/// time; the sequence itself only fixes what is sent and in which order.
pub(super) const FIXED_SCRIPT: &[LoginStage] = &[
    LoginStage::WaitMenuSettle,
    LoginStage::Click(ClickTarget::AccountMenu),
    LoginStage::WaitScreenTransition,
    LoginStage::Click(ClickTarget::UsernameField),
    LoginStage::WaitEditorSettle,
    LoginStage::Click(ClickTarget::EditorTextArea),
    LoginStage::SendUsername,
    LoginStage::SendCommandF2,
    LoginStage::WaitScreenTransition,
    LoginStage::RevalidateForm,
    LoginStage::Click(ClickTarget::PasswordField),
    LoginStage::WaitEditorSettle,
    LoginStage::Click(ClickTarget::EditorTextArea),
    LoginStage::SendPassword,
    LoginStage::SendCommandF2,
    LoginStage::WaitScreenTransition,
    LoginStage::RevalidateForm,
    LoginStage::SendCommandF1,
];

/// Normalizes a client dimension to reference DPI.
///
/// A zero or unsupported DPI fails closed rather than dividing by zero or trusting a bogus scale.
pub(super) fn normalize_dimension(dimension: i32, dpi: u32) -> Option<i32> {
    if dpi == 0 || dimension <= 0 {
        return None;
    }
    let scaled = i64::from(dimension) * i64::from(REFERENCE_DPI);
    let dpi = i64::from(dpi);
    // Round half away from zero, matching `round()` in the measurement formula.
    let normalized = (scaled + dpi / 2) / dpi;
    i32::try_from(normalized).ok()
}

/// Requires the normalized client shape to match the measured reference within tolerance.
///
/// This rejects a resized or structurally different window while preserving the measured layout
/// across DPI scaling.
pub(super) fn client_shape_is_qualified(width: i32, height: i32, dpi: u32) -> bool {
    let (Some(normalized_width), Some(normalized_height)) = (
        normalize_dimension(width, dpi),
        normalize_dimension(height, dpi),
    ) else {
        return false;
    };
    (normalized_width - REFERENCE_CLIENT_WIDTH).abs() <= NORMALIZED_TOLERANCE
        && (normalized_height - REFERENCE_CLIENT_HEIGHT).abs() <= NORMALIZED_TOLERANCE
}

/// Scales one reference click into the current client rectangle.
///
/// Width and height scale independently, exactly as measured.
pub(super) fn scale_click(point: (i32, i32), client_width: i32, client_height: i32) -> (i32, i32) {
    let x = i64::from(point.0) * i64::from(client_width) / i64::from(REFERENCE_CLIENT_WIDTH);
    let y = i64::from(point.1) * i64::from(client_height) / i64::from(REFERENCE_CLIENT_HEIGHT);
    (x as i32, y as i32)
}

/// Requires a scaled click to stay inside the client rectangle and the virtual desktop.
///
/// A partially off-screen or clipped target fails before any mouse input is emitted.
pub(super) fn click_is_deliverable(
    client_point: (i32, i32),
    client_width: i32,
    client_height: i32,
    screen_point: (i32, i32),
    virtual_desktop: (i32, i32, i32, i32),
) -> bool {
    let inside_client =
        (0..client_width).contains(&client_point.0) && (0..client_height).contains(&client_point.1);
    let (left, top, right, bottom) = virtual_desktop;
    let inside_desktop =
        (left..right).contains(&screen_point.0) && (top..bottom).contains(&screen_point.1);
    inside_client && inside_desktop
}

/// Requires every modifier to be physically up before the script starts.
///
/// A held Shift, Ctrl, Alt, or Windows key would change what the fixed script types.
pub(super) fn modifiers_are_clear(
    shift: bool,
    control: bool,
    alt: bool,
    windows: bool,
) -> Result<(), LoginFailureCode> {
    if shift || control || alt || windows {
        return Err(LoginFailureCode::TargetMismatch);
    }
    Ok(())
}

/// Requires the target's integrity level to be at most the UI's.
///
/// Windows never reveals whether UIPI dropped an input, so an elevated target is rejected up front
/// instead of being diagnosed after a silent failure.
pub(super) fn integrity_allows_input(target: u32, ui: u32) -> Result<(), LoginFailureCode> {
    if target > ui {
        return Err(LoginFailureCode::IntegrityMismatch);
    }
    Ok(())
}

/// One synthetic input event, built as data so a test can assert the exact array literally.
///
/// This mirrors the `INPUT` records handed to `SendInput`; keeping it as a pure value means the whole
/// batch shape is verifiable without emitting real input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InputEvent {
    MouseMoveAbsolute {
        x: i32,
        y: i32,
    },
    MouseLeftDown,
    MouseLeftUp,
    KeyDown(u16),
    KeyUp(u16),
    /// One UTF-16 code unit typed as text, so non-ASCII credentials survive unchanged.
    UnicodeDown(u16),
    UnicodeUp(u16),
}

/// Virtual-key codes the fixed script uses.
pub(super) const VK_CONTROL_CODE: u16 = 17;
pub(super) const VK_A_CODE: u16 = 65;
pub(super) const VK_F1_CODE: u16 = 112;
pub(super) const VK_F2_CODE: u16 = 113;

/// Builds one absolute left click at a normalized absolute coordinate.
pub(super) fn click_batch(absolute_x: i32, absolute_y: i32) -> Vec<InputEvent> {
    vec![
        InputEvent::MouseMoveAbsolute {
            x: absolute_x,
            y: absolute_y,
        },
        InputEvent::MouseLeftDown,
        InputEvent::MouseLeftUp,
    ]
}

/// Builds the single batch that selects all existing text and replaces it with `text`.
///
/// Ctrl+A precedes the text in the same array, so a partially typed value can never be appended to a
/// stale editor value, and no other input can be interspersed within the batch.
pub(super) fn replace_text_batch(text: &[u16]) -> Vec<InputEvent> {
    let mut batch = Vec::with_capacity(4 + text.len() * 2);
    batch.push(InputEvent::KeyDown(VK_CONTROL_CODE));
    batch.push(InputEvent::KeyDown(VK_A_CODE));
    batch.push(InputEvent::KeyUp(VK_A_CODE));
    batch.push(InputEvent::KeyUp(VK_CONTROL_CODE));
    for unit in text {
        batch.push(InputEvent::UnicodeDown(*unit));
        batch.push(InputEvent::UnicodeUp(*unit));
    }
    batch
}

/// Builds one soft-command keypress.
pub(super) fn command_batch(virtual_key: u16) -> Vec<InputEvent> {
    vec![
        InputEvent::KeyDown(virtual_key),
        InputEvent::KeyUp(virtual_key),
    ]
}

/// Absolute-coordinate denominator used by `SendInput`'s normalized mouse space.
const ABSOLUTE_RANGE: i64 = 65_535;

/// Converts a screen point into `SendInput`'s normalized absolute space.
pub(super) fn to_absolute(
    screen_point: (i32, i32),
    virtual_desktop: (i32, i32, i32, i32),
) -> Option<(i32, i32)> {
    let (left, top, right, bottom) = virtual_desktop;
    let width = i64::from(right) - i64::from(left);
    let height = i64::from(bottom) - i64::from(top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let x = (i64::from(screen_point.0) - i64::from(left)) * ABSOLUTE_RANGE / width;
    let y = (i64::from(screen_point.1) - i64::from(top)) * ABSOLUTE_RANGE / height;
    Some((x as i32, y as i32))
}

/// Attaches the foreground and target input queues for exactly one stage.
///
/// Plain `SetForegroundWindow` measured 1/4 on stable game windows; attaching the queues first
/// measured 4/4. The attachment is always undone, including on every failure path, because a leaked
/// attachment would couple the worker's input queue to the game's.
#[cfg(windows)]
pub(super) struct ForegroundAttachment {
    foreground_thread: u32,
    target_thread: u32,
    attached: bool,
}

#[cfg(windows)]
impl ForegroundAttachment {
    /// Acquires the foreground for `target`, confirming the result rather than trusting the call.
    pub(super) fn acquire(
        target: isize,
        foreground_thread: u32,
        target_thread: u32,
    ) -> Result<Self, LoginFailureCode> {
        use windows_sys::Win32::System::Threading::AttachThreadInput;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, SetForegroundWindow,
        };

        if foreground_thread == 0 || target_thread == 0 {
            return Err(LoginFailureCode::ForegroundDenied);
        }
        // SAFETY: both thread ids come from GetWindowThreadProcessId on live windows.
        let attached = foreground_thread == target_thread
            || unsafe { AttachThreadInput(foreground_thread, target_thread, 1) } != 0;
        let attachment = Self {
            foreground_thread,
            target_thread,
            attached: attached && foreground_thread != target_thread,
        };
        if !attached {
            return Err(LoginFailureCode::ForegroundDenied);
        }
        // SAFETY: `target` is a live top-level window handle.
        unsafe {
            SetForegroundWindow(target as *mut _);
        }
        // Require the result instead of trusting the return value.
        // SAFETY: no arguments; returns the current foreground window or null.
        let foreground = unsafe { GetForegroundWindow() } as isize;
        if foreground != target {
            return Err(LoginFailureCode::ForegroundDenied);
        }
        Ok(attachment)
    }
}

#[cfg(windows)]
impl Drop for ForegroundAttachment {
    fn drop(&mut self) {
        if !self.attached {
            return;
        }
        use windows_sys::Win32::System::Threading::AttachThreadInput;
        // SAFETY: detaching the same pair that was attached; ignoring the result is intentional
        // because there is no recovery and the process is exiting the stage either way.
        unsafe {
            AttachThreadInput(self.foreground_thread, self.target_thread, 0);
        }
    }
}

/// Emits one built batch through `SendInput`.
///
/// A zero or short return is reported only as a generic rejection: Windows never reveals whether UIPI
/// discarded the input, so guessing a cause here would be a fabricated diagnosis.
#[cfg(windows)]
pub(super) fn send_batch(batch: &[InputEvent]) -> Result<(), LoginFailureCode> {
    use std::mem::size_of;

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
    };

    if batch.is_empty() {
        return Ok(());
    }
    let inputs: Vec<INPUT> = batch
        .iter()
        .map(|event| match *event {
            InputEvent::MouseMoveAbsolute { x, y } => INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: x,
                        dy: y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            InputEvent::MouseLeftDown => mouse_button(MOUSEEVENTF_LEFTDOWN),
            InputEvent::MouseLeftUp => mouse_button(MOUSEEVENTF_LEFTUP),
            InputEvent::KeyDown(code) => key_event(code, 0),
            InputEvent::KeyUp(code) => key_event(code, KEYEVENTF_KEYUP),
            InputEvent::UnicodeDown(unit) => unicode_event(unit, KEYEVENTF_UNICODE),
            InputEvent::UnicodeUp(unit) => unicode_event(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        })
        .collect();

    // SAFETY: `inputs` is a live, correctly sized array of INPUT records.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        return Err(LoginFailureCode::InputRejected);
    }
    Ok(())
}

#[cfg(windows)]
fn mouse_button(
    flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(windows)]
fn key_event(
    code: u16,
    flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(windows)]
fn unicode_event(
    unit: u16,
    flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_login_input_normalizes_every_supported_dpi() {
        // The reference measurement itself normalizes to exactly the reference size.
        assert_eq!(normalize_dimension(239, 120), Some(239));
        assert_eq!(normalize_dimension(362, 120), Some(362));
        // 96, 144, and 192 DPI clients of the same logical window all normalize back.
        assert_eq!(normalize_dimension(191, 96), Some(239));
        assert_eq!(normalize_dimension(290, 96), Some(363));
        assert_eq!(normalize_dimension(287, 144), Some(239));
        assert_eq!(normalize_dimension(434, 144), Some(362));
        assert_eq!(normalize_dimension(382, 192), Some(239));
        assert_eq!(normalize_dimension(579, 192), Some(362));
        // Zero or nonsensical input fails closed instead of dividing by zero.
        assert_eq!(normalize_dimension(239, 0), None);
        assert_eq!(normalize_dimension(0, 120), None);
        assert_eq!(normalize_dimension(-239, 120), None);
    }

    #[test]
    fn windows_login_input_accepts_two_pixels_and_rejects_three() {
        for delta in -NORMALIZED_TOLERANCE..=NORMALIZED_TOLERANCE {
            assert!(
                client_shape_is_qualified(
                    REFERENCE_CLIENT_WIDTH + delta,
                    REFERENCE_CLIENT_HEIGHT + delta,
                    REFERENCE_DPI
                ),
                "normalized delta {delta} must be accepted"
            );
        }
        for delta in [
            -(NORMALIZED_TOLERANCE + 1),
            NORMALIZED_TOLERANCE + 1,
            NORMALIZED_TOLERANCE + 40,
        ] {
            assert!(
                !client_shape_is_qualified(
                    REFERENCE_CLIENT_WIDTH + delta,
                    REFERENCE_CLIENT_HEIGHT,
                    REFERENCE_DPI
                ),
                "normalized width delta {delta} must be rejected"
            );
            assert!(
                !client_shape_is_qualified(
                    REFERENCE_CLIENT_WIDTH,
                    REFERENCE_CLIENT_HEIGHT + delta,
                    REFERENCE_DPI
                ),
                "normalized height delta {delta} must be rejected"
            );
        }
        // A structurally different window is rejected at every DPI.
        assert!(!client_shape_is_qualified(800, 600, REFERENCE_DPI));
        assert!(!client_shape_is_qualified(800, 600, 96));
        assert!(!client_shape_is_qualified(239, 362, 0));
    }

    #[test]
    fn windows_login_input_scales_every_measured_click() {
        // At the reference size the measured coordinates are used unchanged.
        for target in [
            ClickTarget::AccountMenu,
            ClickTarget::UsernameField,
            ClickTarget::EditorTextArea,
            ClickTarget::PasswordField,
        ] {
            assert_eq!(
                scale_click(
                    target.reference_point(),
                    REFERENCE_CLIENT_WIDTH,
                    REFERENCE_CLIENT_HEIGHT
                ),
                target.reference_point()
            );
        }
        // Width and height scale independently.
        assert_eq!(scale_click(CLICK_ACCOUNT_MENU, 478, 362), (300, 210));
        assert_eq!(scale_click(CLICK_ACCOUNT_MENU, 239, 724), (150, 420));
        assert_eq!(scale_click(CLICK_PASSWORD_FIELD, 478, 724), (140, 400));
        // The measured reference points are exactly the four spec section 15 values.
        assert_eq!(CLICK_ACCOUNT_MENU, (150, 210));
        assert_eq!(CLICK_USERNAME_FIELD, (150, 178));
        assert_eq!(CLICK_EDITOR_TEXT_AREA, (150, 178));
        assert_eq!(CLICK_PASSWORD_FIELD, (70, 200));
    }

    #[test]
    fn windows_login_input_requires_clicks_inside_client_and_desktop() {
        let desktop = (0, 0, 1920, 1080);
        assert!(click_is_deliverable(
            (150, 210),
            239,
            362,
            (250, 310),
            desktop
        ));
        // Outside the client rectangle.
        assert!(!click_is_deliverable(
            (239, 210),
            239,
            362,
            (339, 310),
            desktop
        ));
        assert!(!click_is_deliverable(
            (150, 362),
            239,
            362,
            (250, 462),
            desktop
        ));
        // Inside the client but off the virtual desktop: a clipped or off-screen target.
        assert!(!click_is_deliverable(
            (150, 210),
            239,
            362,
            (-1, 310),
            desktop
        ));
        assert!(!click_is_deliverable(
            (150, 210),
            239,
            362,
            (250, 1080),
            desktop
        ));
        // A multi-monitor desktop with negative origin is still honored.
        assert!(click_is_deliverable(
            (150, 210),
            239,
            362,
            (-800, 310),
            (-1920, 0, 1920, 1080)
        ));
    }

    #[test]
    fn windows_login_input_script_is_the_measured_sequence() {
        // Asserted literally: the fixed script cannot drift without failing this test.
        assert_eq!(
            FIXED_SCRIPT,
            &[
                LoginStage::WaitMenuSettle,
                LoginStage::Click(ClickTarget::AccountMenu),
                LoginStage::WaitScreenTransition,
                LoginStage::Click(ClickTarget::UsernameField),
                LoginStage::WaitEditorSettle,
                LoginStage::Click(ClickTarget::EditorTextArea),
                LoginStage::SendUsername,
                LoginStage::SendCommandF2,
                LoginStage::WaitScreenTransition,
                LoginStage::RevalidateForm,
                LoginStage::Click(ClickTarget::PasswordField),
                LoginStage::WaitEditorSettle,
                LoginStage::Click(ClickTarget::EditorTextArea),
                LoginStage::SendPassword,
                LoginStage::SendCommandF2,
                LoginStage::WaitScreenTransition,
                LoginStage::RevalidateForm,
                LoginStage::SendCommandF1,
            ]
        );
        // Exactly one password-bearing stage, and it follows the username commit.
        let password_stages = FIXED_SCRIPT
            .iter()
            .filter(|stage| **stage == LoginStage::SendPassword)
            .count();
        assert_eq!(password_stages, 1);
        let username_index = FIXED_SCRIPT
            .iter()
            .position(|stage| *stage == LoginStage::SendUsername)
            .expect("the username stage exists");
        let password_index = FIXED_SCRIPT
            .iter()
            .position(|stage| *stage == LoginStage::SendPassword)
            .expect("the password stage exists");
        assert!(username_index < password_index);
        // A revalidation precedes the password stage, so a mismatch stops before credentials.
        assert!(
            FIXED_SCRIPT[..password_index].contains(&LoginStage::RevalidateForm),
            "the password stage must be guarded by a revalidation"
        );
        // Submission is last, and it is the only left soft command.
        assert_eq!(FIXED_SCRIPT.last(), Some(&LoginStage::SendCommandF1));
        assert_eq!(
            FIXED_SCRIPT
                .iter()
                .filter(|stage| **stage == LoginStage::SendCommandF1)
                .count(),
            1
        );
    }

    #[test]
    fn windows_login_input_builds_the_exact_click_and_command_arrays() {
        // A click is exactly move-absolute, down, up.
        assert_eq!(
            click_batch(32100, 15400),
            vec![
                InputEvent::MouseMoveAbsolute { x: 32100, y: 15400 },
                InputEvent::MouseLeftDown,
                InputEvent::MouseLeftUp,
            ]
        );
        // Soft commands are a single down/up pair each.
        assert_eq!(
            command_batch(VK_F2_CODE),
            vec![InputEvent::KeyDown(113), InputEvent::KeyUp(113)]
        );
        assert_eq!(
            command_batch(VK_F1_CODE),
            vec![InputEvent::KeyDown(112), InputEvent::KeyUp(112)]
        );
    }

    #[test]
    fn windows_login_input_replaces_editor_text_in_one_batch() {
        let username: Vec<u16> = "ab".encode_utf16().collect();
        // Ctrl+A leads the same array, so text replaces rather than appends.
        assert_eq!(
            replace_text_batch(&username),
            vec![
                InputEvent::KeyDown(VK_CONTROL_CODE),
                InputEvent::KeyDown(VK_A_CODE),
                InputEvent::KeyUp(VK_A_CODE),
                InputEvent::KeyUp(VK_CONTROL_CODE),
                InputEvent::UnicodeDown(97),
                InputEvent::UnicodeUp(97),
                InputEvent::UnicodeDown(98),
                InputEvent::UnicodeUp(98),
            ]
        );

        // The password batch is one array: nothing can be interspersed inside it.
        let password: Vec<u16> = "P@1".encode_utf16().collect();
        let batch = replace_text_batch(&password);
        assert_eq!(batch.len(), 4 + password.len() * 2);
        assert_eq!(batch[0], InputEvent::KeyDown(VK_CONTROL_CODE));
        assert_eq!(batch[3], InputEvent::KeyUp(VK_CONTROL_CODE));
        // Every credential unit is typed as text, so the exact bytes survive.
        assert_eq!(
            batch[4..],
            [
                InputEvent::UnicodeDown(80),
                InputEvent::UnicodeUp(80),
                InputEvent::UnicodeDown(64),
                InputEvent::UnicodeUp(64),
                InputEvent::UnicodeDown(49),
                InputEvent::UnicodeUp(49),
            ]
        );
        // An empty replacement still clears the field.
        assert_eq!(replace_text_batch(&[]).len(), 4);
    }

    #[test]
    fn windows_login_input_maps_screen_points_into_absolute_space() {
        let desktop = (0, 0, 1920, 1080);
        // Origin and far corner anchor the normalized range.
        assert_eq!(to_absolute((0, 0), desktop), Some((0, 0)));
        assert_eq!(to_absolute((1920, 1080), desktop), Some((65535, 65535)));
        // A mid-screen point lands mid-range.
        let (x, y) = to_absolute((960, 540), desktop).expect("mid-screen maps");
        assert!((32_000..33_000).contains(&x), "x was {x}");
        assert!((32_000..33_000).contains(&y), "y was {y}");
        // A multi-monitor desktop with negative origin is offset, not clamped.
        let spanning = (-1920, 0, 1920, 1080);
        assert_eq!(to_absolute((-1920, 0), spanning), Some((0, 0)));
        assert_eq!(to_absolute((0, 0), spanning), Some((32767, 0)));
        // A degenerate desktop fails closed instead of dividing by zero.
        assert_eq!(to_absolute((0, 0), (0, 0, 0, 1080)), None);
        assert_eq!(to_absolute((0, 0), (0, 0, 1920, 0)), None);
    }

    #[test]
    fn windows_login_input_rejects_held_modifiers_and_elevated_targets() {
        assert!(modifiers_are_clear(false, false, false, false).is_ok());
        for (shift, control, alt, windows) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert_eq!(
                modifiers_are_clear(shift, control, alt, windows),
                Err(LoginFailureCode::TargetMismatch)
            );
        }

        // Equal integrity is allowed; a higher-integrity target is refused up front.
        assert!(integrity_allows_input(0x2000, 0x2000).is_ok());
        assert!(integrity_allows_input(0x1000, 0x2000).is_ok());
        assert_eq!(
            integrity_allows_input(0x3000, 0x2000),
            Err(LoginFailureCode::IntegrityMismatch)
        );
    }
}

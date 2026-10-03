use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    RcIdle = 0,
    RcNativeWait = 1,
    RcLogin = 2,
    RcServer = 3,
    RcCharacter = 4,
    RcWorldSettle = 5,
    RcOther = 6,
    RcLoading = 7,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockButton {
    pub cmd_id: i32,
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockDialog {
    pub is_ah: bool,
    pub text: String,
    pub buttons: Vec<MockButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockLoginScreen {
    pub center_cmd: Option<MockButton>,
    pub username_text: Option<String>,
    pub password_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchedAction {
    pub timestamp: i64,
    pub action_type: &'static str,
    pub attempt: i32,
    pub cmd_id: i32,
    pub caption: String,
    pub armed_backoff_until: i64,
}

pub struct ReconnectR2CSimulator {
    // Episode observer state
    pub active: bool,
    pub state: ReconnectState,
    pub started_at: i64,
    pub state_since: i64,
    pub episode_id: i32,
    pub transitions: i32,
    pub ever_stable_world_seen: bool,
    pub world_seen_before_episode: bool,
    pub last_reason: String,

    // R2B1 Dialog recovery action state
    pub recovery_attempts: i32,
    pub recovery_last_action_at: i64,
    pub recovery_backoff_until: i64,
    pub recovery_last_fingerprint: String,

    // R2B2 Login recovery action state
    pub login_attempts: i32,
    pub login_last_action_at: i64,
    pub login_backoff_until: i64,
    pub login_last_fingerprint: String,

    // Dispatches
    pub dispatches: Vec<DispatchedAction>,
}

impl ReconnectR2CSimulator {
    pub const RC_RECOVERY_COOLDOWN_MS: i64 = 5000;
    pub const RC_RECOVERY_MAX_ACTIONS: i32 = 4;
    pub const RC_RECOVERY_BACKOFF_MS: i64 = 600000;
    pub const RC_NATIVE_GRACE_MS: i64 = 5000;
    pub const RC_STATE_DWELL_MS: i64 = 5000;

    pub const RC_LOGIN_DWELL_MS: i64 = 5000;
    pub const RC_LOGIN_RETRY_INTERVAL_MS: i64 = 30000;
    pub const RC_LOGIN_MAX_ACTIONS: i32 = 3;
    pub const RC_LOGIN_BACKOFF_MS: i64 = 600000;

    pub fn new() -> Self {
        Self {
            active: false,
            state: ReconnectState::RcIdle,
            started_at: 0,
            state_since: 0,
            episode_id: 0,
            transitions: 0,
            ever_stable_world_seen: false,
            world_seen_before_episode: false,
            last_reason: String::new(),

            recovery_attempts: 0,
            recovery_last_action_at: 0,
            recovery_backoff_until: 0,
            recovery_last_fingerprint: String::new(),

            login_attempts: 0,
            login_last_action_at: 0,
            login_backoff_until: 0,
            login_last_fingerprint: String::new(),

            dispatches: Vec::new(),
        }
    }

    pub fn normalize(s: &str) -> String {
        let source = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡ\
                      ùúụủũưừứựửữỳýỵỷỹđ\
                      ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠ\
                      ÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐ";
        let target = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo\
                      uuuuuuuuuuuyyyyyd\
                      aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo\
                      uuuuuuuuuuuyyyyyd";
        let lower = s.to_lowercase();
        let src_chars: Vec<char> = source.chars().collect();
        let tgt_chars: Vec<char> = target.chars().collect();
        let mut out = String::with_capacity(lower.len());
        for c in lower.chars() {
            if let Some(pos) = src_chars.iter().position(|&sc| sc == c) {
                if pos < tgt_chars.len() {
                    out.push(tgt_chars[pos]);
                } else {
                    out.push(c);
                }
            } else {
                out.push(c);
            }
        }
        out.replace(['\r', '\n', '\t'], " ").trim().to_string()
    }

    pub fn is_strong_disconnect(dialog: Option<&MockDialog>, bv_a: bool) -> (bool, &'static str) {
        if bv_a {
            return (true, "NATIVE_BV_A");
        }
        if let Some(d) = dialog {
            if d.is_ah {
                let norm_txt = Self::normalize(&d.text);
                if norm_txt.contains("mat ket noi") {
                    return (true, "MODAL_DISCONNECT_MAT_KET_NOI");
                }
                if norm_txt.contains("ket noi that bai") {
                    return (true, "MODAL_DISCONNECT_KET_NOI_THAT_BAI");
                }
                if norm_txt.contains("vui long dang nhap lai") {
                    return (true, "MODAL_DISCONNECT_VUI_LONG_DANG_NHAP_LAI");
                }
            }
        }
        (false, "")
    }

    pub fn find_reconnect_ok_button(dialog: &MockDialog) -> Option<MockButton> {
        for btn in &dialog.buttons {
            if btn.cmd_id == 0 {
                let norm_cap = Self::normalize(&btn.caption);
                if norm_cap == "ok" || norm_cap == "o k" {
                    return Some(btn.clone());
                }
            }
        }
        None
    }

    /// Exact R2C reconnectWorldReady predicate:
    /// fu.a == fu.c && cn.g != null && mapStable()
    pub fn reconnect_world_ready(
        screen: Option<&str>,
        cn_g_present: bool,
        map_stable: bool,
    ) -> bool {
        screen == Some("world") && cn_g_present && map_stable
    }

    /// R2B1 Fail-closed recovery action evaluation with final revalidation before accounting
    pub fn evaluate_recovery_action(
        &mut self,
        now: i64,
        dialog: Option<&MockDialog>,
        bv_a: bool,
        bv_b: i64,
        final_validation_success: bool,
    ) {
        if !self.active || self.state != ReconnectState::RcNativeWait || !self.world_seen_before_episode {
            return;
        }
        let dlg = match dialog {
            Some(d) if d.is_ah => d,
            _ => return,
        };

        let norm_txt = Self::normalize(&dlg.text);
        let is_proven_disconnect = norm_txt.contains("mat ket noi")
            || norm_txt.contains("ket noi that bai")
            || norm_txt.contains("vui long dang nhap lai");
        if !is_proven_disconnect {
            return;
        }
        if norm_txt.contains("dang ket noi") || norm_txt.contains("vui long cho") || norm_txt.contains("cho ket noi") {
            return;
        }

        let ok_btn = match Self::find_reconnect_ok_button(dlg) {
            Some(b) => b,
            None => return,
        };

        if bv_a && bv_b > 0 && now < bv_b + Self::RC_NATIVE_GRACE_MS {
            return;
        }
        let dwell = if now >= self.state_since { now - self.state_since } else { 0 };
        if dwell < Self::RC_STATE_DWELL_MS {
            return;
        }

        if self.recovery_last_action_at > 0 {
            let elapsed = if now >= self.recovery_last_action_at { now - self.recovery_last_action_at } else { -1 };
            if elapsed < Self::RC_RECOVERY_COOLDOWN_MS {
                return;
            }
        }

        if self.recovery_backoff_until > 0 {
            if now < self.recovery_backoff_until {
                return;
            } else {
                self.recovery_backoff_until = 0;
                self.recovery_attempts = 0;
            }
        }

        // R2C Fail-closed final live validation check BEFORE accounting
        if !final_validation_success {
            // Must return without modifying attempts, last_action_at, backoff or fingerprint
            return;
        }

        self.recovery_last_fingerprint = format!("{}|{}|{}:{}", norm_txt, dlg.buttons.len(), ok_btn.cmd_id, Self::normalize(&ok_btn.caption));
        self.recovery_attempts += 1;
        self.recovery_last_action_at = now;
        if self.recovery_attempts >= Self::RC_RECOVERY_MAX_ACTIONS {
            self.recovery_backoff_until = now + Self::RC_RECOVERY_BACKOFF_MS;
        }

        self.dispatches.push(DispatchedAction {
            timestamp: now,
            action_type: "DIALOG_RECOVERY",
            attempt: self.recovery_attempts,
            cmd_id: ok_btn.cmd_id,
            caption: ok_btn.caption.clone(),
            armed_backoff_until: self.recovery_backoff_until,
        });
    }

    /// R2B2 Fail-closed login action evaluation with privacy cleanup and final revalidation before accounting
    pub fn evaluate_login_action(
        &mut self,
        now: i64,
        screen_is_login: bool,
        has_modal: bool,
        has_alert: bool,
        context_menu_active: bool,
        context_menu_null: bool,
        has_popup_overlay: bool,
        login_screen: Option<&MockLoginScreen>,
        strong_disconnect: bool,
        final_validation_success: bool,
    ) {
        if !self.active || !self.world_seen_before_episode || self.state != ReconnectState::RcLogin {
            return;
        }
        if strong_disconnect {
            return;
        }
        if !screen_is_login || has_modal || has_alert || context_menu_active || context_menu_null || has_popup_overlay {
            return;
        }

        let ls = match login_screen {
            Some(s) => s,
            None => return,
        };

        let btn = match &ls.center_cmd {
            Some(b) if b.cmd_id == 0 => b,
            _ => return,
        };
        let cap = Self::normalize(&btn.caption);
        if cap != "choi tiep" {
            return;
        }

        let user = match &ls.username_text {
            Some(u) if !u.trim().is_empty() => u.trim(),
            _ => return,
        };
        let pass = match &ls.password_text {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => return,
        };
        let _ = (user, pass);

        let dwell = if now >= self.state_since { now - self.state_since } else { 0 };
        if dwell < Self::RC_LOGIN_DWELL_MS {
            return;
        }

        if self.login_backoff_until > 0 {
            if now < self.login_backoff_until {
                return;
            } else {
                self.login_backoff_until = 0;
                self.login_attempts = 0;
            }
        }

        if self.login_last_action_at > 0 {
            let elapsed = if now >= self.login_last_action_at { now - self.login_last_action_at } else { -1 };
            if elapsed < Self::RC_LOGIN_RETRY_INTERVAL_MS {
                return;
            }
        }

        // R2C Fail-closed final live validation check BEFORE accounting
        if !final_validation_success {
            // Must return without modifying attempts, last_action_at, backoff or fingerprint
            return;
        }

        // Privacy cleanup: fingerprint must NOT include username or password
        self.login_last_fingerprint = format!("{}|{}", cap, btn.cmd_id);

        self.login_attempts += 1;
        self.login_last_action_at = now;
        if self.login_attempts >= Self::RC_LOGIN_MAX_ACTIONS {
            self.login_backoff_until = now + Self::RC_LOGIN_BACKOFF_MS;
        }

        self.dispatches.push(DispatchedAction {
            timestamp: now,
            action_type: "LOGIN_RECOVERY",
            attempt: self.login_attempts,
            cmd_id: btn.cmd_id,
            caption: btn.caption.clone(),
            armed_backoff_until: self.login_backoff_until,
        });
    }

    pub fn tick(
        &mut self,
        now: i64,
        screen: Option<&str>, // "loading", "login", "server", "character", "world", "other", or None
        dialog: Option<&MockDialog>,
        login_screen: Option<&MockLoginScreen>,
        bv_a: bool,
        bv_b: i64,
        cn_g_present: bool,
        map_stable: bool,
        // UI flags
        has_alert: bool,
        context_menu_active: bool,
        context_menu_null: bool,
        has_popup_overlay: bool,
        is_server_overlay: bool,
        // Final validation control for testing fail-closed semantics
        final_validation_dialog: bool,
        final_validation_login: bool,
    ) {
        if now < self.started_at {
            self.started_at = now;
        }
        if now < self.state_since {
            self.state_since = now;
        }
        if now < self.recovery_last_action_at {
            if self.recovery_backoff_until > self.recovery_last_action_at {
                self.recovery_backoff_until = now + Self::RC_RECOVERY_BACKOFF_MS;
            }
            self.recovery_last_action_at = now;
        }
        if now < self.login_last_action_at {
            if self.login_backoff_until > self.login_last_action_at {
                self.login_backoff_until = now + Self::RC_LOGIN_BACKOFF_MS;
            }
            self.login_last_action_at = now;
        }

        let world_ready = Self::reconnect_world_ready(screen, cn_g_present, map_stable);

        // Normal prior stable gameplay sets ever_stable_world_seen when no episode is active
        if !self.active && world_ready {
            self.ever_stable_world_seen = true;
        }

        let (strong_disconnect, reason) = Self::is_strong_disconnect(dialog, bv_a);

        // Episode Open
        if !self.active {
            if strong_disconnect {
                self.active = true;
                self.episode_id += 1;
                self.started_at = now;
                self.state_since = now;
                self.state = ReconnectState::RcNativeWait;
                self.transitions = 0;
                self.world_seen_before_episode = self.ever_stable_world_seen;
                self.last_reason = reason.to_string();

                self.recovery_attempts = 0;
                self.recovery_backoff_until = 0;
                self.recovery_last_fingerprint.clear();

                self.login_attempts = 0;
                self.login_last_action_at = 0;
                self.login_backoff_until = 0;
                self.login_last_fingerprint.clear();
            }
            return;
        }

        // Active Episode Handling
        let cur_screen = match screen {
            Some(s) => s,
            None => return, // transient null pauses observation
        };

        // R2C Episode Close: requires !strong_disconnect && reconnectWorldReady()
        if !strong_disconnect && world_ready {
            self.ever_stable_world_seen = true;
            if self.state != ReconnectState::RcIdle {
                self.transitions += 1;
            }
            self.active = false;
            self.state = ReconnectState::RcIdle;
            self.state_since = now;
            self.last_reason.clear();

            self.recovery_attempts = 0;
            self.recovery_backoff_until = 0;
            self.recovery_last_fingerprint.clear();

            self.login_attempts = 0;
            self.login_last_action_at = 0;
            self.login_backoff_until = 0;
            self.login_last_fingerprint.clear();
            return;
        }

        // Derive state for active episode:
        // strongDisconnect outranks screen type -> RC_NATIVE_WAIT
        // fu.a == fu.d -> RC_LOADING
        // fu.a == fu.b && fu.t == fu.g -> RC_SERVER
        // fu.a == fu.b -> RC_LOGIN
        // fu.a == fu.i -> RC_CHARACTER
        // fu.a == fu.c -> RC_WORLD_SETTLE
        // other -> RC_OTHER
        let target_state = if strong_disconnect {
            ReconnectState::RcNativeWait
        } else if cur_screen == "loading" {
            ReconnectState::RcLoading
        } else if cur_screen == "login" && is_server_overlay {
            ReconnectState::RcServer
        } else if cur_screen == "login" {
            ReconnectState::RcLogin
        } else if cur_screen == "character" {
            ReconnectState::RcCharacter
        } else if cur_screen == "world" {
            ReconnectState::RcWorldSettle
        } else {
            ReconnectState::RcOther
        };

        if target_state != self.state {
            self.state = target_state;
            self.state_since = now;
            self.transitions += 1;
        }

        // Recovery actions
        if self.state == ReconnectState::RcNativeWait && self.world_seen_before_episode {
            self.evaluate_recovery_action(now, dialog, bv_a, bv_b, final_validation_dialog);
        } else if self.state == ReconnectState::RcLogin && self.world_seen_before_episode {
            self.evaluate_login_action(
                now,
                cur_screen == "login",
                dialog.is_some(),
                has_alert,
                context_menu_active,
                context_menu_null,
                has_popup_overlay,
                login_screen,
                strong_disconnect,
                final_validation_login,
            );
        }
        // RC_LOADING is observation-only: performs NO recovery action
    }
}

// ===========================================================================
// SOURCE CONTRACT VERIFICATION
// ===========================================================================

#[test]
fn test_r2c_state_model_source_and_constants() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path).expect("Zeus.java must exist");

    // 1. Existing numeric values RC_IDLE through RC_OTHER must remain unchanged
    assert!(content.contains("public static final int RC_IDLE = 0;"), "RC_IDLE must be 0");
    assert!(content.contains("public static final int RC_NATIVE_WAIT = 1;"), "RC_NATIVE_WAIT must be 1");
    assert!(content.contains("public static final int RC_LOGIN = 2;"), "RC_LOGIN must be 2");
    assert!(content.contains("public static final int RC_SERVER = 3;"), "RC_SERVER must be 3");
    assert!(content.contains("public static final int RC_CHARACTER = 4;"), "RC_CHARACTER must be 4");
    assert!(content.contains("public static final int RC_WORLD_SETTLE = 5;"), "RC_WORLD_SETTLE must be 5");
    assert!(content.contains("public static final int RC_OTHER = 6;"), "RC_OTHER must be 6");

    // 2. RC_LOADING must be appended as 7
    assert!(content.contains("public static final int RC_LOADING = 7;"), "RC_LOADING must be appended as 7");

    // 3. stateName(int state) must include RC_LOADING
    assert!(content.contains("case RC_LOADING: return \"RC_LOADING\";"), "stateName must map RC_LOADING to \"RC_LOADING\"");

    // 4. In supervisor, fu.a == fu.d must map to RC_LOADING after strongDisconnect precedence
    let section_start = content.find("// ---- RECONNECT").expect("RECONNECT section must exist");
    let section_end = content[section_start..].find("// ---- end RECONNECT").expect("end RECONNECT marker must exist");
    let supervisor_body = &content[section_start..section_start + section_end];

    assert!(
        supervisor_body.contains("fu.a == fu.d"),
        "Supervisor state derivation must check fu.a == fu.d"
    );
    assert!(
        supervisor_body.contains("targetState = RC_LOADING;"),
        "Supervisor state derivation must assign targetState = RC_LOADING"
    );

    // Verify strongDisconnect outranks fu.a == fu.d
    let strong_pos = supervisor_body.find("if (strongDisconnect)").expect("if (strongDisconnect) must exist");
    let loading_pos = supervisor_body.find("fu.a == fu.d").expect("fu.a == fu.d must exist");
    assert!(strong_pos < loading_pos, "strongDisconnect precedence must precede fu.a == fu.d");

    // 5. RC_LOADING must perform NO action (observation-only)
    assert!(
        !supervisor_body.contains("if (reconnectState == RC_LOADING"),
        "reconnectState == RC_LOADING must have no action branch in supervisor"
    );
}

#[test]
fn test_r2c_reconnect_world_ready_source_and_semantics() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path).expect("Zeus.java must exist");

    // 1. reconnectWorldReady helper must exist
    let fn_start = content.find("boolean reconnectWorldReady()")
        .expect("reconnectWorldReady() method must exist");
    let fn_body = &content[fn_start..fn_start + 300];

    // 2. Exact semantics: return fu.a == fu.c && cn.g != null && mapStable();
    assert!(
        fn_body.contains("fu.a == fu.c") && fn_body.contains("cn.g != null") && fn_body.contains("mapStable()"),
        "reconnectWorldReady() must return fu.a == fu.c && cn.g != null && mapStable()"
    );

    // 3. Must NOT check gameReady(), alive(), captcha(), noDialog(), or coordinates
    assert!(!fn_body.contains("gameReady()"), "reconnectWorldReady must not call gameReady()");
    assert!(!fn_body.contains("alive()"), "reconnectWorldReady must not call alive()");
    assert!(!fn_body.contains("captcha()"), "reconnectWorldReady must not call captcha()");
    assert!(!fn_body.contains("noDialog()"), "reconnectWorldReady must not call noDialog()");
    assert!(!fn_body.contains("cn.g.cx"), "reconnectWorldReady must not check cn.g.cx");
    assert!(!fn_body.contains("cn.g.cy"), "reconnectWorldReady must not check cn.g.cy");

    // 4. gameReady() must remain unchanged
    let game_ready_start = content.find("public static boolean gameReady()").expect("gameReady() must exist");
    let game_ready_body = &content[game_ready_start..game_ready_start + 400];
    assert!(game_ready_body.contains("!alive()"), "gameReady() must still check !alive()");
    assert!(game_ready_body.contains("captcha()"), "gameReady() must still check captcha()");
    assert!(game_ready_body.contains("!noDialog()"), "gameReady() must still check !noDialog()");

    // 5. Supervisor must use reconnectWorldReady() for episode close and normal prior world seen
    let section_start = content.find("// ---- RECONNECT").expect("RECONNECT section must exist");
    let section_end = content[section_start..].find("// ---- end RECONNECT").expect("end RECONNECT marker must exist");
    let supervisor_body = &content[section_start..section_start + section_end];

    assert!(
        supervisor_body.contains("!strongDisconnect && reconnectWorldReady()"),
        "Episode close must require !strongDisconnect && reconnectWorldReady()"
    );
    assert!(
        supervisor_body.contains("!reconnectEpisodeActive && reconnectWorldReady()"),
        "Prior stable world seen must be set by !reconnectEpisodeActive && reconnectWorldReady()"
    );
    assert!(
        !supervisor_body.contains("gameReady()"),
        "Supervisor must no longer call gameReady() for reconnect lifecycle"
    );
}

#[test]
fn test_r2c_privacy_and_fingerprint_cleanup() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path).expect("Zeus.java must exist");

    let fn_start = content.find("private static void evaluateReconnectLoginAction")
        .expect("evaluateReconnectLoginAction must exist");
    let fn_body = &content[fn_start..fn_start + 6000.min(content.len() - fn_start)];

    // Must NOT contain user.trim() or raw user in fingerprint
    let fp_line = fn_body.lines().find(|l| l.contains("reconnectLoginLastFingerprint ="))
        .expect("reconnectLoginLastFingerprint assignment must exist");
    assert!(
        !fp_line.contains("user"),
        "reconnectLoginLastFingerprint must not embed username"
    );
    assert!(
        !fp_line.contains("pass"),
        "reconnectLoginLastFingerprint must not embed password"
    );
    assert!(
        fp_line.contains("caption") && fp_line.contains("loginBtn.e"),
        "reconnectLoginLastFingerprint must contain only caption and command id"
    );
}

#[test]
fn test_r2c_dispatch_accounting_hardening_source_contract() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path).expect("Zeus.java must exist");

    // 1. R2B2 LoginScreen: Final live validation MUST occur before attempts increment
    let login_start = content.find("private static void evaluateReconnectLoginAction")
        .expect("evaluateReconnectLoginAction must exist");
    let login_body = &content[login_start..login_start + 6000.min(content.len() - login_start)];

    let attempts_inc_pos = login_body.find("reconnectLoginAttempts++")
        .or_else(|| login_body.find("++reconnectLoginAttempts"))
        .expect("Attempts increment must exist");
    let dispatch_pos = login_body.find("loginBtn.a();").expect("loginBtn.a() dispatch must exist");

    // Accounting must be before dispatch
    assert!(attempts_inc_pos < dispatch_pos, "Accounting must precede loginBtn.a()");

    // Final live validation must precede accounting
    let final_valid_pos = login_body.find("fu.b.ab != loginBtn")
        .or_else(|| login_body.find("fu.b.ab == loginBtn"))
        .expect("Final live button check must exist");
    assert!(final_valid_pos < attempts_inc_pos, "Final live validation must precede accounting increment");

    // 2. R2B1 Dialog: Final live validation MUST occur before attempts increment
    let recovery_start = content.find("private static void evaluateReconnectRecoveryAction")
        .expect("evaluateReconnectRecoveryAction must exist");
    let recovery_body = &content[recovery_start..recovery_start + 6000.min(content.len() - recovery_start)];

    let rec_attempts_pos = recovery_body.find("reconnectRecoveryAttempts++")
        .or_else(|| recovery_body.find("++reconnectRecoveryAttempts"))
        .expect("Recovery attempts increment must exist");
    let rec_dispatch_pos = recovery_body.find("okBtn.a();").expect("okBtn.a() dispatch must exist");

    assert!(rec_attempts_pos < rec_dispatch_pos, "Accounting must precede okBtn.a()");

    // Live dialog recheck must precede accounting
    let rec_live_pos = recovery_body.find("findReconnectOkButton")
        .expect("Live button recheck must exist");
    assert!(rec_live_pos < rec_attempts_pos, "Final live dialog/button validation must precede accounting");
}

// ===========================================================================
// BEHAVIORAL CONTRACT TESTS
// ===========================================================================

#[test]
fn test_r2c_state_model_loading_and_precedence() {
    let mut sim = ReconnectR2CSimulator::new();

    // 1. Establish prior stable world
    sim.tick(1000, Some("world"), None, None, false, 0, true, true, false, false, false, false, false, true, true);
    assert!(sim.ever_stable_world_seen);

    // 2. Strong disconnect opens episode
    sim.tick(2000, Some("world"), None, None, true, 0, false, false, false, false, false, false, false, true, true);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);

    // 3. Clear disconnect, screen is loading (fu.a == fu.d)
    sim.tick(3000, Some("loading"), None, None, false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcLoading, "Screen loading must map to RC_LOADING");
    assert_eq!(sim.transitions, 1);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOADING must perform no native action");

    // 4. Strong disconnect while in loading -> RC_NATIVE_WAIT takes precedence
    sim.tick(4000, Some("loading"), None, None, true, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcNativeWait, "Strong disconnect while in loading maps to RC_NATIVE_WAIT");
    assert_eq!(sim.transitions, 2);

    // 5. Clear disconnect back to loading
    sim.tick(5000, Some("loading"), None, None, false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcLoading);
    assert_eq!(sim.transitions, 3);
    assert_eq!(sim.dispatches.len(), 0, "No actions dispatched from loading");

    // 6. Unknown non-null screen maps to RC_OTHER
    sim.tick(6000, Some("unknown_screen"), None, None, false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcOther, "Unknown screen maps to RC_OTHER");
}

#[test]
fn test_r2c_authoritative_world_ready_close_matrix() {
    let mut sim = ReconnectR2CSimulator::new();

    // Prior world established
    sim.tick(1000, Some("world"), None, None, false, 0, true, true, false, false, false, false, false, true, true);
    assert!(sim.ever_stable_world_seen);

    // Open episode
    sim.tick(2000, Some("world"), None, None, true, 0, false, false, false, false, false, false, false, true, true);
    assert!(sim.active);

    // Case 1: Dead character, captcha present, informational non-disconnect dialog present,
    // BUT mapStable() == true and cn.g != null -> AUTHORITATIVE SUCCESSFUL CLOSE!
    let info_dialog = MockDialog {
        is_ah: true,
        text: "Thong bao: Su kien cuoi tuan bat dau".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };
    sim.tick(3000, Some("world"), Some(&info_dialog), None, false, 0, true, true, false, false, false, false, false, true, true);
    assert!(!sim.active, "Dead/captcha/informational dialog world MUST successfully close reconnect episode");
    assert_eq!(sim.state, ReconnectState::RcIdle);

    // Re-open episode for negative cases
    sim.tick(4000, Some("world"), None, None, true, 0, false, false, false, false, false, false, false, true, true);
    assert!(sim.active);

    // Case 2: cn.g == null prevents close
    sim.tick(5000, Some("world"), None, None, false, 0, false, true, false, false, false, false, false, true, true);
    assert!(sim.active, "cn.g == null must prevent close");
    assert_eq!(sim.state, ReconnectState::RcWorldSettle);

    // Case 3: mapStable() == false prevents close
    sim.tick(6000, Some("world"), None, None, false, 0, true, false, false, false, false, false, false, true, true);
    assert!(sim.active, "mapStable() == false must prevent close");
    assert_eq!(sim.state, ReconnectState::RcWorldSettle);

    // Case 4: strongDisconnect == true prevents close even if cn.g != null && mapStable() == true
    sim.tick(7000, Some("world"), None, None, true, 0, true, true, false, false, false, false, false, true, true);
    assert!(sim.active, "strongDisconnect must prevent close even when world is ready");
    assert_eq!(sim.state, ReconnectState::RcNativeWait);

    // Case 5: When strong disconnect clears, close succeeds
    sim.tick(8000, Some("world"), None, None, false, 0, true, true, false, false, false, false, false, true, true);
    assert!(!sim.active, "Episode closes once strong disconnect clears");
    assert_eq!(sim.state, ReconnectState::RcIdle);
}

#[test]
fn test_r2c_fail_closed_r2b2_login_accounting() {
    let mut sim = ReconnectR2CSimulator::new();
    let login_screen = MockLoginScreen {
        center_cmd: Some(MockButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("player1".to_string()),
        password_text: Some("secret".to_string()),
    };

    // Prior world & disconnect
    sim.tick(1000, Some("world"), None, None, false, 0, true, true, false, false, false, false, false, true, true);
    sim.tick(2000, Some("world"), None, None, true, 0, false, false, false, false, false, false, false, true, true);
    sim.tick(3000, Some("login"), None, Some(&login_screen), false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcLogin);

    // Dwell >= 5000 ms at now=8000, but final UI validation FAILS (e.g. screen changed or overlay appeared)
    sim.tick(8000, Some("login"), None, Some(&login_screen), false, 0, false, false, false, false, false, false, false, true, false);
    assert_eq!(sim.dispatches.len(), 0, "No dispatch on failed final validation");
    assert_eq!(sim.login_attempts, 0, "Zero attempts consumed on failed final validation");
    assert_eq!(sim.login_last_action_at, 0, "last_action_at NOT updated on failed final validation");
    assert_eq!(sim.login_backoff_until, 0, "Backoff NOT armed on failed final validation");
    assert!(sim.login_last_fingerprint.is_empty(), "Fingerprint NOT updated on failed final validation");

    // When final validation succeeds at now=9000, action is dispatched and accounting armed
    sim.tick(9000, Some("login"), None, Some(&login_screen), false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.dispatches.len(), 1, "Dispatched on successful final validation");
    assert_eq!(sim.login_attempts, 1);
    assert_eq!(sim.login_last_action_at, 9000);
    assert!(!sim.login_last_fingerprint.contains("player1"), "Fingerprint must not contain username");
}

#[test]
fn test_r2c_fail_closed_r2b1_dialog_accounting() {
    let mut sim = ReconnectR2CSimulator::new();
    let disconnect_dialog = MockDialog {
        is_ah: true,
        text: "Mat ket noi voi may chu".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };

    // Prior world & disconnect
    sim.tick(1000, Some("world"), None, None, false, 0, true, true, false, false, false, false, false, true, true);
    sim.tick(2000, Some("world"), Some(&disconnect_dialog), None, false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);

    // Dwell >= 5000 ms at now=7000, but final dialog validation FAILS (e.g. dialog replaced)
    sim.tick(7000, Some("world"), Some(&disconnect_dialog), None, false, 0, false, false, false, false, false, false, false, false, true);
    assert_eq!(sim.dispatches.len(), 0, "No dispatch on failed final validation");
    assert_eq!(sim.recovery_attempts, 0, "Zero recovery attempts consumed on failed final validation");
    assert_eq!(sim.recovery_last_action_at, 0, "last_action_at NOT updated on failed final validation");
    assert_eq!(sim.recovery_backoff_until, 0, "Backoff NOT armed on failed final validation");

    // When final validation succeeds at now=8000, action is dispatched and accounting armed
    sim.tick(8000, Some("world"), Some(&disconnect_dialog), None, false, 0, false, false, false, false, false, false, false, true, true);
    assert_eq!(sim.dispatches.len(), 1, "Dispatched on successful final validation");
    assert_eq!(sim.recovery_attempts, 1);
    assert_eq!(sim.recovery_last_action_at, 8000);
}

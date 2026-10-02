use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    RcIdle,
    RcNativeWait,
    RcLogin,
    RcServer,
    RcCharacter,
    RcWorldSettle,
    RcOther,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockLoginButton {
    pub cmd_id: i32,
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockLoginScreen {
    pub center_cmd: Option<MockLoginButton>,
    pub username_text: Option<String>,
    pub password_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchedLoginAction {
    pub timestamp: i64,
    pub attempt: i32,
    pub cmd_id: i32,
    pub caption: String,
    pub armed_backoff_until: i64,
}

pub struct ReconnectLoginSimulator {
    // Observer state
    pub active: bool,
    pub state: ReconnectState,
    pub started_at: i64,
    pub state_since: i64,
    pub episode_id: i32,
    pub transitions: i32,
    pub ever_stable_world_seen: bool,
    pub world_seen_before_episode: bool,
    pub last_reason: String,

    // R2B1 Dialog recovery state (kept separate)
    pub recovery_attempts: i32,
    pub recovery_last_action_at: i64,
    pub recovery_backoff_until: i64,

    // R2B2 Login action state
    pub login_attempts: i32,
    pub login_last_action_at: i64,
    pub login_backoff_until: i64,
    pub login_last_fingerprint: String,

    // Recorded dispatches
    pub dispatches: Vec<DispatchedLoginAction>,
}

impl ReconnectLoginSimulator {
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

    pub fn is_strong_disconnect(has_modal: bool, modal_text: &str, bv_a: bool) -> (bool, &'static str) {
        if bv_a {
            return (true, "NATIVE_BV_A");
        }
        if has_modal {
            let norm_txt = Self::normalize(modal_text);
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
        (false, "")
    }

    pub fn evaluate_login_action(
        &mut self,
        now: i64,
        screen_is_login: bool, // fu.a == fu.b
        has_modal: bool,        // fu.s != null
        has_alert: bool,        // fu.t != null
        context_menu_active: bool, // fu.p != null && fu.p.a
        context_menu_null: bool,   // fu.p == null
        has_popup_overlay: bool,   // d.b == true
        login_screen: Option<&MockLoginScreen>, // fu.b
        strong_disconnect: bool,
    ) {
        // Strict authorization check
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

        // Center command check
        let btn = match &ls.center_cmd {
            Some(b) => b,
            None => return,
        };
        if btn.cmd_id != 0 {
            return;
        }
        let cap = Self::normalize(&btn.caption);
        if cap != "choi tiep" {
            return;
        }

        // Normal credentials check
        let user = match &ls.username_text {
            Some(u) => u.trim(),
            None => return,
        };
        let pass = match &ls.password_text {
            Some(p) => p.trim(),
            None => return,
        };
        if user.is_empty() || pass.is_empty() {
            return;
        }

        // Dwell time check: >= 5000 ms in RC_LOGIN
        let dwell = if now >= self.state_since { now - self.state_since } else { 0 };
        if dwell < Self::RC_LOGIN_DWELL_MS {
            return;
        }

        // Backoff check
        if self.login_backoff_until > 0 {
            if now < self.login_backoff_until {
                return;
            } else {
                // Backoff expired; reset attempts for another bounded cycle
                self.login_backoff_until = 0;
                self.login_attempts = 0;
            }
        }

        // Cooldown / retry interval check (30000 ms)
        if self.login_last_action_at > 0 {
            let elapsed = if now >= self.login_last_action_at { now - self.login_last_action_at } else { -1 };
            if elapsed < Self::RC_LOGIN_RETRY_INTERVAL_MS {
                return;
            }
        }

        // Diagnostics fingerprint (non-sensitive action identity only)
        self.login_last_fingerprint = format!("{}|{}", cap, btn.cmd_id);

        // Increment and arm BEFORE dispatch
        self.login_attempts += 1;
        self.login_last_action_at = now;
        let mut armed_backoff = 0;
        if self.login_attempts >= Self::RC_LOGIN_MAX_ACTIONS {
            self.login_backoff_until = now + Self::RC_LOGIN_BACKOFF_MS;
            armed_backoff = self.login_backoff_until;
        }

        // Dispatch live command
        self.dispatches.push(DispatchedLoginAction {
            timestamp: now,
            attempt: self.login_attempts,
            cmd_id: btn.cmd_id,
            caption: btn.caption.clone(),
            armed_backoff_until: armed_backoff,
        });
    }

    pub fn tick(
        &mut self,
        now: i64,
        screen: Option<&str>,
        login_screen: Option<&MockLoginScreen>,
        has_modal: bool,
        modal_text: &str,
        has_alert: bool,
        context_menu_active: bool,
        context_menu_null: bool,
        has_popup_overlay: bool,
        is_server_overlay: bool,
        bv_a: bool,
        game_ready: bool,
        map_stable: bool,
    ) {
        // Clock rollback defense
        if now < self.started_at {
            self.started_at = now;
        }
        if now < self.state_since {
            self.state_since = now;
        }
        if now < self.login_last_action_at {
            if self.login_backoff_until > self.login_last_action_at {
                self.login_backoff_until = now + Self::RC_LOGIN_BACKOFF_MS;
            }
            self.login_last_action_at = now;
        }

        // Normal prior stable gameplay
        if !self.active && screen == Some("world") && game_ready && map_stable {
            self.ever_stable_world_seen = true;
        }

        let (strong_disconnect, reason) = Self::is_strong_disconnect(has_modal, modal_text, bv_a);

        // Episode open
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

                self.login_attempts = 0;
                self.login_last_action_at = 0;
                self.login_backoff_until = 0;
                self.login_last_fingerprint.clear();
            }
            return;
        }

        // Active episode handling
        let cur_screen = match screen {
            Some(s) => s,
            None => return,
        };

        // Successful close
        if !strong_disconnect && cur_screen == "world" && game_ready && map_stable {
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

            self.login_attempts = 0;
            self.login_last_action_at = 0;
            self.login_backoff_until = 0;
            self.login_last_fingerprint.clear();
            return;
        }

        // Derive state
        let target_state = if strong_disconnect {
            ReconnectState::RcNativeWait
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

        // R2B2 Login Recovery Action
        if self.state == ReconnectState::RcLogin && self.world_seen_before_episode {
            self.evaluate_login_action(
                now,
                cur_screen == "login",
                has_modal,
                has_alert,
                context_menu_active,
                context_menu_null,
                has_popup_overlay,
                login_screen,
                strong_disconnect,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// SOURCE INTEGRITY CONTRACT TEST (Java Code Analysis)
// ---------------------------------------------------------------------------

#[test]
fn test_reconnect_login_java_source_contract() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path)
        .expect("Zeus.java must exist at mod/zeus/src/Zeus.java");

    // 1. Constants existence and values
    assert!(
        content.contains("long RC_LOGIN_DWELL_MS = 5000L;")
            || content.contains("int RC_LOGIN_DWELL_MS = 5000;"),
        "RC_LOGIN_DWELL_MS must be 5000"
    );
    assert!(
        content.contains("long RC_LOGIN_RETRY_INTERVAL_MS = 30000L;")
            || content.contains("int RC_LOGIN_RETRY_INTERVAL_MS = 30000;"),
        "RC_LOGIN_RETRY_INTERVAL_MS must be 30000"
    );
    assert!(
        content.contains("int RC_LOGIN_MAX_ACTIONS = 3;"),
        "RC_LOGIN_MAX_ACTIONS must be 3"
    );
    assert!(
        content.contains("long RC_LOGIN_BACKOFF_MS = 600000L;")
            || content.contains("int RC_LOGIN_BACKOFF_MS = 600000;"),
        "RC_LOGIN_BACKOFF_MS must be 600000"
    );

    // 2. Action state fields existence
    assert!(
        content.contains("reconnectLoginAttempts"),
        "reconnectLoginAttempts field must exist"
    );
    assert!(
        content.contains("reconnectLoginLastActionAt"),
        "reconnectLoginLastActionAt field must exist"
    );
    assert!(
        content.contains("reconnectLoginBackoffUntil"),
        "reconnectLoginBackoffUntil field must exist"
    );

    // 3. Supervisor and evaluation methods
    assert!(
        content.contains("evaluateReconnectLoginAction"),
        "evaluateReconnectLoginAction method must exist"
    );

    // Find the body of evaluateReconnectLoginAction
    let fn_start = content.find("private static void evaluateReconnectLoginAction")
        .expect("evaluateReconnectLoginAction method definition must exist");
    let fn_body = &content[fn_start..fn_start + 6000.min(content.len() - fn_start)];

    // 4. Strict UI and credential guards
    assert!(
        fn_body.contains("fu.a == fu.b") || fn_body.contains("fu.a != fu.b"),
        "Must verify fu.a == fu.b"
    );
    assert!(
        fn_body.contains("fu.s == null") || fn_body.contains("fu.s != null"),
        "Must verify modal absence (fu.s == null)"
    );
    assert!(
        fn_body.contains("fu.t == null") || fn_body.contains("fu.t != null"),
        "Must verify alert absence (fu.t == null)"
    );
    assert!(
        fn_body.contains("fu.p") && (fn_body.contains(".a") || fn_body.contains("!fu.p.a")),
        "Must verify context menu inactive (fu.p != null && !fu.p.a)"
    );
    assert!(
        fn_body.contains("d.b"),
        "Must verify popup overlay absence (!d.b)"
    );
    assert!(
        fn_body.contains(".ab"),
        "Must inspect fu.b.ab center command slot"
    );
    assert!(
        fn_body.contains(".e == 0") || fn_body.contains(".e != 0"),
        "Must verify command id 0"
    );
    assert!(
        fn_body.contains("choi tiep"),
        "Must check normalized caption 'choi tiep'"
    );
    assert!(
        fn_body.contains("bs.g") && fn_body.contains("bs.h"),
        "Must check normal username (bs.g) and password (bs.h) textboxes"
    );

    // 5. Dispatch ordering: accounting BEFORE live dispatch, and final validation BEFORE accounting
    let dispatch_pos = fn_body.find(".a();").expect("Live button dispatch .a() must exist");
    let attempts_inc_pos = fn_body.find("reconnectLoginAttempts++")
        .or_else(|| fn_body.find("++reconnectLoginAttempts"))
        .expect("Attempts increment must exist");
    assert!(
        attempts_inc_pos < dispatch_pos,
        "Action accounting (reconnectLoginAttempts++) must precede .a() dispatch"
    );

    // Final live validation must precede accounting
    let final_valid_pos = fn_body.find("fu.b.ab != loginBtn")
        .or_else(|| fn_body.find("fu.b.ab == loginBtn"))
        .expect("Final live button check must exist");
    assert!(
        final_valid_pos < attempts_inc_pos,
        "Final live validation must precede action accounting"
    );

    // Privacy check: fingerprint must NOT contain username or user
    let fp_line = fn_body.lines().find(|l| l.contains("reconnectLoginLastFingerprint ="))
        .expect("reconnectLoginLastFingerprint assignment must exist");
    assert!(
        !fp_line.contains("user"),
        "reconnectLoginLastFingerprint must NOT embed username"
    );

    // 6. Forbidden primitives check
    assert!(
        !fn_body.contains("bs.c()"),
        "bs.c() must NOT be called as retry primitive"
    );
    assert!(
        !fn_body.contains("bs.i()"),
        "bs.i() must NOT be called to reload credentials"
    );
    assert!(
        !fn_body.contains("b(0,") && !fn_body.contains("b(0 ,"),
        "Direct LoginScreen.b(0,...) must NOT be called"
    );
    assert!(
        !fn_body.contains("fu.I ="),
        "fu.I must NOT be modified"
    );
}

// ---------------------------------------------------------------------------
// BEHAVIORAL CONTRACT TESTS
// ---------------------------------------------------------------------------

#[test]
fn test_login_authorization_matrix() {
    let mut sim = ReconnectLoginSimulator::new();

    let valid_login_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("hero123".to_string()),
        password_text: Some("secretPass".to_string()),
    };

    // 1. Initial startup LoginScreen without reconnect episode -> no action
    sim.tick(1000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "Initial startup LoginScreen -> no action");

    // Establish prior stable world
    sim.tick(2000, Some("world"), None, false, "", false, false, false, false, false, false, true, true);
    assert!(sim.ever_stable_world_seen);

    // 2. Reconnect episode opened without prior stable world -> no action
    let mut sim_no_prior = ReconnectLoginSimulator::new();
    sim_no_prior.tick(1000, Some("world"), None, true, "mat ket noi", false, false, false, false, false, false, false, false);
    assert!(sim_no_prior.active);
    assert!(!sim_no_prior.world_seen_before_episode);
    // Transition to login screen after 10000ms
    sim_no_prior.tick(11000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, false, false, false, false, false);
    sim_no_prior.tick(20000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim_no_prior.dispatches.len(), 0, "No action when world_seen_before_episode is false");

    // Open valid episode with prior stable world
    sim.tick(3000, Some("world"), None, true, "mat ket noi", false, false, false, false, false, false, false, false);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);
    assert!(sim.world_seen_before_episode);

    // Transition to RC_LOGIN at now=4000
    sim.tick(4000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcLogin);

    // 3. RC_LOGIN with modal -> no action
    sim.tick(10000, Some("login"), Some(&valid_login_screen), true, "thong bao", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with modal -> no action");

    // 4. RC_LOGIN with alert -> no action
    sim.tick(10000, Some("login"), Some(&valid_login_screen), false, "", true, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with alert -> no action");

    // 5. RC_LOGIN with context menu active -> no action
    sim.tick(10000, Some("login"), Some(&valid_login_screen), false, "", false, true, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with context menu active -> no action");

    // 6. RC_LOGIN with popup overlay (d.b == true) -> no action
    sim.tick(10000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, true, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with popup overlay -> no action");

    // 7. RC_LOGIN with null center command -> no action
    let null_cmd_screen = MockLoginScreen {
        center_cmd: None,
        username_text: Some("hero".to_string()),
        password_text: Some("pass".to_string()),
    };
    sim.tick(10000, Some("login"), Some(&null_cmd_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with null command -> no action");

    // 8. RC_LOGIN with command id != 0 -> no action
    let wrong_id_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 1, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("hero".to_string()),
        password_text: Some("pass".to_string()),
    };
    sim.tick(10000, Some("login"), Some(&wrong_id_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with cmd_id != 0 -> no action");

    // 9. RC_LOGIN with unexpected caption -> no action
    let wrong_caption_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Dang Ky".to_string() }),
        username_text: Some("hero".to_string()),
        password_text: Some("pass".to_string()),
    };
    sim.tick(10000, Some("login"), Some(&wrong_caption_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with unexpected caption -> no action");

    // 10. RC_LOGIN with empty normal username -> no action
    let empty_user_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("".to_string()),
        password_text: Some("pass".to_string()),
    };
    sim.tick(10000, Some("login"), Some(&empty_user_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with empty username -> no action");

    // 11. RC_LOGIN with empty normal password -> no action
    let empty_pass_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("hero".to_string()),
        password_text: Some("".to_string()),
    };
    sim.tick(10000, Some("login"), Some(&empty_pass_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "RC_LOGIN with empty password -> no action");

    // 12. RC_SERVER / RC_CHARACTER / RC_WORLD_SETTLE / RC_OTHER -> no R2B2 action
    sim.tick(10000, Some("login"), Some(&valid_login_screen), false, "", false, false, false, false, true, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcServer);
    assert_eq!(sim.dispatches.len(), 0, "RC_SERVER -> no action");

    sim.tick(10000, Some("character"), Some(&valid_login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcCharacter);
    assert_eq!(sim.dispatches.len(), 0, "RC_CHARACTER -> no action");
}

#[test]
fn test_login_timing_and_bounded_cycle() {
    let mut sim = ReconnectLoginSimulator::new();
    let login_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("player1".to_string()),
        password_text: Some("secret".to_string()),
    };

    // Prior world
    sim.tick(1000, Some("world"), None, false, "", false, false, false, false, false, false, true, true);
    // Disconnect -> open episode
    sim.tick(2000, Some("world"), None, true, "mat ket noi", false, false, false, false, false, false, false, false);
    assert!(sim.active);

    // Enter RC_LOGIN at now=3000
    sim.tick(3000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcLogin);

    // 1. Dwell check: dwell < 5000 ms (now=7000, elapsed=4000) -> no action
    sim.tick(7000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 0, "Dwell < 5000 ms -> no action");

    // 2. First action at now=8000 (dwell = 5000 ms) -> exactly 1 action
    sim.tick(8000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 1, "First eligible action dispatches");
    assert_eq!(sim.login_attempts, 1);
    assert_eq!(sim.dispatches[0].attempt, 1);
    assert_eq!(sim.dispatches[0].cmd_id, 0);

    // 3. Second action < 30000 ms (now=20000, elapsed=12000) -> prohibited
    sim.tick(20000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Second action within 30000 ms prohibited");

    // 4. Second action >= 30000 ms (now=38000, elapsed=30000) -> permitted
    sim.tick(38000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 2, "Second action permitted at 30000 ms");
    assert_eq!(sim.login_attempts, 2);
    assert_eq!(sim.login_backoff_until, 0, "Backoff not armed before third action");

    // 5. Third action >= 30000 ms (now=68000) -> arms 600000 ms backoff
    sim.tick(68000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 3, "Third action permitted");
    assert_eq!(sim.login_attempts, 3);
    assert_eq!(sim.login_backoff_until, 68000 + 600000, "Third action arms 600000 ms backoff");
    assert_eq!(sim.dispatches[2].armed_backoff_until, 68000 + 600000);

    // 6. No action during 600000 ms backoff (now=200000)
    sim.tick(200000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 3, "No action during backoff");

    // 7. Backoff expiry at now=668001 resets attempts and permits new cycle action
    sim.tick(668001, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 4, "Action permitted after backoff expiry");
    assert_eq!(sim.login_attempts, 1, "Attempts reset to 1 for new bounded cycle");
    assert_eq!(sim.dispatches[3].attempt, 1);

    // 8. Successful world close resets all login action state
    sim.tick(675000, Some("world"), None, false, "", false, false, false, false, false, false, true, true);
    assert!(!sim.active);
    assert_eq!(sim.login_attempts, 0);
    assert_eq!(sim.login_last_action_at, 0);
    assert_eq!(sim.login_backoff_until, 0);
}

#[test]
fn test_login_clock_rollback_safety() {
    let mut sim = ReconnectLoginSimulator::new();
    let login_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("player1".to_string()),
        password_text: Some("secret".to_string()),
    };

    sim.tick(10000, Some("world"), None, false, "", false, false, false, false, false, false, true, true);
    sim.tick(20000, Some("world"), None, true, "mat ket noi", false, false, false, false, false, false, false, false);
    sim.tick(25000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);

    // First action at now=30000 (dwell 5000ms)
    sim.tick(30000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 1);

    // Clock rollback: now jumps back to 20000
    sim.tick(20000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Clock rollback must not trigger extra login action");

    // Interval after re-anchor: requires 30000 ms from re-anchored timestamp (20000 + 30000 = 50000)
    sim.tick(45000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Still within retry interval after rollback");

    sim.tick(50001, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    assert_eq!(sim.dispatches.len(), 2, "Action permitted once interval elapses after rollback");
}

#[test]
fn test_r2b1_and_r2b2_budgets_are_independent() {
    let mut sim = ReconnectLoginSimulator::new();
    let login_screen = MockLoginScreen {
        center_cmd: Some(MockLoginButton { cmd_id: 0, caption: "Chơi Tiếp".to_string() }),
        username_text: Some("player1".to_string()),
        password_text: Some("secret".to_string()),
    };

    // Prior world & disconnect
    sim.tick(1000, Some("world"), None, false, "", false, false, false, false, false, false, true, true);
    sim.tick(2000, Some("world"), None, true, "mat ket noi", false, false, false, false, false, false, false, false);

    // Simulate R2B1 dialog backoff active
    sim.recovery_attempts = 4;
    sim.recovery_backoff_until = 2000 + 600000;

    // UI progresses to clean RC_LOGIN
    sim.tick(10000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);
    // Dwell >= 5000 ms
    sim.tick(15000, Some("login"), Some(&login_screen), false, "", false, false, false, false, false, false, false, false);

    // R2B1 dialog backoff must NOT prevent safe R2B2 login action
    assert_eq!(sim.dispatches.len(), 1, "R2B1 dialog backoff does not block safe RC_LOGIN action");
    assert_eq!(sim.login_attempts, 1);
    assert_eq!(sim.recovery_attempts, 4, "R2B1 dialog recovery counter unchanged");
}

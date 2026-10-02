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
pub struct DispatchedAction {
    pub timestamp: i64,
    pub attempt: i32,
    pub cmd_id: i32,
    pub caption: String,
    pub armed_backoff_until: i64,
}

pub struct ReconnectRecoverySimulator {
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

    // Recovery action state
    pub attempts: i32,
    pub last_action_at: i64,
    pub backoff_until: i64,
    pub last_fingerprint: String,

    // Recorded dispatches
    pub dispatches: Vec<DispatchedAction>,
}

impl ReconnectRecoverySimulator {
    pub const RC_RECOVERY_COOLDOWN_MS: i64 = 5000;
    pub const RC_RECOVERY_MAX_ACTIONS: i32 = 4;
    pub const RC_RECOVERY_BACKOFF_MS: i64 = 600000;
    pub const RC_NATIVE_GRACE_MS: i64 = 5000;
    pub const RC_STATE_DWELL_MS: i64 = 5000;

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

            attempts: 0,
            last_action_at: 0,
            backoff_until: 0,
            last_fingerprint: String::new(),

            dispatches: Vec::new(),
        }
    }

    pub fn normalize(s: &str) -> String {
        s.to_lowercase()
            .replace(['\r', '\n', '\t'], " ")
            .trim()
            .to_string()
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

    pub fn is_connecting_wait(dialog: Option<&MockDialog>) -> bool {
        if let Some(d) = dialog {
            let norm_txt = Self::normalize(&d.text);
            if norm_txt.contains("dang ket noi")
                || norm_txt.contains("vui long cho")
                || norm_txt.contains("cho ket noi")
            {
                return true;
            }
        }
        false
    }

    pub fn find_reconnect_ok_button<'a>(dialog: &'a MockDialog) -> Option<&'a MockButton> {
        if !dialog.is_ah {
            return None;
        }
        for btn in &dialog.buttons {
            if btn.cmd_id == 0 {
                let cap = Self::normalize(&btn.caption);
                if cap == "ok" || cap == "o k" {
                    return Some(btn);
                }
            }
        }
        None
    }

    pub fn tick(
        &mut self,
        now: i64,
        screen: Option<&str>,
        dialog: Option<&MockDialog>,
        bv_a: bool,
        bv_b: i64,
        game_ready: bool,
        map_stable: bool,
    ) {
        // Clock rollback defensively re-anchors
        if now < self.started_at {
            self.started_at = now;
        }
        if now < self.state_since {
            self.state_since = now;
        }
        if now < self.last_action_at {
            if self.backoff_until > self.last_action_at {
                self.backoff_until = now + Self::RC_RECOVERY_BACKOFF_MS;
            }
            self.last_action_at = now;
        }

        // Normal prior stable gameplay sets ever_stable_world_seen outside active episode
        if !self.active && screen == Some("world") && game_ready && map_stable {
            self.ever_stable_world_seen = true;
        }

        let (strong_disconnect, reason) = Self::is_strong_disconnect(dialog, bv_a);

        // Episode Opening
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

                // Reset per-episode action state on open
                self.attempts = 0;
                self.backoff_until = 0;
                self.last_fingerprint.clear();
            }
            return;
        }

        // Active Episode Handling
        if screen.is_none() {
            return;
        }

        let current_screen = screen.unwrap();

        // Successful close rule
        if !strong_disconnect && current_screen == "world" && game_ready && map_stable {
            self.ever_stable_world_seen = true;
            self.active = false;
            self.state = ReconnectState::RcIdle;
            self.state_since = now;
            self.transitions += 1;

            // Reset per-episode action state on successful close
            self.attempts = 0;
            self.backoff_until = 0;
            self.last_fingerprint.clear();
            return;
        }

        // Derive state for active episode
        let target_state = if strong_disconnect {
            ReconnectState::RcNativeWait
        } else if current_screen == "server" {
            ReconnectState::RcServer
        } else if current_screen == "login" {
            ReconnectState::RcLogin
        } else if current_screen == "character" {
            ReconnectState::RcCharacter
        } else if current_screen == "world" {
            ReconnectState::RcWorldSettle
        } else {
            ReconnectState::RcOther
        };

        if target_state != self.state {
            self.state = target_state;
            self.state_since = now;
            self.transitions += 1;
        }

        // R2B1 Native Dialog Fallback Recovery Evaluation
        self.evaluate_recovery_action(now, dialog, bv_a, bv_b);
    }

    fn evaluate_recovery_action(
        &mut self,
        now: i64,
        dialog: Option<&MockDialog>,
        bv_a: bool,
        bv_b: i64,
    ) {
        // Authorization gates
        if !self.active || self.state != ReconnectState::RcNativeWait {
            return;
        }
        if !self.world_seen_before_episode {
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
        if Self::is_connecting_wait(Some(dlg)) {
            return;
        }

        let ok_btn = match Self::find_reconnect_ok_button(dlg) {
            Some(b) => b.clone(),
            None => return,
        };

        // Native Deadline & Grace Policy
        if bv_a && bv_b > 0 {
            if now < bv_b + Self::RC_NATIVE_GRACE_MS {
                return;
            }
        }
        let dwell = if now >= self.state_since {
            now - self.state_since
        } else {
            0
        };
        if dwell < Self::RC_STATE_DWELL_MS {
            return;
        }

        // Cooldown check
        if self.last_action_at > 0 {
            let elapsed = if now >= self.last_action_at {
                now - self.last_action_at
            } else {
                -1
            };
            if elapsed < Self::RC_RECOVERY_COOLDOWN_MS {
                return;
            }
        }

        // Backoff check
        if self.backoff_until > 0 {
            if now < self.backoff_until {
                return;
            } else {
                // Backoff expired; reset attempts for new bounded cycle
                self.backoff_until = 0;
                self.attempts = 0;
            }
        }

        // Arm accounting BEFORE dispatch
        self.attempts += 1;
        self.last_action_at = now;
        if self.attempts >= Self::RC_RECOVERY_MAX_ACTIONS {
            self.backoff_until = now + Self::RC_RECOVERY_BACKOFF_MS;
        }

        self.dispatches.push(DispatchedAction {
            timestamp: now,
            attempt: self.attempts,
            cmd_id: ok_btn.cmd_id,
            caption: ok_btn.caption.clone(),
            armed_backoff_until: self.backoff_until,
        });
    }
}

// ---------------------------------------------------------------------------
// SOURCE CONTRACT VERIFICATION
// ---------------------------------------------------------------------------

#[test]
fn test_reconnect_recovery_java_source_contract() {
    let zeus_java_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path)
        .expect("Zeus.java must exist and be readable");

    let section_start = content.find("// ---- RECONNECT")
        .expect("RECONNECT section must exist");
    let section_end = content[section_start..]
        .find("// ---- end RECONNECT")
        .expect("end RECONNECT marker must exist");
    let supervisor_body = &content[section_start..section_start + section_end];

    // 1. Constants verification
    assert!(supervisor_body.contains("RC_RECOVERY_COOLDOWN_MS = 5000"), "Cooldown constant must be 5000 ms");
    assert!(supervisor_body.contains("RC_RECOVERY_MAX_ACTIONS = 4"), "Max actions constant must be 4");
    assert!(supervisor_body.contains("RC_RECOVERY_BACKOFF_MS = 600000"), "Backoff constant must be 600000 ms");

    // 2. Action state fields
    let action_fields = [
        "reconnectRecoveryAttempts",
        "reconnectRecoveryLastActionAt",
        "reconnectRecoveryBackoffUntil",
        "reconnectRecoveryLastFingerprint",
    ];
    for f in action_fields {
        assert!(supervisor_body.contains(f), "Zeus.java must define field: {f}");
    }

    // 3. Safe button helper
    assert!(
        supervisor_body.contains("findReconnectOkButton"),
        "Zeus.java must define findReconnectOkButton helper"
    );

    // 4. Safe dispatch: live bt.a() only, no direct b(0,0), no b(6,0)
    assert!(supervisor_body.contains(".a();"), "Recovery action must invoke live button .a()");
    assert!(!supervisor_body.contains(".b(0, 0)") && !supervisor_body.contains(".b(0,0)"),
        "Must NOT call direct .b(0,0)");
    assert!(!supervisor_body.contains(".b(6, 0)") && !supervisor_body.contains(".b(6,0)"),
        "Must NOT call direct .b(6,0)");
    assert!(!supervisor_body.contains("fu.s.b("), "Must NOT call fu.s.b()");
    assert!(!supervisor_body.contains("ah.b("), "Must NOT call ah.b()");

    // 5. Accounting before dispatch: verify attempts/lastActionAt set before dispatch call
    let dispatch_pos = supervisor_body.find(".a();").expect("Live button dispatch must exist");
    let attempts_inc_pos = supervisor_body.find("reconnectRecoveryAttempts++")
        .or_else(|| supervisor_body.find("++reconnectRecoveryAttempts"))
        .expect("Attempts increment must exist in supervisor body");
    assert!(
        attempts_inc_pos < dispatch_pos,
        "Action accounting (attempts++) must be armed BEFORE button .a() dispatch"
    );

    // 6. Reset on episode open & successful close
    assert!(
        supervisor_body.contains("reconnectRecoveryAttempts = 0;"),
        "reconnectRecoveryAttempts must be reset"
    );
}

// ---------------------------------------------------------------------------
// BEHAVIORAL CONTRACT TESTS
// ---------------------------------------------------------------------------

#[test]
fn test_authorization_matrix() {
    let mut sim = ReconnectRecoverySimulator::new();

    let valid_ok_dialog = MockDialog {
        is_ah: true,
        text: "Mat ket noi voi may chu".to_string(),
        buttons: vec![
            MockButton { cmd_id: 0, caption: "OK".to_string() },
            MockButton { cmd_id: 6, caption: "Thoat".to_string() },
        ],
    };

    // 1. No action when no active episode
    sim.tick(1000, Some("world"), Some(&valid_ok_dialog), false, 0, true, true);
    assert_eq!(sim.dispatches.len(), 0, "No action when no active episode");

    // Establish prior stable world
    sim.tick(2000, Some("world"), None, false, 0, true, true);
    assert!(sim.ever_stable_world_seen);

    // Open episode via bv.a with native deadline in future (now=3000, deadline=33000)
    sim.tick(3000, Some("world"), Some(&valid_ok_dialog), true, 33000, false, false);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);
    assert!(sim.world_seen_before_episode);

    // 2. No action when native deadline in future
    sim.tick(10000, Some("world"), Some(&valid_ok_dialog), true, 33000, false, false);
    assert_eq!(sim.dispatches.len(), 0, "No action while native deadline in future");

    // 3. No action when native deadline passed but 5000 ms grace not elapsed (deadline 33000, now 35000 < 38000)
    sim.tick(35000, Some("world"), Some(&valid_ok_dialog), true, 33000, false, false);
    assert_eq!(sim.dispatches.len(), 0, "No action while grace period not elapsed");

    // 4. Eligible after deadline + grace + dwell (now = 38001)
    sim.tick(38001, Some("world"), Some(&valid_ok_dialog), true, 33000, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Eligible after native deadline + grace + dwell");
    assert_eq!(sim.dispatches[0].attempt, 1);
    assert_eq!(sim.dispatches[0].cmd_id, 0);

    // 5. No action when reconnectWorldSeenBeforeEpisode = false (new sim without prior stable world)
    let mut sim_no_prior = ReconnectRecoverySimulator::new();
    sim_no_prior.tick(1000, Some("world"), Some(&valid_ok_dialog), true, 0, false, false);
    assert!(sim_no_prior.active);
    assert_eq!(sim_no_prior.world_seen_before_episode, false);
    sim_no_prior.tick(10000, Some("world"), Some(&valid_ok_dialog), true, 0, false, false);
    assert_eq!(sim_no_prior.dispatches.len(), 0, "No action if no prior stable world");

    // 6. No action in other screens/states (RC_LOGIN, RC_SERVER, RC_CHARACTER, RC_WORLD_SETTLE, RC_OTHER)
    let mut sim_screens = ReconnectRecoverySimulator::new();
    sim_screens.tick(1000, Some("world"), None, false, 0, true, true);
    sim_screens.tick(2000, Some("world"), Some(&valid_ok_dialog), true, 0, false, false);
    // Transition to login (dialog dismissed or absent)
    sim_screens.tick(8000, Some("login"), None, false, 0, false, false);
    assert_eq!(sim_screens.state, ReconnectState::RcLogin);
    assert_eq!(sim_screens.dispatches.len(), 0, "No action in RC_LOGIN");

    // 7. No action when dialog is absent or not ah
    let non_ah_dialog = MockDialog {
        is_ah: false,
        text: "Mat ket noi".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };
    let mut sim_non_ah = ReconnectRecoverySimulator::new();
    sim_non_ah.tick(1000, Some("world"), None, false, 0, true, true);
    sim_non_ah.tick(2000, Some("world"), None, true, 0, false, false);
    sim_non_ah.tick(8000, Some("world"), Some(&non_ah_dialog), true, 0, false, false);
    assert_eq!(sim_non_ah.dispatches.len(), 0, "No action for non-ah dialog");

    // 8. No action for generic connecting/wait dialog
    let wait_dialog = MockDialog {
        is_ah: true,
        text: "Dang ket noi voi may chu...".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };
    let mut sim_wait = ReconnectRecoverySimulator::new();
    sim_wait.tick(1000, Some("world"), None, false, 0, true, true);
    sim_wait.tick(2000, Some("world"), None, true, 0, false, false);
    sim_wait.tick(8000, Some("world"), Some(&wait_dialog), true, 0, false, false);
    assert_eq!(sim_wait.dispatches.len(), 0, "No action for generic connecting/wait dialog");
}

#[test]
fn test_safe_button_validation_matrix() {
    // 1. Exact cmd 0 + "OK" is accepted
    let d1 = MockDialog {
        is_ah: true,
        text: "Mat ket noi".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };
    assert!(ReconnectRecoverySimulator::find_reconnect_ok_button(&d1).is_some());

    // 2. Exact cmd 0 + "o k" is accepted
    let d2 = MockDialog {
        is_ah: true,
        text: "Mat ket noi".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "o k".to_string() }],
    };
    assert!(ReconnectRecoverySimulator::find_reconnect_ok_button(&d2).is_some());

    // 3. Command 0 with unrelated caption rejected
    let d3 = MockDialog {
        is_ah: true,
        text: "Mat ket noi".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "Dong y".to_string() }],
    };
    assert!(ReconnectRecoverySimulator::find_reconnect_ok_button(&d3).is_none());

    // 4. Command 6 rejected even if caption is OK
    let d4 = MockDialog {
        is_ah: true,
        text: "Mat ket noi".to_string(),
        buttons: vec![MockButton { cmd_id: 6, caption: "OK".to_string() }],
    };
    assert!(ReconnectRecoverySimulator::find_reconnect_ok_button(&d4).is_none());

    // 5. First-button fallback forbidden: if first button is Thoat (cmd 6) and second is OK (cmd 0), finds OK
    let d5 = MockDialog {
        is_ah: true,
        text: "Mat ket noi".to_string(),
        buttons: vec![
            MockButton { cmd_id: 6, caption: "Thoat".to_string() },
            MockButton { cmd_id: 0, caption: "OK".to_string() },
        ],
    };
    let btn = ReconnectRecoverySimulator::find_reconnect_ok_button(&d5).unwrap();
    assert_eq!(btn.cmd_id, 0);
    assert_eq!(btn.caption, "OK");
}

#[test]
fn test_bounded_retry_and_backoff_cycle() {
    let mut sim = ReconnectRecoverySimulator::new();
    let dialog = MockDialog {
        is_ah: true,
        text: "Ket noi that bai".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };

    // Prior world
    sim.tick(1000, Some("world"), None, false, 0, true, true);
    // Open episode without native deadline
    sim.tick(2000, Some("world"), Some(&dialog), false, 0, false, false);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);

    // State dwell 5000 ms: at now=7000, action 1 occurs
    sim.tick(7000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 1);
    assert_eq!(sim.attempts, 1);
    assert_eq!(sim.dispatches[0].attempt, 1);

    // Inside cooldown (5000 ms): at now=9000, no action
    sim.tick(9000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Cooldown prevents action at 2000ms");

    // Action 2 at now=12000 (>= 5000 ms cooldown)
    sim.tick(12000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 2);
    assert_eq!(sim.attempts, 2);

    // Action 3 at now=17000
    sim.tick(17000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 3);
    assert_eq!(sim.attempts, 3);
    assert_eq!(sim.backoff_until, 0, "Backoff not armed before 4th action");

    // Action 4 at now=22000: armed with 600000 ms backoff
    sim.tick(22000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 4);
    assert_eq!(sim.attempts, 4);
    assert_eq!(sim.backoff_until, 22000 + 600000, "4th action arms 600000 ms backoff");
    assert_eq!(sim.dispatches[3].armed_backoff_until, 22000 + 600000);

    // During backoff: now=100000, no action
    sim.tick(100000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 4, "No actions during backoff");

    // At backoff expiry: now=622000
    sim.tick(622001, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 5, "Backoff expiry allows new cycle action");
    assert_eq!(sim.attempts, 1, "Attempts reset for new cycle");
    assert_eq!(sim.dispatches[4].attempt, 1);

    // Successful close resets recovery state
    sim.tick(630000, Some("world"), None, false, 0, true, true);
    assert!(!sim.active);
    assert_eq!(sim.attempts, 0);
    assert_eq!(sim.backoff_until, 0);
}

#[test]
fn test_clock_rollback_safety() {
    let mut sim = ReconnectRecoverySimulator::new();
    let dialog = MockDialog {
        is_ah: true,
        text: "Mat ket noi voi may chu".to_string(),
        buttons: vec![MockButton { cmd_id: 0, caption: "OK".to_string() }],
    };

    sim.tick(10000, Some("world"), None, false, 0, true, true);
    sim.tick(20000, Some("world"), Some(&dialog), false, 0, false, false);
    sim.tick(25000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 1);

    // Clock rollback: now jumps back to 15000
    sim.tick(15000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Clock rollback must not trigger immediate action");

    // Dwell after re-anchor: requires 5000 ms from re-anchored timestamp (15000 + 5000 = 20000)
    sim.tick(19000, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 1, "Still within cooldown after rollback");

    sim.tick(20001, Some("world"), Some(&dialog), false, 0, false, false);
    assert_eq!(sim.dispatches.len(), 2, "Action permitted once cooldown elapses after rollback");
}

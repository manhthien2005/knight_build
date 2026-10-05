use std::fs;
use tempfile::TempDir;

#[path = "../src/launch.rs"]
#[allow(dead_code)]
mod launch;

mod spot_scan {
    pub const SPOT_REQUEST_FILE_NAME: &str = "zeus-spot-req.json";
    pub const SPOT_RESULT_PAYLOAD_FILE_NAME: &str = "zeus-spot-payload.json";
    pub const SPOT_RESULT_READY_FILE_NAME: &str = "zeus-spot-ready.txt";
}
mod inventory {
    pub const INVENTORY_FILE_NAME: &str = "zeus-inventory.json";
}
mod enhancement {
    pub const ENHANCE_REQUEST_FILE_NAME: &str = "zeus-enhance-req.json";
    pub const ENHANCE_STATUS_FILE_NAME: &str = "zeus-enhance-status.json";
    pub const ENHANCE_CANCEL_FILE_NAME: &str = "zeus-enhance-cancel.txt";
}

/// Strictly parses and validates a server index against zeus_core::SERVER_COUNT (9).
/// Rejects negative numbers and values >= SERVER_COUNT (9..). Never clamps.
pub fn parse_strict_server_index(raw: i64) -> Result<u8, &'static str> {
    if (0..zeus_core::SERVER_COUNT as i64).contains(&raw) {
        Ok(raw as u8)
    } else {
        Err("server_index out of range (must be 0..8)")
    }
}

/// Transition state evaluated when stopping a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopTransition {
    ConfirmedStopped,
    FailedStillAlive,
}

/// JVM-startup identity fields defining managed runtime credentials and immutable server target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedIdentity {
    pub server_index: u8,
    pub username: String,
    pub secret_sealed: serde_json::Value,
}

impl ManagedIdentity {
    pub fn new(server_index: u8, username: impl Into<String>, secret_sealed: serde_json::Value) -> Self {
        Self {
            server_index,
            username: username.into(),
            secret_sealed,
        }
    }

    pub fn has_changed_from(
        &self,
        target_server: u8,
        target_username: &str,
        target_sealed: &serde_json::Value,
    ) -> bool {
        self.server_index != target_server
            || self.username != target_username
            || &self.secret_sealed != target_sealed
    }
}

/// Action determined by evaluating incoming cloud AccountChanged against current account state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityChangeAction {
    /// Server index was invalid (<0 or >=SERVER_COUNT); rejected, no state change, no restart.
    RejectInvalidServerIndex { raw_index: i64 },
    /// No identity field changed (e.g. control-only, config-only, or identical identity fields).
    NoIdentityChange,
    /// Identity changed, but no running process was alive; state updated directly, normal reconcile.
    ApplyWithoutRestart {
        target_server: u8,
        target_username: String,
        target_sealed: serde_json::Value,
    },
    /// Identity changed while running process was alive; must stop running process before applying replacement.
    StopRunningProcessForReplacement {
        old_identity: ManagedIdentity,
        target_server: u8,
        target_username: String,
        target_sealed: serde_json::Value,
    },
}

/// Evaluates incoming account change intent against current managed identity and process state.
pub fn evaluate_account_change_intent(
    current_identity: &ManagedIdentity,
    is_process_alive: bool,
    incoming_server_index_raw: Option<i64>,
    incoming_username: Option<&str>,
    incoming_secret_sealed: Option<&serde_json::Value>,
) -> IdentityChangeAction {
    let target_server = if let Some(raw_si) = incoming_server_index_raw {
        match parse_strict_server_index(raw_si) {
            Ok(si) => si,
            Err(_) => return IdentityChangeAction::RejectInvalidServerIndex { raw_index: raw_si },
        }
    } else {
        current_identity.server_index
    };

    let target_un = incoming_username.unwrap_or(&current_identity.username);
    let target_sealed = incoming_secret_sealed.unwrap_or(&current_identity.secret_sealed);

    if !current_identity.has_changed_from(target_server, target_un, target_sealed) {
        return IdentityChangeAction::NoIdentityChange;
    }

    if is_process_alive {
        IdentityChangeAction::StopRunningProcessForReplacement {
            old_identity: current_identity.clone(),
            target_server,
            target_username: target_un.to_string(),
            target_sealed: target_sealed.clone(),
        }
    } else {
        IdentityChangeAction::ApplyWithoutRestart {
            target_server,
            target_username: target_un.to_string(),
            target_sealed: target_sealed.clone(),
        }
    }
}

/// Resolution of a stop attempt during a managed identity replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementStopResolution {
    /// Old process stopped successfully; proceed with RMS clear, identity update, and new launch.
    ProceedWithReplacement,
    /// Old process failed to stop; retain process handle, do NOT reseed RMS, do NOT launch second JVM.
    AbortRetainProcess,
}

/// Evaluates whether an identity replacement may proceed based on the stop transition outcome.
pub fn evaluate_replacement_stop(transition: StopTransition) -> ReplacementStopResolution {
    match transition {
        StopTransition::ConfirmedStopped => ReplacementStopResolution::ProceedWithReplacement,
        StopTransition::FailedStillAlive => ReplacementStopResolution::AbortRetainProcess,
    }
}

#[derive(Debug, Clone)]
struct SyntheticProcess {
    pid: u32,
    is_alive: bool,
    canonical_host: String,
    username: String,
}

struct SyntheticSlotSupervisor {
    slot_index: i32,
    home: TempDir,
    desired_state: String,
    server_index: u8,
    username: String,
    secret_sealed: serde_json::Value,
    process: Option<SyntheticProcess>,
    next_pid: u32,

    // Lifecycle audit telemetry
    stop_call_count: usize,
    spawn_call_count: usize,
    reseed_call_count: usize,
    error_status: Option<String>,
    events: Vec<String>,
}

impl SyntheticSlotSupervisor {
    fn new(slot_index: i32, server_index: u8, username: &str, desired_state: &str) -> Self {
        let home = TempDir::new().expect("create temp home");
        Self {
            slot_index,
            home,
            desired_state: desired_state.to_string(),
            server_index,
            username: username.to_string(),
            secret_sealed: serde_json::json!({"password": "initial_password"}),
            process: None,
            next_pid: 1000,
            stop_call_count: 0,
            spawn_call_count: 0,
            reseed_call_count: 0,
            error_status: None,
            events: Vec::new(),
        }
    }

    fn spawn_initial(&mut self) {
        assert!(self.process.is_none());
        let pid = self.next_pid;
        self.next_pid += 1;

        // Seed initial RMS
        let pass = self.secret_sealed["password"].as_str().unwrap_or("pass");
        zeus_core::wire::seed_credentials(self.home.path(), &self.username, pass, self.server_index)
            .expect("seed initial rms");
        self.reseed_call_count += 1;

        let spec_server = zeus_core::server_spec(self.server_index).expect("valid server spec");
        self.process = Some(SyntheticProcess {
            pid,
            is_alive: true,
            canonical_host: spec_server.host.to_string(),
            username: self.username.clone(),
        });
        self.spawn_call_count += 1;
        self.events.push(format!("spawn:pid={pid}:server={}", self.server_index));
    }

    fn handle_cloud_account_changed(
        &mut self,
        record: &serde_json::Value,
        stop_transition_outcome: StopTransition,
    ) {
        let current_id = ManagedIdentity::new(
            self.server_index,
            &self.username,
            self.secret_sealed.clone(),
        );

        let incoming_raw_si = record.get("server_index").and_then(|v| v.as_i64());
        let incoming_un = record.get("username").and_then(|v| v.as_str());
        let incoming_sealed = record.get("secret_sealed");

        let is_alive = self.process.as_ref().map(|p| p.is_alive).unwrap_or(false);

        let intent = evaluate_account_change_intent(
            &current_id,
            is_alive,
            incoming_raw_si,
            incoming_un,
            incoming_sealed,
        );

        if let Some(ds) = record["desired_state"].as_str() {
            self.desired_state = ds.to_string();
        }

        match intent {
            IdentityChangeAction::RejectInvalidServerIndex { raw_index } => {
                self.error_status = Some(format!("invalid server_index {raw_index}"));
                self.events.push(format!("reject_invalid_server_index:{raw_index}"));
            }
            IdentityChangeAction::NoIdentityChange => {
                self.events.push("no_identity_change".to_string());
            }
            IdentityChangeAction::ApplyWithoutRestart {
                target_server,
                target_username,
                target_sealed,
            } => {
                self.server_index = target_server;
                self.username = target_username;
                self.secret_sealed = target_sealed;
                self.events.push(format!("apply_without_restart:server={target_server}"));

                if self.desired_state == "stopped" {
                    let _ = zeus_core::wire::clear_credentials(self.home.path());
                } else if self.desired_state == "running" && !is_alive {
                    // Reconcile spawns since no process is alive
                    let pass = self.secret_sealed["password"].as_str().unwrap_or("pass");
                    zeus_core::wire::seed_credentials(
                        self.home.path(),
                        &self.username,
                        pass,
                        self.server_index,
                    )
                    .expect("seed rms");
                    self.reseed_call_count += 1;
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    let spec_server = zeus_core::server_spec(self.server_index).unwrap();
                    self.process = Some(SyntheticProcess {
                        pid,
                        is_alive: true,
                        canonical_host: spec_server.host.to_string(),
                        username: self.username.clone(),
                    });
                    self.spawn_call_count += 1;
                    self.events.push(format!("spawn:pid={pid}:server={}", self.server_index));
                }
            }
            IdentityChangeAction::StopRunningProcessForReplacement {
                target_server,
                target_username,
                target_sealed,
                ..
            } => {
                self.stop_call_count += 1;
                self.events.push("stop_invoked".to_string());

                match evaluate_replacement_stop(stop_transition_outcome) {
                    ReplacementStopResolution::ProceedWithReplacement => {
                        // Confirm old process dead before anything else
                        if let Some(ref mut proc) = self.process {
                            proc.is_alive = false;
                        }
                        self.events.push("old_process_confirmed_stopped".to_string());

                        // Clear credentials before new identity appears
                        zeus_core::wire::clear_credentials(self.home.path()).unwrap();
                        self.events.push("old_credentials_cleared".to_string());

                        // Old process handle dropped
                        self.process = None;

                        // Identity in state updated only after old process confirmed stopped
                        self.server_index = target_server;
                        self.username = target_username;
                        self.secret_sealed = target_sealed;
                        self.events.push(format!("state_identity_updated:server={target_server}"));

                        // Reconcile desired state: seed new RMS and launch replacement
                        let pass = self.secret_sealed["password"].as_str().unwrap_or("pass");
                        zeus_core::wire::seed_credentials(
                            self.home.path(),
                            &self.username,
                            pass,
                            self.server_index,
                        )
                        .expect("seed replacement rms");
                        self.reseed_call_count += 1;
                        self.events.push(format!("new_rms_seeded:server={target_server}"));

                        let pid = self.next_pid;
                        self.next_pid += 1;
                        let spec_server = zeus_core::server_spec(self.server_index).unwrap();
                        self.process = Some(SyntheticProcess {
                            pid,
                            is_alive: true,
                            canonical_host: spec_server.host.to_string(),
                            username: self.username.clone(),
                        });
                        self.spawn_call_count += 1;
                        self.events.push(format!("replacement_spawned:pid={pid}:server={target_server}"));
                    }
                    ReplacementStopResolution::AbortRetainProcess => {
                        self.error_status = Some("identity change replacement failed: old process could not be confirmed stopped".to_string());
                        self.events.push("stop_failed_abort_retain_process".to_string());
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase 7 & Phase 3 Synthetic Lifecycle Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn running_account_server_6_to_8_triggers_exactly_one_controlled_replacement() {
    let mut sup = SyntheticSlotSupervisor::new(1, 6, "player_hero", "running");
    sup.spawn_initial();
    assert_eq!(sup.spawn_call_count, 1);
    assert_eq!(sup.stop_call_count, 0);
    assert_eq!(sup.process.as_ref().unwrap().canonical_host, "hs7.teamobi.com");

    let cloud_event = serde_json::json!({
        "server_index": 8,
        "username": "player_hero"
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    // Assert exactly one replacement stop and spawn occurred
    assert_eq!(sup.stop_call_count, 1, "exactly one stop must be called");
    assert_eq!(sup.spawn_call_count, 2, "initial + replacement = 2 spawns total");
    assert_eq!(sup.server_index, 8);

    // Verify replacement JVM canonical host
    let current_proc = sup.process.as_ref().unwrap();
    assert_eq!(current_proc.canonical_host, "hs8.teamobi.com");
    assert_eq!(current_proc.pid, 1001);
    assert!(current_proc.is_alive);

    // Verify LaunchSpec argv for server 8 (Bạch Hổ)
    let paths = launch::AccountPaths::for_slot(sup.slot_index);
    let mut spec = launch::LaunchSpec::default_for_paths(paths);
    spec.set_managed_server(8).expect("set managed server 8");
    let cmd = spec.command();
    let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
    let host_props: Vec<&String> = args.iter().filter(|a| a.contains("-Dzeus.server.host=")).collect();
    assert_eq!(host_props.len(), 1, "exactly one host property in replacement argv");
    assert_eq!(host_props[0], "-Dzeus.server.host=hs8.teamobi.com");

    // Phase 3 invariants:
    // Verify RMS on disk for Bạch Hổ: selectedServerHost hs8, isIndexServer bootstrap 0
    let suite = sup.home.path().join(".microemulator").join("suite-null");
    let host_file = suite.join("selectedServerHost.rs");
    assert!(host_file.exists());
    let host_bytes = fs::read(&host_file).unwrap_or_default();
    let expected_complemented: Vec<u8> = b"hs8.teamobi.com".iter().map(|b| !b).collect();
    assert!(
        host_bytes.windows(expected_complemented.len()).any(|w| w == expected_complemented.as_slice()),
        "selectedServerHost must contain complemented canonical host hs8.teamobi.com"
    );

    let index_file = suite.join("isIndexServer.rs");
    assert!(index_file.exists());
    let index_bytes = fs::read(&index_file).unwrap_or_default();
    assert!(
        index_bytes.contains(&(!0u8)),
        "isIndexServer for Bạch Hổ must contain complemented bootstrap index 0 (0xFF)"
    );

    // Verify ordering sequence in event trace
    let stop_pos = sup.events.iter().position(|e| e == "old_process_confirmed_stopped").unwrap();
    let clear_pos = sup.events.iter().position(|e| e == "old_credentials_cleared").unwrap();
    let state_pos = sup.events.iter().position(|e| e.starts_with("state_identity_updated")).unwrap();
    let reseed_pos = sup.events.iter().position(|e| e.starts_with("new_rms_seeded")).unwrap();
    let spawn_pos = sup.events.iter().position(|e| e.starts_with("replacement_spawned")).unwrap();

    assert!(stop_pos < clear_pos, "old process confirmed stopped before credentials cleared");
    assert!(clear_pos < state_pos, "credentials cleared before state identity updated");
    assert!(state_pos < reseed_pos, "state updated before new RMS seeded");
    assert!(reseed_pos < spawn_pos, "new RMS seeded before replacement spawned");
}

#[test]
fn running_account_server_8_to_7_triggers_exactly_one_controlled_replacement() {
    let mut sup = SyntheticSlotSupervisor::new(1, 8, "player_hero", "running");
    sup.spawn_initial();
    assert_eq!(sup.process.as_ref().unwrap().canonical_host, "hs8.teamobi.com");

    let cloud_event = serde_json::json!({
        "server_index": 7,
        "username": "player_hero"
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 1);
    assert_eq!(sup.spawn_call_count, 2);
    assert_eq!(sup.server_index, 7);

    let current_proc = sup.process.as_ref().unwrap();
    assert_eq!(current_proc.canonical_host, "hs4.teamobi.com");

    // LaunchSpec for server 7
    let paths = launch::AccountPaths::for_slot(sup.slot_index);
    let mut spec = launch::LaunchSpec::default_for_paths(paths);
    spec.set_managed_server(7).expect("set managed server 7");
    let cmd = spec.command();
    let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
    let host_props: Vec<&String> = args.iter().filter(|a| a.contains("-Dzeus.server.host=")).collect();
    assert_eq!(host_props.len(), 1);
    assert_eq!(host_props[0], "-Dzeus.server.host=hs4.teamobi.com");

    // Phase 3 invariants:
    // Verify RMS for server 7: selectedServerHost hs4, isIndexServer bootstrap 7 (!7 = 248)
    let suite = sup.home.path().join(".microemulator").join("suite-null");
    let host_file = suite.join("selectedServerHost.rs");
    assert!(host_file.exists());
    let host_bytes = fs::read(&host_file).unwrap_or_default();
    let expected_complemented_7: Vec<u8> = b"hs4.teamobi.com".iter().map(|b| !b).collect();
    assert!(
        host_bytes.windows(expected_complemented_7.len()).any(|w| w == expected_complemented_7.as_slice()),
        "selectedServerHost must contain complemented canonical host hs4.teamobi.com"
    );

    let index_file = suite.join("isIndexServer.rs");
    assert!(index_file.exists());
    let index_bytes = fs::read(&index_file).unwrap_or_default();
    assert!(
        index_bytes.contains(&(!7u8)),
        "isIndexServer for server 7 must contain complemented bootstrap index 7"
    );
}

#[test]
fn same_server_repeated_realtime_event_causes_no_restart() {
    let mut sup = SyntheticSlotSupervisor::new(1, 8, "player_hero", "running");
    sup.spawn_initial();

    let cloud_event = serde_json::json!({
        "server_index": 8,
        "username": "player_hero",
        "secret_sealed": {"password": "initial_password"}
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 0, "repeated identical event must not stop JVM");
    assert_eq!(sup.spawn_call_count, 1, "no extra spawn");
    assert_eq!(sup.process.as_ref().unwrap().pid, 1000);
}

#[test]
fn control_only_realtime_event_causes_no_restart() {
    let mut sup = SyntheticSlotSupervisor::new(1, 8, "player_hero", "running");
    sup.spawn_initial();

    let cloud_event = serde_json::json!({
        "control_version": 2,
        "control": {"nav.detectSpots": true}
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 0, "control-only update must not stop JVM");
    assert_eq!(sup.spawn_call_count, 1);
    assert_eq!(sup.process.as_ref().unwrap().pid, 1000);
}

#[test]
fn password_change_causes_controlled_replacement() {
    let mut sup = SyntheticSlotSupervisor::new(1, 8, "player_hero", "running");
    sup.spawn_initial();

    let cloud_event = serde_json::json!({
        "secret_sealed": {"password": "rotated_super_secret_pw"}
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 1, "password change must stop old JVM");
    assert_eq!(sup.spawn_call_count, 2, "replacement spawned");
    assert_eq!(sup.secret_sealed["password"], "rotated_super_secret_pw");
    assert_eq!(sup.process.as_ref().unwrap().pid, 1001);
}

#[test]
fn username_change_causes_controlled_replacement() {
    let mut sup = SyntheticSlotSupervisor::new(1, 8, "player_hero", "running");
    sup.spawn_initial();

    let cloud_event = serde_json::json!({
        "username": "player_legend"
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 1, "username change must stop old JVM");
    assert_eq!(sup.spawn_call_count, 2, "replacement spawned");
    assert_eq!(sup.username, "player_legend");
    assert_eq!(sup.process.as_ref().unwrap().username, "player_legend");
}

#[test]
fn stopped_account_identity_change_does_not_spawn() {
    let mut sup = SyntheticSlotSupervisor::new(1, 6, "player_hero", "stopped");
    // No initial spawn because desired_state is stopped

    let cloud_event = serde_json::json!({
        "server_index": 8,
        "username": "player_hero",
        "desired_state": "stopped"
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 0, "no stop since no live process");
    assert_eq!(sup.spawn_call_count, 0, "stopped account must not spawn");
    assert_eq!(sup.server_index, 8, "state updated to 8");
    assert!(sup.process.is_none());

    // Credentials on disk must be clear
    let suite = sup.home.path().join(".microemulator").join("suite-null");
    assert!(!suite.join("user_pass.rs").exists());
}

#[test]
fn dead_nonexistent_process_identity_change_uses_normal_single_spawn() {
    let mut sup = SyntheticSlotSupervisor::new(1, 6, "player_hero", "running");
    // No process alive currently (e.g. startup or crashed previously)

    let cloud_event = serde_json::json!({
        "server_index": 8,
        "username": "player_hero"
    });

    sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

    assert_eq!(sup.stop_call_count, 0, "no stop invoked when no live process");
    assert_eq!(sup.spawn_call_count, 1, "exactly 1 spawn for new identity");
    assert_eq!(sup.server_index, 8);
    assert_eq!(sup.process.as_ref().unwrap().canonical_host, "hs8.teamobi.com");
}

#[test]
fn stop_failure_produces_no_rms_reseed_and_no_second_process() {
    let mut sup = SyntheticSlotSupervisor::new(1, 6, "player_hero", "running");
    sup.spawn_initial();
    let initial_reseed_count = sup.reseed_call_count;

    let cloud_event = serde_json::json!({
        "server_index": 8,
        "username": "player_hero"
    });

    // Simulate stop failure (e.g. timeout / process group still alive)
    sup.handle_cloud_account_changed(&cloud_event, StopTransition::FailedStillAlive);

    assert_eq!(sup.stop_call_count, 1, "stop was attempted");
    assert_eq!(sup.spawn_call_count, 1, "NO second process spawned");
    assert_eq!(sup.reseed_call_count, initial_reseed_count, "NO RMS reseed permitted");

    // Old process handle retained and still marked alive with server 6
    assert!(sup.process.is_some());
    let proc = sup.process.as_ref().unwrap();
    assert!(proc.is_alive);
    assert_eq!(proc.canonical_host, "hs7.teamobi.com");
    assert_eq!(proc.pid, 1000);

    // Error status recorded
    assert!(sup.error_status.is_some());
    assert!(sup.error_status.as_ref().unwrap().contains("old process could not be confirmed stopped"));
}

#[test]
fn invalid_server_index_produces_no_restart_reseed_to_different_server() {
    let mut sup = SyntheticSlotSupervisor::new(1, 6, "player_hero", "running");
    sup.spawn_initial();

    for invalid_si in [-1i64, 9, 99, 255] {
        let cloud_event = serde_json::json!({
            "server_index": invalid_si,
            "username": "player_hero"
        });

        sup.handle_cloud_account_changed(&cloud_event, StopTransition::ConfirmedStopped);

        assert_eq!(sup.stop_call_count, 0, "must not stop for invalid server_index");
        assert_eq!(sup.spawn_call_count, 1, "no restart");
        assert_eq!(sup.server_index, 6, "server_index must remain 6, never clamp to 8");
        assert_eq!(sup.process.as_ref().unwrap().pid, 1000);
        assert!(sup.error_status.is_some());
        assert!(sup.error_status.as_ref().unwrap().contains("invalid server_index"));
    }
}

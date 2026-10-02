use std::fs;
use std::path::Path;

#[test]
fn test_reconnect_supervisor_java_source_contract() {
    let zeus_java_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path)
        .expect("Zeus.java must exist and be readable");

    // 1. Tick ordering contract:
    // healthSidecarTick() -> if (fu.a == null) -> sessionTick() -> reconnectSupervisorTick() -> auth()
    let tick_pos = content.find("public static void tick()").expect("tick() must exist");
    let tick_body = &content[tick_pos..tick_pos + 1200];

    let health_call_pos = tick_body.find("healthSidecarTick();")
        .expect("healthSidecarTick() must be called in tick()");
    let null_guard_pos = tick_body.find("if (fu.a == null)")
        .expect("if (fu.a == null) guard must exist in tick()");
    let session_tick_pos = tick_body.find("sessionTick();")
        .expect("sessionTick() must be called in tick()");
    let supervisor_call_pos = tick_body.find("reconnectSupervisorTick();")
        .expect("reconnectSupervisorTick() must be called in tick()");
    let auth_call_pos = tick_body.find("auth();")
        .expect("auth() must be called in tick()");

    assert!(health_call_pos < null_guard_pos, "healthSidecarTick must be before null guard");
    assert!(null_guard_pos < session_tick_pos, "sessionTick must be after null guard");
    assert!(session_tick_pos < supervisor_call_pos, "sessionTick must precede reconnectSupervisorTick");
    assert!(supervisor_call_pos < auth_call_pos, "reconnectSupervisorTick must precede auth");

    // 2. Supervisor method and isolation
    let _supervisor_def = content.find("private static void reconnectSupervisorTick()")
        .expect("reconnectSupervisorTick() definition must exist");
    assert!(
        content.contains("// ---- RECONNECT"),
        "Zeus.java must contain an isolated RECONNECT section"
    );

    // 3. States constants: RC_IDLE, RC_NATIVE_WAIT, RC_LOGIN, RC_SERVER, RC_CHARACTER, RC_WORLD_SETTLE, RC_OTHER
    let states = [
        "RC_IDLE",
        "RC_NATIVE_WAIT",
        "RC_LOGIN",
        "RC_SERVER",
        "RC_CHARACTER",
        "RC_WORLD_SETTLE",
        "RC_OTHER",
    ];
    for state in states {
        assert!(
            content.contains(&format!("int {state} = ")),
            "Zeus.java must define state constant: {state}"
        );
    }

    // 4. Episode contract fields (including R2A-S2 reconnectEverStableWorldSeen)
    let episode_fields = [
        "reconnectEpisodeActive",
        "reconnectState",
        "reconnectStartedAt",
        "reconnectStateSince",
        "reconnectEpisodeId",
        "reconnectTransitions",
        "reconnectEverStableWorldSeen",
        "reconnectWorldSeenBeforeEpisode",
        "reconnectLastReason",
    ];
    for field in episode_fields {
        assert!(
            content.contains(field),
            "Zeus.java must define episode field: {field}"
        );
    }

    // 5. sessionReset() must NOT reset reconnect supervisor
    let session_reset_start = content.find("public static void sessionReset()")
        .expect("sessionReset() must exist");
    let session_reset_body = &content[session_reset_start..session_reset_start + 600];
    assert!(
        !session_reset_body.contains("reconnect"),
        "sessionReset() must NOT call reconnect supervisor reset or mutate reconnect fields"
    );

    // 6. No Action Contract (in supervisor section)
    let section_start = content.find("// ---- RECONNECT")
        .expect("RECONNECT section must exist");
    let section_end = content[section_start..]
        .find("// ---- end RECONNECT")
        .expect("end RECONNECT marker must exist");
    let supervisor_body = &content[section_start..section_start + section_end];

    // No button/dialog click actions
    assert!(!supervisor_body.contains("bt.a()"), "Supervisor must not invoke bt.a()");
    assert!(!supervisor_body.contains(".a()") || supervisor_body.contains("dx.a()"), 
        "Supervisor must not invoke action methods (.a()) other than dx.a()");
    assert!(!supervisor_body.contains(".b("), "Supervisor must not invoke .b(...) UI/action methods");
    assert!(!supervisor_body.contains("fu.s.b("), "Supervisor must not invoke fu.s.b()");
    assert!(!supervisor_body.contains("ah.b("), "Supervisor must not invoke ah.b()");
    assert!(!supervisor_body.contains("bs.b("), "Supervisor must not invoke bs.b()");

    // No network dispatch
    assert!(!supervisor_body.contains("Socket"), "Supervisor must not use Socket");
    assert!(!supervisor_body.contains("Http"), "Supervisor must not use Http");
    assert!(!supervisor_body.contains("send("), "Supervisor must not send packets");
    assert!(!supervisor_body.contains("writePacket"), "Supervisor must not write packets");

    // No native field mutations
    assert!(!supervisor_body.contains("bv.a ="), "Supervisor must not write bv.a");
    assert!(!supervisor_body.contains("bv.b ="), "Supervisor must not write bv.b");
    assert!(!supervisor_body.contains("bv.c ="), "Supervisor must not write bv.c");
    assert!(!supervisor_body.contains("ah.k ="), "Supervisor must not write ah.k");
    assert!(!supervisor_body.contains("fu.i.k ="), "Supervisor must not write fu.i.k");
    assert!(!supervisor_body.lines().any(|l| l.contains("fu.a = ") && !l.contains("fu.a == ")), "Supervisor must not mutate fu.a");
    assert!(!supervisor_body.lines().any(|l| l.contains("fu.s = ") && !l.contains("fu.s == ")), "Supervisor must not mutate fu.s");
    assert!(!supervisor_body.lines().any(|l| l.contains("fu.t = ") && !l.contains("fu.t == ")), "Supervisor must not mutate fu.t");

    // No process restart or watchdog
    assert!(!supervisor_body.contains("exit("), "Supervisor must not exit JVM");
    assert!(!supervisor_body.contains("destroy("), "Supervisor must not call destroy");
    assert!(!supervisor_body.contains("restart"), "Supervisor must not call restart");

    // 7. Strong disconnect classification phrases
    let strong_phrases = [
        "\"mat ket noi\"",
        "\"ket noi that bai\"",
        "\"vui long dang nhap lai\"",
    ];
    for phrase in strong_phrases {
        assert!(
            supervisor_body.contains(phrase),
            "Supervisor must inspect strong phrase: {phrase}"
        );
    }

    // Must use dialogText and norm helpers
    assert!(supervisor_body.contains("dialogText("), "Supervisor must use dialogText helper");
    assert!(supervisor_body.contains("norm("), "Supervisor must use norm helper");

    // R2A-S1: Strong disconnect precedence over successful close
    assert!(
        supervisor_body.contains("!strongDisconnect && fu.a == fu.c && gameReady() && mapStable()"),
        "Successful close must require !strongDisconnect"
    );

    // R2A-S2: Snapshot reconnectEverStableWorldSeen into reconnectWorldSeenBeforeEpisode at open
    assert!(
        supervisor_body.contains("reconnectWorldSeenBeforeEpisode = reconnectEverStableWorldSeen;"),
        "reconnectWorldSeenBeforeEpisode must be snapshotted from reconnectEverStableWorldSeen at open"
    );

    // R2A-S3: Transition count increment on close to RC_IDLE
    assert!(
        supervisor_body.contains("reconnectTransitions++;") || supervisor_body.contains("++reconnectTransitions;"),
        "Successful close must increment reconnectTransitions"
    );

    // 8. Health and AUTH contracts intact
    assert!(content.contains("AUTH_MAX_ATTEMPTS = 3"), "AUTH_MAX_ATTEMPTS must remain 3");
    assert!(content.contains("AUTH_RETRY_INTERVAL_TICKS = 75"), "AUTH_RETRY_INTERVAL_TICKS must remain 75");
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ReconnectState {
    RcIdle,
    RcNativeWait,
    RcLogin,
    RcServer,
    RcCharacter,
    RcWorldSettle,
    RcOther,
}

struct ReconnectSupervisorSimulator {
    active: bool,
    state: ReconnectState,
    started_at: u64,
    state_since: u64,
    episode_id: u32,
    transitions: u32,
    ever_stable_world_seen: bool,
    world_seen_before_episode: bool,
    last_reason: String,
}

impl ReconnectSupervisorSimulator {
    fn new() -> Self {
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
        }
    }

    fn is_strong_disconnect(dialog_text: Option<&str>, bv_a: bool) -> (bool, &'static str) {
        if bv_a {
            return (true, "NATIVE_BV_A");
        }
        if let Some(text) = dialog_text {
            let lower = text.to_lowercase();
            if lower.contains("mat ket noi") {
                return (true, "MODAL_DISCONNECT_MAT_KET_NOI");
            }
            if lower.contains("ket noi that bai") {
                return (true, "MODAL_DISCONNECT_KET_NOI_THAT_BAI");
            }
            if lower.contains("vui long dang nhap lai") {
                return (true, "MODAL_DISCONNECT_VUI_LONG_DANG_NHAP_LAI");
            }
        }
        (false, "")
    }

    fn tick(
        &mut self,
        now: u64,
        screen: Option<&str>,      // "login", "server", "character", "world", "other", or None (null)
        dialog_text: Option<&str>,
        bv_a: bool,
        game_ready: bool,
        map_stable: bool,
    ) {
        // Clock rollback handling
        if now < self.started_at {
            self.started_at = now;
        }
        if now < self.state_since {
            self.state_since = now;
        }

        // Normal prior stable gameplay sets ever_stable_world_seen when no episode is active
        if !self.active && screen == Some("world") && game_ready && map_stable {
            self.ever_stable_world_seen = true;
        }

        let (strong_disconnect, reason) = Self::is_strong_disconnect(dialog_text, bv_a);

        // Episode Opening Rule
        if !self.active {
            if strong_disconnect {
                self.active = true;
                self.episode_id += 1;
                self.started_at = now;
                self.state_since = now;
                self.state = ReconnectState::RcNativeWait;
                self.transitions = 0;
                // R2A-S2: Snapshot immutable copy from ever_stable_world_seen exactly once at open
                self.world_seen_before_episode = self.ever_stable_world_seen;
                self.last_reason = reason.to_string();
            }
            return;
        }

        // Active Episode Handling
        if screen.is_none() {
            // Transient null-screen pauses observation, does NOT reset episode
            return;
        }

        let current_screen = screen.unwrap();

        // R2A-S1: Successful close rule is ONLY allowed if !strong_disconnect
        if !strong_disconnect && current_screen == "world" && game_ready && map_stable {
            self.ever_stable_world_seen = true;
            self.active = false;
            self.state = ReconnectState::RcIdle;
            self.state_since = now;
            // R2A-S3: increment transitions on successful close
            self.transitions += 1;
            return;
        }

        // State Derivation for Active Episode
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
    }
}

#[test]
fn test_state_machine_contract_scenarios() {
    let mut sim = ReconnectSupervisorSimulator::new();

    // 1. No episode on normal initial LoginScreen without transport evidence
    sim.tick(1000, Some("login"), None, false, false, false);
    assert!(!sim.active, "1. Normal initial LoginScreen must not open episode");
    assert_eq!(sim.state, ReconnectState::RcIdle);
    assert_eq!(sim.episode_id, 0);

    // 2. No episode on CharacterSelect without transport evidence
    sim.tick(2000, Some("character"), None, false, false, false);
    assert!(!sim.active, "2. CharacterSelect without transport must not open episode");

    // Transition into world and become ready/stable
    sim.tick(3000, Some("world"), None, false, true, true);
    assert!(!sim.active);
    assert!(sim.ever_stable_world_seen);

    // 3. No episode when manually leaving world without transport evidence
    sim.tick(4000, Some("login"), None, false, false, false);
    assert!(!sim.active, "3. Manual logout without transport evidence must not open episode");
    assert_eq!(sim.episode_id, 0);

    // 4. bv.a opens exactly one reconnect episode
    sim.tick(5000, Some("world"), None, true, false, false);
    assert!(sim.active, "4. bv.a must open reconnect episode");
    assert_eq!(sim.episode_id, 1);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);
    assert_eq!(sim.last_reason, "NATIVE_BV_A");
    assert!(sim.world_seen_before_episode, "Episode snapshotted prior stable world");

    // 6. Repeated same disconnect evidence does not increment episode id repeatedly
    sim.tick(5100, Some("world"), None, true, false, false);
    assert_eq!(sim.episode_id, 1, "6. Repeated disconnect must not increment episode id");
    assert_eq!(sim.transitions, 0, "No state change when bv.a continues");

    // 7. Active episode + login maps to RC_LOGIN
    sim.tick(6000, Some("login"), None, false, false, false);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcLogin, "7. Must map to RC_LOGIN");
    assert_eq!(sim.transitions, 1);

    // 8. Active episode + fu.a == fu.b && fu.t == fu.g maps to RC_SERVER
    sim.tick(7000, Some("server"), None, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcServer, "8. Must map to RC_SERVER");
    assert_eq!(sim.transitions, 2);

    // 9. Active episode + character select maps to RC_CHARACTER
    sim.tick(8000, Some("character"), None, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcCharacter, "9. Must map to RC_CHARACTER");
    assert_eq!(sim.transitions, 3);

    // 10. Active episode + world not ready/stable maps to RC_WORLD_SETTLE
    sim.tick(9000, Some("world"), None, false, false, false);
    assert_eq!(sim.state, ReconnectState::RcWorldSettle, "10. Must map to RC_WORLD_SETTLE");
    assert_eq!(sim.transitions, 4);

    // 11. World screen alone does not close episode
    sim.tick(9100, Some("world"), None, false, true, false); // ready but not stable
    assert!(sim.active, "11. World screen alone must not close episode");
    assert_eq!(sim.state, ReconnectState::RcWorldSettle);

    // 13. Transient null-screen does not implicitly reset episode
    sim.tick(9200, None, None, false, false, false);
    assert!(sim.active, "13. Transient null screen must not reset episode");
    assert_eq!(sim.state, ReconnectState::RcWorldSettle);

    // 12. gameReady()+mapStable() closes the episode
    sim.tick(9300, Some("world"), None, false, true, true);
    assert!(!sim.active, "12. gameReady()+mapStable() must close episode");
    assert_eq!(sim.state, ReconnectState::RcIdle);
    assert_eq!(sim.transitions, 5, "Successful close increments transition count to 5");

    // 5. Strong recognized disconnect dialog opens exactly one reconnect episode
    sim.tick(10000, Some("world"), Some("Mat ket noi voi may chu"), false, false, false);
    assert!(sim.active, "5. Strong disconnect phrase opens episode");
    assert_eq!(sim.episode_id, 2);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);
    assert_eq!(sim.last_reason, "MODAL_DISCONNECT_MAT_KET_NOI");

    // 14. State transition counter changes only on real transitions
    let t_before = sim.transitions;
    sim.tick(10100, Some("world"), Some("Mat ket noi voi may chu"), false, false, false);
    assert_eq!(sim.transitions, t_before, "14. No transition increment on unchanged state");

    // 15. Clock rollback cannot create negative-action behavior
    sim.tick(5000, Some("world"), Some("Mat ket noi voi may chu"), false, false, false);
    assert!(sim.started_at <= 5000);
    assert!(sim.state_since <= 5000);
}

#[test]
fn test_r2a_s1_strong_disconnect_blocks_successful_close() {
    let mut sim = ReconnectSupervisorSimulator::new();

    // Prior normal gameplay
    sim.tick(1000, Some("world"), None, false, true, true);
    assert!(sim.ever_stable_world_seen);

    // Open episode via bv.a
    sim.tick(2000, Some("world"), None, true, false, false);
    assert!(sim.active);
    assert_eq!(sim.state, ReconnectState::RcNativeWait);

    // Scenario: World screen returns, gameReady=true and mapStable=true,
    // BUT bv.a=true still indicates disconnect!
    sim.tick(3000, Some("world"), None, true, true, true);
    assert!(sim.active, "Episode must NOT close while bv.a is true");
    assert_eq!(sim.state, ReconnectState::RcNativeWait, "State must remain RC_NATIVE_WAIT");

    // Also modal disconnect dialog while in stable world
    sim.tick(4000, Some("world"), Some("Ket noi that bai"), false, true, true);
    assert!(sim.active, "Episode must NOT close while disconnect modal exists");
    assert_eq!(sim.state, ReconnectState::RcNativeWait, "State must remain RC_NATIVE_WAIT");

    // Once disconnect evidence clears on next stable tick, close is authorized
    sim.tick(5000, Some("world"), None, false, true, true);
    assert!(!sim.active, "Episode successfully closes once disconnect evidence clears");
    assert_eq!(sim.state, ReconnectState::RcIdle);
}

#[test]
fn test_r2a_s2_world_seen_before_episode_snapshot_invariance() {
    let mut sim = ReconnectSupervisorSimulator::new();

    // 1. Initial failure before any stable world
    sim.tick(1000, Some("login"), Some("Vui long dang nhap lai"), false, false, false);
    assert!(sim.active);
    assert_eq!(sim.episode_id, 1);
    assert_eq!(sim.world_seen_before_episode, false, "Initial failure has world_seen_before_episode=false");

    // During this episode, stable world conditions appear while bv.a=true
    sim.tick(2000, Some("world"), None, true, true, true);
    assert_eq!(
        sim.world_seen_before_episode, false,
        "world_seen_before_episode must NOT be mutated during active episode"
    );

    // Close episode
    sim.tick(3000, Some("world"), None, false, true, true);
    assert!(!sim.active);
    assert!(sim.ever_stable_world_seen, "Successful close establishes ever_stable_world_seen");

    // 2. Subsequent failure snapshots true
    sim.tick(4000, Some("world"), None, true, false, false);
    assert!(sim.active);
    assert_eq!(sim.episode_id, 2);
    assert_eq!(sim.world_seen_before_episode, true, "Episode 2 snapshots prior stable world as true");
}

#[test]
fn test_r2a_s3_successful_close_increments_transition_parity() {
    let mut sim = ReconnectSupervisorSimulator::new();

    // Start episode
    sim.tick(1000, Some("world"), None, true, false, false);
    assert_eq!(sim.transitions, 0);

    // Transition 1: RcLogin
    sim.tick(2000, Some("login"), None, false, false, false);
    assert_eq!(sim.transitions, 1);

    // Transition 2: RcCharacter
    sim.tick(3000, Some("character"), None, false, false, false);
    assert_eq!(sim.transitions, 2);

    // Transition 3: RcWorldSettle
    sim.tick(4000, Some("world"), None, false, false, false);
    assert_eq!(sim.transitions, 3);

    // Transition 4: RcIdle (successful close)
    sim.tick(5000, Some("world"), None, false, true, true);
    assert!(!sim.active);
    assert_eq!(sim.state, ReconnectState::RcIdle);
    assert_eq!(sim.transitions, 4, "Close to RC_IDLE must increment transitions");

    // Subsequent ticks while idle do not increment transitions
    sim.tick(6000, Some("world"), None, false, true, true);
    assert_eq!(sim.transitions, 4, "Idle ticks must not increment transitions");
}

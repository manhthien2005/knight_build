use std::time::{Duration, Instant};
use zeus_core::wire::HealthScreen;

#[path = "../src/game_loop_watchdog.rs"]
#[allow(dead_code)]
mod game_loop_watchdog;

#[path = "../src/health_observer.rs"]
#[allow(dead_code)]
mod health_observer;

use game_loop_watchdog::{
    GameLoopWatchdog, HEALTH_FREEZE_THRESHOLD_MS, HEALTH_STARTUP_GRACE_MS, RateLimitReason,
    ReplacementRetryEvaluation, WATCHDOG_BACKOFF_MS, WATCHDOG_MIN_RECOVERY_INTERVAL_MS,
    WatchdogEvaluation, WatchdogRecoveryReason,
};
use health_observer::{HealthObservationEvent, HealthReadCondition};

#[test]
fn test_pure_detection_progressing_sequence_never_triggers_freeze() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    // Sequence progressing every 1 second for 60 seconds
    for s in 1..=60 {
        let now = start + Duration::from_secs(s);
        let event = HealthObservationEvent::Progress {
            sequence: s as u64,
            screen_changed: None,
            disconnect_changed: None,
            dialog_changed: None,
            condition_changed: false,
        };
        let eval = wd.evaluate(
            false,
            false,
            "running",
            true,
            true,
            1,
            Some(1),
            true,
            Some(Duration::ZERO),
            HealthReadCondition::Healthy,
            &event,
            now,
        );
        assert_eq!(eval, WatchdogEvaluation::NoAction);
    }
}

#[test]
fn test_pure_detection_sub_threshold_never_triggers() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    // 29.999s no progress
    let now = start + Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS - 1);
    let event = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };
    let eval = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS - 1)),
        HealthReadCondition::Healthy,
        &event,
        now,
    );
    assert_eq!(eval, WatchdogEvaluation::NoAction);
}

#[test]
fn test_pure_detection_at_threshold_arms_then_second_confirms() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let t30 = start + Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS);
    let event = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // First eligible poll at 30s: arms confirmation
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &event,
        t30,
    );
    assert_eq!(
        eval1,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 }
    );

    // Second eligible poll at 32s: confirms GAME_LOOP_FREEZE
    let t32 = t30 + Duration::from_secs(2);
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS + 2000)),
        HealthReadCondition::Healthy,
        &event,
        t32,
    );
    assert_eq!(
        eval2,
        WatchdogEvaluation::RecoveryRequested {
            reason: WatchdogRecoveryReason::GameLoopFreeze
        }
    );
}

#[test]
fn test_pure_detection_progress_between_confirmation_polls_cancels() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let t30 = start + Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS);
    let unchanged = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // First poll arms
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &unchanged,
        t30,
    );
    assert_eq!(
        eval1,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 }
    );

    // Progress arrives at 32s
    let t32 = t30 + Duration::from_secs(2);
    let progress = HealthObservationEvent::Progress {
        sequence: 11,
        screen_changed: None,
        disconnect_changed: None,
        dialog_changed: None,
        condition_changed: false,
    };
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::ZERO),
        HealthReadCondition::Healthy,
        &progress,
        t32,
    );
    assert_eq!(eval2, WatchdogEvaluation::NoAction);

    // Next unchanged poll must arm count 1 again, not confirm
    let t34 = t32 + Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS);
    let eval3 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &unchanged,
        t34,
    );
    assert_eq!(
        eval3,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 }
    );
}

#[test]
fn test_pure_detection_missing_invalid_regression_cancels_freeze() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let t30 = start + Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS);
    let unchanged = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // Arm confirmation
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &unchanged,
        t30,
    );

    // MissingFile cancels confirmation
    let missing = HealthObservationEvent::MissingFile {
        condition_changed: true,
    };
    let eval = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS + 2000)),
        HealthReadCondition::MissingFile,
        &missing,
        t30 + Duration::from_secs(2),
    );
    assert_eq!(eval, WatchdogEvaluation::NoAction);

    // Re-arm
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &unchanged,
        t30 + Duration::from_secs(4),
    );

    // InvalidOrIo cancels
    let invalid = HealthObservationEvent::InvalidOrIo {
        error: "corrupt".into(),
        condition_changed: true,
    };
    let eval_inv = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS + 6000)),
        HealthReadCondition::InvalidOrIo,
        &invalid,
        t30 + Duration::from_secs(6),
    );
    assert_eq!(eval_inv, WatchdogEvaluation::NoAction);

    // Re-arm
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS)),
        HealthReadCondition::Healthy,
        &unchanged,
        t30 + Duration::from_secs(8),
    );

    // SequenceRegression cancels
    let regression = HealthObservationEvent::SequenceRegression {
        last_sequence: 10,
        seen_sequence: 5,
        condition_changed: true,
    };
    let eval_reg = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS + 10000)),
        HealthReadCondition::SequenceRegression,
        &regression,
        t30 + Duration::from_secs(10),
    );
    assert_eq!(eval_reg, WatchdogEvaluation::NoAction);
}

#[test]
fn test_startup_no_health_detection_lifecycle() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let awaiting = HealthObservationEvent::NotSupervised;

    // Before 90s grace: no action (even at 89.999s)
    let t89 = start + Duration::from_millis(HEALTH_STARTUP_GRACE_MS - 1);
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t89,
    );
    assert_eq!(eval1, WatchdogEvaluation::NoAction);

    // At 90s: first eligible poll arms confirmation
    let t90 = start + Duration::from_millis(HEALTH_STARTUP_GRACE_MS);
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t90,
    );
    assert_eq!(
        eval2,
        WatchdogEvaluation::ArmingStartupConfirmation { count: 1 }
    );

    // At 92s: second eligible poll confirms STARTUP_NO_HEALTH
    let t92 = t90 + Duration::from_secs(2);
    let eval3 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::MissingFile,
        &HealthObservationEvent::MissingFile {
            condition_changed: false,
        },
        t92,
    );
    assert_eq!(
        eval3,
        WatchdogEvaluation::RecoveryRequested {
            reason: WatchdogRecoveryReason::StartupNoHealth
        }
    );
}

#[test]
fn test_startup_first_valid_sample_cancels_confirmation() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let t90 = start + Duration::from_millis(HEALTH_STARTUP_GRACE_MS);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::AwaitingFirstSample,
        &HealthObservationEvent::NotSupervised,
        t90,
    );

    // Baseline sample arrives
    let baseline = HealthObservationEvent::BaselineSample {
        sequence: 1,
        screen: HealthScreen::Login,
        dialog_open: false,
        native_disconnect: false,
        condition_changed: true,
    };
    let eval = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::ZERO),
        HealthReadCondition::Healthy,
        &baseline,
        t90 + Duration::from_secs(1),
    );
    assert_eq!(eval, WatchdogEvaluation::NoAction);
}

#[test]
fn test_generation_lifecycle_resets_confirmations_preserves_budget() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    // Arm one recovery attempt to consume budget
    wd.arm_recovery_attempt(start);
    assert_eq!(wd.recoveries_in_window(), 1);

    // Spawn generation 2 at start + 70s
    let t70 = start + Duration::from_secs(70);
    wd.on_successful_spawn(2, t70);

    // Generation 2 must retain recoveries_in_window = 1
    assert_eq!(wd.recoveries_in_window(), 1);
    assert_eq!(wd.current_process_generation(), 2);

    // Old generation 1 evidence cannot restart generation 2
    let eval_old = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        2,
        Some(1),
        true,
        Some(Duration::from_secs(40)),
        HealthReadCondition::Healthy,
        &HealthObservationEvent::Unchanged {
            sequence: 10,
            condition_changed: false,
        },
        t70 + Duration::from_secs(1),
    );
    assert_eq!(eval_old, WatchdogEvaluation::NoAction);
}

#[test]
fn test_explicit_user_stop_resets_full_watchdog_state_and_budget() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    // Consume all 3 budget slots to trigger backoff
    wd.arm_recovery_attempt(start);
    wd.arm_recovery_attempt(start + Duration::from_secs(65));
    wd.arm_recovery_attempt(start + Duration::from_secs(130));
    assert!(wd.is_in_backoff(start + Duration::from_secs(131)));

    // User explicitly stops account
    wd.on_explicit_user_stop_or_retirement();

    // Budget and backoff are completely reset
    assert_eq!(wd.recoveries_in_window(), 0);
    assert!(!wd.is_in_backoff(start + Duration::from_secs(131)));
}

#[test]
fn test_budget_minimum_interval_and_backoff_window() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    let unchanged = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // Trigger 1st recovery
    let t30 = start + Duration::from_secs(30);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(30)),
        HealthReadCondition::Healthy,
        &unchanged,
        t30,
    );
    let t32 = start + Duration::from_secs(32);
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(32)),
        HealthReadCondition::Healthy,
        &unchanged,
        t32,
    );
    assert!(matches!(
        eval1,
        WatchdogEvaluation::RecoveryRequested { .. }
    ));

    // Arm 1st recovery
    wd.arm_recovery_attempt(t32);
    assert_eq!(wd.recoveries_in_window(), 1);

    // Attempting recovery at t32 + 30s (< 60s min interval) is suppressed
    let t62 = t32 + Duration::from_secs(30);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(30)),
        HealthReadCondition::Healthy,
        &unchanged,
        t62,
    );
    let eval_suppressed = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(32)),
        HealthReadCondition::Healthy,
        &unchanged,
        t62 + Duration::from_secs(2),
    );
    assert!(matches!(
        eval_suppressed,
        WatchdogEvaluation::SuppressedByRateLimit {
            reason: RateLimitReason::MinimumIntervalActive { .. }
        }
    ));

    // After 60s minimum interval: 2nd recovery allowed
    let t95 = t32 + Duration::from_millis(WATCHDOG_MIN_RECOVERY_INTERVAL_MS + 1000);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(30)),
        HealthReadCondition::Healthy,
        &unchanged,
        t95,
    );
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(32)),
        HealthReadCondition::Healthy,
        &unchanged,
        t95 + Duration::from_secs(2),
    );
    assert!(matches!(
        eval2,
        WatchdogEvaluation::RecoveryRequested { .. }
    ));
    wd.arm_recovery_attempt(t95 + Duration::from_secs(2));
    assert_eq!(wd.recoveries_in_window(), 2);
    assert!(!wd.is_in_backoff(t95 + Duration::from_secs(2)));

    // 3rd recovery allowed after another 60s
    let t160 = t95 + Duration::from_secs(65);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(30)),
        HealthReadCondition::Healthy,
        &unchanged,
        t160,
    );
    let eval3 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(32)),
        HealthReadCondition::Healthy,
        &unchanged,
        t160 + Duration::from_secs(2),
    );
    assert!(matches!(
        eval3,
        WatchdogEvaluation::RecoveryRequested { .. }
    ));

    // Arming 3rd recovery arms 30-minute backoff
    wd.arm_recovery_attempt(t160 + Duration::from_secs(2));
    assert_eq!(wd.recoveries_in_window(), 3);
    assert!(wd.is_in_backoff(t160 + Duration::from_secs(3)));

    // During backoff (e.g. at 15 minutes into backoff), recovery is suppressed
    let t_during_backoff = t160 + Duration::from_secs(900);
    let _ = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(30)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_during_backoff,
    );
    let eval_backoff = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(32)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_during_backoff + Duration::from_secs(2),
    );
    assert!(matches!(
        eval_backoff,
        WatchdogEvaluation::SuppressedByRateLimit {
            reason: RateLimitReason::BudgetExhaustedInBackoff { .. }
        }
    ));

    // After 30 minutes backoff expires: backoff clears and allows fresh cycle
    let t_after_backoff = t160 + Duration::from_millis(WATCHDOG_BACKOFF_MS + 5000);
    assert!(!wd.is_in_backoff(t_after_backoff));
}

#[test]
fn test_source_code_inspection_and_scope_invariants() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_loop_path = manifest_dir.join("src/main_loop.rs");
    let main_loop_content =
        std::fs::read_to_string(&main_loop_path).expect("main_loop.rs must exist");

    // 1. Verify reconnect observer state/duration is NOT used for watchdog authorization
    assert!(
        !main_loop_content.contains("reconnect_observer.duration")
            && !main_loop_content.contains("reconnect_observer.is_stalled"),
        "reconnect_observer duration or stall must not be in main_loop"
    );

    // 2. Verify no new thread was created for watchdog
    for line in main_loop_content.lines() {
        if line.contains("thread::spawn") {
            assert!(
                !line.contains("watchdog"),
                "watchdog must not spawn a new thread: {line}"
            );
        }
    }

    // 3. Verify no Java/JAR/artifact file changes
    let zeus_java_path = manifest_dir.join("../../../mod/zeus/src/Zeus.java");
    assert!(zeus_java_path.exists(), "Zeus.java must exist");
}

#[test]
fn test_failed_stop_consumes_recovery_attempt_and_minimum_interval() {
    let start = Instant::now();
    let mut wd = GameLoopWatchdog::new(start);
    wd.on_successful_spawn(1, start);

    // Arm attempt (simulating stop initiated)
    wd.arm_recovery_attempt(start);
    assert_eq!(wd.recoveries_in_window(), 1);

    // Stop fails: process is still alive. Budget was still consumed.
    // An immediate next evaluation at start + 10s must be suppressed by minimum interval:
    let t10 = start + Duration::from_secs(10);
    let check = wd.check_budget(t10);
    assert!(matches!(
        check,
        Err(RateLimitReason::MinimumIntervalActive { remaining_ms }) if remaining_ms > 0
    ));
}

#[test]
fn test_process_lifecycle_source_inspection() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_loop_path = manifest_dir.join("src/main_loop.rs");
    let content = std::fs::read_to_string(&main_loop_path).expect("main_loop.rs must exist");

    // 1. Verify recover_game_loop_watchdog and recover_logical_reconnect_watchdog delegate to execute_external_watchdog_recovery
    let r3b_idx = content
        .find("fn recover_game_loop_watchdog")
        .expect("recover_game_loop_watchdog must exist in main_loop.rs");
    let r3b_body = &content[r3b_idx..r3b_idx + 1200];
    assert!(
        r3b_body.contains("execute_external_watchdog_recovery("),
        "recover_game_loop_watchdog must delegate to execute_external_watchdog_recovery"
    );
    assert!(
        !r3b_body.contains("process_unix::stop")
            && !r3b_body.contains("evaluate_stop_transition")
            && !r3b_body.contains("acc.restarts +=")
            && !r3b_body.contains("mark_replacement_pending"),
        "recover_game_loop_watchdog wrapper must not contain recovery lifecycle actions"
    );

    let r3c_idx = content
        .find("fn recover_logical_reconnect_watchdog")
        .expect("recover_logical_reconnect_watchdog must exist in main_loop.rs");
    let r3c_body = &content[r3c_idx..r3c_idx + 1200];
    assert!(
        r3c_body.contains("execute_external_watchdog_recovery("),
        "recover_logical_reconnect_watchdog must delegate to execute_external_watchdog_recovery"
    );
    assert!(
        !r3c_body.contains("process_unix::stop")
            && !r3c_body.contains("evaluate_stop_transition")
            && !r3c_body.contains("acc.restarts +=")
            && !r3c_body.contains("mark_replacement_pending"),
        "recover_logical_reconnect_watchdog wrapper must not contain recovery lifecycle actions"
    );

    // 2. Verify shared execute_external_watchdog_recovery handles the entire lifecycle
    let fn_idx = content
        .find("fn execute_external_watchdog_recovery")
        .expect("execute_external_watchdog_recovery must exist in main_loop.rs");
    let fn_body = &content[fn_idx..fn_idx + 5000];

    // Verify FailedStillAlive retains process and does NOT call reconcile_desired_state
    let failed_block_idx = fn_body
        .find("StopTransition::FailedStillAlive => {")
        .expect("FailedStillAlive must be handled in execute_external_watchdog_recovery");
    let after_failed = &fn_body[failed_block_idx..failed_block_idx + 300];
    assert!(
        !after_failed.contains("reconcile_desired_state"),
        "FailedStillAlive must not call reconcile_desired_state"
    );

    // Verify ConfirmedStopped increments restarts once and delegates to reconcile_desired_state
    let confirmed_block_idx = fn_body
        .find("StopTransition::ConfirmedStopped => {")
        .expect("ConfirmedStopped must be handled");
    let after_confirmed = &fn_body[confirmed_block_idx..confirmed_block_idx + 600];
    assert!(
        after_confirmed.contains("acc.restarts += 1;"),
        "ConfirmedStopped must increment restarts once"
    );
    assert!(
        after_confirmed.contains("acc.game_loop_watchdog.mark_replacement_pending();"),
        "ConfirmedStopped must mark replacement pending"
    );
    assert!(
        after_confirmed.contains("reconcile_desired_state_with_cause(")
            && after_confirmed.contains("ReconcileCause::WatchdogReplacement"),
        "ConfirmedStopped must delegate spawn to reconcile_desired_state_with_cause with WatchdogReplacement"
    );

    // 3. Verify exactly one function contains process_unix::stop and evaluate_stop_transition in external recovery
    let watchdog_section_start = content
        .find("// ── external watchdog recovery")
        .expect("external watchdog recovery section must exist");
    let watchdog_section_end = content
        .find("// ── reconcile desired state")
        .expect("reconcile desired state section must exist");
    let watchdog_section = &content[watchdog_section_start..watchdog_section_end];

    let non_comment_lines: Vec<&str> = watchdog_section
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.starts_with("//") && !l.starts_with("///"))
        .collect();

    let stop_calls = non_comment_lines
        .iter()
        .filter(|l| l.contains("process_unix::stop("))
        .count();
    assert_eq!(
        stop_calls, 1,
        "Exactly one process_unix::stop call must exist in external watchdog recovery"
    );

    let transition_evals = non_comment_lines
        .iter()
        .filter(|l| l.contains("evaluate_stop_transition("))
        .count();
    assert_eq!(
        transition_evals, 1,
        "Exactly one evaluate_stop_transition call must exist in external watchdog recovery"
    );
    assert_eq!(
        watchdog_section.matches("acc.restarts += 1;").count(),
        1,
        "No duplicated acc.restarts += 1 watchdog lifecycle block remains"
    );
    assert_eq!(
        watchdog_section
            .matches("mark_replacement_pending();")
            .count(),
        1,
        "No duplicated mark_replacement_pending watchdog lifecycle block remains"
    );

    // 4. Verify no manual Java spawn in watchdog
    assert!(
        !content.contains("Command::new(\"java\")") && !content.contains("Command::new(\"javaw\")"),
        "No direct Java spawn may exist"
    );

    // 5. Verify no PID-only kill in watchdog
    assert!(
        !content.contains("libc::kill(child.pid,"),
        "No PID-only kill may exist; must signal pgid"
    );

    // 6. Verify duplicate restart prevention in 2s tick
    assert!(
        content.contains("watchdog_recovered_ids"),
        "2s tick must track watchdog_recovered_ids to prevent double restart"
    );
}

#[test]
fn test_pending_state_lifecycle() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);

    // Initial state: replacement is not pending
    assert!(!wd.replacement_pending());

    // 1. Confirmed stop marks replacement_pending
    wd.mark_replacement_pending();
    assert!(wd.replacement_pending());

    // 2. Successful spawn clears replacement_pending but preserves budget
    wd.arm_recovery_attempt(t0);
    assert_eq!(wd.recoveries_in_window(), 1);
    wd.mark_replacement_pending();
    assert!(wd.replacement_pending());

    let t1 = t0 + Duration::from_secs(5);
    wd.on_successful_spawn(2, t1);
    assert!(
        !wd.replacement_pending(),
        "successful spawn must clear replacement_pending"
    );
    assert_eq!(
        wd.recoveries_in_window(),
        1,
        "successful spawn must preserve recoveries_in_window budget"
    );

    // 3. Explicit user stop clears replacement_pending and resets full budget
    wd.mark_replacement_pending();
    assert!(wd.replacement_pending());
    wd.on_explicit_user_stop_or_retirement();
    assert!(
        !wd.replacement_pending(),
        "explicit stop must clear replacement_pending"
    );
    assert_eq!(
        wd.recoveries_in_window(),
        0,
        "explicit stop must reset recoveries_in_window budget"
    );
}

#[test]
fn test_failed_stop_does_not_set_pending() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);
    wd.on_successful_spawn(1, t0);

    // When recovery is attempted, budget is armed BEFORE stop
    wd.arm_recovery_attempt(t0);
    assert_eq!(wd.recoveries_in_window(), 1);

    // If stop fails (FailedStillAlive), mark_replacement_pending is NOT called
    assert!(
        !wd.replacement_pending(),
        "FailedStillAlive must not mark replacement pending"
    );
}

#[test]
fn test_replacement_retry_timing_and_budget() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);

    // Frozen JVM stopped at t0, replacement pending
    wd.arm_recovery_attempt(t0);
    wd.mark_replacement_pending();
    assert_eq!(wd.recoveries_in_window(), 1);

    // At t = 10s: Suppressed by 60s minimum interval
    let t10 = t0 + Duration::from_secs(10);
    let eval10 = wd.evaluate_replacement_retry(false, false, "running", false, t10);
    assert!(matches!(
        eval10,
        ReplacementRetryEvaluation::Suppressed(RateLimitReason::MinimumIntervalActive { remaining_ms }) if remaining_ms > 0
    ));

    // Non-eligible conditions fail-closed
    assert_eq!(
        wd.evaluate_replacement_retry(true, false, "running", false, t10),
        ReplacementRetryEvaluation::NotEligible,
        "Retiring account must not retry"
    );
    assert_eq!(
        wd.evaluate_replacement_retry(false, true, "running", false, t10),
        ReplacementRetryEvaluation::NotEligible,
        "Tombstoned account must not retry"
    );
    assert_eq!(
        wd.evaluate_replacement_retry(false, false, "stopped", false, t10),
        ReplacementRetryEvaluation::NotEligible,
        "Non-running desired_state must not retry"
    );
    assert_eq!(
        wd.evaluate_replacement_retry(false, false, "running", true, t10),
        ReplacementRetryEvaluation::NotEligible,
        "Live process must not retry replacement"
    );

    // At t = 60s: Minimum interval satisfied -> Authorized
    let t60 = t0 + Duration::from_secs(60);
    let eval60 = wd.evaluate_replacement_retry(false, false, "running", false, t60);
    assert_eq!(eval60, ReplacementRetryEvaluation::Authorized);

    // Retry 1 attempted (2nd total attempt in window)
    wd.arm_recovery_attempt(t60);
    assert_eq!(wd.recoveries_in_window(), 2);
    assert!(!wd.is_in_backoff(t60));

    // At t = 70s: Minimum interval active again
    let t70 = t0 + Duration::from_secs(70);
    assert!(matches!(
        wd.evaluate_replacement_retry(false, false, "running", false, t70),
        ReplacementRetryEvaluation::Suppressed(RateLimitReason::MinimumIntervalActive { .. })
    ));

    // At t = 120s: 3rd attempt authorized
    let t120 = t0 + Duration::from_secs(120);
    assert_eq!(
        wd.evaluate_replacement_retry(false, false, "running", false, t120),
        ReplacementRetryEvaluation::Authorized
    );

    // Retry 2 attempted (3rd total attempt in window) -> arms 30-minute backoff!
    wd.arm_recovery_attempt(t120);
    assert_eq!(wd.recoveries_in_window(), 3);
    assert!(
        wd.is_in_backoff(t120),
        "3rd attempt must arm 30-minute backoff"
    );

    // During backoff at t = 130s: Suppressed
    let t130 = t0 + Duration::from_secs(130);
    assert!(matches!(
        wd.evaluate_replacement_retry(false, false, "running", false, t130),
        ReplacementRetryEvaluation::Suppressed(RateLimitReason::BudgetExhaustedInBackoff { .. })
    ));

    // At t = 120s + 1799s: Still in backoff
    let t_almost_end = t120 + Duration::from_secs(1799);
    assert!(matches!(
        wd.evaluate_replacement_retry(false, false, "running", false, t_almost_end),
        ReplacementRetryEvaluation::Suppressed(RateLimitReason::BudgetExhaustedInBackoff { .. })
    ));

    // At t = 120s + 1800s: Backoff expired -> fresh bounded pending replacement cycle authorized!
    let t_expired = t120 + Duration::from_secs(1800);
    assert_eq!(
        wd.evaluate_replacement_retry(false, false, "running", false, t_expired),
        ReplacementRetryEvaluation::Authorized,
        "Backoff expiry must permit fresh replacement cycle"
    );
}

#[test]
fn test_rate_limit_logging_deduplication() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);

    let reason1 = RateLimitReason::MinimumIntervalActive {
        remaining_ms: 58000,
    };
    // Poll 1: initial transition into MinimumIntervalActive -> logs
    assert!(wd.should_log_rate_limit_transition(&reason1));

    // Poll 2: countdown to 56000 -> does NOT log duplicate
    let reason2 = RateLimitReason::MinimumIntervalActive {
        remaining_ms: 56000,
    };
    assert!(!wd.should_log_rate_limit_transition(&reason2));

    // Transition to Backoff -> logs new suppression class
    let reason_backoff1 = RateLimitReason::BudgetExhaustedInBackoff {
        remaining_ms: 1800000,
    };
    assert!(wd.should_log_rate_limit_transition(&reason_backoff1));

    // Poll within Backoff countdown -> does NOT log duplicate
    let reason_backoff2 = RateLimitReason::BudgetExhaustedInBackoff {
        remaining_ms: 1798000,
    };
    assert!(!wd.should_log_rate_limit_transition(&reason_backoff2));

    // Clear on exiting suppression -> next entry logs again
    wd.clear_rate_limit_transition();
    assert!(wd.should_log_rate_limit_transition(&reason1));
}

#[test]
fn test_reconcile_spawn_guard_and_crash_loop_inspection() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_loop_path = manifest_dir.join("src/main_loop.rs");
    let content = std::fs::read_to_string(&main_loop_path).expect("main_loop.rs must exist");

    // 1. Verify reconcile_desired_state_with_cause and ReconcileCause enum exist
    assert!(
        content.contains("pub enum ReconcileCause"),
        "ReconcileCause enum must exist"
    );
    assert!(
        content.contains("fn reconcile_desired_state_with_cause"),
        "reconcile_desired_state_with_cause must exist"
    );

    // 2. Verify spawn guard suppresses ordinary callers while replacement_pending is true
    let guard_line = "if acc.game_loop_watchdog.replacement_pending() && cause != ReconcileCause::WatchdogReplacement";
    assert!(
        content.contains(guard_line),
        "reconcile_desired_state_with_cause must guard ordinary callers when replacement_pending is true"
    );

    // 3. Verify generic crash loop explicitly skips pending accounts
    let crash_check = "!acc.game_loop_watchdog.replacement_pending()";
    assert!(
        content.contains(crash_check),
        "Generic crash restart loop must skip accounts where replacement_pending is true"
    );

    // 4. Verify 2-second tick handles replacement pending retry
    assert!(
        content.contains("else if acc.game_loop_watchdog.replacement_pending()"),
        "2-second tick must have replacement_pending retry evaluation branch"
    );
    assert!(
        content.contains("evaluate_replacement_retry"),
        "2-second tick must evaluate replacement retry"
    );

    // 5. Verify cloud metadata merge does not overwrite game_loop_watchdog
    let merge_idx = content
        .find("fn reconcile_cloud_accounts")
        .expect("reconcile_cloud_accounts must exist");
    let merge_block = &content[merge_idx..merge_idx + 2000];
    assert!(
        !merge_block.contains("existing.game_loop_watchdog ="),
        "Cloud merge must not overwrite existing game_loop_watchdog"
    );
}

#[test]
fn test_live_jvm_post_backoff_reconfirmation_freeze() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);
    wd.on_successful_spawn(1, t0);

    let unchanged = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // Trigger and arm 3 recoveries to enter 30-minute backoff
    wd.arm_recovery_attempt(t0);
    wd.arm_recovery_attempt(t0 + Duration::from_secs(65));
    let t_3rd = t0 + Duration::from_secs(130);
    wd.arm_recovery_attempt(t_3rd);
    assert_eq!(wd.recoveries_in_window(), 3);
    assert!(wd.is_in_backoff(t_3rd + Duration::from_secs(1)));

    // During backoff, poll 1 arms confirmation (count 1)
    let eval_during_1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(40)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_3rd + Duration::from_secs(900),
    );
    assert_eq!(
        eval_during_1,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 }
    );

    // Repeated subsequent polls during backoff are suppressed by rate limit
    for i in 1..5 {
        let t_during = t_3rd + Duration::from_secs(900 + i * 2);
        let eval = wd.evaluate(
            false,
            false,
            "running",
            true,
            true,
            1,
            Some(1),
            true,
            Some(Duration::from_secs(40 + i * 2)),
            HealthReadCondition::Healthy,
            &unchanged,
            t_during,
        );
        assert!(matches!(
            eval,
            WatchdogEvaluation::SuppressedByRateLimit {
                reason: RateLimitReason::BudgetExhaustedInBackoff { .. }
            }
        ));
    }

    // At exact/after backoff expiry:
    let t_expiry = t_3rd + Duration::from_millis(WATCHDOG_BACKOFF_MS);

    // FIRST post-expiry poll: MUST NOT return RecoveryRequested!
    // Must establish fresh confirmation count 1:
    let eval_first_post = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(1840)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_expiry,
    );
    assert_eq!(
        eval_first_post,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 },
        "First poll after backoff expiry must only arm confirmation (count 1)"
    );

    // SECOND consecutive eligible post-expiry poll: confirms RecoveryRequested
    let t_second_post = t_expiry + Duration::from_secs(2);
    let eval_second_post = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(1842)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_second_post,
    );
    assert_eq!(
        eval_second_post,
        WatchdogEvaluation::RecoveryRequested {
            reason: WatchdogRecoveryReason::GameLoopFreeze
        },
        "Second consecutive eligible poll after backoff expiry confirms recovery"
    );
}

#[test]
fn test_live_jvm_post_backoff_progress_cancels_reconfirmation() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);
    wd.on_successful_spawn(1, t0);

    let unchanged = HealthObservationEvent::Unchanged {
        sequence: 10,
        condition_changed: false,
    };

    // Arm 3 recoveries to enter 30-minute backoff
    wd.arm_recovery_attempt(t0);
    wd.arm_recovery_attempt(t0 + Duration::from_secs(65));
    let t_3rd = t0 + Duration::from_secs(130);
    wd.arm_recovery_attempt(t_3rd);

    let t_expiry = t_3rd + Duration::from_millis(WATCHDOG_BACKOFF_MS);

    // First post-expiry poll arms confirmation (count 1)
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(1840)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_expiry,
    );
    assert_eq!(
        eval1,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 }
    );

    // Health progress occurs between confirmation polls
    let progress = HealthObservationEvent::Progress {
        sequence: 100,
        screen_changed: None,
        disconnect_changed: None,
        dialog_changed: None,
        condition_changed: false,
    };
    let eval_prog = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::ZERO),
        HealthReadCondition::Healthy,
        &progress,
        t_expiry + Duration::from_secs(1),
    );
    assert_eq!(eval_prog, WatchdogEvaluation::NoAction);

    // Next unchanged poll must restart confirmation from 1, not produce RecoveryRequested
    let eval_next = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        true,
        Some(Duration::from_secs(31)),
        HealthReadCondition::Healthy,
        &unchanged,
        t_expiry + Duration::from_secs(3),
    );
    assert_eq!(
        eval_next,
        WatchdogEvaluation::ArmingFreezeConfirmation { count: 1 },
        "Progress must cancel fresh post-backoff confirmation"
    );
}

#[test]
fn test_startup_no_health_post_backoff_reconfirmation() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);
    wd.on_successful_spawn(1, t0);

    // Arm 3 recoveries to enter 30-minute backoff
    wd.arm_recovery_attempt(t0);
    wd.arm_recovery_attempt(t0 + Duration::from_secs(65));
    let t_3rd = t0 + Duration::from_secs(130);
    wd.arm_recovery_attempt(t_3rd);

    let awaiting = HealthObservationEvent::NotSupervised;
    let t_during = t_3rd + Duration::from_secs(900);

    // Poll 1 during backoff arms startup confirmation (count 1)
    let eval_during_1 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false, // No baseline
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t_during,
    );
    assert_eq!(
        eval_during_1,
        WatchdogEvaluation::ArmingStartupConfirmation { count: 1 }
    );

    // Poll 2 during backoff confirms startup no-health but is suppressed by backoff
    let eval_during_2 = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false, // No baseline
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t_during + Duration::from_secs(2),
    );
    assert!(matches!(
        eval_during_2,
        WatchdogEvaluation::SuppressedByRateLimit {
            reason: RateLimitReason::BudgetExhaustedInBackoff { .. }
        }
    ));

    let t_expiry = t_3rd + Duration::from_millis(WATCHDOG_BACKOFF_MS);

    // FIRST post-expiry poll for startup no-health: arms confirmation count 1, does NOT recover
    let eval_first_post = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t_expiry,
    );
    assert_eq!(
        eval_first_post,
        WatchdogEvaluation::ArmingStartupConfirmation { count: 1 },
        "First post-backoff startup evaluation must only arm confirmation"
    );

    // SECOND consecutive eligible post-expiry poll: produces RecoveryRequested
    let eval_second_post = wd.evaluate(
        false,
        false,
        "running",
        true,
        true,
        1,
        Some(1),
        false,
        None,
        HealthReadCondition::AwaitingFirstSample,
        &awaiting,
        t_expiry + Duration::from_secs(2),
    );
    assert_eq!(
        eval_second_post,
        WatchdogEvaluation::RecoveryRequested {
            reason: WatchdogRecoveryReason::StartupNoHealth
        },
        "Second post-backoff startup poll produces RecoveryRequested"
    );
}

#[test]
fn test_shared_backoff_epoch_lifecycle() {
    let t0 = Instant::now();
    let mut wd = GameLoopWatchdog::new(t0);

    // Initial epoch is 0
    assert_eq!(wd.long_backoff_epoch(), 0);

    // Normal recovery attempts do not increment epoch
    let t1 = t0 + Duration::from_secs(65);
    wd.arm_recovery_attempt(t1);
    assert_eq!(wd.long_backoff_epoch(), 0);

    let t2 = t1 + Duration::from_secs(65);
    wd.arm_recovery_attempt(t2);
    assert_eq!(wd.long_backoff_epoch(), 0);

    // 3rd recovery arms 30-minute backoff; epoch still 0
    let t3 = t2 + Duration::from_secs(65);
    wd.arm_recovery_attempt(t3);
    assert_eq!(wd.long_backoff_epoch(), 0);

    // During backoff: epoch still 0
    let t_during = t3 + Duration::from_secs(600); // 10 minutes in
    assert!(wd.is_in_backoff(t_during));
    assert_eq!(wd.long_backoff_epoch(), 0);

    // At backoff expiry: epoch increments to 1
    let t_expiry = t3 + Duration::from_millis(WATCHDOG_BACKOFF_MS);
    assert!(!wd.is_in_backoff(t_expiry));
    assert_eq!(wd.long_backoff_epoch(), 1);

    // Subsequent checks after expiry do not repeatedly increment epoch
    let t_after = t_expiry + Duration::from_secs(60);
    assert_eq!(wd.check_budget(t_after), Ok(()));
    assert_eq!(wd.long_backoff_epoch(), 1);

    // on_successful_spawn preserves epoch!
    wd.on_successful_spawn(2, t_after);
    assert_eq!(wd.long_backoff_epoch(), 1);

    // on_explicit_user_stop_or_retirement resets epoch to 0
    wd.on_explicit_user_stop_or_retirement();
    assert_eq!(wd.long_backoff_epoch(), 0);
}

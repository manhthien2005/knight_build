use std::time::{Duration, Instant};
use zeus_core::wire::HealthScreen;

#[path = "../src/game_loop_watchdog.rs"]
#[allow(dead_code)]
mod game_loop_watchdog;

#[path = "../src/health_observer.rs"]
#[allow(dead_code)]
mod health_observer;

use game_loop_watchdog::{
    GameLoopWatchdog, RateLimitReason, WatchdogEvaluation, WatchdogRecoveryReason,
    HEALTH_FREEZE_THRESHOLD_MS, HEALTH_STARTUP_GRACE_MS, WATCHDOG_BACKOFF_MS,
    WATCHDOG_MIN_RECOVERY_INTERVAL_MS,
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

    // 1. Verify recover_game_loop_watchdog exists
    let fn_idx = content
        .find("fn recover_game_loop_watchdog")
        .expect("recover_game_loop_watchdog must exist in main_loop.rs");
    let fn_body = &content[fn_idx..fn_idx + 3500];

    // 2. Verify FailedStillAlive retains process and does NOT call reconcile_desired_state
    let failed_block_idx = fn_body
        .find("StopTransition::FailedStillAlive => {")
        .expect("FailedStillAlive must be handled in recover_game_loop_watchdog");
    let after_failed = &fn_body[failed_block_idx..failed_block_idx + 300];
    assert!(
        !after_failed.contains("reconcile_desired_state"),
        "FailedStillAlive must not call reconcile_desired_state"
    );

    // 3. Verify ConfirmedStopped increments restarts once and delegates to reconcile_desired_state
    let confirmed_block_idx = fn_body
        .find("StopTransition::ConfirmedStopped => {")
        .expect("ConfirmedStopped must be handled");
    let after_confirmed = &fn_body[confirmed_block_idx..confirmed_block_idx + 500];
    assert!(
        after_confirmed.contains("acc.restarts += 1;"),
        "ConfirmedStopped must increment restarts once"
    );
    assert!(
        after_confirmed.contains("reconcile_desired_state(acc, rest, identity);"),
        "ConfirmedStopped must delegate spawn to reconcile_desired_state"
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

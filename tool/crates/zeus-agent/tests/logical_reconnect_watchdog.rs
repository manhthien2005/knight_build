use std::time::{Duration, Instant};
use zeus_core::wire::{ReconnectState, ReconnectStatusSnapshot};

#[path = "../src/health_observer.rs"]
#[allow(dead_code)]
mod health_observer;

#[path = "../src/reconnect_observer.rs"]
#[allow(dead_code)]
mod reconnect_observer;

#[path = "../src/game_loop_watchdog.rs"]
#[allow(dead_code)]
mod game_loop_watchdog;

#[path = "../src/logical_reconnect_watchdog.rs"]
#[allow(dead_code)]
mod logical_reconnect_watchdog;

use health_observer::{HealthObservationEvent, HealthReadCondition};
use logical_reconnect_watchdog::{
    HEALTH_TRANSPORT_FRESH_MS, LOGICAL_LOADING_STALL_MS, LOGICAL_WORLD_SETTLE_STALL_MS,
    LogicalReconnectWatchdog, LogicalStallRecoveryReason, LogicalWatchdogEvaluation,
    RECONNECT_TRANSPORT_FRESH_MS,
};
use reconnect_observer::{ReconnectObservationEvent, ReconnectReadCondition};

fn make_valid_reconnect_snapshot(
    seq: u64,
    ep: u32,
    state: ReconnectState,
    active: bool,
    world_before: bool,
) -> ReconnectStatusSnapshot {
    ReconnectStatusSnapshot {
        written_at_unix_ms: 1_700_000_000_000 + (seq as i64 * 1000),
        sequence: seq,
        episode_id: ep,
        active,
        state,
        transitions: 1,
        world_seen_before_episode: world_before,
    }
}

fn make_healthy_reconnect_event(seq: u64) -> ReconnectObservationEvent {
    ReconnectObservationEvent::Unchanged {
        sequence: seq,
        condition_changed: false,
    }
}

fn make_healthy_health_event(seq: u64) -> HealthObservationEvent {
    HealthObservationEvent::Unchanged {
        sequence: seq,
        condition_changed: false,
    }
}

#[test]
fn test_loading_sub_threshold_never_authorizes() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // 179_999 ms is just below the 180_000 ms threshold
    let sub_threshold_duration = Duration::from_millis(LOGICAL_LOADING_STALL_MS - 1);

    let eval = wd.evaluate(
        false, // retiring
        false, // tombstoned
        "running",
        false,   // replacement_pending
        true,    // has_process
        true,    // is_alive
        1,       // process_gen
        Some(1), // health_gen
        Some(1), // reconnect_gen
        true,    // has_health_baseline
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)), // health fresh <= 10s
        &health_event,
        true, // has_reconnect_baseline
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)), // reconnect fresh <= 10s
        &reconnect_event,
        Some(&snap),
        Some(1), // current episode id
        Some(sub_threshold_duration),
        0, // shared backoff epoch
        now,
    );

    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
    assert_eq!(wd.loading_confirmation_count(), 0);
}

#[test]
fn test_loading_at_threshold_arms_poll_1_then_poll_2_recovers() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    let threshold_duration = Duration::from_millis(LOGICAL_LOADING_STALL_MS);

    // Poll 1: Exactly at threshold (>= 180s)
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(threshold_duration),
        0,
        now,
    );

    assert_eq!(
        eval1,
        LogicalWatchdogEvaluation::ArmingLoadingConfirmation { count: 1 }
    );
    assert_eq!(wd.loading_confirmation_count(), 1);

    // Poll 2: 2 seconds later, still eligible -> requests recovery
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(threshold_duration + Duration::from_secs(2)),
        0,
        now + Duration::from_secs(2),
    );

    assert_eq!(
        eval2,
        LogicalWatchdogEvaluation::RecoveryRequested {
            reason: LogicalStallRecoveryReason::ReconnectLoadingStall,
        }
    );
    assert_eq!(wd.loading_confirmation_count(), 2);
}

#[test]
fn test_world_settle_threshold_arms_then_confirms() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::WorldSettle, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // 119_999 ms fails closed
    let eval_sub = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_millis(LOGICAL_WORLD_SETTLE_STALL_MS - 1)),
        0,
        now,
    );
    assert_eq!(eval_sub, LogicalWatchdogEvaluation::NoAction);
    assert_eq!(wd.world_settle_confirmation_count(), 0);

    // Poll 1 at 120_000 ms: arms confirmation
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_millis(LOGICAL_WORLD_SETTLE_STALL_MS)),
        0,
        now,
    );
    assert_eq!(
        eval1,
        LogicalWatchdogEvaluation::ArmingWorldSettleConfirmation { count: 1 }
    );
    assert_eq!(wd.world_settle_confirmation_count(), 1);

    // Poll 2 at 122_000 ms: confirms ReconnectWorldSettleStall
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_millis(LOGICAL_WORLD_SETTLE_STALL_MS + 2000)),
        0,
        now + Duration::from_secs(2),
    );
    assert_eq!(
        eval2,
        LogicalWatchdogEvaluation::RecoveryRequested {
            reason: LogicalStallRecoveryReason::ReconnectWorldSettleStall,
        }
    );
    assert_eq!(wd.world_settle_confirmation_count(), 2);
}

#[test]
fn test_world_before_false_blocks_recovery() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    // world_seen_before_episode = false (initial startup loading)
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, false);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
    assert_eq!(wd.loading_confirmation_count(), 0);
}

#[test]
fn test_inactive_episode_blocks_recovery() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    // active = false
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, false, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
}

#[test]
fn test_state_change_resets_confirmation() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap_loading = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // Poll 1: Loading arms confirmation 1
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap_loading),
        Some(1),
        Some(Duration::from_secs(185)),
        0,
        now,
    );
    assert_eq!(
        eval1,
        LogicalWatchdogEvaluation::ArmingLoadingConfirmation { count: 1 }
    );

    // State changes to WorldSettle (sub-threshold, 5s)
    let snap_settle = make_valid_reconnect_snapshot(11, 1, ReconnectState::WorldSettle, true, true);
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap_settle),
        Some(1),
        Some(Duration::from_secs(5)),
        0,
        now + Duration::from_secs(2),
    );
    assert_eq!(eval2, LogicalWatchdogEvaluation::NoAction);
    assert_eq!(wd.loading_confirmation_count(), 0);
    assert_eq!(wd.world_settle_confirmation_count(), 0);
}

#[test]
fn test_episode_change_resets_confirmation() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap_ep1 = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // Poll 1: Loading arms confirmation 1
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap_ep1),
        Some(1),
        Some(Duration::from_secs(185)),
        0,
        now,
    );
    assert_eq!(
        eval1,
        LogicalWatchdogEvaluation::ArmingLoadingConfirmation { count: 1 }
    );

    // Episode changes to 2
    let snap_ep2 = make_valid_reconnect_snapshot(15, 2, ReconnectState::Loading, true, true);
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap_ep2),
        Some(2),
        Some(Duration::from_secs(5)),
        0,
        now + Duration::from_secs(2),
    );
    assert_eq!(eval2, LogicalWatchdogEvaluation::NoAction);
    assert_eq!(wd.loading_confirmation_count(), 0);
}

#[test]
fn test_transport_safety_stale_health_blocks_recovery() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // Health progress is 10_001 ms old (> 10s fresh threshold)
    let stale_health_age = Duration::from_millis(HEALTH_TRANSPORT_FRESH_MS + 1);

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(stale_health_age),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
}

#[test]
fn test_transport_safety_stale_reconnect_blocks_recovery() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // Reconnect progress is 10_001 ms old (> 10s fresh threshold)
    let stale_reconnect_age = Duration::from_millis(RECONNECT_TRANSPORT_FRESH_MS + 1);

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(stale_reconnect_age),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
}

#[test]
fn test_transport_safety_error_reads_block_recovery() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // 1. Health MissingFile
    let missing_health = HealthObservationEvent::MissingFile {
        condition_changed: true,
    };
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::MissingFile,
        Some(Duration::from_millis(1000)),
        &missing_health,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );
    assert_eq!(eval1, LogicalWatchdogEvaluation::NoAction);

    // 2. Health InvalidOrIo
    let invalid_health = HealthObservationEvent::InvalidOrIo {
        error: "err".into(),
        condition_changed: true,
    };
    let eval2 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::InvalidOrIo,
        Some(Duration::from_millis(1000)),
        &invalid_health,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );
    assert_eq!(eval2, LogicalWatchdogEvaluation::NoAction);

    // 3. Reconnect MissingFile
    let missing_reconnect = ReconnectObservationEvent::MissingFile {
        condition_changed: true,
    };
    let eval3 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::MissingFile,
        Some(Duration::from_millis(1000)),
        &missing_reconnect,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );
    assert_eq!(eval3, LogicalWatchdogEvaluation::NoAction);

    // 4. Reconnect SequenceRegression
    let reg_reconnect = ReconnectObservationEvent::SequenceRegression {
        last_sequence: 10,
        seen_sequence: 9,
        condition_changed: true,
    };
    let eval4 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::SequenceRegression,
        Some(Duration::from_millis(1000)),
        &reg_reconnect,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );
    assert_eq!(eval4, LogicalWatchdogEvaluation::NoAction);
}

#[test]
fn test_unauthorized_states_never_recover() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    let unauthorized_states = [
        ReconnectState::Idle,
        ReconnectState::NativeWait,
        ReconnectState::Login,
        ReconnectState::Server,
        ReconnectState::Character,
        ReconnectState::Other,
    ];

    for state in unauthorized_states {
        let snap = make_valid_reconnect_snapshot(10, 1, state, true, true);
        let eval = wd.evaluate(
            false,
            false,
            "running",
            false,
            true,
            true,
            1,
            Some(1),
            Some(1),
            true,
            HealthReadCondition::Healthy,
            Some(Duration::from_millis(1000)),
            &health_event,
            true,
            ReconnectReadCondition::Healthy,
            Some(Duration::from_millis(1000)),
            &reconnect_event,
            Some(&snap),
            Some(1),
            Some(Duration::from_secs(500)),
            0,
            now,
        );
        assert_eq!(
            eval,
            LogicalWatchdogEvaluation::NoAction,
            "State {:?} must never authorize logical watchdog recovery",
            state
        );
    }
}

#[test]
fn test_shared_backoff_epoch_change_resets_confirmation() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let reconnect_event = make_healthy_reconnect_event(10);

    // Poll 1 with epoch 0: arms confirmation 1
    let eval1 = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );
    assert_eq!(
        eval1,
        LogicalWatchdogEvaluation::ArmingLoadingConfirmation { count: 1 }
    );
    assert_eq!(wd.loading_confirmation_count(), 1);

    // Suppose a 30-minute backoff occurs and expires, advancing epoch to 1!
    // Next poll with epoch 1 MUST reset confirmation and become poll 1 ONLY!
    let eval_epoch_changed = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(2002)),
        1, // epoch is now 1
        now + Duration::from_secs(1802),
    );
    // Crucial: Must be Arming (count 1), NOT RecoveryRequested!
    assert_eq!(
        eval_epoch_changed,
        LogicalWatchdogEvaluation::ArmingLoadingConfirmation { count: 1 }
    );
    assert_eq!(wd.loading_confirmation_count(), 1);
    assert_eq!(wd.last_seen_shared_backoff_epoch(), 1);

    // Second consecutive poll with epoch 1 can now recover!
    let eval_epoch_2nd = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(2004)),
        1,
        now + Duration::from_secs(1804),
    );
    assert_eq!(
        eval_epoch_2nd,
        LogicalWatchdogEvaluation::RecoveryRequested {
            reason: LogicalStallRecoveryReason::ReconnectLoadingStall,
        }
    );
    assert_eq!(wd.loading_confirmation_count(), 2);
}

#[test]
fn test_health_frozen_jvm_cannot_pass_r3c_fresh_health_authorization() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    let snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let reconnect_event = make_healthy_reconnect_event(10);

    // If health is frozen for 30s (as in R3B freeze condition), health progress age is 30_000ms.
    // R3C requires health progress age <= 10_000ms.
    let frozen_health_age = Duration::from_millis(30000);
    let health_event = HealthObservationEvent::Unchanged {
        sequence: 50,
        condition_changed: false,
    };

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(frozen_health_age),
        &health_event,
        true,
        ReconnectReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &reconnect_event,
        Some(&snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    // Strictly blocked: cannot authorize logical restart on frozen game loop!
    assert_eq!(eval, LogicalWatchdogEvaluation::NoAction);
}

#[test]
fn test_stale_snapshot_with_missing_read_cannot_authorize() {
    let mut wd = LogicalReconnectWatchdog::new(0);
    wd.on_successful_spawn(1, 0);

    let now = Instant::now();
    // Suppose last_valid_snapshot from disk 5 minutes ago was Loading,
    // but the current read is MissingFile!
    let stale_snap = make_valid_reconnect_snapshot(10, 1, ReconnectState::Loading, true, true);
    let health_event = make_healthy_health_event(50);
    let missing_reconnect = ReconnectObservationEvent::MissingFile {
        condition_changed: true,
    };

    let eval = wd.evaluate(
        false,
        false,
        "running",
        false,
        true,
        true,
        1,
        Some(1),
        Some(1),
        true,
        HealthReadCondition::Healthy,
        Some(Duration::from_millis(1000)),
        &health_event,
        true,
        ReconnectReadCondition::MissingFile,
        Some(Duration::from_millis(1000)),
        &missing_reconnect,
        Some(&stale_snap),
        Some(1),
        Some(Duration::from_secs(200)),
        0,
        now,
    );

    assert_eq!(
        eval,
        LogicalWatchdogEvaluation::NoAction,
        "Stale last-known snapshot with MissingFile current read must not authorize"
    );
}

#[test]
fn test_shared_budget_combined_r3b_and_r3c_exhaustion() {
    // Verifies that R3C uses GameLoopWatchdog shared budget:
    // Two previous R3B freeze recoveries + one R3C action causes shared 3rd-attempt 30-minute backoff.
    use game_loop_watchdog::GameLoopWatchdog;

    let t0 = Instant::now();
    let mut shared_watchdog = GameLoopWatchdog::new(t0);

    // 1st recovery (e.g. R3B GameLoopFreeze)
    let t1 = t0 + Duration::from_secs(65);
    assert_eq!(shared_watchdog.check_budget(t1), Ok(()));
    shared_watchdog.arm_recovery_attempt(t1);
    assert_eq!(shared_watchdog.recoveries_in_window(), 1);

    // 2nd recovery (e.g. R3B StartupNoHealth)
    let t2 = t1 + Duration::from_secs(65);
    assert_eq!(shared_watchdog.check_budget(t2), Ok(()));
    shared_watchdog.arm_recovery_attempt(t2);
    assert_eq!(shared_watchdog.recoveries_in_window(), 2);

    // 3rd recovery: R3C LogicalReconnectWatchdog confirms Loading stall
    let t3 = t2 + Duration::from_secs(65);
    assert_eq!(shared_watchdog.check_budget(t3), Ok(()));
    shared_watchdog.arm_recovery_attempt(t3); // shared budget consumed!
    assert_eq!(shared_watchdog.recoveries_in_window(), 3);

    // 3rd action must arm 30-minute backoff!
    assert!(shared_watchdog.is_in_backoff(t3 + Duration::from_secs(1)));

    // R3C cannot act during shared backoff:
    let t_during = t3 + Duration::from_secs(300);
    assert!(shared_watchdog.check_budget(t_during).is_err());
}

#[test]
fn test_no_second_budget_or_direct_java_spawn_in_logical_watchdog() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let logical_path = manifest_dir.join("src/logical_reconnect_watchdog.rs");
    let content =
        std::fs::read_to_string(&logical_path).expect("logical_reconnect_watchdog.rs must exist");

    // Forbidden in logical_reconnect_watchdog.rs:
    let forbidden = [
        "backoff_until",
        "recoveries_in_window",
        "window_started_at",
        "check_budget",
        "arm_recovery_attempt",
        "process_unix::stop",
        "process_unix::spawn",
        "Command::new(\"java\"",
        "reconcile_desired_state",
        "restarts +=",
    ];

    for word in forbidden {
        assert!(
            !content.contains(word),
            "logical_reconnect_watchdog.rs must NOT own budget or process lifecycle ({word}): pure authorization only!"
        );
    }
}

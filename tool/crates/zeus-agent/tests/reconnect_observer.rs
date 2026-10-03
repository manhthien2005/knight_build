use std::time::{Duration, Instant};
use zeus_core::wire::{
    RECONNECT_STATUS_FILE_NAME, ReconnectState, ReconnectStatusError, ReconnectStatusSnapshot,
    clear_reconnect_status,
};

#[path = "../src/reconnect_observer.rs"]
#[allow(dead_code)]
mod reconnect_observer;

use reconnect_observer::{ReconnectObservationEvent, ReconnectObserver, ReconnectReadCondition};

#[test]
fn test_first_sample_establishes_baseline() {
    let mut observer = ReconnectObserver::new();
    let now = Instant::now();

    // Not supervised initially
    assert_eq!(
        observer.observe_reading(Ok(None), now),
        ReconnectObservationEvent::NotSupervised
    );

    observer.reset_for_new_generation(1, Some(1234));
    assert_eq!(
        observer.last_read_condition(),
        ReconnectReadCondition::AwaitingFirstSample
    );

    let sample = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Login,
        transitions: 2,
        world_seen_before_episode: true,
    };

    let event = observer.observe_reading(Ok(Some(sample.clone())), now);
    assert_eq!(
        event,
        ReconnectObservationEvent::BaselineSample {
            sequence: 1,
            episode_id: 10,
            active: true,
            state: ReconnectState::Login,
            transitions: 2,
            world_seen_before_episode: true,
            condition_changed: true,
        }
    );

    assert_eq!(observer.last_sequence(), Some(1));
    assert_eq!(observer.current_episode_id(), Some(10));
    assert_eq!(observer.current_state(), Some(ReconnectState::Login));
    assert_eq!(observer.state_since(), Some(now));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample));
}

#[test]
fn test_first_sample_inactive_idle_establishes_baseline_consistently() {
    let mut observer = ReconnectObserver::new();
    let now = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 0,
        active: false,
        state: ReconnectState::Idle,
        transitions: 0,
        world_seen_before_episode: false,
    };

    let event = observer.observe_reading(Ok(Some(sample)), now);
    assert_eq!(
        event,
        ReconnectObservationEvent::BaselineSample {
            sequence: 1,
            episode_id: 0,
            active: false,
            state: ReconnectState::Idle,
            transitions: 0,
            world_seen_before_episode: false,
            condition_changed: true,
        }
    );

    assert_eq!(observer.last_sequence(), Some(1));
    assert_eq!(observer.current_episode_id(), Some(0));
    assert_eq!(observer.current_state(), Some(ReconnectState::Idle));
    assert_eq!(observer.state_since(), Some(now));
}

#[test]
fn test_same_episode_state_increasing_seq_keeps_state_since_unchanged() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 5,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.state_since(), Some(t0));

    // Tick advances seq to 2 after 1 second
    let t1 = t0 + Duration::from_secs(1);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 2000,
        sequence: 2,
        episode_id: 5,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    // Sequence advanced, but episode/state/active did NOT change
    assert_eq!(
        event,
        ReconnectObservationEvent::Progress {
            sequence: 2,
            episode_changed: None,
            active_changed: None,
            state_changed: None,
            transitions_changed: None,
            condition_changed: false,
        }
    );

    // CRITICAL: state_since MUST remain t0!
    assert_eq!(observer.state_since(), Some(t0));
    assert_eq!(
        observer.duration_in_current_state(t1),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn test_state_change_resets_state_since() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 5,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.state_since(), Some(t0));

    // Transition to WorldSettle at t1
    let t1 = t0 + Duration::from_secs(3);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 4000,
        sequence: 2,
        episode_id: 5,
        active: true,
        state: ReconnectState::WorldSettle,
        transitions: 2,
        world_seen_before_episode: true,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    assert_eq!(
        event,
        ReconnectObservationEvent::Progress {
            sequence: 2,
            episode_changed: None,
            active_changed: None,
            state_changed: Some((ReconnectState::Loading, ReconnectState::WorldSettle)),
            transitions_changed: Some((1, 2)),
            condition_changed: false,
        }
    );

    // state_since is reset to t1
    assert_eq!(observer.state_since(), Some(t1));
    assert_eq!(observer.current_state(), Some(ReconnectState::WorldSettle));
    assert_eq!(
        observer.duration_in_current_state(t1),
        Some(Duration::from_secs(0))
    );
}

#[test]
fn test_episode_id_change_resets_state_since() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 1,
        active: true,
        state: ReconnectState::NativeWait,
        transitions: 0,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);

    let t1 = t0 + Duration::from_secs(5);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 6000,
        sequence: 2,
        episode_id: 2,
        active: true,
        state: ReconnectState::NativeWait,
        transitions: 0,
        world_seen_before_episode: true,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    assert_eq!(
        event,
        ReconnectObservationEvent::Progress {
            sequence: 2,
            episode_changed: Some((1, 2)),
            active_changed: None,
            state_changed: None,
            transitions_changed: None,
            condition_changed: false,
        }
    );

    assert_eq!(observer.state_since(), Some(t1));
    assert_eq!(observer.current_episode_id(), Some(2));
}

#[test]
fn test_active_to_idle_resets_state_since_consistently() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 1,
        active: true,
        state: ReconnectState::Login,
        transitions: 2,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);

    // Episode closes -> active=0, state=idle
    let t1 = t0 + Duration::from_secs(10);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 11000,
        sequence: 2,
        episode_id: 1,
        active: false,
        state: ReconnectState::Idle,
        transitions: 2,
        world_seen_before_episode: false,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    assert_eq!(
        event,
        ReconnectObservationEvent::Progress {
            sequence: 2,
            episode_changed: None,
            active_changed: Some((true, false)),
            state_changed: Some((ReconnectState::Login, ReconnectState::Idle)),
            transitions_changed: None,
            condition_changed: false,
        }
    );

    assert_eq!(observer.state_since(), Some(t1));
    assert_eq!(observer.current_state(), Some(ReconnectState::Idle));
    assert_eq!(
        observer.duration_in_current_state(t1),
        Some(Duration::from_secs(0))
    );
}

#[test]
fn test_missing_read_preserves_last_good_state() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 5,
        episode_id: 2,
        active: true,
        state: ReconnectState::Character,
        transitions: 3,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    // Gap: file is missing
    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(Ok(None), t1);
    assert_eq!(
        event,
        ReconnectObservationEvent::MissingFile {
            condition_changed: true
        }
    );

    // Preserves last known good state and state_since
    assert_eq!(observer.last_sequence(), Some(5));
    assert_eq!(observer.current_episode_id(), Some(2));
    assert_eq!(observer.current_state(), Some(ReconnectState::Character));
    assert_eq!(observer.state_since(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
}

#[test]
fn test_malformed_read_preserves_last_good_state() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 5,
        episode_id: 2,
        active: true,
        state: ReconnectState::Server,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(Err(ReconnectStatusError::InvalidEncoding), t1);
    assert!(matches!(
        event,
        ReconnectObservationEvent::InvalidOrIo { .. }
    ));

    // Preserves last known good state and state_since
    assert_eq!(observer.last_sequence(), Some(5));
    assert_eq!(observer.current_state(), Some(ReconnectState::Server));
    assert_eq!(observer.state_since(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
}

#[test]
fn test_same_generation_seq_regression_does_not_reset_state_duration() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 10,
        episode_id: 3,
        active: true,
        state: ReconnectState::Login,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    // Regressed seq 8 in same generation (anomaly)
    let t1 = t0 + Duration::from_secs(2);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 900,
        sequence: 8,
        episode_id: 3,
        active: true,
        state: ReconnectState::Login,
        transitions: 1,
        world_seen_before_episode: true,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    assert_eq!(
        event,
        ReconnectObservationEvent::SequenceRegression {
            last_sequence: 10,
            seen_sequence: 8,
            condition_changed: true,
        }
    );

    // Sequence regression is NOT accepted as progress, state duration is preserved
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.state_since(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
}

#[test]
fn test_new_process_generation_accepts_lower_seq_episode_as_new_baseline() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(1234));

    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 50000,
        sequence: 100,
        episode_id: 15,
        active: true,
        state: ReconnectState::Loading,
        transitions: 3,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.last_sequence(), Some(100));

    // Explicit new process generation (e.g. JVM restart)
    let t1 = t0 + Duration::from_secs(10);
    observer.reset_for_new_generation(2, Some(5678));
    assert_eq!(observer.last_sequence(), None);
    assert_eq!(observer.state_since(), None);

    // New generation publishes seq=1, episode=0
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 0,
        active: false,
        state: ReconnectState::Idle,
        transitions: 0,
        world_seen_before_episode: false,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    assert_eq!(
        event,
        ReconnectObservationEvent::BaselineSample {
            sequence: 1,
            episode_id: 0,
            active: false,
            state: ReconnectState::Idle,
            transitions: 0,
            world_seen_before_episode: false,
            condition_changed: true,
        }
    );
    assert_eq!(observer.last_sequence(), Some(1));
    assert_eq!(observer.current_episode_id(), Some(0));
    assert_eq!(observer.current_state(), Some(ReconnectState::Idle));
    assert_eq!(observer.state_since(), Some(t1));
}

#[test]
fn test_duration_in_current_state_uses_local_instant() {
    let mut observer = ReconnectObserver::new();
    let t0 = Instant::now();
    observer.reset_for_new_generation(1, Some(100));

    let sample = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 1,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample)), t0);

    let t_plus_5s = t0 + Duration::from_secs(5);
    assert_eq!(
        observer.duration_in_current_state(t_plus_5s),
        Some(Duration::from_secs(5))
    );

    let t_plus_12s = t0 + Duration::from_secs(12);
    assert_eq!(
        observer.duration_in_current_state(t_plus_12s),
        Some(Duration::from_secs(12))
    );
}

#[test]
fn test_lifecycle_safe_clear_reconnect_status() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let status_path = home.join(RECONNECT_STATUS_FILE_NAME);

    // Missing file clear succeeds
    assert!(clear_reconnect_status(home).is_ok());

    // Existing file is removed
    std::fs::write(&status_path, "v=1\n").unwrap();
    assert!(status_path.exists());
    assert!(clear_reconnect_status(home).is_ok());
    assert!(!status_path.exists());
}

#[test]
fn test_invariants_source_code_inspection() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let observer_path = manifest_dir.join("src/reconnect_observer.rs");
    let content =
        std::fs::read_to_string(&observer_path).expect("reconnect_observer.rs must exist");

    // 1. Verify no stall thresholds are defined in R3A
    assert!(
        !content.contains("RC_LOADING_STALE_SECS"),
        "RC_LOADING_STALE_SECS must not be defined"
    );
    assert!(
        !content.contains("RC_WORLD_SETTLE_STALE_SECS"),
        "RC_WORLD_SETTLE_STALE_SECS must not be defined"
    );

    // 2. Verify no restart, stop, or kill conclusions in observer
    let forbidden_words = [
        "stuck",
        "frozen",
        "restart required",
        "process_unix::stop",
        "reconcile_desired_state",
    ];
    for word in forbidden_words {
        assert!(
            !content.contains(word),
            "reconnect_observer.rs must not contain forbidden phrase: {word}"
        );
    }
}

#[test]
fn test_reconnect_first_valid_sample_sets_last_progress_at() {
    let mut observer = ReconnectObserver::new();
    assert_eq!(observer.last_progress_at(), None);

    observer.reset_for_new_generation(1, Some(1234));
    assert_eq!(observer.last_progress_at(), None);

    let t0 = Instant::now();
    let sample = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };

    let event = observer.observe_reading(Ok(Some(sample)), t0);
    assert!(matches!(
        event,
        ReconnectObservationEvent::BaselineSample { .. }
    ));
    assert_eq!(observer.last_progress_at(), Some(t0));
}

#[test]
fn test_reconnect_sequence_progress_updates_last_progress_at() {
    let mut observer = ReconnectObserver::new();
    observer.reset_for_new_generation(1, Some(1234));

    let t0 = Instant::now();
    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.last_progress_at(), Some(t0));

    let t1 = t0 + Duration::from_secs(2);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 3000,
        sequence: 2,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    let event = observer.observe_reading(Ok(Some(sample2)), t1);
    assert!(matches!(event, ReconnectObservationEvent::Progress { .. }));
    assert_eq!(observer.last_progress_at(), Some(t1));
}

#[test]
fn test_reconnect_unchanged_does_not_update_last_progress_at() {
    let mut observer = ReconnectObserver::new();
    observer.reset_for_new_generation(1, Some(1234));

    let t0 = Instant::now();
    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1.clone())), t0);
    assert_eq!(observer.last_progress_at(), Some(t0));

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(Ok(Some(sample1)), t1);
    assert!(matches!(event, ReconnectObservationEvent::Unchanged { .. }));
    // Crucial: last_progress_at MUST NOT be updated on Unchanged!
    assert_eq!(observer.last_progress_at(), Some(t0));
}

#[test]
fn test_reconnect_missing_invalid_regression_does_not_update_last_progress_at() {
    let mut observer = ReconnectObserver::new();
    observer.reset_for_new_generation(1, Some(1234));

    let t0 = Instant::now();
    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 5,
        episode_id: 10,
        active: true,
        state: ReconnectState::WorldSettle,
        transitions: 2,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.last_progress_at(), Some(t0));

    // MissingFile
    let t1 = t0 + Duration::from_secs(2);
    let event1 = observer.observe_reading(Ok(None), t1);
    assert!(matches!(
        event1,
        ReconnectObservationEvent::MissingFile { .. }
    ));
    assert_eq!(observer.last_progress_at(), Some(t0));

    // InvalidOrIo
    let t2 = t1 + Duration::from_secs(2);
    let event2 = observer.observe_reading(Err(ReconnectStatusError::MalformedInteger("seq")), t2);
    assert!(matches!(
        event2,
        ReconnectObservationEvent::InvalidOrIo { .. }
    ));
    assert_eq!(observer.last_progress_at(), Some(t0));

    // SequenceRegression
    let t3 = t2 + Duration::from_secs(2);
    let sample_regression = ReconnectStatusSnapshot {
        written_at_unix_ms: 2000,
        sequence: 3, // regression from 5
        episode_id: 10,
        active: true,
        state: ReconnectState::WorldSettle,
        transitions: 2,
        world_seen_before_episode: true,
    };
    let event3 = observer.observe_reading(Ok(Some(sample_regression)), t3);
    assert!(matches!(
        event3,
        ReconnectObservationEvent::SequenceRegression { .. }
    ));
    assert_eq!(observer.last_progress_at(), Some(t0));
}

#[test]
fn test_reconnect_new_generation_and_stopped_resets_last_progress_at() {
    let mut observer = ReconnectObserver::new();
    observer.reset_for_new_generation(1, Some(1234));

    let t0 = Instant::now();
    let sample1 = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.last_progress_at(), Some(t0));

    // reset_for_new_generation clears last_progress_at
    observer.reset_for_new_generation(2, Some(5678));
    assert_eq!(observer.last_progress_at(), None);

    // establish baseline in gen 2
    let t1 = t0 + Duration::from_secs(10);
    let sample2 = ReconnectStatusSnapshot {
        written_at_unix_ms: 11000,
        sequence: 1,
        episode_id: 1,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample2)), t1);
    assert_eq!(observer.last_progress_at(), Some(t1));

    // reset_stopped clears last_progress_at
    observer.reset_stopped();
    assert_eq!(observer.last_progress_at(), None);
}

#[test]
fn test_reconnect_duration_since_last_progress_uses_local_instant() {
    let mut observer = ReconnectObserver::new();
    assert_eq!(observer.duration_since_last_progress(Instant::now()), None);

    observer.reset_for_new_generation(1, Some(1234));
    let t0 = Instant::now();
    let sample = ReconnectStatusSnapshot {
        written_at_unix_ms: 1000,
        sequence: 1,
        episode_id: 10,
        active: true,
        state: ReconnectState::Loading,
        transitions: 1,
        world_seen_before_episode: true,
    };
    observer.observe_reading(Ok(Some(sample)), t0);

    let t1 = t0 + Duration::from_millis(4500);
    assert_eq!(
        observer.duration_since_last_progress(t1),
        Some(Duration::from_millis(4500))
    );

    // monotonic check: earlier time returns None (saturating/checked)
    let t_before = t0 - Duration::from_secs(1);
    assert_eq!(observer.duration_since_last_progress(t_before), None);
}

#[test]
fn test_main_loop_reconnect_integration_inspection() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_loop_path = manifest_dir.join("src/main_loop.rs");
    let content = std::fs::read_to_string(&main_loop_path).expect("main_loop.rs must exist");

    // 1. Reconnect status cleared before spawn
    assert!(
        content.contains("clear_reconnect_status(&paths.home)"),
        "clear_reconnect_status must be called in main_loop.rs"
    );

    // 2. Both observers use the same process generation and pid on spawn
    assert!(
        content.contains("acc.health_observer.reset_for_new_generation(acc.process_generation, Some(child.pid as u32));")
            && content.contains("acc.reconnect_observer.reset_for_new_generation(acc.process_generation, Some(child.pid as u32));"),
        "both health and reconnect observers must reset with the same process generation and pid"
    );

    // 3. Both observers reset stopped on confirmed stop
    assert!(
        content.contains("acc.health_observer.reset_stopped();\n                    acc.reconnect_observer.reset_stopped();"),
        "both observers must reset_stopped on confirmed stop"
    );

    // 4. Both observers observed in 2-second tick
    assert!(
        content.contains("acc.health_observer.observe_account_home(&paths.home, &acc.id, now);")
            && content.contains(
                "acc.reconnect_observer.observe_account_home(&paths.home, &acc.id, now);"
            ),
        "both observers must be called on 2s supervision tick"
    );

    // 5. No new polling thread exists for reconnect
    for line in content.lines() {
        if line.contains("thread::spawn") {
            assert!(
                !line.contains("reconnect"),
                "no new thread should be created for reconnect observation: {line}"
            );
        }
    }

    // 6. No reconnect-state-derived stop/restart/reconcile path exists
    for line in content.lines() {
        if line.contains("reconnect_observer") {
            assert!(
                !line.contains("stop(")
                    && !line.contains("restarts +=")
                    && !line.contains("reconcile_desired_state"),
                "reconnect observation line must not invoke stop, restarts, or reconcile: {line}"
            );
        }
    }

    // 7. RuntimePayload does not contain reconnect fields
    let supabase_rest_path = manifest_dir.join("src/supabase_rest.rs");
    let sb_content =
        std::fs::read_to_string(&supabase_rest_path).expect("supabase_rest.rs must exist");
    let start = sb_content
        .find("pub struct RuntimePayload {")
        .expect("RuntimePayload must exist");
    let end = sb_content[start..]
        .find('}')
        .expect("RuntimePayload closing brace");
    let struct_body = &sb_content[start..start + end];
    assert!(
        !struct_body.contains("reconnect"),
        "RuntimePayload struct must not contain reconnect fields"
    );
}

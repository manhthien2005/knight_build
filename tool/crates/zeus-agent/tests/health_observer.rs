//! Integration tests for the observe-only health sensor (AUTO-RECONNECT-R1B).

use std::time::{Duration, Instant};
use zeus_core::wire::{
    clear_health, read_health, HealthError, HealthScreen, HealthSnapshot, HEALTH_FILE_NAME,
};

#[path = "../src/health_observer.rs"]
#[allow(dead_code)]
mod health_observer;

use health_observer::{HealthObservationEvent, HealthObserver, HealthReadCondition};

fn make_sample(seq: u64, screen: HealthScreen, dialog: bool, disconnect: bool) -> HealthSnapshot {
    HealthSnapshot {
        written_at_unix_ms: 1_700_000_000_000 + (seq as i64 * 1000),
        sequence: seq,
        screen,
        dialog_open: dialog,
        native_disconnect: disconnect,
    }
}

#[test]
fn test_new_process_generation_starts_awaiting_first_sample() {
    let mut observer = HealthObserver::new();
    assert_eq!(observer.current_process_generation(), None);
    assert_eq!(observer.last_read_condition(), HealthReadCondition::Stopped);

    observer.reset_for_new_generation(1, Some(1001));
    assert_eq!(observer.current_process_generation(), Some(1));
    assert_eq!(observer.current_pid(), Some(1001));
    assert_eq!(observer.last_sequence(), None);
    assert_eq!(observer.last_progress_at(), None);
    assert_eq!(observer.last_valid_snapshot(), None);
    assert_eq!(
        observer.last_read_condition(),
        HealthReadCondition::AwaitingFirstSample
    );
}

#[test]
fn test_first_valid_health_sample_establishes_baseline_and_progress_time() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample = make_sample(42, HealthScreen::Login, false, false);
    let event = observer.observe_reading(Ok(Some(sample.clone())), t0);

    assert_eq!(observer.last_sequence(), Some(42));
    assert_eq!(observer.last_progress_at(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample));
    assert_eq!(observer.last_read_condition(), HealthReadCondition::Healthy);

    match event {
        HealthObservationEvent::BaselineSample {
            sequence,
            screen,
            dialog_open,
            native_disconnect,
            condition_changed,
        } => {
            assert_eq!(sequence, 42);
            assert_eq!(screen, HealthScreen::Login);
            assert!(!dialog_open);
            assert!(!native_disconnect);
            assert!(condition_changed);
        }
        other => panic!("expected BaselineSample, got {:?}", other),
    }
}

#[test]
fn test_increasing_sequence_advances_last_progress_at() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(10, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1)), t0);

    let t1 = t0 + Duration::from_secs(2);
    let sample2 = make_sample(11, HealthScreen::World, false, false);
    let event = observer.observe_reading(Ok(Some(sample2.clone())), t1);

    assert_eq!(observer.last_sequence(), Some(11));
    assert_eq!(observer.last_progress_at(), Some(t1));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample2));
    assert_eq!(observer.last_read_condition(), HealthReadCondition::Healthy);

    match event {
        HealthObservationEvent::Progress {
            sequence,
            screen_changed,
            disconnect_changed,
            dialog_changed,
            condition_changed,
        } => {
            assert_eq!(sequence, 11);
            assert_eq!(screen_changed, None);
            assert_eq!(disconnect_changed, None);
            assert_eq!(dialog_changed, None);
            assert!(!condition_changed);
        }
        other => panic!("expected Progress, got {:?}", other),
    }
}

#[test]
fn test_same_sequence_does_not_advance_last_progress_at() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(10, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    let t1 = t0 + Duration::from_secs(2);
    let sample2 = make_sample(10, HealthScreen::World, false, false);
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    // Sequence unchanged: progress time remains t0!
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.last_progress_at(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
    assert_eq!(observer.last_read_condition(), HealthReadCondition::Healthy);

    match event {
        HealthObservationEvent::Unchanged {
            sequence,
            condition_changed,
        } => {
            assert_eq!(sequence, 10);
            assert!(!condition_changed);
        }
        other => panic!("expected Unchanged, got {:?}", other),
    }
}

#[test]
fn test_missing_health_file_preserves_last_good_sequence_and_progress_time() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(10, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(Ok(None), t1);

    // State is preserved
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.last_progress_at(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
    assert_eq!(
        observer.last_read_condition(),
        HealthReadCondition::MissingFile
    );

    match event {
        HealthObservationEvent::MissingFile { condition_changed } => {
            assert!(condition_changed);
        }
        other => panic!("expected MissingFile, got {:?}", other),
    }

    // A second missing file does not flag condition_changed again
    let t2 = t1 + Duration::from_secs(2);
    let event2 = observer.observe_reading(Ok(None), t2);
    match event2 {
        HealthObservationEvent::MissingFile { condition_changed } => {
            assert!(!condition_changed);
        }
        other => panic!(
            "expected MissingFile without condition change, got {:?}",
            other
        ),
    }
}

#[test]
fn test_malformed_health_result_preserves_last_good_sequence_and_progress_time() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(10, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(Err(HealthError::MalformedInteger("seq")), t1);

    // State is preserved
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.last_progress_at(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
    assert_eq!(
        observer.last_read_condition(),
        HealthReadCondition::InvalidOrIo
    );

    match event {
        HealthObservationEvent::InvalidOrIo {
            error,
            condition_changed,
        } => {
            assert!(error.contains("seq"));
            assert!(condition_changed);
        }
        other => panic!("expected InvalidOrIo, got {:?}", other),
    }
}

#[test]
fn test_sequence_regression_in_same_generation_is_observable_and_not_progress() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(10, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1.clone())), t0);

    let t1 = t0 + Duration::from_secs(2);
    let sample2 = make_sample(9, HealthScreen::World, false, false);
    let event = observer.observe_reading(Ok(Some(sample2)), t1);

    // Sequence regression in same generation: preserved good state, not progress
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.last_progress_at(), Some(t0));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample1));
    assert_eq!(
        observer.last_read_condition(),
        HealthReadCondition::SequenceRegression
    );

    match event {
        HealthObservationEvent::SequenceRegression {
            last_sequence,
            seen_sequence,
            condition_changed,
        } => {
            assert_eq!(last_sequence, 10);
            assert_eq!(seen_sequence, 9);
            assert!(condition_changed);
        }
        other => panic!("expected SequenceRegression, got {:?}", other),
    }
}

#[test]
fn test_explicit_new_process_generation_allows_lower_sequence_as_new_baseline() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample1 = make_sample(500, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample1)), t0);
    assert_eq!(observer.last_sequence(), Some(500));

    // Process restarted with generation 2
    observer.reset_for_new_generation(2, Some(2002));
    assert_eq!(observer.current_process_generation(), Some(2));
    assert_eq!(observer.current_pid(), Some(2002));
    assert_eq!(observer.last_sequence(), None);
    assert_eq!(observer.last_progress_at(), None);

    // New generation publishes seq=1 (lower than 500)
    let t1 = t0 + Duration::from_secs(10);
    let sample2 = make_sample(1, HealthScreen::Login, false, false);
    let event = observer.observe_reading(Ok(Some(sample2.clone())), t1);

    assert_eq!(observer.last_sequence(), Some(1));
    assert_eq!(observer.last_progress_at(), Some(t1));
    assert_eq!(observer.last_valid_snapshot(), Some(&sample2));

    match event {
        HealthObservationEvent::BaselineSample { sequence, .. } => {
            assert_eq!(sequence, 1);
        }
        other => panic!("expected BaselineSample, got {:?}", other),
    }
}

#[test]
fn test_process_stopped_reset_clears_generation_specific_observation_state() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    let sample = make_sample(20, HealthScreen::World, false, false);
    observer.observe_reading(Ok(Some(sample)), t0);

    observer.reset_stopped();

    assert_eq!(observer.current_process_generation(), None);
    assert_eq!(observer.current_pid(), None);
    assert_eq!(observer.last_sequence(), None);
    assert_eq!(observer.last_progress_at(), None);
    assert_eq!(observer.last_valid_snapshot(), None);
    assert_eq!(observer.last_read_condition(), HealthReadCondition::Stopped);

    let event = observer.observe_reading(
        Ok(Some(make_sample(21, HealthScreen::World, false, false))),
        t0,
    );
    assert_eq!(event, HealthObservationEvent::NotSupervised);
}

#[test]
fn test_screen_transition_is_observable() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    observer.observe_reading(
        Ok(Some(make_sample(1, HealthScreen::Login, false, false))),
        t0,
    );

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(
        Ok(Some(make_sample(2, HealthScreen::Server, false, false))),
        t1,
    );

    match event {
        HealthObservationEvent::Progress {
            screen_changed,
            disconnect_changed,
            ..
        } => {
            assert_eq!(
                screen_changed,
                Some((HealthScreen::Login, HealthScreen::Server))
            );
            assert_eq!(disconnect_changed, None);
        }
        other => panic!("expected Progress with screen transition, got {:?}", other),
    }
}

#[test]
fn test_native_disconnect_transition_is_observable() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    observer.observe_reading(
        Ok(Some(make_sample(1, HealthScreen::World, false, false))),
        t0,
    );

    let t1 = t0 + Duration::from_secs(2);
    let event = observer.observe_reading(
        Ok(Some(make_sample(2, HealthScreen::World, false, true))),
        t1,
    );

    match event {
        HealthObservationEvent::Progress {
            screen_changed,
            disconnect_changed,
            ..
        } => {
            assert_eq!(screen_changed, None);
            assert_eq!(disconnect_changed, Some((false, true)));
        }
        other => panic!(
            "expected Progress with disconnect transition, got {:?}",
            other
        ),
    }
}

#[test]
fn test_transient_missing_read_between_advancing_valid_samples_does_not_break_progression() {
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(1001));

    let t0 = Instant::now();
    observer.observe_reading(
        Ok(Some(make_sample(10, HealthScreen::World, false, false))),
        t0,
    );

    let t1 = t0 + Duration::from_secs(2);
    let event_missing = observer.observe_reading(Ok(None), t1);
    assert_eq!(
        event_missing,
        HealthObservationEvent::MissingFile {
            condition_changed: true
        }
    );
    assert_eq!(observer.last_sequence(), Some(10));
    assert_eq!(observer.last_progress_at(), Some(t0));

    let t2 = t1 + Duration::from_secs(2);
    let event_resume = observer.observe_reading(
        Ok(Some(make_sample(11, HealthScreen::World, false, false))),
        t2,
    );
    match event_resume {
        HealthObservationEvent::Progress {
            sequence,
            condition_changed,
            ..
        } => {
            assert_eq!(sequence, 11);
            assert!(condition_changed); // Recovered from MissingFile to Healthy!
        }
        other => panic!("expected Progress on resumption, got {:?}", other),
    }
    assert_eq!(observer.last_sequence(), Some(11));
    assert_eq!(observer.last_progress_at(), Some(t2));
}

#[test]
fn test_pure_duration_since_last_progress() {
    let mut observer = HealthObserver::new();
    assert_eq!(observer.duration_since_last_progress(Instant::now()), None);

    observer.reset_for_new_generation(1, Some(1001));
    assert_eq!(observer.duration_since_last_progress(Instant::now()), None);

    let t0 = Instant::now();
    observer.observe_reading(
        Ok(Some(make_sample(1, HealthScreen::Login, false, false))),
        t0,
    );

    let t5 = t0 + Duration::from_secs(5);
    assert_eq!(
        observer.duration_since_last_progress(t5),
        Some(Duration::from_secs(5))
    );
}

#[test]
fn test_health_file_lifecycle_safe_clear() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let health_path = home.join(HEALTH_FILE_NAME);

    // Initial state: no file, clear_health succeeds without error
    assert!(clear_health(home).is_ok());

    // Write a dummy health file
    std::fs::write(
        &health_path,
        "v=1\nt=100\nseq=1\nscreen=login\ndialog=0\ndisconnect=0\n",
    )
    .unwrap();
    assert!(health_path.exists());

    // Safe clear removes file
    assert!(clear_health(home).is_ok());
    assert!(!health_path.exists());

    // Redundant clear succeeds
    assert!(clear_health(home).is_ok());
}

#[test]
fn test_lifecycle_clear_before_spawn_prevents_stale_inheritance() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let health_path = home.join(HEALTH_FILE_NAME);

    // Old JVM left a health file
    std::fs::write(
        &health_path,
        "v=1\nt=100\nseq=999\nscreen=world\ndialog=0\ndisconnect=0\n",
    )
    .unwrap();
    assert!(health_path.exists());

    // Lifecycle rule: before every spawn, call clear_health
    assert!(clear_health(home).is_ok());
    assert!(!health_path.exists());

    // New generation starts clean
    let mut observer = HealthObserver::new();
    observer.reset_for_new_generation(1, Some(5000));
    assert_eq!(observer.last_sequence(), None);
    assert_eq!(
        observer.observe_reading(read_health(home), Instant::now()),
        HealthObservationEvent::MissingFile {
            condition_changed: true
        }
    );
    assert_eq!(observer.last_sequence(), None);
}

#[test]
fn test_lifecycle_clear_failure_does_not_panic() {
    let non_existent_home = std::path::Path::new("Z:\\non_existent_drive_zeus_12345");
    // clear_health should return an error or Ok, but must not panic
    let result = clear_health(non_existent_home);
    // Even if it returns Err or Ok, it handles it gracefully
    let _ = result;
}

#[test]
fn test_invariants_source_code_inspection() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_loop_path = manifest_dir.join("src/main_loop.rs");
    let content = std::fs::read_to_string(&main_loop_path).expect("main_loop.rs must exist");

    // 1. Check that health_observer is called in 2-second tick
    assert!(
        content.contains("observe_account_home"),
        "observe_account_home must be called in main_loop.rs"
    );
    assert!(
        content.contains("if now.duration_since(last_snapshot_tick) >= Duration::from_secs(2)"),
        "2-second supervision tick must exist in main_loop.rs"
    );

    // 2. Check that no new threads are spawned for health
    for line in content.lines() {
        if line.contains("thread::spawn") {
            assert!(
                !line.contains("health"),
                "no new thread should be created for health observation: {line}"
            );
        }
    }

    // 3. Check that no staleness thresholds exist
    assert!(
        !content.contains("HEALTH_STALE_SECS"),
        "HEALTH_STALE_SECS must not exist"
    );
    assert!(
        !content.contains("stale JVM") && !content.contains("restart required"),
        "main_loop must not conclude stale JVM or restart required due to health"
    );

    // 4. Verify that health observation does not call process_unix::stop or increment restarts
    for line in content.lines() {
        if line.contains("health_observer") || line.contains("observe_account_home") {
            assert!(
                !line.contains("stop(") && !line.contains("restarts +="),
                "health observation line must not invoke stop or modify restarts: {line}"
            );
        }
    }
}

#[test]
fn test_runtime_payload_has_no_health_fields() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let supabase_rest_path = manifest_dir.join("src/supabase_rest.rs");
    let content =
        std::fs::read_to_string(&supabase_rest_path).expect("supabase_rest.rs must exist");

    // Find struct RuntimePayload in supabase_rest.rs
    let start = content
        .find("pub struct RuntimePayload {")
        .expect("RuntimePayload must exist");
    let end = content[start..]
        .find('}')
        .expect("RuntimePayload closing brace");
    let struct_body = &content[start..start + end];

    assert!(
        !struct_body.contains("health"),
        "RuntimePayload struct must not contain health fields"
    );
}

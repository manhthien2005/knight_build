#![cfg(windows)]

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;
use zeus_core::runtime::CapabilityState;
use zeus_core::{CoreError, CoreState, ManagerController, ManagerErrorCode, ManagerOperation};
use zeus_core::{
    ManagerWorker, ManagerWorkerErrorCode, ManagerWorkerEvent, ManagerWorkerOperation,
    ManagerWorkerState,
};

#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;
use runtime_fixture::RuntimeFixture;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("zeus-manager-{label}-{}", Uuid::new_v4())))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn assert_send<T: Send>() {}

fn next_worker_event(worker: &mut ManagerWorker) -> ManagerWorkerEvent {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match worker.try_next_event() {
            Ok(Some(event)) => return event,
            Ok(None) if Instant::now() < deadline => thread::yield_now(),
            Ok(None) => panic!("worker event deadline expired"),
            Err(error) => panic!("worker event polling failed: {error:?}"),
        }
    }
}

fn consume_worker_ready(worker: &mut ManagerWorker) {
    assert!(matches!(
        next_worker_event(worker),
        ManagerWorkerEvent::Ready
    ));
    assert_eq!(worker.state(), ManagerWorkerState::Ready);
}

#[test]
fn manager_public_contract_is_windows_send_and_stably_coded() {
    assert_send::<ManagerController>();
    assert_eq!(ManagerErrorCode::CleanupPending.as_str(), "cleanup_pending");
    assert_eq!(ManagerOperation::StartProfile.as_str(), "start_profile");
}

#[test]
fn worker_public_protocol_is_windows_only_send_and_stably_coded() {
    assert_send::<ManagerWorker>();
    assert_send::<ManagerWorkerEvent>();
    assert_eq!(
        ManagerWorkerErrorCode::CommandQueueFull.as_str(),
        "command_queue_full"
    );
    assert_eq!(
        ManagerWorkerOperation::ReceiveEvent.as_str(),
        "receive_event"
    );
    assert_eq!(ManagerWorkerState::Starting, ManagerWorkerState::Starting);
}

#[test]
fn worker_takes_real_controller_ownership_and_releases_the_core_lock_on_drop() {
    let data_root = TestDirectory::new("worker-controller-transfer");
    let core = CoreState::open_at(&data_root.0).expect("open worker transfer Core");
    let controller = ManagerController::from_core(core);
    let mut worker = ManagerWorker::spawn_controller(controller).expect("spawn controller worker");

    let ready_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match worker.try_next_event() {
            Ok(Some(ManagerWorkerEvent::Ready)) => break,
            Ok(Some(other)) => panic!("Ready must be the first worker event, got {other:?}"),
            Ok(None) if Instant::now() < ready_deadline => thread::yield_now(),
            Ok(None) => panic!("worker Ready deadline expired"),
            Err(error) => panic!("worker startup polling failed: {error:?}"),
        }
    }
    assert_eq!(worker.state(), ManagerWorkerState::Ready);
    drop(worker);

    let reopen_deadline = Instant::now() + Duration::from_secs(1);
    let reopened = loop {
        match CoreState::open_at(&data_root.0) {
            Ok(core) => break core,
            Err(CoreError::AlreadyRunning) if Instant::now() < reopen_deadline => {
                thread::yield_now();
            }
            Err(CoreError::AlreadyRunning) => panic!("worker did not release the Core lock"),
            Err(error) => panic!("unexpected Core reopen failure: {error:?}"),
        }
    };
    drop(reopened);
}

#[test]
fn worker_queries_real_catalogs_and_empty_sessions_without_sensitive_fields() {
    let runtime_fixture = RuntimeFixture::new("worker-real-query");
    let data_root = TestDirectory::new("worker-real-query");
    let mut core = CoreState::open_at(&data_root.0).expect("open worker query Core");
    let registered = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register worker query runtime fixture");
    let profile = core
        .create_profile("Worker visible profile", &registered.runtime_id)
        .expect("create worker query profile");
    let path_sentinel = registered.runtime_root.to_string_lossy().into_owned();
    let hash_sentinel = registered.descriptor_sha256.clone();

    let controller = ManagerController::from_core(core);
    let mut worker = ManagerWorker::spawn_controller(controller).expect("spawn query worker");
    consume_worker_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_runtimes(None, 100)
            .expect("submit real runtime query")
            .get(),
        1
    );
    assert_eq!(
        worker
            .try_list_profiles(None, 100, false)
            .expect("submit real profile query")
            .get(),
        2
    );
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit real session query")
            .get(),
        3
    );

    match next_worker_event(&mut worker) {
        ManagerWorkerEvent::RuntimesListed { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            let page = result.expect("real runtime query succeeds");
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.next_cursor, None);
            let view = &page.items[0];
            assert_eq!(view.runtime_id, registered.runtime_id);
            assert_eq!(view.target_os, registered.target_os);
            assert_eq!(view.target_arch, registered.target_arch);
            assert_eq!(view.java_vendor, registered.java_vendor);
            assert_eq!(view.java_version, registered.java_version);
            assert_eq!(view.microemulator_version, registered.microemulator_version);
            assert_eq!(view.game_bundle, registered.game_bundle);
            assert_eq!(view.capability_state, CapabilityState::NeedsValidation);
            assert_eq!(view.validation_reason, registered.validation_reason);
            let rendered = format!("{page:?}");
            assert!(!rendered.contains(&path_sentinel));
            assert!(!rendered.contains(&hash_sentinel));
        }
        other => panic!("expected RuntimesListed first, got {other:?}"),
    }
    match next_worker_event(&mut worker) {
        ManagerWorkerEvent::ProfilesListed { request_id, result } => {
            assert_eq!(request_id.get(), 2);
            let page = result.expect("real profile query succeeds");
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.next_cursor, None);
            let view = &page.items[0];
            assert_eq!(view.profile_id, profile.profile_id);
            assert_eq!(view.revision, profile.revision);
            assert_eq!(view.display_name, profile.display_name);
            assert_eq!(view.runtime_id, profile.runtime_id);
            assert!(!view.archived);
            assert_eq!(view.active_session_id, None);
            let rendered = format!("{page:?}");
            assert!(!rendered.contains(&path_sentinel));
            assert!(!rendered.contains(&hash_sentinel));
        }
        other => panic!("expected ProfilesListed second, got {other:?}"),
    }
    match next_worker_event(&mut worker) {
        ManagerWorkerEvent::SessionsListed {
            request_id,
            sessions,
        } => {
            assert_eq!(request_id.get(), 3);
            assert!(sessions.is_empty());
        }
        other => panic!("expected SessionsListed third, got {other:?}"),
    }

    drop(worker);
    let reopen_deadline = Instant::now() + Duration::from_secs(1);
    let reopened = loop {
        match CoreState::open_at(&data_root.0) {
            Ok(core) => break core,
            Err(CoreError::AlreadyRunning) if Instant::now() < reopen_deadline => {
                thread::yield_now();
            }
            Err(CoreError::AlreadyRunning) => panic!("query worker did not release the Core lock"),
            Err(error) => panic!("unexpected Core reopen failure: {error:?}"),
        }
    };
    drop(reopened);
}

#[test]
fn manager_runtime_catalog_is_paginated_and_redacted() {
    let runtime_fixture = RuntimeFixture::new("manager-runtime-catalog");
    let data_root = TestDirectory::new("runtime-catalog");
    let mut core = CoreState::open_at(&data_root.0).expect("open manager test Core");
    let registered = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register manager runtime fixture");
    let path_sentinel = registered.runtime_root.to_string_lossy().into_owned();
    let hash_sentinel = registered.descriptor_sha256.clone();

    let manager = ManagerController::from_core(core);
    let page = manager
        .list_runtimes(None, 1)
        .expect("list manager runtimes");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next_cursor, None);
    let view = &page.items[0];
    assert_eq!(view.runtime_id, registered.runtime_id);
    assert_eq!(view.target_os, registered.target_os);
    assert_eq!(view.target_arch, registered.target_arch);
    assert_eq!(view.java_vendor, registered.java_vendor);
    assert_eq!(view.java_version, registered.java_version);
    assert_eq!(view.microemulator_version, registered.microemulator_version);
    assert_eq!(view.game_bundle, registered.game_bundle);
    assert_eq!(view.capability_state, CapabilityState::NeedsValidation);
    assert_eq!(view.validation_reason, registered.validation_reason);
    assert!(manager.list_sessions().is_empty());

    let rendered = format!("{page:?}");
    assert!(!rendered.contains(&path_sentinel));
    assert!(!rendered.contains(&hash_sentinel));
}

#[test]
fn manager_profile_catalog_preserves_pages_archive_filter_and_safe_fields() {
    let runtime_fixture = RuntimeFixture::new("manager-profile-catalog");
    let data_root = TestDirectory::new("profile-catalog");
    let mut core = CoreState::open_at(&data_root.0).expect("open manager profile Core");
    let runtime = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register profile catalog runtime");
    let visible = core
        .create_profile("Visible profile", &runtime.runtime_id)
        .expect("create visible profile");
    let archived = core
        .create_profile("Archived profile", &runtime.runtime_id)
        .expect("create archived profile");
    let archived = core
        .archive_profile(&archived.profile_id, archived.revision)
        .expect("archive profile");

    let manager = ManagerController::from_core(core);
    let visible_page = manager
        .list_profiles(None, 100, false)
        .expect("list visible profiles");
    assert_eq!(visible_page.items.len(), 1);
    assert_eq!(visible_page.items[0].profile_id, visible.profile_id);
    assert!(!visible_page.items[0].archived);
    assert_eq!(visible_page.items[0].active_session_id, None);

    let first_page = manager
        .list_profiles(None, 1, true)
        .expect("list first profile page");
    assert_eq!(first_page.items.len(), 1);
    let cursor = first_page
        .next_cursor
        .as_deref()
        .expect("two profiles require a next cursor");
    let second_page = manager
        .list_profiles(Some(cursor), 1, true)
        .expect("list second profile page");
    assert_eq!(second_page.items.len(), 1);
    assert_eq!(second_page.next_cursor, None);

    let all = [first_page.items[0].clone(), second_page.items[0].clone()];
    let archived_view = all
        .iter()
        .find(|profile| profile.profile_id == archived.profile_id)
        .expect("archived profile view");
    assert_eq!(archived_view.revision, archived.revision);
    assert_eq!(archived_view.display_name, archived.display_name);
    assert_eq!(archived_view.runtime_id, archived.runtime_id);
    assert!(archived_view.archived);
    assert_eq!(archived_view.active_session_id, None);

    let rendered = format!("{all:?}");
    assert!(!rendered.contains("stagger_class"));
    assert!(!rendered.contains("presentation_json"));
}

#[test]
fn manager_open_maps_exclusive_lock_without_raw_source() {
    let data_root = TestDirectory::new("exclusive-lock");
    let first = ManagerController::open_at(&data_root.0).expect("open first manager");

    let error = match ManagerController::open_at(&data_root.0) {
        Ok(_) => panic!("a second manager must not own the same data root"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ManagerErrorCode::ControllerAlreadyOpen);
    assert_eq!(error.operation(), ManagerOperation::Open);
    assert!(error.source().is_none());

    drop(first);
    drop(CoreState::open_at(&data_root.0).expect("manager drop releases Core lock"));
}

#[test]
fn manager_page_error_preserves_only_bounded_limit_context() {
    let data_root = TestDirectory::new("page-limit");
    let manager = ManagerController::from_core(
        CoreState::open_at(&data_root.0).expect("open page-limit manager Core"),
    );

    let error = manager
        .list_runtimes(None, 0)
        .expect_err("zero page limit must fail");
    assert_eq!(error.code(), ManagerErrorCode::InvalidPageLimit);
    assert_eq!(error.operation(), ManagerOperation::ListRuntimes);
    assert_eq!(error.maximum(), Some(100));
    assert!(error.source().is_none());
}

#[test]
fn public_start_rejects_invalid_profile_before_process_work() {
    let data_root = TestDirectory::new("invalid-public-start");
    let mut manager = ManagerController::from_core(
        CoreState::open_at(&data_root.0).expect("open invalid-start manager Core"),
    );

    let error = manager
        .start_profile("not-a-canonical-profile-id", 1)
        .expect_err("invalid public profile ID must fail");

    assert_eq!(error.code(), ManagerErrorCode::InvalidProfileId);
    assert_eq!(error.operation(), ManagerOperation::StartProfile);
    assert!(manager.list_sessions().is_empty());
}

#[test]
fn public_empty_close_is_idempotent_and_terminal() {
    let data_root = TestDirectory::new("public-empty-close");
    let mut manager = ManagerController::from_core(
        CoreState::open_at(&data_root.0).expect("open public close manager Core"),
    );

    manager.close().expect("close empty public manager");
    manager.close().expect("repeat empty public close");

    assert!(manager.list_sessions().is_empty());
    for (error, operation) in [
        (
            manager
                .list_profiles(None, 0, false)
                .expect_err("closed public profile catalog"),
            ManagerOperation::ListProfiles,
        ),
        (
            manager
                .observe_session("not-a-session-id")
                .expect_err("closed public observe"),
            ManagerOperation::ObserveSession,
        ),
        (
            manager
                .stop_session("not-a-session-id")
                .expect_err("closed public stop"),
            ManagerOperation::StopSession,
        ),
        (
            manager
                .retry_cleanup("not-a-session-id")
                .expect_err("closed public cleanup retry"),
            ManagerOperation::RetryCleanup,
        ),
    ] {
        assert_eq!(error.code(), ManagerErrorCode::ControllerClosed);
        assert_eq!(error.operation(), operation);
    }
}

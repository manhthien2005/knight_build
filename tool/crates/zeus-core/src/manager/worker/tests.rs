use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::ManagerAccountPassword;
use super::engine::{WorkerCommand, WorkerControl, WorkerWake};
use super::login::{LoginCompletion, LoginFailureCode, LoginOutcome};
use super::{
    ManagerRequestId, ManagerWorker, ManagerWorkerError, ManagerWorkerErrorCode,
    ManagerWorkerOperation, ManagerWorkerState,
};
use crate::manager::{
    MAX_RUN_BATCH, ManagerAccountId, ManagerAccountStatus, ManagerAccountView, ManagerError,
    ManagerErrorCode, ManagerObservation, ManagerOperation, ManagerProfilePage, ManagerProfileView,
    ManagerResult, ManagerRunRejection, ManagerRunSchedule, ManagerRunScheduleOutcome,
    ManagerRuntimePage, ManagerRuntimeView, ManagerSessionExit, ManagerSessionState,
    ManagerSessionView, ManagerWorkerEvent,
};
use crate::runtime::CapabilityState;

const EVENT_DEADLINE: Duration = Duration::from_secs(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("zeus-manager-worker-{label}-{}", Uuid::new_v4())))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

struct CloseBarrier {
    entered_tx: SyncSender<()>,
    release_rx: Receiver<()>,
}

struct QueryBarrier {
    entered_tx: SyncSender<()>,
    release_rx: Receiver<()>,
}

#[derive(Debug, PartialEq, Eq)]
enum QueryCall {
    Profiles {
        after: Option<String>,
        limit: u32,
        include_archived: bool,
    },
    Runtimes {
        after: Option<String>,
        limit: u32,
    },
    Sessions,
}

struct FakeController {
    close_barrier: Option<CloseBarrier>,
    close_call_count: usize,
    close_call_tx: Option<SyncSender<usize>>,
    close_results: VecDeque<ManagerResult<()>>,
    panic_on_list_sessions: bool,
    query_barrier: RefCell<Option<QueryBarrier>>,
    query_call_tx: Option<SyncSender<QueryCall>>,
    session_call_count: Cell<usize>,
    session_call_tx: Option<SyncSender<usize>>,
    profile_page: ManagerProfilePage,
    runtime_page: ManagerRuntimePage,
    sessions: Vec<ManagerSessionView>,
}

impl Default for FakeController {
    fn default() -> Self {
        Self {
            close_barrier: None,
            close_call_count: 0,
            close_call_tx: None,
            close_results: VecDeque::new(),
            panic_on_list_sessions: false,
            query_barrier: RefCell::new(None),
            query_call_tx: None,
            session_call_count: Cell::new(0),
            session_call_tx: None,
            profile_page: ManagerProfilePage {
                items: Vec::new(),
                next_cursor: None,
            },
            runtime_page: ManagerRuntimePage {
                items: Vec::new(),
                next_cursor: None,
            },
            sessions: Vec::new(),
        }
    }
}

impl FakeController {
    fn record_thread_name(thread_name_tx: SyncSender<String>) -> Self {
        thread_name_tx
            .send(
                thread::current()
                    .name()
                    .expect("worker thread has a name")
                    .to_owned(),
            )
            .expect("thread-name receiver remains connected");
        Self::default()
    }

    fn with_close_barrier(entered_tx: SyncSender<()>, release_rx: Receiver<()>) -> Self {
        Self {
            close_barrier: Some(CloseBarrier {
                entered_tx,
                release_rx,
            }),
            ..Self::default()
        }
    }

    fn with_close_results(
        close_call_tx: SyncSender<usize>,
        close_results: VecDeque<ManagerResult<()>>,
    ) -> Self {
        Self {
            close_call_tx: Some(close_call_tx),
            close_results,
            ..Self::default()
        }
    }

    fn panics_during_list_sessions() -> Self {
        Self {
            panic_on_list_sessions: true,
            ..Self::default()
        }
    }

    fn with_query_fixture(query_call_tx: SyncSender<QueryCall>) -> Self {
        Self {
            query_call_tx: Some(query_call_tx),
            profile_page: ManagerProfilePage {
                items: vec![
                    ManagerProfileView {
                        profile_id: "00000000-0000-4000-8000-000000000011".to_owned(),
                        revision: 4,
                        display_name: "Alpha profile".to_owned(),
                        runtime_id: "runtime-alpha".to_owned(),
                        archived: false,
                        active_session_id: Some("00000000-0000-4000-8000-000000000031".to_owned()),
                    },
                    ManagerProfileView {
                        profile_id: "00000000-0000-4000-8000-000000000012".to_owned(),
                        revision: 7,
                        display_name: "Archived profile".to_owned(),
                        runtime_id: "runtime-alpha".to_owned(),
                        archived: true,
                        active_session_id: None,
                    },
                ],
                next_cursor: Some("00000000-0000-4000-8000-000000000012".to_owned()),
            },
            runtime_page: ManagerRuntimePage {
                items: vec![ManagerRuntimeView {
                    runtime_id: "windows-x64_fixture-java11_microemu204_ko402".to_owned(),
                    target_os: "windows".to_owned(),
                    target_arch: "x64".to_owned(),
                    java_vendor: "Fixture Vendor".to_owned(),
                    java_version: "11.0.32+9".to_owned(),
                    microemulator_version: "2.0.4".to_owned(),
                    game_bundle: "402".to_owned(),
                    capability_state: CapabilityState::NeedsValidation,
                    validation_reason: "fixture_static_only".to_owned(),
                }],
                next_cursor: None,
            },
            sessions: vec![ManagerSessionView {
                session_id: "00000000-0000-4000-8000-000000000031".to_owned(),
                profile_id: "00000000-0000-4000-8000-000000000011".to_owned(),
                profile_revision: 4,
                runtime_id: "runtime-alpha".to_owned(),
                state: ManagerSessionState::Running,
            }],
            ..Self::default()
        }
    }

    fn recording_queries(query_call_tx: SyncSender<QueryCall>) -> Self {
        Self {
            query_call_tx: Some(query_call_tx),
            ..Self::default()
        }
    }

    fn with_query_barrier(entered_tx: SyncSender<()>, release_rx: Receiver<()>) -> Self {
        Self {
            query_barrier: RefCell::new(Some(QueryBarrier {
                entered_tx,
                release_rx,
            })),
            ..Self::default()
        }
    }

    fn with_backpressure_signals(
        session_call_tx: SyncSender<usize>,
        close_entered_tx: SyncSender<()>,
        close_release_rx: Receiver<()>,
        close_call_tx: SyncSender<usize>,
    ) -> Self {
        Self {
            close_barrier: Some(CloseBarrier {
                entered_tx: close_entered_tx,
                release_rx: close_release_rx,
            }),
            close_call_tx: Some(close_call_tx),
            session_call_tx: Some(session_call_tx),
            ..Self::default()
        }
    }
}

impl WorkerControl for FakeController {
    fn list_profiles(
        &self,
        after: Option<&str>,
        limit: u32,
        archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        if let Some(query_call_tx) = &self.query_call_tx {
            query_call_tx
                .send(QueryCall::Profiles {
                    after: after.map(str::to_owned),
                    limit,
                    include_archived: archived,
                })
                .expect("query-call receiver remains connected");
        }
        Ok(self.profile_page.clone())
    }

    fn list_runtimes(&self, after: Option<&str>, limit: u32) -> ManagerResult<ManagerRuntimePage> {
        if let Some(query_call_tx) = &self.query_call_tx {
            query_call_tx
                .send(QueryCall::Runtimes {
                    after: after.map(str::to_owned),
                    limit,
                })
                .expect("query-call receiver remains connected");
        }
        Ok(self.runtime_page.clone())
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        if self.panic_on_list_sessions {
            panic!("worker-panic-secret-sentinel-7821");
        }
        if let Some(query_call_tx) = &self.query_call_tx {
            query_call_tx
                .send(QueryCall::Sessions)
                .expect("query-call receiver remains connected");
        }
        if let Some(session_call_tx) = &self.session_call_tx {
            let call = self.session_call_count.get() + 1;
            self.session_call_count.set(call);
            session_call_tx
                .send(call)
                .expect("session-call receiver remains connected");
        }
        if let Some(barrier) = self.query_barrier.borrow_mut().take() {
            barrier
                .entered_tx
                .send(())
                .expect("query-entered receiver remains connected");
            barrier
                .release_rx
                .recv()
                .expect("query release sender remains connected");
        }
        self.sessions.clone()
    }

    fn start_profile(&mut self, profile: &str, revision: i64) -> ManagerResult<ManagerSessionView> {
        Ok(ManagerSessionView {
            session_id: "00000000-0000-4000-8000-000000000001".to_owned(),
            profile_id: profile.to_owned(),
            profile_revision: revision,
            runtime_id: "test-runtime".to_owned(),
            state: ManagerSessionState::Running,
        })
    }

    fn observe_session(&mut self, session: &str) -> ManagerResult<ManagerObservation> {
        Ok(ManagerObservation::Running(ManagerSessionView {
            session_id: session.to_owned(),
            profile_id: "00000000-0000-4000-8000-000000000002".to_owned(),
            profile_revision: 1,
            runtime_id: "test-runtime".to_owned(),
            state: ManagerSessionState::Running,
        }))
    }

    fn stop_session(&mut self, session: &str) -> ManagerResult<ManagerSessionExit> {
        Ok(ManagerSessionExit {
            session_id: session.to_owned(),
            profile_id: "00000000-0000-4000-8000-000000000002".to_owned(),
            profile_revision: 1,
            runtime_id: "test-runtime".to_owned(),
        })
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        Ok(())
    }

    fn close(&mut self) -> ManagerResult<()> {
        self.close_call_count += 1;
        if let Some(close_call_tx) = &self.close_call_tx {
            close_call_tx
                .send(self.close_call_count)
                .expect("close-call receiver remains connected");
        }
        if let Some(barrier) = self.close_barrier.take() {
            barrier
                .entered_tx
                .send(())
                .expect("close-entered receiver remains connected");
            barrier
                .release_rx
                .recv()
                .expect("close release sender remains connected");
        }
        self.close_results.pop_front().unwrap_or(Ok(()))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LifecycleCall {
    Start {
        profile_id: String,
        expected_revision: i64,
    },
    Observe {
        session_id: String,
    },
    Stop {
        session_id: String,
    },
    RetryCleanup {
        session_id: String,
    },
}

struct LifecycleController {
    call_tx: SyncSender<LifecycleCall>,
    start_result: Option<ManagerResult<ManagerSessionView>>,
    observe_results: VecDeque<ManagerResult<ManagerObservation>>,
    stop_result: Option<ManagerResult<ManagerSessionExit>>,
    retry_cleanup_results: VecDeque<ManagerResult<()>>,
}

impl LifecycleController {
    fn new(call_tx: SyncSender<LifecycleCall>) -> Self {
        Self {
            call_tx,
            start_result: None,
            observe_results: VecDeque::new(),
            stop_result: None,
            retry_cleanup_results: VecDeque::new(),
        }
    }

    fn record(&self, call: LifecycleCall) {
        self.call_tx
            .send(call)
            .expect("lifecycle-call receiver remains connected");
    }
}

impl WorkerControl for LifecycleController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        Ok(ManagerProfilePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        Ok(ManagerRuntimePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        Vec::new()
    }

    fn start_profile(&mut self, profile: &str, revision: i64) -> ManagerResult<ManagerSessionView> {
        self.record(LifecycleCall::Start {
            profile_id: profile.to_owned(),
            expected_revision: revision,
        });
        self.start_result
            .take()
            .expect("start result is scripted exactly once")
    }

    fn observe_session(&mut self, session: &str) -> ManagerResult<ManagerObservation> {
        self.record(LifecycleCall::Observe {
            session_id: session.to_owned(),
        });
        self.observe_results
            .pop_front()
            .expect("observation result is scripted")
    }

    fn stop_session(&mut self, session: &str) -> ManagerResult<ManagerSessionExit> {
        self.record(LifecycleCall::Stop {
            session_id: session.to_owned(),
        });
        self.stop_result
            .take()
            .expect("stop result is scripted exactly once")
    }

    fn retry_cleanup(&mut self, session: &str) -> ManagerResult<()> {
        self.record(LifecycleCall::RetryCleanup {
            session_id: session.to_owned(),
        });
        self.retry_cleanup_results
            .pop_front()
            .expect("cleanup result is scripted")
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

struct SerialCallProbe {
    thread_ids: Arc<Mutex<Vec<thread::ThreadId>>>,
    in_call: Arc<AtomicUsize>,
    maximum_in_call: Arc<AtomicUsize>,
    first_entered_tx: Option<SyncSender<()>>,
    first_release_rx: Option<Receiver<()>>,
}

impl SerialCallProbe {
    fn enter(&mut self) {
        let in_call = self.in_call.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_in_call.fetch_max(in_call, Ordering::SeqCst);
        self.thread_ids
            .lock()
            .expect("thread-ID recorder is not poisoned")
            .push(thread::current().id());

        if let Some(entered_tx) = self.first_entered_tx.take() {
            entered_tx
                .send(())
                .expect("first-call receiver remains connected");
            self.first_release_rx
                .take()
                .expect("first-call release is configured")
                .recv()
                .expect("first-call release sender remains connected");
        }

        self.in_call.fetch_sub(1, Ordering::SeqCst);
    }
}

struct SerialLifecycleController {
    probe: SerialCallProbe,
}

impl WorkerControl for SerialLifecycleController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        unreachable!("serial lifecycle test submits no catalog calls")
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        unreachable!("serial lifecycle test submits no catalog calls")
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        unreachable!("serial lifecycle test submits no catalog calls")
    }

    fn start_profile(&mut self, profile: &str, revision: i64) -> ManagerResult<ManagerSessionView> {
        self.probe.enter();
        Ok(ManagerSessionView {
            session_id: "serial-session".to_owned(),
            profile_id: profile.to_owned(),
            profile_revision: revision,
            runtime_id: "serial-runtime".to_owned(),
            state: ManagerSessionState::Running,
        })
    }

    fn observe_session(&mut self, session: &str) -> ManagerResult<ManagerObservation> {
        self.probe.enter();
        Ok(ManagerObservation::Running(ManagerSessionView {
            session_id: session.to_owned(),
            profile_id: "serial-profile".to_owned(),
            profile_revision: 5,
            runtime_id: "serial-runtime".to_owned(),
            state: ManagerSessionState::Running,
        }))
    }

    fn stop_session(&mut self, session: &str) -> ManagerResult<ManagerSessionExit> {
        self.probe.enter();
        Ok(ManagerSessionExit {
            session_id: session.to_owned(),
            profile_id: "serial-profile".to_owned(),
            profile_revision: 5,
            runtime_id: "serial-runtime".to_owned(),
        })
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        self.probe.enter();
        Ok(())
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

fn next_event(worker: &mut ManagerWorker) -> ManagerWorkerEvent {
    let deadline = Instant::now() + EVENT_DEADLINE;
    loop {
        match worker.try_next_event() {
            Ok(Some(event)) => return event,
            Ok(None) if Instant::now() < deadline => thread::yield_now(),
            Ok(None) => panic!("worker event deadline expired"),
            Err(error) => panic!("worker event polling failed: {error:?}"),
        }
    }
}

fn consume_ready(worker: &mut ManagerWorker) {
    assert!(matches!(next_event(worker), ManagerWorkerEvent::Ready));
    assert_eq!(worker.state(), ManagerWorkerState::Ready);
}

fn assert_worker_error<T>(
    result: Result<T, ManagerWorkerError>,
    code: ManagerWorkerErrorCode,
    operation: ManagerWorkerOperation,
) {
    let error = result.err().expect("worker operation must be rejected");
    assert_eq!(error.code(), code);
    assert_eq!(error.operation(), operation);
    assert_eq!(error.maximum(), None);
}

#[test]
fn request_id_preserves_its_nonzero_value() {
    let request_id = ManagerRequestId::new(NonZeroU64::new(7).expect("literal is nonzero"));

    assert_eq!(request_id.get(), 7);
}

#[test]
fn worker_error_renders_only_stable_bounded_context() {
    let error = ManagerWorkerError::new(
        ManagerWorkerErrorCode::InputTooLong,
        ManagerWorkerOperation::ListRuntimes,
    )
    .with_maximum(160);

    assert_eq!(error.code(), ManagerWorkerErrorCode::InputTooLong);
    assert_eq!(error.operation(), ManagerWorkerOperation::ListRuntimes);
    assert_eq!(error.maximum(), Some(160));

    let display = error.to_string();
    let debug = format!("{error:?}");
    assert_eq!(
        display,
        "code=input_too_long operation=list_runtimes maximum=160"
    );
    assert_eq!(
        debug,
        "ManagerWorkerError { code=input_too_long operation=list_runtimes maximum=160 }"
    );
    for sentinel in [
        r"C:\Games\KnightOnline\runtime",
        "os_code=5",
        "submitted-runtime-cursor",
    ] {
        assert!(!display.contains(sentinel));
        assert!(!debug.contains(sentinel));
    }
    assert!(error.source().is_none());
}

#[test]
fn worker_rejects_commands_until_ready_is_consumed() {
    let (boot_entered_tx, boot_entered_rx) = sync_channel(0);
    let (boot_release_tx, boot_release_rx) = sync_channel(0);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        boot_entered_tx
            .send(())
            .expect("boot-entered receiver remains connected");
        boot_release_rx
            .recv()
            .expect("boot release sender remains connected");
        Ok(FakeController::default())
    })
    .expect("spawn barrier-controlled worker");

    boot_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("worker reaches boot barrier");
    assert_eq!(worker.state(), ManagerWorkerState::Starting);
    assert!(
        worker
            .try_next_event()
            .expect("empty event polling remains available while starting")
            .is_none()
    );
    for (result, operation) in [
        (
            worker.try_list_profiles(None, 1, false),
            ManagerWorkerOperation::ListProfiles,
        ),
        (
            worker.try_list_runtimes(None, 1),
            ManagerWorkerOperation::ListRuntimes,
        ),
        (
            worker.try_list_sessions(),
            ManagerWorkerOperation::ListSessions,
        ),
        (
            worker.try_start_profile("starting-profile", 1),
            ManagerWorkerOperation::StartProfile,
        ),
        (
            worker.try_observe_session("starting-session"),
            ManagerWorkerOperation::ObserveSession,
        ),
        (
            worker.try_stop_session("starting-session"),
            ManagerWorkerOperation::StopSession,
        ),
        (
            worker.try_retry_cleanup("starting-session"),
            ManagerWorkerOperation::RetryCleanup,
        ),
        (worker.try_shutdown(), ManagerWorkerOperation::Shutdown),
    ] {
        assert_worker_error(result, ManagerWorkerErrorCode::NotReady, operation);
    }

    boot_release_tx
        .send(())
        .expect("boot receiver remains connected");
    assert_eq!(worker.state(), ManagerWorkerState::Starting);
    let before_consuming_ready = worker
        .try_test_probe()
        .expect_err("Ready must be consumed before command admission");
    assert_eq!(
        before_consuming_ready.code(),
        ManagerWorkerErrorCode::NotReady
    );

    consume_ready(&mut worker);
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("starting rejections consume no request ID")
            .get(),
        1
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id.get() == 1
    ));
}

#[test]
fn worker_open_failure_is_one_redacted_startup_event() {
    let parent = TestDirectory::new("open-failure");
    drop(crate::CoreState::open_at(&parent.0).expect("create private UUID test directory"));
    let sentinel_path = parent.0.join("unmanaged-path-sentinel.txt");
    fs::write(&sentinel_path, b"not a data root").expect("create unmanaged file path");
    let sentinel = sentinel_path.to_string_lossy().into_owned();

    let mut worker = ManagerWorker::spawn_at(&sentinel_path).expect("thread creation succeeds");
    assert_eq!(worker.state(), ManagerWorkerState::Starting);

    let error = match next_event(&mut worker) {
        ManagerWorkerEvent::OpenFailed(error) => error,
        other => panic!("expected OpenFailed first, got {other:?}"),
    };
    assert_eq!(error.code(), ManagerErrorCode::InvalidDataRoot);
    assert_eq!(error.operation(), ManagerOperation::Open);
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
    assert!(!error.to_string().contains(&sentinel));
    assert!(!format!("{error:?}").contains(&sentinel));

    let later = worker
        .try_next_event()
        .expect_err("closed worker has no second startup event");
    assert_eq!(later.code(), ManagerWorkerErrorCode::Closed);
}

#[test]
fn worker_controller_runs_on_the_named_worker_thread() {
    let (boot_entered_tx, boot_entered_rx) = sync_channel(0);
    let (boot_release_tx, boot_release_rx) = sync_channel(0);
    let (thread_name_tx, thread_name_rx) = sync_channel(0);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        boot_entered_tx
            .send(())
            .expect("boot-entered receiver remains connected");
        boot_release_rx
            .recv()
            .expect("boot release sender remains connected");
        Ok(FakeController::record_thread_name(thread_name_tx))
    })
    .expect("spawn named worker");

    boot_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("worker reaches boot barrier");
    boot_release_tx
        .send(())
        .expect("boot receiver remains connected");
    assert_eq!(
        thread_name_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("fake records owner thread name"),
        "zeus-manager-worker-v1"
    );
    consume_ready(&mut worker);
}

#[test]
fn backpressure_command_disconnect_cleanup_is_background_once_and_drop_does_not_join() {
    let (close_entered_tx, close_entered_rx) = sync_channel(0);
    let (close_release_tx, close_release_rx) = sync_channel(0);
    let (close_call_tx, close_call_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = FakeController::with_close_barrier(close_entered_tx, close_release_rx);
        controller.close_call_tx = Some(close_call_tx);
        Ok(controller)
    })
    .expect("spawn drop-controlled worker");
    consume_ready(&mut worker);

    let (dropper_returned_tx, dropper_returned_rx) = sync_channel(0);
    let dropper = thread::spawn(move || {
        drop(worker);
        dropper_returned_tx
            .send(())
            .expect("dropper receiver remains connected");
    });

    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("command disconnect enters background cleanup"),
        1
    );
    close_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("background close reaches its held barrier");
    dropper_returned_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("dropping the handle must not join the worker");
    close_release_tx
        .send(())
        .expect("worker remains in disconnect close");

    match close_call_rx.recv_timeout(EVENT_DEADLINE) {
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        Ok(call) => panic!("command disconnect called close more than once: call {call}"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("worker did not return and drop the controller after command disconnect")
        }
    }
    dropper.join().expect("dropper thread exits cleanly");
}

#[test]
fn backpressure_command_capacity_is_sixteen_and_full_preserves_the_next_id() {
    let (query_entered_tx, query_entered_rx) = sync_channel(0);
    let (query_release_tx, query_release_rx) = sync_channel(0);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_query_barrier(
            query_entered_tx,
            query_release_rx,
        ))
    })
    .expect("spawn command-backpressure worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit the barrier-controlled call")
            .get(),
        1
    );
    query_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("call 1 enters the controller barrier");

    for expected_id in 2..=17 {
        assert_eq!(
            worker
                .try_list_sessions()
                .expect("exactly sixteen commands fit behind the active call")
                .get(),
            expected_id
        );
    }
    let full = worker
        .try_list_sessions()
        .expect_err("the seventeenth waiting command is rejected immediately");
    assert_eq!(full.code(), ManagerWorkerErrorCode::CommandQueueFull);
    assert_eq!(full.operation(), ManagerWorkerOperation::ListSessions);
    assert_eq!(full.maximum(), Some(16));

    query_release_tx
        .send(())
        .expect("release the barrier-controlled call");
    for expected_id in 1..=17 {
        assert!(matches!(
            next_event(&mut worker),
            ManagerWorkerEvent::SessionsListed { request_id, sessions }
                if request_id.get() == expected_id && sessions.is_empty()
        ));
    }

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("the rejected submission leaves ID 18 available")
            .get(),
        18
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 18 && sessions.is_empty()
    ));
}

#[test]
fn backpressure_event_capacity_is_thirty_two_and_disconnect_cleanup_is_background_once() {
    let (session_call_tx, session_call_rx) = sync_channel(0);
    let (close_entered_tx, close_entered_rx) = sync_channel(0);
    let (close_release_tx, close_release_rx) = sync_channel(0);
    let (close_call_tx, close_call_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_backpressure_signals(
            session_call_tx,
            close_entered_tx,
            close_release_rx,
            close_call_tx,
        ))
    })
    .expect("spawn event-backpressure worker");
    consume_ready(&mut worker);

    for expected_call in 1..=33 {
        assert_eq!(
            worker
                .try_list_sessions()
                .expect("drive one controller call at a time without polling events")
                .get(),
            expected_call
        );
        assert_eq!(
            session_call_rx
                .recv_timeout(EVENT_DEADLINE)
                .expect("the submitted command reaches the controller"),
            expected_call as usize
        );
    }

    for expected_id in 34..=49 {
        assert_eq!(
            worker
                .try_list_sessions()
                .expect("sixteen commands fit behind blocked event delivery")
                .get(),
            expected_id
        );
    }
    let full = worker
        .try_list_sessions()
        .expect_err("the command queue is full behind event 33");
    assert_eq!(full.code(), ManagerWorkerErrorCode::CommandQueueFull);
    assert_eq!(full.operation(), ManagerWorkerOperation::ListSessions);
    assert_eq!(full.maximum(), Some(16));

    let (dropper_returned_tx, dropper_returned_rx) = sync_channel(0);
    let dropper = thread::spawn(move || {
        drop(worker);
        dropper_returned_tx
            .send(())
            .expect("dropper receiver remains connected");
    });

    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("event disconnect enters background cleanup"),
        1
    );
    close_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("background close reaches its held barrier");
    dropper_returned_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("dropping the UI handle returns before close is released");
    close_release_tx
        .send(())
        .expect("release the background close");

    match close_call_rx.recv_timeout(EVENT_DEADLINE) {
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        Ok(call) => panic!("disconnect cleanup called close more than once: call {call}"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("worker did not return and drop the controller after disconnect cleanup")
        }
    }
    dropper.join().expect("dropper thread exits cleanly");
}

#[test]
fn query_results_map_to_fifo_events_with_gapless_ids() {
    let (query_call_tx, query_call_rx) = sync_channel(3);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_query_fixture(query_call_tx))
    })
    .expect("spawn query worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_profiles(Some("profile-cursor"), 2, true)
            .expect("submit profile query")
            .get(),
        1
    );
    assert_eq!(
        worker
            .try_list_runtimes(Some("runtime-cursor"), 1)
            .expect("submit runtime query")
            .get(),
        2
    );
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit session query")
            .get(),
        3
    );

    match next_event(&mut worker) {
        ManagerWorkerEvent::ProfilesListed { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            let page = result.expect("fake profile page is successful");
            assert_eq!(page.items.len(), 2);
            assert_eq!(
                page.items[0],
                ManagerProfileView {
                    profile_id: "00000000-0000-4000-8000-000000000011".to_owned(),
                    revision: 4,
                    display_name: "Alpha profile".to_owned(),
                    runtime_id: "runtime-alpha".to_owned(),
                    archived: false,
                    active_session_id: Some("00000000-0000-4000-8000-000000000031".to_owned()),
                }
            );
            assert_eq!(
                page.items[1],
                ManagerProfileView {
                    profile_id: "00000000-0000-4000-8000-000000000012".to_owned(),
                    revision: 7,
                    display_name: "Archived profile".to_owned(),
                    runtime_id: "runtime-alpha".to_owned(),
                    archived: true,
                    active_session_id: None,
                }
            );
            assert_eq!(
                page.next_cursor.as_deref(),
                Some("00000000-0000-4000-8000-000000000012")
            );
        }
        other => panic!("expected ProfilesListed first, got {other:?}"),
    }
    match next_event(&mut worker) {
        ManagerWorkerEvent::RuntimesListed { request_id, result } => {
            assert_eq!(request_id.get(), 2);
            let page = result.expect("fake runtime page is successful");
            assert_eq!(page.items.len(), 1);
            let runtime = &page.items[0];
            assert_eq!(
                runtime.runtime_id,
                "windows-x64_fixture-java11_microemu204_ko402"
            );
            assert_eq!(runtime.target_os, "windows");
            assert_eq!(runtime.target_arch, "x64");
            assert_eq!(runtime.java_vendor, "Fixture Vendor");
            assert_eq!(runtime.java_version, "11.0.32+9");
            assert_eq!(runtime.microemulator_version, "2.0.4");
            assert_eq!(runtime.game_bundle, "402");
            assert_eq!(runtime.capability_state, CapabilityState::NeedsValidation);
            assert_eq!(runtime.validation_reason, "fixture_static_only");
            assert_eq!(page.next_cursor, None);
        }
        other => panic!("expected RuntimesListed second, got {other:?}"),
    }
    match next_event(&mut worker) {
        ManagerWorkerEvent::SessionsListed {
            request_id,
            sessions,
        } => {
            assert_eq!(request_id.get(), 3);
            assert_eq!(
                sessions,
                vec![ManagerSessionView {
                    session_id: "00000000-0000-4000-8000-000000000031".to_owned(),
                    profile_id: "00000000-0000-4000-8000-000000000011".to_owned(),
                    profile_revision: 4,
                    runtime_id: "runtime-alpha".to_owned(),
                    state: ManagerSessionState::Running,
                }]
            );
        }
        other => panic!("expected SessionsListed third, got {other:?}"),
    }

    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("profile call reaches fake"),
        QueryCall::Profiles {
            after: Some("profile-cursor".to_owned()),
            limit: 2,
            include_archived: true,
        }
    );
    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("runtime call reaches fake"),
        QueryCall::Runtimes {
            after: Some("runtime-cursor".to_owned()),
            limit: 1,
        }
    );
    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("session call reaches fake"),
        QueryCall::Sessions
    );
}

#[test]
fn query_oversize_cursors_are_rejected_before_enqueue_without_consuming_id() {
    let (query_call_tx, query_call_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::recording_queries(query_call_tx))
    })
    .expect("spawn query-cap worker");
    consume_ready(&mut worker);

    let profile_cursor = "p".repeat(37);
    let profile_error = worker
        .try_list_profiles(Some(&profile_cursor), 1, false)
        .expect_err("37-byte profile cursor is rejected");
    assert_eq!(profile_error.code(), ManagerWorkerErrorCode::InputTooLong);
    assert_eq!(
        profile_error.operation(),
        ManagerWorkerOperation::ListProfiles
    );
    assert_eq!(profile_error.maximum(), Some(36));
    assert!(!profile_error.to_string().contains(&profile_cursor));

    let runtime_cursor = "r".repeat(161);
    let runtime_error = worker
        .try_list_runtimes(Some(&runtime_cursor), 1)
        .expect_err("161-byte runtime cursor is rejected");
    assert_eq!(runtime_error.code(), ManagerWorkerErrorCode::InputTooLong);
    assert_eq!(
        runtime_error.operation(),
        ManagerWorkerOperation::ListRuntimes
    );
    assert_eq!(runtime_error.maximum(), Some(160));
    assert!(!runtime_error.to_string().contains(&runtime_cursor));

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("rejections leave ID 1 available")
            .get(),
        1
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 1 && sessions.is_empty()
    ));
    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("only the accepted query reaches fake"),
        QueryCall::Sessions
    );
    assert!(query_call_rx.try_recv().is_err());
}

#[test]
fn query_short_malformed_cursors_reach_controller_unchanged() {
    let (query_call_tx, query_call_rx) = sync_channel(2);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::recording_queries(query_call_tx))
    })
    .expect("spawn malformed-cursor worker");
    consume_ready(&mut worker);

    worker
        .try_list_profiles(Some("not-a-profile-cursor"), 9, false)
        .expect("short malformed profile cursor is admitted");
    worker
        .try_list_runtimes(Some("!runtime cursor!"), 8)
        .expect("short malformed runtime cursor is admitted");
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ProfilesListed { request_id, .. } if request_id.get() == 1
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::RuntimesListed { request_id, .. } if request_id.get() == 2
    ));

    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("profile cursor reaches fake"),
        QueryCall::Profiles {
            after: Some("not-a-profile-cursor".to_owned()),
            limit: 9,
            include_archived: false,
        }
    );
    assert_eq!(
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("runtime cursor reaches fake"),
        QueryCall::Runtimes {
            after: Some("!runtime cursor!".to_owned()),
            limit: 8,
        }
    );
}

#[test]
fn query_request_id_accepts_u64_max_once_then_exhausts() {
    let mut worker = ManagerWorker::spawn_with_boot(|| Ok(FakeController::default()))
        .expect("spawn request-exhaustion worker");
    consume_ready(&mut worker);
    worker.next_request_id = NonZeroU64::new(u64::MAX);

    let final_id = worker
        .try_list_sessions()
        .expect("u64::MAX remains a valid final request ID");
    assert_eq!(final_id.get(), u64::MAX);
    let exhausted = worker
        .try_list_sessions()
        .expect_err("request IDs are exhausted after accepting u64::MAX");
    assert_eq!(exhausted.code(), ManagerWorkerErrorCode::RequestIdExhausted);
    assert_eq!(exhausted.operation(), ManagerWorkerOperation::ListSessions);
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == u64::MAX && sessions.is_empty()
    ));
}

#[test]
fn query_queue_full_at_u64_max_preserves_final_id() {
    let (query_entered_tx, query_entered_rx) = sync_channel(0);
    let (query_release_tx, query_release_rx) = sync_channel(0);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_query_barrier(
            query_entered_tx,
            query_release_rx,
        ))
    })
    .expect("spawn queue-controlled worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit barrier query")
            .get(),
        1
    );
    query_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("worker is blocked inside the first query");
    for _ in 0..16 {
        assert!(worker.command_tx.try_send(WorkerCommand::Probe).is_ok());
    }
    worker.next_request_id = NonZeroU64::new(u64::MAX);

    let full = worker
        .try_list_sessions()
        .expect_err("the bounded command queue is full");
    assert_eq!(full.code(), ManagerWorkerErrorCode::CommandQueueFull);
    assert_eq!(full.operation(), ManagerWorkerOperation::ListSessions);
    assert_eq!(full.maximum(), Some(16));

    query_release_tx
        .send(())
        .expect("release the barrier-controlled query");
    let admission_deadline = Instant::now() + EVENT_DEADLINE;
    let final_id = loop {
        match worker.try_list_sessions() {
            Ok(request_id) => break request_id,
            Err(error)
                if error.code() == ManagerWorkerErrorCode::CommandQueueFull
                    && Instant::now() < admission_deadline =>
            {
                thread::yield_now();
            }
            Err(error) => panic!("final request ID was not retained: {error:?}"),
        }
    };
    assert_eq!(final_id.get(), u64::MAX);
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id.get() == 1
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id.get() == u64::MAX
    ));
}

#[test]
fn lifecycle_start_maps_one_result_and_forwards_revision() {
    let started = ManagerSessionView {
        session_id: "00000000-0000-4000-8000-000000000141".to_owned(),
        profile_id: "00000000-0000-4000-8000-000000000101".to_owned(),
        profile_revision: -27,
        runtime_id: "lifecycle-runtime-start".to_owned(),
        state: ManagerSessionState::Running,
    };
    let (call_tx, call_rx) = sync_channel(1);
    let expected_started = started.clone();
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = LifecycleController::new(call_tx);
        controller.start_result = Some(Ok(started));
        Ok(controller)
    })
    .expect("spawn start lifecycle worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_start_profile("00000000-0000-4000-8000-000000000101", -27)
            .expect("submit start command")
            .get(),
        1
    );
    match next_event(&mut worker) {
        ManagerWorkerEvent::ProfileStarted { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            assert_eq!(result.expect("scripted start succeeds"), expected_started);
        }
        other => panic!("expected one ProfileStarted event, got {other:?}"),
    }
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit marker after start result")
            .get(),
        2
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 2 && sessions.is_empty()
    ));
    assert_eq!(
        call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("start call reaches controller"),
        LifecycleCall::Start {
            profile_id: "00000000-0000-4000-8000-000000000101".to_owned(),
            expected_revision: -27,
        }
    );
    assert!(call_rx.try_recv().is_err());
}

#[test]
fn lifecycle_observe_maps_running_cleanup_pending_and_exited() {
    let running = ManagerSessionView {
        session_id: "00000000-0000-4000-8000-000000000151".to_owned(),
        profile_id: "00000000-0000-4000-8000-000000000111".to_owned(),
        profile_revision: 31,
        runtime_id: "lifecycle-runtime-observe".to_owned(),
        state: ManagerSessionState::Running,
    };
    let cleanup_pending = ManagerSessionView {
        session_id: "00000000-0000-4000-8000-000000000152".to_owned(),
        profile_id: "00000000-0000-4000-8000-000000000112".to_owned(),
        profile_revision: 32,
        runtime_id: "lifecycle-runtime-cleanup".to_owned(),
        state: ManagerSessionState::CleanupPending,
    };
    let exited = ManagerSessionExit {
        session_id: "00000000-0000-4000-8000-000000000153".to_owned(),
        profile_id: "00000000-0000-4000-8000-000000000113".to_owned(),
        profile_revision: 33,
        runtime_id: "lifecycle-runtime-exited".to_owned(),
    };
    let expected = [
        ManagerObservation::Running(running.clone()),
        ManagerObservation::CleanupPending(cleanup_pending.clone()),
        ManagerObservation::Exited(exited.clone()),
    ];
    let (call_tx, call_rx) = sync_channel(3);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = LifecycleController::new(call_tx);
        controller.observe_results = VecDeque::from([
            Ok(ManagerObservation::Running(running)),
            Ok(ManagerObservation::CleanupPending(cleanup_pending)),
            Ok(ManagerObservation::Exited(exited)),
        ]);
        Ok(controller)
    })
    .expect("spawn observe lifecycle worker");
    consume_ready(&mut worker);

    for (index, session_id) in [
        "00000000-0000-4000-8000-000000000151",
        "00000000-0000-4000-8000-000000000152",
        "00000000-0000-4000-8000-000000000153",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            worker
                .try_observe_session(session_id)
                .expect("submit observation command")
                .get(),
            index as u64 + 1
        );
    }

    for (index, expected_observation) in expected.into_iter().enumerate() {
        match next_event(&mut worker) {
            ManagerWorkerEvent::SessionObserved { request_id, result } => {
                assert_eq!(request_id.get(), index as u64 + 1);
                assert_eq!(
                    result.expect("scripted observation succeeds"),
                    expected_observation
                );
            }
            other => panic!("expected SessionObserved event, got {other:?}"),
        }
    }
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit marker after observation results")
            .get(),
        4
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 4 && sessions.is_empty()
    ));
    for session_id in [
        "00000000-0000-4000-8000-000000000151",
        "00000000-0000-4000-8000-000000000152",
        "00000000-0000-4000-8000-000000000153",
    ] {
        assert_eq!(
            call_rx
                .recv_timeout(EVENT_DEADLINE)
                .expect("observe call reaches controller"),
            LifecycleCall::Observe {
                session_id: session_id.to_owned(),
            }
        );
    }
    assert!(call_rx.try_recv().is_err());
}

#[test]
fn lifecycle_stop_maps_confirmed_exit() {
    let exited = ManagerSessionExit {
        session_id: "00000000-0000-4000-8000-000000000161".to_owned(),
        profile_id: "00000000-0000-4000-8000-000000000121".to_owned(),
        profile_revision: 41,
        runtime_id: "lifecycle-runtime-stop".to_owned(),
    };
    let expected_exited = exited.clone();
    let (call_tx, call_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = LifecycleController::new(call_tx);
        controller.stop_result = Some(Ok(exited));
        Ok(controller)
    })
    .expect("spawn stop lifecycle worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_stop_session("00000000-0000-4000-8000-000000000161")
            .expect("submit stop command")
            .get(),
        1
    );
    match next_event(&mut worker) {
        ManagerWorkerEvent::SessionStopped { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            assert_eq!(result.expect("scripted stop succeeds"), expected_exited);
        }
        other => panic!("expected one SessionStopped event, got {other:?}"),
    }
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit marker after stop result")
            .get(),
        2
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 2 && sessions.is_empty()
    ));
    assert_eq!(
        call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("stop call reaches controller"),
        LifecycleCall::Stop {
            session_id: "00000000-0000-4000-8000-000000000161".to_owned(),
        }
    );
    assert!(call_rx.try_recv().is_err());
}

#[test]
fn lifecycle_retry_cleanup_maps_unit_success_and_redacted_failure() {
    let (call_tx, call_rx) = sync_channel(2);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = LifecycleController::new(call_tx);
        controller.retry_cleanup_results = VecDeque::from([
            Ok(()),
            Err(ManagerError::new(
                ManagerErrorCode::ProcessFailure,
                ManagerOperation::StopSession,
            )),
        ]);
        Ok(controller)
    })
    .expect("spawn retry-cleanup lifecycle worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_retry_cleanup("00000000-0000-4000-8000-000000000171")
            .expect("submit successful cleanup retry")
            .get(),
        1
    );
    assert_eq!(
        worker
            .try_retry_cleanup("00000000-0000-4000-8000-000000000172")
            .expect("submit failing cleanup retry")
            .get(),
        2
    );

    match next_event(&mut worker) {
        ManagerWorkerEvent::CleanupRetried { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            assert_eq!(result.expect("scripted cleanup succeeds"), ());
        }
        other => panic!("expected successful CleanupRetried event, got {other:?}"),
    }
    match next_event(&mut worker) {
        ManagerWorkerEvent::CleanupRetried { request_id, result } => {
            assert_eq!(request_id.get(), 2);
            let error = result.expect_err("scripted cleanup failure is forwarded");
            assert_eq!(error.code(), ManagerErrorCode::ProcessFailure);
            assert_eq!(error.operation(), ManagerOperation::StopSession);
            assert!(error.source().is_none());
            let display = error.to_string();
            let debug = format!("{error:?}");
            for sentinel in [r"C:\Games\KnightOnline\secret-runtime", "os_code=995"] {
                assert!(!display.contains(sentinel));
                assert!(!debug.contains(sentinel));
            }
        }
        other => panic!("expected failing CleanupRetried event, got {other:?}"),
    }
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit marker after cleanup results")
            .get(),
        3
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 3 && sessions.is_empty()
    ));
    for session_id in [
        "00000000-0000-4000-8000-000000000171",
        "00000000-0000-4000-8000-000000000172",
    ] {
        assert_eq!(
            call_rx
                .recv_timeout(EVENT_DEADLINE)
                .expect("cleanup call reaches controller"),
            LifecycleCall::RetryCleanup {
                session_id: session_id.to_owned(),
            }
        );
    }
    assert!(call_rx.try_recv().is_err());
}

#[test]
fn lifecycle_calls_are_serial_on_exactly_one_worker_thread() {
    let thread_ids = Arc::new(Mutex::new(Vec::new()));
    let in_call = Arc::new(AtomicUsize::new(0));
    let maximum_in_call = Arc::new(AtomicUsize::new(0));
    let (first_entered_tx, first_entered_rx) = sync_channel(0);
    let (first_release_tx, first_release_rx) = sync_channel(0);
    let controller_thread_ids = Arc::clone(&thread_ids);
    let controller_in_call = Arc::clone(&in_call);
    let controller_maximum_in_call = Arc::clone(&maximum_in_call);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(SerialLifecycleController {
            probe: SerialCallProbe {
                thread_ids: controller_thread_ids,
                in_call: controller_in_call,
                maximum_in_call: controller_maximum_in_call,
                first_entered_tx: Some(first_entered_tx),
                first_release_rx: Some(first_release_rx),
            },
        })
    })
    .expect("spawn serialized lifecycle worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_start_profile("serial-profile", 5)
            .expect("submit first serialized call")
            .get(),
        1
    );
    first_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("first lifecycle call enters controller");
    assert_eq!(
        worker
            .try_observe_session("serial-session")
            .expect("queue observation behind first call")
            .get(),
        2
    );
    assert_eq!(
        worker
            .try_stop_session("serial-session")
            .expect("queue stop behind observation")
            .get(),
        3
    );
    assert_eq!(
        worker
            .try_retry_cleanup("serial-session")
            .expect("queue cleanup retry behind stop")
            .get(),
        4
    );
    first_release_tx
        .send(())
        .expect("release first lifecycle call");

    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ProfileStarted { request_id, .. } if request_id.get() == 1
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionObserved { request_id, .. } if request_id.get() == 2
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionStopped { request_id, .. } if request_id.get() == 3
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::CleanupRetried { request_id, .. } if request_id.get() == 4
    ));

    let recorded_thread_ids = thread_ids
        .lock()
        .expect("thread-ID recorder is not poisoned");
    assert_eq!(recorded_thread_ids.len(), 4);
    assert!(
        recorded_thread_ids
            .iter()
            .all(|thread_id| *thread_id == recorded_thread_ids[0])
    );
    assert_ne!(recorded_thread_ids[0], thread::current().id());
    assert_eq!(maximum_in_call.load(Ordering::SeqCst), 1);
    assert_eq!(in_call.load(Ordering::SeqCst), 0);
}

#[test]
fn lifecycle_oversize_ids_fail_before_enqueue_without_consuming_id() {
    let (call_tx, call_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = LifecycleController::new(call_tx);
        controller.start_result = Some(Ok(ManagerSessionView {
            session_id: "accepted-session".to_owned(),
            profile_id: "short-malformed-profile".to_owned(),
            profile_revision: 73,
            runtime_id: "lifecycle-runtime-cap".to_owned(),
            state: ManagerSessionState::Running,
        }));
        Ok(controller)
    })
    .expect("spawn lifecycle input-cap worker");
    consume_ready(&mut worker);

    let oversized = "x".repeat(37);
    for (error, operation) in [
        (
            worker
                .try_start_profile(&oversized, 73)
                .expect_err("oversize profile ID is rejected"),
            ManagerWorkerOperation::StartProfile,
        ),
        (
            worker
                .try_observe_session(&oversized)
                .expect_err("oversize observe session ID is rejected"),
            ManagerWorkerOperation::ObserveSession,
        ),
        (
            worker
                .try_stop_session(&oversized)
                .expect_err("oversize stop session ID is rejected"),
            ManagerWorkerOperation::StopSession,
        ),
        (
            worker
                .try_retry_cleanup(&oversized)
                .expect_err("oversize cleanup session ID is rejected"),
            ManagerWorkerOperation::RetryCleanup,
        ),
    ] {
        assert_eq!(error.code(), ManagerWorkerErrorCode::InputTooLong);
        assert_eq!(error.operation(), operation);
        assert_eq!(error.maximum(), Some(36));
        assert!(!error.to_string().contains(&oversized));
    }
    assert!(call_rx.try_recv().is_err());

    assert_eq!(
        worker
            .try_start_profile("short-malformed-profile", 73)
            .expect("short malformed profile ID is forwarded")
            .get(),
        1
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ProfileStarted { request_id, result }
            if request_id.get() == 1 && result.is_ok()
    ));
    assert_eq!(
        call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("only accepted lifecycle call reaches controller"),
        LifecycleCall::Start {
            profile_id: "short-malformed-profile".to_owned(),
            expected_revision: 73,
        }
    );
    assert!(call_rx.try_recv().is_err());
}

#[test]
fn shutdown_success_is_fifo_and_closes_only_when_its_result_is_consumed() {
    let (close_call_tx, close_call_rx) = sync_channel(4);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController {
            close_call_tx: Some(close_call_tx),
            ..FakeController::default()
        })
    })
    .expect("spawn ordered-shutdown worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit first command before shutdown")
            .get(),
        1
    );
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit second command before shutdown")
            .get(),
        2
    );
    assert_eq!(
        worker
            .try_shutdown()
            .expect("submit shutdown behind prior commands")
            .get(),
        3
    );
    assert_eq!(worker.state(), ManagerWorkerState::Closing);

    for (result, operation) in [
        (
            worker.try_list_profiles(None, 1, false),
            ManagerWorkerOperation::ListProfiles,
        ),
        (
            worker.try_list_runtimes(None, 1),
            ManagerWorkerOperation::ListRuntimes,
        ),
        (
            worker.try_list_sessions(),
            ManagerWorkerOperation::ListSessions,
        ),
        (
            worker.try_start_profile("shutdown-profile", 1),
            ManagerWorkerOperation::StartProfile,
        ),
        (
            worker.try_observe_session("shutdown-session"),
            ManagerWorkerOperation::ObserveSession,
        ),
        (
            worker.try_stop_session("shutdown-session"),
            ManagerWorkerOperation::StopSession,
        ),
        (
            worker.try_retry_cleanup("shutdown-session"),
            ManagerWorkerOperation::RetryCleanup,
        ),
        (worker.try_shutdown(), ManagerWorkerOperation::Shutdown),
    ] {
        assert_worker_error(result, ManagerWorkerErrorCode::ShutdownPending, operation);
    }

    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id.get() == 1
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id.get() == 2
    ));
    assert_eq!(worker.state(), ManagerWorkerState::Closing);
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ShutdownResult { request_id, result }
            if request_id.get() == 3 && result.is_ok()
    ));
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("accepted shutdown reaches close"),
        1
    );
    assert!(matches!(
        close_call_rx.recv_timeout(EVENT_DEADLINE),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));

    for (result, operation) in [
        (
            worker.try_list_profiles(None, 1, false),
            ManagerWorkerOperation::ListProfiles,
        ),
        (
            worker.try_list_runtimes(None, 1),
            ManagerWorkerOperation::ListRuntimes,
        ),
        (
            worker.try_list_sessions(),
            ManagerWorkerOperation::ListSessions,
        ),
        (
            worker.try_start_profile("closed-profile", 1),
            ManagerWorkerOperation::StartProfile,
        ),
        (
            worker.try_observe_session("closed-session"),
            ManagerWorkerOperation::ObserveSession,
        ),
        (
            worker.try_stop_session("closed-session"),
            ManagerWorkerOperation::StopSession,
        ),
        (
            worker.try_retry_cleanup("closed-session"),
            ManagerWorkerOperation::RetryCleanup,
        ),
        (worker.try_shutdown(), ManagerWorkerOperation::Shutdown),
    ] {
        assert_worker_error(result, ManagerWorkerErrorCode::Closed, operation);
    }
    assert_worker_error(
        worker.try_next_event(),
        ManagerWorkerErrorCode::Closed,
        ManagerWorkerOperation::ReceiveEvent,
    );
}

#[test]
fn shutdown_queue_full_keeps_ready_and_preserves_the_request_id() {
    let (query_entered_tx, query_entered_rx) = sync_channel(0);
    let (query_release_tx, query_release_rx) = sync_channel(0);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_query_barrier(
            query_entered_tx,
            query_release_rx,
        ))
    })
    .expect("spawn shutdown-backpressure worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit the barrier-controlled command")
            .get(),
        1
    );
    query_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("first command enters the controller barrier");
    for expected_id in 2..=17 {
        assert_eq!(
            worker
                .try_list_sessions()
                .expect("fill all sixteen waiting command slots")
                .get(),
            expected_id
        );
    }

    let full = worker
        .try_shutdown()
        .expect_err("shutdown is rejected while the command queue is full");
    assert_eq!(full.code(), ManagerWorkerErrorCode::CommandQueueFull);
    assert_eq!(full.operation(), ManagerWorkerOperation::Shutdown);
    assert_eq!(full.maximum(), Some(16));
    assert_eq!(worker.state(), ManagerWorkerState::Ready);

    query_release_tx
        .send(())
        .expect("release the barrier-controlled command");
    for expected_id in 1..=17 {
        assert!(matches!(
            next_event(&mut worker),
            ManagerWorkerEvent::SessionsListed { request_id, .. }
                if request_id.get() == expected_id
        ));
    }
    assert_eq!(
        worker
            .try_shutdown()
            .expect("retry shutdown after queue capacity is available")
            .get(),
        18
    );
    assert_eq!(worker.state(), ManagerWorkerState::Closing);
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ShutdownResult { request_id, result }
            if request_id.get() == 18 && result.is_ok()
    ));
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
}

#[test]
fn shutdown_close_incomplete_allows_ordered_recovery_and_request_five_retry() {
    let remaining = ManagerSessionView {
        session_id: "recovery-session".to_owned(),
        profile_id: "recovery-profile".to_owned(),
        profile_revision: 9,
        runtime_id: "recovery-runtime".to_owned(),
        state: ManagerSessionState::Running,
    };
    let incomplete = ManagerError::new(ManagerErrorCode::CloseIncomplete, ManagerOperation::Close)
        .with_remaining_sessions(vec![remaining.clone()]);
    let (close_call_tx, close_call_rx) = sync_channel(4);
    let controller_remaining = remaining.clone();
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = FakeController::with_close_results(
            close_call_tx,
            VecDeque::from([Err(incomplete), Ok(())]),
        );
        controller.sessions = vec![controller_remaining];
        Ok(controller)
    })
    .expect("spawn retryable-shutdown worker");
    consume_ready(&mut worker);

    assert_eq!(
        worker.try_shutdown().expect("submit first shutdown").get(),
        1
    );
    assert_worker_error(
        worker.try_shutdown(),
        ManagerWorkerErrorCode::ShutdownPending,
        ManagerWorkerOperation::Shutdown,
    );
    assert_eq!(worker.state(), ManagerWorkerState::Closing);

    match next_event(&mut worker) {
        ManagerWorkerEvent::ShutdownResult { request_id, result } => {
            assert_eq!(request_id.get(), 1);
            let error = result.expect_err("first close remains incomplete");
            assert_eq!(error.code(), ManagerErrorCode::CloseIncomplete);
            assert_eq!(error.operation(), ManagerOperation::Close);
            assert_eq!(error.remaining_sessions(), std::slice::from_ref(&remaining));
        }
        other => panic!("expected incomplete ShutdownResult, got {other:?}"),
    }
    assert_eq!(worker.state(), ManagerWorkerState::Closing);

    assert_worker_error(
        worker.try_list_profiles(None, 1, false),
        ManagerWorkerErrorCode::Closing,
        ManagerWorkerOperation::ListProfiles,
    );
    assert_worker_error(
        worker.try_list_runtimes(None, 1),
        ManagerWorkerErrorCode::Closing,
        ManagerWorkerOperation::ListRuntimes,
    );
    assert_worker_error(
        worker.try_start_profile("recovery-profile", 9),
        ManagerWorkerErrorCode::Closing,
        ManagerWorkerOperation::StartProfile,
    );

    assert_eq!(
        worker
            .try_list_sessions()
            .expect("inspect retained sessions")
            .get(),
        2
    );
    assert_eq!(
        worker
            .try_observe_session("recovery-session")
            .expect("observe retained session")
            .get(),
        3
    );
    assert_eq!(
        worker
            .try_stop_session("recovery-session")
            .expect("stop retained session")
            .get(),
        4
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionsListed { request_id, sessions }
            if request_id.get() == 2 && sessions == vec![remaining.clone()]
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionObserved { request_id, result }
            if request_id.get() == 3 && result.is_ok()
    ));
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::SessionStopped { request_id, result }
            if request_id.get() == 4 && result.is_ok()
    ));
    assert_eq!(
        worker
            .try_shutdown()
            .expect("retry shutdown after recovery")
            .get(),
        5
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ShutdownResult { request_id, result }
            if request_id.get() == 5 && result.is_ok()
    ));
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("first close is recorded"),
        1
    );
    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("shutdown retry is recorded"),
        2
    );
}

#[test]
fn shutdown_close_incomplete_admits_retry_cleanup() {
    let remaining = ManagerSessionView {
        session_id: "cleanup-session".to_owned(),
        profile_id: "cleanup-profile".to_owned(),
        profile_revision: 11,
        runtime_id: "cleanup-runtime".to_owned(),
        state: ManagerSessionState::CleanupPending,
    };
    let incomplete = ManagerError::new(ManagerErrorCode::CloseIncomplete, ManagerOperation::Close)
        .with_remaining_sessions(vec![remaining]);
    let (close_call_tx, _close_call_rx) = sync_channel(4);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController::with_close_results(
            close_call_tx,
            VecDeque::from([Err(incomplete), Ok(())]),
        ))
    })
    .expect("spawn cleanup-recovery worker");
    consume_ready(&mut worker);

    assert_eq!(worker.try_shutdown().expect("submit shutdown").get(), 1);
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ShutdownResult { request_id, result }
            if request_id.get() == 1 && result.is_err()
    ));
    assert_eq!(
        worker
            .try_retry_cleanup("cleanup-session")
            .expect("retry cleanup while closing")
            .get(),
        2
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::CleanupRetried { request_id, result }
            if request_id.get() == 2 && result.is_ok()
    ));
    assert_eq!(
        worker
            .try_shutdown()
            .expect("retry shutdown after cleanup")
            .get(),
        3
    );
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::ShutdownResult { request_id, result }
            if request_id.get() == 3 && result.is_ok()
    ));
}

#[test]
fn disconnect_command_sender_runs_one_best_effort_close() {
    let (close_call_tx, close_call_rx) = sync_channel(4);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController {
            close_call_tx: Some(close_call_tx),
            ..FakeController::default()
        })
    })
    .expect("spawn command-disconnect worker");
    consume_ready(&mut worker);

    worker.disconnect_command_sender_for_test();
    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("command disconnect reaches close"),
        1
    );
    assert!(matches!(
        close_call_rx.recv_timeout(EVENT_DEADLINE),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));

    let deadline = Instant::now() + EVENT_DEADLINE;
    let error = loop {
        match worker.try_next_event() {
            Err(error) => break error,
            Ok(None) if Instant::now() < deadline => thread::yield_now(),
            Ok(None) => panic!("command disconnect was not observed"),
            Ok(Some(event)) => panic!("command disconnect emitted an event: {event:?}"),
        }
    };
    assert_eq!(error.code(), ManagerWorkerErrorCode::Disconnected);
    assert_eq!(error.operation(), ManagerWorkerOperation::ReceiveEvent);
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
}

#[cfg(panic = "unwind")]
#[test]
fn disconnect_worker_panic_is_redacted_as_receive_disconnect() {
    const PANIC_SENTINEL: &str = "worker-panic-secret-sentinel-7821";

    let mut worker =
        ManagerWorker::spawn_with_boot(|| Ok(FakeController::panics_during_list_sessions()))
            .expect("spawn panic-redaction worker");
    consume_ready(&mut worker);
    assert_eq!(
        worker
            .try_list_sessions()
            .expect("submit panicking operation")
            .get(),
        1
    );

    let deadline = Instant::now() + EVENT_DEADLINE;
    let error = loop {
        match worker.try_next_event() {
            Err(error) => break error,
            Ok(None) if Instant::now() < deadline => thread::yield_now(),
            Ok(None) => panic!("panicked worker did not disconnect"),
            Ok(Some(event)) => panic!("panicked operation emitted an event: {event:?}"),
        }
    };
    assert_eq!(error.code(), ManagerWorkerErrorCode::Disconnected);
    assert_eq!(error.operation(), ManagerWorkerOperation::ReceiveEvent);
    assert_eq!(error.maximum(), None);
    assert!(!error.to_string().contains(PANIC_SENTINEL));
    assert!(!format!("{error:?}").contains(PANIC_SENTINEL));
    assert!(error.source().is_none());
    assert_eq!(worker.state(), ManagerWorkerState::Closed);
    assert_worker_error(
        worker.try_next_event(),
        ManagerWorkerErrorCode::Closed,
        ManagerWorkerOperation::ReceiveEvent,
    );
}

#[test]
fn shutdown_success_delivery_disconnect_does_not_close_twice() {
    let (close_entered_tx, close_entered_rx) = sync_channel(0);
    let (close_release_tx, close_release_rx) = sync_channel(0);
    let (close_call_tx, close_call_rx) = sync_channel(4);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller =
            FakeController::with_close_results(close_call_tx, VecDeque::from([Ok(())]));
        controller.close_barrier = Some(CloseBarrier {
            entered_tx: close_entered_tx,
            release_rx: close_release_rx,
        });
        Ok(controller)
    })
    .expect("spawn successful shutdown-delivery-disconnect worker");
    consume_ready(&mut worker);

    assert_eq!(worker.try_shutdown().expect("submit shutdown").get(), 1);
    close_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("shutdown reaches held close");
    drop(worker);
    close_release_tx.send(()).expect("release successful close");

    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("shutdown close is recorded"),
        1
    );
    assert!(matches!(
        close_call_rx.recv_timeout(EVENT_DEADLINE),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn shutdown_failure_delivery_disconnect_adds_one_best_effort_close() {
    let incomplete = ManagerError::new(ManagerErrorCode::CloseIncomplete, ManagerOperation::Close);
    let (close_entered_tx, close_entered_rx) = sync_channel(0);
    let (close_release_tx, close_release_rx) = sync_channel(0);
    let (close_call_tx, close_call_rx) = sync_channel(4);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        let mut controller = FakeController::with_close_results(
            close_call_tx,
            VecDeque::from([Err(incomplete), Ok(())]),
        );
        controller.close_barrier = Some(CloseBarrier {
            entered_tx: close_entered_tx,
            release_rx: close_release_rx,
        });
        Ok(controller)
    })
    .expect("spawn failed shutdown-delivery-disconnect worker");
    consume_ready(&mut worker);

    assert_eq!(worker.try_shutdown().expect("submit shutdown").get(), 1);
    close_entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("shutdown reaches held close");
    drop(worker);
    close_release_tx.send(()).expect("release incomplete close");

    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("shutdown close is recorded"),
        1
    );
    assert_eq!(
        close_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("disconnect cleanup close is recorded"),
        2
    );
    assert!(matches!(
        close_call_rx.recv_timeout(EVENT_DEADLINE),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

/// Fake control with real account behavior, so CRUD admission and FIFO results can be tested without
/// SQLite. Atomicity itself is proved against real SQLite in the Task 5 repository tests.
#[derive(Default)]
struct AccountController {
    accounts: RefCell<Vec<ManagerAccountView>>,
    calls: RefCell<Vec<&'static str>>,
    entered_tx: Option<SyncSender<()>>,
    release_rx: Option<Receiver<()>>,
    fail_import_with: Option<ManagerErrorCode>,
}

impl AccountController {
    fn with_accounts(views: Vec<ManagerAccountView>) -> Self {
        Self {
            accounts: RefCell::new(views),
            ..Self::default()
        }
    }
}

fn account_view(username: &str, revision: i64, status: ManagerAccountStatus) -> ManagerAccountView {
    ManagerAccountView {
        account_id: ManagerAccountId::new(Uuid::new_v4()),
        revision,
        username: username.to_owned(),
        status,
        last_run_at_unix_ms: None,
        server_index: 0,
    }
}

impl WorkerControl for AccountController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        Ok(ManagerProfilePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        Ok(ManagerRuntimePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        Vec::new()
    }

    fn start_profile(
        &mut self,
        _profile: &str,
        _revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StartProfile,
        ))
    }

    fn observe_session(&mut self, _session: &str) -> ManagerResult<ManagerObservation> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::ObserveSession,
        ))
    }

    fn stop_session(&mut self, _session: &str) -> ManagerResult<ManagerSessionExit> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StopSession,
        ))
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        Ok(())
    }

    fn list_accounts(&self) -> ManagerResult<Vec<ManagerAccountView>> {
        self.calls.borrow_mut().push("list");
        // Optional barrier proves the call really happens on the worker thread.
        if let Some(entered_tx) = &self.entered_tx {
            entered_tx
                .send(())
                .expect("entered receiver stays connected");
            self.release_rx
                .as_ref()
                .expect("barrier supplies a release channel")
                .recv()
                .expect("release sender stays connected");
        }
        Ok(self.accounts.borrow().clone())
    }

    fn import_account(
        &mut self,
        username: &str,
        password: ManagerAccountPassword,
    ) -> ManagerResult<ManagerAccountView> {
        self.calls.borrow_mut().push("import");
        // The worker owns the secret and drops it here; nothing may escape.
        drop(password);
        if let Some(code) = self.fail_import_with {
            return Err(ManagerError::new(code, ManagerOperation::ImportAccount));
        }
        let view = account_view(username, 1, ManagerAccountStatus::Idle);
        self.accounts.borrow_mut().push(view.clone());
        Ok(view)
    }

    fn update_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: &str,
        replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerResult<ManagerAccountView> {
        self.calls.borrow_mut().push("update");
        drop(replacement_password);
        let mut accounts = self.accounts.borrow_mut();
        let existing = accounts
            .iter_mut()
            .find(|account| account.account_id == account_id)
            .ok_or_else(|| {
                ManagerError::new(
                    ManagerErrorCode::AccountNotFound,
                    ManagerOperation::UpdateAccount,
                )
            })?;
        if existing.revision != expected_revision {
            return Err(ManagerError::new(
                ManagerErrorCode::RevisionConflict,
                ManagerOperation::UpdateAccount,
            ));
        }
        existing.revision += 1;
        existing.username = username.to_owned();
        Ok(existing.clone())
    }

    fn delete_account(
        &mut self,
        account_id: ManagerAccountId,
        _expected_revision: i64,
    ) -> ManagerResult<()> {
        self.calls.borrow_mut().push("delete");
        let mut accounts = self.accounts.borrow_mut();
        let before = accounts.len();
        accounts.retain(|account| account.account_id != account_id);
        if accounts.len() == before {
            return Err(ManagerError::new(
                ManagerErrorCode::AccountNotFound,
                ManagerOperation::DeleteAccount,
            ));
        }
        Ok(())
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

fn password(value: &str) -> ManagerAccountPassword {
    ManagerAccountPassword::try_from_utf16(
        ManagerWorkerOperation::ImportAccount,
        value.encode_utf16().collect(),
    )
    .expect("printable ASCII password is accepted")
}

#[test]
fn account_crud_returns_one_fifo_request_result_per_accepted_command() {
    let existing = account_view("Existing", 1, ManagerAccountStatus::Idle);
    let existing_id = existing.account_id;
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(AccountController::with_accounts(vec![existing]))
    })
    .expect("spawn account worker");
    consume_ready(&mut worker);

    let list = worker.try_list_accounts().expect("list is admitted");
    let import = worker
        .try_import_account("Imported", password("Secret-1"))
        .expect("import is admitted");
    let update = worker
        .try_update_account(existing_id, 1, "Renamed", None)
        .expect("update is admitted");
    let delete = worker
        .try_delete_account(existing_id, 2)
        .expect("delete is admitted");

    // Request IDs are gapless and results arrive in submission order.
    assert_eq!(
        [list.get(), import.get(), update.get(), delete.get()],
        [1, 2, 3, 4]
    );
    let ManagerWorkerEvent::AccountsListed { request_id, result } = next_event(&mut worker) else {
        panic!("list must produce an AccountsListed result");
    };
    assert_eq!(request_id, list);
    assert_eq!(result.expect("list succeeds").len(), 1);

    let ManagerWorkerEvent::AccountImported { request_id, result } = next_event(&mut worker) else {
        panic!("import must produce an AccountImported result");
    };
    assert_eq!(request_id, import);
    assert_eq!(result.expect("import succeeds").username, "Imported");

    let ManagerWorkerEvent::AccountUpdated { request_id, result } = next_event(&mut worker) else {
        panic!("update must produce an AccountUpdated result");
    };
    assert_eq!(request_id, update);
    let updated = result.expect("update succeeds");
    assert_eq!(updated.username, "Renamed");
    assert_eq!(updated.revision, 2);

    let ManagerWorkerEvent::AccountDeleted { request_id, result } = next_event(&mut worker) else {
        panic!("delete must produce an AccountDeleted result");
    };
    assert_eq!(request_id, delete);
    result.expect("delete succeeds");
}

#[test]
fn account_errors_stay_redacted_and_classified() {
    let mut worker = ManagerWorker::spawn_with_boot(|| {
        Ok(AccountController {
            fail_import_with: Some(ManagerErrorCode::DuplicateUsername),
            ..AccountController::default()
        })
    })
    .expect("spawn failing-import worker");
    consume_ready(&mut worker);

    let import = worker
        .try_import_account("Duplicate", password("Secret-1"))
        .expect("import is admitted before the controller rejects it");
    let ManagerWorkerEvent::AccountImported { request_id, result } = next_event(&mut worker) else {
        panic!("import must produce an AccountImported result");
    };
    assert_eq!(request_id, import);
    let error = result.expect_err("duplicate username is rejected");
    assert_eq!(error.code(), ManagerErrorCode::DuplicateUsername);
    assert_eq!(error.operation(), ManagerOperation::ImportAccount);
    // The rendered failure never carries the username or any credential material.
    let rendered = error.to_string();
    assert!(!rendered.contains("Duplicate"), "rendered: {rendered}");
    assert!(!rendered.contains("Secret-1"), "rendered: {rendered}");

    // A revision conflict on a missing account reports not-found, never a panic.
    let missing = ManagerAccountId::new(Uuid::new_v4());
    let update = worker
        .try_update_account(missing, 1, "Ghost", None)
        .expect("update is admitted");
    let ManagerWorkerEvent::AccountUpdated { request_id, result } = next_event(&mut worker) else {
        panic!("update must produce an AccountUpdated result");
    };
    assert_eq!(request_id, update);
    assert_eq!(
        result.expect_err("a missing account is rejected").code(),
        ManagerErrorCode::AccountNotFound
    );
}

#[test]
fn account_commands_are_rejected_before_ready_and_after_close() {
    let mut worker = ManagerWorker::spawn_with_boot(|| Ok(AccountController::default()))
        .expect("spawn admission worker");

    // Starting: every account command is NotReady and consumes no request ID.
    assert_worker_error(
        worker.try_list_accounts(),
        ManagerWorkerErrorCode::NotReady,
        ManagerWorkerOperation::ListAccounts,
    );
    assert_worker_error(
        worker.try_import_account("Early", password("Secret-1")),
        ManagerWorkerErrorCode::NotReady,
        ManagerWorkerOperation::ImportAccount,
    );
    consume_ready(&mut worker);
    assert_eq!(
        worker
            .try_list_accounts()
            .expect("first admitted command")
            .get(),
        1,
        "rejected commands must not consume request IDs"
    );

    let shutdown = worker.try_shutdown().expect("shutdown is admitted");
    // Shutdown in flight: mutations are refused with ShutdownPending.
    assert_worker_error(
        worker.try_import_account("Late", password("Secret-1")),
        ManagerWorkerErrorCode::ShutdownPending,
        ManagerWorkerOperation::ImportAccount,
    );
    loop {
        match next_event(&mut worker) {
            ManagerWorkerEvent::ShutdownResult { request_id, result } => {
                assert_eq!(request_id, shutdown);
                result.expect("close succeeds");
                break;
            }
            ManagerWorkerEvent::AccountsListed { .. } => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

#[test]
fn account_username_longer_than_the_bound_is_rejected_without_a_request_id() {
    let mut worker = ManagerWorker::spawn_with_boot(|| Ok(AccountController::default()))
        .expect("spawn bounds worker");
    consume_ready(&mut worker);

    let error = worker
        .try_import_account(&"u".repeat(65), password("Secret-1"))
        .expect_err("an over-long username is refused at the boundary");
    assert_eq!(error.code(), ManagerWorkerErrorCode::InputTooLong);
    assert_eq!(error.operation(), ManagerWorkerOperation::ImportAccount);
    assert_eq!(error.maximum(), Some(64));
    // The refused command consumed no request ID.
    assert_eq!(
        worker
            .try_list_accounts()
            .expect("first admitted command")
            .get(),
        1
    );
}

#[test]
fn account_list_work_happens_on_the_worker_thread() {
    let (entered_tx, entered_rx) = sync_channel(1);
    let (release_tx, release_rx) = sync_channel(1);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(AccountController {
            entered_tx: Some(entered_tx),
            release_rx: Some(release_rx),
            ..AccountController::default()
        })
    })
    .expect("spawn barrier worker");
    consume_ready(&mut worker);

    let list = worker.try_list_accounts().expect("list is admitted");
    // The call is already running on the worker thread while this thread still has no event.
    entered_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("the worker thread entered list_accounts");
    assert!(
        worker.try_next_event().expect("polling succeeds").is_none(),
        "the result must not be observable until the worker finishes"
    );
    release_tx.send(()).expect("release the worker thread");
    assert!(matches!(
        next_event(&mut worker),
        ManagerWorkerEvent::AccountsListed { request_id, .. } if request_id == list
    ));
}

/// `WorkerWake<u8>` at capacity four: the brief's exact unit shape.
#[test]
fn worker_park_wake_queue_is_bounded_and_drains_in_order() {
    let wake = WorkerWake::new(thread::current(), 4);
    assert!(wake.is_empty());

    for value in 1_u8..=4 {
        wake.push(value)
            .expect("pushes within capacity are accepted");
    }
    assert!(!wake.is_empty());
    // A full queue hands the value back instead of growing or blocking.
    assert_eq!(wake.push(5), Err(5));

    assert_eq!(wake.drain(), vec![1, 2, 3, 4]);
    assert!(wake.is_empty());
    // Draining frees the capacity again.
    wake.push(6).expect("capacity is reusable after a drain");
    assert_eq!(wake.drain(), vec![6]);
}

#[test]
fn worker_park_wake_unparks_the_target_thread() {
    let (ready_tx, ready_rx) = sync_channel(1);
    let (done_tx, done_rx) = sync_channel(1);
    let waiter = thread::spawn(move || {
        ready_tx
            .send(thread::current())
            .expect("wake receiver stays connected");
        // Parks until the pushing thread unparks it; a lost wake would hang this join.
        thread::park();
        done_tx.send(()).expect("done receiver stays connected");
    });
    let target = ready_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("waiter published its thread handle");

    let wake = WorkerWake::new(target, 4);
    wake.push(1_u8).expect("push is accepted");

    done_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("the push unparked the waiting thread");
    waiter.join().expect("waiter thread completes");
}

#[test]
fn worker_park_does_not_lose_a_command_enqueued_before_the_park() {
    let (query_call_tx, query_call_rx) = sync_channel(8);
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(FakeController {
            query_call_tx: Some(query_call_tx),
            ..FakeController::default()
        })
    })
    .expect("spawn park worker");
    consume_ready(&mut worker);

    // Submit repeatedly so at least some sends land while the worker is parked between commands.
    for _ in 0..16 {
        let request = worker
            .try_list_sessions()
            .expect("list is admitted while the worker parks");
        query_call_rx
            .recv_timeout(EVENT_DEADLINE)
            .expect("a parked worker still observes the command");
        assert!(matches!(
            next_event(&mut worker),
            ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id == request
        ));
    }
}

#[test]
fn worker_park_drop_releases_core_so_the_next_open_succeeds() {
    let root = TestDirectory::new("park-drop");
    {
        let mut worker = ManagerWorker::spawn_at(&root.0).expect("spawn a real Core-backed worker");
        consume_ready(&mut worker);
        // A second worker cannot open the same root while the first holds the instance lock.
        let mut blocked = ManagerWorker::spawn_at(&root.0).expect("spawn the blocked worker");
        assert!(matches!(
            next_event(&mut blocked),
            ManagerWorkerEvent::OpenFailed(_)
        ));
        // Dropping while the worker thread is parked must still close Core.
    }

    // The lock is released without any sleep: a fresh worker reaches Ready.
    let deadline = Instant::now() + EVENT_DEADLINE;
    loop {
        let mut reopened = ManagerWorker::spawn_at(&root.0).expect("respawn worker");
        match next_event(&mut reopened) {
            ManagerWorkerEvent::Ready => break,
            ManagerWorkerEvent::OpenFailed(error) if Instant::now() < deadline => {
                drop(reopened);
                assert_eq!(error.code(), ManagerErrorCode::ControllerAlreadyOpen);
                thread::yield_now();
            }
            other => panic!("reopen after Drop failed: {other:?}"),
        }
    }
}

/// Fake control that records account lifecycle calls and can report a session exit.
#[derive(Default)]
struct AccountLifecycleController {
    calls: RefCell<Vec<&'static str>>,
    stop_result: Option<ManagerAccountView>,
    stop_error: Option<ManagerErrorCode>,
}

impl WorkerControl for AccountLifecycleController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        Ok(ManagerProfilePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        Ok(ManagerRuntimePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        Vec::new()
    }

    fn start_profile(
        &mut self,
        _profile: &str,
        _revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StartProfile,
        ))
    }

    fn observe_session(&mut self, _session: &str) -> ManagerResult<ManagerObservation> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::ObserveSession,
        ))
    }

    fn stop_session(&mut self, _session: &str) -> ManagerResult<ManagerSessionExit> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StopSession,
        ))
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        Ok(())
    }
    fn stop_account(&mut self, _account_id: ManagerAccountId) -> ManagerResult<ManagerAccountView> {
        self.calls.borrow_mut().push("stop");
        if let Some(code) = self.stop_error {
            return Err(ManagerError::new(code, ManagerOperation::StopAccount));
        }
        self.stop_result.clone().ok_or_else(|| {
            ManagerError::new(
                ManagerErrorCode::AccountNotRunning,
                ManagerOperation::StopAccount,
            )
        })
    }
    fn retry_account_cleanup(
        &mut self,
        _account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        self.calls.borrow_mut().push("retry");
        self.stop_result.clone().ok_or_else(|| {
            ManagerError::new(
                ManagerErrorCode::AccountNotRunning,
                ManagerOperation::RetryAccountCleanup,
            )
        })
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

#[test]
fn account_lifecycle_stop_and_retry_return_fifo_results() {
    let stopped = ManagerAccountView {
        account_id: ManagerAccountId::new(Uuid::new_v4()),
        revision: 3,
        username: "Farmer".to_owned(),
        status: ManagerAccountStatus::Idle,
        last_run_at_unix_ms: Some(1_700_000_000_000),
        server_index: 0,
    };
    let expected = stopped.clone();
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(AccountLifecycleController {
            stop_result: Some(stopped),
            ..AccountLifecycleController::default()
        })
    })
    .expect("spawn lifecycle worker");
    consume_ready(&mut worker);
    let account = ManagerAccountId::new(Uuid::new_v4());

    let stop = worker.try_stop_account(account).expect("stop is admitted");
    let retry = worker
        .try_retry_account_cleanup(account)
        .expect("retry is admitted");
    assert_eq!([stop.get(), retry.get()], [1, 2]);

    let ManagerWorkerEvent::AccountStopResult { request_id, result } = next_event(&mut worker)
    else {
        panic!("stop must produce an AccountStopResult");
    };
    assert_eq!(request_id, stop);
    assert_eq!(result.expect("stop succeeds"), expected);

    let ManagerWorkerEvent::AccountCleanupRetried { request_id, result } = next_event(&mut worker)
    else {
        panic!("retry must produce an AccountCleanupRetried");
    };
    assert_eq!(request_id, retry);
    assert_eq!(result.expect("retry succeeds"), expected);
}

#[test]
fn account_lifecycle_stop_reports_a_redacted_not_running_failure() {
    let mut worker = ManagerWorker::spawn_with_boot(|| Ok(AccountLifecycleController::default()))
        .expect("spawn lifecycle worker");
    consume_ready(&mut worker);
    let account = ManagerAccountId::new(Uuid::new_v4());

    let stop = worker.try_stop_account(account).expect("stop is admitted");
    let ManagerWorkerEvent::AccountStopResult { request_id, result } = next_event(&mut worker)
    else {
        panic!("stop must produce an AccountStopResult");
    };
    assert_eq!(request_id, stop);
    let error = result.expect_err("an account with no session cannot be stopped");
    assert_eq!(error.code(), ManagerErrorCode::AccountNotRunning);
    assert_eq!(error.operation(), ManagerOperation::StopAccount);
    // The stable code carries no account identity.
    assert_eq!(error.code().as_str(), "account_not_running");
}

#[test]
fn account_lifecycle_commands_follow_worker_admission() {
    let mut worker = ManagerWorker::spawn_with_boot(|| Ok(AccountLifecycleController::default()))
        .expect("spawn lifecycle worker");
    let account = ManagerAccountId::new(Uuid::new_v4());

    assert_worker_error(
        worker.try_stop_account(account),
        ManagerWorkerErrorCode::NotReady,
        ManagerWorkerOperation::StopAccount,
    );
    assert_worker_error(
        worker.try_retry_account_cleanup(account),
        ManagerWorkerErrorCode::NotReady,
        ManagerWorkerOperation::RetryAccountCleanup,
    );
    consume_ready(&mut worker);
    // Rejected commands consumed no request ID.
    assert_eq!(
        worker
            .try_stop_account(account)
            .expect("first admitted command")
            .get(),
        1
    );
}

/// Fake control recording Run batches, so the public bounded-batch contract is testable.
#[derive(Default)]
struct RunController {
    batches: RefCell<Vec<Vec<(ManagerAccountId, i64)>>>,
    reject_all: bool,
}

impl WorkerControl for RunController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        Ok(ManagerProfilePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        Ok(ManagerRuntimePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        Vec::new()
    }

    fn start_profile(
        &mut self,
        _profile: &str,
        _revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StartProfile,
        ))
    }

    fn observe_session(&mut self, _session: &str) -> ManagerResult<ManagerObservation> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::ObserveSession,
        ))
    }

    fn stop_session(&mut self, _session: &str) -> ManagerResult<ManagerSessionExit> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StopSession,
        ))
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        Ok(())
    }

    fn run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerResult<Vec<ManagerRunSchedule>> {
        self.batches.borrow_mut().push(requests.to_vec());
        Ok(requests
            .iter()
            .map(|(account_id, _)| ManagerRunSchedule {
                account_id: *account_id,
                outcome: if self.reject_all {
                    ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::TaskLimitReached)
                } else {
                    ManagerRunScheduleOutcome::Scheduled
                },
            })
            .collect())
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

#[test]
fn run_batch_returns_one_bounded_result_per_accepted_command() {
    let mut worker =
        ManagerWorker::spawn_with_boot(|| Ok(RunController::default())).expect("spawn run worker");
    consume_ready(&mut worker);
    let batch: Vec<(ManagerAccountId, i64)> = (0..MAX_RUN_BATCH)
        .map(|_| (ManagerAccountId::new(Uuid::new_v4()), 1))
        .collect();

    let request = worker
        .try_run_accounts(&batch)
        .expect("a four-item batch is admitted");
    let ManagerWorkerEvent::AccountsRunScheduled { request_id, result } = next_event(&mut worker)
    else {
        panic!("run must produce an AccountsRunScheduled result");
    };
    assert_eq!(request_id, request);
    let scheduled = result.expect("the batch schedules");
    // One request result carries at most four per-account outcomes, in submission order.
    assert_eq!(scheduled.len(), MAX_RUN_BATCH);
    for (index, schedule) in scheduled.iter().enumerate() {
        assert_eq!(schedule.account_id, batch[index].0);
        assert_eq!(schedule.outcome, ManagerRunScheduleOutcome::Scheduled);
    }
}

#[test]
fn run_single_account_uses_the_same_one_item_batch_path() {
    let mut worker =
        ManagerWorker::spawn_with_boot(|| Ok(RunController::default())).expect("spawn run worker");
    consume_ready(&mut worker);
    let account = ManagerAccountId::new(Uuid::new_v4());

    let request = worker
        .try_run_account(account, 3)
        .expect("a single account is admitted");
    let ManagerWorkerEvent::AccountsRunScheduled { request_id, result } = next_event(&mut worker)
    else {
        panic!("run must produce an AccountsRunScheduled result");
    };
    assert_eq!(request_id, request);
    let scheduled = result.expect("the single item schedules");
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].account_id, account);
}

#[test]
fn run_rejects_an_oversized_or_empty_batch_without_a_request_id() {
    let mut worker =
        ManagerWorker::spawn_with_boot(|| Ok(RunController::default())).expect("spawn run worker");
    consume_ready(&mut worker);

    let oversized: Vec<(ManagerAccountId, i64)> = (0..=MAX_RUN_BATCH)
        .map(|_| (ManagerAccountId::new(Uuid::new_v4()), 1))
        .collect();
    let error = worker
        .try_run_accounts(&oversized)
        .expect_err("a fifth member is refused at the boundary");
    assert_eq!(error.code(), ManagerWorkerErrorCode::InputTooLong);
    assert_eq!(error.operation(), ManagerWorkerOperation::RunAccounts);
    assert_eq!(error.maximum(), Some(MAX_RUN_BATCH as u32));

    assert_worker_error(
        worker.try_run_accounts(&[]),
        ManagerWorkerErrorCode::InvalidInput,
        ManagerWorkerOperation::RunAccounts,
    );
    // Neither rejection consumed a request ID.
    assert_eq!(
        worker
            .try_run_account(ManagerAccountId::new(Uuid::new_v4()), 1)
            .expect("first admitted command")
            .get(),
        1
    );
}

#[test]
fn run_rejection_outcomes_stay_redacted_and_stable() {
    let mut worker = ManagerWorker::spawn_with_boot(|| {
        Ok(RunController {
            reject_all: true,
            ..RunController::default()
        })
    })
    .expect("spawn rejecting run worker");
    consume_ready(&mut worker);

    let request = worker
        .try_run_account(ManagerAccountId::new(Uuid::new_v4()), 1)
        .expect("run is admitted before the coordinator rejects it");
    let ManagerWorkerEvent::AccountsRunScheduled { request_id, result } = next_event(&mut worker)
    else {
        panic!("run must produce an AccountsRunScheduled result");
    };
    assert_eq!(request_id, request);
    let scheduled = result.expect("a rejection is still a successful schedule result");
    assert_eq!(
        scheduled[0].outcome,
        ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::TaskLimitReached)
    );
    assert_eq!(
        ManagerRunRejection::TaskLimitReached.as_str(),
        "task_limit_reached"
    );
}

#[test]
fn run_commands_follow_worker_admission() {
    let mut worker =
        ManagerWorker::spawn_with_boot(|| Ok(RunController::default())).expect("spawn run worker");
    let account = ManagerAccountId::new(Uuid::new_v4());

    assert_worker_error(
        worker.try_run_account(account, 1),
        ManagerWorkerErrorCode::NotReady,
        ManagerWorkerOperation::RunAccounts,
    );
    consume_ready(&mut worker);
    let request = worker
        .try_run_account(account, 1)
        .expect("run is admitted once ready");
    assert_eq!(request.get(), 1);

    let shutdown = worker.try_shutdown().expect("shutdown is admitted");
    assert_worker_error(
        worker.try_run_account(account, 1),
        ManagerWorkerErrorCode::ShutdownPending,
        ManagerWorkerOperation::RunAccounts,
    );
    loop {
        match next_event(&mut worker) {
            ManagerWorkerEvent::ShutdownResult { request_id, result } => {
                assert_eq!(request_id, shutdown);
                result.expect("close succeeds");
                break;
            }
            ManagerWorkerEvent::AccountsRunScheduled { .. } => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

/// Fake control that accepts or drops completions, so the notification path is testable without Windows.
#[derive(Default)]
struct CompletionController {
    /// Completions whose session key matches are accepted; everything else is treated as stale.
    live_session: Option<String>,
    applied: RefCell<Vec<String>>,
    /// Publishes the worker's private queue back to the test thread, which is the only way to reach a
    /// queue that is created on the worker thread.
    queue_tx: Option<SyncSender<Arc<WorkerWake<LoginCompletion>>>>,
}

impl WorkerControl for CompletionController {
    fn list_profiles(
        &self,
        _after: Option<&str>,
        _limit: u32,
        _archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        Ok(ManagerProfilePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_runtimes(
        &self,
        _after: Option<&str>,
        _limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        Ok(ManagerRuntimePage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        Vec::new()
    }

    fn start_profile(
        &mut self,
        _profile: &str,
        _revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StartProfile,
        ))
    }

    fn observe_session(&mut self, _session: &str) -> ManagerResult<ManagerObservation> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::ObserveSession,
        ))
    }

    fn stop_session(&mut self, _session: &str) -> ManagerResult<ManagerSessionExit> {
        Err(ManagerError::new(
            ManagerErrorCode::InternalInvariant,
            ManagerOperation::StopSession,
        ))
    }

    fn retry_cleanup(&mut self, _session: &str) -> ManagerResult<()> {
        Ok(())
    }

    fn attach_readiness_completions(&mut self, completions: Arc<WorkerWake<LoginCompletion>>) {
        if let Some(queue_tx) = &self.queue_tx {
            queue_tx
                .send(completions)
                .expect("queue receiver stays connected");
        }
    }

    fn apply_readiness_completion(
        &mut self,
        completion: LoginCompletion,
    ) -> Option<ManagerAccountView> {
        // Only a matching session key is accepted; anything else is stale and dropped silently.
        if self.live_session.as_deref() != Some(completion.session_key.as_str()) {
            return None;
        }
        self.applied
            .borrow_mut()
            .push(completion.session_key.clone());
        Some(ManagerAccountView {
            account_id: completion.account_id,
            revision: 2,
            username: "Operator".to_owned(),
            status: match completion.outcome {
                LoginOutcome::InputSent => ManagerAccountStatus::Running,
                _ => ManagerAccountStatus::LoginFailed,
            },
            last_run_at_unix_ms: Some(1_800_000_000_000),
            server_index: 0,
        })
    }

    fn close(&mut self) -> ManagerResult<()> {
        Ok(())
    }
}

/// Spawns a completion worker and returns it with its private queue.
fn spawn_completion_worker(
    live_session: &str,
) -> (ManagerWorker, Arc<WorkerWake<LoginCompletion>>) {
    let (queue_tx, queue_rx) = sync_channel(1);
    let live_session = live_session.to_owned();
    let mut worker = ManagerWorker::spawn_with_boot(move || {
        Ok(CompletionController {
            live_session: Some(live_session),
            queue_tx: Some(queue_tx),
            ..CompletionController::default()
        })
    })
    .expect("spawn completion worker");
    consume_ready(&mut worker);
    let queue = queue_rx
        .recv_timeout(EVENT_DEADLINE)
        .expect("the worker shared its private completion queue during boot");
    (worker, queue)
}

fn completion(
    account: ManagerAccountId,
    session_key: &str,
    outcome: LoginOutcome,
) -> LoginCompletion {
    LoginCompletion {
        account_id: account,
        session_key: session_key.to_owned(),
        epoch: 1,
        outcome,
    }
}

#[test]
fn worker_completion_publishes_one_notification_per_accepted_record() {
    let (mut worker, queue) = spawn_completion_worker("session-live");
    let account = ManagerAccountId::new(Uuid::new_v4());

    queue
        .push(completion(account, "session-live", LoginOutcome::InputSent))
        .expect("the bounded queue accepts one record");

    let ManagerWorkerEvent::AccountStateChanged(view) = next_event(&mut worker) else {
        panic!("an accepted completion must publish AccountStateChanged");
    };
    // The notification carries no request ID and reports the reconciled row.
    assert_eq!(view.account_id, account);
    assert_eq!(view.status, ManagerAccountStatus::Running);
    assert_eq!(view.revision, 2);
}

#[test]
fn worker_completion_reports_a_failed_login_as_login_failed() {
    let (mut worker, queue) = spawn_completion_worker("session-live");
    let account = ManagerAccountId::new(Uuid::new_v4());

    queue
        .push(completion(
            account,
            "session-live",
            LoginOutcome::Failed(LoginFailureCode::ReadyTimeout),
        ))
        .expect("the bounded queue accepts one record");

    let ManagerWorkerEvent::AccountStateChanged(view) = next_event(&mut worker) else {
        panic!("a failed login must still publish AccountStateChanged");
    };
    assert_eq!(view.status, ManagerAccountStatus::LoginFailed);
}

#[test]
fn worker_completion_drops_a_stale_record_without_any_event() {
    let (mut worker, queue) = spawn_completion_worker("session-live");

    // A record for a session the controller no longer owns is dropped silently.
    queue
        .push(completion(
            ManagerAccountId::new(Uuid::new_v4()),
            "session-gone",
            LoginOutcome::InputSent,
        ))
        .expect("the bounded queue accepts one record");

    // A subsequent command still completes, proving the loop kept running.
    let request = worker.try_list_sessions().expect("list is admitted");
    let event = next_event(&mut worker);
    assert!(
        matches!(
            event,
            ManagerWorkerEvent::SessionsListed { request_id, .. } if request_id == request
        ),
        "a dropped completion must publish no event, got {event:?}"
    );
}

#[test]
fn worker_completion_bounded_queue_refuses_a_fifth_record() {
    let (_worker, queue) = spawn_completion_worker("session-live");
    let account = ManagerAccountId::new(Uuid::new_v4());

    // Four readiness tasks means at most four in-flight records.
    let mut accepted = 0;
    for index in 0..8 {
        let record = completion(
            account,
            &format!("session-{index}"),
            LoginOutcome::InputSent,
        );
        if queue.push(record).is_ok() {
            accepted += 1;
        }
    }
    // The worker drains concurrently, so the queue never exceeds its bound but may accept more than
    // four over time. What must hold is that it never grows without limit.
    assert!(accepted >= 4, "the queue must accept at least its capacity");
    assert!(accepted <= 8);
}

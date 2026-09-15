use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::backend::{BackendSpawnFailure, ProcessBackend};
use super::{
    MAX_ACTIVE_SESSIONS, SessionFailure, SessionObservation, StartSessionFailure, SupervisorCore,
};
use crate::process_adapter::{ProcessBirthId, RootExit};
use crate::{CoreError, CoreState, ProcessLaunchSpec, ProfileRecord};

use crate::runtime_fixture;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "zeus-session-supervisor-{label}-{}",
            Uuid::new_v4()
        )))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

struct PreparedCore {
    _runtime: runtime_fixture::RuntimeFixture,
    _data_root: TestDirectory,
    core: CoreState,
    profile: ProfileRecord,
}

fn prepared_core_with_profile(label: &str) -> PreparedCore {
    let runtime = runtime_fixture::RuntimeFixture::new(label);
    let data_root = TestDirectory::new(label);
    let mut core = CoreState::open_at(&data_root.0).expect("open Supervisor test Core");
    let registered = core
        .register_runtime_descriptor(runtime.descriptor_path())
        .expect("register inert Supervisor runtime");
    let profile = core
        .create_profile(label, &registered.runtime_id)
        .expect("create Supervisor test profile");
    PreparedCore {
        _runtime: runtime,
        _data_root: data_root,
        core,
        profile,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeError {
    Rejected,
    ObserveFailed,
    TerminateFailed,
    CleanupFailed,
}

impl fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => "fake-rejected",
            Self::ObserveFailed => "fake-observe-failed",
            Self::TerminateFailed => "fake-terminate-failed",
            Self::CleanupFailed => "fake-cleanup-failed",
        })
    }
}

impl std::error::Error for FakeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpawnedSpec {
    session_id: Uuid,
    profile_id: Uuid,
    argv_count: usize,
}

enum FakeSpawnOutcome {
    Accept,
    Reject(FakeError),
    CleanupPending(FakeError),
}

enum FakeObserveOutcome {
    Running,
    Exited(u32),
    Error(FakeError),
}

struct FakeBackend {
    outcomes: VecDeque<FakeSpawnOutcome>,
    observe_outcomes: VecDeque<FakeObserveOutcome>,
    terminate_outcomes: VecDeque<Result<u32, FakeError>>,
    cleanup_outcomes: VecDeque<Result<(), FakeError>>,
    spawned_specs: Vec<SpawnedSpec>,
    next_owner: u32,
    observe_calls: usize,
    terminate_deadlines: Vec<Instant>,
    cleanup_deadlines: Vec<Instant>,
}

impl FakeBackend {
    fn accepting() -> Self {
        Self {
            outcomes: VecDeque::new(),
            observe_outcomes: VecDeque::new(),
            terminate_outcomes: VecDeque::new(),
            cleanup_outcomes: VecDeque::new(),
            spawned_specs: Vec::new(),
            next_owner: 1,
            observe_calls: 0,
            terminate_deadlines: Vec::new(),
            cleanup_deadlines: Vec::new(),
        }
    }

    fn spawned_specs(&self) -> &[SpawnedSpec] {
        &self.spawned_specs
    }

    fn with_outcomes(outcomes: impl IntoIterator<Item = FakeSpawnOutcome>) -> Self {
        Self {
            outcomes: outcomes.into_iter().collect(),
            observe_outcomes: VecDeque::new(),
            terminate_outcomes: VecDeque::new(),
            cleanup_outcomes: VecDeque::new(),
            spawned_specs: Vec::new(),
            next_owner: 1,
            observe_calls: 0,
            terminate_deadlines: Vec::new(),
            cleanup_deadlines: Vec::new(),
        }
    }

    fn script_observe(mut self, outcomes: impl IntoIterator<Item = FakeObserveOutcome>) -> Self {
        self.observe_outcomes = outcomes.into_iter().collect();
        self
    }

    fn script_terminate(
        mut self,
        outcomes: impl IntoIterator<Item = Result<u32, FakeError>>,
    ) -> Self {
        self.terminate_outcomes = outcomes.into_iter().collect();
        self
    }

    fn script_cleanup(mut self, outcomes: impl IntoIterator<Item = Result<(), FakeError>>) -> Self {
        self.cleanup_outcomes = outcomes.into_iter().collect();
        self
    }
}

impl ProcessBackend for FakeBackend {
    type RunningOwner = u32;
    type CleanupOwner = u32;
    type Error = FakeError;

    fn spawn(
        &mut self,
        spec: ProcessLaunchSpec,
        _deadline: Instant,
    ) -> Result<Self::RunningOwner, BackendSpawnFailure<Self::CleanupOwner, Self::Error>> {
        self.spawned_specs.push(SpawnedSpec {
            session_id: spec.session_id(),
            profile_id: spec.profile_id(),
            argv_count: spec.arguments().len(),
        });
        let owner = self.next_owner;
        self.next_owner += 1;
        match self
            .outcomes
            .pop_front()
            .unwrap_or(FakeSpawnOutcome::Accept)
        {
            FakeSpawnOutcome::Accept => Ok(owner),
            FakeSpawnOutcome::Reject(error) => Err(BackendSpawnFailure::Rejected(error)),
            FakeSpawnOutcome::CleanupPending(error) => {
                Err(BackendSpawnFailure::CleanupUnconfirmed { error, owner })
            }
        }
    }

    fn birth_id(&self, owner: &Self::RunningOwner) -> ProcessBirthId {
        ProcessBirthId::for_test(*owner, 10_000 + u64::from(*owner))
    }

    fn try_wait_root(
        &mut self,
        owner: &mut Self::RunningOwner,
    ) -> Result<Option<RootExit>, Self::Error> {
        self.observe_calls += 1;
        match self
            .observe_outcomes
            .pop_front()
            .unwrap_or(FakeObserveOutcome::Running)
        {
            FakeObserveOutcome::Running => Ok(None),
            FakeObserveOutcome::Exited(exit_code) => {
                Ok(Some(RootExit::for_test(self.birth_id(owner), exit_code)))
            }
            FakeObserveOutcome::Error(error) => Err(error),
        }
    }

    fn terminate_tree_and_wait(
        &mut self,
        owner: &mut Self::RunningOwner,
        deadline: Instant,
    ) -> Result<RootExit, Self::Error> {
        self.terminate_deadlines.push(deadline);
        self.terminate_outcomes
            .pop_front()
            .unwrap_or(Ok(0xffff_fffd))
            .map(|exit_code| RootExit::for_test(self.birth_id(owner), exit_code))
    }

    fn retry_cleanup(
        &mut self,
        _owner: &mut Self::CleanupOwner,
        deadline: Instant,
    ) -> Result<(), Self::Error> {
        self.cleanup_deadlines.push(deadline);
        self.cleanup_outcomes.pop_front().unwrap_or(Ok(()))
    }
}

#[test]
fn natural_exit_is_removed_only_after_tree_empty_confirmation() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-natural-exit");
    let backend = FakeBackend::accepting()
        .script_observe([
            FakeObserveOutcome::Exited(37),
            FakeObserveOutcome::Exited(37),
        ])
        .script_terminate([Err(FakeError::TerminateFailed), Ok(37)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start natural-exit fake session");
    let session_id = started.metadata().session_id();

    assert!(matches!(
        supervisor.observe_session(session_id, deadline),
        Err(SessionFailure::Process {
            session_id: failed_session_id,
            error: FakeError::TerminateFailed,
        }) if failed_session_id == session_id
    ));
    assert_eq!(supervisor.active_session_count(), 1);

    let observation = supervisor
        .observe_session(session_id, deadline)
        .expect("second observation should confirm empty tree");
    let exit = match observation {
        SessionObservation::Exited(exit) => exit,
        other => panic!("unexpected terminal observation: {other:?}"),
    };
    assert_eq!(exit.metadata(), started.metadata());
    assert_eq!(exit.birth_id(), started.birth_id());
    assert_eq!(exit.exit_code(), 37);
    assert_eq!(supervisor.active_session_count(), 0);
    assert!(supervisor.profile_index.is_empty());
    assert!(supervisor.sessions.is_empty());
}

#[test]
fn still_running_observation_is_non_blocking_and_retains_owner() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-running-observation");
    let backend = FakeBackend::accepting().script_observe([FakeObserveOutcome::Running]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start running fake session");

    let observation = supervisor
        .observe_session(started.metadata().session_id(), deadline)
        .expect("observe running session");

    assert!(matches!(
        observation,
        SessionObservation::Running {
            metadata,
            birth_id,
        } if metadata == *started.metadata() && birth_id == started.birth_id()
    ));
    assert_eq!(supervisor.backend().observe_calls, 1);
    assert!(supervisor.backend().terminate_deadlines.is_empty());
    assert_eq!(supervisor.active_session_count(), 1);
}

#[test]
fn root_observation_error_retains_running_owner() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-observe-error");
    let backend = FakeBackend::accepting()
        .script_observe([FakeObserveOutcome::Error(FakeError::ObserveFailed)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start observe-error fake session");

    assert!(matches!(
        supervisor.observe_session(started.metadata().session_id(), deadline),
        Err(SessionFailure::Process {
            error: FakeError::ObserveFailed,
            ..
        })
    ));
    assert_eq!(supervisor.active_session_count(), 1);
    assert!(supervisor.backend().terminate_deadlines.is_empty());
}

#[test]
fn hard_stop_error_retains_running_owner_and_profile_admission() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-stop-error");
    let backend = FakeBackend::accepting().script_terminate([Err(FakeError::TerminateFailed)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start stop-error fake session");

    assert!(matches!(
        supervisor.stop_session(started.metadata().session_id(), deadline),
        Err(SessionFailure::Process {
            error: FakeError::TerminateFailed,
            ..
        })
    ));
    assert_eq!(supervisor.backend().terminate_deadlines, [deadline]);
    assert_eq!(supervisor.active_session_count(), 1);
    assert!(matches!(
        supervisor.start_session(&profile.profile_id, profile.revision, deadline),
        Err(StartSessionFailure::ProfileAlreadyActive { .. })
    ));
}

#[test]
fn cleanup_retry_releases_admission_only_after_confirmed_success() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-cleanup-retry");
    let backend =
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeError::Rejected)])
            .script_cleanup([Err(FakeError::CleanupFailed), Ok(())]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let session_id = match supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("start should retain cleanup owner")
    {
        StartSessionFailure::CleanupPending { session_id, .. } => session_id,
        other => panic!("unexpected start failure: {other}"),
    };

    assert!(matches!(
        supervisor.retry_cleanup(session_id, deadline),
        Err(SessionFailure::Process {
            error: FakeError::CleanupFailed,
            ..
        })
    ));
    assert_eq!(supervisor.active_session_count(), 1);
    assert!(matches!(
        supervisor.start_session(&profile.profile_id, profile.revision, deadline),
        Err(StartSessionFailure::ProfileAlreadyActive { .. })
    ));

    supervisor
        .retry_cleanup(session_id, deadline)
        .expect("second retry should confirm cleanup");
    assert_eq!(supervisor.backend().cleanup_deadlines, [deadline, deadline]);
    assert_eq!(supervisor.active_session_count(), 0);
    supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("confirmed cleanup should release same-profile admission");
}

#[test]
fn wrong_state_and_unknown_operations_do_not_mutate_ownership() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-wrong-state");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);
    let running = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start wrong-state running session");
    let unknown = Uuid::new_v4();

    assert!(matches!(
        supervisor.retry_cleanup(running.metadata().session_id(), deadline),
        Err(SessionFailure::WrongState {
            expected: super::SessionStateKind::CleanupPending,
            actual: super::SessionStateKind::Running,
            ..
        })
    ));
    assert!(matches!(
        supervisor.stop_session(unknown, deadline),
        Err(SessionFailure::SessionNotFound { session_id }) if session_id == unknown
    ));
    assert!(matches!(
        supervisor.observe_session(unknown, deadline),
        Err(SessionFailure::SessionNotFound { session_id }) if session_id == unknown
    ));
    assert!(matches!(
        supervisor.retry_cleanup(unknown, deadline),
        Err(SessionFailure::SessionNotFound { session_id }) if session_id == unknown
    ));
    assert!(supervisor.backend().cleanup_deadlines.is_empty());
    assert!(supervisor.backend().terminate_deadlines.is_empty());
    assert_eq!(supervisor.active_session_count(), 1);
}

#[test]
fn stop_rejects_cleanup_pending_without_invoking_backend() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-stop-cleanup-pending");
    let backend =
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeError::Rejected)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let session_id = match supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("start should retain cleanup owner")
    {
        StartSessionFailure::CleanupPending { session_id, .. } => session_id,
        other => panic!("unexpected start failure: {other}"),
    };

    assert!(matches!(
        supervisor
            .observe_session(session_id, deadline)
            .expect("observe cleanup-pending session"),
        SessionObservation::CleanupPending { metadata }
            if metadata.session_id() == session_id
    ));

    assert!(matches!(
        supervisor.stop_session(session_id, deadline),
        Err(SessionFailure::WrongState {
            expected: super::SessionStateKind::Running,
            actual: super::SessionStateKind::CleanupPending,
            ..
        })
    ));
    assert!(supervisor.backend().terminate_deadlines.is_empty());
    assert_eq!(supervisor.active_session_count(), 1);
}

#[test]
fn confirmed_hard_stop_returns_exact_exit_and_releases_both_indexes() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-stop-success");
    let backend = FakeBackend::accepting().script_terminate([Ok(91)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start stop-success session");

    let exit = supervisor
        .stop_session(started.metadata().session_id(), deadline)
        .expect("confirmed hard stop should return exact exit");

    assert_eq!(exit.metadata(), started.metadata());
    assert_eq!(exit.birth_id(), started.birth_id());
    assert_eq!(exit.exit_code(), 91);
    assert_eq!(supervisor.backend().terminate_deadlines, [deadline]);
    assert_eq!(supervisor.active_session_count(), 0);
    assert!(supervisor.sessions.is_empty());
    assert!(supervisor.profile_index.is_empty());
}

#[test]
fn cleanup_index_mismatch_fails_closed_after_backend_confirmation() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-cleanup-index-mismatch");
    let backend =
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeError::Rejected)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let session_id = match supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("start should retain cleanup owner")
    {
        StartSessionFailure::CleanupPending { session_id, .. } => session_id,
        other => panic!("unexpected start failure: {other}"),
    };
    supervisor.profile_index.insert(
        Uuid::parse_str(&profile.profile_id).expect("profile UUID"),
        Uuid::new_v4(),
    );

    assert!(matches!(
        supervisor.retry_cleanup(session_id, deadline),
        Err(SessionFailure::InvariantViolation {
            code: "supervisor_profile_index_mismatch"
        })
    ));
    assert_eq!(supervisor.backend().cleanup_deadlines, [deadline]);
    assert_eq!(supervisor.active_session_count(), 1);
    assert!(supervisor.sessions.contains_key(&session_id));
}

#[test]
fn profile_index_mismatch_fails_closed_and_retains_owner() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-index-mismatch");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start index-mismatch session");
    supervisor
        .profile_index
        .insert(started.metadata().profile_id(), Uuid::new_v4());

    assert!(matches!(
        supervisor.stop_session(started.metadata().session_id(), deadline),
        Err(SessionFailure::InvariantViolation {
            code: "supervisor_profile_index_mismatch"
        })
    ));
    assert_eq!(supervisor.active_session_count(), 1);
    assert!(
        supervisor
            .sessions
            .contains_key(&started.metadata().session_id())
    );
}

#[test]
fn confirmed_terminal_sessions_leave_no_history() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-no-history");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);

    for _ in 0..2 {
        let started = supervisor
            .start_session(&profile.profile_id, profile.revision, deadline)
            .expect("start no-history session");
        supervisor
            .stop_session(started.metadata().session_id(), deadline)
            .expect("confirmed stop should remove no-history session");
        assert!(supervisor.sessions.is_empty());
        assert!(supervisor.profile_index.is_empty());
    }
}

#[test]
fn successful_start_uses_core_snapshot_and_reserves_profile() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-start");
    let backend = FakeBackend::accepting();
    let mut supervisor = SupervisorCore::new(core, backend);

    let started = supervisor
        .start_session(
            &profile.profile_id,
            profile.revision,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("start fake contained session");

    assert_eq!(
        started.metadata().profile_id().to_string(),
        profile.profile_id
    );
    assert_eq!(started.metadata().profile_revision(), profile.revision);
    assert_ne!(started.birth_id().pid(), 0);
    assert_eq!(started.metadata().runtime_id(), profile.runtime_id);
    assert_eq!(started.metadata().descriptor_sha256().len(), 64);
    assert_eq!(supervisor.active_session_count(), 1);
    assert_eq!(supervisor.backend().spawned_specs().len(), 1);
    assert_eq!(
        supervisor.backend().spawned_specs()[0].session_id,
        started.metadata().session_id()
    );
    assert_eq!(
        supervisor.backend().spawned_specs()[0].profile_id,
        started.metadata().profile_id()
    );
    assert!(supervisor.backend().spawned_specs()[0].argv_count > 0);
}

#[test]
fn active_profile_is_rejected_before_another_preflight_or_spawn() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-duplicate");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("first start should be admitted");
    let spawn_count = supervisor.backend().spawned_specs().len();
    let diagnostics = supervisor.core().runtime_preflight_diagnostics();

    let failure = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("same profile must be rejected before preflight");

    assert!(matches!(
        failure,
        StartSessionFailure::ProfileAlreadyActive {
            profile_id,
            session_id,
        } if profile_id == started.metadata().profile_id()
            && session_id == started.metadata().session_id()
    ));
    assert_eq!(supervisor.backend().spawned_specs().len(), spawn_count);
    let after = supervisor.core().runtime_preflight_diagnostics();
    assert_eq!(after.mode(), diagnostics.mode());
    assert_eq!(after.cache_entries(), diagnostics.cache_entries());
    assert_eq!(
        after.content_bytes_hashed(),
        diagnostics.content_bytes_hashed()
    );
    assert_eq!(
        after.jre_content_bytes_hashed(),
        diagnostics.jre_content_bytes_hashed()
    );
}

#[test]
fn cleanup_unconfirmed_start_is_retained_and_consumes_admission() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-cleanup-pending");
    let backend =
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeError::Rejected)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);

    let failure = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("cleanup-unconfirmed start should report retained ownership");
    let session_id = match failure {
        StartSessionFailure::CleanupPending { session_id, .. } => session_id,
        other => panic!("unexpected cleanup-unconfirmed failure: {other}"),
    };

    assert_eq!(supervisor.active_session_count(), 1);
    assert!(matches!(
        supervisor.sessions.get(&session_id),
        Some(super::SessionEntry {
            owner: super::SessionOwner::CleanupPending(_),
            ..
        })
    ));
    assert!(matches!(
        supervisor.start_session(&profile.profile_id, profile.revision, deadline),
        Err(StartSessionFailure::ProfileAlreadyActive {
            session_id: active_session_id,
            ..
        }) if active_session_id == session_id
    ));
}

#[test]
fn four_distinct_profiles_are_admitted_and_the_fifth_is_rejected() {
    let runtime = runtime_fixture::RuntimeFixture::new("supervisor-capacity");
    let data_root = TestDirectory::new("supervisor-capacity");
    let mut core = CoreState::open_at(&data_root.0).expect("open capacity Core");
    let registered = core
        .register_runtime_descriptor(runtime.descriptor_path())
        .expect("register capacity runtime");
    let profiles = (0..=MAX_ACTIVE_SESSIONS)
        .map(|index| {
            core.create_profile(&format!("capacity-{index}"), &registered.runtime_id)
                .expect("create capacity profile")
        })
        .collect::<Vec<_>>();
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);

    for profile in &profiles[..MAX_ACTIVE_SESSIONS] {
        supervisor
            .start_session(&profile.profile_id, profile.revision, deadline)
            .expect("profile inside capacity should start");
    }
    let failure = supervisor
        .start_session(
            &profiles[MAX_ACTIVE_SESSIONS].profile_id,
            profiles[MAX_ACTIVE_SESSIONS].revision,
            deadline,
        )
        .expect_err("fifth ownership-bearing session must be rejected");

    assert!(matches!(
        failure,
        StartSessionFailure::CapacityReached { maximum }
            if maximum == MAX_ACTIVE_SESSIONS
    ));
    assert_eq!(supervisor.active_session_count(), MAX_ACTIVE_SESSIONS);
    assert_eq!(
        supervisor.backend().spawned_specs().len(),
        MAX_ACTIVE_SESSIONS
    );
}

#[test]
fn rejected_spawn_consumes_no_admission() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-rejected");
    let backend = FakeBackend::with_outcomes([FakeSpawnOutcome::Reject(FakeError::Rejected)]);
    let mut supervisor = SupervisorCore::new(core, backend);

    let failure = supervisor
        .start_session(
            &profile.profile_id,
            profile.revision,
            Instant::now() + Duration::from_secs(5),
        )
        .expect_err("backend rejection should be typed");

    assert!(matches!(
        failure,
        StartSessionFailure::SpawnRejected {
            error: FakeError::Rejected,
            ..
        }
    ));
    assert_eq!(supervisor.active_session_count(), 0);
    assert!(supervisor.profile_index.is_empty());
    assert_eq!(supervisor.backend().spawned_specs().len(), 1);
}

#[test]
fn preflight_failures_consume_no_admission_or_spawn() {
    let PreparedCore {
        mut core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-preflight-failures");
    let archived = core
        .create_profile("archived", &profile.runtime_id)
        .expect("create archived profile");
    core.archive_profile(&archived.profile_id, archived.revision)
        .expect("archive profile");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);
    let missing = Uuid::new_v4().to_string();

    for failure in [
        supervisor.start_session("not-a-uuid", profile.revision, deadline),
        supervisor.start_session(&missing, profile.revision, deadline),
        supervisor.start_session(&profile.profile_id, profile.revision + 1, deadline),
        supervisor.start_session(&archived.profile_id, archived.revision + 1, deadline),
    ] {
        assert!(matches!(failure, Err(StartSessionFailure::Core(_))));
        assert_eq!(supervisor.active_session_count(), 0);
        assert!(supervisor.profile_index.is_empty());
        assert!(supervisor.backend().spawned_specs().is_empty());
    }

    assert!(matches!(
        supervisor.start_session("not-a-uuid", profile.revision, deadline),
        Err(StartSessionFailure::Core(CoreError::InvalidProfileId))
    ));

    let runtime = supervisor
        .core()
        .inspect_runtime(&profile.runtime_id)
        .expect("inspect registered runtime before mutation");
    fs::remove_file(&runtime.game_path).expect("remove inert game artifact");
    assert!(matches!(
        supervisor.start_session(&profile.profile_id, profile.revision, deadline),
        Err(StartSessionFailure::Core(_))
    ));
    assert_eq!(supervisor.active_session_count(), 0);
    assert!(supervisor.backend().spawned_specs().is_empty());
}

#[test]
fn session_epoch_is_invalidated_before_stop_so_a_captured_target_goes_stale() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-epoch-stop");
    let backend = FakeBackend::accepting().script_terminate([Ok(0)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start epoch session");
    let session_id = started.metadata().session_id();

    // A readiness task captures the epoch while the session is live.
    let epoch = supervisor
        .session_epoch(session_id)
        .expect("a live session shares its epoch");
    let captured = epoch.load(Ordering::Acquire);

    supervisor
        .stop_session(session_id, deadline)
        .expect("stop the epoch session");

    // Stop bumped the epoch, so the captured value is stale and the task must abandon its work.
    assert_ne!(
        epoch.load(Ordering::Acquire),
        captured,
        "stop must invalidate the captured epoch"
    );
    // The session is gone, so no further epoch handle is available.
    assert!(supervisor.session_epoch(session_id).is_none());
}

#[test]
fn session_epoch_is_invalidated_before_cleanup_retry() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-epoch-cleanup");
    let backend =
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeError::Rejected)]);
    let mut supervisor = SupervisorCore::new(core, backend);
    let deadline = Instant::now() + Duration::from_secs(5);
    let failure = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect_err("cleanup-pending start reports a failure");
    let StartSessionFailure::CleanupPending { session_id, .. } = failure else {
        panic!("expected a cleanup-pending start failure");
    };

    let epoch = supervisor
        .session_epoch(session_id)
        .expect("a cleanup-pending session shares its epoch");
    let captured = epoch.load(Ordering::Acquire);

    let _ = supervisor.retry_cleanup(session_id, deadline);

    assert_ne!(
        epoch.load(Ordering::Acquire),
        captured,
        "cleanup retry must invalidate the captured epoch"
    );
}

#[test]
fn session_epoch_resolves_the_owning_session_for_one_profile() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("supervisor-epoch-profile");
    let mut supervisor = SupervisorCore::new(core, FakeBackend::accepting());
    let deadline = Instant::now() + Duration::from_secs(5);
    let profile_uuid = Uuid::parse_str(&profile.profile_id).expect("profile UUID");
    assert!(supervisor.session_for_profile(profile_uuid).is_none());

    let started = supervisor
        .start_session(&profile.profile_id, profile.revision, deadline)
        .expect("start profile session");

    assert_eq!(
        supervisor.session_for_profile(profile_uuid),
        Some(started.metadata().session_id())
    );
    // Each session owns an independent epoch handle.
    let unknown = Uuid::new_v4();
    assert!(supervisor.session_epoch(unknown).is_none());
}

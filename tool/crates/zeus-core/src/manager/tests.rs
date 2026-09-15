use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{
    DeadlineSource, ManagerCore, ManagerError, ManagerErrorCode, ManagerObservation,
    ManagerOperation, ManagerSessionState, ManagerSessionView, RedactedProcessError,
};
use crate::process_adapter::{ProcessBirthId, RootExit};
use crate::session_supervisor::{BackendSpawnFailure, ProcessBackend, StartSessionFailure};
use crate::{CoreError, CoreState, ProcessLaunchSpec, ProfileRecord};
use uuid::Uuid;

use crate::runtime_fixture;

const SESSION_ID: &str = "2b6f0cc9-04f8-4d0f-9c4d-1a7cfa5b7a11";
const PROFILE_ID: &str = "8f14e45f-ce8b-4a3f-8b7b-6c2f9f0a1d42";
const PROFILE_REVISION: i64 = 7;
const RUNTIME_ID: &str = "runtime-a";
const MAX_RENDER_BYTES: usize = 512;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("zeus-manager-unit-{label}-{}", Uuid::new_v4())))
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
    let mut core = CoreState::open_at(&data_root.0).expect("open manager unit Core");
    let registered = core
        .register_runtime_descriptor(runtime.descriptor_path())
        .expect("register manager unit runtime");
    let profile = core
        .create_profile(label, &registered.runtime_id)
        .expect("create manager unit profile");
    PreparedCore {
        _runtime: runtime,
        _data_root: data_root,
        core,
        profile,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeProcessError {
    Rejected,
    DeadlineExpired,
    ObserveFailed,
    TerminateFailed,
    CleanupFailed,
}

impl fmt::Display for FakeProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => r"fake rejected at D:\private\runtime os_code=1234",
            Self::DeadlineExpired => r"fake deadline at D:\private\runtime os_code=5678",
            Self::ObserveFailed => r"fake observe at D:\private\runtime os_code=2233",
            Self::TerminateFailed => r"fake terminate at D:\private\runtime os_code=3344",
            Self::CleanupFailed => r"fake cleanup at D:\private\runtime os_code=4455",
        })
    }
}

impl Error for FakeProcessError {}

impl RedactedProcessError for FakeProcessError {
    fn deadline_expired(&self) -> bool {
        matches!(self, Self::DeadlineExpired)
    }
}

enum FakeSpawnOutcome {
    Accept,
    Reject(FakeProcessError),
    CleanupPending(FakeProcessError),
}

enum FakeObserveOutcome {
    Running,
    Exited(u32),
    Error(FakeProcessError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeAction {
    Terminate { owner: u32, deadline: Instant },
    Cleanup { owner: u32, deadline: Instant },
}

struct FakeBackend {
    outcomes: VecDeque<FakeSpawnOutcome>,
    spawn_deadlines: Vec<Instant>,
    spawn_count: usize,
    next_owner: u32,
    observe_outcomes: VecDeque<FakeObserveOutcome>,
    terminate_outcomes: VecDeque<Result<u32, FakeProcessError>>,
    cleanup_outcomes: VecDeque<Result<(), FakeProcessError>>,
    observe_count: usize,
    terminate_deadlines: Vec<Instant>,
    cleanup_deadlines: Vec<Instant>,
    action_log: Vec<FakeAction>,
}

impl FakeBackend {
    fn with_outcomes(outcomes: impl IntoIterator<Item = FakeSpawnOutcome>) -> Self {
        Self {
            outcomes: outcomes.into_iter().collect(),
            spawn_deadlines: Vec::new(),
            spawn_count: 0,
            next_owner: 1,
            observe_outcomes: VecDeque::new(),
            terminate_outcomes: VecDeque::new(),
            cleanup_outcomes: VecDeque::new(),
            observe_count: 0,
            terminate_deadlines: Vec::new(),
            cleanup_deadlines: Vec::new(),
            action_log: Vec::new(),
        }
    }

    fn script_observe(mut self, outcomes: impl IntoIterator<Item = FakeObserveOutcome>) -> Self {
        self.observe_outcomes = outcomes.into_iter().collect();
        self
    }

    fn script_terminate(
        mut self,
        outcomes: impl IntoIterator<Item = Result<u32, FakeProcessError>>,
    ) -> Self {
        self.terminate_outcomes = outcomes.into_iter().collect();
        self
    }

    fn script_cleanup(
        mut self,
        outcomes: impl IntoIterator<Item = Result<(), FakeProcessError>>,
    ) -> Self {
        self.cleanup_outcomes = outcomes.into_iter().collect();
        self
    }
}

impl ProcessBackend for FakeBackend {
    type RunningOwner = u32;
    type CleanupOwner = u32;
    type Error = FakeProcessError;

    fn spawn(
        &mut self,
        _spec: ProcessLaunchSpec,
        deadline: Instant,
    ) -> Result<Self::RunningOwner, BackendSpawnFailure<Self::CleanupOwner, Self::Error>> {
        self.spawn_count += 1;
        self.spawn_deadlines.push(deadline);
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
        self.observe_count += 1;
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
        self.action_log.push(FakeAction::Terminate {
            owner: *owner,
            deadline,
        });
        self.terminate_outcomes
            .pop_front()
            .unwrap_or(Ok(0))
            .map(|exit_code| RootExit::for_test(self.birth_id(owner), exit_code))
    }

    fn retry_cleanup(
        &mut self,
        owner: &mut Self::CleanupOwner,
        deadline: Instant,
    ) -> Result<(), Self::Error> {
        self.cleanup_deadlines.push(deadline);
        self.action_log.push(FakeAction::Cleanup {
            owner: *owner,
            deadline,
        });
        self.cleanup_outcomes.pop_front().unwrap_or(Ok(()))
    }
}

struct FixedDeadlineSource(Instant);

impl DeadlineSource for FixedDeadlineSource {
    fn now(&mut self) -> Instant {
        self.0
    }
}

struct ScriptedDeadlineSource(VecDeque<Instant>);

impl DeadlineSource for ScriptedDeadlineSource {
    fn now(&mut self) -> Instant {
        self.0
            .pop_front()
            .expect("manager requested an unexpected deadline base")
    }
}

#[test]
fn public_error_rendering_is_bounded() {
    let view = ManagerSessionView {
        session_id: SESSION_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        profile_revision: PROFILE_REVISION,
        runtime_id: RUNTIME_ID.to_owned(),
        state: ManagerSessionState::CleanupPending,
    };
    let error = ManagerError::new(ManagerErrorCode::CloseIncomplete, ManagerOperation::Close)
        .with_remaining_sessions(vec![view]);

    assert_eq!(error.code().as_str(), "close_incomplete");
    assert_eq!(error.operation().as_str(), "close");
    assert_eq!(error.profile_id(), None);
    assert_eq!(error.session_id(), None);
    assert_eq!(error.expected_revision(), None);
    assert_eq!(error.actual_revision(), None);
    assert_eq!(error.maximum(), None);
    assert!(error.retained_session().is_none());

    let remaining = error.remaining_sessions();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].session_id, SESSION_ID);
    assert_eq!(remaining[0].profile_id, PROFILE_ID);
    assert_eq!(remaining[0].profile_revision, PROFILE_REVISION);
    assert_eq!(remaining[0].runtime_id, RUNTIME_ID);
    assert!(matches!(
        remaining[0].state,
        ManagerSessionState::CleanupPending
    ));

    assert!(
        error.source().is_none(),
        "public error must not expose an internal source"
    );

    for render in [error.to_string(), format!("{error:?}")] {
        assert!(
            render.len() < MAX_RENDER_BYTES,
            "render must stay under {MAX_RENDER_BYTES} bytes, got {}",
            render.len()
        );
        assert!(
            render.contains(ManagerErrorCode::CloseIncomplete.as_str()),
            "render must contain the stable code: {render}"
        );
        let without_code = render.replace(ManagerErrorCode::CloseIncomplete.as_str(), "");
        assert!(
            without_code.contains(ManagerOperation::Close.as_str()),
            "render must name the operation independently of the code: {render}"
        );
    }
}

#[test]
fn core_error_mapping_drops_raw_io_source() {
    const RAW_SENTINEL: &str = r"D:\private\operator\secret-runtime";
    let core_error = CoreError::io("read secret runtime path", io::Error::other(RAW_SENTINEL));

    let error = ManagerError::from_core(core_error, ManagerOperation::ListRuntimes);

    assert_eq!(error.code(), ManagerErrorCode::StorageFailure);
    assert_eq!(error.operation(), ManagerOperation::ListRuntimes);
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(RAW_SENTINEL));
    assert!(!format!("{error:?}").contains(RAW_SENTINEL));
}

#[test]
fn successful_start_returns_redacted_running_view_with_fixed_deadline() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-success");
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );

    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start fake manager session");

    assert_eq!(started.profile_id, profile.profile_id);
    assert_eq!(started.profile_revision, profile.revision);
    assert_eq!(started.runtime_id, profile.runtime_id);
    assert_eq!(started.state, ManagerSessionState::Running);
    assert_eq!(manager.list_sessions(), vec![started.clone()]);
    let page = manager
        .list_profiles(None, 100, false)
        .expect("list active profile");
    assert_eq!(
        page.items[0].active_session_id.as_deref(),
        Some(started.session_id.as_str())
    );
    let backend = manager.supervisor().backend();
    assert_eq!(backend.spawn_count, 1);
    assert_eq!(
        backend.spawn_deadlines,
        vec![base + Duration::from_secs(10)]
    );

    let rendered = format!("{started:?}");
    assert!(!rendered.contains("10001"));
    assert!(!rendered.contains("os_code"));
}

#[test]
fn start_preflight_failures_do_not_spawn() {
    let PreparedCore {
        mut core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-preflight");
    let archived = core
        .create_profile("Archived", &profile.runtime_id)
        .expect("create archived manager profile");
    let archived = core
        .archive_profile(&archived.profile_id, archived.revision)
        .expect("archive manager profile");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([]),
        FixedDeadlineSource(Instant::now()),
    );

    let invalid = manager
        .start_profile("not-a-uuid", profile.revision)
        .expect_err("invalid profile ID must fail");
    assert_eq!(invalid.code(), ManagerErrorCode::InvalidProfileId);

    let missing_id = Uuid::new_v4().hyphenated().to_string();
    let missing = manager
        .start_profile(&missing_id, 1)
        .expect_err("missing profile must fail");
    assert_eq!(missing.code(), ManagerErrorCode::ProfileNotFound);
    assert_eq!(missing.profile_id(), Some(missing_id.as_str()));

    let stale = manager
        .start_profile(&profile.profile_id, profile.revision + 1)
        .expect_err("stale profile revision must fail");
    assert_eq!(stale.code(), ManagerErrorCode::RevisionConflict);
    assert_eq!(stale.expected_revision(), Some(profile.revision + 1));
    assert_eq!(stale.actual_revision(), Some(profile.revision));

    let archived_error = manager
        .start_profile(&archived.profile_id, archived.revision)
        .expect_err("archived profile must fail");
    assert_eq!(archived_error.code(), ManagerErrorCode::ProfileArchived);
    assert_eq!(manager.supervisor().backend().spawn_count, 0);
    assert!(manager.list_sessions().is_empty());
}

#[test]
fn active_profile_is_rejected_with_original_redacted_session() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-active-profile");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(Instant::now()),
    );
    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start first profile session");

    let error = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("active profile must be rejected");

    assert_eq!(error.code(), ManagerErrorCode::ProfileAlreadyActive);
    assert_eq!(error.profile_id(), Some(profile.profile_id.as_str()));
    assert_eq!(error.session_id(), Some(started.session_id.as_str()));
    assert_eq!(manager.supervisor().backend().spawn_count, 1);
    assert_eq!(manager.list_sessions(), vec![started]);
}

#[test]
fn four_profiles_are_admitted_and_fifth_is_rejected_before_spawn() {
    let PreparedCore {
        mut core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-capacity");
    let mut profiles = vec![profile];
    for index in 2..=5 {
        profiles.push(
            core.create_profile(
                &format!("Capacity profile {index}"),
                &profiles[0].runtime_id,
            )
            .expect("create capacity profile"),
        );
    }
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
        ]),
        FixedDeadlineSource(Instant::now()),
    );
    for profile in &profiles[..4] {
        manager
            .start_profile(&profile.profile_id, profile.revision)
            .expect("admit capacity session");
    }

    let error = manager
        .start_profile(&profiles[4].profile_id, profiles[4].revision)
        .expect_err("fifth session must exceed capacity");

    assert_eq!(error.code(), ManagerErrorCode::CapacityReached);
    assert_eq!(error.maximum(), Some(4));
    assert_eq!(manager.supervisor().backend().spawn_count, 4);
    let sessions = manager.list_sessions();
    assert_eq!(sessions.len(), 4);
    assert!(
        sessions
            .windows(2)
            .all(|pair| pair[0].session_id < pair[1].session_id)
    );
}

#[test]
fn rejected_spawn_is_redacted_and_retains_no_session() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-rejected");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Reject(FakeProcessError::Rejected)]),
        FixedDeadlineSource(Instant::now()),
    );

    let error = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("fake spawn rejection");

    assert_eq!(error.code(), ManagerErrorCode::StartRejected);
    assert!(error.session_id().is_some());
    assert!(manager.list_sessions().is_empty());
    assert!(!error.to_string().contains("private"));
    assert!(!format!("{error:?}").contains("os_code"));
}

#[test]
fn start_deadline_maps_without_platform_detail() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-deadline");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Reject(FakeProcessError::DeadlineExpired)]),
        FixedDeadlineSource(Instant::now()),
    );

    let error = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("fake start deadline");

    assert_eq!(error.code(), ManagerErrorCode::StartDeadlineExpired);
    assert!(!error.to_string().contains("5678"));
    assert!(manager.list_sessions().is_empty());
}

#[test]
fn cleanup_pending_start_returns_and_retains_redacted_owner() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("start-cleanup-pending");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected)]),
        FixedDeadlineSource(Instant::now()),
    );

    let error = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("unconfirmed startup cleanup must fail with retained owner");

    assert_eq!(error.code(), ManagerErrorCode::CleanupPending);
    let retained = error
        .retained_session()
        .expect("cleanup-pending error must carry retained view");
    assert_eq!(
        retained.session_id,
        error.session_id().expect("retained ID")
    );
    assert_eq!(retained.profile_id, profile.profile_id);
    assert_eq!(retained.state, ManagerSessionState::CleanupPending);
    assert_eq!(manager.list_sessions(), vec![retained.clone()]);
    let page = manager
        .list_profiles(None, 100, false)
        .expect("list cleanup-pending profile");
    assert_eq!(
        page.items[0].active_session_id.as_deref(),
        Some(retained.session_id.as_str())
    );

    let repeated = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("cleanup-pending profile remains admitted");
    assert_eq!(repeated.code(), ManagerErrorCode::ProfileAlreadyActive);
    assert_eq!(manager.supervisor().backend().spawn_count, 1);
}

#[test]
fn session_collision_maps_to_stable_redacted_code() {
    let PreparedCore {
        core,
        _runtime,
        _data_root,
        ..
    } = prepared_core_with_profile("start-collision-map");
    let manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([]),
        FixedDeadlineSource(Instant::now()),
    );
    let session_id = Uuid::new_v4();

    let error = manager.map_start_failure(StartSessionFailure::SessionIdCollision { session_id });

    assert_eq!(error.code(), ManagerErrorCode::SessionCollision);
    assert_eq!(
        error.session_id(),
        Some(session_id.hyphenated().to_string().as_str())
    );
    assert!(error.source().is_none());
}

#[test]
fn running_and_cleanup_pending_observations_are_redacted_and_retained() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("observe-retained");
    let mut running = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept])
            .script_observe([FakeObserveOutcome::Running]),
        FixedDeadlineSource(Instant::now()),
    );
    let started = running
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start running observation owner");

    let observation = running
        .observe_session(&started.session_id)
        .expect("observe running owner");

    assert_eq!(observation, ManagerObservation::Running(started.clone()));
    assert_eq!(running.list_sessions(), vec![started]);
    assert_eq!(running.supervisor().backend().observe_count, 1);
    let rendered = format!("{observation:?}");
    assert!(!rendered.contains("birth_id"));
    assert!(!rendered.contains("exit_code"));

    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("observe-cleanup-pending");
    let mut cleanup_pending = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected)]),
        FixedDeadlineSource(Instant::now()),
    );
    let retained = cleanup_pending
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("create cleanup-pending owner")
        .retained_session()
        .expect("retained cleanup view")
        .clone();

    let observation = cleanup_pending
        .observe_session(&retained.session_id)
        .expect("observe cleanup-pending owner");

    assert_eq!(
        observation,
        ManagerObservation::CleanupPending(retained.clone())
    );
    assert_eq!(cleanup_pending.list_sessions(), vec![retained]);
    assert_eq!(cleanup_pending.supervisor().backend().observe_count, 0);
}

#[test]
fn exited_observation_is_redacted_and_delivered_once() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("observe-exited-once");
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept])
            .script_observe([FakeObserveOutcome::Exited(4_294_967_291)])
            .script_terminate([Ok(4_294_967_279)]),
        FixedDeadlineSource(base),
    );
    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start terminal observation owner");

    let observation = manager
        .observe_session(&started.session_id)
        .expect("observe confirmed terminal owner");

    let ManagerObservation::Exited(exited) = &observation else {
        panic!("expected a redacted exited observation");
    };
    assert_eq!(exited.session_id, started.session_id);
    assert_eq!(exited.profile_id, started.profile_id);
    assert_eq!(exited.profile_revision, started.profile_revision);
    assert_eq!(exited.runtime_id, started.runtime_id);
    assert!(manager.list_sessions().is_empty());
    assert_eq!(
        manager.supervisor().backend().terminate_deadlines,
        vec![base + Duration::from_secs(10)]
    );
    let rendered = format!("{observation:?}");
    assert!(!rendered.contains("4294967291"));
    assert!(!rendered.contains("4294967279"));
    assert!(!rendered.contains("10001"));

    let repeated = manager
        .observe_session(&started.session_id)
        .expect_err("terminal observation must be delivered only once");
    assert_eq!(repeated.code(), ManagerErrorCode::SessionNotFound);
    assert_eq!(repeated.session_id(), Some(started.session_id.as_str()));
}

#[test]
fn failed_terminal_cleanup_is_redacted_retained_and_retryable() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("observe-terminal-retry");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept])
            .script_observe([
                FakeObserveOutcome::Exited(41),
                FakeObserveOutcome::Exited(42),
            ])
            .script_terminate([Err(FakeProcessError::TerminateFailed), Ok(43)]),
        FixedDeadlineSource(Instant::now()),
    );
    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start terminal retry owner");

    let failed = manager
        .observe_session(&started.session_id)
        .expect_err("unconfirmed terminal cleanup must retain ownership");

    assert_eq!(failed.code(), ManagerErrorCode::ProcessFailure);
    assert_eq!(failed.operation(), ManagerOperation::ObserveSession);
    assert_eq!(failed.session_id(), Some(started.session_id.as_str()));
    assert!(!failed.to_string().contains("private"));
    assert!(!format!("{failed:?}").contains("3344"));
    assert_eq!(manager.list_sessions(), vec![started.clone()]);

    assert!(matches!(
        manager
            .observe_session(&started.session_id)
            .expect("retry terminal cleanup"),
        ManagerObservation::Exited(_)
    ));
    assert!(manager.list_sessions().is_empty());
}

#[test]
fn observe_rejects_invalid_noncanonical_and_unknown_session_ids_before_backend() {
    let PreparedCore {
        core,
        _runtime,
        _data_root,
        ..
    } = prepared_core_with_profile("observe-id-validation");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([]),
        FixedDeadlineSource(Instant::now()),
    );

    for invalid in ["not-a-uuid", &SESSION_ID.to_ascii_uppercase()] {
        let error = manager
            .observe_session(invalid)
            .expect_err("invalid session ID must fail");
        assert_eq!(error.code(), ManagerErrorCode::InvalidSessionId);
        assert_eq!(error.operation(), ManagerOperation::ObserveSession);
        assert_eq!(error.session_id(), None);
    }

    let unknown_id = Uuid::new_v4().hyphenated().to_string();
    let unknown = manager
        .observe_session(&unknown_id)
        .expect_err("unknown canonical session ID must fail");
    assert_eq!(unknown.code(), ManagerErrorCode::SessionNotFound);
    assert_eq!(unknown.session_id(), Some(unknown_id.as_str()));
    assert_eq!(manager.supervisor().backend().observe_count, 0);
}

#[test]
fn stop_failure_retains_owner_and_consecutive_retry_uses_fresh_deadlines() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("stop-retry");
    let bases = [Instant::now(), Instant::now(), Instant::now()];
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept])
            .script_terminate([Err(FakeProcessError::TerminateFailed), Ok(4_294_967_267)]),
        ScriptedDeadlineSource(bases.into()),
    );
    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start stop retry owner");

    let failed = manager
        .stop_session(&started.session_id)
        .expect_err("failed stop must retain owner");

    assert_eq!(failed.code(), ManagerErrorCode::ProcessFailure);
    assert_eq!(failed.operation(), ManagerOperation::StopSession);
    assert_eq!(manager.list_sessions(), vec![started.clone()]);

    let exited = manager
        .stop_session(&started.session_id)
        .expect("retry stop owner");
    assert_eq!(exited.session_id, started.session_id);
    assert_eq!(exited.profile_id, started.profile_id);
    assert_eq!(exited.profile_revision, started.profile_revision);
    assert_eq!(exited.runtime_id, started.runtime_id);
    assert!(manager.list_sessions().is_empty());
    assert_eq!(
        manager.supervisor().backend().terminate_deadlines,
        vec![
            bases[1] + Duration::from_secs(10),
            bases[2] + Duration::from_secs(10),
        ]
    );
    let rendered = format!("{exited:?}");
    assert!(!rendered.contains("4294967267"));
    assert!(!rendered.contains("10001"));
}

#[test]
fn stop_cleanup_pending_and_retry_running_report_wrong_state() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("wrong-state-running");
    let mut running = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(Instant::now()),
    );
    let started = running
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start running wrong-state owner");
    let retry_error = running
        .retry_cleanup(&started.session_id)
        .expect_err("running owner cannot use cleanup retry");
    assert_eq!(retry_error.code(), ManagerErrorCode::WrongSessionState);
    assert_eq!(running.list_sessions(), vec![started]);

    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("wrong-state-cleanup");
    let mut cleanup = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected)]),
        FixedDeadlineSource(Instant::now()),
    );
    let retained = cleanup
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("create cleanup wrong-state owner")
        .retained_session()
        .expect("retained cleanup owner")
        .clone();
    let stop_error = cleanup
        .stop_session(&retained.session_id)
        .expect_err("cleanup-pending owner cannot be stopped as running");
    assert_eq!(stop_error.code(), ManagerErrorCode::WrongSessionState);
    assert_eq!(cleanup.list_sessions(), vec![retained]);
}

#[test]
fn cleanup_retry_failure_retains_owner_then_success_removes_it_with_fresh_deadlines() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("cleanup-retry");
    let bases = [Instant::now(), Instant::now(), Instant::now()];
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected)])
            .script_cleanup([Err(FakeProcessError::CleanupFailed), Ok(())]),
        ScriptedDeadlineSource(bases.into()),
    );
    let retained = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect_err("create retryable cleanup owner")
        .retained_session()
        .expect("cleanup-pending view")
        .clone();

    let failed = manager
        .retry_cleanup(&retained.session_id)
        .expect_err("failed cleanup retry must retain owner");

    assert_eq!(failed.code(), ManagerErrorCode::ProcessFailure);
    assert_eq!(failed.operation(), ManagerOperation::RetryCleanup);
    assert_eq!(failed.session_id(), Some(retained.session_id.as_str()));
    assert!(!failed.to_string().contains("private"));
    assert_eq!(manager.list_sessions(), vec![retained.clone()]);

    manager
        .retry_cleanup(&retained.session_id)
        .expect("retry retained cleanup owner");
    assert!(manager.list_sessions().is_empty());
    assert_eq!(
        manager.supervisor().backend().cleanup_deadlines,
        vec![
            bases[1] + Duration::from_secs(10),
            bases[2] + Duration::from_secs(10),
        ]
    );
}

#[test]
fn lifecycle_errors_map_without_platform_detail_and_retain_owner() {
    let PreparedCore {
        core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("lifecycle-deadline");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]).script_observe([
            FakeObserveOutcome::Error(FakeProcessError::ObserveFailed),
            FakeObserveOutcome::Error(FakeProcessError::DeadlineExpired),
        ]),
        FixedDeadlineSource(Instant::now()),
    );
    let started = manager
        .start_profile(&profile.profile_id, profile.revision)
        .expect("start lifecycle deadline owner");

    let process_error = manager
        .observe_session(&started.session_id)
        .expect_err("fake lifecycle failure");

    assert_eq!(process_error.code(), ManagerErrorCode::ProcessFailure);
    assert_eq!(process_error.operation(), ManagerOperation::ObserveSession);
    assert!(!process_error.to_string().contains("private"));
    assert!(!format!("{process_error:?}").contains("2233"));
    assert_eq!(manager.list_sessions(), vec![started.clone()]);

    let deadline_error = manager
        .observe_session(&started.session_id)
        .expect_err("fake lifecycle deadline");

    assert_eq!(
        deadline_error.code(),
        ManagerErrorCode::ProcessDeadlineExpired
    );
    assert_eq!(deadline_error.operation(), ManagerOperation::ObserveSession);
    assert!(!deadline_error.to_string().contains("private"));
    assert!(!format!("{deadline_error:?}").contains("5678"));
    assert_eq!(manager.list_sessions(), vec![started]);
}

#[test]
fn close_empty_controller_is_closed_and_idempotent() {
    let PreparedCore {
        core,
        _runtime,
        _data_root,
        ..
    } = prepared_core_with_profile("close-empty");
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([]),
        FixedDeadlineSource(Instant::now()),
    );

    manager.close().expect("close empty controller");
    manager.close().expect("repeat closed close");

    assert!(manager.list_sessions().is_empty());
    assert!(manager.supervisor().backend().action_log.is_empty());
}

#[test]
fn close_continues_once_per_owner_and_is_retryable() {
    let PreparedCore {
        mut core,
        profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("close-retryable");
    let mut profiles = vec![profile];
    for index in 2..=4 {
        profiles.push(
            core.create_profile(&format!("Close profile {index}"), &profiles[0].runtime_id)
                .expect("create close profile"),
        );
    }
    let origin = Instant::now();
    let bases = (0_u64..10)
        .map(|offset| origin + Duration::from_secs(offset))
        .collect::<Vec<_>>();
    let backend = FakeBackend::with_outcomes([
        FakeSpawnOutcome::Accept,
        FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected),
        FakeSpawnOutcome::Accept,
        FakeSpawnOutcome::CleanupPending(FakeProcessError::Rejected),
    ])
    .script_terminate([Err(FakeProcessError::TerminateFailed), Ok(31)])
    .script_cleanup([Ok(()), Err(FakeProcessError::CleanupFailed)]);
    let mut manager = ManagerCore::new(core, backend, ScriptedDeadlineSource(bases.clone().into()));

    let mut owners = Vec::new();
    for (index, profile) in profiles.iter().enumerate() {
        let result = manager.start_profile(&profile.profile_id, profile.revision);
        let view = match index {
            0 | 2 => result.expect("start running close owner"),
            1 | 3 => result
                .expect_err("create cleanup-pending close owner")
                .retained_session()
                .expect("retained close owner")
                .clone(),
            _ => unreachable!(),
        };
        owners.push((view, u32::try_from(index + 1).expect("small fake owner")));
    }
    let mut sorted_owners = owners.clone();
    sorted_owners.sort_by(|left, right| left.0.session_id.cmp(&right.0.session_id));

    let first_error = manager
        .close()
        .expect_err("first close must retain failed owners");

    assert_eq!(first_error.code(), ManagerErrorCode::CloseIncomplete);
    assert_eq!(first_error.operation(), ManagerOperation::Close);
    assert!(first_error.source().is_none());
    assert!(!first_error.to_string().contains("private"));

    let backend = manager.supervisor().backend();
    let expected_first_actions = sorted_owners
        .iter()
        .enumerate()
        .map(|(action_index, (view, owner))| match view.state {
            ManagerSessionState::Running => FakeAction::Terminate {
                owner: *owner,
                deadline: bases[4 + action_index] + Duration::from_secs(10),
            },
            ManagerSessionState::CleanupPending => FakeAction::Cleanup {
                owner: *owner,
                deadline: bases[4 + action_index] + Duration::from_secs(10),
            },
        })
        .collect::<Vec<_>>();
    assert_eq!(backend.action_log, expected_first_actions);

    let first_running = sorted_owners
        .iter()
        .find(|(view, _)| view.state == ManagerSessionState::Running)
        .expect("first sorted running owner")
        .0
        .clone();
    let last_cleanup = sorted_owners
        .iter()
        .rev()
        .find(|(view, _)| view.state == ManagerSessionState::CleanupPending)
        .expect("last sorted cleanup owner")
        .0
        .clone();
    let mut expected_remaining = vec![first_running, last_cleanup];
    expected_remaining.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    assert_eq!(first_error.remaining_sessions(), expected_remaining);
    assert_eq!(manager.list_sessions(), expected_remaining);

    let starts_before = manager.supervisor().backend().spawn_count;
    let closing_start = manager
        .start_profile("not-a-uuid", -1)
        .expect_err("closing controller must reject start before preflight");
    assert_eq!(closing_start.code(), ManagerErrorCode::ControllerClosing);
    assert_eq!(closing_start.operation(), ManagerOperation::StartProfile);
    assert_eq!(manager.supervisor().backend().spawn_count, starts_before);
    manager
        .list_profiles(None, 100, false)
        .expect("catalog remains readable while closing");

    manager
        .close()
        .expect("second close removes retained owners");

    let backend = manager.supervisor().backend();
    assert_eq!(backend.action_log.len(), 6);
    let expected_retry_ids = expected_remaining
        .iter()
        .map(|view| {
            owners
                .iter()
                .find(|(owner_view, _)| owner_view.session_id == view.session_id)
                .expect("remaining owner mapping")
        })
        .enumerate()
        .map(|(action_index, (view, owner))| match view.state {
            ManagerSessionState::Running => FakeAction::Terminate {
                owner: *owner,
                deadline: bases[8 + action_index] + Duration::from_secs(10),
            },
            ManagerSessionState::CleanupPending => FakeAction::Cleanup {
                owner: *owner,
                deadline: bases[8 + action_index] + Duration::from_secs(10),
            },
        })
        .collect::<Vec<_>>();
    assert_eq!(&backend.action_log[4..], expected_retry_ids);
    assert!(manager.list_sessions().is_empty());

    for error in [
        manager
            .list_profiles(None, 0, false)
            .expect_err("closed profile catalog"),
        manager
            .list_runtimes(None, 0)
            .expect_err("closed runtime catalog"),
        manager
            .start_profile("not-a-uuid", -1)
            .expect_err("closed start"),
        manager
            .observe_session("not-a-uuid")
            .expect_err("closed observe"),
        manager.stop_session("not-a-uuid").expect_err("closed stop"),
        manager
            .retry_cleanup("not-a-uuid")
            .expect_err("closed cleanup retry"),
    ] {
        assert_eq!(error.code(), ManagerErrorCode::ControllerClosed);
    }
    manager.close().expect("closed close remains idempotent");
    assert_eq!(manager.supervisor().backend().action_log.len(), 6);
}

/// Imports `count` accounts into a prepared Core, returning their public ids.
#[cfg(windows)]
fn imported_accounts(core: &mut CoreState, count: usize) -> Vec<super::ManagerAccountId> {
    (0..count)
        .map(|index| {
            let stored = core
                .create_account_with_profile(
                    &format!("RunUser{index}"),
                    crate::credential_vault::SecretBytes::new(b"Secret-1".to_vec()),
                )
                .expect("import run account");
            super::ManagerAccountId::new(stored.account_id)
        })
        .collect()
}

#[cfg(windows)]
#[test]
fn account_run_starts_each_member_and_reports_per_row_outcomes() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-run-batch");
    let accounts = imported_accounts(&mut core, 2);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept, FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );

    let requests: Vec<(super::ManagerAccountId, i64)> =
        accounts.iter().map(|account| (*account, 1)).collect();
    let schedules = manager
        .run_accounts(&requests)
        .expect("a two-account batch is admitted");

    // Every member is scheduled, in submission order, and a real session now exists per member.
    assert_eq!(schedules.len(), 2);
    for (index, schedule) in schedules.iter().enumerate() {
        assert_eq!(schedule.account_id, accounts[index]);
        assert_eq!(
            schedule.outcome,
            super::ManagerRunScheduleOutcome::Scheduled,
            "member {index} must start"
        );
    }
    assert_eq!(manager.list_sessions().len(), 2);
}

#[cfg(windows)]
#[test]
fn account_run_rejects_a_member_that_is_already_running() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-run-busy");
    let accounts = imported_accounts(&mut core, 1);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept, FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );
    manager
        .run_accounts(&[(accounts[0], 1)])
        .expect("first run is admitted");

    let second = manager
        .run_accounts(&[(accounts[0], 1)])
        .expect("a second run is still a successful schedule result");

    // The account already holds a session, so it is refused rather than started twice.
    assert_eq!(
        second[0].outcome,
        super::ManagerRunScheduleOutcome::Rejected(super::ManagerRunRejection::AlreadyRunning)
    );
    assert_eq!(manager.list_sessions().len(), 1);
}

#[cfg(windows)]
#[test]
fn account_run_isolates_one_start_failure_from_its_siblings() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-run-isolation");
    let accounts = imported_accounts(&mut core, 2);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        // The first spawn is rejected, the second accepted.
        FakeBackend::with_outcomes([
            FakeSpawnOutcome::Reject(FakeProcessError::Rejected),
            FakeSpawnOutcome::Accept,
        ]),
        FixedDeadlineSource(base),
    );

    let schedules = manager
        .run_accounts(&[(accounts[0], 1), (accounts[1], 1)])
        .expect("the batch still returns a schedule result");

    // One member failing to start never aborts the batch or its sibling.
    assert_eq!(
        schedules[0].outcome,
        super::ManagerRunScheduleOutcome::Rejected(super::ManagerRunRejection::StartFailed)
    );
    assert_eq!(
        schedules[1].outcome,
        super::ManagerRunScheduleOutcome::Scheduled
    );
    assert_eq!(manager.list_sessions().len(), 1);
}

#[cfg(windows)]
#[test]
fn account_run_refuses_a_batch_beyond_the_session_ceiling() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-run-ceiling");
    let accounts = imported_accounts(&mut core, 5);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
            FakeSpawnOutcome::Accept,
        ]),
        FixedDeadlineSource(base),
    );

    let oversized: Vec<(super::ManagerAccountId, i64)> =
        accounts.iter().map(|account| (*account, 1)).collect();
    let error = manager
        .run_accounts(&oversized)
        .expect_err("a five-member batch is refused whole");
    assert_eq!(error.code(), ManagerErrorCode::CapacityReached);
    // Nothing started: the refusal happens before any profile start.
    assert!(manager.list_sessions().is_empty());

    // Four fit exactly, and a later fifth is rejected per row rather than whole.
    let schedules = manager
        .run_accounts(&oversized[..4])
        .expect("a four-member batch is admitted");
    assert!(
        schedules
            .iter()
            .all(|schedule| schedule.outcome == super::ManagerRunScheduleOutcome::Scheduled)
    );
    assert_eq!(manager.list_sessions().len(), 4);
    let fifth = manager
        .run_accounts(&[(accounts[4], 1)])
        .expect("a fifth run returns a schedule result");
    assert_eq!(
        fifth[0].outcome,
        super::ManagerRunScheduleOutcome::Rejected(super::ManagerRunRejection::TaskLimitReached)
    );
    assert_eq!(manager.list_sessions().len(), 4);
}

#[cfg(windows)]
#[test]
fn account_completion_persists_started_and_keeps_the_session() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-completion-started");
    let accounts = imported_accounts(&mut core, 1);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );
    manager
        .run_accounts(&[(accounts[0], 1)])
        .expect("run is admitted");
    let session = manager.list_sessions()[0].session_id.clone();
    let session_key = Uuid::parse_str(&session).expect("session id is a UUID");
    let epoch = manager
        .supervisor
        .session_epoch(session_key)
        .expect("a live session shares its epoch")
        .load(std::sync::atomic::Ordering::Acquire);

    let view = manager
        .apply_readiness_completion(accounts[0], session_key, epoch, true)
        .expect("a matching completion is accepted");

    // Admission is recorded and the session keeps running.
    assert_eq!(view.account_id, accounts[0]);
    assert_eq!(view.status, super::ManagerAccountStatus::Running);
    assert!(view.last_run_at_unix_ms.is_some());
    assert_eq!(manager.list_sessions().len(), 1);
}

#[cfg(windows)]
#[test]
fn account_completion_failure_stops_only_that_account() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-completion-failure");
    let accounts = imported_accounts(&mut core, 2);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept, FakeSpawnOutcome::Accept])
            .script_terminate([Ok(0)]),
        FixedDeadlineSource(base),
    );
    manager
        .run_accounts(&[(accounts[0], 1), (accounts[1], 1)])
        .expect("both runs are admitted");
    assert_eq!(manager.list_sessions().len(), 2);
    let failing = manager
        .account_session_id(accounts[0], super::ManagerOperation::RunAccounts)
        .expect("the failing account owns a session");
    let epoch = manager
        .supervisor
        .session_epoch(failing)
        .expect("a live session shares its epoch")
        .load(std::sync::atomic::Ordering::Acquire);

    let view = manager
        .apply_readiness_completion(accounts[0], failing, epoch, false)
        .expect("a failed completion is still accepted");

    // The failure is persisted and only that account's session is stopped.
    assert_eq!(view.status, super::ManagerAccountStatus::LoginFailed);
    let remaining = manager.list_sessions();
    assert_eq!(remaining.len(), 1, "the sibling session must keep running");
    assert!(
        manager
            .account_session_id(accounts[1], super::ManagerOperation::RunAccounts)
            .is_ok(),
        "the sibling account must still own its session"
    );
}

#[cfg(windows)]
#[test]
fn account_completion_drops_a_stale_epoch_or_session() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-completion-stale");
    let accounts = imported_accounts(&mut core, 1);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );
    manager
        .run_accounts(&[(accounts[0], 1)])
        .expect("run is admitted");
    let session_key = manager
        .account_session_id(accounts[0], super::ManagerOperation::RunAccounts)
        .expect("the account owns a session");
    let epoch = manager
        .supervisor
        .session_epoch(session_key)
        .expect("a live session shares its epoch")
        .load(std::sync::atomic::Ordering::Acquire);

    // A stale epoch means the session is being torn down: nothing is persisted.
    assert!(
        manager
            .apply_readiness_completion(accounts[0], session_key, epoch + 1, true)
            .is_none()
    );
    // A record naming a different session is also dropped.
    assert!(
        manager
            .apply_readiness_completion(accounts[0], Uuid::new_v4(), epoch, true)
            .is_none()
    );
    // An unknown account is dropped rather than panicking.
    assert!(
        manager
            .apply_readiness_completion(
                super::ManagerAccountId::new(Uuid::new_v4()),
                session_key,
                epoch,
                true
            )
            .is_none()
    );
    // The session is untouched by every dropped record.
    assert_eq!(manager.list_sessions().len(), 1);
}

#[cfg(windows)]
#[test]
fn account_player_snapshot_crosses_the_boundary_as_values_and_is_cleared_on_stop() {
    let PreparedCore {
        mut core,
        profile: _profile,
        _runtime,
        _data_root,
    } = prepared_core_with_profile("account-player-snapshot");
    let accounts = imported_accounts(&mut core, 1);
    let base = Instant::now();
    let mut manager = ManagerCore::new(
        core,
        FakeBackend::with_outcomes([FakeSpawnOutcome::Accept]),
        FixedDeadlineSource(base),
    );

    // Before a character is entered nothing has been published. That is ordinary, not a failure: a
    // read that errored here would light up the panel with a problem the operator cannot act on.
    assert_eq!(
        manager
            .observe_account_player(accounts[0])
            .expect("an absent snapshot is not a failure"),
        None
    );

    // The path is the mod's real destination: the same private `microemu-home` the launch argument
    // names, resolved from the account rather than supplied by the caller.
    let profile_id = manager
        .supervisor
        .core()
        .account_profile_id(accounts[0].get())
        .expect("the account owns a profile");
    let snapshot_path = manager
        .supervisor
        .core()
        .profile_directory(&profile_id)
        .expect("the profile directory is reachable")
        .join(crate::rms::MICROEMU_HOME_DIRECTORY)
        .join(crate::player::SNAPSHOT_FILE_NAME);
    fs::write(
        &snapshot_path,
        concat!(
            "v=6\nt=1788240611417\nname=Sentinel\nlv=80\nxp=105\n",
            "hp=45991\nhpmax=45991\nmp=9406\nmpmax=9406\n",
            "wallet=1\ngold=44916\ngem=0\nmap=1\nzone=13\npx=504\npy=264\n",
            "quota=30000\nbag=37\nbagmax=42\nstate=0\nmount=-1\nmounts=\nguild=\n",
            "xprate=0\nstale=0\nctl=1\n",
            "atkphase=1\natkstate=0\ntarget=1\nstuck=0\npotions=1\nrevives=0\n",
            "pkrank=1\npkmphp=0\npkgold=1\nbuffs=110\ndrops=------\n",
            "travel=0\ntravelwhy=0\ntravelgoal=-1\ntravelhops=0\n",
            // ---- ENHANCE ----
            "enhancephase=0\nenhancewhy=0\nenhancedone=0\n",
            // ---- end ENHANCE ----
            // The dungeon module's own keys, mirroring the live fixture in `player.rs`: the
            // snapshot parser requires them, so a body without them is rejected outright.
            "dungeonstate=0\ndungeonwhy=0\ndungeonruns=0\ndungeongoal=-1\n",
        ),
    )
    .expect("the profile snapshot is writable");

    let reading = manager
        .observe_account_player(accounts[0])
        .expect("a published snapshot reads")
        .expect("a published snapshot is present");
    assert_eq!(reading.character_name, "Sentinel");
    assert_eq!(reading.level, 80);
    // 105 permille is 10,5% of the current level, not 105 experience points.
    assert_eq!(reading.xp_permille, 105);
    assert_eq!(reading.gold, 44_916);
    assert!(reading.wallet_known);
    assert_eq!(reading.map_id, Some(1));
    assert_eq!(reading.hp_percent(), Some(100));
    // The modules report their own state, so a silent module is distinguishable from a working one.
    assert!(reading.auto_running());
    assert!(reading.has_target);
    assert_eq!(reading.potions, 1);
    // The pickup bytes and buff slots are read back out of the client, not echoed from what the
    // tool asked for. A boundary that dropped them would show the operator their own request and
    // call it the client's state, which is the one thing this panel exists to distinguish.
    assert_eq!(reading.pickup, Some((1, 0, 1)));
    assert_eq!(reading.buffs, [true, true, false]);

    // Nothing the boundary hands over may name the account, the profile, or a path on disk.
    let rendered = format!("{reading:?}");
    for forbidden in [
        profile_id.as_str(),
        "microemu-home",
        crate::player::SNAPSHOT_FILE_NAME,
        "RunUser0",
        r"\",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "the reading exposes {forbidden}"
        );
    }

    // A malformed reading is refused rather than half-rendered as fact.
    fs::write(&snapshot_path, "v=1\nlv=80\n").expect("the profile snapshot is rewritable");
    let error = manager
        .observe_account_player(accounts[0])
        .expect_err("a malformed snapshot fails");
    assert_eq!(
        error.code(),
        ManagerErrorCode::PlayerSnapshotUnreadable,
        "a parse failure must not be reported as something the operator can retry"
    );
    assert_eq!(error.operation(), ManagerOperation::ObserveAccountPlayer);
    // The failure names no file, no path, and no parse rule.
    let rendered = format!("{error:?}{error}");
    for forbidden in ["microemu-home", crate::player::SNAPSHOT_FILE_NAME, r"\"] {
        assert!(
            !rendered.contains(forbidden),
            "the failure exposes {forbidden}"
        );
    }

    // Stopping the account removes the reading. Without this the panel would keep showing a live
    // character for a session that has already exited.
    manager
        .run_accounts(&[(accounts[0], 1)])
        .expect("run is admitted");
    fs::write(&snapshot_path, "v=1\n").expect("the profile snapshot is rewritable");
    assert!(
        snapshot_path.exists(),
        "the fixture snapshot was not planted"
    );
    // The reconciled row, not a bare acknowledgement: the UI marks the row optimistically on submit,
    // and a stopped session publishes no further notification, so the stop's own answer is what
    // releases the row for another Run.
    let stopped = manager
        .stop_account(accounts[0])
        .expect("the account stops cleanly");
    assert_eq!(stopped.account_id, accounts[0]);
    assert_eq!(stopped.status, super::ManagerAccountStatus::Idle);
    assert!(
        !snapshot_path.exists(),
        "a stopped account left its character reading on disk"
    );
    assert_eq!(
        manager
            .observe_account_player(accounts[0])
            .expect("a cleared snapshot reads"),
        None
    );
}

/// The operator's actual complaint: rebuilding the jar used to force a data-root reset, and a reset
/// deletes every account. This is the end-to-end proof that it no longer does.
#[test]
fn rebuilding_the_game_jar_keeps_every_imported_account() {
    use super::{ManagerAccountPassword, ManagerController, ManagerWorkerOperation};

    let runtime = runtime_fixture::RuntimeFixture::new("repin-accounts");
    let runtime_root = fs::canonicalize(
        runtime
            .descriptor_path()
            .parent()
            .expect("fixture runtime root"),
    )
    .expect("canonicalize fixture runtime root");
    let data_root = TestDirectory::new("repin-accounts");

    let imported = {
        let mut core = CoreState::open_at(&data_root.0).expect("open Core");
        core.register_runtime_descriptor(runtime.descriptor_path())
            .expect("register the fixture runtime");
        let mut manager = ManagerController::from_core(core);
        let secret = ManagerAccountPassword::try_from_utf16(
            ManagerWorkerOperation::ImportAccount,
            "Secret-1".encode_utf16().collect(),
        )
        .expect("printable ASCII password is accepted");
        let account = manager
            .import_account("Farmer", secret)
            .expect("import an account before the rebuild");
        manager.close().expect("close the manager cleanly");
        account
    };

    runtime.rebuild_game_jar(b"fixture-game-402-rebuilt-for-the-account-test");

    let manager = ManagerController::open_portable_at(&data_root.0, &runtime_root)
        .expect("a rebuilt game jar must not block opening the data root");
    let accounts = manager
        .list_accounts()
        .expect("list accounts after the re-pin");
    assert_eq!(accounts.len(), 1, "the imported account was lost");
    assert_eq!(accounts[0].account_id, imported.account_id);
    assert_eq!(accounts[0].username, imported.username);
    assert_eq!(accounts[0].revision, imported.revision);
    assert_eq!(accounts[0].server_index, imported.server_index);
}

mod backend;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use uuid::{Uuid, Version};

use crate::process_adapter::{ProcessBirthId, WindowsProcessError};
use crate::{CoreError, CoreState, ProcessLaunchSpec};

pub(crate) use self::backend::{BackendSpawnFailure, ProcessBackend, WindowsProcessBackend};

pub(crate) const MAX_ACTIVE_SESSIONS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionMetadata {
    session_id: Uuid,
    profile_id: Uuid,
    profile_revision: i64,
    runtime_id: String,
    descriptor_sha256: String,
}

#[allow(
    dead_code,
    reason = "crate-private lifecycle results have no production caller in v1"
)]
impl SessionMetadata {
    pub(crate) fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub(crate) fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub(crate) fn profile_revision(&self) -> i64 {
        self.profile_revision
    }

    pub(crate) fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub(crate) fn descriptor_sha256(&self) -> &str {
        &self.descriptor_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionStarted {
    metadata: SessionMetadata,
    birth_id: ProcessBirthId,
}

#[allow(
    dead_code,
    reason = "crate-private lifecycle results have no production caller in v1"
)]
impl SessionStarted {
    pub(crate) fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub(crate) fn birth_id(&self) -> ProcessBirthId {
        self.birth_id
    }
}

enum SessionOwner<B: ProcessBackend> {
    Running(B::RunningOwner),
    CleanupPending(B::CleanupOwner),
}

struct SessionEntry<B: ProcessBackend> {
    metadata: SessionMetadata,
    owner: SessionOwner<B>,
    /// Bumped before any stop or cleanup so an in-flight readiness task observes a stale epoch and
    /// abandons its work instead of acting on a session that is going away.
    epoch: Arc<AtomicU64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionObservation {
    Running {
        metadata: SessionMetadata,
        birth_id: ProcessBirthId,
    },
    CleanupPending {
        metadata: SessionMetadata,
    },
    Exited(SessionExit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionExit {
    metadata: SessionMetadata,
    birth_id: ProcessBirthId,
    exit_code: u32,
}

#[allow(
    dead_code,
    reason = "crate-private lifecycle results have no production caller in v1"
)]
impl SessionExit {
    pub(crate) fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub(crate) fn birth_id(&self) -> ProcessBirthId {
        self.birth_id
    }

    pub(crate) fn exit_code(&self) -> u32 {
        self.exit_code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStateKind {
    Running,
    CleanupPending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionSummary {
    metadata: SessionMetadata,
    state: SessionStateKind,
}

impl SessionSummary {
    pub(crate) fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub(crate) fn state(&self) -> SessionStateKind {
        self.state
    }
}

#[derive(Debug)]
pub(crate) enum SessionFailure<E> {
    SessionNotFound {
        session_id: Uuid,
    },
    WrongState {
        session_id: Uuid,
        expected: SessionStateKind,
        actual: SessionStateKind,
    },
    Process {
        session_id: Uuid,
        error: E,
    },
    InvariantViolation {
        code: &'static str,
    },
}

impl<E: fmt::Display> fmt::Display for SessionFailure<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound { session_id } => {
                write!(formatter, "SessionNotFound(session_id={session_id})")
            }
            Self::WrongState {
                session_id,
                expected,
                actual,
            } => write!(
                formatter,
                "WrongState(session_id={session_id}, expected={expected:?}, actual={actual:?})"
            ),
            Self::Process { session_id, error } => {
                write!(formatter, "Process(session_id={session_id}, error={error})")
            }
            Self::InvariantViolation { code } => {
                write!(formatter, "InvariantViolation(code={code})")
            }
        }
    }
}

impl<E> std::error::Error for SessionFailure<E> where E: std::error::Error + 'static {}

#[derive(Debug)]
pub(crate) enum StartSessionFailure<E> {
    Core(CoreError),
    ProfileAlreadyActive { profile_id: Uuid, session_id: Uuid },
    CapacityReached { maximum: usize },
    SessionIdCollision { session_id: Uuid },
    SpawnRejected { session_id: Uuid, error: E },
    CleanupPending { session_id: Uuid, error: E },
}

impl<E: fmt::Display> fmt::Display for StartSessionFailure<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(formatter, "Core: {error}"),
            Self::ProfileAlreadyActive {
                profile_id,
                session_id,
            } => write!(
                formatter,
                "ProfileAlreadyActive(profile_id={profile_id}, session_id={session_id})"
            ),
            Self::CapacityReached { maximum } => {
                write!(formatter, "CapacityReached(maximum={maximum})")
            }
            Self::SessionIdCollision { session_id } => {
                write!(formatter, "SessionIdCollision(session_id={session_id})")
            }
            Self::SpawnRejected { session_id, error } => {
                write!(
                    formatter,
                    "SpawnRejected(session_id={session_id}, error={error})"
                )
            }
            Self::CleanupPending { session_id, error } => {
                write!(
                    formatter,
                    "CleanupPending(session_id={session_id}, error={error})"
                )
            }
        }
    }
}

impl<E> std::error::Error for StartSessionFailure<E> where E: std::error::Error + 'static {}

pub(crate) struct SupervisorCore<B: ProcessBackend> {
    sessions: HashMap<Uuid, SessionEntry<B>>,
    profile_index: HashMap<Uuid, Uuid>,
    backend: B,
    core: CoreState,
}

impl<B: ProcessBackend> SupervisorCore<B> {
    pub(crate) fn new(core: CoreState, backend: B) -> Self {
        Self {
            sessions: HashMap::new(),
            profile_index: HashMap::new(),
            backend,
            core,
        }
    }

    pub(crate) fn start_session(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
        deadline: Instant,
    ) -> Result<SessionStarted, StartSessionFailure<B::Error>> {
        let profile_id = parse_profile_id(profile_id).map_err(StartSessionFailure::Core)?;
        self.check_profile_admission(profile_id)?;

        let snapshot = self
            .core
            .prepare_launch_snapshot(&profile_id.to_string(), expected_revision)
            .map_err(StartSessionFailure::Core)?;
        let metadata = SessionMetadata {
            session_id: snapshot.session_id(),
            profile_id: snapshot.profile_id(),
            profile_revision: snapshot.profile_revision(),
            runtime_id: snapshot.runtime_id().to_owned(),
            descriptor_sha256: snapshot.descriptor_sha256().to_owned(),
        };
        self.check_session_collision(metadata.session_id)?;
        let spec = snapshot
            .process_launch_spec()
            .map_err(StartSessionFailure::Core)?;
        self.spawn_and_insert(metadata, spec, deadline)
    }

    fn check_profile_admission(
        &self,
        profile_id: Uuid,
    ) -> Result<(), StartSessionFailure<B::Error>> {
        if let Some(session_id) = self.profile_index.get(&profile_id).copied() {
            return Err(StartSessionFailure::ProfileAlreadyActive {
                profile_id,
                session_id,
            });
        }
        if self.sessions.len() >= MAX_ACTIVE_SESSIONS {
            return Err(StartSessionFailure::CapacityReached {
                maximum: MAX_ACTIVE_SESSIONS,
            });
        }
        Ok(())
    }

    fn check_session_collision(
        &self,
        session_id: Uuid,
    ) -> Result<(), StartSessionFailure<B::Error>> {
        if self.sessions.contains_key(&session_id) {
            return Err(StartSessionFailure::SessionIdCollision { session_id });
        }
        Ok(())
    }

    fn spawn_and_insert(
        &mut self,
        metadata: SessionMetadata,
        spec: ProcessLaunchSpec,
        deadline: Instant,
    ) -> Result<SessionStarted, StartSessionFailure<B::Error>> {
        let session_id = metadata.session_id;
        let profile_id = metadata.profile_id;
        match self.backend.spawn(spec, deadline) {
            Ok(owner) => {
                let birth_id = self.backend.birth_id(&owner);
                self.sessions.insert(
                    session_id,
                    SessionEntry {
                        metadata: metadata.clone(),
                        owner: SessionOwner::Running(owner),
                        epoch: Arc::new(AtomicU64::new(1)),
                    },
                );
                self.profile_index.insert(profile_id, session_id);
                Ok(SessionStarted { metadata, birth_id })
            }
            Err(BackendSpawnFailure::Rejected(error)) => {
                Err(StartSessionFailure::SpawnRejected { session_id, error })
            }
            Err(BackendSpawnFailure::CleanupUnconfirmed { error, owner }) => {
                self.sessions.insert(
                    session_id,
                    SessionEntry {
                        metadata,
                        owner: SessionOwner::CleanupPending(owner),
                        epoch: Arc::new(AtomicU64::new(1)),
                    },
                );
                self.profile_index.insert(profile_id, session_id);
                Err(StartSessionFailure::CleanupPending { session_id, error })
            }
        }
    }

    pub(crate) fn observe_session(
        &mut self,
        session_id: Uuid,
        cleanup_deadline: Instant,
    ) -> Result<SessionObservation, SessionFailure<B::Error>> {
        let terminal = {
            let Some(entry) = self.sessions.get_mut(&session_id) else {
                return Err(SessionFailure::SessionNotFound { session_id });
            };
            let metadata = entry.metadata.clone();
            match &mut entry.owner {
                SessionOwner::CleanupPending(_) => {
                    return Ok(SessionObservation::CleanupPending { metadata });
                }
                SessionOwner::Running(owner) => {
                    let birth_id = self.backend.birth_id(owner);
                    match self.backend.try_wait_root(owner) {
                        Ok(None) => {
                            return Ok(SessionObservation::Running { metadata, birth_id });
                        }
                        Ok(Some(_)) => self
                            .backend
                            .terminate_tree_and_wait(owner, cleanup_deadline)
                            .map(|root_exit| (metadata, root_exit))
                            .map_err(|error| SessionFailure::Process { session_id, error })?,
                        Err(error) => {
                            return Err(SessionFailure::Process { session_id, error });
                        }
                    }
                }
            }
        };

        self.remove_confirmed_exit(session_id, terminal.0, terminal.1)
            .map(SessionObservation::Exited)
    }

    pub(crate) fn stop_session(
        &mut self,
        session_id: Uuid,
        deadline: Instant,
    ) -> Result<SessionExit, SessionFailure<B::Error>> {
        // Invalidate before any cancellation, so no readiness task can act on this session while it
        // is being terminated.
        self.invalidate_session_epoch(session_id);
        let terminal = {
            let Some(entry) = self.sessions.get_mut(&session_id) else {
                return Err(SessionFailure::SessionNotFound { session_id });
            };
            let metadata = entry.metadata.clone();
            match &mut entry.owner {
                SessionOwner::Running(owner) => self
                    .backend
                    .terminate_tree_and_wait(owner, deadline)
                    .map(|root_exit| (metadata, root_exit))
                    .map_err(|error| SessionFailure::Process { session_id, error })?,
                SessionOwner::CleanupPending(_) => {
                    return Err(SessionFailure::WrongState {
                        session_id,
                        expected: SessionStateKind::Running,
                        actual: SessionStateKind::CleanupPending,
                    });
                }
            }
        };

        self.remove_confirmed_exit(session_id, terminal.0, terminal.1)
    }

    pub(crate) fn retry_cleanup(
        &mut self,
        session_id: Uuid,
        deadline: Instant,
    ) -> Result<(), SessionFailure<B::Error>> {
        // Cleanup retry also tears the session down, so stale-epoch invalidation applies here too.
        self.invalidate_session_epoch(session_id);
        let profile_id = {
            let Some(entry) = self.sessions.get_mut(&session_id) else {
                return Err(SessionFailure::SessionNotFound { session_id });
            };
            match &mut entry.owner {
                SessionOwner::Running(_) => {
                    return Err(SessionFailure::WrongState {
                        session_id,
                        expected: SessionStateKind::CleanupPending,
                        actual: SessionStateKind::Running,
                    });
                }
                SessionOwner::CleanupPending(owner) => {
                    self.backend
                        .retry_cleanup(owner, deadline)
                        .map_err(|error| SessionFailure::Process { session_id, error })?;
                    entry.metadata.profile_id
                }
            }
        };

        if self.profile_index.get(&profile_id) != Some(&session_id) {
            return Err(SessionFailure::InvariantViolation {
                code: "supervisor_profile_index_mismatch",
            });
        }
        self.profile_index.remove(&profile_id);
        self.sessions.remove(&session_id);
        Ok(())
    }

    fn remove_confirmed_exit(
        &mut self,
        session_id: Uuid,
        metadata: SessionMetadata,
        root_exit: crate::process_adapter::RootExit,
    ) -> Result<SessionExit, SessionFailure<B::Error>> {
        let profile_id = metadata.profile_id;
        if self.profile_index.get(&profile_id) != Some(&session_id) {
            return Err(SessionFailure::InvariantViolation {
                code: "supervisor_profile_index_mismatch",
            });
        }
        self.profile_index.remove(&profile_id);
        self.sessions.remove(&session_id);
        Ok(SessionExit {
            metadata,
            birth_id: root_exit.identity(),
            exit_code: root_exit.exit_code(),
        })
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    pub(crate) fn session_summaries(&self) -> Vec<SessionSummary> {
        self.sessions
            .values()
            .map(|entry| SessionSummary {
                metadata: entry.metadata.clone(),
                state: match entry.owner {
                    SessionOwner::Running(_) => SessionStateKind::Running,
                    SessionOwner::CleanupPending(_) => SessionStateKind::CleanupPending,
                },
            })
            .collect()
    }

    pub(crate) fn core(&self) -> &CoreState {
        &self.core
    }

    /// Resolves the live session owning one profile, if any.
    pub(crate) fn session_for_profile(&self, profile_id: Uuid) -> Option<Uuid> {
        self.profile_index.get(&profile_id).copied()
    }

    /// Shares one session's epoch handle with a readiness task.
    #[allow(
        dead_code,
        reason = "no production readiness task captures a session epoch yet"
    )]
    pub(crate) fn session_epoch(&self, session_id: Uuid) -> Option<Arc<AtomicU64>> {
        self.sessions
            .get(&session_id)
            .map(|entry| Arc::clone(&entry.epoch))
    }

    /// Invalidates one session's epoch, so every captured epoch for it becomes stale.
    ///
    /// Called before stop and before cleanup retry: a readiness task that captured the old value must
    /// abandon its work rather than act on a session that is being torn down.
    pub(crate) fn invalidate_session_epoch(&self, session_id: Uuid) {
        if let Some(entry) = self.sessions.get(&session_id) {
            entry.epoch.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Narrow mutable-Core seam for the enumerated ManagerController account operations only.
    ///
    /// Deliberately named so no generic mutable accessor is introduced: lifecycle state lives in
    /// `sessions`/`profile_index`, and account mutations must never bypass it.
    pub(crate) fn core_mut_for_account(&mut self) -> &mut CoreState {
        &mut self.core
    }

    #[cfg(test)]
    pub(crate) fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    pub(crate) fn running_birth_id_for_test(&self, session_id: Uuid) -> Option<ProcessBirthId> {
        let entry = self.sessions.get(&session_id)?;
        match &entry.owner {
            SessionOwner::Running(owner) => Some(self.backend.birth_id(owner)),
            SessionOwner::CleanupPending(_) => None,
        }
    }
}

fn parse_profile_id(value: &str) -> Result<Uuid, CoreError> {
    let parsed = Uuid::parse_str(value).map_err(|_| CoreError::InvalidProfileId)?;
    if parsed.get_version() != Some(Version::Random) || parsed.hyphenated().to_string() != value {
        return Err(CoreError::InvalidProfileId);
    }
    Ok(parsed)
}

#[allow(
    dead_code,
    reason = "Session Supervisor v1 is crate-private and has no public caller"
)]
pub(crate) struct SessionSupervisor {
    inner: SupervisorCore<WindowsProcessBackend>,
}

#[allow(
    dead_code,
    reason = "Session Supervisor v1 is crate-private and has no public caller"
)]
impl SessionSupervisor {
    pub(crate) fn new(core: CoreState) -> Self {
        Self {
            inner: SupervisorCore::new(core, WindowsProcessBackend),
        }
    }

    pub(crate) fn start_session(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
        deadline: Instant,
    ) -> Result<SessionStarted, StartSessionFailure<WindowsProcessError>> {
        self.inner
            .start_session(profile_id, expected_revision, deadline)
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.inner.active_session_count()
    }

    pub(crate) fn session_summaries(&self) -> Vec<SessionSummary> {
        self.inner.session_summaries()
    }

    pub(crate) fn observe_session(
        &mut self,
        session_id: Uuid,
        cleanup_deadline: Instant,
    ) -> Result<SessionObservation, SessionFailure<WindowsProcessError>> {
        self.inner.observe_session(session_id, cleanup_deadline)
    }

    pub(crate) fn stop_session(
        &mut self,
        session_id: Uuid,
        deadline: Instant,
    ) -> Result<SessionExit, SessionFailure<WindowsProcessError>> {
        self.inner.stop_session(session_id, deadline)
    }

    pub(crate) fn retry_cleanup(
        &mut self,
        session_id: Uuid,
        deadline: Instant,
    ) -> Result<(), SessionFailure<WindowsProcessError>> {
        self.inner.retry_cleanup(session_id, deadline)
    }

    pub(crate) fn core(&self) -> &CoreState {
        self.inner.core()
    }
}

#[cfg(test)]
impl SessionSupervisor {
    pub(super) fn start_spec_for_test(
        &mut self,
        spec: ProcessLaunchSpec,
        deadline: Instant,
    ) -> Result<SessionStarted, StartSessionFailure<WindowsProcessError>> {
        let metadata = SessionMetadata {
            session_id: spec.session_id(),
            profile_id: spec.profile_id(),
            profile_revision: 1,
            runtime_id: "test-windows-probe".to_owned(),
            descriptor_sha256: "0".repeat(64),
        };
        self.inner.check_profile_admission(metadata.profile_id)?;
        self.inner.check_session_collision(metadata.session_id)?;
        self.inner.spawn_and_insert(metadata, spec, deadline)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod windows_tests;

#[cfg(all(test, windows))]
mod windows_live_test_support;

#[cfg(all(test, windows))]
mod windows_live_runtime_tests;

#[cfg(all(test, windows))]
pub(crate) use self::windows_live_test_support::{
    LIVE_RUNTIME_ID, LiveGateFailure, LiveManagerCoreFixture, LiveTestDirectory, STOP_DEADLINE,
    WINDOW_READY_DEADLINE, prepare_live_manager_core_fixture, require_qualified_ready_window,
    wait_for_process_signal,
};

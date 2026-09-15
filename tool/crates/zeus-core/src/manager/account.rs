//! Redacted public account values for the Windows manager boundary.
//!
//! These types are the only account shape the native UI ever sees. They deliberately carry no
//! password, ciphertext, config, profile ID, session ID, runtime ID, or process identity.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use uuid::Uuid;

use crate::process_adapter::ProcessBirthId;

use crate::store::{StoredAccount, StoredAccountOutcome};

/// Opaque account handle retained by the UI as row model data.
///
/// It is `Copy + Eq + Ord + Hash` so a row model can key on it, and deliberately exposes no public
/// constructor, `Display`, or string accessor: the UI must never render or reconstruct an account
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManagerAccountId(Uuid);

impl ManagerAccountId {
    pub(crate) fn new(value: Uuid) -> Self {
        Self(value)
    }

    pub(crate) fn get(self) -> Uuid {
        self.0
    }
}

/// Reconciled account state. These four values are the complete public state space.
///
/// `Starting`, `Authenticating`, and `Stopping` are deliberately absent: they are UI-local pending
/// states derived from accepted commands, never persisted or reconciled here.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerAccountStatus {
    Idle,
    Running,
    LoginFailed,
    CleanupPending,
}

/// One account row as the UI sees it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerAccountView {
    pub account_id: ManagerAccountId,
    pub revision: i64,
    pub username: String,
    pub status: ManagerAccountStatus,
    pub last_run_at_unix_ms: Option<i64>,
    /// World this account logs into, as an index into the client's server table.
    pub server_index: u8,
}

/// Live session state of one account, supplied by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountSessionState {
    Running,
    CleanupPending,
}

impl ManagerAccountView {
    /// Projects a stored account, deriving status in the exact spec order.
    ///
    /// `CleanupPending` outranks `Running`, which outranks a persisted `LoginFailed`; anything else is
    /// `Idle`. A live session therefore always wins over a stale persisted failure.
    pub(crate) fn from_stored(
        account: &StoredAccount,
        session: Option<AccountSessionState>,
    ) -> Self {
        let status = match session {
            Some(AccountSessionState::CleanupPending) => ManagerAccountStatus::CleanupPending,
            Some(AccountSessionState::Running) => ManagerAccountStatus::Running,
            None => match account.last_outcome {
                Some(StoredAccountOutcome::LoginFailed) => ManagerAccountStatus::LoginFailed,
                Some(StoredAccountOutcome::Started) | None => ManagerAccountStatus::Idle,
            },
        };
        Self {
            account_id: ManagerAccountId::new(account.account_id),
            revision: account.revision,
            username: account.username.clone(),
            status,
            last_run_at_unix_ms: account.last_run_at_unix_ms,
            server_index: account.config.server_index,
        }
    }
}

/// Maximum accounts in one bounded Run batch (spec section 14).
pub const MAX_RUN_BATCH: usize = 4;

/// Why one account was not scheduled. Stable, redacted strings for the UI.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerRunRejection {
    /// The four-task ceiling was reached. Refused before profile start, secret decrypt, or task
    /// creation.
    TaskLimitReached,
    AlreadyRunning,
    StartFailed,
    /// The record stores the client reads at startup could not be written, so no process was launched.
    LoginSeedFailed,
}

impl ManagerRunRejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskLimitReached => "task_limit_reached",
            Self::AlreadyRunning => "already_running",
            Self::StartFailed => "start_failed",
            Self::LoginSeedFailed => "login_seed_failed",
        }
    }
}

/// One account's Run scheduling outcome.
///
/// `Scheduled` means the profile started and a readiness task was admitted. It is never a claim that
/// input was sent or that authentication succeeded.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerRunScheduleOutcome {
    Scheduled,
    Rejected(ManagerRunRejection),
}

/// Bounded per-account Run result carried by one request result.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerRunSchedule {
    pub account_id: ManagerAccountId,
    pub outcome: ManagerRunScheduleOutcome,
}

/// Private, non-cloneable authorization to run the fixed submit script against one live session.
///
/// It is deliberately not `Clone`: exactly one readiness task may hold it. It carries no Core handle,
/// no controller, and no credential, so a task holding it cannot mutate lifecycle state or persist
/// anything. Task 9 adds the coordinator that consumes it.
#[allow(
    dead_code,
    reason = "no production readiness task consumes a login target yet"
)]
pub(crate) struct LoginTarget {
    session_id: Uuid,
    birth_id: ProcessBirthId,
    expected_process_image: String,
    expected_window_class: String,
    epoch: Arc<AtomicU64>,
    captured_epoch: u64,
}

#[allow(
    dead_code,
    reason = "no production readiness task consumes a login target yet"
)]
impl LoginTarget {
    pub(crate) fn new(
        session_id: Uuid,
        birth_id: ProcessBirthId,
        expected_process_image: String,
        expected_window_class: String,
        epoch: Arc<AtomicU64>,
    ) -> Self {
        let captured_epoch = epoch.load(Ordering::Acquire);
        Self {
            session_id,
            birth_id,
            expected_process_image,
            expected_window_class,
            epoch,
            captured_epoch,
        }
    }

    /// Read-only probe surface. It can inspect and compare, never start, stop, or clean up.
    pub(crate) fn probe(&self) -> LoginTargetProbe<'_> {
        LoginTargetProbe { target: self }
    }
}

/// Named read-only view of a [`LoginTarget`].
#[allow(
    dead_code,
    reason = "no production readiness task consumes a login target yet"
)]
pub(crate) struct LoginTargetProbe<'target> {
    target: &'target LoginTarget,
}

#[allow(
    dead_code,
    reason = "no production readiness task consumes a login target yet"
)]
impl LoginTargetProbe<'_> {
    pub(crate) fn session_id(&self) -> Uuid {
        self.target.session_id
    }

    pub(crate) fn birth_id(&self) -> ProcessBirthId {
        self.target.birth_id
    }

    pub(crate) fn expected_process_image(&self) -> &str {
        &self.target.expected_process_image
    }

    pub(crate) fn expected_window_class(&self) -> &str {
        &self.target.expected_window_class
    }

    /// Reports whether this target still authorizes work.
    ///
    /// The worker bumps the epoch before any stop or cleanup, so a stale reading means the session is
    /// being torn down and the task must abandon its work without touching lifecycle state.
    pub(crate) fn is_current(&self) -> bool {
        self.target.epoch.load(Ordering::Acquire) == self.target.captured_epoch
    }
}

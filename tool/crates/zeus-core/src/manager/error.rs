//! Stable, redacted failure taxonomy for the Windows manager control boundary.

use std::error::Error;
use std::fmt;

use super::ManagerSessionView;
use crate::CoreError;

/// Result of every public manager operation.
pub type ManagerResult<T> = Result<T, ManagerError>;

/// Public operation that produced a [`ManagerError`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerOperation {
    Open,
    ListProfiles,
    ListRuntimes,
    StartProfile,
    ObserveSession,
    StopSession,
    RetryCleanup,
    ListAccounts,
    ImportAccount,
    UpdateAccount,
    SetAccountServer,
    DeleteAccount,
    RunAccounts,
    StopAccount,
    RetryAccountCleanup,
    ObserveAccountPlayer,
    SetAccountControl,
    ObserveSpots,
    SaveSpot,
    ClearSpot,
    Close,
}

impl ManagerOperation {
    /// Stable lowercase snake-case identifier of this operation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ListProfiles => "list_profiles",
            Self::ListRuntimes => "list_runtimes",
            Self::StartProfile => "start_profile",
            Self::ObserveSession => "observe_session",
            Self::StopSession => "stop_session",
            Self::RetryCleanup => "retry_cleanup",
            Self::ListAccounts => "list_accounts",
            Self::ImportAccount => "import_account",
            Self::UpdateAccount => "update_account",
            Self::SetAccountServer => "set_account_server",
            Self::DeleteAccount => "delete_account",
            Self::RunAccounts => "run_accounts",
            Self::StopAccount => "stop_account",
            Self::RetryAccountCleanup => "retry_account_cleanup",
            Self::ObserveAccountPlayer => "observe_account_player",
            Self::SetAccountControl => "set_account_control",
            Self::ObserveSpots => "observe_spots",
            Self::SaveSpot => "save_spot",
            Self::ClearSpot => "clear_spot",
            Self::Close => "close",
        }
    }
}

/// Stable failure classification of the manager boundary.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerErrorCode {
    InvalidDataRoot,
    DataRootNotManaged,
    DataRootInsecure,
    ControllerAlreadyOpen,
    StorageFailure,
    RuntimeRejected,
    RuntimeNotFound,
    InvalidPageLimit,
    InvalidProfileId,
    InvalidRevision,
    ProfileNotFound,
    ProfileArchived,
    RevisionConflict,
    ProfileAlreadyActive,
    CapacityReached,
    SessionCollision,
    StartRejected,
    StartDeadlineExpired,
    CleanupPending,
    InvalidSessionId,
    SessionNotFound,
    WrongSessionState,
    ProcessDeadlineExpired,
    ProcessFailure,
    CloseIncomplete,
    ControllerClosing,
    ControllerClosed,
    InvalidUsername,
    InvalidPassword,
    PlayerSnapshotUnreadable,
    ControlSettingsRejected,
    DuplicateUsername,
    AccountLimitReached,
    AccountNotFound,
    CredentialVaultUnavailable,
    AccountNotRunning,
    LoginSeedFailed,
    InternalInvariant,
}

impl ManagerErrorCode {
    /// Stable lowercase snake-case identifier of this failure class.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDataRoot => "invalid_data_root",
            Self::DataRootNotManaged => "data_root_not_managed",
            Self::DataRootInsecure => "data_root_insecure",
            Self::ControllerAlreadyOpen => "controller_already_open",
            Self::StorageFailure => "storage_failure",
            Self::RuntimeRejected => "runtime_rejected",
            Self::RuntimeNotFound => "runtime_not_found",
            Self::InvalidPageLimit => "invalid_page_limit",
            Self::InvalidProfileId => "invalid_profile_id",
            Self::InvalidRevision => "invalid_revision",
            Self::ProfileNotFound => "profile_not_found",
            Self::ProfileArchived => "profile_archived",
            Self::RevisionConflict => "revision_conflict",
            Self::ProfileAlreadyActive => "profile_already_active",
            Self::CapacityReached => "capacity_reached",
            Self::SessionCollision => "session_collision",
            Self::StartRejected => "start_rejected",
            Self::StartDeadlineExpired => "start_deadline_expired",
            Self::CleanupPending => "cleanup_pending",
            Self::InvalidSessionId => "invalid_session_id",
            Self::SessionNotFound => "session_not_found",
            Self::WrongSessionState => "wrong_session_state",
            Self::ProcessDeadlineExpired => "process_deadline_expired",
            Self::ProcessFailure => "process_failure",
            Self::CloseIncomplete => "close_incomplete",
            Self::ControllerClosing => "controller_closing",
            Self::ControllerClosed => "controller_closed",
            Self::InvalidUsername => "invalid_username",
            Self::InvalidPassword => "invalid_password",
            Self::PlayerSnapshotUnreadable => "player_snapshot_unreadable",
            Self::ControlSettingsRejected => "control_settings_rejected",
            Self::DuplicateUsername => "duplicate_username",
            Self::AccountLimitReached => "account_limit_reached",
            Self::AccountNotFound => "account_not_found",
            Self::CredentialVaultUnavailable => "credential_vault_unavailable",
            Self::AccountNotRunning => "account_not_running",
            Self::LoginSeedFailed => "login_seed_failed",
            Self::InternalInvariant => "internal_invariant",
        }
    }
}

/// Manager failure carrying only approved, redacted context.
///
/// The error owns no `CoreError`, no platform error, no path, and no free-form
/// message, and its [`Error::source`] is always `None`.
pub struct ManagerError {
    code: ManagerErrorCode,
    operation: ManagerOperation,
    profile_id: Option<String>,
    session_id: Option<String>,
    expected_revision: Option<i64>,
    actual_revision: Option<i64>,
    maximum: Option<u32>,
    retained_session: Option<Box<ManagerSessionView>>,
    remaining_sessions: Box<[ManagerSessionView]>,
}

impl ManagerError {
    pub(crate) fn new(code: ManagerErrorCode, operation: ManagerOperation) -> Self {
        Self {
            code,
            operation,
            profile_id: None,
            session_id: None,
            expected_revision: None,
            actual_revision: None,
            maximum: None,
            retained_session: None,
            remaining_sessions: Box::default(),
        }
    }

    pub(crate) fn from_core(error: CoreError, operation: ManagerOperation) -> Self {
        match error {
            CoreError::InvalidDataRoot { .. } => {
                Self::new(ManagerErrorCode::InvalidDataRoot, operation)
            }
            CoreError::UnmanagedDataRoot | CoreError::UnmanagedDatabase => {
                Self::new(ManagerErrorCode::DataRootNotManaged, operation)
            }
            CoreError::InsecureDataRoot | CoreError::PortableRepair { .. } => {
                Self::new(ManagerErrorCode::DataRootInsecure, operation)
            }
            CoreError::AlreadyRunning => {
                Self::new(ManagerErrorCode::ControllerAlreadyOpen, operation)
            }
            CoreError::DatabaseTooLarge { .. }
            | CoreError::UnsupportedSchema { .. }
            | CoreError::Io { .. }
            | CoreError::Database(_) => Self::new(ManagerErrorCode::StorageFailure, operation),
            CoreError::RuntimeValidation { .. }
            | CoreError::RuntimeConflict { .. }
            | CoreError::RuntimeRegistryMismatch { .. }
            | CoreError::ProcessLaunchSpec { .. } => {
                Self::new(ManagerErrorCode::RuntimeRejected, operation)
            }
            CoreError::InvalidPageLimit { maximum } => {
                let mut result = Self::new(ManagerErrorCode::InvalidPageLimit, operation);
                result.maximum = Some(maximum);
                result
            }
            CoreError::InvalidProfileName | CoreError::ProfileDirectoryCollision => {
                Self::new(ManagerErrorCode::InternalInvariant, operation)
            }
            // The seed code names which record store rejected the write, so it stays inside Core; the
            // boundary reports only that the login seed failed.
            CoreError::RmsSeed { .. } => Self::new(ManagerErrorCode::LoginSeedFailed, operation),
            // The snapshot code names which parse rule the mod's file broke, which is a detail of a
            // file this boundary does not expose; the operator only needs to know it was unreadable.
            CoreError::PlayerSnapshot { .. } => {
                Self::new(ManagerErrorCode::PlayerSnapshotUnreadable, operation)
            }
            // The settings code names which rule the file broke, which is a detail of a file this
            // boundary does not expose; the operator only needs to know the settings were rejected.
            CoreError::ControlSettings { .. } | CoreError::SavedSpots { .. } => {
                Self::new(ManagerErrorCode::ControlSettingsRejected, operation)
            }
            // Account failures now reach this boundary through the Task 6 CRUD operations, so each
            // one maps to its own stable redacted code instead of a generic invariant violation.
            CoreError::InvalidUsername => Self::new(ManagerErrorCode::InvalidUsername, operation),
            CoreError::InvalidPassword => Self::new(ManagerErrorCode::InvalidPassword, operation),
            CoreError::DuplicateUsername => {
                Self::new(ManagerErrorCode::DuplicateUsername, operation)
            }
            CoreError::AccountLimitReached { maximum } => {
                let mut result = Self::new(ManagerErrorCode::AccountLimitReached, operation);
                result.maximum = Some(maximum);
                result
            }
            CoreError::AccountNotFound { .. } => {
                Self::new(ManagerErrorCode::AccountNotFound, operation)
            }
            // The account ID is deliberately dropped: revision conflicts reuse the existing code and
            // the UI already knows which row it submitted.
            CoreError::AccountRevisionConflict {
                expected, actual, ..
            } => {
                let mut result = Self::new(ManagerErrorCode::RevisionConflict, operation);
                result.expected_revision = Some(expected);
                result.actual_revision = Some(actual);
                result
            }
            CoreError::CredentialVaultUnavailable => {
                Self::new(ManagerErrorCode::CredentialVaultUnavailable, operation)
            }
            CoreError::InvalidProfileId => Self::new(ManagerErrorCode::InvalidProfileId, operation),
            CoreError::InvalidRevision => Self::new(ManagerErrorCode::InvalidRevision, operation),
            CoreError::RuntimeNotFound { .. } => {
                Self::new(ManagerErrorCode::RuntimeNotFound, operation)
            }
            CoreError::ProfileNotFound { profile_id } => {
                let mut result = Self::new(ManagerErrorCode::ProfileNotFound, operation);
                result.profile_id = Some(profile_id);
                result
            }
            CoreError::ProfileArchived { profile_id } => {
                let mut result = Self::new(ManagerErrorCode::ProfileArchived, operation);
                result.profile_id = Some(profile_id);
                result
            }
            CoreError::RevisionConflict {
                profile_id,
                expected,
                actual,
            } => {
                let mut result = Self::new(ManagerErrorCode::RevisionConflict, operation);
                result.profile_id = Some(profile_id);
                result.expected_revision = Some(expected);
                result.actual_revision = Some(actual);
                result
            }
        }
    }

    pub(crate) fn with_profile_id(mut self, profile_id: String) -> Self {
        self.profile_id = Some(profile_id);
        self
    }

    pub(crate) fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub(crate) fn with_maximum(mut self, maximum: u32) -> Self {
        self.maximum = Some(maximum);
        self
    }

    pub(crate) fn with_retained_session(mut self, retained_session: ManagerSessionView) -> Self {
        self.retained_session = Some(Box::new(retained_session));
        self
    }

    pub(crate) fn with_remaining_sessions(
        mut self,
        remaining_sessions: Vec<ManagerSessionView>,
    ) -> Self {
        self.remaining_sessions = remaining_sessions.into_boxed_slice();
        self
    }

    pub fn code(&self) -> ManagerErrorCode {
        self.code
    }

    pub fn operation(&self) -> ManagerOperation {
        self.operation
    }

    pub fn profile_id(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn expected_revision(&self) -> Option<i64> {
        self.expected_revision
    }

    pub fn actual_revision(&self) -> Option<i64> {
        self.actual_revision
    }

    pub fn maximum(&self) -> Option<u32> {
        self.maximum
    }

    pub fn retained_session(&self) -> Option<&ManagerSessionView> {
        self.retained_session.as_deref()
    }

    pub fn remaining_sessions(&self) -> &[ManagerSessionView] {
        &self.remaining_sessions
    }

    /// Writes the stable code, operation, and safe scalar context only.
    ///
    /// Session views are never rendered whole: a retained view contributes its
    /// canonical session ID and remaining views contribute only their count.
    fn write_redacted_context(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "code={} operation={}",
            self.code.as_str(),
            self.operation.as_str()
        )?;
        if let Some(profile_id) = &self.profile_id {
            write!(formatter, " profile_id={profile_id}")?;
        }
        if let Some(session_id) = &self.session_id {
            write!(formatter, " session_id={session_id}")?;
        }
        if let Some(expected_revision) = self.expected_revision {
            write!(formatter, " expected_revision={expected_revision}")?;
        }
        if let Some(actual_revision) = self.actual_revision {
            write!(formatter, " actual_revision={actual_revision}")?;
        }
        if let Some(maximum) = self.maximum {
            write!(formatter, " maximum={maximum}")?;
        }
        if let Some(retained_session) = &self.retained_session {
            write!(
                formatter,
                " retained_session_id={}",
                retained_session.session_id
            )?;
        }
        if !self.remaining_sessions.is_empty() {
            write!(
                formatter,
                " remaining_sessions={}",
                self.remaining_sessions.len()
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for ManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_redacted_context(formatter)
    }
}

impl fmt::Debug for ManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagerError { ")?;
        self.write_redacted_context(formatter)?;
        formatter.write_str(" }")
    }
}

impl Error for ManagerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

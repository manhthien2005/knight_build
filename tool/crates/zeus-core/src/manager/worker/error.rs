//! Stable, redacted failure taxonomy for the manager worker boundary.

use std::error::Error;
use std::fmt;

/// Result of every public manager worker operation.
pub type ManagerWorkerResult<T> = Result<T, ManagerWorkerError>;

/// Public operation that produced a [`ManagerWorkerError`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerWorkerOperation {
    Spawn,
    ListProfiles,
    ListRuntimes,
    ListSessions,
    StartProfile,
    ObserveSession,
    StopSession,
    RetryCleanup,
    Shutdown,
    ReceiveEvent,
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
    ObserveAccountControl,
}

impl ManagerWorkerOperation {
    /// Stable lowercase snake-case identifier of this operation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::ListProfiles => "list_profiles",
            Self::ListRuntimes => "list_runtimes",
            Self::ListSessions => "list_sessions",
            Self::StartProfile => "start_profile",
            Self::ObserveSession => "observe_session",
            Self::StopSession => "stop_session",
            Self::RetryCleanup => "retry_cleanup",
            Self::Shutdown => "shutdown",
            Self::ReceiveEvent => "receive_event",
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
            Self::ObserveAccountControl => "observe_account_control",
        }
    }
}

/// Stable failure classification of the manager worker boundary.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerWorkerErrorCode {
    ThreadSpawnFailed,
    NotReady,
    InputTooLong,
    CommandQueueFull,
    Closing,
    ShutdownPending,
    Closed,
    Disconnected,
    RequestIdExhausted,
    InvalidInput,
}

impl ManagerWorkerErrorCode {
    /// Stable lowercase snake-case identifier of this failure class.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThreadSpawnFailed => "thread_spawn_failed",
            Self::NotReady => "not_ready",
            Self::InputTooLong => "input_too_long",
            Self::CommandQueueFull => "command_queue_full",
            Self::Closing => "closing",
            Self::ShutdownPending => "shutdown_pending",
            Self::Closed => "closed",
            Self::Disconnected => "disconnected",
            Self::RequestIdExhausted => "request_id_exhausted",
            Self::InvalidInput => "invalid_input",
        }
    }
}

/// Worker failure carrying only approved, redacted context.
pub struct ManagerWorkerError {
    code: ManagerWorkerErrorCode,
    operation: ManagerWorkerOperation,
    maximum: Option<u32>,
}

impl ManagerWorkerError {
    pub(crate) fn new(code: ManagerWorkerErrorCode, operation: ManagerWorkerOperation) -> Self {
        Self {
            code,
            operation,
            maximum: None,
        }
    }

    pub(crate) fn with_maximum(mut self, maximum: u32) -> Self {
        self.maximum = Some(maximum);
        self
    }

    pub fn code(&self) -> ManagerWorkerErrorCode {
        self.code
    }

    pub fn operation(&self) -> ManagerWorkerOperation {
        self.operation
    }

    pub fn maximum(&self) -> Option<u32> {
        self.maximum
    }

    fn write_redacted_context(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "code={} operation={}",
            self.code.as_str(),
            self.operation.as_str()
        )?;
        if let Some(maximum) = self.maximum {
            write!(formatter, " maximum={maximum}")?;
        }
        Ok(())
    }
}

impl fmt::Display for ManagerWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_redacted_context(formatter)
    }
}

impl fmt::Debug for ManagerWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagerWorkerError { ")?;
        self.write_redacted_context(formatter)?;
        formatter.write_str(" }")
    }
}

impl Error for ManagerWorkerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

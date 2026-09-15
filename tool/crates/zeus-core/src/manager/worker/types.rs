//! Owned, redacted value types for the manager worker boundary.

use crate::spots::SpotBook;
use std::num::NonZeroU64;

use super::super::{
    ControlSettings, ManagerAccountView, ManagerError, ManagerObservation, ManagerProfilePage,
    ManagerResult, ManagerRunSchedule, ManagerRuntimePage, ManagerSessionExit, ManagerSessionView,
    PlayerSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManagerRequestId(NonZeroU64);

impl ManagerRequestId {
    pub fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerWorkerState {
    Starting,
    Ready,
    Closing,
    Closed,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum ManagerWorkerEvent {
    Ready,
    OpenFailed(ManagerError),
    ProfilesListed {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerProfilePage>,
    },
    RuntimesListed {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerRuntimePage>,
    },
    SessionsListed {
        request_id: ManagerRequestId,
        sessions: Vec<ManagerSessionView>,
    },
    ProfileStarted {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerSessionView>,
    },
    SessionObserved {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerObservation>,
    },
    SessionStopped {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerSessionExit>,
    },
    CleanupRetried {
        request_id: ManagerRequestId,
        result: ManagerResult<()>,
    },
    AccountsListed {
        request_id: ManagerRequestId,
        result: ManagerResult<Vec<ManagerAccountView>>,
    },
    AccountImported {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerAccountView>,
    },
    AccountUpdated {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerAccountView>,
    },
    AccountDeleted {
        request_id: ManagerRequestId,
        result: ManagerResult<()>,
    },
    AccountsRunScheduled {
        request_id: ManagerRequestId,
        result: ManagerResult<Vec<ManagerRunSchedule>>,
    },
    /// Confirmed stop, answering with the account's reconciled row.
    ///
    /// The row, not the process exit: a stop that answered with only an acknowledgement left the UI
    /// holding its optimistic `Stopping` status until something else happened to refresh the list.
    AccountStopResult {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerAccountView>,
    },
    /// Confirmed cleanup retry, answering with the account's reconciled row for the same reason.
    AccountCleanupRetried {
        request_id: ManagerRequestId,
        result: ManagerResult<ManagerAccountView>,
    },
    /// One account's published character reading, or `None` when nothing has been published yet.
    AccountPlayerObserved {
        request_id: ManagerRequestId,
        result: ManagerResult<Option<PlayerSnapshot>>,
    },
    /// One account's attack and item settings, as the mod will actually read them.
    ///
    /// Answers both writing and reading them: the reply is the same shape either way, and a second
    /// event would make every consumer handle two variants of the same fact.
    AccountControlApplied {
        request_id: ManagerRequestId,
        result: ManagerResult<ControlSettings>,
    },
    /// Every saved monster spot, after reading, saving or clearing one.
    ///
    /// One event for all three, like the control one above: the reply is the whole book either way,
    /// and a caller that just saved wants to see the result rather than reconcile a delta.
    SpotsApplied {
        request_id: ManagerRequestId,
        result: ManagerResult<SpotBook>,
    },
    ShutdownResult {
        request_id: ManagerRequestId,
        result: ManagerResult<()>,
    },
    /// The one notification M3.1 adds to M2's request-result-only rule: terminal asynchronous
    /// readiness completion. It carries no request ID and may arrive in account order.
    AccountStateChanged(ManagerAccountView),
}

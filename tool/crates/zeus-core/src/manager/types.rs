//! Owned, redacted value types for the Windows manager control boundary.

use crate::runtime::CapabilityState;

/// Catalog view of one configured profile.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerProfileView {
    pub profile_id: String,
    pub revision: i64,
    pub display_name: String,
    pub runtime_id: String,
    pub archived: bool,
    pub active_session_id: Option<String>,
}

/// One keyset page of profile views.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerProfilePage {
    pub items: Vec<ManagerProfileView>,
    pub next_cursor: Option<String>,
}

/// Catalog view of one registered runtime, without paths, hashes, or timestamps.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerRuntimeView {
    pub runtime_id: String,
    pub target_os: String,
    pub target_arch: String,
    pub java_vendor: String,
    pub java_version: String,
    pub microemulator_version: String,
    pub game_bundle: String,
    pub capability_state: CapabilityState,
    pub validation_reason: String,
}

/// One keyset page of runtime views.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerRuntimePage {
    pub items: Vec<ManagerRuntimeView>,
    pub next_cursor: Option<String>,
}

/// Retained ownership state of one session.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerSessionState {
    Running,
    CleanupPending,
}

/// View of one retained session, without process identity.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerSessionView {
    pub session_id: String,
    pub profile_id: String,
    pub profile_revision: i64,
    pub runtime_id: String,
    pub state: ManagerSessionState,
}

/// View of one session whose cleanup the supervisor confirmed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerSessionExit {
    pub session_id: String,
    pub profile_id: String,
    pub profile_revision: i64,
    pub runtime_id: String,
}

/// Outcome of observing one owned session.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerObservation {
    Running(ManagerSessionView),
    CleanupPending(ManagerSessionView),
    Exited(ManagerSessionExit),
}

//! Lightweight, synchronous foundation for the Zeus HSO Manager Core.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
extern crate self as zeus_core;

mod account;
mod control;
mod credential_vault;
mod data_root;
mod error;
mod launch_snapshot;
#[cfg(windows)]
mod manager;
mod player;
#[cfg(windows)]
mod process_adapter;
mod process_launch_spec;
mod profile;
mod rms;
pub mod runtime;
#[cfg(windows)]
mod session_supervisor;
mod spots;
mod store;
pub mod wire;

#[cfg(all(test, windows))]
#[path = "../tests/support/runtime_fixture.rs"]
mod runtime_fixture;

pub use control::{
    AttackMode, AttackSpot, BUFF_SLOTS, ControlSettings, ENHANCE_CHARM_MAX, ENHANCE_LEVEL_MAX,
    ENHANCE_LEVEL_MIN, GoldPickup, ItemRank, MATERIAL_LABELS, MATERIAL_SLOTS, MOUNT_ANY,
    MOUNT_TEMPLATE_IDS, PotionPickup, REVIVE_DELAY_MAX, ReviveMode, ZoneMode,
};
pub use error::{CoreError, CoreResult};
pub use launch_snapshot::{
    HeapSettings, JvmFlag, LaunchEnvironmentKey, LaunchEnvironmentVariable, LaunchSnapshot,
    RmsMode, ScreenSize,
};
#[cfg(windows)]
pub use manager::{
    MAX_RUN_BATCH, ManagerAccountId, ManagerAccountPassword, ManagerAccountStatus,
    ManagerAccountView, ManagerController, ManagerError, ManagerErrorCode, ManagerObservation,
    ManagerOperation, ManagerProfilePage, ManagerProfileView, ManagerRequestId, ManagerResult,
    ManagerRunRejection, ManagerRunSchedule, ManagerRunScheduleOutcome, ManagerRuntimePage,
    ManagerRuntimeView, ManagerSessionExit, ManagerSessionState, ManagerSessionView, ManagerWorker,
    ManagerWorkerError, ManagerWorkerErrorCode, ManagerWorkerEvent, ManagerWorkerOperation,
    ManagerWorkerResult, ManagerWorkerState,
};
pub use player::PlayerSnapshot;
pub use process_launch_spec::{
    MAX_PROCESS_ARGUMENT_NATIVE_UNITS, MAX_PROCESS_ARGUMENTS, MAX_WINDOWS_COMMAND_LINE_UNITS,
    PROCESS_ARGV_SCHEMA_VERSION, ProcessLaunchSpec, ProcessStdio,
};
pub use profile::{ProfilePage, ProfileRecord};
pub use rms::SERVER_NAMES;
pub use runtime::{
    MAX_RUNTIME_PREFLIGHT_CACHE_ENTRIES, RuntimePreflightDiagnostics, RuntimePreflightMode,
};
pub use spots::SpotBook;
pub use store::{
    CoreState, DatabaseInvariants, MAX_DATABASE_BYTES, RuntimePage, RuntimeRecord, RuntimeRepin,
    SCHEMA_VERSION,
};

/// Version of the bounded diagnostic command schema exposed by this foundation.
pub const COMMAND_SCHEMA_VERSION: u32 = 1;

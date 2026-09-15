use std::time::Instant;

use crate::ProcessLaunchSpec;
use crate::process_adapter::{
    ProcessBirthId, RootExit, UnconfirmedWindowsProcess, WindowsContainedProcess,
    WindowsProcessError, WindowsSpawnFailure, spawn,
};

pub(crate) trait ProcessBackend {
    type RunningOwner;
    type CleanupOwner;
    type Error;

    fn spawn(
        &mut self,
        spec: ProcessLaunchSpec,
        deadline: Instant,
    ) -> Result<Self::RunningOwner, BackendSpawnFailure<Self::CleanupOwner, Self::Error>>;

    fn birth_id(&self, owner: &Self::RunningOwner) -> ProcessBirthId;

    fn try_wait_root(
        &mut self,
        owner: &mut Self::RunningOwner,
    ) -> Result<Option<RootExit>, Self::Error>;

    fn terminate_tree_and_wait(
        &mut self,
        owner: &mut Self::RunningOwner,
        deadline: Instant,
    ) -> Result<RootExit, Self::Error>;

    fn retry_cleanup(
        &mut self,
        owner: &mut Self::CleanupOwner,
        deadline: Instant,
    ) -> Result<(), Self::Error>;
}

pub(crate) enum BackendSpawnFailure<C, E> {
    Rejected(E),
    CleanupUnconfirmed { error: E, owner: C },
}

pub(crate) struct WindowsProcessBackend;

impl ProcessBackend for WindowsProcessBackend {
    type RunningOwner = WindowsContainedProcess;
    type CleanupOwner = UnconfirmedWindowsProcess;
    type Error = WindowsProcessError;

    fn spawn(
        &mut self,
        spec: ProcessLaunchSpec,
        deadline: Instant,
    ) -> Result<Self::RunningOwner, BackendSpawnFailure<Self::CleanupOwner, Self::Error>> {
        spawn(spec, deadline).map_err(|failure| match failure {
            WindowsSpawnFailure::Rejected(error) => BackendSpawnFailure::Rejected(error),
            WindowsSpawnFailure::CleanupUnconfirmed { error, owner } => {
                BackendSpawnFailure::CleanupUnconfirmed { error, owner }
            }
        })
    }

    fn birth_id(&self, owner: &Self::RunningOwner) -> ProcessBirthId {
        owner.birth_id()
    }

    fn try_wait_root(
        &mut self,
        owner: &mut Self::RunningOwner,
    ) -> Result<Option<RootExit>, Self::Error> {
        owner.try_wait_root()
    }

    fn terminate_tree_and_wait(
        &mut self,
        owner: &mut Self::RunningOwner,
        deadline: Instant,
    ) -> Result<RootExit, Self::Error> {
        owner.terminate_tree_and_wait(deadline)
    }

    fn retry_cleanup(
        &mut self,
        owner: &mut Self::CleanupOwner,
        deadline: Instant,
    ) -> Result<(), Self::Error> {
        owner.retry_cleanup(deadline)
    }
}

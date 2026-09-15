use std::time::Instant;

use crate::process_adapter::test_support::{
    ProcessObservation, open_identity_checked_process_for_metrics,
};
use crate::runtime::CapabilityState;
use crate::session_supervisor::{
    LIVE_RUNTIME_ID, LiveGateFailure, LiveManagerCoreFixture, LiveTestDirectory, STOP_DEADLINE,
    WINDOW_READY_DEADLINE, prepare_live_manager_core_fixture, require_qualified_ready_window,
    wait_for_process_signal,
};

use super::{
    ManagerController, ManagerErrorCode, ManagerObservation, ManagerSessionState,
    ManagerSessionView,
};

enum LiveOwnerState {
    NotStarted,
    Running(String),
    CleanupPending(String),
    Cleaned,
}

struct ManagerLiveCleanupGuard {
    directory: Option<LiveTestDirectory>,
    manager: Option<ManagerController>,
    owner: LiveOwnerState,
    observation: Option<ProcessObservation>,
}

impl ManagerLiveCleanupGuard {
    fn new(fixture: LiveManagerCoreFixture) -> (Self, String, i64, std::path::PathBuf) {
        let (directory, core, profile_id, profile_revision, profile_root) = fixture.into_parts();
        (
            Self {
                directory: Some(directory),
                manager: Some(ManagerController::from_core(core)),
                owner: LiveOwnerState::NotStarted,
                observation: None,
            },
            profile_id,
            profile_revision,
            profile_root,
        )
    }

    fn manager(&self) -> &ManagerController {
        self.manager
            .as_ref()
            .expect("live manager remains owned until confirmed cleanup")
    }

    fn manager_mut(&mut self) -> &mut ManagerController {
        self.manager
            .as_mut()
            .expect("live manager remains owned until confirmed cleanup")
    }

    fn observation(&self) -> Result<&ProcessObservation, LiveGateFailure> {
        self.observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("manager_live_observation_missing"))
    }

    fn finish(mut self, original: Result<(), LiveGateFailure>) -> Result<(), LiveGateFailure> {
        match self.cleanup() {
            Ok(()) => original,
            Err(cleanup) => Err(cleanup),
        }
    }

    fn cleanup(&mut self) -> Result<(), LiveGateFailure> {
        if !self.manager().list_sessions().is_empty() {
            let action = match &self.owner {
                LiveOwnerState::Running(session_id) => Some((session_id.clone(), false)),
                LiveOwnerState::CleanupPending(session_id) => Some((session_id.clone(), true)),
                LiveOwnerState::NotStarted | LiveOwnerState::Cleaned => None,
            };
            let Some((session_id, cleanup_pending)) = action else {
                return Err(LiveGateFailure::stage("manager_live_cleanup_untracked"));
            };
            if cleanup_pending {
                let _ = self.manager_mut().retry_cleanup(&session_id);
            } else {
                let _ = self.manager_mut().stop_session(&session_id);
            }
        }

        let mut close_result = self.manager_mut().close();
        if close_result.is_err() {
            close_result = self.manager_mut().close();
        }
        close_result.map_err(|_| LiveGateFailure::stage("manager_live_cleanup_close"))?;
        if !self.manager().list_sessions().is_empty() {
            return Err(LiveGateFailure::stage(
                "manager_live_cleanup_active_sessions",
            ));
        }
        if let Some(observation) = &self.observation {
            wait_for_process_signal(observation, Instant::now() + STOP_DEADLINE)?;
        }

        drop(self.manager.take());
        let directory = self
            .directory
            .take()
            .expect("guarded live directory remains until confirmed cleanup");
        let root = directory.path().to_owned();
        directory.remove_after_cleanup_confirmation()?;
        if root.exists() {
            return Err(LiveGateFailure::stage("manager_live_cleanup_root_remains"));
        }
        Ok(())
    }
}

fn require_running_view(
    observation: ManagerObservation,
    expected: &ManagerSessionView,
) -> Result<(), LiveGateFailure> {
    match observation {
        ManagerObservation::Running(view) if view == *expected => Ok(()),
        _ => Err(LiveGateFailure::stage("manager_live_observed_state")),
    }
}

fn run_public_manager_live_gate() -> Result<(), LiveGateFailure> {
    let fixture = prepare_live_manager_core_fixture("Manager Control Live")?;
    let (mut guard, profile_id, profile_revision, profile_root) =
        ManagerLiveCleanupGuard::new(fixture);

    let live_result = (|| {
        let runtime_page = guard
            .manager()
            .list_runtimes(None, 100)
            .map_err(|_| LiveGateFailure::stage("manager_live_runtime_list"))?;
        let runtime = runtime_page
            .items
            .iter()
            .find(|runtime| runtime.runtime_id == LIVE_RUNTIME_ID)
            .ok_or_else(|| LiveGateFailure::stage("manager_live_runtime_missing"))?;
        if runtime.capability_state != CapabilityState::NeedsValidation {
            return Err(LiveGateFailure::stage("manager_live_capability_state"));
        }

        let started = match guard
            .manager_mut()
            .start_profile(&profile_id, profile_revision)
        {
            Ok(started) => started,
            Err(error) if error.code() == ManagerErrorCode::CleanupPending => {
                let session_id = error
                    .retained_session()
                    .map(|session| session.session_id.clone())
                    .or_else(|| error.session_id().map(str::to_owned))
                    .ok_or_else(|| LiveGateFailure::stage("manager_live_start_retained"))?;
                guard.owner = LiveOwnerState::CleanupPending(session_id);
                return Err(LiveGateFailure::stage("manager_live_start_cleanup_pending"));
            }
            Err(_) => return Err(LiveGateFailure::stage("manager_live_start")),
        };
        guard.owner = LiveOwnerState::Running(started.session_id.clone());

        let birth_id = guard
            .manager()
            .running_birth_id_for_test(&started.session_id)
            .ok_or_else(|| LiveGateFailure::stage("manager_live_birth_identity"))?;
        if birth_id.pid() == 0 || birth_id.creation_time_100ns() == 0 {
            return Err(LiveGateFailure::stage("manager_live_birth_identity"));
        }
        let observation = open_identity_checked_process_for_metrics(
            birth_id.pid(),
            birth_id.creation_time_100ns(),
        )
        .map_err(|_| LiveGateFailure::stage("manager_live_observation_open"))?;
        if observation.birth_id() != birth_id {
            return Err(LiveGateFailure::stage("manager_live_observation_identity"));
        }
        guard.observation = Some(observation);

        if guard.manager().list_sessions() != vec![started.clone()]
            || started.state != ManagerSessionState::Running
        {
            return Err(LiveGateFailure::stage("manager_live_session_list"));
        }
        require_running_view(
            guard
                .manager_mut()
                .observe_session(&started.session_id)
                .map_err(|_| LiveGateFailure::stage("manager_live_observe"))?,
            &started,
        )?;

        let readiness_deadline = Instant::now() + WINDOW_READY_DEADLINE;
        require_qualified_ready_window(guard.observation()?, &profile_root, readiness_deadline)?;
        require_running_view(
            guard
                .manager_mut()
                .observe_session(&started.session_id)
                .map_err(|_| LiveGateFailure::stage("manager_live_observe_ready"))?,
            &started,
        )?;

        let runtime_page = guard
            .manager()
            .list_runtimes(None, 100)
            .map_err(|_| LiveGateFailure::stage("manager_live_runtime_recheck"))?;
        if runtime_page.items.len() != 1
            || runtime_page.items[0].runtime_id != LIVE_RUNTIME_ID
            || runtime_page.items[0].capability_state != CapabilityState::NeedsValidation
        {
            return Err(LiveGateFailure::stage("manager_live_capability_recheck"));
        }

        let exited = guard
            .manager_mut()
            .stop_session(&started.session_id)
            .map_err(|_| LiveGateFailure::stage("manager_live_stop"))?;
        if exited.session_id != started.session_id
            || exited.profile_id != started.profile_id
            || exited.profile_revision != started.profile_revision
            || exited.runtime_id != started.runtime_id
        {
            return Err(LiveGateFailure::stage("manager_live_exit_metadata"));
        }
        guard.owner = LiveOwnerState::Cleaned;
        wait_for_process_signal(guard.observation()?, Instant::now() + STOP_DEADLINE)?;
        if !guard.manager().list_sessions().is_empty() {
            return Err(LiveGateFailure::stage("manager_live_stop_inventory"));
        }
        guard
            .manager_mut()
            .close()
            .map_err(|_| LiveGateFailure::stage("manager_live_close"))?;
        if !guard.manager().list_sessions().is_empty() {
            return Err(LiveGateFailure::stage("manager_live_closed_inventory"));
        }
        Ok(())
    })();

    guard.finish(live_result)
}

#[test]
#[ignore = "launches one provisioned exact Windows game through public Manager control; run the dedicated manager live runner"]
fn exact_runtime_runs_through_public_manager_control() {
    if let Err(error) = run_public_manager_live_gate() {
        panic!("{error}");
    }
}

use std::thread;
use std::time::{Duration, Instant};

use crate::process_adapter::test_support::{
    ProcessObservation, open_identity_checked_process_for_metrics,
};
use crate::runtime::CapabilityState;
use crate::session_supervisor::{
    LIVE_RUNTIME_ID, LiveGateFailure, LiveManagerCoreFixture, LiveTestDirectory, STOP_DEADLINE,
    WINDOW_READY_DEADLINE, prepare_live_manager_core_fixture, require_qualified_ready_window,
    wait_for_process_signal,
};

use super::super::{
    ManagerController, ManagerError, ManagerErrorCode, ManagerObservation, ManagerSessionState,
    ManagerSessionView,
};
use super::{ManagerRequestId, ManagerWorker, ManagerWorkerEvent, ManagerWorkerState};

const EVENT_DEADLINE: Duration = Duration::from_secs(15);
const EVENT_POLL_CADENCE: Duration = Duration::from_millis(10);

enum LiveOwnerState {
    NotStarted,
    Running(String),
    CleanupPending(String),
    Cleaned,
}

struct WorkerLiveCleanupGuard {
    directory: Option<LiveTestDirectory>,
    worker: Option<ManagerWorker>,
    owner: LiveOwnerState,
    observation: Option<ProcessObservation>,
    shutdown_request: Option<ManagerRequestId>,
}

impl WorkerLiveCleanupGuard {
    fn new(
        fixture: LiveManagerCoreFixture,
    ) -> Result<(Self, String, i64, std::path::PathBuf), LiveGateFailure> {
        let (directory, core, profile_id, profile_revision, profile_root) = fixture.into_parts();
        let worker = match ManagerWorker::spawn_controller(ManagerController::from_core(core)) {
            Ok(worker) => worker,
            Err(_) => {
                return match directory.remove_after_cleanup_confirmation() {
                    Ok(()) => Err(LiveGateFailure::stage("worker_live_spawn")),
                    Err(cleanup) => Err(cleanup),
                };
            }
        };
        Ok((
            Self {
                directory: Some(directory),
                worker: Some(worker),
                owner: LiveOwnerState::NotStarted,
                observation: None,
                shutdown_request: None,
            },
            profile_id,
            profile_revision,
            profile_root,
        ))
    }

    fn worker(&self) -> &ManagerWorker {
        self.worker
            .as_ref()
            .expect("live worker remains owned until confirmed cleanup")
    }

    fn worker_mut(&mut self) -> &mut ManagerWorker {
        self.worker
            .as_mut()
            .expect("live worker remains owned until confirmed cleanup")
    }

    fn observation(&self) -> Result<&ProcessObservation, LiveGateFailure> {
        self.observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("worker_live_observation_missing"))
    }

    fn finish(mut self, original: Result<(), LiveGateFailure>) -> Result<(), LiveGateFailure> {
        match self.cleanup() {
            Ok(()) => original,
            Err(cleanup) => Err(cleanup),
        }
    }

    fn request_shutdown(&mut self) -> Result<(), LiveGateFailure> {
        let request_id = self
            .worker_mut()
            .try_shutdown()
            .map_err(|_| LiveGateFailure::stage("worker_live_cleanup_shutdown_submit"))?;
        self.shutdown_request = Some(request_id);
        Ok(())
    }

    fn await_shutdown(&mut self) -> Result<Result<(), ManagerError>, LiveGateFailure> {
        let request_id = self
            .shutdown_request
            .ok_or_else(|| LiveGateFailure::stage("worker_live_cleanup_shutdown_missing"))?;
        let event = wait_for_request_event(
            self.worker_mut(),
            request_id,
            "worker_live_cleanup_shutdown_event",
        )?;
        self.shutdown_request = None;
        match event {
            ManagerWorkerEvent::ShutdownResult { result, .. } => Ok(result),
            _ => Err(LiveGateFailure::stage(
                "worker_live_cleanup_shutdown_variant",
            )),
        }
    }

    fn cleanup_owner(&mut self) -> Result<(), LiveGateFailure> {
        let action = match &self.owner {
            LiveOwnerState::Running(session_id) => Some((session_id.clone(), false)),
            LiveOwnerState::CleanupPending(session_id) => Some((session_id.clone(), true)),
            LiveOwnerState::NotStarted | LiveOwnerState::Cleaned => None,
        };
        let Some((session_id, cleanup_pending)) = action else {
            return Ok(());
        };

        if cleanup_pending {
            let request_id = self
                .worker_mut()
                .try_retry_cleanup(&session_id)
                .map_err(|_| LiveGateFailure::stage("worker_live_cleanup_retry_submit"))?;
            match wait_for_request_event(
                self.worker_mut(),
                request_id,
                "worker_live_cleanup_retry_event",
            )? {
                ManagerWorkerEvent::CleanupRetried { result: Ok(()), .. } => {
                    self.owner = LiveOwnerState::Cleaned;
                    Ok(())
                }
                _ => Err(LiveGateFailure::stage("worker_live_cleanup_retry_result")),
            }
        } else {
            let request_id = self
                .worker_mut()
                .try_stop_session(&session_id)
                .map_err(|_| LiveGateFailure::stage("worker_live_cleanup_stop_submit"))?;
            match wait_for_request_event(
                self.worker_mut(),
                request_id,
                "worker_live_cleanup_stop_event",
            )? {
                ManagerWorkerEvent::SessionStopped { result: Ok(_), .. } => {
                    self.owner = LiveOwnerState::Cleaned;
                    Ok(())
                }
                ManagerWorkerEvent::SessionStopped {
                    result: Err(error), ..
                } if error.code() == ManagerErrorCode::CleanupPending => {
                    self.owner = LiveOwnerState::CleanupPending(session_id);
                    self.cleanup_owner()
                }
                _ => Err(LiveGateFailure::stage("worker_live_cleanup_stop_result")),
            }
        }
    }

    fn cleanup_remaining(
        &mut self,
        remaining: &[ManagerSessionView],
    ) -> Result<(), LiveGateFailure> {
        for session in remaining {
            self.owner = match session.state {
                ManagerSessionState::Running => LiveOwnerState::Running(session.session_id.clone()),
                ManagerSessionState::CleanupPending => {
                    LiveOwnerState::CleanupPending(session.session_id.clone())
                }
            };
            self.cleanup_owner()?;
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), LiveGateFailure> {
        if self.worker().state() == ManagerWorkerState::Starting {
            require_ready(self.worker_mut())?;
        }

        let mut retry_shutdown = false;
        if self.shutdown_request.is_some() {
            match self.await_shutdown()? {
                Ok(()) => {}
                Err(error) => {
                    self.cleanup_remaining(error.remaining_sessions())?;
                    retry_shutdown = true;
                }
            }
        }

        if self.worker().state() != ManagerWorkerState::Closed {
            self.cleanup_owner()?;
            self.request_shutdown()?;
            match self.await_shutdown()? {
                Ok(()) => {}
                Err(error) if !retry_shutdown => {
                    self.cleanup_remaining(error.remaining_sessions())?;
                    self.request_shutdown()?;
                    self.await_shutdown()?.map_err(|_| {
                        LiveGateFailure::stage("worker_live_cleanup_shutdown_retry")
                    })?;
                }
                Err(_) => {
                    return Err(LiveGateFailure::stage(
                        "worker_live_cleanup_shutdown_incomplete",
                    ));
                }
            }
        }

        if let Some(observation) = &self.observation {
            wait_for_process_signal(observation, Instant::now() + STOP_DEADLINE)?;
        }

        drop(self.worker.take());
        let directory = self
            .directory
            .take()
            .expect("guarded live directory remains until confirmed cleanup");
        let root = directory.path().to_owned();
        directory.remove_after_cleanup_confirmation()?;
        if root.exists() {
            return Err(LiveGateFailure::stage("worker_live_cleanup_root_remains"));
        }
        Ok(())
    }
}

fn next_event(
    worker: &mut ManagerWorker,
    deadline: Instant,
    stage: &'static str,
) -> Result<ManagerWorkerEvent, LiveGateFailure> {
    loop {
        match worker.try_next_event() {
            Ok(Some(event)) => return Ok(event),
            Ok(None) => {}
            Err(_) => return Err(LiveGateFailure::stage(stage)),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(LiveGateFailure::stage(stage));
        }
        thread::sleep((deadline - now).min(EVENT_POLL_CADENCE));
    }
}

fn event_request_id(event: &ManagerWorkerEvent) -> Option<ManagerRequestId> {
    match event {
        ManagerWorkerEvent::ProfilesListed { request_id, .. }
        | ManagerWorkerEvent::RuntimesListed { request_id, .. }
        | ManagerWorkerEvent::SessionsListed { request_id, .. }
        | ManagerWorkerEvent::ProfileStarted { request_id, .. }
        | ManagerWorkerEvent::SessionObserved { request_id, .. }
        | ManagerWorkerEvent::SessionStopped { request_id, .. }
        | ManagerWorkerEvent::CleanupRetried { request_id, .. }
        | ManagerWorkerEvent::AccountsListed { request_id, .. }
        | ManagerWorkerEvent::AccountImported { request_id, .. }
        | ManagerWorkerEvent::AccountUpdated { request_id, .. }
        | ManagerWorkerEvent::AccountDeleted { request_id, .. }
        | ManagerWorkerEvent::AccountsRunScheduled { request_id, .. }
        | ManagerWorkerEvent::AccountStopResult { request_id, .. }
        | ManagerWorkerEvent::AccountCleanupRetried { request_id, .. }
        | ManagerWorkerEvent::AccountPlayerObserved { request_id, .. }
        | ManagerWorkerEvent::SpotsApplied { request_id, .. }
        | ManagerWorkerEvent::AccountControlApplied { request_id, .. }
        | ManagerWorkerEvent::SpotsApplied { request_id, .. }
        | ManagerWorkerEvent::ShutdownResult { request_id, .. } => Some(*request_id),
        // The one M3.1 notification carries no request ID.
        ManagerWorkerEvent::AccountStateChanged(_) => None,
        ManagerWorkerEvent::Ready | ManagerWorkerEvent::OpenFailed(_) => None,
    }
}

fn wait_for_request_event(
    worker: &mut ManagerWorker,
    request_id: ManagerRequestId,
    stage: &'static str,
) -> Result<ManagerWorkerEvent, LiveGateFailure> {
    let event = next_event(worker, Instant::now() + EVENT_DEADLINE, stage)?;
    if event_request_id(&event) != Some(request_id) {
        return Err(LiveGateFailure::stage(stage));
    }
    Ok(event)
}

fn require_ready(worker: &mut ManagerWorker) -> Result<(), LiveGateFailure> {
    match next_event(worker, Instant::now() + EVENT_DEADLINE, "worker_live_ready")? {
        ManagerWorkerEvent::Ready if worker.state() == ManagerWorkerState::Ready => Ok(()),
        _ => Err(LiveGateFailure::stage("worker_live_ready")),
    }
}

fn require_running_view(
    observation: ManagerObservation,
    expected: &ManagerSessionView,
) -> Result<(), LiveGateFailure> {
    match observation {
        ManagerObservation::Running(view) if view == *expected => Ok(()),
        _ => Err(LiveGateFailure::stage("worker_live_observed_state")),
    }
}

fn run_public_manager_worker_live_gate() -> Result<(), LiveGateFailure> {
    let fixture = prepare_live_manager_core_fixture("Manager Worker Live")?;
    let (mut guard, profile_id, profile_revision, profile_root) =
        WorkerLiveCleanupGuard::new(fixture)?;

    let live_result = (|| {
        require_ready(guard.worker_mut())?;

        let runtime_request = guard
            .worker_mut()
            .try_list_runtimes(None, 100)
            .map_err(|_| LiveGateFailure::stage("worker_live_runtime_submit"))?;
        let runtime_page = match wait_for_request_event(
            guard.worker_mut(),
            runtime_request,
            "worker_live_runtime_event",
        )? {
            ManagerWorkerEvent::RuntimesListed {
                result: Ok(page), ..
            } => page,
            _ => return Err(LiveGateFailure::stage("worker_live_runtime_result")),
        };
        let runtime = runtime_page
            .items
            .iter()
            .find(|runtime| runtime.runtime_id == LIVE_RUNTIME_ID)
            .ok_or_else(|| LiveGateFailure::stage("worker_live_runtime_missing"))?;
        if runtime.capability_state != CapabilityState::NeedsValidation {
            return Err(LiveGateFailure::stage("worker_live_capability_state"));
        }

        let start_request = guard
            .worker_mut()
            .try_start_profile(&profile_id, profile_revision)
            .map_err(|_| LiveGateFailure::stage("worker_live_start_submit"))?;
        let started = match wait_for_request_event(
            guard.worker_mut(),
            start_request,
            "worker_live_start_event",
        )? {
            ManagerWorkerEvent::ProfileStarted {
                result: Ok(started),
                ..
            } => started,
            ManagerWorkerEvent::ProfileStarted {
                result: Err(error), ..
            } if error.code() == ManagerErrorCode::CleanupPending => {
                let session_id = error
                    .retained_session()
                    .map(|session| session.session_id.clone())
                    .or_else(|| error.session_id().map(str::to_owned))
                    .ok_or_else(|| LiveGateFailure::stage("worker_live_start_retained"))?;
                guard.owner = LiveOwnerState::CleanupPending(session_id);
                return Err(LiveGateFailure::stage("worker_live_start_cleanup_pending"));
            }
            _ => return Err(LiveGateFailure::stage("worker_live_start_result")),
        };
        guard.owner = LiveOwnerState::Running(started.session_id.clone());

        let birth_id = guard
            .worker_mut()
            .running_birth_id_for_live_test(&started.session_id)
            .ok_or_else(|| LiveGateFailure::stage("worker_live_birth_identity"))?;
        if birth_id.pid() == 0 || birth_id.creation_time_100ns() == 0 {
            return Err(LiveGateFailure::stage("worker_live_birth_identity"));
        }
        let observation = open_identity_checked_process_for_metrics(
            birth_id.pid(),
            birth_id.creation_time_100ns(),
        )
        .map_err(|_| LiveGateFailure::stage("worker_live_observation_open"))?;
        if observation.birth_id() != birth_id {
            return Err(LiveGateFailure::stage("worker_live_observation_identity"));
        }
        guard.observation = Some(observation);

        require_qualified_ready_window(
            guard.observation()?,
            &profile_root,
            Instant::now() + WINDOW_READY_DEADLINE,
        )?;

        let observe_request = guard
            .worker_mut()
            .try_observe_session(&started.session_id)
            .map_err(|_| LiveGateFailure::stage("worker_live_observe_submit"))?;
        match wait_for_request_event(
            guard.worker_mut(),
            observe_request,
            "worker_live_observe_event",
        )? {
            ManagerWorkerEvent::SessionObserved {
                result: Ok(observation),
                ..
            } => require_running_view(observation, &started)?,
            _ => return Err(LiveGateFailure::stage("worker_live_observe_result")),
        }

        let stop_request = guard
            .worker_mut()
            .try_stop_session(&started.session_id)
            .map_err(|_| LiveGateFailure::stage("worker_live_stop_submit"))?;
        let exited = match wait_for_request_event(
            guard.worker_mut(),
            stop_request,
            "worker_live_stop_event",
        )? {
            ManagerWorkerEvent::SessionStopped {
                result: Ok(exited), ..
            } => exited,
            ManagerWorkerEvent::SessionStopped {
                result: Err(error), ..
            } if error.code() == ManagerErrorCode::CleanupPending => {
                guard.owner = LiveOwnerState::CleanupPending(started.session_id.clone());
                return Err(LiveGateFailure::stage("worker_live_stop_cleanup_pending"));
            }
            _ => return Err(LiveGateFailure::stage("worker_live_stop_result")),
        };
        if exited.session_id != started.session_id
            || exited.profile_id != started.profile_id
            || exited.profile_revision != started.profile_revision
            || exited.runtime_id != started.runtime_id
        {
            return Err(LiveGateFailure::stage("worker_live_exit_metadata"));
        }
        guard.owner = LiveOwnerState::Cleaned;

        let sessions_request = guard
            .worker_mut()
            .try_list_sessions()
            .map_err(|_| LiveGateFailure::stage("worker_live_sessions_submit"))?;
        match wait_for_request_event(
            guard.worker_mut(),
            sessions_request,
            "worker_live_sessions_event",
        )? {
            ManagerWorkerEvent::SessionsListed { sessions, .. } if sessions.is_empty() => {}
            _ => return Err(LiveGateFailure::stage("worker_live_sessions_remain")),
        }

        guard.request_shutdown()?;
        if let Err(error) = guard.await_shutdown()? {
            guard.cleanup_remaining(error.remaining_sessions())?;
            return Err(LiveGateFailure::stage("worker_live_shutdown_result"));
        }
        if guard.worker().state() != ManagerWorkerState::Closed {
            return Err(LiveGateFailure::stage("worker_live_shutdown_state"));
        }
        wait_for_process_signal(guard.observation()?, Instant::now() + STOP_DEADLINE)?;
        Ok(())
    })();

    guard.finish(live_result)
}

#[test]
#[ignore = "launches one provisioned exact Windows game through public ManagerWorker; run the dedicated worker live runner"]
fn exact_runtime_runs_through_public_manager_worker() {
    if let Err(error) = run_public_manager_worker_live_gate() {
        panic!("{error}");
    }
}

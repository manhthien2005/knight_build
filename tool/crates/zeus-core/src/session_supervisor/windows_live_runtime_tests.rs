use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::process_adapter::ProcessBirthId;
use crate::process_adapter::test_support::{
    ProbeChild, ProcessObservation, child_process_birth_id, decode_live_owner_record_bytes,
    live_owner_ready_path, open_identity_checked_process_for_metrics,
    open_identity_checked_process_for_termination_result, publish_live_owner_record,
    read_live_owner_record, terminate_process_observation,
};
use crate::runtime::CapabilityState;

use super::windows_live_test_support::{
    AGGREGATE_CPU_X100_LIMIT, AGGREGATE_GROWTH_LIMIT, AGGREGATE_HANDLE_LIMIT,
    AGGREGATE_MEMORY_LIMIT, AggregatePerformanceEvidenceV1, CONCURRENT_SESSION_COUNT,
    LIVE_DESCRIPTOR_SHA256, LIVE_RUNTIME_ID, LiveGateFailure, LiveMultiProfileFixture,
    LivePerformanceEvidenceV1, LiveRuntimeFixture, LiveTestDirectory, PER_SESSION_HANDLE_LIMIT,
    PER_SESSION_MEMORY_LIMIT, ProcessResourceSample, SAMPLE_COUNT, SAMPLE_INTERVAL,
    STABILIZATION_DURATION, START_DEADLINE, STOP_DEADLINE, SessionPerformanceEvidenceV1,
    WINDOW_POLL_CADENCE, WINDOW_READY_DEADLINE, prepare_live_child_runtime_fixture,
    prepare_live_multi_profile_fixture, prepare_live_runtime_fixture, require_live_runtime_root,
    sample_offsets, sample_process_resources, validate_parent_crash_guarded_root,
    wait_for_process_signal, wait_for_profile_config, wait_for_ready_window, window_is_responsive,
};
use super::{
    MAX_ACTIVE_SESSIONS, SessionFailure, SessionMetadata, SessionObservation, StartSessionFailure,
};

const LIVE_PERFORMANCE_PREFIX: &str = "ZEUS_LIVE_PERF_V1=";
const PARENT_CRASH_CHILD_TOKEN_ENV: &str = "ZEUS_LIVE_PARENT_CRASH_CHILD_TOKEN";
const PARENT_CRASH_ROOT_ENV: &str = "ZEUS_LIVE_PARENT_CRASH_ROOT";
const PARENT_CRASH_CHILD_FILTER: &str =
    "session_supervisor::windows_live_runtime_tests::live_runtime_parent_crash_child";

fn parent_crash_child_command(
    current_exe: &Path,
    runtime_root: &Path,
    guarded_root: &Path,
    child_token: &str,
) -> Command {
    let mut command = Command::new(current_exe);
    command
        .arg("--exact")
        .arg(PARENT_CRASH_CHILD_FILTER)
        .arg("--ignored")
        .arg("--nocapture")
        .env_clear()
        .env("ZEUS_EXACT_RUNTIME_ROOT", runtime_root)
        .env(PARENT_CRASH_CHILD_TOKEN_ENV, child_token)
        .env(PARENT_CRASH_ROOT_ENV, guarded_root)
        .env(
            "ZEUS_LIVE_RUNTIME_BRIDGE",
            super::windows_live_test_support::LIVE_APPROVAL_TOKEN,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

enum CleanupState {
    NotStarted,
    Running {
        session_id: Uuid,
        birth_id: ProcessBirthId,
        observation: Option<ProcessObservation>,
    },
    CleanupPending {
        session_id: Uuid,
    },
}

struct SingleSessionCleanupGuard {
    fixture: Option<LiveRuntimeFixture>,
    state: CleanupState,
}

impl SingleSessionCleanupGuard {
    fn new(fixture: LiveRuntimeFixture) -> Self {
        Self {
            fixture: Some(fixture),
            state: CleanupState::NotStarted,
        }
    }

    fn fixture(&self) -> &LiveRuntimeFixture {
        self.fixture
            .as_ref()
            .expect("live fixture should remain owned until confirmed cleanup")
    }

    fn fixture_mut(&mut self) -> &mut LiveRuntimeFixture {
        self.fixture
            .as_mut()
            .expect("live fixture should remain owned until confirmed cleanup")
    }

    fn record_running(&mut self, session_id: Uuid, birth_id: ProcessBirthId) {
        self.state = CleanupState::Running {
            session_id,
            birth_id,
            observation: None,
        };
    }

    fn record_observation(&mut self, observation: ProcessObservation) {
        match &mut self.state {
            CleanupState::Running {
                observation: owned, ..
            } => *owned = Some(observation),
            _ => panic!("process observation requires a running live session"),
        }
    }

    fn observation(&self) -> Result<&ProcessObservation, LiveGateFailure> {
        match &self.state {
            CleanupState::Running {
                observation: Some(observation),
                ..
            } => Ok(observation),
            _ => Err(LiveGateFailure::stage("live_observation_missing")),
        }
    }

    fn finish(mut self, original: Result<(), LiveGateFailure>) -> Result<(), LiveGateFailure> {
        let original = original.map_err(|error| {
            if error.stage_name() == "window_process_exited" {
                self.classify_early_exit()
            } else {
                error
            }
        });
        match self.cleanup() {
            Ok(()) => original,
            Err(cleanup) => Err(cleanup),
        }
    }

    fn classify_early_exit(&self) -> LiveGateFailure {
        let profile_root = &self.fixture().profile_root;
        let config = profile_root.join("microemu-home/.microemulator/config2.xml");
        if config.is_file() {
            return LiveGateFailure::stage("live_exit_after_config");
        }
        let microemu_state = profile_root.join("microemu-home/.microemulator");
        if microemu_state.is_dir() {
            return LiveGateFailure::stage("live_exit_after_microemu_state");
        }
        let has_jvm_error_artifact = fs::read_dir(profile_root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .any(|name| name.starts_with("hs_err_pid") && name.ends_with(".log"));
        if has_jvm_error_artifact {
            LiveGateFailure::stage("live_exit_with_jvm_error_artifact")
        } else {
            LiveGateFailure::stage("live_exit_before_microemu_state")
        }
    }

    fn cleanup(&mut self) -> Result<(), LiveGateFailure> {
        let cleanup_deadline = Instant::now() + STOP_DEADLINE;
        match &self.state {
            CleanupState::NotStarted => {
                if self.fixture().supervisor.active_session_count() != 0 {
                    return Err(LiveGateFailure::stage("cleanup_untracked_session"));
                }
            }
            CleanupState::CleanupPending { session_id } => {
                let session_id = *session_id;
                self.fixture_mut()
                    .supervisor
                    .retry_cleanup(session_id, cleanup_deadline)
                    .map_err(|_| LiveGateFailure::stage("cleanup_retry"))?;
            }
            CleanupState::Running {
                session_id,
                birth_id,
                ..
            } => {
                let session_id = *session_id;
                let birth_id = *birth_id;
                if self.fixture().supervisor.active_session_count() != 0 {
                    let stopped = self
                        .fixture_mut()
                        .supervisor
                        .stop_session(session_id, cleanup_deadline)
                        .map_err(|_| LiveGateFailure::stage("cleanup_stop"))?;
                    if stopped.birth_id() != birth_id {
                        return Err(LiveGateFailure::stage("cleanup_birth_identity"));
                    }
                }
                wait_for_process_signal(self.observation()?, cleanup_deadline)?;
            }
        }

        if self.fixture().supervisor.active_session_count() != 0 {
            return Err(LiveGateFailure::stage("cleanup_active_sessions"));
        }
        let capability = self
            .fixture()
            .supervisor
            .core()
            .inspect_runtime(LIVE_RUNTIME_ID)
            .map_err(|_| LiveGateFailure::stage("cleanup_runtime_inspect"))?
            .capability_state;
        if capability != CapabilityState::NeedsValidation {
            return Err(LiveGateFailure::stage("cleanup_capability_state"));
        }

        let fixture = self
            .fixture
            .take()
            .expect("confirmed cleanup should still own the live fixture");
        let root = fixture.directory.path().to_owned();
        drop(fixture.supervisor);
        fixture.directory.remove_after_cleanup_confirmation()?;
        if root.exists() {
            return Err(LiveGateFailure::stage("cleanup_root_remains"));
        }
        Ok(())
    }
}

fn verify_metadata(
    metadata: &SessionMetadata,
    expected_profile_id: Uuid,
    expected_revision: i64,
) -> Result<(), LiveGateFailure> {
    if metadata.profile_id() != expected_profile_id
        || metadata.profile_revision() != expected_revision
        || metadata.runtime_id() != LIVE_RUNTIME_ID
        || metadata.descriptor_sha256() != LIVE_DESCRIPTOR_SHA256
    {
        return Err(LiveGateFailure::stage("live_session_metadata"));
    }
    Ok(())
}

fn run_single_profile_live_gate() -> Result<(), LiveGateFailure> {
    let fixture = prepare_live_runtime_fixture("Live Bridge Single")?;
    let expected_profile_id = Uuid::parse_str(&fixture.profile_id)
        .map_err(|_| LiveGateFailure::stage("live_profile_identity"))?;
    let expected_revision = fixture.profile_revision;
    let profile_id = fixture.profile_id.clone();
    let started_at = Instant::now();
    let mut guard = SingleSessionCleanupGuard::new(fixture);

    let started = match guard.fixture_mut().supervisor.start_session(
        &profile_id,
        expected_revision,
        started_at + START_DEADLINE,
    ) {
        Ok(started) => started,
        Err(StartSessionFailure::CleanupPending { session_id, .. }) => {
            guard.state = CleanupState::CleanupPending { session_id };
            return guard.finish(Err(LiveGateFailure::stage("live_start_cleanup_pending")));
        }
        Err(_) => return guard.finish(Err(LiveGateFailure::stage("live_start"))),
    };

    let session_id = started.metadata().session_id();
    let birth_id = started.birth_id();
    guard.record_running(session_id, birth_id);

    let live_result = (|| {
        verify_metadata(started.metadata(), expected_profile_id, expected_revision)?;
        if birth_id.pid() == 0 || birth_id.creation_time_100ns() == 0 {
            return Err(LiveGateFailure::stage("live_birth_identity"));
        }
        let observation = open_identity_checked_process_for_metrics(
            birth_id.pid(),
            birth_id.creation_time_100ns(),
        )
        .map_err(|_| LiveGateFailure::stage("live_observation_open"))?;
        if observation.birth_id() != birth_id {
            return Err(LiveGateFailure::stage("live_observation_identity"));
        }
        guard.record_observation(observation);

        if guard.fixture().supervisor.active_session_count() != 1 {
            return Err(LiveGateFailure::stage("live_active_session_count"));
        }
        let readiness_deadline = Instant::now() + WINDOW_READY_DEADLINE;
        match guard
            .fixture_mut()
            .supervisor
            .observe_session(session_id, readiness_deadline)
            .map_err(|_| LiveGateFailure::stage("live_observe"))?
        {
            SessionObservation::Running {
                metadata,
                birth_id: observed_birth,
            } => {
                verify_metadata(&metadata, expected_profile_id, expected_revision)?;
                if observed_birth != birth_id {
                    return Err(LiveGateFailure::stage("live_observed_birth_identity"));
                }
            }
            _ => return Err(LiveGateFailure::stage("live_observed_state")),
        }

        let window = wait_for_ready_window(guard.observation()?, readiness_deadline)?;
        let config = wait_for_profile_config(&guard.fixture().profile_root, readiness_deadline)?;
        if !config.starts_with(&guard.fixture().profile_root) {
            return Err(LiveGateFailure::stage("live_config_containment"));
        }
        match guard
            .fixture_mut()
            .supervisor
            .observe_session(session_id, readiness_deadline)
            .map_err(|_| LiveGateFailure::stage("live_observe_ready"))?
        {
            SessionObservation::Running {
                metadata,
                birth_id: observed_birth,
            } => {
                verify_metadata(&metadata, expected_profile_id, expected_revision)?;
                if observed_birth != birth_id {
                    return Err(LiveGateFailure::stage("live_ready_birth_identity"));
                }
            }
            _ => return Err(LiveGateFailure::stage("live_ready_state")),
        }
        if guard
            .observation()?
            .is_signaled()
            .map_err(|_| LiveGateFailure::stage("live_ready_process_wait"))?
        {
            return Err(LiveGateFailure::stage("live_ready_process_exited"));
        }
        if !window_is_responsive(&window, birth_id.pid())? {
            return Err(LiveGateFailure::stage("live_ready_window"));
        }
        let start_to_window_ms = u64::try_from(started_at.elapsed().as_millis())
            .map_err(|_| LiveGateFailure::stage("live_start_to_window_overflow"))?;
        if start_to_window_ms == 0 {
            return Err(LiveGateFailure::stage("live_start_to_window_value"));
        }
        if guard
            .fixture()
            .supervisor
            .core()
            .inspect_runtime(LIVE_RUNTIME_ID)
            .map_err(|_| LiveGateFailure::stage("live_runtime_inspect"))?
            .capability_state
            != CapabilityState::NeedsValidation
        {
            return Err(LiveGateFailure::stage("live_capability_state"));
        }
        Ok(())
    })();

    guard.finish(live_result)
}

struct FourSession {
    index: usize,
    session_id: Uuid,
    birth_id: ProcessBirthId,
    observation: Option<ProcessObservation>,
    profile_root: PathBuf,
    microemu_home: Option<PathBuf>,
    temp_root: Option<PathBuf>,
    config: Option<PathBuf>,
    window: Option<super::windows_live_test_support::ReadyWindow>,
    start_to_window_ms: Option<u64>,
    confirmed_removed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupIntent {
    Success,
    Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupAction {
    Stop,
    ObserveSignaled,
    AcceptPreviouslyRemoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CleanupPlan {
    action: CleanupAction,
    success_violation: bool,
}

fn cleanup_plan(intent: CleanupIntent, signaled: bool, confirmed_removed: bool) -> CleanupPlan {
    match (intent, signaled, confirmed_removed) {
        (CleanupIntent::Success, false, false) => CleanupPlan {
            action: CleanupAction::Stop,
            success_violation: false,
        },
        (CleanupIntent::Success, _, _) => CleanupPlan {
            action: if confirmed_removed {
                CleanupAction::AcceptPreviouslyRemoved
            } else {
                CleanupAction::ObserveSignaled
            },
            success_violation: true,
        },
        (CleanupIntent::Failure, _, true) => CleanupPlan {
            action: CleanupAction::AcceptPreviouslyRemoved,
            success_violation: false,
        },
        (CleanupIntent::Failure, true, false) => CleanupPlan {
            action: CleanupAction::ObserveSignaled,
            success_violation: false,
        },
        (CleanupIntent::Failure, false, false) => CleanupPlan {
            action: CleanupAction::Stop,
            success_violation: false,
        },
    }
}

struct FourSessionCleanupGuard {
    fixture: Option<LiveMultiProfileFixture>,
    sessions: Vec<FourSession>,
    cleanup_pending: Vec<Uuid>,
}

impl FourSessionCleanupGuard {
    fn new(fixture: LiveMultiProfileFixture) -> Self {
        Self {
            fixture: Some(fixture),
            sessions: Vec::with_capacity(CONCURRENT_SESSION_COUNT),
            cleanup_pending: Vec::new(),
        }
    }

    fn fixture(&self) -> &LiveMultiProfileFixture {
        self.fixture
            .as_ref()
            .expect("four-profile fixture should remain owned until cleanup")
    }

    fn fixture_mut(&mut self) -> &mut LiveMultiProfileFixture {
        self.fixture
            .as_mut()
            .expect("four-profile fixture should remain owned until cleanup")
    }

    fn finish<T>(mut self, original: Result<T, LiveGateFailure>) -> Result<T, LiveGateFailure> {
        let intent = if original.is_ok() {
            CleanupIntent::Success
        } else {
            CleanupIntent::Failure
        };
        match self.cleanup(intent) {
            Ok(()) => original,
            Err(cleanup) => Err(cleanup),
        }
    }

    fn cleanup(&mut self, intent: CleanupIntent) -> Result<(), LiveGateFailure> {
        let mut first_error = None;
        let mut owners_confirmed = true;
        let cleanup_pending = self.cleanup_pending.clone();
        for session_id in cleanup_pending.into_iter().rev() {
            if self
                .fixture_mut()
                .supervisor
                .retry_cleanup(session_id, Instant::now() + STOP_DEADLINE)
                .is_err()
            {
                retain_first_error(
                    &mut first_error,
                    LiveGateFailure::stage("four_cleanup_retry"),
                );
                owners_confirmed = false;
            }
        }

        for index in (0..self.sessions.len()).rev() {
            let deadline = Instant::now() + STOP_DEADLINE;
            let session_id = self.sessions[index].session_id;
            let birth_id = self.sessions[index].birth_id;
            let confirmed_removed = self.sessions[index].confirmed_removed;
            let signaled = match self.sessions[index]
                .observation
                .as_ref()
                .map(|observation| observation.is_signaled())
                .transpose()
            {
                Ok(value) => value.unwrap_or(false),
                Err(_) => {
                    retain_first_error(
                        &mut first_error,
                        LiveGateFailure::stage("four_cleanup_process_wait"),
                    );
                    false
                }
            };
            let plan = cleanup_plan(intent, signaled, confirmed_removed);
            if plan.success_violation {
                retain_first_error(
                    &mut first_error,
                    LiveGateFailure::stage("four_success_root_signaled"),
                );
            }

            let mut owner_confirmed = match plan.action {
                CleanupAction::Stop => match self
                    .fixture_mut()
                    .supervisor
                    .stop_session(session_id, deadline)
                {
                    Ok(stopped) if stopped.birth_id() == birth_id => true,
                    Ok(_) => {
                        retain_first_error(
                            &mut first_error,
                            LiveGateFailure::stage("four_cleanup_birth_identity"),
                        );
                        false
                    }
                    Err(SessionFailure::SessionNotFound { .. }) if confirmed_removed => true,
                    Err(_) => {
                        retain_first_error(
                            &mut first_error,
                            LiveGateFailure::stage("four_cleanup_stop"),
                        );
                        false
                    }
                },
                CleanupAction::ObserveSignaled => match self
                    .fixture_mut()
                    .supervisor
                    .observe_session(session_id, deadline)
                {
                    Ok(SessionObservation::Exited(exit)) if exit.birth_id() == birth_id => {
                        self.sessions[index].confirmed_removed = true;
                        true
                    }
                    Err(SessionFailure::SessionNotFound { .. }) if confirmed_removed => true,
                    _ => {
                        retain_first_error(
                            &mut first_error,
                            LiveGateFailure::stage("four_cleanup_observed_exit"),
                        );
                        false
                    }
                },
                CleanupAction::AcceptPreviouslyRemoved => true,
            };

            if !owner_confirmed && plan.action == CleanupAction::ObserveSignaled {
                owner_confirmed = match self
                    .fixture_mut()
                    .supervisor
                    .stop_session(session_id, deadline)
                {
                    Ok(stopped) if stopped.birth_id() == birth_id => true,
                    Err(SessionFailure::SessionNotFound { .. }) if confirmed_removed => true,
                    _ => false,
                };
            }

            if owner_confirmed {
                if let Some(observation) = &self.sessions[index].observation {
                    if wait_for_process_signal(observation, deadline).is_err() {
                        retain_first_error(
                            &mut first_error,
                            LiveGateFailure::stage("four_cleanup_process_deadline"),
                        );
                        owner_confirmed = false;
                    }
                } else if intent == CleanupIntent::Success {
                    retain_first_error(
                        &mut first_error,
                        LiveGateFailure::stage("four_cleanup_observation_missing"),
                    );
                    owner_confirmed = false;
                }
            }
            owners_confirmed &= owner_confirmed;
        }

        let active_zero = self.fixture().supervisor.active_session_count() == 0;
        if !active_zero {
            retain_first_error(
                &mut first_error,
                LiveGateFailure::stage("four_cleanup_active_sessions"),
            );
        }
        let capability_confirmed = self
            .fixture()
            .supervisor
            .core()
            .inspect_runtime(LIVE_RUNTIME_ID)
            .map(|runtime| runtime.capability_state == CapabilityState::NeedsValidation)
            .unwrap_or(false);
        if !capability_confirmed {
            retain_first_error(
                &mut first_error,
                LiveGateFailure::stage("four_cleanup_capability_state"),
            );
        }

        if owners_confirmed && active_zero && capability_confirmed {
            let fixture = self
                .fixture
                .take()
                .expect("confirmed cleanup should own the four-profile fixture");
            let root = fixture.directory.path().to_owned();
            drop(fixture.supervisor);
            if fixture
                .directory
                .remove_after_cleanup_confirmation()
                .is_err()
                || root.exists()
            {
                retain_first_error(
                    &mut first_error,
                    LiveGateFailure::stage("four_cleanup_root_remains"),
                );
            }
        } else {
            owners_confirmed = false;
        }

        if !owners_confirmed && first_error.is_none() {
            first_error = Some(LiveGateFailure::stage("four_cleanup_unconfirmed"));
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

fn retain_first_error(first: &mut Option<LiveGateFailure>, error: LiveGateFailure) {
    if first.is_none() {
        *first = Some(error);
    }
}

fn verify_running_session(
    supervisor: &mut super::SessionSupervisor,
    session: &mut FourSession,
) -> Result<(), LiveGateFailure> {
    let observation = session
        .observation
        .as_ref()
        .ok_or_else(|| LiveGateFailure::stage("four_observation_missing"))?;
    if observation
        .is_signaled()
        .map_err(|_| LiveGateFailure::stage("four_process_wait"))?
    {
        if let Ok(SessionObservation::Exited(exit)) =
            supervisor.observe_session(session.session_id, Instant::now() + STOP_DEADLINE)
        {
            if exit.birth_id() != session.birth_id {
                return Err(LiveGateFailure::stage("four_observed_birth_identity"));
            }
            session.confirmed_removed = true;
        }
        return Err(LiveGateFailure::stage("four_process_exited"));
    }
    match supervisor
        .observe_session(session.session_id, Instant::now() + STOP_DEADLINE)
        .map_err(|_| LiveGateFailure::stage("four_observe"))?
    {
        SessionObservation::Running { metadata, birth_id } => {
            let expected_profile_id = Uuid::parse_str(
                session
                    .profile_root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| LiveGateFailure::stage("four_profile_identity"))?,
            )
            .map_err(|_| LiveGateFailure::stage("four_profile_identity"))?;
            verify_metadata(&metadata, expected_profile_id, 1)?;
            if birth_id != session.birth_id {
                return Err(LiveGateFailure::stage("four_observed_birth_identity"));
            }
        }
        SessionObservation::Exited(exit) => {
            if exit.birth_id() != session.birth_id {
                return Err(LiveGateFailure::stage("four_observed_birth_identity"));
            }
            session.confirmed_removed = true;
            return Err(LiveGateFailure::stage("four_process_exited"));
        }
        SessionObservation::CleanupPending { .. } => {
            return Err(LiveGateFailure::stage("four_observed_state"));
        }
    }
    let window = session
        .window
        .as_ref()
        .ok_or_else(|| LiveGateFailure::stage("four_window_missing"))?;
    if !window_is_responsive(window, session.birth_id.pid())? {
        return Err(LiveGateFailure::stage("four_window_unresponsive"));
    }
    Ok(())
}

fn checked_sum_u64(values: impl IntoIterator<Item = u64>) -> Result<u64, LiveGateFailure> {
    values.into_iter().try_fold(0u64, |sum, value| {
        sum.checked_add(value)
            .ok_or_else(|| LiveGateFailure::stage("four_metric_sum_overflow"))
    })
}

fn checked_sum_u32(values: impl IntoIterator<Item = u32>) -> Result<u32, LiveGateFailure> {
    values.into_iter().try_fold(0u32, |sum, value| {
        sum.checked_add(value)
            .ok_or_else(|| LiveGateFailure::stage("four_metric_sum_overflow"))
    })
}

fn duration_100ns(duration: Duration) -> Result<u64, LiveGateFailure> {
    let units = duration.as_nanos() / 100;
    if units == 0 {
        return Err(LiveGateFailure::stage("four_elapsed_value"));
    }
    u64::try_from(units).map_err(|_| LiveGateFailure::stage("four_elapsed_overflow"))
}

fn cpu_percent_one_core_x100(
    first: u64,
    final_value: u64,
    elapsed_100ns: u64,
) -> Result<u64, LiveGateFailure> {
    let delta = final_value
        .checked_sub(first)
        .ok_or_else(|| LiveGateFailure::stage("four_cpu_regression"))?;
    let numerator = u128::from(delta)
        .checked_mul(10_000)
        .ok_or_else(|| LiveGateFailure::stage("four_cpu_overflow"))?;
    let rounded = numerator
        .checked_add(u128::from(elapsed_100ns) / 2)
        .ok_or_else(|| LiveGateFailure::stage("four_cpu_overflow"))?
        / u128::from(elapsed_100ns);
    u64::try_from(rounded).map_err(|_| LiveGateFailure::stage("four_cpu_overflow"))
}

fn require_aggregate_interval_cpu_within_limit(
    samples: &[ProcessResourceSample],
    capture_offsets: &[Duration],
) -> Result<(), LiveGateFailure> {
    if samples.len() != capture_offsets.len() || samples.len() < 2 {
        return Err(LiveGateFailure::stage("four_interval_sample_count"));
    }
    for index in 1..samples.len() {
        let elapsed = capture_offsets[index]
            .checked_sub(capture_offsets[index - 1])
            .ok_or_else(|| LiveGateFailure::stage("four_elapsed_regression"))?;
        let interval_cpu = cpu_percent_one_core_x100(
            samples[index - 1].cpu_time_100ns,
            samples[index].cpu_time_100ns,
            duration_100ns(elapsed)?,
        )?;
        if interval_cpu > AGGREGATE_CPU_X100_LIMIT {
            return Err(LiveGateFailure::stage("four_aggregate_interval_cpu_limit"));
        }
    }
    Ok(())
}

fn signed_growth(first: u64, final_value: u64) -> Result<i64, LiveGateFailure> {
    let growth = i128::from(final_value) - i128::from(first);
    i64::try_from(growth).map_err(|_| LiveGateFailure::stage("four_growth_overflow"))
}

fn wait_for_absolute_target(target: Instant) -> Result<Instant, LiveGateFailure> {
    if let Some(remaining) = target.checked_duration_since(Instant::now()) {
        thread::sleep(remaining);
    }
    let captured = Instant::now();
    if captured.duration_since(target) > Duration::from_secs(1) {
        return Err(LiveGateFailure::stage("four_sample_late"));
    }
    Ok(captured)
}

fn require_path_isolation(sessions: &[FourSession]) -> Result<(), LiveGateFailure> {
    for left in 0..sessions.len() {
        for right in (left + 1)..sessions.len() {
            let left_paths = [
                sessions[left].profile_root.as_path(),
                sessions[left]
                    .microemu_home
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
                sessions[left]
                    .temp_root
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
                sessions[left]
                    .config
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
            ];
            let right_paths = [
                sessions[right].profile_root.as_path(),
                sessions[right]
                    .microemu_home
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
                sessions[right]
                    .temp_root
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
                sessions[right]
                    .config
                    .as_deref()
                    .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?,
            ];
            for left_path in left_paths {
                for right_path in right_paths {
                    if left_path == right_path
                        || left_path.starts_with(&sessions[right].profile_root)
                        || right_path.starts_with(&sessions[left].profile_root)
                    {
                        return Err(LiveGateFailure::stage("four_profile_path_isolation"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn build_performance_evidence(
    samples: &[Vec<ProcessResourceSample>; CONCURRENT_SESSION_COUNT],
    aggregate_samples: &[ProcessResourceSample],
    elapsed: Duration,
    start_to_window_ms: [u64; CONCURRENT_SESSION_COUNT],
) -> Result<LivePerformanceEvidenceV1, LiveGateFailure> {
    if samples.iter().any(|values| values.len() != SAMPLE_COUNT)
        || aggregate_samples.len() != SAMPLE_COUNT
    {
        return Err(LiveGateFailure::stage("four_sample_count"));
    }
    let elapsed_100ns = duration_100ns(elapsed)?;
    let mut per_session = Vec::with_capacity(CONCURRENT_SESSION_COUNT);
    for (index, values) in samples.iter().enumerate() {
        let first = values[0];
        let final_value = values[SAMPLE_COUNT - 1];
        let evidence = SessionPerformanceEvidenceV1 {
            index: u32::try_from(index + 1)
                .map_err(|_| LiveGateFailure::stage("four_session_index"))?,
            max_working_set_bytes: values
                .iter()
                .map(|value| value.working_set_bytes)
                .max()
                .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
            final_working_set_bytes: final_value.working_set_bytes,
            max_private_bytes: values
                .iter()
                .map(|value| value.private_bytes)
                .max()
                .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
            final_private_bytes: final_value.private_bytes,
            max_handle_count: values
                .iter()
                .map(|value| value.handle_count)
                .max()
                .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
            cpu_percent_one_core_x100: cpu_percent_one_core_x100(
                first.cpu_time_100ns,
                final_value.cpu_time_100ns,
                elapsed_100ns,
            )?,
        };
        if evidence.max_working_set_bytes > PER_SESSION_MEMORY_LIMIT
            || evidence.final_working_set_bytes > PER_SESSION_MEMORY_LIMIT
            || evidence.max_private_bytes > PER_SESSION_MEMORY_LIMIT
            || evidence.final_private_bytes > PER_SESSION_MEMORY_LIMIT
            || evidence.max_handle_count > PER_SESSION_HANDLE_LIMIT
        {
            return Err(LiveGateFailure::stage("four_session_evidence_limit"));
        }
        per_session.push(evidence);
    }
    let per_session = per_session
        .try_into()
        .map_err(|_| LiveGateFailure::stage("four_session_evidence_count"))?;

    let first = aggregate_samples[0];
    let final_value = aggregate_samples[SAMPLE_COUNT - 1];
    let aggregate = AggregatePerformanceEvidenceV1 {
        first_working_set_bytes: first.working_set_bytes,
        max_working_set_bytes: aggregate_samples
            .iter()
            .map(|value| value.working_set_bytes)
            .max()
            .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
        final_working_set_bytes: final_value.working_set_bytes,
        working_set_growth_bytes: signed_growth(
            first.working_set_bytes,
            final_value.working_set_bytes,
        )?,
        first_private_bytes: first.private_bytes,
        max_private_bytes: aggregate_samples
            .iter()
            .map(|value| value.private_bytes)
            .max()
            .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
        final_private_bytes: final_value.private_bytes,
        private_growth_bytes: signed_growth(first.private_bytes, final_value.private_bytes)?,
        max_handle_count: aggregate_samples
            .iter()
            .map(|value| value.handle_count)
            .max()
            .ok_or_else(|| LiveGateFailure::stage("four_sample_count"))?,
        cpu_percent_one_core_x100: cpu_percent_one_core_x100(
            first.cpu_time_100ns,
            final_value.cpu_time_100ns,
            elapsed_100ns,
        )?,
    };
    if aggregate.first_working_set_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.max_working_set_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.final_working_set_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.first_private_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.max_private_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.final_private_bytes > AGGREGATE_MEMORY_LIMIT
        || aggregate.max_handle_count > AGGREGATE_HANDLE_LIMIT
        || aggregate.cpu_percent_one_core_x100 > AGGREGATE_CPU_X100_LIMIT
        || aggregate.working_set_growth_bytes > AGGREGATE_GROWTH_LIMIT
        || aggregate.private_growth_bytes > AGGREGATE_GROWTH_LIMIT
    {
        return Err(LiveGateFailure::stage("four_aggregate_evidence_limit"));
    }

    Ok(LivePerformanceEvidenceV1 {
        schema_version: 1,
        runtime_id: LIVE_RUNTIME_ID,
        concurrent_sessions: 4,
        stabilization_seconds: STABILIZATION_DURATION.as_secs(),
        observation_seconds: 60,
        sample_interval_seconds: SAMPLE_INTERVAL.as_secs(),
        samples_per_session: u32::try_from(SAMPLE_COUNT)
            .map_err(|_| LiveGateFailure::stage("four_sample_count"))?,
        representative_of_1gib_target: false,
        capacity_rejection_confirmed: true,
        all_windows_responsive: true,
        cleanup_confirmed: true,
        start_to_window_ms,
        per_session,
        aggregate,
    })
}

fn live_performance_record_line(
    evidence: &LivePerformanceEvidenceV1,
) -> Result<String, LiveGateFailure> {
    let json = serde_json::to_string(evidence)
        .map_err(|_| LiveGateFailure::stage("four_evidence_serialize"))?;
    if json.is_empty() || json.len() > 64 * 1024 {
        return Err(LiveGateFailure::stage("four_evidence_size"));
    }
    let mut line = String::with_capacity(LIVE_PERFORMANCE_PREFIX.len() + json.len());
    line.push_str(LIVE_PERFORMANCE_PREFIX);
    line.push_str(&json);
    Ok(line)
}

fn run_four_profile_live_gate() -> Result<LivePerformanceEvidenceV1, LiveGateFailure> {
    let fixture = prepare_live_multi_profile_fixture()?;
    let mut guard = FourSessionCleanupGuard::new(fixture);

    let live_result = (|| {
        for index in 0..CONCURRENT_SESSION_COUNT {
            let profile_id = guard.fixture().profiles[index].profile_id.clone();
            let revision = guard.fixture().profiles[index].profile_revision;
            let profile_root = guard.fixture().profiles[index].profile_root.clone();
            let expected_profile_id = Uuid::parse_str(&profile_id)
                .map_err(|_| LiveGateFailure::stage("four_profile_identity"))?;
            let started_at = Instant::now();
            let started = match guard.fixture_mut().supervisor.start_session(
                &profile_id,
                revision,
                started_at + START_DEADLINE,
            ) {
                Ok(started) => started,
                Err(StartSessionFailure::CleanupPending { session_id, .. }) => {
                    guard.cleanup_pending.push(session_id);
                    return Err(LiveGateFailure::stage("four_start_cleanup_pending"));
                }
                Err(_) => return Err(LiveGateFailure::stage("four_start")),
            };
            let session_id = started.metadata().session_id();
            let birth_id = started.birth_id();
            let duplicate_identity = guard
                .sessions
                .iter()
                .any(|session| session.session_id == session_id || session.birth_id == birth_id);
            guard.sessions.push(FourSession {
                index,
                session_id,
                birth_id,
                observation: None,
                profile_root,
                microemu_home: None,
                temp_root: None,
                config: None,
                window: None,
                start_to_window_ms: None,
                confirmed_removed: false,
            });
            verify_metadata(started.metadata(), expected_profile_id, revision)?;
            if birth_id.pid() == 0 || birth_id.creation_time_100ns() == 0 {
                return Err(LiveGateFailure::stage("four_birth_identity"));
            }
            if duplicate_identity {
                return Err(LiveGateFailure::stage("four_session_identity_unique"));
            }
            let observation = open_identity_checked_process_for_metrics(
                birth_id.pid(),
                birth_id.creation_time_100ns(),
            )
            .map_err(|_| LiveGateFailure::stage("four_observation_open"))?;
            if observation.birth_id() != birth_id {
                return Err(LiveGateFailure::stage("four_observation_identity"));
            }
            guard.sessions[index].observation = Some(observation);
            let readiness_deadline = started_at + WINDOW_READY_DEADLINE;
            let window = wait_for_ready_window(
                guard.sessions[index]
                    .observation
                    .as_ref()
                    .expect("live observation was just installed"),
                readiness_deadline,
            )?;
            let start_to_window_ms = u64::try_from(started_at.elapsed().as_millis())
                .map_err(|_| LiveGateFailure::stage("four_start_to_window_overflow"))?;
            if start_to_window_ms == 0 {
                return Err(LiveGateFailure::stage("four_start_to_window_value"));
            }
            let config =
                wait_for_profile_config(&guard.sessions[index].profile_root, readiness_deadline)?;
            let microemu_home =
                fs::canonicalize(guard.sessions[index].profile_root.join("microemu-home"))
                    .map_err(|_| LiveGateFailure::stage("four_microemu_home"))?;
            let temp_root = fs::canonicalize(guard.sessions[index].profile_root.join("temp"))
                .map_err(|_| LiveGateFailure::stage("four_temp_root"))?;
            if config.parent().and_then(Path::parent) != Some(microemu_home.as_path())
                || microemu_home.parent() != Some(guard.sessions[index].profile_root.as_path())
                || temp_root.parent() != Some(guard.sessions[index].profile_root.as_path())
                || !window_is_responsive(&window, birth_id.pid())?
            {
                return Err(LiveGateFailure::stage("four_readiness_identity"));
            }
            guard.sessions[index].microemu_home = Some(microemu_home);
            guard.sessions[index].temp_root = Some(temp_root);
            guard.sessions[index].config = Some(config);
            guard.sessions[index].window = Some(window);
            guard.sessions[index].start_to_window_ms = Some(start_to_window_ms);
        }

        if guard.fixture().supervisor.active_session_count() != MAX_ACTIVE_SESSIONS
            || guard.sessions.len() != CONCURRENT_SESSION_COUNT
        {
            return Err(LiveGateFailure::stage("four_active_session_count"));
        }
        require_path_isolation(&guard.sessions)?;

        let fifth = &guard.fixture().profiles[CONCURRENT_SESSION_COUNT];
        let fifth_profile_root = fifth.profile_root.clone();
        let fifth_microemu_home = fifth_profile_root.join("microemu-home");
        let fifth_temp = fifth_profile_root.join("temp");
        if fifth_microemu_home.exists() || fifth_temp.exists() {
            return Err(LiveGateFailure::stage("four_fifth_preexisting_artifact"));
        }
        let fifth_profile_id = fifth.profile_id.clone();
        let fifth_revision = fifth.profile_revision;
        match guard.fixture_mut().supervisor.start_session(
            &fifth_profile_id,
            fifth_revision,
            Instant::now() + START_DEADLINE,
        ) {
            Err(StartSessionFailure::CapacityReached { maximum })
                if maximum == MAX_ACTIVE_SESSIONS => {}
            Ok(started) => {
                guard.sessions.push(FourSession {
                    index: CONCURRENT_SESSION_COUNT,
                    session_id: started.metadata().session_id(),
                    birth_id: started.birth_id(),
                    observation: None,
                    profile_root: fifth_profile_root,
                    microemu_home: None,
                    temp_root: None,
                    config: None,
                    window: None,
                    start_to_window_ms: None,
                    confirmed_removed: false,
                });
                return Err(LiveGateFailure::stage("four_fifth_spawned"));
            }
            Err(_) => return Err(LiveGateFailure::stage("four_fifth_rejection")),
        }
        if guard.fixture().supervisor.active_session_count() != MAX_ACTIVE_SESSIONS
            || fifth_microemu_home.exists()
            || fifth_temp.exists()
        {
            return Err(LiveGateFailure::stage("four_fifth_admission_artifact"));
        }

        let stabilization_base = Instant::now();
        for second in 0..=STABILIZATION_DURATION.as_secs() {
            let target = stabilization_base + Duration::from_secs(second);
            if let Some(remaining) = target.checked_duration_since(Instant::now()) {
                thread::sleep(remaining);
            }
            let (fixture, sessions) = (
                guard.fixture.as_mut().expect("fixture should remain owned"),
                &mut guard.sessions,
            );
            for session in sessions.iter_mut() {
                verify_running_session(&mut fixture.supervisor, session)?;
            }
            if Instant::now().duration_since(target) > Duration::from_secs(1) {
                return Err(LiveGateFailure::stage("four_stabilization_rate"));
            }
        }

        let mut samples: [Vec<ProcessResourceSample>; CONCURRENT_SESSION_COUNT] =
            std::array::from_fn(|_| Vec::with_capacity(SAMPLE_COUNT));
        let mut aggregate_samples = Vec::with_capacity(SAMPLE_COUNT);
        let mut capture_offsets = Vec::with_capacity(SAMPLE_COUNT);
        let sample_base = Instant::now();
        let mut first_capture = None;
        let mut final_capture = None;
        for (sample_index, offset) in sample_offsets().into_iter().enumerate() {
            let captured = wait_for_absolute_target(sample_base + offset)?;
            if sample_index == 0 {
                first_capture = Some(captured);
            }
            if sample_index == SAMPLE_COUNT - 1 {
                final_capture = Some(captured);
            }
            capture_offsets.push(
                captured
                    .checked_duration_since(sample_base)
                    .ok_or_else(|| LiveGateFailure::stage("four_elapsed_regression"))?,
            );
            let (fixture, sessions) = (
                guard.fixture.as_mut().expect("fixture should remain owned"),
                &mut guard.sessions,
            );
            let mut batch = Vec::with_capacity(CONCURRENT_SESSION_COUNT);
            for session in sessions.iter_mut() {
                verify_running_session(&mut fixture.supervisor, session)?;
                let sample = sample_process_resources(
                    session
                        .observation
                        .as_ref()
                        .ok_or_else(|| LiveGateFailure::stage("four_observation_missing"))?,
                )?;
                if sample.working_set_bytes > PER_SESSION_MEMORY_LIMIT
                    || sample.private_bytes > PER_SESSION_MEMORY_LIMIT
                    || sample.handle_count > PER_SESSION_HANDLE_LIMIT
                {
                    return Err(LiveGateFailure::stage("four_session_sample_limit"));
                }
                samples[session.index].push(sample);
                batch.push(sample);
            }
            let aggregate = ProcessResourceSample {
                working_set_bytes: checked_sum_u64(
                    batch.iter().map(|sample| sample.working_set_bytes),
                )?,
                private_bytes: checked_sum_u64(batch.iter().map(|sample| sample.private_bytes))?,
                handle_count: checked_sum_u32(batch.iter().map(|sample| sample.handle_count))?,
                cpu_time_100ns: checked_sum_u64(batch.iter().map(|sample| sample.cpu_time_100ns))?,
            };
            if aggregate.working_set_bytes > AGGREGATE_MEMORY_LIMIT
                || aggregate.private_bytes > AGGREGATE_MEMORY_LIMIT
                || aggregate.handle_count > AGGREGATE_HANDLE_LIMIT
            {
                return Err(LiveGateFailure::stage("four_aggregate_sample_limit"));
            }
            aggregate_samples.push(aggregate);
            if aggregate_samples.len() >= 2 {
                require_aggregate_interval_cpu_within_limit(&aggregate_samples, &capture_offsets)?;
            }
        }

        let elapsed = final_capture
            .and_then(|final_value| {
                first_capture
                    .and_then(|first_value| final_value.checked_duration_since(first_value))
            })
            .ok_or_else(|| LiveGateFailure::stage("four_elapsed_value"))?;
        let mut start_to_window_ms = [0u64; CONCURRENT_SESSION_COUNT];
        for (index, value) in start_to_window_ms.iter_mut().enumerate() {
            *value = guard.sessions[index]
                .start_to_window_ms
                .ok_or_else(|| LiveGateFailure::stage("four_readiness_missing"))?;
        }
        build_performance_evidence(&samples, &aggregate_samples, elapsed, start_to_window_ms)
    })();

    guard.finish(live_result)
}

struct ParentCrashOuterGuard {
    directory: Option<LiveTestDirectory>,
    child: Option<ProbeChild>,
    child_observation: Option<ProcessObservation>,
    java_observation: Option<ProcessObservation>,
    crash_confirmed: bool,
}

impl ParentCrashOuterGuard {
    fn new(directory: LiveTestDirectory) -> Self {
        Self {
            directory: Some(directory),
            child: None,
            child_observation: None,
            java_observation: None,
            crash_confirmed: false,
        }
    }

    fn root(&self) -> &Path {
        self.directory
            .as_ref()
            .expect("parent crash directory should remain owned")
            .path()
    }

    fn confirm_parent_crash(
        &mut self,
        require_parent_termination: bool,
    ) -> Result<(), LiveGateFailure> {
        let child_observation = self
            .child_observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("parent_child_observation_missing"))?;
        let java_observation = self
            .java_observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("parent_java_observation_missing"))?;
        let child_signaled = child_observation
            .is_signaled()
            .map_err(|_| LiveGateFailure::stage("parent_child_wait"))?;
        if child_signaled && require_parent_termination {
            return Err(LiveGateFailure::stage(
                "parent_child_exited_before_terminate",
            ));
        }
        if require_parent_termination
            && java_observation
                .is_signaled()
                .map_err(|_| LiveGateFailure::stage("parent_java_wait"))?
        {
            return Err(LiveGateFailure::stage(
                "parent_java_exited_before_terminate",
            ));
        }
        if !child_signaled {
            terminate_process_observation(child_observation, 0xffff_fffc)
                .map_err(|_| LiveGateFailure::stage("parent_child_terminate"))?;
        }
        let deadline = Instant::now() + STOP_DEADLINE;
        wait_for_process_signal(java_observation, deadline)
            .map_err(|_| LiveGateFailure::stage("parent_java_signal"))?;
        wait_for_process_signal(child_observation, deadline)
            .map_err(|_| LiveGateFailure::stage("parent_child_signal"))?;
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| LiveGateFailure::stage("parent_child_missing"))?;
        child.deadline = deadline;
        child
            .wait_until_exit()
            .map_err(|_| LiveGateFailure::stage("parent_child_reap"))?;
        self.crash_confirmed = true;
        Ok(())
    }

    fn terminate_and_reap_child_without_java(&mut self) -> Result<(), LiveGateFailure> {
        let child_observation = self
            .child_observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("parent_child_observation_missing"))?;
        let deadline = Instant::now() + STOP_DEADLINE;
        if !child_observation
            .is_signaled()
            .map_err(|_| LiveGateFailure::stage("parent_child_wait"))?
        {
            terminate_process_observation(child_observation, 0xffff_fffc)
                .map_err(|_| LiveGateFailure::stage("parent_child_terminate"))?;
        }
        wait_for_process_signal(child_observation, deadline)
            .map_err(|_| LiveGateFailure::stage("parent_child_signal"))?;
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| LiveGateFailure::stage("parent_child_missing"))?;
        child.deadline = deadline;
        child
            .wait_until_exit()
            .map_err(|_| LiveGateFailure::stage("parent_child_reap"))?;
        Ok(())
    }

    fn finish(mut self, original: Result<(), LiveGateFailure>) -> Result<(), LiveGateFailure> {
        let full_java_identity = self.child.is_some()
            && self.child_observation.is_some()
            && self.java_observation.is_some();
        if original.is_err() && !full_java_identity {
            let cleanup = if self.child.is_some() && self.child_observation.is_some() {
                self.terminate_and_reap_child_without_java()
            } else {
                if let Some(child) = self.child.as_mut() {
                    child.deadline = Instant::now() + STOP_DEADLINE;
                }
                Err(LiveGateFailure::stage("parent_cleanup_unconfirmed"))
            };
            return merge_parent_crash_finish_results(original, cleanup, false);
        }
        let cleanup = if self.crash_confirmed {
            Ok(())
        } else if full_java_identity {
            self.confirm_parent_crash(false)
        } else {
            Err(LiveGateFailure::stage("parent_cleanup_unconfirmed"))
        };
        if cleanup.is_ok() && self.crash_confirmed {
            let directory = self
                .directory
                .take()
                .expect("confirmed parent crash should retain guarded root");
            let root = directory.path().to_owned();
            directory.remove_after_cleanup_confirmation()?;
            if root.exists() {
                return Err(LiveGateFailure::stage("parent_cleanup_root_remains"));
            }
        }
        merge_parent_crash_finish_results(original, cleanup, full_java_identity)
    }
}

fn merge_parent_crash_finish_results(
    original: Result<(), LiveGateFailure>,
    cleanup: Result<(), LiveGateFailure>,
    full_java_identity: bool,
) -> Result<(), LiveGateFailure> {
    match (original, cleanup, full_java_identity) {
        (Err(original), _, false) => Err(original),
        (_, Err(cleanup), _) => Err(cleanup),
        (Err(original), Ok(()), _) => Err(original),
        (Ok(()), Ok(()), _) => Ok(()),
    }
}

fn require_parent_crash_child_context() -> Result<(PathBuf, PathBuf), LiveGateFailure> {
    let runtime_root = require_live_runtime_root();
    let child_token = std::env::var(PARENT_CRASH_CHILD_TOKEN_ENV)
        .map_err(|_| LiveGateFailure::stage("parent_child_token"))?;
    let guarded_root = std::env::var_os(PARENT_CRASH_ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| LiveGateFailure::stage("parent_guarded_root"))?;
    let guarded_root = validate_parent_crash_guarded_root(&guarded_root, &child_token)?;
    if live_owner_ready_path(&guarded_root).exists()
        || guarded_root.join("live-owner.partial").exists()
    {
        return Err(LiveGateFailure::stage("parent_owner_record_exists"));
    }
    Ok((runtime_root, guarded_root))
}

fn run_parent_crash_child() -> Result<(), LiveGateFailure> {
    let (runtime_root, guarded_root) = require_parent_crash_child_context()?;
    let mut fixture = prepare_live_child_runtime_fixture(
        &guarded_root,
        &runtime_root,
        "Live Bridge Parent Crash",
    )?;
    let expected_profile_id = Uuid::parse_str(&fixture.profile_id)
        .map_err(|_| LiveGateFailure::stage("parent_profile_identity"))?;
    let profile_id = fixture.profile_id.clone();
    let revision = fixture.profile_revision;
    let started_at = Instant::now();
    let started = fixture
        .supervisor
        .start_session(&profile_id, revision, started_at + START_DEADLINE)
        .map_err(|_| LiveGateFailure::stage("parent_child_start"))?;
    verify_metadata(started.metadata(), expected_profile_id, revision)?;
    let birth_id = started.birth_id();
    let observation =
        open_identity_checked_process_for_metrics(birth_id.pid(), birth_id.creation_time_100ns())
            .map_err(|_| LiveGateFailure::stage("parent_java_observation"))?;
    if observation.birth_id() != birth_id || fixture.supervisor.active_session_count() != 1 {
        return Err(LiveGateFailure::stage("parent_java_identity"));
    }
    let readiness_deadline = started_at + WINDOW_READY_DEADLINE;
    match fixture
        .supervisor
        .observe_session(started.metadata().session_id(), readiness_deadline)
        .map_err(|_| LiveGateFailure::stage("parent_child_observe"))?
    {
        SessionObservation::Running {
            metadata,
            birth_id: observed_birth,
        } if observed_birth == birth_id => {
            verify_metadata(&metadata, expected_profile_id, revision)?;
        }
        _ => return Err(LiveGateFailure::stage("parent_child_observed_state")),
    }
    let window = wait_for_ready_window(&observation, readiness_deadline)?;
    let config = wait_for_profile_config(&fixture.profile_root, readiness_deadline)?;
    if !config.starts_with(&fixture.profile_root)
        || !window_is_responsive(&window, birth_id.pid())?
        || fixture
            .supervisor
            .core()
            .inspect_runtime(LIVE_RUNTIME_ID)
            .map_err(|_| LiveGateFailure::stage("parent_runtime_inspect"))?
            .capability_state
            != CapabilityState::NeedsValidation
    {
        return Err(LiveGateFailure::stage("parent_child_readiness"));
    }
    publish_live_owner_record(
        &guarded_root,
        birth_id.pid(),
        birth_id.creation_time_100ns(),
    )
    .map_err(|_| LiveGateFailure::stage("parent_owner_publish"))?;
    loop {
        thread::park_timeout(Duration::from_secs(1));
    }
}

fn wait_for_live_owner_record(
    guard: &mut ParentCrashOuterGuard,
    deadline: Instant,
) -> Result<ProcessBirthId, LiveGateFailure> {
    loop {
        if live_owner_ready_path(guard.root()).is_file() {
            return read_live_owner_record(guard.root())
                .map_err(|_| LiveGateFailure::stage("parent_owner_decode"));
        }
        if guard
            .child_observation
            .as_ref()
            .ok_or_else(|| LiveGateFailure::stage("parent_child_observation_missing"))?
            .is_signaled()
            .map_err(|_| LiveGateFailure::stage("parent_child_wait"))?
        {
            return Err(LiveGateFailure::stage("parent_child_exited_before_ready"));
        }
        let now = Instant::now();
        let Some(remaining) = deadline.checked_duration_since(now) else {
            return Err(LiveGateFailure::stage("parent_owner_ready_deadline"));
        };
        thread::sleep(remaining.min(WINDOW_POLL_CADENCE));
    }
}

fn run_real_java_parent_crash_gate() -> Result<(), LiveGateFailure> {
    let runtime_root = require_live_runtime_root();
    let directory = LiveTestDirectory::create()?;
    let child_token = directory.leaf().to_owned();
    let guarded_root = directory.path().to_owned();
    validate_parent_crash_guarded_root(&guarded_root, &child_token)?;
    let current_exe = fs::canonicalize(
        std::env::current_exe().map_err(|_| LiveGateFailure::stage("parent_current_exe"))?,
    )
    .map_err(|_| LiveGateFailure::stage("parent_current_exe"))?;
    let mut command =
        parent_crash_child_command(&current_exe, &runtime_root, &guarded_root, &child_token);
    let readiness_deadline = Instant::now() + WINDOW_READY_DEADLINE;
    let mut guard = ParentCrashOuterGuard::new(directory);
    let child = ProbeChild::spawn(&mut command, readiness_deadline)
        .map_err(|_| LiveGateFailure::stage("parent_child_spawn"))?;
    guard.child = Some(child);
    let live_result = (|| {
        let child_birth = child_process_birth_id(
            guard
                .child
                .as_mut()
                .expect("spawned child should remain owned")
                .child_mut(),
        )
        .map_err(|_| LiveGateFailure::stage("parent_child_identity"))?;
        let child_observation = open_identity_checked_process_for_termination_result(
            child_birth.pid(),
            child_birth.creation_time_100ns(),
        )
        .map_err(|_| LiveGateFailure::stage("parent_child_observation"))?;
        if child_observation.birth_id() != child_birth {
            return Err(LiveGateFailure::stage("parent_child_identity"));
        }
        guard.child_observation = Some(child_observation);

        let java_birth = wait_for_live_owner_record(&mut guard, readiness_deadline)?;
        let java_observation = open_identity_checked_process_for_metrics(
            java_birth.pid(),
            java_birth.creation_time_100ns(),
        )
        .map_err(|_| LiveGateFailure::stage("parent_java_observation"))?;
        if java_observation.birth_id() != java_birth {
            return Err(LiveGateFailure::stage("parent_java_identity"));
        }
        guard.java_observation = Some(java_observation);
        guard.confirm_parent_crash(true)
    })();
    guard.finish(live_result)
}

#[test]
fn four_session_performance_math_is_checked_rounded_and_signed() {
    assert_eq!(cpu_percent_one_core_x100(100, 600, 1_000).unwrap(), 5_000);
    assert_eq!(cpu_percent_one_core_x100(0, 1, 3).unwrap(), 3_333);
    assert_eq!(signed_growth(20, 10).unwrap(), -10);
    assert_eq!(signed_growth(10, 20).unwrap(), 10);

    let samples: [Vec<ProcessResourceSample>; CONCURRENT_SESSION_COUNT] =
        std::array::from_fn(|index| {
            (0..SAMPLE_COUNT)
                .map(|sample_index| ProcessResourceSample {
                    working_set_bytes: 1_000 + index as u64 + sample_index as u64,
                    private_bytes: 2_000 + index as u64 + sample_index as u64,
                    handle_count: 10 + u32::try_from(index).unwrap(),
                    cpu_time_100ns: 1_000 + u64::try_from(sample_index).unwrap() * 100,
                })
                .collect()
        });
    let aggregate_samples = (0..SAMPLE_COUNT)
        .map(|sample_index| ProcessResourceSample {
            working_set_bytes: checked_sum_u64(
                samples
                    .iter()
                    .map(|values| values[sample_index].working_set_bytes),
            )
            .unwrap(),
            private_bytes: checked_sum_u64(
                samples
                    .iter()
                    .map(|values| values[sample_index].private_bytes),
            )
            .unwrap(),
            handle_count: checked_sum_u32(
                samples
                    .iter()
                    .map(|values| values[sample_index].handle_count),
            )
            .unwrap(),
            cpu_time_100ns: checked_sum_u64(
                samples
                    .iter()
                    .map(|values| values[sample_index].cpu_time_100ns),
            )
            .unwrap(),
        })
        .collect::<Vec<_>>();
    let evidence = build_performance_evidence(
        &samples,
        &aggregate_samples,
        Duration::from_secs(60),
        [1, 2, 3, 4],
    )
    .unwrap();
    assert_eq!(
        evidence.per_session.map(|session| session.index),
        [1, 2, 3, 4]
    );
    assert_eq!(evidence.aggregate.working_set_growth_bytes, 48);
    assert_eq!(evidence.aggregate.private_growth_bytes, 48);
    assert!(evidence.aggregate.cpu_percent_one_core_x100 <= AGGREGATE_CPU_X100_LIMIT);
    let line = live_performance_record_line(&evidence).unwrap();
    assert!(line.starts_with(LIVE_PERFORMANCE_PREFIX));
    assert_eq!(line.matches(LIVE_PERFORMANCE_PREFIX).count(), 1);
    let json = &line[LIVE_PERFORMANCE_PREFIX.len()..];
    assert!(!json.is_empty());
    assert!(json.len() <= 64 * 1024);
    for forbidden in [
        "pid",
        "creation_time",
        "path",
        "title",
        "argv",
        "environment",
        "username",
        "hostname",
        "machine_id",
    ] {
        assert!(!json.contains(forbidden));
    }
}

#[test]
fn aggregate_interval_cpu_rejects_a_spike_hidden_by_the_sixty_second_average() {
    let mut samples = (0..SAMPLE_COUNT)
        .map(|index| ProcessResourceSample {
            working_set_bytes: 1,
            private_bytes: 1,
            handle_count: 1,
            cpu_time_100ns: if index == 0 { 1 } else { 60_000_002 },
        })
        .collect::<Vec<_>>();
    let capture_offsets = (0..SAMPLE_COUNT)
        .map(|index| Duration::from_secs(u64::try_from(index).unwrap() * 5))
        .collect::<Vec<_>>();

    assert!(
        cpu_percent_one_core_x100(
            samples[0].cpu_time_100ns,
            samples[SAMPLE_COUNT - 1].cpu_time_100ns,
            duration_100ns(Duration::from_secs(60)).unwrap(),
        )
        .unwrap()
            < AGGREGATE_CPU_X100_LIMIT
    );
    let spike = require_aggregate_interval_cpu_within_limit(&samples, &capture_offsets)
        .expect_err("one over-limit interval must reject an otherwise low final average");
    assert_eq!(spike.stage_name(), "four_aggregate_interval_cpu_limit");

    for sample in &mut samples {
        sample.cpu_time_100ns = 10;
    }
    samples[2].cpu_time_100ns = 9;
    let regression = require_aggregate_interval_cpu_within_limit(&samples, &capture_offsets)
        .expect_err("cumulative CPU regression must fail closed");
    assert_eq!(regression.stage_name(), "four_cpu_regression");

    let mut duplicate_offsets = capture_offsets.clone();
    duplicate_offsets[1] = duplicate_offsets[0];
    let zero_elapsed = require_aggregate_interval_cpu_within_limit(&samples, &duplicate_offsets)
        .expect_err("zero elapsed interval must fail closed");
    assert_eq!(zero_elapsed.stage_name(), "four_elapsed_value");

    let overflow = cpu_percent_one_core_x100(0, u64::MAX, 1)
        .expect_err("fixed-point result outside u64 must fail closed");
    assert_eq!(overflow.stage_name(), "four_cpu_overflow");
}

#[test]
fn cleanup_policy_requires_stops_for_success_and_narrows_removed_failure_owners() {
    let running_success = cleanup_plan(CleanupIntent::Success, false, false);
    assert_eq!(running_success.action, CleanupAction::Stop);
    assert!(!running_success.success_violation);

    let signaled_success = cleanup_plan(CleanupIntent::Success, true, false);
    assert_eq!(signaled_success.action, CleanupAction::ObserveSignaled);
    assert!(signaled_success.success_violation);

    let removed_failure = cleanup_plan(CleanupIntent::Failure, true, true);
    assert_eq!(
        removed_failure.action,
        CleanupAction::AcceptPreviouslyRemoved
    );
    assert!(!removed_failure.success_violation);

    let unconfirmed_failure = cleanup_plan(CleanupIntent::Failure, true, false);
    assert_eq!(unconfirmed_failure.action, CleanupAction::ObserveSignaled);
    assert!(!unconfirmed_failure.success_violation);
}

#[test]
fn live_owner_record_is_exact_fixed_width_and_rejects_invalid_identity() {
    let encoded = crate::process_adapter::test_support::encode_live_owner_record(7, 11)
        .expect("nonzero owner identity should encode");
    assert_eq!(encoded.len(), 24);
    assert_eq!(&encoded[..8], b"ZHSLIVE1");
    assert_eq!(&encoded[8..12], &1u32.to_le_bytes());
    let decoded = decode_live_owner_record_bytes(&encoded).expect("exact record should decode");
    assert_eq!(decoded.pid(), 7);
    assert_eq!(decoded.creation_time_100ns(), 11);

    for malformed in [&encoded[..23], &[encoded.as_slice(), &[0]].concat()[..]] {
        assert!(decode_live_owner_record_bytes(malformed).is_err());
    }
    let mut wrong_magic = encoded;
    wrong_magic[0] ^= 1;
    assert!(decode_live_owner_record_bytes(&wrong_magic).is_err());
    let mut wrong_schema = encoded;
    wrong_schema[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_live_owner_record_bytes(&wrong_schema).is_err());
    let mut zero_pid = encoded;
    zero_pid[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode_live_owner_record_bytes(&zero_pid).is_err());
    let mut zero_creation = encoded;
    zero_creation[16..24].copy_from_slice(&0u64.to_le_bytes());
    assert!(decode_live_owner_record_bytes(&zero_creation).is_err());
}

#[test]
fn live_owner_record_publication_is_fixed_atomic_and_cleanup_guarded() {
    let directory = super::windows_live_test_support::LiveTestDirectory::create()
        .expect("live owner record fixture should be created");
    let root = directory.path().to_owned();
    publish_live_owner_record(&root, 7, 11).expect("owner record should publish atomically");
    let ready = live_owner_ready_path(&root);
    assert_eq!(ready.file_name().unwrap(), "live-owner.ready");
    assert_eq!(fs::metadata(&ready).unwrap().len(), 24);
    assert!(!root.join("live-owner.partial").exists());
    let decoded = crate::process_adapter::test_support::read_live_owner_record(&root)
        .expect("fixed ready record should decode");
    assert_eq!(decoded.pid(), 7);
    assert_eq!(decoded.creation_time_100ns(), 11);
    directory
        .remove_after_cleanup_confirmation()
        .expect("record fixture should be removed through guarded cleanup");
    assert!(!root.exists());
}

#[test]
fn parent_crash_child_command_has_one_exact_filter_and_four_scoped_values() {
    let command = parent_crash_child_command(
        Path::new(r"C:\bounded\tests.exe"),
        Path::new(r"C:\bounded\runtime"),
        Path::new(r"C:\bounded\root"),
        "01234567-89ab-4def-8123-456789abcdef",
    );
    assert_eq!(command.get_program(), r"C:\bounded\tests.exe");
    assert_eq!(
        command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        [
            "--exact",
            "session_supervisor::windows_live_runtime_tests::live_runtime_parent_crash_child",
            "--ignored",
            "--nocapture",
        ]
    );
    let environments = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(environments.len(), 4);
    assert_eq!(
        environments["ZEUS_EXACT_RUNTIME_ROOT"],
        r"C:\bounded\runtime"
    );
    assert_eq!(
        environments["ZEUS_LIVE_PARENT_CRASH_CHILD_TOKEN"],
        "01234567-89ab-4def-8123-456789abcdef"
    );
    assert_eq!(
        environments["ZEUS_LIVE_PARENT_CRASH_ROOT"],
        r"C:\bounded\root"
    );
    assert_eq!(
        environments["ZEUS_LIVE_RUNTIME_BRIDGE"],
        super::windows_live_test_support::LIVE_APPROVAL_TOKEN
    );
}

#[test]
fn parent_crash_child_requires_the_exact_canonical_guarded_root() {
    let directory = LiveTestDirectory::create().expect("guarded root fixture should be created");
    let canonical = validate_parent_crash_guarded_root(directory.path(), directory.leaf())
        .expect("canonical guarded root should pass");
    assert_eq!(canonical, directory.path());

    let aliased = PathBuf::from(format!(
        r"{}\..\{}",
        directory.path().display(),
        directory.leaf()
    ));
    assert!(validate_parent_crash_guarded_root(&aliased, directory.leaf()).is_err());

    let root = directory.path().to_owned();
    directory
        .remove_after_cleanup_confirmation()
        .expect("guarded root fixture should be removed through guarded cleanup");
    assert!(!root.exists());
}

#[test]
fn parent_crash_finish_preserves_original_without_full_java_identity() {
    let original = LiveGateFailure::stage("parent_original_stage");
    let cleanup = LiveGateFailure::stage("parent_cleanup_stage");
    assert_eq!(
        merge_parent_crash_finish_results(Err(original), Err(cleanup), false),
        Err(original)
    );
    assert_eq!(
        merge_parent_crash_finish_results(Ok(()), Err(cleanup), false),
        Err(cleanup)
    );
}

#[test]
#[ignore = "launches the provisioned exact Windows game; run scripts/Invoke-LiveRuntimeBridgeTests.ps1 -AllowGameLaunch"]
fn exact_runtime_launches_through_production_supervisor_and_hard_stops() {
    if let Err(error) = run_single_profile_live_gate() {
        panic!("{error}");
    }
}

#[test]
#[ignore = "launches four provisioned exact Windows game sessions and samples 60 seconds; run the dedicated live bridge runner"]
fn four_exact_profiles_stay_isolated_within_performance_bounds() {
    let evidence = run_four_profile_live_gate().unwrap_or_else(|error| panic!("{error}"));
    let record = live_performance_record_line(&evidence).unwrap_or_else(|error| panic!("{error}"));
    println!();
    println!("{record}");
}

#[test]
#[ignore = "launches exact Java in a child Supervisor owner and terminates that owner; run the dedicated live bridge runner"]
fn real_java_exits_when_supervisor_owner_is_terminated() {
    if let Err(error) = run_real_java_parent_crash_gate() {
        panic!("{error}");
    }
}

#[test]
#[ignore = "child-only harness for the exact-runtime Supervisor parent-crash gate"]
fn live_runtime_parent_crash_child() {
    if let Err(error) = run_parent_crash_child() {
        panic!("{error}");
    }
}

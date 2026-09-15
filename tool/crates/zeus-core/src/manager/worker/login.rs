//! Bounded readiness coordinator and input gate.
//!
//! This module is deliberately free of Windows APIs: the port trait is injected, so every concurrency
//! rule in spec sections 13, 14, 16, and 17 is testable with fakes. Task 10 supplies the Windows port.
//!
//! Serialization boundary: profile starts, stops, and cleanup stay on the worker thread. Independence
//! means no account's readiness or input wait blocks another account's start or stop.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[cfg(windows)]
mod windows;

use crate::account::AccountConfigV1;
use crate::credential_vault::SecretBytes;

use super::super::ManagerAccountId;
use super::super::account::LoginTarget;

/// Maximum readiness tasks, retained sessions, and batch members (spec section 14).
pub(super) const MAX_LOGIN_TASKS: usize = 4;

/// One readiness task's private, exclusively owned work.
///
/// It carries exactly one account's identity, secret, config snapshot, and target. It holds no Core
/// handle and no controller, so a task cannot mutate lifecycle state or persist anything.
pub(super) struct LoginJob {
    pub(super) account_id: ManagerAccountId,
    pub(super) session_key: String,
    pub(super) epoch: u64,
    pub(super) username: String,
    pub(super) password: SecretBytes,
    pub(super) config: AccountConfigV1,
    pub(super) target: LoginTarget,
}

impl LoginJob {
    /// Clears the secret at the first failure, before any completion is enqueued.
    pub(super) fn clear_secret(&mut self) {
        self.password = SecretBytes::new(Vec::new());
    }
}

/// Why a login failed. Deliberately coarse: Windows never reveals whether UIPI ate an input, so a
/// zero or short send is reported only as a generic rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginFailureCode {
    ReadyTimeout,
    TargetMismatch,
    ForegroundDenied,
    IntegrityMismatch,
    InputRejected,
}

impl LoginFailureCode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ReadyTimeout => "ready_timeout",
            Self::TargetMismatch => "target_mismatch",
            Self::ForegroundDenied => "foreground_denied",
            Self::IntegrityMismatch => "integrity_mismatch",
            Self::InputRejected => "input_rejected",
        }
    }
}

/// Terminal outcome of one readiness task.
///
/// `InputSent` states only that the complete fixed script was injected. It is never an authentication
/// claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginOutcome {
    InputSent,
    Failed(LoginFailureCode),
    Cancelled,
}

/// Redacted record handed back to the worker. It carries no secret, HWND, or Core handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoginCompletion {
    pub(super) account_id: ManagerAccountId,
    pub(super) session_key: String,
    pub(super) epoch: u64,
    pub(super) outcome: LoginOutcome,
}

/// A job that reached stable-window readiness. Only the arbiter may consume it.
pub(super) struct ReadyJob(pub(super) LoginJob);

/// A job that failed readiness, handed back so its owner can clear the secret and report.
///
/// Boxed because it carries the whole job: an unboxed large `Err` would bloat every caller's stack.
pub(super) struct FailedJob {
    pub(super) job: LoginJob,
    pub(super) code: LoginFailureCode,
}

/// Injected readiness and input behavior, split so readiness may overlap while input is serialized.
pub(super) trait LoginPort: Send + Sync {
    fn wait_until_ready(
        &self,
        job: LoginJob,
        cancel: &AtomicBool,
    ) -> Result<ReadyJob, Box<FailedJob>>;

    fn run_input(&self, ready: ReadyJob, cancel: &AtomicBool) -> LoginOutcome;
}

/// Per-account result of admitting a Run command. It is a scheduling outcome, never a login claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunScheduleOutcome {
    Scheduled,
    Rejected(RunRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunRejection {
    /// The batch would exceed the four-task ceiling. Rejected before start, decrypt, or task creation.
    TaskLimitReached,
    AlreadyRunning,
    StartFailed,
}

impl RunRejection {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::TaskLimitReached => "task_limit_reached",
            Self::AlreadyRunning => "already_running",
            Self::StartFailed => "start_failed",
        }
    }
}

/// One admission generation over a bounded Run batch.
///
/// The generation is reserved atomically before any member starts, and closes only after every
/// reserved member reaches start success or failure. Global input opens only after every started
/// member is Ready or terminally failed, so a newly activating window cannot steal foreground from an
/// in-flight script.
#[derive(Debug, Default)]
pub(super) struct BatchGeneration {
    reserved: usize,
    started: usize,
    settled: usize,
    ready_or_failed: usize,
}

impl BatchGeneration {
    pub(super) fn reserve(count: usize) -> Self {
        Self {
            reserved: count,
            ..Self::default()
        }
    }

    pub(super) fn record_start_success(&mut self) {
        self.started += 1;
        self.settled += 1;
    }

    pub(super) fn record_start_failure(&mut self) {
        self.settled += 1;
    }

    /// Reports whether every reserved member has settled, closing the generation.
    pub(super) fn is_closed(&self) -> bool {
        self.settled >= self.reserved
    }

    pub(super) fn record_ready_or_failed(&mut self) {
        self.ready_or_failed += 1;
    }

    /// Input opens only once the generation is closed and every started member settled its readiness.
    pub(super) fn input_may_open(&self) -> bool {
        self.is_closed() && self.ready_or_failed >= self.started
    }
}

/// Serializes `run_input` so exactly one ready job holds the foreground at a time.
pub(super) struct InputArbiter {
    queue: Mutex<VecDeque<ReadyJob>>,
    injection_in_progress: Arc<AtomicBool>,
}

impl InputArbiter {
    pub(super) fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(MAX_LOGIN_TASKS)),
            injection_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Shares the injection flag with the worker, which defers new starts while input is active.
    pub(super) fn injection_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.injection_in_progress)
    }

    pub(super) fn injection_in_progress(&self) -> bool {
        self.injection_in_progress.load(Ordering::Acquire)
    }

    /// Enqueues one ready job, or hands it back when the four-slot queue is full.
    ///
    /// The rejected job is boxed so the overflow path does not widen every caller's stack frame.
    pub(super) fn enqueue(&self, ready: ReadyJob) -> Result<(), Box<ReadyJob>> {
        let mut queue = self.lock();
        if queue.len() >= MAX_LOGIN_TASKS {
            return Err(Box::new(ready));
        }
        queue.push_back(ready);
        Ok(())
    }

    pub(super) fn queued(&self) -> usize {
        self.lock().len()
    }

    /// Runs queued jobs one at a time, returning each outcome in queue order.
    ///
    /// The flag is cleared on every path, including a cancellation, so the worker never defers starts
    /// forever.
    pub(super) fn drain_serialized(
        &self,
        port: &dyn LoginPort,
        cancel: &AtomicBool,
    ) -> Vec<(ManagerAccountId, String, u64, LoginOutcome)> {
        let mut outcomes = Vec::new();
        loop {
            let Some(ready) = self.lock().pop_front() else {
                break;
            };
            let account_id = ready.0.account_id;
            let session_key = ready.0.session_key.clone();
            let epoch = ready.0.epoch;
            self.injection_in_progress.store(true, Ordering::Release);
            let outcome = port.run_input(ready, cancel);
            self.injection_in_progress.store(false, Ordering::Release);
            outcomes.push((account_id, session_key, epoch, outcome));
        }
        outcomes
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<ReadyJob>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for InputArbiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure bookkeeping for the readiness fleet: admission ceiling, generation fence, and cancellation.
///
/// It owns no Core handle, no controller, and no secret, so every rule here is testable without
/// Windows.
pub(super) struct LoginCoordinator {
    active: usize,
    generation: Option<BatchGeneration>,
    cancel: Arc<AtomicBool>,
    epochs: Vec<(ManagerAccountId, String, u64)>,
}

impl LoginCoordinator {
    pub(super) fn new() -> Self {
        Self {
            active: 0,
            generation: None,
            cancel: Arc::new(AtomicBool::new(false)),
            epochs: Vec::with_capacity(MAX_LOGIN_TASKS),
        }
    }

    /// Shares the cancellation flag with every readiness task, so shutdown cancels all of them.
    pub(super) fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    pub(super) fn cancel_all(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub(super) fn active_tasks(&self) -> usize {
        self.active
    }

    /// Atomically reserves one generation for a bounded batch, before any member starts.
    ///
    /// A batch that would exceed the four-task ceiling is rejected entirely and reserves nothing, so
    /// a fifth Run never reaches profile start, secret decrypt, or task creation.
    pub(super) fn reserve_batch(
        &mut self,
        requested: &[ManagerAccountId],
    ) -> Result<Vec<RunScheduleOutcome>, RunRejection> {
        if requested.is_empty() || requested.len() > MAX_LOGIN_TASKS {
            return Err(RunRejection::TaskLimitReached);
        }
        if self.generation.is_some() {
            return Err(RunRejection::TaskLimitReached);
        }
        let mut outcomes = Vec::with_capacity(requested.len());
        let mut reserved = 0;
        for account_id in requested {
            if self.epochs.iter().any(|(known, _, _)| known == account_id) {
                outcomes.push(RunScheduleOutcome::Rejected(RunRejection::AlreadyRunning));
                continue;
            }
            if self.active + reserved >= MAX_LOGIN_TASKS {
                outcomes.push(RunScheduleOutcome::Rejected(RunRejection::TaskLimitReached));
                continue;
            }
            reserved += 1;
            outcomes.push(RunScheduleOutcome::Scheduled);
        }
        if reserved == 0 {
            return Ok(outcomes);
        }
        self.generation = Some(BatchGeneration::reserve(reserved));
        Ok(outcomes)
    }

    /// Records one member's successful start and the readiness task it now owns.
    pub(super) fn record_start_success(
        &mut self,
        account_id: ManagerAccountId,
        session_key: String,
        epoch: u64,
    ) {
        self.active += 1;
        self.epochs.push((account_id, session_key, epoch));
        if let Some(generation) = self.generation.as_mut() {
            generation.record_start_success();
        }
    }

    pub(super) fn record_start_failure(&mut self) {
        if let Some(generation) = self.generation.as_mut() {
            generation.record_start_failure();
        }
    }

    pub(super) fn record_ready_or_failed(&mut self) {
        if let Some(generation) = self.generation.as_mut() {
            generation.record_ready_or_failed();
        }
    }

    /// Reports whether the global input gate may open.
    pub(super) fn input_may_open(&self) -> bool {
        self.generation
            .as_ref()
            .is_some_and(BatchGeneration::input_may_open)
    }

    /// Closes the generation once input has drained, freeing the next batch.
    pub(super) fn close_generation(&mut self) {
        self.generation = None;
    }

    /// Accepts a completion only when its account, session, and epoch still match.
    ///
    /// A stale record is dropped without any state, persistence, or event change: the session it
    /// referred to is already gone.
    pub(super) fn accept_completion(&mut self, completion: &LoginCompletion) -> bool {
        let position = self
            .epochs
            .iter()
            .position(|(account_id, session_key, epoch)| {
                *account_id == completion.account_id
                    && *session_key == completion.session_key
                    && *epoch == completion.epoch
            });
        match position {
            Some(index) => {
                self.epochs.swap_remove(index);
                self.active = self.active.saturating_sub(1);
                true
            }
            None => false,
        }
    }
}

impl Default for LoginCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Captures a session's live epoch for completion matching.
pub(super) fn current_epoch(epoch: &Arc<AtomicU64>) -> u64 {
    epoch.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::mpsc::sync_channel;
    use std::thread;
    use std::time::Duration;

    use uuid::Uuid;

    use super::*;
    use crate::process_adapter::ProcessBirthId;

    const DEADLINE: Duration = Duration::from_secs(10);

    fn account() -> ManagerAccountId {
        ManagerAccountId::new(Uuid::new_v4())
    }

    fn job(account_id: ManagerAccountId, session_key: &str, epoch: u64) -> LoginJob {
        let session_id = Uuid::new_v4();
        let epoch_handle = Arc::new(AtomicU64::new(epoch));
        LoginJob {
            account_id,
            session_key: session_key.to_owned(),
            epoch,
            username: "Operator".to_owned(),
            password: SecretBytes::new(b"Secret-1".to_vec()),
            config: AccountConfigV1::defaults(),
            target: LoginTarget::new(
                session_id,
                ProcessBirthId::for_test(4242, 777),
                "javaw.exe".to_owned(),
                "SunAwtFrame".to_owned(),
                epoch_handle,
            ),
        }
    }

    /// Port that parks every readiness call on a shared barrier, proving real overlap.
    struct BarrierPort {
        barrier: Arc<Barrier>,
        input_order: Mutex<Vec<ManagerAccountId>>,
        concurrent_inputs: Arc<AtomicU64>,
        max_concurrent_inputs: Arc<AtomicU64>,
    }

    impl BarrierPort {
        fn new(parties: usize) -> Self {
            Self {
                barrier: Arc::new(Barrier::new(parties)),
                input_order: Mutex::new(Vec::new()),
                concurrent_inputs: Arc::new(AtomicU64::new(0)),
                max_concurrent_inputs: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    impl LoginPort for BarrierPort {
        fn wait_until_ready(
            &self,
            job: LoginJob,
            cancel: &AtomicBool,
        ) -> Result<ReadyJob, Box<FailedJob>> {
            // Every task must arrive before any is released: readiness truly overlaps.
            self.barrier.wait();
            if cancel.load(Ordering::Acquire) {
                return Err(Box::new(FailedJob {
                    job,
                    code: LoginFailureCode::ReadyTimeout,
                }));
            }
            Ok(ReadyJob(job))
        }

        fn run_input(&self, ready: ReadyJob, _cancel: &AtomicBool) -> LoginOutcome {
            let live = self.concurrent_inputs.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_concurrent_inputs.fetch_max(live, Ordering::AcqRel);
            self.input_order
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ready.0.account_id);
            self.concurrent_inputs.fetch_sub(1, Ordering::AcqRel);
            LoginOutcome::InputSent
        }
    }

    #[test]
    fn login_coordinator_reserves_one_generation_before_any_start() {
        let mut coordinator = LoginCoordinator::new();
        let batch = [account(), account(), account(), account()];

        let outcomes = coordinator
            .reserve_batch(&batch)
            .expect("a four-item batch is admitted");

        assert_eq!(outcomes, vec![RunScheduleOutcome::Scheduled; 4]);
        // The generation exists before a single member has started.
        assert!(!coordinator.input_may_open());
        assert_eq!(coordinator.active_tasks(), 0);
    }

    #[test]
    fn login_coordinator_rejects_a_fifth_run_before_start_or_decrypt() {
        let mut coordinator = LoginCoordinator::new();
        let batch = [account(), account(), account(), account()];
        coordinator
            .reserve_batch(&batch)
            .expect("first batch is admitted");
        for (index, account_id) in batch.iter().enumerate() {
            coordinator.record_start_success(*account_id, format!("session-{index}"), 1);
        }

        // A fifth Run is refused while four tasks are live, reserving nothing.
        let fifth = coordinator.reserve_batch(&[account()]);
        assert_eq!(fifth, Err(RunRejection::TaskLimitReached));
        assert_eq!(coordinator.active_tasks(), MAX_LOGIN_TASKS);

        // An over-sized batch is refused whole.
        let mut fresh = LoginCoordinator::new();
        assert_eq!(
            fresh.reserve_batch(&[account(), account(), account(), account(), account()]),
            Err(RunRejection::TaskLimitReached)
        );
        assert_eq!(fresh.active_tasks(), 0);
    }

    #[test]
    fn login_coordinator_opens_input_only_after_every_started_member_settles() {
        let mut coordinator = LoginCoordinator::new();
        let batch = [account(), account(), account()];
        coordinator.reserve_batch(&batch).expect("batch admitted");

        coordinator.record_start_success(batch[0], "session-a".to_owned(), 1);
        // The generation is still open: two members have not started.
        assert!(!coordinator.input_may_open());
        coordinator.record_start_success(batch[1], "session-b".to_owned(), 1);
        // One member fails to start; the generation still closes.
        coordinator.record_start_failure();
        assert!(
            !coordinator.input_may_open(),
            "input must wait for readiness, not merely for starts"
        );

        coordinator.record_ready_or_failed();
        assert!(!coordinator.input_may_open());
        coordinator.record_ready_or_failed();
        // Every started member is Ready or terminally failed, so the gate opens.
        assert!(coordinator.input_may_open());
    }

    #[test]
    fn login_coordinator_releases_the_gate_when_one_task_times_out() {
        let mut coordinator = LoginCoordinator::new();
        let batch = [account(), account(), account(), account()];
        coordinator.reserve_batch(&batch).expect("batch admitted");
        for (index, account_id) in batch.iter().enumerate() {
            coordinator.record_start_success(*account_id, format!("session-{index}"), 1);
        }

        // A hangs and reports a terminal ready timeout; B, C, and D reach readiness.
        for _ in 0..MAX_LOGIN_TASKS {
            coordinator.record_ready_or_failed();
        }

        // The gate opens in finite time even though one member never became ready.
        assert!(coordinator.input_may_open());
        // Only A's completion removes A; the other three stay live.
        let stopped = LoginCompletion {
            account_id: batch[0],
            session_key: "session-0".to_owned(),
            epoch: 1,
            outcome: LoginOutcome::Failed(LoginFailureCode::ReadyTimeout),
        };
        assert!(coordinator.accept_completion(&stopped));
        assert_eq!(coordinator.active_tasks(), 3);
    }

    #[test]
    fn login_coordinator_drops_a_stale_or_mismatched_completion() {
        let mut coordinator = LoginCoordinator::new();
        let live = account();
        coordinator.reserve_batch(&[live]).expect("batch admitted");
        coordinator.record_start_success(live, "session-live".to_owned(), 7);

        // Wrong epoch: the session was torn down and restarted.
        assert!(!coordinator.accept_completion(&LoginCompletion {
            account_id: live,
            session_key: "session-live".to_owned(),
            epoch: 6,
            outcome: LoginOutcome::InputSent,
        }));
        // Wrong session key for the right account.
        assert!(!coordinator.accept_completion(&LoginCompletion {
            account_id: live,
            session_key: "session-other".to_owned(),
            epoch: 7,
            outcome: LoginOutcome::InputSent,
        }));
        // Unknown account.
        assert!(!coordinator.accept_completion(&LoginCompletion {
            account_id: account(),
            session_key: "session-live".to_owned(),
            epoch: 7,
            outcome: LoginOutcome::InputSent,
        }));
        // A dropped completion changes nothing.
        assert_eq!(coordinator.active_tasks(), 1);

        // The exact match is accepted once and only once.
        let exact = LoginCompletion {
            account_id: live,
            session_key: "session-live".to_owned(),
            epoch: 7,
            outcome: LoginOutcome::InputSent,
        };
        assert!(coordinator.accept_completion(&exact));
        assert!(!coordinator.accept_completion(&exact));
        assert_eq!(coordinator.active_tasks(), 0);
    }

    #[test]
    fn login_readiness_tasks_overlap_while_input_stays_serialized() {
        let port = Arc::new(BarrierPort::new(MAX_LOGIN_TASKS));
        let arbiter = Arc::new(InputArbiter::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = sync_channel(MAX_LOGIN_TASKS);

        let mut handles = Vec::new();
        for index in 0..MAX_LOGIN_TASKS {
            let port = Arc::clone(&port);
            let arbiter = Arc::clone(&arbiter);
            let cancel = Arc::clone(&cancel);
            let ready_tx = ready_tx.clone();
            let job = job(account(), &format!("session-{index}"), 1);
            handles.push(
                thread::Builder::new()
                    .name(format!("zeus-login-{index}"))
                    .spawn(move || {
                        // All four block on the barrier inside wait_until_ready: if readiness were
                        // serialized this would deadlock instead of completing.
                        match port.wait_until_ready(job, &cancel) {
                            Ok(ready) => {
                                assert!(arbiter.enqueue(ready).is_ok(), "queue holds four");
                                ready_tx.send(Ok(())).expect("receiver stays connected");
                            }
                            Err(failed) => {
                                ready_tx
                                    .send(Err(failed.code))
                                    .expect("receiver stays connected");
                            }
                        }
                    })
                    .expect("spawn readiness task"),
            );
        }
        drop(ready_tx);

        for _ in 0..MAX_LOGIN_TASKS {
            ready_rx
                .recv_timeout(DEADLINE)
                .expect("every readiness task finished")
                .expect("readiness succeeded");
        }
        for handle in handles {
            handle.join().expect("readiness task completed");
        }
        assert_eq!(arbiter.queued(), MAX_LOGIN_TASKS);

        // Input drains one job at a time and clears the flag on every path.
        let outcomes = arbiter.drain_serialized(port.as_ref(), &cancel);
        assert_eq!(outcomes.len(), MAX_LOGIN_TASKS);
        assert!(
            outcomes
                .iter()
                .all(|(_, _, _, outcome)| *outcome == LoginOutcome::InputSent)
        );
        assert_eq!(
            port.max_concurrent_inputs.load(Ordering::Acquire),
            1,
            "input must never overlap"
        );
        assert!(!arbiter.injection_in_progress());
        assert_eq!(arbiter.queued(), 0);
    }

    #[test]
    fn login_input_arbiter_bounds_its_queue_and_signals_injection() {
        let arbiter = InputArbiter::new();
        let flag = arbiter.injection_flag();
        assert!(!flag.load(Ordering::Acquire));

        for index in 0..MAX_LOGIN_TASKS {
            assert!(
                arbiter
                    .enqueue(ReadyJob(job(account(), &format!("session-{index}"), 1)))
                    .is_ok(),
                "queue accepts four"
            );
        }
        // A fifth ready job is handed back rather than growing the queue.
        let overflow = ReadyJob(job(account(), "session-overflow", 1));
        assert!(arbiter.enqueue(overflow).is_err());
        assert_eq!(arbiter.queued(), MAX_LOGIN_TASKS);
    }

    #[test]
    fn login_cancellation_stops_every_readiness_task() {
        let port = BarrierPort::new(1);
        let coordinator = LoginCoordinator::new();
        let cancel = coordinator.cancel_flag();
        coordinator.cancel_all();
        assert!(coordinator.is_cancelled());

        // A cancelled task reports Cancelled-equivalent failure and never reaches input.
        let Err(failed) = port.wait_until_ready(job(account(), "session-cancel", 1), &cancel)
        else {
            panic!("a cancelled task cannot become ready");
        };
        assert_eq!(failed.code, LoginFailureCode::ReadyTimeout);
        assert_eq!(failed.job.session_key, "session-cancel");
    }

    #[test]
    fn login_job_clears_its_secret_before_completion() {
        let mut failing = job(account(), "session-clear", 1);
        failing.clear_secret();
        // The secret is gone before any completion record is built.
        assert!(failing.password.is_empty_for_test());
    }

    #[test]
    fn login_failure_and_rejection_codes_are_stable_and_redacted() {
        for (code, expected) in [
            (LoginFailureCode::ReadyTimeout, "ready_timeout"),
            (LoginFailureCode::TargetMismatch, "target_mismatch"),
            (LoginFailureCode::ForegroundDenied, "foreground_denied"),
            (LoginFailureCode::IntegrityMismatch, "integrity_mismatch"),
            (LoginFailureCode::InputRejected, "input_rejected"),
        ] {
            assert_eq!(code.as_str(), expected);
        }
        for (rejection, expected) in [
            (RunRejection::TaskLimitReached, "task_limit_reached"),
            (RunRejection::AlreadyRunning, "already_running"),
            (RunRejection::StartFailed, "start_failed"),
        ] {
            assert_eq!(rejection.as_str(), expected);
        }
    }
}

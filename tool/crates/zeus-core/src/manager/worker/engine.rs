//! Private controller ownership and worker-loop boundary.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, Thread};

#[cfg(all(test, windows))]
use crate::process_adapter::ProcessBirthId;

use crate::control::AttackSpot;
use crate::spots::SpotBook;

use super::super::{
    ControlSettings, ManagerAccountId, ManagerAccountView, ManagerController, ManagerError,
    ManagerErrorCode, ManagerObservation, ManagerOperation, ManagerProfilePage, ManagerResult,
    ManagerRunSchedule, ManagerRuntimePage, ManagerSessionExit, ManagerSessionView, PlayerSnapshot,
};
use super::login::{
    LoginCompletion as ReadinessCompletion, LoginOutcome as ReadinessOutcome,
    MAX_LOGIN_TASKS as MAX_READINESS_TASKS,
};
use super::{ManagerAccountPassword, ManagerRequestId, ManagerWorkerEvent};

#[allow(
    clippy::enum_variant_names,
    reason = "Task 3's exact query commands share a prefix until lifecycle commands are added"
)]
pub(super) enum WorkerCommand {
    ListProfiles {
        request_id: ManagerRequestId,
        after: Option<String>,
        limit: u32,
        include_archived: bool,
    },
    ListRuntimes {
        request_id: ManagerRequestId,
        after: Option<String>,
        limit: u32,
    },
    ListSessions {
        request_id: ManagerRequestId,
    },
    StartProfile {
        request_id: ManagerRequestId,
        profile_id: String,
        expected_revision: i64,
    },
    ObserveSession {
        request_id: ManagerRequestId,
        session_id: String,
    },
    StopSession {
        request_id: ManagerRequestId,
        session_id: String,
    },
    RetryCleanup {
        request_id: ManagerRequestId,
        session_id: String,
    },
    ListAccounts {
        request_id: ManagerRequestId,
    },
    ImportAccount {
        request_id: ManagerRequestId,
        username: String,
        password: ManagerAccountPassword,
    },
    UpdateAccount {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: String,
        replacement_password: Option<ManagerAccountPassword>,
    },
    /// Sets which world one account logs into. Separate from `UpdateAccount` because it carries no
    /// identity and no secret, so the operator is never asked to re-enter anything.
    SetAccountServer {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
        server_index: u8,
    },
    DeleteAccount {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
        expected_revision: i64,
    },
    RunAccounts {
        request_id: ManagerRequestId,
        requests: Vec<(ManagerAccountId, i64)>,
    },
    StopAccount {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
    },
    RetryAccountCleanup {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
    },
    /// Reads one account's published character reading. Carries no identity beyond the opaque handle.
    ObserveAccountPlayer {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
    },
    /// Writes one account's attack and item settings, so its running client picks them up.
    SetAccountControl {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    },
    /// Reads every saved monster spot, keyed by map. Shared by all accounts, so it carries no
    /// account handle at all.
    ObserveSpots {
        request_id: ManagerRequestId,
    },
    /// Saves one map's spot, replacing whatever that map held.
    SaveSpot {
        request_id: ManagerRequestId,
        spot: AttackSpot,
        name: String,
    },
    /// Forgets one map's spot, leaving that map empty again.
    ClearSpot {
        request_id: ManagerRequestId,
        map_id: u16,
        name: String,
    },
    /// Reads one account's attack and item settings back, for the dialog to open on.
    ObserveAccountControl {
        request_id: ManagerRequestId,
        account_id: ManagerAccountId,
    },
    Shutdown {
        request_id: ManagerRequestId,
    },
    #[cfg(all(test, windows))]
    BirthForLiveTest {
        session_id: String,
        reply: SyncSender<Option<ProcessBirthId>>,
    },
    #[cfg(test)]
    Probe,
}

pub(super) trait WorkerControl: Send + 'static {
    fn list_profiles(
        &self,
        after: Option<&str>,
        limit: u32,
        archived: bool,
    ) -> ManagerResult<ManagerProfilePage>;
    fn list_runtimes(&self, after: Option<&str>, limit: u32) -> ManagerResult<ManagerRuntimePage>;
    fn list_sessions(&self) -> Vec<ManagerSessionView>;
    fn start_profile(&mut self, profile: &str, revision: i64) -> ManagerResult<ManagerSessionView>;
    fn observe_session(&mut self, session: &str) -> ManagerResult<ManagerObservation>;
    fn stop_session(&mut self, session: &str) -> ManagerResult<ManagerSessionExit>;
    fn retry_cleanup(&mut self, session: &str) -> ManagerResult<()>;
    /// Account operations default to an invariant failure so lifecycle-only test doubles, which never
    /// receive an account command, need no account behavior.
    fn list_accounts(&self) -> ManagerResult<Vec<ManagerAccountView>> {
        Err(unsupported_account_operation(
            ManagerOperation::ListAccounts,
        ))
    }
    fn import_account(
        &mut self,
        _username: &str,
        _password: ManagerAccountPassword,
    ) -> ManagerResult<ManagerAccountView> {
        Err(unsupported_account_operation(
            ManagerOperation::ImportAccount,
        ))
    }
    fn update_account(
        &mut self,
        _account_id: ManagerAccountId,
        _expected_revision: i64,
        _username: &str,
        _replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerResult<ManagerAccountView> {
        Err(unsupported_account_operation(
            ManagerOperation::UpdateAccount,
        ))
    }
    fn set_account_server(
        &mut self,
        _account_id: ManagerAccountId,
        _server_index: u8,
    ) -> ManagerResult<ManagerAccountView> {
        Err(unsupported_account_operation(
            ManagerOperation::SetAccountServer,
        ))
    }
    fn delete_account(
        &mut self,
        _account_id: ManagerAccountId,
        _expected_revision: i64,
    ) -> ManagerResult<()> {
        Err(unsupported_account_operation(
            ManagerOperation::DeleteAccount,
        ))
    }
    fn run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerResult<Vec<ManagerRunSchedule>> {
        let _ = requests;
        Err(unsupported_account_operation(ManagerOperation::RunAccounts))
    }
    /// Shares the worker's private completion queue with the readiness fleet.
    ///
    /// Defaulted so lifecycle-only test doubles, which never start a readiness task, need no behavior.
    fn attach_readiness_completions(&mut self, completions: Arc<WorkerWake<ReadinessCompletion>>) {
        let _ = completions;
    }

    /// Applies one terminal readiness completion, returning the row to notify.
    ///
    /// `None` means the record was stale or unmatched and was dropped without any state, persistence,
    /// or event change.
    fn apply_readiness_completion(
        &mut self,
        completion: ReadinessCompletion,
    ) -> Option<ManagerAccountView> {
        let _ = completion;
        None
    }
    fn stop_account(&mut self, account_id: ManagerAccountId) -> ManagerResult<ManagerAccountView> {
        let _ = account_id;
        Err(unsupported_account_operation(ManagerOperation::StopAccount))
    }
    fn retry_account_cleanup(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        let _ = account_id;
        Err(unsupported_account_operation(
            ManagerOperation::RetryAccountCleanup,
        ))
    }
    /// Reads one account's published character reading. Takes `&self`: it changes no lifecycle state.
    fn observe_account_player(
        &self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<Option<PlayerSnapshot>> {
        let _ = account_id;
        Err(unsupported_account_operation(
            ManagerOperation::ObserveAccountPlayer,
        ))
    }
    fn set_account_control(
        &self,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    ) -> ManagerResult<ControlSettings> {
        let _ = (account_id, settings);
        Err(unsupported_account_operation(
            ManagerOperation::SetAccountControl,
        ))
    }
    fn observe_spots(&self) -> ManagerResult<SpotBook> {
        Err(unsupported_account_operation(
            ManagerOperation::ObserveSpots,
        ))
    }
    fn save_spot(&mut self, spot: AttackSpot, name: &str) -> ManagerResult<SpotBook> {
        let _ = (spot, name);
        Err(unsupported_account_operation(ManagerOperation::SaveSpot))
    }
    fn clear_spot(&mut self, map_id: u16, name: &str) -> ManagerResult<SpotBook> {
        let _ = (map_id, name);
        Err(unsupported_account_operation(ManagerOperation::ClearSpot))
    }
    fn account_control(&self, account_id: ManagerAccountId) -> ManagerResult<ControlSettings> {
        let _ = account_id;
        Err(unsupported_account_operation(
            ManagerOperation::SetAccountControl,
        ))
    }
    fn close(&mut self) -> ManagerResult<()>;

    #[cfg(all(test, windows))]
    fn running_birth_id_for_live_test(&self, _session: &str) -> Option<ProcessBirthId> {
        None
    }
}

impl WorkerControl for ManagerController {
    fn list_profiles(
        &self,
        after: Option<&str>,
        limit: u32,
        archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        self.list_profiles(after, limit, archived)
    }

    fn list_runtimes(&self, after: Option<&str>, limit: u32) -> ManagerResult<ManagerRuntimePage> {
        self.list_runtimes(after, limit)
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        self.list_sessions()
    }

    fn start_profile(&mut self, profile: &str, revision: i64) -> ManagerResult<ManagerSessionView> {
        self.start_profile(profile, revision)
    }

    fn observe_session(&mut self, session: &str) -> ManagerResult<ManagerObservation> {
        self.observe_session(session)
    }

    fn stop_session(&mut self, session: &str) -> ManagerResult<ManagerSessionExit> {
        self.stop_session(session)
    }

    fn retry_cleanup(&mut self, session: &str) -> ManagerResult<()> {
        self.retry_cleanup(session)
    }

    fn list_accounts(&self) -> ManagerResult<Vec<ManagerAccountView>> {
        self.list_accounts()
    }

    fn import_account(
        &mut self,
        username: &str,
        password: ManagerAccountPassword,
    ) -> ManagerResult<ManagerAccountView> {
        self.import_account(username, password)
    }

    fn update_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: &str,
        replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerResult<ManagerAccountView> {
        self.update_account(
            account_id,
            expected_revision,
            username,
            replacement_password,
        )
    }

    fn set_account_server(
        &mut self,
        account_id: ManagerAccountId,
        server_index: u8,
    ) -> ManagerResult<ManagerAccountView> {
        self.set_account_server(account_id, server_index)
    }

    fn delete_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
    ) -> ManagerResult<()> {
        self.delete_account(account_id, expected_revision)
    }

    fn run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerResult<Vec<ManagerRunSchedule>> {
        self.run_accounts(requests)
    }

    fn apply_readiness_completion(
        &mut self,
        completion: ReadinessCompletion,
    ) -> Option<ManagerAccountView> {
        // The session key crosses the boundary as text, so an unparsable value is treated as stale
        // rather than panicking on a record this controller does not own.
        let session_key = uuid::Uuid::parse_str(&completion.session_key).ok()?;
        let succeeded = completion.outcome == ReadinessOutcome::InputSent;
        self.apply_readiness_completion(
            completion.account_id,
            session_key,
            completion.epoch,
            succeeded,
        )
    }

    fn stop_account(&mut self, account_id: ManagerAccountId) -> ManagerResult<ManagerAccountView> {
        self.stop_account(account_id)
    }

    fn retry_account_cleanup(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        self.retry_account_cleanup(account_id)
    }

    fn observe_account_player(
        &self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<Option<PlayerSnapshot>> {
        self.observe_account_player(account_id)
    }

    fn set_account_control(
        &self,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    ) -> ManagerResult<ControlSettings> {
        self.set_account_control(account_id, settings)
    }

    fn account_control(&self, account_id: ManagerAccountId) -> ManagerResult<ControlSettings> {
        self.account_control(account_id)
    }

    fn observe_spots(&self) -> ManagerResult<SpotBook> {
        self.saved_spot_book()
    }

    fn save_spot(&mut self, spot: AttackSpot, name: &str) -> ManagerResult<SpotBook> {
        self.store_spot(spot, name)
    }

    fn clear_spot(&mut self, map_id: u16, name: &str) -> ManagerResult<SpotBook> {
        self.forget_spot(map_id, name)
    }

    fn close(&mut self) -> ManagerResult<()> {
        self.close()
    }

    #[cfg(all(test, windows))]
    fn running_birth_id_for_live_test(&self, session: &str) -> Option<ProcessBirthId> {
        self.running_birth_id_for_test(session)
    }
}

fn unsupported_account_operation(operation: ManagerOperation) -> ManagerError {
    ManagerError::new(ManagerErrorCode::InternalInvariant, operation)
}

/// Bounded internal wake queue shared by the UI thread and the worker thread.
///
/// Only small owned values cross this mutex. Core, the controller, secrets, HWNDs, and public events
/// are deliberately never stored here: the worker thread owns them exclusively.
// `push` and the fields it reads have no production caller yet: no readiness task is spawned, so the
// completion queue is currently filled only by the worker's own tests. See the C2 gap in
// docs/windows-account-ui-v1-checkpoint.md.
#[allow(
    dead_code,
    reason = "no production readiness task pushes completions yet"
)]
pub(super) struct WorkerWake<T> {
    queue: Mutex<VecDeque<T>>,
    thread: Thread,
    capacity: usize,
}

#[allow(
    dead_code,
    reason = "no production readiness task pushes completions yet"
)]
impl<T> WorkerWake<T> {
    pub(super) fn new(thread: Thread, capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            thread,
            capacity,
        }
    }

    /// Pushes one value and unparks the worker, or returns it when the queue is full.
    ///
    /// The unpark happens after the lock is released, so the woken thread never immediately blocks on
    /// the mutex this call still holds.
    pub(super) fn push(&self, value: T) -> Result<(), T> {
        {
            let mut queue = self.lock();
            if queue.len() >= self.capacity {
                return Err(value);
            }
            queue.push_back(value);
        }
        self.thread.unpark();
        Ok(())
    }

    /// Removes every queued value, leaving the queue empty.
    pub(super) fn drain(&self) -> Vec<T> {
        self.lock().drain(..).collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// A poisoned wake queue cannot corrupt anything: the values are owned and independent, so the
    /// guard is recovered rather than propagating a panic into the worker loop.
    fn lock(&self) -> MutexGuard<'_, VecDeque<T>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) trait WorkerBoot: Send + 'static {
    type Control: WorkerControl;

    fn boot(self) -> ManagerResult<Self::Control>;
}

impl<F, C> WorkerBoot for F
where
    F: FnOnce() -> ManagerResult<C> + Send + 'static,
    C: WorkerControl,
{
    type Control = C;

    fn boot(self) -> ManagerResult<Self::Control> {
        self()
    }
}

pub(super) fn run_worker<B>(
    boot: B,
    command_rx: Receiver<WorkerCommand>,
    event_tx: SyncSender<ManagerWorkerEvent>,
) where
    B: WorkerBoot,
{
    let mut controller = match boot.boot() {
        Ok(controller) => controller,
        Err(error) => {
            let _ = deliver(&event_tx, ManagerWorkerEvent::OpenFailed(error));
            return;
        }
    };

    // Bounded private completion queue shared with the readiness tasks, plus this thread's waker.
    let completions: Arc<WorkerWake<ReadinessCompletion>> =
        Arc::new(WorkerWake::new(thread::current(), MAX_READINESS_TASKS));
    controller.attach_readiness_completions(Arc::clone(&completions));

    if !deliver(&event_tx, ManagerWorkerEvent::Ready) {
        cleanup_after_disconnect(&mut controller);
        return;
    }

    loop {
        // Loop order: drain private completions, drain commands, recheck, then park. Completions come
        // first so a terminal readiness result is never starved by a busy command queue.
        if !drain_readiness_completions(&completions, &mut controller, &event_tx) {
            cleanup_after_disconnect(&mut controller);
            return;
        }
        let command = match next_command(&command_rx, &completions) {
            NextCommand::Command(command) => command,
            NextCommand::Completions => continue,
            NextCommand::Disconnected => {
                cleanup_after_disconnect(&mut controller);
                return;
            }
        };

        if let WorkerCommand::Shutdown { request_id } = &command {
            let request_id = *request_id;
            let result = controller.close();
            let close_succeeded = result.is_ok();
            if !deliver(
                &event_tx,
                ManagerWorkerEvent::ShutdownResult { request_id, result },
            ) {
                if !close_succeeded {
                    cleanup_after_disconnect(&mut controller);
                }
                return;
            }
            if close_succeeded {
                return;
            }
            continue;
        }

        let event = match command {
            WorkerCommand::ListProfiles {
                request_id,
                after,
                limit,
                include_archived,
            } => ManagerWorkerEvent::ProfilesListed {
                request_id,
                result: controller.list_profiles(after.as_deref(), limit, include_archived),
            },
            WorkerCommand::ListRuntimes {
                request_id,
                after,
                limit,
            } => ManagerWorkerEvent::RuntimesListed {
                request_id,
                result: controller.list_runtimes(after.as_deref(), limit),
            },
            WorkerCommand::ListSessions { request_id } => ManagerWorkerEvent::SessionsListed {
                request_id,
                sessions: controller.list_sessions(),
            },
            WorkerCommand::StartProfile {
                request_id,
                profile_id,
                expected_revision,
            } => ManagerWorkerEvent::ProfileStarted {
                request_id,
                result: controller.start_profile(&profile_id, expected_revision),
            },
            WorkerCommand::ObserveSession {
                request_id,
                session_id,
            } => ManagerWorkerEvent::SessionObserved {
                request_id,
                result: controller.observe_session(&session_id),
            },
            WorkerCommand::StopSession {
                request_id,
                session_id,
            } => ManagerWorkerEvent::SessionStopped {
                request_id,
                result: controller.stop_session(&session_id),
            },
            WorkerCommand::ListAccounts { request_id } => ManagerWorkerEvent::AccountsListed {
                request_id,
                result: controller.list_accounts(),
            },
            WorkerCommand::ImportAccount {
                request_id,
                username,
                password,
            } => ManagerWorkerEvent::AccountImported {
                request_id,
                result: controller.import_account(&username, password),
            },
            WorkerCommand::UpdateAccount {
                request_id,
                account_id,
                expected_revision,
                username,
                replacement_password,
            } => ManagerWorkerEvent::AccountUpdated {
                request_id,
                result: controller.update_account(
                    account_id,
                    expected_revision,
                    &username,
                    replacement_password,
                ),
            },
            // Reported as an ordinary account update: the UI renders one row either way, and a
            // separate event would make every consumer handle two shapes of the same change.
            WorkerCommand::SetAccountServer {
                request_id,
                account_id,
                server_index,
            } => ManagerWorkerEvent::AccountUpdated {
                request_id,
                result: controller.set_account_server(account_id, server_index),
            },
            WorkerCommand::DeleteAccount {
                request_id,
                account_id,
                expected_revision,
            } => ManagerWorkerEvent::AccountDeleted {
                request_id,
                result: controller.delete_account(account_id, expected_revision),
            },
            WorkerCommand::RunAccounts {
                request_id,
                requests,
            } => ManagerWorkerEvent::AccountsRunScheduled {
                request_id,
                result: controller.run_accounts(&requests),
            },
            WorkerCommand::StopAccount {
                request_id,
                account_id,
            } => ManagerWorkerEvent::AccountStopResult {
                request_id,
                result: controller.stop_account(account_id),
            },
            WorkerCommand::RetryAccountCleanup {
                request_id,
                account_id,
            } => ManagerWorkerEvent::AccountCleanupRetried {
                request_id,
                result: controller.retry_account_cleanup(account_id),
            },
            WorkerCommand::ObserveAccountPlayer {
                request_id,
                account_id,
            } => ManagerWorkerEvent::AccountPlayerObserved {
                request_id,
                result: controller.observe_account_player(account_id),
            },
            WorkerCommand::SetAccountControl {
                request_id,
                account_id,
                settings,
            } => ManagerWorkerEvent::AccountControlApplied {
                request_id,
                result: controller.set_account_control(account_id, settings),
            },
            WorkerCommand::ObserveAccountControl {
                request_id,
                account_id,
            } => ManagerWorkerEvent::AccountControlApplied {
                request_id,
                result: controller.account_control(account_id),
            },
            WorkerCommand::ObserveSpots { request_id } => ManagerWorkerEvent::SpotsApplied {
                request_id,
                result: controller.observe_spots(),
            },
            WorkerCommand::SaveSpot {
                request_id,
                spot,
                name,
            } => ManagerWorkerEvent::SpotsApplied {
                request_id,
                result: controller.save_spot(spot, &name),
            },
            WorkerCommand::ClearSpot {
                request_id,
                map_id,
                name,
            } => ManagerWorkerEvent::SpotsApplied {
                request_id,
                result: controller.clear_spot(map_id, &name),
            },
            WorkerCommand::RetryCleanup {
                request_id,
                session_id,
            } => ManagerWorkerEvent::CleanupRetried {
                request_id,
                result: controller.retry_cleanup(&session_id),
            },
            WorkerCommand::Shutdown { .. } => unreachable!("shutdown is handled before dispatch"),
            #[cfg(all(test, windows))]
            WorkerCommand::BirthForLiveTest { session_id, reply } => {
                let _ = reply.send(controller.running_birth_id_for_live_test(&session_id));
                continue;
            }
            #[cfg(test)]
            WorkerCommand::Probe => continue,
        };

        if !deliver(&event_tx, event) {
            cleanup_after_disconnect(&mut controller);
            return;
        }
    }
}

/// Waits for the next command without a blocking receive, so `Drop` can wake a parked worker.
///
/// Returns `None` only when the command sender is gone, which is the worker's shutdown signal.
/// Why `next_command` returned.
enum NextCommand {
    Command(WorkerCommand),
    /// A completion arrived; the caller must drain it before parking again.
    Completions,
    Disconnected,
}

fn next_command(
    command_rx: &Receiver<WorkerCommand>,
    completions: &WorkerWake<ReadinessCompletion>,
) -> NextCommand {
    loop {
        match command_rx.try_recv() {
            Ok(command) => return NextCommand::Command(command),
            Err(TryRecvError::Disconnected) => return NextCommand::Disconnected,
            Err(TryRecvError::Empty) => {}
        }
        if !completions.is_empty() {
            return NextCommand::Completions;
        }
        // Recheck after the empty reads: a command, a completion, or a disconnect may have landed in
        // between, and a stale unpark token would otherwise be consumed by the park below.
        match command_rx.try_recv() {
            Ok(command) => return NextCommand::Command(command),
            Err(TryRecvError::Disconnected) => return NextCommand::Disconnected,
            Err(TryRecvError::Empty) => {
                if !completions.is_empty() {
                    return NextCommand::Completions;
                }
                thread::park();
            }
        }
    }
}

/// Drains every queued completion, publishing one notification per accepted record.
///
/// Returns `false` only when the event receiver is gone.
fn drain_readiness_completions<C>(
    completions: &WorkerWake<ReadinessCompletion>,
    controller: &mut C,
    event_tx: &SyncSender<ManagerWorkerEvent>,
) -> bool
where
    C: WorkerControl,
{
    for completion in completions.drain() {
        // A stale or unmatched record yields no row, so it is dropped silently by design.
        if let Some(view) = controller.apply_readiness_completion(completion)
            && !deliver(event_tx, ManagerWorkerEvent::AccountStateChanged(view))
        {
            return false;
        }
    }
    true
}

fn deliver(event_tx: &SyncSender<ManagerWorkerEvent>, event: ManagerWorkerEvent) -> bool {
    event_tx.send(event).is_ok()
}

fn cleanup_after_disconnect<C>(controller: &mut C)
where
    C: WorkerControl,
{
    let _ = controller.close();
}

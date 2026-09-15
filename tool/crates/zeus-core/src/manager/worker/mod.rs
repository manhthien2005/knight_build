//! Windows-only bounded worker over the manager control boundary.

use crate::control::AttackSpot;
use std::cell::Cell;
use std::marker::PhantomData;
use std::mem;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
#[cfg(all(test, windows))]
use std::time::Duration;

#[cfg(all(test, windows))]
use crate::process_adapter::ProcessBirthId;

mod account;
mod engine;
mod error;
// The readiness subsystem is reachable only from its own tests: nothing spawns a readiness task yet,
// so no production path constructs a job or drives the coordinator. This is the C2 gap recorded in
// docs/windows-account-ui-v1-checkpoint.md, not a task that already shipped.
#[allow(
    dead_code,
    reason = "no production readiness-task spawn seam exists yet"
)]
mod login;
mod types;

use super::{MAX_RUN_BATCH, ManagerAccountId, ManagerController};
use crate::ControlSettings;
use engine::{WorkerBoot, WorkerCommand, run_worker};

pub use account::ManagerAccountPassword;
pub use error::{
    ManagerWorkerError, ManagerWorkerErrorCode, ManagerWorkerOperation, ManagerWorkerResult,
};
pub use types::{ManagerRequestId, ManagerWorkerEvent, ManagerWorkerState};

const COMMAND_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 32;
const PROFILE_OR_SESSION_ID_MAX_BYTES: usize = 36;
/// Spec section 8 username bound.
const USERNAME_MAX_BYTES: usize = 64;
const RUNTIME_ID_MAX_BYTES: usize = 160;
const WORKER_THREAD_NAME: &str = "zeus-manager-worker-v1";

/// Single-owner, non-blocking handle to the dedicated manager worker thread.
pub struct ManagerWorker {
    command_tx: SyncSender<WorkerCommand>,
    event_rx: Receiver<ManagerWorkerEvent>,
    join: Option<JoinHandle<()>>,
    state: ManagerWorkerState,
    next_request_id: Option<NonZeroU64>,
    shutdown_in_flight: bool,
    _not_sync: PhantomData<Cell<()>>,
}

impl ManagerWorker {
    pub fn spawn_at(path: &Path) -> ManagerWorkerResult<Self> {
        let path = path.to_owned();
        Self::spawn_with_boot(move || ManagerController::open_at(&path))
    }

    /// Boots a portable Core: repairs a copied data root, then relocates the pinned runtime.
    pub fn spawn_portable(
        data_root: &Path,
        exact_runtime_root: &Path,
    ) -> ManagerWorkerResult<Self> {
        let data_root = data_root.to_owned();
        let exact_runtime_root = exact_runtime_root.to_owned();
        // All portable boot work happens on the worker thread; a failure surfaces as OpenFailed.
        Self::spawn_with_boot(move || {
            ManagerController::open_portable_at(&data_root, &exact_runtime_root)
        })
    }

    pub fn spawn_default() -> ManagerWorkerResult<Self> {
        Self::spawn_with_boot(ManagerController::open_default)
    }

    pub fn spawn_controller(controller: ManagerController) -> ManagerWorkerResult<Self> {
        Self::spawn_with_boot(move || Ok(controller))
    }

    fn spawn_with_boot<B>(boot: B) -> ManagerWorkerResult<Self>
    where
        B: WorkerBoot,
    {
        let (command_tx, command_rx) = sync_channel(COMMAND_CAPACITY);
        let (event_tx, event_rx) = sync_channel(EVENT_CAPACITY);
        let join = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || run_worker(boot, command_rx, event_tx))
            .map_err(|_| {
                ManagerWorkerError::new(
                    ManagerWorkerErrorCode::ThreadSpawnFailed,
                    ManagerWorkerOperation::Spawn,
                )
            })?;

        Ok(Self {
            command_tx,
            event_rx,
            join: Some(join),
            state: ManagerWorkerState::Starting,
            next_request_id: NonZeroU64::new(1),
            shutdown_in_flight: false,
            _not_sync: PhantomData,
        })
    }

    pub fn state(&self) -> ManagerWorkerState {
        self.state
    }

    pub fn try_list_profiles(
        &mut self,
        after_profile_id: Option<&str>,
        limit: u32,
        include_archived: bool,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ListProfiles;
        self.require_admitted(operation)?;
        Self::validate_optional_input(
            after_profile_id,
            PROFILE_OR_SESSION_ID_MAX_BYTES,
            operation,
        )?;
        self.admit_command(operation, |request_id| WorkerCommand::ListProfiles {
            request_id,
            after: after_profile_id.map(str::to_owned),
            limit,
            include_archived,
        })
    }

    pub fn try_list_runtimes(
        &mut self,
        after_runtime_id: Option<&str>,
        limit: u32,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ListRuntimes;
        self.require_admitted(operation)?;
        Self::validate_optional_input(after_runtime_id, RUNTIME_ID_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ListRuntimes {
            request_id,
            after: after_runtime_id.map(str::to_owned),
            limit,
        })
    }

    pub fn try_list_sessions(&mut self) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ListSessions;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ListSessions {
            request_id,
        })
    }

    pub fn try_start_profile(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::StartProfile;
        self.require_admitted(operation)?;
        Self::validate_input(profile_id, PROFILE_OR_SESSION_ID_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::StartProfile {
            request_id,
            profile_id: profile_id.to_owned(),
            expected_revision,
        })
    }

    pub fn try_observe_session(
        &mut self,
        session_id: &str,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ObserveSession;
        self.require_admitted(operation)?;
        Self::validate_input(session_id, PROFILE_OR_SESSION_ID_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ObserveSession {
            request_id,
            session_id: session_id.to_owned(),
        })
    }

    pub fn try_stop_session(&mut self, session_id: &str) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::StopSession;
        self.require_admitted(operation)?;
        Self::validate_input(session_id, PROFILE_OR_SESSION_ID_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::StopSession {
            request_id,
            session_id: session_id.to_owned(),
        })
    }

    pub fn try_retry_cleanup(&mut self, session_id: &str) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::RetryCleanup;
        self.require_admitted(operation)?;
        Self::validate_input(session_id, PROFILE_OR_SESSION_ID_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::RetryCleanup {
            request_id,
            session_id: session_id.to_owned(),
        })
    }

    pub fn try_list_accounts(&mut self) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ListAccounts;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ListAccounts {
            request_id,
        })
    }

    pub fn try_import_account(
        &mut self,
        username: &str,
        password: ManagerAccountPassword,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ImportAccount;
        self.require_admitted(operation)?;
        Self::validate_input(username, USERNAME_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ImportAccount {
            request_id,
            username: username.to_owned(),
            password,
        })
    }

    pub fn try_update_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: &str,
        replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::UpdateAccount;
        self.require_admitted(operation)?;
        Self::validate_input(username, USERNAME_MAX_BYTES, operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::UpdateAccount {
            request_id,
            account_id,
            expected_revision,
            username: username.to_owned(),
            replacement_password,
        })
    }

    /// Sets which world one account logs into.
    ///
    /// The index is validated here so an out-of-range value never reaches the worker thread, and
    /// again in Core before it is stored.
    pub fn try_set_account_server(
        &mut self,
        account_id: ManagerAccountId,
        server_index: u8,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::SetAccountServer;
        self.require_admitted(operation)?;
        if usize::from(server_index) >= crate::SERVER_NAMES.len() {
            return Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::InvalidInput,
                operation,
            ));
        }
        self.admit_command(operation, |request_id| WorkerCommand::SetAccountServer {
            request_id,
            account_id,
            server_index,
        })
    }

    pub fn try_delete_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::DeleteAccount;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::DeleteAccount {
            request_id,
            account_id,
            expected_revision,
        })
    }

    /// Runs one account. This is the one-element adapter over [`Self::try_run_accounts`].
    pub fn try_run_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        self.try_run_accounts(&[(account_id, expected_revision)])
    }

    /// Runs a bounded batch of at most [`MAX_RUN_BATCH`] accounts.
    ///
    /// One request result carries at most four redacted per-account schedule outcomes. Admission
    /// closes one generation before any member reaches input readiness, and a batch larger than the
    /// ceiling is refused before profile start, secret decrypt, or task creation.
    pub fn try_run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::RunAccounts;
        self.require_admitted(operation)?;
        if requests.is_empty() {
            return Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::InvalidInput,
                operation,
            ));
        }
        if requests.len() > MAX_RUN_BATCH {
            return Err(
                ManagerWorkerError::new(ManagerWorkerErrorCode::InputTooLong, operation)
                    .with_maximum(MAX_RUN_BATCH as u32),
            );
        }
        let requests = requests.to_vec();
        self.admit_command(operation, move |request_id| WorkerCommand::RunAccounts {
            request_id,
            requests,
        })
    }

    /// Stops the live session owned by one account. Run is added by the Task 9 coordinator.
    pub fn try_stop_account(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::StopAccount;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::StopAccount {
            request_id,
            account_id,
        })
    }

    pub fn try_retry_account_cleanup(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::RetryAccountCleanup;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::RetryAccountCleanup {
            request_id,
            account_id,
        })
    }

    /// Requests one account's published character reading.
    ///
    /// Polled rather than pushed: the client rewrites the reading about once a second, and only the row
    /// the operator is looking at needs to be current, so a push would spend the bounded event queue on
    /// rows nobody is reading.
    pub fn try_observe_account_player(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ObserveAccountPlayer;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| {
            WorkerCommand::ObserveAccountPlayer {
                request_id,
                account_id,
            }
        })
    }

    /// Writes one account's attack and item settings.
    ///
    /// Accepted whether or not the account is running: the client polls the file, so a change takes
    /// effect mid-session, and a change made before a run arms it for the start.
    pub fn try_set_account_control(
        &mut self,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::SetAccountControl;
        self.require_admitted(operation)?;
        self.admit_command(operation, move |request_id| {
            WorkerCommand::SetAccountControl {
                request_id,
                account_id,
                settings,
            }
        })
    }

    /// Reads every saved monster spot. Shared by all accounts, so it takes no account handle.
    pub fn try_observe_spots(&mut self) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ObserveSpots;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| WorkerCommand::ObserveSpots {
            request_id,
        })
    }

    /// Saves one map's spot, replacing whatever that map held.
    pub fn try_save_spot(
        &mut self,
        spot: AttackSpot,
        name: String,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::SaveSpot;
        self.require_admitted(operation)?;
        self.admit_command(operation, move |request_id| WorkerCommand::SaveSpot {
            request_id,
            spot,
            name,
        })
    }

    /// Forgets one map's spot, leaving that map empty again.
    pub fn try_clear_spot(
        &mut self,
        map_id: u16,
        name: String,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ClearSpot;
        self.require_admitted(operation)?;
        self.admit_command(operation, move |request_id| WorkerCommand::ClearSpot {
            request_id,
            map_id,
            name,
        })
    }

    /// Reads one account's attack and item settings back, so a dialog opens on the current values.
    pub fn try_observe_account_control(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::ObserveAccountControl;
        self.require_admitted(operation)?;
        self.admit_command(operation, |request_id| {
            WorkerCommand::ObserveAccountControl {
                request_id,
                account_id,
            }
        })
    }

    pub fn try_shutdown(&mut self) -> ManagerWorkerResult<ManagerRequestId> {
        let operation = ManagerWorkerOperation::Shutdown;
        self.require_admitted(operation)?;
        let request_id = self.admit_command(operation, |request_id| WorkerCommand::Shutdown {
            request_id,
        })?;
        self.state = ManagerWorkerState::Closing;
        self.shutdown_in_flight = true;
        Ok(request_id)
    }

    pub fn try_next_event(&mut self) -> ManagerWorkerResult<Option<ManagerWorkerEvent>> {
        if self.state == ManagerWorkerState::Closed {
            return Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::Closed,
                ManagerWorkerOperation::ReceiveEvent,
            ));
        }

        match self.event_rx.try_recv() {
            Ok(event) => {
                match &event {
                    ManagerWorkerEvent::Ready => self.state = ManagerWorkerState::Ready,
                    ManagerWorkerEvent::OpenFailed(_) => self.state = ManagerWorkerState::Closed,
                    ManagerWorkerEvent::ShutdownResult { result, .. } => {
                        self.shutdown_in_flight = false;
                        self.state = if result.is_ok() {
                            ManagerWorkerState::Closed
                        } else {
                            ManagerWorkerState::Closing
                        };
                    }
                    _ => {}
                }
                Ok(Some(event))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.state = ManagerWorkerState::Closed;
                Err(ManagerWorkerError::new(
                    ManagerWorkerErrorCode::Disconnected,
                    ManagerWorkerOperation::ReceiveEvent,
                ))
            }
        }
    }

    #[cfg(all(test, windows))]
    fn running_birth_id_for_live_test(&mut self, session_id: &str) -> Option<ProcessBirthId> {
        let (reply, response) = sync_channel(1);
        self.command_tx
            .try_send(WorkerCommand::BirthForLiveTest {
                session_id: session_id.to_owned(),
                reply,
            })
            .ok()?;
        response
            .recv_timeout(Duration::from_secs(10))
            .ok()
            .flatten()
    }

    fn require_admitted(&self, operation: ManagerWorkerOperation) -> ManagerWorkerResult<()> {
        let code = match self.state {
            ManagerWorkerState::Starting => ManagerWorkerErrorCode::NotReady,
            ManagerWorkerState::Ready if self.shutdown_in_flight => {
                ManagerWorkerErrorCode::ShutdownPending
            }
            ManagerWorkerState::Ready => return Ok(()),
            ManagerWorkerState::Closing if self.shutdown_in_flight => {
                ManagerWorkerErrorCode::ShutdownPending
            }
            ManagerWorkerState::Closing
                if matches!(
                    operation,
                    ManagerWorkerOperation::ListSessions
                        | ManagerWorkerOperation::ObserveSession
                        | ManagerWorkerOperation::StopSession
                        | ManagerWorkerOperation::RetryCleanup
                        | ManagerWorkerOperation::Shutdown
                ) =>
            {
                return Ok(());
            }
            ManagerWorkerState::Closing => ManagerWorkerErrorCode::Closing,
            ManagerWorkerState::Closed => ManagerWorkerErrorCode::Closed,
        };
        Err(ManagerWorkerError::new(code, operation))
    }

    fn validate_optional_input(
        value: Option<&str>,
        maximum: usize,
        operation: ManagerWorkerOperation,
    ) -> ManagerWorkerResult<()> {
        if let Some(value) = value {
            Self::validate_input(value, maximum, operation)?;
        }
        Ok(())
    }

    fn validate_input(
        value: &str,
        maximum: usize,
        operation: ManagerWorkerOperation,
    ) -> ManagerWorkerResult<()> {
        if value.len() > maximum {
            return Err(
                ManagerWorkerError::new(ManagerWorkerErrorCode::InputTooLong, operation)
                    .with_maximum(maximum as u32),
            );
        }
        Ok(())
    }

    /// Wakes the worker thread. Never blocks, so it is safe from `Drop` and from every UI call.
    fn unpark_worker(&self) {
        if let Some(handle) = self.join.as_ref() {
            handle.thread().unpark();
        }
    }

    fn admit_command<F>(
        &mut self,
        operation: ManagerWorkerOperation,
        build_command: F,
    ) -> ManagerWorkerResult<ManagerRequestId>
    where
        F: FnOnce(ManagerRequestId) -> WorkerCommand,
    {
        let next_request_id = self.next_request_id.ok_or_else(|| {
            ManagerWorkerError::new(ManagerWorkerErrorCode::RequestIdExhausted, operation)
        })?;
        let request_id = ManagerRequestId::new(next_request_id);
        let command = build_command(request_id);

        match self.command_tx.try_send(command) {
            Ok(()) => {
                // The worker parks instead of blocking on recv, so an accepted command must unpark it.
                self.unpark_worker();
                self.next_request_id = next_request_id
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU64::new);
                Ok(request_id)
            }
            Err(TrySendError::Full(_)) => Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::CommandQueueFull,
                operation,
            )
            .with_maximum(COMMAND_CAPACITY as u32)),
            Err(TrySendError::Disconnected(_)) => {
                self.state = ManagerWorkerState::Closed;
                Err(ManagerWorkerError::new(
                    ManagerWorkerErrorCode::Disconnected,
                    operation,
                ))
            }
        }
    }

    #[cfg(test)]
    fn try_test_probe(&mut self) -> ManagerWorkerResult<()> {
        match self.state {
            ManagerWorkerState::Starting => {
                return Err(ManagerWorkerError::new(
                    ManagerWorkerErrorCode::NotReady,
                    ManagerWorkerOperation::ListSessions,
                ));
            }
            ManagerWorkerState::Closing => {
                return Err(ManagerWorkerError::new(
                    ManagerWorkerErrorCode::Closing,
                    ManagerWorkerOperation::ListSessions,
                ));
            }
            ManagerWorkerState::Closed => {
                return Err(ManagerWorkerError::new(
                    ManagerWorkerErrorCode::Closed,
                    ManagerWorkerOperation::ListSessions,
                ));
            }
            ManagerWorkerState::Ready => {}
        }

        self.command_tx
            .try_send(WorkerCommand::Probe)
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => ManagerWorkerError::new(
                    ManagerWorkerErrorCode::CommandQueueFull,
                    ManagerWorkerOperation::ListSessions,
                )
                .with_maximum(COMMAND_CAPACITY as u32),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    self.state = ManagerWorkerState::Closed;
                    ManagerWorkerError::new(
                        ManagerWorkerErrorCode::Disconnected,
                        ManagerWorkerOperation::ListSessions,
                    )
                }
            })
    }

    #[cfg(test)]
    fn disconnect_command_sender_for_test(&mut self) {
        let (replacement_command_tx, replacement_command_rx) = sync_channel(0);
        drop(replacement_command_rx);
        drop(mem::replace(&mut self.command_tx, replacement_command_tx));
        // Dropping a sender never wakes a parked thread, so every disconnect must unpark.
        self.unpark_worker();
    }
}

impl Drop for ManagerWorker {
    fn drop(&mut self) {
        // Disconnect both bounded queues before detaching; UI-side Drop never joins the worker.
        let (replacement_command_tx, replacement_command_rx) = sync_channel(0);
        drop(replacement_command_rx);
        drop(mem::replace(&mut self.command_tx, replacement_command_tx));

        let (replacement_event_tx, replacement_event_rx) = sync_channel(0);
        drop(replacement_event_tx);
        drop(mem::replace(&mut self.event_rx, replacement_event_rx));

        // Both queues are disconnected, so the woken worker observes the disconnect and closes Core.
        // Without this unpark a parked worker would hold the Core instance lock forever.
        self.unpark_worker();
        drop(self.join.take());
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_live_runtime_tests;

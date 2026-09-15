//! Windows-only manager control boundary: redacted public types over Core.

mod account;
mod error;
mod types;
mod worker;

use std::cell::Cell;
use std::marker::PhantomData;
use std::path::Path;
use std::time::{Duration, Instant};

use uuid::{Uuid, Version};

#[cfg(all(test, windows))]
use crate::process_adapter::ProcessBirthId;
use crate::process_adapter::{WindowsProcessError, WindowsProcessErrorKind};
use std::sync::atomic::Ordering;

use crate::control::AttackSpot;
use crate::session_supervisor::MAX_ACTIVE_SESSIONS;
use crate::session_supervisor::{
    ProcessBackend, SessionExit, SessionFailure, SessionMetadata, SessionObservation,
    SessionStateKind, SessionSummary, StartSessionFailure, SupervisorCore, WindowsProcessBackend,
};
use crate::spots::SpotBook;
use crate::store::StoredAccount;
use crate::{ControlSettings, CoreState, PlayerSnapshot, ProfileRecord, RuntimeRecord};

use account::AccountSessionState;
pub use account::{
    MAX_RUN_BATCH, ManagerAccountId, ManagerAccountStatus, ManagerAccountView, ManagerRunRejection,
    ManagerRunSchedule, ManagerRunScheduleOutcome,
};
pub use error::{ManagerError, ManagerErrorCode, ManagerOperation, ManagerResult};
pub use types::{
    ManagerObservation, ManagerProfilePage, ManagerProfileView, ManagerRuntimePage,
    ManagerRuntimeView, ManagerSessionExit, ManagerSessionState, ManagerSessionView,
};
pub use worker::{
    ManagerAccountPassword, ManagerRequestId, ManagerWorker, ManagerWorkerError,
    ManagerWorkerErrorCode, ManagerWorkerEvent, ManagerWorkerOperation, ManagerWorkerResult,
    ManagerWorkerState,
};

const MANAGER_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

trait DeadlineSource {
    fn now(&mut self) -> Instant;
}

struct SystemDeadlineSource;

impl DeadlineSource for SystemDeadlineSource {
    fn now(&mut self) -> Instant {
        Instant::now()
    }
}

trait RedactedProcessError {
    fn deadline_expired(&self) -> bool;
}

impl RedactedProcessError for WindowsProcessError {
    fn deadline_expired(&self) -> bool {
        self.kind() == WindowsProcessErrorKind::DeadlineExpired
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerState {
    Open,
    Closing,
    Closed,
}

struct ManagerCore<B: ProcessBackend, D> {
    supervisor: SupervisorCore<B>,
    deadlines: D,
    state: ControllerState,
}

impl<B: ProcessBackend, D> ManagerCore<B, D> {
    fn new(core: CoreState, backend: B, deadlines: D) -> Self {
        Self {
            supervisor: SupervisorCore::new(core, backend),
            deadlines,
            state: ControllerState::Open,
        }
    }

    fn list_profiles(
        &self,
        after_profile_id: Option<&str>,
        limit: u32,
        include_archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        self.ensure_available(ManagerOperation::ListProfiles)?;
        let page = self
            .supervisor
            .core()
            .list_profiles(after_profile_id, limit, include_archived)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::ListProfiles))?;
        let sessions = self.supervisor.session_summaries();
        Ok(ManagerProfilePage {
            items: page
                .items
                .into_iter()
                .map(|record| profile_view(record, &sessions))
                .collect(),
            next_cursor: page.next_cursor,
        })
    }

    fn list_runtimes(
        &self,
        after_runtime_id: Option<&str>,
        limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        self.ensure_available(ManagerOperation::ListRuntimes)?;
        self.supervisor
            .core()
            .list_runtimes(after_runtime_id, limit)
            .map(|page| ManagerRuntimePage {
                items: page.items.into_iter().map(runtime_view).collect(),
                next_cursor: page.next_cursor,
            })
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::ListRuntimes))
    }

    fn list_sessions(&self) -> Vec<ManagerSessionView> {
        if self.state == ControllerState::Closed {
            return Vec::new();
        }
        let mut sessions = self
            .supervisor
            .session_summaries()
            .into_iter()
            .map(session_view)
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        sessions
    }

    fn list_accounts(&self) -> ManagerResult<Vec<ManagerAccountView>> {
        self.ensure_available(ManagerOperation::ListAccounts)?;
        let accounts = self
            .supervisor
            .core()
            .list_accounts()
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::ListAccounts))?;
        let sessions = self.supervisor.session_summaries();
        Ok(accounts
            .iter()
            .map(|account| {
                ManagerAccountView::from_stored(account, account_session(account, &sessions))
            })
            .collect())
    }

    fn import_account(
        &mut self,
        username: &str,
        password: ManagerAccountPassword,
    ) -> ManagerResult<ManagerAccountView> {
        self.ensure_available(ManagerOperation::ImportAccount)?;
        let stored = self
            .supervisor
            .core_mut_for_account()
            .create_account_with_profile(username, password.into_secret())
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::ImportAccount))?;
        // A freshly imported account has no session and no persisted outcome, so it is always Idle.
        Ok(ManagerAccountView::from_stored(&stored, None))
    }

    fn update_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: &str,
        replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerResult<ManagerAccountView> {
        self.ensure_available(ManagerOperation::UpdateAccount)?;
        let replacement = replacement_password.map(ManagerAccountPassword::into_secret);
        let stored = self
            .supervisor
            .core_mut_for_account()
            .update_account_and_profile(account_id.get(), expected_revision, username, replacement)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::UpdateAccount))?;
        let sessions = self.supervisor.session_summaries();
        let session = account_session(&stored, &sessions);
        Ok(ManagerAccountView::from_stored(&stored, session))
    }

    /// Sets which world one account logs into.
    ///
    /// Deliberately not folded into [`Self::update_account`]: that call requires the operator's
    /// current username and treats an absent password as "keep", so routing a server change through it
    /// would make choosing a world look like an identity edit.
    fn set_account_server(
        &mut self,
        account_id: ManagerAccountId,
        server_index: u8,
    ) -> ManagerResult<ManagerAccountView> {
        self.ensure_available(ManagerOperation::SetAccountServer)?;
        let stored = self
            .supervisor
            .core_mut_for_account()
            .set_account_server(account_id.get(), server_index)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::SetAccountServer))?;
        let sessions = self.supervisor.session_summaries();
        let session = account_session(&stored, &sessions);
        Ok(ManagerAccountView::from_stored(&stored, session))
    }

    fn delete_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
    ) -> ManagerResult<()> {
        self.ensure_available(ManagerOperation::DeleteAccount)?;
        self.supervisor
            .core_mut_for_account()
            .delete_account_and_archive_profile(account_id.get(), expected_revision)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::DeleteAccount))
    }

    /// Resolves account -> linked profile -> live session, reporting a redacted failure otherwise.
    fn account_session_id(
        &self,
        account_id: ManagerAccountId,
        operation: ManagerOperation,
    ) -> ManagerResult<Uuid> {
        let profile_id = self
            .supervisor
            .core()
            .account_profile_id(account_id.get())
            .map_err(|error| ManagerError::from_core(error, operation))?;
        let profile_id = Uuid::parse_str(&profile_id)
            .map_err(|_| ManagerError::new(ManagerErrorCode::InternalInvariant, operation))?;
        self.supervisor
            .session_for_profile(profile_id)
            .ok_or_else(|| ManagerError::new(ManagerErrorCode::AccountNotRunning, operation))
    }

    /// Reconciled view of one account, for a caller that must report its state after a lifecycle change.
    ///
    /// Reads the single row rather than the whole list: the caller already knows which account moved,
    /// and a stop that answered with only a process exit left the UI holding a pending status forever.
    fn account_view(
        &self,
        account_id: ManagerAccountId,
        operation: ManagerOperation,
    ) -> ManagerResult<ManagerAccountView> {
        let stored = self
            .supervisor
            .core()
            .account(account_id.get())
            .map_err(|error| ManagerError::from_core(error, operation))?;
        let sessions = self.supervisor.session_summaries();
        let session = account_session(&stored, &sessions);
        Ok(ManagerAccountView::from_stored(&stored, session))
    }

    fn ensure_available(&self, operation: ManagerOperation) -> ManagerResult<()> {
        if self.state == ControllerState::Closed {
            Err(ManagerError::new(
                ManagerErrorCode::ControllerClosed,
                operation,
            ))
        } else {
            Ok(())
        }
    }

    fn ensure_start_allowed(&self) -> ManagerResult<()> {
        match self.state {
            ControllerState::Open => Ok(()),
            ControllerState::Closing => Err(ManagerError::new(
                ManagerErrorCode::ControllerClosing,
                ManagerOperation::StartProfile,
            )),
            ControllerState::Closed => Err(ManagerError::new(
                ManagerErrorCode::ControllerClosed,
                ManagerOperation::StartProfile,
            )),
        }
    }

    #[cfg(test)]
    fn supervisor(&self) -> &SupervisorCore<B> {
        &self.supervisor
    }
}

impl<B, D> ManagerCore<B, D>
where
    B: ProcessBackend,
    B::Error: RedactedProcessError,
    D: DeadlineSource,
{
    fn start_profile(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        self.ensure_start_allowed()?;
        let deadline = self.fresh_deadline(ManagerOperation::StartProfile)?;
        match self
            .supervisor
            .start_session(profile_id, expected_revision, deadline)
        {
            Ok(started) => Ok(session_metadata_view(
                started.metadata(),
                ManagerSessionState::Running,
            )),
            Err(error) => Err(self.map_start_failure(error)),
        }
    }

    fn observe_session(&mut self, session_id: &str) -> ManagerResult<ManagerObservation> {
        self.ensure_available(ManagerOperation::ObserveSession)?;
        let session_id = parse_session_id(session_id, ManagerOperation::ObserveSession)?;
        let deadline = self.fresh_deadline(ManagerOperation::ObserveSession)?;
        self.supervisor
            .observe_session(session_id, deadline)
            .map(|observation| match observation {
                SessionObservation::Running { metadata, .. } => ManagerObservation::Running(
                    session_metadata_view(&metadata, ManagerSessionState::Running),
                ),
                SessionObservation::CleanupPending { metadata } => {
                    ManagerObservation::CleanupPending(session_metadata_view(
                        &metadata,
                        ManagerSessionState::CleanupPending,
                    ))
                }
                SessionObservation::Exited(exited) => {
                    ManagerObservation::Exited(session_exit_view(&exited))
                }
            })
            .map_err(|error| self.map_session_failure(error, ManagerOperation::ObserveSession))
    }

    /// Applies one terminal readiness completion, returning the row to notify.
    ///
    /// The record is accepted only when its account still resolves to the same live session and that
    /// session's epoch is unchanged. A stale record is dropped with no persistence and no event, because
    /// the session it referred to is already gone.
    ///
    /// A failed login stops only that account. Every other session keeps running.
    fn apply_readiness_completion(
        &mut self,
        account_id: ManagerAccountId,
        session_key: Uuid,
        epoch: u64,
        succeeded: bool,
    ) -> Option<ManagerAccountView> {
        let operation = ManagerOperation::RunAccounts;
        let live_session = self.account_session_id(account_id, operation).ok()?;
        if live_session != session_key {
            return None;
        }
        // The epoch is bumped before any stop or cleanup, so a mismatch means this session is being
        // torn down and the record must not touch persisted state.
        let current_epoch = self.supervisor.session_epoch(session_key)?;
        if current_epoch.load(Ordering::Acquire) != epoch {
            return None;
        }

        let stored = if succeeded {
            self.supervisor
                .core_mut_for_account()
                .mark_account_started(account_id.get())
        } else {
            self.supervisor
                .core_mut_for_account()
                .mark_account_login_failed(account_id.get())
        }
        .ok()?;

        if !succeeded {
            // Confirmed stop for this account only, so a login failure never leaves the game running.
            if let Ok(deadline) = self.fresh_deadline(operation) {
                let _ = self.supervisor.stop_session(session_key, deadline);
            }
        }
        let sessions = self.supervisor.session_summaries();
        let session = account_session(&stored, &sessions);
        Some(ManagerAccountView::from_stored(&stored, session))
    }

    /// Starts one bounded batch of accounts, one at a time, on this thread.
    ///
    /// Lifecycle stays serialized: each member's profile start completes before the next begins, so no
    /// two starts race. Independence is per member, not per call: one member's failure is recorded as
    /// its own rejection and never aborts the batch.
    ///
    /// The batch is refused whole when it would exceed the four-session ceiling, so a fifth member
    /// never reaches profile start or secret decrypt.
    fn run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerResult<Vec<ManagerRunSchedule>> {
        let operation = ManagerOperation::RunAccounts;
        self.ensure_available(operation)?;
        if requests.is_empty() || requests.len() > MAX_ACTIVE_SESSIONS {
            return Err(ManagerError::new(
                ManagerErrorCode::CapacityReached,
                operation,
            ));
        }
        let available = MAX_ACTIVE_SESSIONS.saturating_sub(self.supervisor.active_session_count());
        let mut schedules = Vec::with_capacity(requests.len());
        let mut admitted = 0usize;
        for (account_id, expected_revision) in requests {
            let outcome = self.schedule_one_account(
                *account_id,
                *expected_revision,
                admitted < available,
                operation,
            );
            if outcome == ManagerRunScheduleOutcome::Scheduled {
                admitted += 1;
            }
            schedules.push(ManagerRunSchedule {
                account_id: *account_id,
                outcome,
            });
        }
        Ok(schedules)
    }

    /// Starts one member, mapping every failure to its own bounded rejection.
    fn schedule_one_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        has_capacity: bool,
        operation: ManagerOperation,
    ) -> ManagerRunScheduleOutcome {
        if !has_capacity {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::TaskLimitReached);
        }
        // An account already holding a live session is rejected rather than started twice.
        if self.account_session_id(account_id, operation).is_ok() {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::AlreadyRunning);
        }
        let Ok(profile_id) = self.supervisor.core().account_profile_id(account_id.get()) else {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed);
        };
        // The caller's revision guards the ACCOUNT row it read, so it is checked against the account
        // here. The launch snapshot guards the PROFILE row, whose revision is an independent counter —
        // passing the account's revision to the snapshot made every start fail with a revision
        // conflict as soon as the two drifted apart.
        match self.supervisor.core().account_revision(account_id.get()) {
            Ok(current) if current == expected_revision => {}
            Ok(_) => {
                return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed);
            }
            Err(_) => {
                return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed);
            }
        }
        let Ok(profile) = self.supervisor.core().inspect_profile(&profile_id) else {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed);
        };
        let Ok(deadline) = self.fresh_deadline(operation) else {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed);
        };
        // Seeded before the process starts, because the client reads `user_pass` while it builds its
        // login screen: a store written after start is never seen. A failure here rejects this member
        // only, and no process is launched, so a run can never reach the login screen unauthenticated.
        if self
            .supervisor
            .core()
            .seed_account_login(account_id.get())
            .is_err()
        {
            return ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::LoginSeedFailed);
        }
        match self
            .supervisor
            .start_session(&profile_id, profile.revision, deadline)
        {
            Ok(_) => ManagerRunScheduleOutcome::Scheduled,
            // Readiness and input are owned by the Task 9 coordinator; a start failure stops here and
            // affects only this member.
            Err(_) => ManagerRunScheduleOutcome::Rejected(ManagerRunRejection::StartFailed),
        }
    }

    /// Stops the live session owned by one account.
    ///
    /// The supervisor bumps the session epoch before terminating, so an in-flight readiness task
    /// observes a stale epoch and abandons its work instead of racing the teardown.
    fn stop_account(&mut self, account_id: ManagerAccountId) -> ManagerResult<ManagerAccountView> {
        let operation = ManagerOperation::StopAccount;
        self.ensure_available(operation)?;
        let session_id = self.account_session_id(account_id, operation)?;
        let deadline = self.fresh_deadline(operation)?;
        self.supervisor
            .stop_session(session_id, deadline)
            .map_err(|error| self.map_session_failure(error, operation))?;
        // The seeded credential outlives the process otherwise. Cleared after the stop confirms, and
        // deliberately not propagated: the session is already gone, so a stale store must not turn a
        // successful stop into a failure. The next run overwrites it regardless.
        let _ = self.supervisor.core().clear_account_login(account_id.get());
        // Same reasoning for the character snapshot: a leftover reading would keep the panel showing a
        // live character for a session that has exited.
        let _ = self
            .supervisor
            .core()
            .clear_account_player_snapshot(account_id.get());
        // The captured spot goes with them. A spot armed for one session must not silently steer the
        // next one; the operator re-arms deliberately.
        let _ = self
            .supervisor
            .core()
            .clear_account_control_settings(account_id.get());
        // The reconciled row, not the process exit: what the operator does next is decide whether this
        // account may run again, and the exit code is not something the UI may see anyway.
        self.account_view(account_id, operation)
    }

    /// Reads the character snapshot the running client published for one account.
    ///
    /// `Ok(None)` means nothing has been published yet, which is the ordinary state until a character
    /// is entered. The snapshot is read on demand rather than pushed, because the client rewrites it
    /// about once a second and only the row the operator is looking at needs to be current.
    ///
    /// It is deliberately not gated on a live session: a snapshot that outlives its process is still
    /// the last thing that was true, and it carries its own staleness flag for the UI to render.
    fn observe_account_player(
        &self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<Option<PlayerSnapshot>> {
        let operation = ManagerOperation::ObserveAccountPlayer;
        self.ensure_available(operation)?;
        self.supervisor
            .core()
            .account_player_snapshot(account_id.get())
            .map_err(|error| ManagerError::from_core(error, operation))
    }

    /// Writes the attack and item settings one account's client reads while it runs.
    ///
    /// Applied whether or not a session is live: the client polls the file, so changing a setting
    /// mid-session takes effect without a restart, and setting one before a run arms it for the start.
    /// The reply is the settings as they were written, clamped, so the UI shows what the mod will
    /// actually see rather than what the operator typed.
    fn set_account_control(
        &self,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    ) -> ManagerResult<ControlSettings> {
        let operation = ManagerOperation::SetAccountControl;
        self.ensure_available(operation)?;
        let settings = settings.clamped();
        self.supervisor
            .core()
            .set_account_control_settings(account_id.get(), &settings)
            .map_err(|error| ManagerError::from_core(error, operation))?;
        Ok(settings)
    }

    /// Reads back one account's settings, or the defaults when none are configured yet.
    fn account_control(&self, account_id: ManagerAccountId) -> ManagerResult<ControlSettings> {
        let operation = ManagerOperation::SetAccountControl;
        self.ensure_available(operation)?;
        self.supervisor
            .core()
            .account_control_settings(account_id.get())
            .map(Option::unwrap_or_default)
            .map_err(|error| ManagerError::from_core(error, operation))
    }

    /// Reads every saved monster spot, keyed by map.
    ///
    /// Not per account: a spot belongs to the world, and two accounts farming the same map want the
    /// same coordinates rather than two copies that can disagree.
    fn saved_spot_book(&self) -> ManagerResult<SpotBook> {
        let operation = ManagerOperation::ObserveSpots;
        self.ensure_available(operation)?;
        self.supervisor
            .core()
            .saved_spots()
            .map_err(|error| ManagerError::from_core(error, operation))
    }

    /// Saves one map's spot, replacing whatever that map held.
    fn store_spot(&self, spot: AttackSpot, name: &str) -> ManagerResult<SpotBook> {
        let operation = ManagerOperation::SaveSpot;
        self.ensure_available(operation)?;
        self.supervisor
            .core()
            .save_spot(spot, name)
            .map_err(|error| ManagerError::from_core(error, operation))
    }

    /// Forgets one map's spot, leaving that map empty again.
    fn forget_spot(&self, map_id: u16, name: &str) -> ManagerResult<SpotBook> {
        let operation = ManagerOperation::ClearSpot;
        self.ensure_available(operation)?;
        self.supervisor
            .core()
            .clear_spot(map_id, name)
            .map_err(|error| ManagerError::from_core(error, operation))
    }

    /// Retries cleanup for one account's session left in `CleanupPending`.
    fn retry_account_cleanup(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        let operation = ManagerOperation::RetryAccountCleanup;
        self.ensure_available(operation)?;
        let session_id = self.account_session_id(account_id, operation)?;
        let deadline = self.fresh_deadline(operation)?;
        self.supervisor
            .retry_cleanup(session_id, deadline)
            .map_err(|error| self.map_session_failure(error, operation))?;
        // Same reason as a stop: confirmed cleanup is what releases the account to run again, so the
        // reconciled row is the answer rather than a bare acknowledgement.
        self.account_view(account_id, operation)
    }

    fn stop_session(&mut self, session_id: &str) -> ManagerResult<ManagerSessionExit> {
        self.ensure_available(ManagerOperation::StopSession)?;
        let session_id = parse_session_id(session_id, ManagerOperation::StopSession)?;
        let deadline = self.fresh_deadline(ManagerOperation::StopSession)?;
        self.supervisor
            .stop_session(session_id, deadline)
            .map(|exited| session_exit_view(&exited))
            .map_err(|error| self.map_session_failure(error, ManagerOperation::StopSession))
    }

    fn retry_cleanup(&mut self, session_id: &str) -> ManagerResult<()> {
        self.ensure_available(ManagerOperation::RetryCleanup)?;
        let session_id = parse_session_id(session_id, ManagerOperation::RetryCleanup)?;
        let deadline = self.fresh_deadline(ManagerOperation::RetryCleanup)?;
        self.supervisor
            .retry_cleanup(session_id, deadline)
            .map_err(|error| self.map_session_failure(error, ManagerOperation::RetryCleanup))
    }

    fn close(&mut self) -> ManagerResult<()> {
        if self.state == ControllerState::Closed {
            return Ok(());
        }
        self.state = ControllerState::Closing;

        for session in self.list_sessions() {
            match session.state {
                ManagerSessionState::Running => {
                    let _ = self.stop_session(&session.session_id);
                }
                ManagerSessionState::CleanupPending => {
                    let _ = self.retry_cleanup(&session.session_id);
                }
            }
        }

        let remaining = self.list_sessions();
        if remaining.is_empty() {
            self.state = ControllerState::Closed;
            Ok(())
        } else {
            Err(
                ManagerError::new(ManagerErrorCode::CloseIncomplete, ManagerOperation::Close)
                    .with_remaining_sessions(remaining),
            )
        }
    }

    fn fresh_deadline(&mut self, operation: ManagerOperation) -> ManagerResult<Instant> {
        self.deadlines
            .now()
            .checked_add(MANAGER_OPERATION_TIMEOUT)
            .ok_or_else(|| ManagerError::new(ManagerErrorCode::InternalInvariant, operation))
    }

    fn map_start_failure(&self, error: StartSessionFailure<B::Error>) -> ManagerError {
        match error {
            StartSessionFailure::Core(error) => {
                ManagerError::from_core(error, ManagerOperation::StartProfile)
            }
            StartSessionFailure::ProfileAlreadyActive {
                profile_id,
                session_id,
            } => ManagerError::new(
                ManagerErrorCode::ProfileAlreadyActive,
                ManagerOperation::StartProfile,
            )
            .with_profile_id(profile_id.to_string())
            .with_session_id(session_id.to_string()),
            StartSessionFailure::CapacityReached { maximum } => match u32::try_from(maximum) {
                Ok(maximum) => ManagerError::new(
                    ManagerErrorCode::CapacityReached,
                    ManagerOperation::StartProfile,
                )
                .with_maximum(maximum),
                Err(_) => ManagerError::new(
                    ManagerErrorCode::InternalInvariant,
                    ManagerOperation::StartProfile,
                ),
            },
            StartSessionFailure::SessionIdCollision { session_id } => ManagerError::new(
                ManagerErrorCode::SessionCollision,
                ManagerOperation::StartProfile,
            )
            .with_session_id(session_id.to_string()),
            StartSessionFailure::SpawnRejected { session_id, error } => {
                let code = if error.deadline_expired() {
                    ManagerErrorCode::StartDeadlineExpired
                } else {
                    ManagerErrorCode::StartRejected
                };
                ManagerError::new(code, ManagerOperation::StartProfile)
                    .with_session_id(session_id.to_string())
            }
            StartSessionFailure::CleanupPending {
                session_id,
                error: _,
            } => {
                let session_id = session_id.to_string();
                match self
                    .list_sessions()
                    .into_iter()
                    .find(|session| session.session_id == session_id)
                {
                    Some(retained) => ManagerError::new(
                        ManagerErrorCode::CleanupPending,
                        ManagerOperation::StartProfile,
                    )
                    .with_session_id(session_id)
                    .with_retained_session(retained),
                    None => ManagerError::new(
                        ManagerErrorCode::InternalInvariant,
                        ManagerOperation::StartProfile,
                    )
                    .with_session_id(session_id),
                }
            }
        }
    }

    fn map_session_failure(
        &self,
        error: SessionFailure<B::Error>,
        operation: ManagerOperation,
    ) -> ManagerError {
        match error {
            SessionFailure::SessionNotFound { session_id } => {
                ManagerError::new(ManagerErrorCode::SessionNotFound, operation)
                    .with_session_id(session_id.to_string())
            }
            SessionFailure::WrongState { session_id, .. } => {
                ManagerError::new(ManagerErrorCode::WrongSessionState, operation)
                    .with_session_id(session_id.to_string())
            }
            SessionFailure::Process { session_id, error } => {
                let code = if error.deadline_expired() {
                    ManagerErrorCode::ProcessDeadlineExpired
                } else {
                    ManagerErrorCode::ProcessFailure
                };
                ManagerError::new(code, operation).with_session_id(session_id.to_string())
            }
            SessionFailure::InvariantViolation { .. } => {
                ManagerError::new(ManagerErrorCode::InternalInvariant, operation)
            }
        }
    }
}

/// Single-owner manager control surface over the private Windows Supervisor.
///
/// The private `PhantomData<Cell<()>>` keeps the type `Send` and explicitly
/// `!Sync` regardless of future Core field changes.
#[non_exhaustive]
pub struct ManagerController {
    inner: ManagerCore<WindowsProcessBackend, SystemDeadlineSource>,
    _not_sync: PhantomData<Cell<()>>,
}

impl ManagerController {
    pub fn open_at(path: &Path) -> ManagerResult<Self> {
        CoreState::open_at(path)
            .map(Self::from_core)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::Open))
    }

    /// Opens a data root that may have been copied or moved, relocating the pinned runtime.
    pub(crate) fn open_portable_at(
        data_root: &Path,
        exact_runtime_root: &Path,
    ) -> ManagerResult<Self> {
        CoreState::open_portable_at(data_root, exact_runtime_root)
            .map(Self::from_core)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::Open))
    }

    pub fn open_default() -> ManagerResult<Self> {
        CoreState::open_default()
            .map(Self::from_core)
            .map_err(|error| ManagerError::from_core(error, ManagerOperation::Open))
    }

    pub fn from_core(core: CoreState) -> Self {
        Self {
            inner: ManagerCore::new(core, WindowsProcessBackend, SystemDeadlineSource),
            _not_sync: PhantomData,
        }
    }

    #[cfg(all(test, windows))]
    pub(super) fn running_birth_id_for_test(&self, session_id: &str) -> Option<ProcessBirthId> {
        let session_id = parse_session_id(session_id, ManagerOperation::ObserveSession).ok()?;
        self.inner.supervisor.running_birth_id_for_test(session_id)
    }

    pub fn list_profiles(
        &self,
        after_profile_id: Option<&str>,
        limit: u32,
        include_archived: bool,
    ) -> ManagerResult<ManagerProfilePage> {
        self.inner
            .list_profiles(after_profile_id, limit, include_archived)
    }

    pub fn list_runtimes(
        &self,
        after_runtime_id: Option<&str>,
        limit: u32,
    ) -> ManagerResult<ManagerRuntimePage> {
        self.inner.list_runtimes(after_runtime_id, limit)
    }

    pub fn list_sessions(&self) -> Vec<ManagerSessionView> {
        self.inner.list_sessions()
    }

    pub fn start_profile(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
    ) -> ManagerResult<ManagerSessionView> {
        self.inner.start_profile(profile_id, expected_revision)
    }

    pub fn observe_session(&mut self, session_id: &str) -> ManagerResult<ManagerObservation> {
        self.inner.observe_session(session_id)
    }

    pub fn stop_session(&mut self, session_id: &str) -> ManagerResult<ManagerSessionExit> {
        self.inner.stop_session(session_id)
    }

    pub fn retry_cleanup(&mut self, session_id: &str) -> ManagerResult<()> {
        self.inner.retry_cleanup(session_id)
    }

    /// Crate-private account operations. The worker thread owns the only callers, so the public
    /// ManagerController surface gains no generic mutable-Core access.
    pub(crate) fn list_accounts(&self) -> ManagerResult<Vec<ManagerAccountView>> {
        self.inner.list_accounts()
    }

    pub(crate) fn import_account(
        &mut self,
        username: &str,
        password: ManagerAccountPassword,
    ) -> ManagerResult<ManagerAccountView> {
        self.inner.import_account(username, password)
    }

    pub(crate) fn update_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
        username: &str,
        replacement_password: Option<ManagerAccountPassword>,
    ) -> ManagerResult<ManagerAccountView> {
        self.inner.update_account(
            account_id,
            expected_revision,
            username,
            replacement_password,
        )
    }

    pub(crate) fn delete_account(
        &mut self,
        account_id: ManagerAccountId,
        expected_revision: i64,
    ) -> ManagerResult<()> {
        self.inner.delete_account(account_id, expected_revision)
    }

    pub(crate) fn set_account_server(
        &mut self,
        account_id: ManagerAccountId,
        server_index: u8,
    ) -> ManagerResult<ManagerAccountView> {
        self.inner.set_account_server(account_id, server_index)
    }

    pub(crate) fn run_accounts(
        &mut self,
        requests: &[(ManagerAccountId, i64)],
    ) -> ManagerResult<Vec<ManagerRunSchedule>> {
        self.inner.run_accounts(requests)
    }

    pub(crate) fn apply_readiness_completion(
        &mut self,
        account_id: ManagerAccountId,
        session_key: Uuid,
        epoch: u64,
        succeeded: bool,
    ) -> Option<ManagerAccountView> {
        self.inner
            .apply_readiness_completion(account_id, session_key, epoch, succeeded)
    }

    pub(crate) fn stop_account(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        self.inner.stop_account(account_id)
    }

    pub(crate) fn retry_account_cleanup(
        &mut self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ManagerAccountView> {
        self.inner.retry_account_cleanup(account_id)
    }

    /// Reads one account's published character snapshot. Takes `&self`: it mutates no lifecycle state.
    pub(crate) fn observe_account_player(
        &self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<Option<PlayerSnapshot>> {
        self.inner.observe_account_player(account_id)
    }

    /// Writes one account's attack and item settings, answering with the clamped values.
    pub(crate) fn set_account_control(
        &self,
        account_id: ManagerAccountId,
        settings: ControlSettings,
    ) -> ManagerResult<ControlSettings> {
        self.inner.set_account_control(account_id, settings)
    }

    /// Reads one account's settings, or the defaults when none are configured yet.
    pub(crate) fn account_control(
        &self,
        account_id: ManagerAccountId,
    ) -> ManagerResult<ControlSettings> {
        self.inner.account_control(account_id)
    }

    /// Reads every saved monster spot, keyed by map.
    pub(crate) fn saved_spot_book(&self) -> ManagerResult<SpotBook> {
        self.inner.saved_spot_book()
    }

    /// Saves one map's spot, replacing whatever that map held.
    pub(crate) fn store_spot(&self, spot: AttackSpot, name: &str) -> ManagerResult<SpotBook> {
        self.inner.store_spot(spot, name)
    }

    /// Forgets one map's spot, leaving that map empty again.
    pub(crate) fn forget_spot(&self, map_id: u16, name: &str) -> ManagerResult<SpotBook> {
        self.inner.forget_spot(map_id, name)
    }

    pub fn close(&mut self) -> ManagerResult<()> {
        self.inner.close()
    }
}

fn profile_view(record: ProfileRecord, sessions: &[SessionSummary]) -> ManagerProfileView {
    let active_session_id = sessions
        .iter()
        .find(|summary| summary.metadata().profile_id().to_string() == record.profile_id)
        .map(|summary| summary.metadata().session_id().to_string());
    ManagerProfileView {
        profile_id: record.profile_id,
        revision: record.revision,
        display_name: record.display_name,
        runtime_id: record.runtime_id,
        archived: record.archived_at_unix_ms.is_some(),
        active_session_id,
    }
}

/// Maps one account's linked profile to its live session state, if any.
fn account_session(
    account: &StoredAccount,
    sessions: &[SessionSummary],
) -> Option<AccountSessionState> {
    sessions
        .iter()
        .find(|summary| summary.metadata().profile_id().to_string() == account.profile_id)
        .map(|summary| match summary.state() {
            SessionStateKind::Running => AccountSessionState::Running,
            SessionStateKind::CleanupPending => AccountSessionState::CleanupPending,
        })
}

fn runtime_view(record: RuntimeRecord) -> ManagerRuntimeView {
    ManagerRuntimeView {
        runtime_id: record.runtime_id,
        target_os: record.target_os,
        target_arch: record.target_arch,
        java_vendor: record.java_vendor,
        java_version: record.java_version,
        microemulator_version: record.microemulator_version,
        game_bundle: record.game_bundle,
        capability_state: record.capability_state,
        validation_reason: record.validation_reason,
    }
}

fn session_view(summary: SessionSummary) -> ManagerSessionView {
    session_metadata_view(
        summary.metadata(),
        match summary.state() {
            SessionStateKind::Running => ManagerSessionState::Running,
            SessionStateKind::CleanupPending => ManagerSessionState::CleanupPending,
        },
    )
}

fn session_metadata_view(
    metadata: &SessionMetadata,
    state: ManagerSessionState,
) -> ManagerSessionView {
    ManagerSessionView {
        session_id: metadata.session_id().to_string(),
        profile_id: metadata.profile_id().to_string(),
        profile_revision: metadata.profile_revision(),
        runtime_id: metadata.runtime_id().to_owned(),
        state,
    }
}

fn session_exit_view(exited: &SessionExit) -> ManagerSessionExit {
    let metadata = exited.metadata();
    ManagerSessionExit {
        session_id: metadata.session_id().to_string(),
        profile_id: metadata.profile_id().to_string(),
        profile_revision: metadata.profile_revision(),
        runtime_id: metadata.runtime_id().to_owned(),
    }
}

fn parse_session_id(value: &str, operation: ManagerOperation) -> ManagerResult<Uuid> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ManagerError::new(ManagerErrorCode::InvalidSessionId, operation))?;
    if parsed.get_version() != Some(Version::Random) || parsed.hyphenated().to_string() != value {
        return Err(ManagerError::new(
            ManagerErrorCode::InvalidSessionId,
            operation,
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_live_runtime_tests;

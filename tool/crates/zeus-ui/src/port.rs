//! Boundary between the pure UI model and the account engine.
//!
//! The trait is platform-independent, so the model can be driven by a fake on any target. The
//! production adapter is the only place that touches the worker handle, and it lives behind
//! `cfg(windows)`.

use std::num::NonZeroU64;

use crate::model::{RowKey, SubmitResult, UiControl, UiEvent, UiSpot};

/// Commands the UI may submit, and the single drain used by the timer and every dialog procedure.
///
/// Every method is non-blocking: the UI thread performs no database, crypto, process, sleep, or input
/// work.
pub trait AccountPort {
    fn list(&mut self) -> SubmitResult;

    /// Imports one account. The password is a temporary UTF-16 copy the port clears after submission.
    fn import(&mut self, username: &str, password: Vec<u16>) -> SubmitResult;

    fn update(
        &mut self,
        row: RowKey,
        expected: i64,
        username: &str,
        password: Option<Vec<u16>>,
    ) -> SubmitResult;

    fn delete(&mut self, row: RowKey, expected: i64) -> SubmitResult;

    /// Sets which world one account logs into. Carries no username and no secret, so choosing a
    /// server never asks the operator to re-enter a password.
    fn set_server(&mut self, row: RowKey, server_index: u8) -> SubmitResult;

    /// Runs one bounded batch. A single-row Run passes exactly one item through this same path.
    fn run_many(&mut self, rows: &[(RowKey, i64)]) -> SubmitResult;

    fn stop(&mut self, row: RowKey) -> SubmitResult;

    fn retry(&mut self, row: RowKey) -> SubmitResult;

    /// Asks for one row's published character reading.
    ///
    /// Polled rather than pushed: the client rewrites the reading about once a second and only the
    /// focused row is on screen, so pushing every row would spend the bounded event queue on data
    /// nobody is looking at.
    fn observe_player(&mut self, row: RowKey) -> SubmitResult;

    /// Writes one row's automation settings.
    ///
    /// Accepted whether or not the account is running: the client polls the settings file, so a change
    /// takes effect mid-session, and one made before a run arms it for the start. The reply carries the
    /// settings as they were actually written, clamped.
    fn set_control(&mut self, row: RowKey, settings: UiControl) -> SubmitResult;

    /// Reads one row's automation settings back, so the config dialog opens on the current values.
    fn observe_control(&mut self, row: RowKey) -> SubmitResult;

    /// Reads every saved monster spot, keyed by map.
    ///
    /// Takes no row: the book is shared by every account, because a monster spot belongs to the world
    /// rather than to a login.
    fn observe_spots(&mut self) -> SubmitResult;

    /// Saves one named spot, replacing any spot of the same name on the same map.
    fn save_spot(&mut self, spot: UiSpot, name: String) -> SubmitResult;

    /// Forgets one named spot.
    fn clear_spot(&mut self, map_id: u16, name: String) -> SubmitResult;

    /// Submits the single shutdown command. The close sequence waits for its consumed result.
    fn shutdown(&mut self) -> SubmitResult;

    /// Drains at most [`crate::model::MAX_EVENTS_PER_POLL`] events, leaving the rest for the next tick.
    fn poll(&mut self) -> Vec<UiEvent>;
}

/// Allocates local row keys, never reusing a value within one session.
#[derive(Debug)]
pub struct RowKeyAllocator {
    next: NonZeroU64,
}

impl RowKeyAllocator {
    pub fn new() -> Self {
        Self {
            next: NonZeroU64::new(1).expect("1 is nonzero"),
        }
    }

    /// Issues the next key, or `None` once the session has exhausted the range.
    pub fn allocate(&mut self) -> Option<RowKey> {
        let key = RowKey::new(self.next);
        self.next = self.next.checked_add(1)?;
        Some(key)
    }
}

impl Default for RowKeyAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
pub use windows_adapter::WorkerAccountPort;

#[cfg(windows)]
mod windows_adapter {
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use zeus_core::{
        AttackMode, AttackSpot, ControlSettings, GoldPickup, ItemRank, MAX_RUN_BATCH,
        ManagerAccountId, ManagerAccountPassword, ManagerAccountStatus, ManagerAccountView,
        ManagerErrorCode, ManagerRunRejection, ManagerRunScheduleOutcome, ManagerWorker,
        ManagerWorkerErrorCode, ManagerWorkerEvent, ManagerWorkerOperation, PlayerSnapshot,
        PotionPickup, ReviveMode, SpotBook, ZoneMode,
    };

    use super::{AccountPort, RowKeyAllocator};
    use crate::model::{
        MAX_EVENTS_PER_POLL, RowKey, SubmitResult, UiAccountRow, UiAccountStatus, UiAutoMode,
        UiBatchRunItem, UiBootFailure, UiControl, UiErrorCode, UiEvent, UiPickup, UiPlayerInfo,
        UiRequestId, UiSavedSpot, UiSpot, UiSpotBook, UiSuccess,
    };

    /// Production adapter over the worker handle.
    ///
    /// It keeps private bidirectional maps between the local `RowKey` and the hidden account
    /// identity. Neither map is ever rendered, so the account identity cannot reach the screen.
    pub struct WorkerAccountPort {
        worker: ManagerWorker,
        keys: RowKeyAllocator,
        by_account: HashMap<ManagerAccountId, RowKey>,
        by_row: HashMap<RowKey, ManagerAccountId>,
        pending: HashMap<u64, PendingOperation>,
    }

    /// What a submitted request will produce, so its result maps to the right payload.
    #[derive(Clone, Debug)]
    enum PendingOperation {
        List,
        Import,
        Update,
        Delete(RowKey),
        Run,
        Stop,
        RetryCleanup,
        /// The row whose reading was requested, so a result can be attributed without guessing.
        ObservePlayer(RowKey),
        /// A settings read or write. The row travels with the request because the reply carries only
        /// values: the settings themselves name no account, which is exactly why they may be shown.
        Control(RowKey),
        /// A spot read or write. Carries no row: the book is shared by every account.
        Spots,
        Shutdown,
    }

    impl WorkerAccountPort {
        pub fn new(worker: ManagerWorker) -> Self {
            Self {
                worker,
                keys: RowKeyAllocator::new(),
                by_account: HashMap::new(),
                by_row: HashMap::new(),
                pending: HashMap::new(),
            }
        }

        /// Resolves, or assigns, the local key for one hidden account identity.
        fn row_for(&mut self, account_id: ManagerAccountId) -> Option<RowKey> {
            if let Some(key) = self.by_account.get(&account_id) {
                return Some(*key);
            }
            let key = self.keys.allocate()?;
            self.by_account.insert(account_id, key);
            self.by_row.insert(key, account_id);
            Some(key)
        }

        fn account_for(&self, row: RowKey) -> Option<ManagerAccountId> {
            self.by_row.get(&row).copied()
        }

        fn submit<F>(&mut self, operation: PendingOperation, call: F) -> SubmitResult
        where
            F: FnOnce(
                &mut ManagerWorker,
            )
                -> Result<zeus_core::ManagerRequestId, zeus_core::ManagerWorkerError>,
        {
            match call(&mut self.worker) {
                Ok(request_id) => {
                    let id: UiRequestId = request_id.get();
                    self.pending.insert(id, operation);
                    SubmitResult::Accepted(id)
                }
                Err(error) => SubmitResult::Rejected(map_worker_error(error.code())),
            }
        }

        fn view_to_row(&mut self, view: &ManagerAccountView) -> Option<UiAccountRow> {
            let key = self.row_for(view.account_id)?;
            Some(UiAccountRow {
                key,
                revision: view.revision,
                username: view.username.clone(),
                status: map_status(view.status),
                last_run_at_unix_ms: view.last_run_at_unix_ms,
                server_index: view.server_index,
            })
        }
    }

    impl AccountPort for WorkerAccountPort {
        fn list(&mut self) -> SubmitResult {
            self.submit(PendingOperation::List, |worker| worker.try_list_accounts())
        }

        fn import(&mut self, username: &str, password: Vec<u16>) -> SubmitResult {
            // The temporary secret copy is built here and consumed by the worker; the dialog keeps its
            // own native buffer until this submission is accepted.
            let secret = match ManagerAccountPassword::try_from_utf16(
                ManagerWorkerOperation::ImportAccount,
                password,
            ) {
                Ok(secret) => secret,
                Err(error) => return SubmitResult::Rejected(map_worker_error(error.code())),
            };
            let username = username.to_owned();
            self.submit(PendingOperation::Import, move |worker| {
                worker.try_import_account(&username, secret)
            })
        }

        fn update(
            &mut self,
            row: RowKey,
            expected: i64,
            username: &str,
            password: Option<Vec<u16>>,
        ) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            let replacement = match password {
                Some(units) => match ManagerAccountPassword::try_from_utf16(
                    ManagerWorkerOperation::UpdateAccount,
                    units,
                ) {
                    Ok(secret) => Some(secret),
                    Err(error) => return SubmitResult::Rejected(map_worker_error(error.code())),
                },
                None => None,
            };
            let username = username.to_owned();
            self.submit(PendingOperation::Update, move |worker| {
                worker.try_update_account(account_id, expected, &username, replacement)
            })
        }

        fn delete(&mut self, row: RowKey, expected: i64) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            self.submit(PendingOperation::Delete(row), move |worker| {
                worker.try_delete_account(account_id, expected)
            })
        }

        fn set_server(&mut self, row: RowKey, server_index: u8) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            // Reported as an ordinary update, because the worker answers with the same event.
            self.submit(PendingOperation::Update, move |worker| {
                worker.try_set_account_server(account_id, server_index)
            })
        }

        fn run_many(&mut self, rows: &[(RowKey, i64)]) -> SubmitResult {
            if rows.is_empty() || rows.len() > MAX_RUN_BATCH {
                return SubmitResult::Rejected(UiErrorCode::LoginCapacity);
            }
            let mut requests = Vec::with_capacity(rows.len());
            for (row, expected) in rows {
                let Some(account_id) = self.account_for(*row) else {
                    return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
                };
                requests.push((account_id, *expected));
            }
            self.submit(PendingOperation::Run, move |worker| {
                worker.try_run_accounts(&requests)
            })
        }

        fn stop(&mut self, row: RowKey) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            self.submit(PendingOperation::Stop, move |worker| {
                worker.try_stop_account(account_id)
            })
        }

        fn retry(&mut self, row: RowKey) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            self.submit(PendingOperation::RetryCleanup, move |worker| {
                worker.try_retry_account_cleanup(account_id)
            })
        }

        fn observe_player(&mut self, row: RowKey) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            self.submit(PendingOperation::ObservePlayer(row), move |worker| {
                worker.try_observe_account_player(account_id)
            })
        }

        fn set_control(&mut self, row: RowKey, settings: UiControl) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            let settings = to_core_settings(settings);
            self.submit(PendingOperation::Control(row), move |worker| {
                worker.try_set_account_control(account_id, settings)
            })
        }

        fn observe_spots(&mut self) -> SubmitResult {
            self.submit(PendingOperation::Spots, |worker| worker.try_observe_spots())
        }

        fn save_spot(&mut self, spot: UiSpot, name: String) -> SubmitResult {
            let spot = to_core_spot(spot);
            self.submit(PendingOperation::Spots, move |worker| {
                worker.try_save_spot(spot, name)
            })
        }

        fn clear_spot(&mut self, map_id: u16, name: String) -> SubmitResult {
            self.submit(PendingOperation::Spots, move |worker| {
                worker.try_clear_spot(map_id, name)
            })
        }

        fn observe_control(&mut self, row: RowKey) -> SubmitResult {
            let Some(account_id) = self.account_for(row) else {
                return SubmitResult::Rejected(UiErrorCode::AccountNotFound);
            };
            self.submit(PendingOperation::Control(row), move |worker| {
                worker.try_observe_account_control(account_id)
            })
        }

        fn shutdown(&mut self) -> SubmitResult {
            self.submit(PendingOperation::Shutdown, |worker| worker.try_shutdown())
        }

        fn poll(&mut self) -> Vec<UiEvent> {
            let mut events = Vec::new();
            while events.len() < MAX_EVENTS_PER_POLL {
                match self.worker.try_next_event() {
                    Ok(Some(event)) => {
                        if let Some(translated) = self.translate(event) {
                            events.push(translated);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        events.push(UiEvent::WorkerClosed);
                        break;
                    }
                }
            }
            events
        }
    }

    impl WorkerAccountPort {
        /// Translates one worker event, dropping anything the UI does not model.
        fn translate(&mut self, event: ManagerWorkerEvent) -> Option<UiEvent> {
            match event {
                ManagerWorkerEvent::AccountsListed { request_id, result } => {
                    let request = request_id.get();
                    self.pending.remove(&request);
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(views) => {
                                let mut rows = Vec::with_capacity(views.len());
                                for view in &views {
                                    if let Some(row) = self.view_to_row(view) {
                                        rows.push(row);
                                    }
                                }
                                Ok(UiSuccess::Accounts(rows))
                            }
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountImported { request_id, result }
                | ManagerWorkerEvent::AccountUpdated { request_id, result } => {
                    let request = request_id.get();
                    self.pending.remove(&request);
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(view) => self
                                .view_to_row(&view)
                                .map(UiSuccess::Account)
                                .ok_or(UiErrorCode::InvalidInput),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountDeleted { request_id, result } => {
                    let request = request_id.get();
                    let deleted = match self.pending.remove(&request) {
                        Some(PendingOperation::Delete(row)) => Some(row),
                        _ => None,
                    };
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match (result, deleted) {
                            (Ok(()), Some(row)) => {
                                if let Some(account_id) = self.by_row.remove(&row) {
                                    self.by_account.remove(&account_id);
                                }
                                Ok(UiSuccess::Deleted(row))
                            }
                            (Ok(()), None) => Err(UiErrorCode::AccountNotFound),
                            (Err(error), _) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountsRunScheduled { request_id, result } => {
                    let request = request_id.get();
                    self.pending.remove(&request);
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(schedules) => {
                                let mut items = Vec::with_capacity(schedules.len());
                                for schedule in &schedules {
                                    let Some(row) = self.row_for(schedule.account_id) else {
                                        continue;
                                    };
                                    items.push(UiBatchRunItem {
                                        row,
                                        scheduled: match schedule.outcome {
                                            ManagerRunScheduleOutcome::Scheduled => Ok(()),
                                            ManagerRunScheduleOutcome::Rejected(rejection) => {
                                                Err(map_rejection(rejection))
                                            }
                                            // A future outcome must not silently read as scheduled.
                                            _ => Err(UiErrorCode::LoginTarget),
                                        },
                                    });
                                }
                                Ok(UiSuccess::RunBatch(items))
                            }
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountStopResult { request_id, result }
                | ManagerWorkerEvent::AccountCleanupRetried { request_id, result } => {
                    let request = request_id.get();
                    self.pending.remove(&request);
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        // The reconciled row, so a confirmed stop clears the optimistic `Stopping`
                        // status on this reply instead of waiting for a notification that a stopped
                        // session no longer produces. It must NOT report Shutdown: that would mark the
                        // whole worker closed and make the UI treat one stopped account as a dead
                        // engine.
                        result: match result {
                            Ok(view) => self
                                .view_to_row(&view)
                                .map(UiSuccess::Account)
                                .ok_or(UiErrorCode::InvalidInput),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountPlayerObserved { request_id, result } => {
                    let request = request_id.get();
                    // The row travels with the request rather than the reply: the reading itself
                    // carries no account identity, which is exactly why it may be rendered.
                    let row = match self.pending.remove(&request) {
                        Some(PendingOperation::ObservePlayer(row)) => Some(row),
                        _ => None,
                    };
                    let row = row?;
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(snapshot) => Ok(UiSuccess::Player {
                                row,
                                info: snapshot.as_ref().map(player_info),
                            }),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountControlApplied { request_id, result } => {
                    let request = request_id.get();
                    // The row travels with the request, as it does for a reading: the settings carry
                    // no account identity of their own.
                    let row = match self.pending.remove(&request) {
                        Some(PendingOperation::Control(row)) => Some(row),
                        _ => None,
                    };
                    let row = row?;
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        // The reply is the settings as written, clamped, so the dialog can open on
                        // what the client will really read rather than on what was typed at it.
                        result: match result {
                            Ok(settings) => Ok(UiSuccess::Control {
                                row,
                                settings: from_core_settings(&settings),
                            }),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::SpotsApplied { request_id, result } => {
                    let request = request_id.get();
                    // No row: the book belongs to the world, not to an account, so there is nothing to
                    // attribute it to.
                    if !matches!(self.pending.remove(&request), Some(PendingOperation::Spots)) {
                        return None;
                    }
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(book) => Ok(UiSuccess::Spots(from_core_spots(&book))),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::ShutdownResult { request_id, result } => {
                    let request = request_id.get();
                    self.pending.remove(&request);
                    Some(UiEvent::RequestFinished {
                        request_id: request,
                        result: match result {
                            Ok(()) => Ok(UiSuccess::Shutdown),
                            Err(error) => Err(map_core_error(error.code())),
                        },
                    })
                }
                ManagerWorkerEvent::AccountStateChanged(view) => {
                    self.view_to_row(&view).map(UiEvent::StateChanged)
                }
                ManagerWorkerEvent::Ready => Some(UiEvent::WorkerReady),
                ManagerWorkerEvent::OpenFailed(error) => {
                    Some(UiEvent::BootFailed(map_boot_failure(error.code())))
                }
                // Profile, runtime, and session events belong to the M2 surface the account UI does
                // not render.
                _ => None,
            }
        }
    }

    /// Translates one engine snapshot into the pure UI reading.
    ///
    /// The age is stamped here, in the impure adapter, because the model layer owns no clock. An
    /// unreadable clock, or a reading stamped in the future, yields `None` rather than a fabricated
    /// zero: "unknown age" and "written this instant" must not look the same.
    fn player_info(snapshot: &PlayerSnapshot) -> UiPlayerInfo {
        UiPlayerInfo {
            character_name: snapshot.character_name.clone(),
            level: snapshot.level,
            xp_permille: snapshot.xp_permille,
            xp_permille_per_hour: snapshot.xp_permille_per_hour,
            hp: snapshot.hp,
            hp_max: snapshot.hp_max,
            mp: snapshot.mp,
            mp_max: snapshot.mp_max,
            wallet_known: snapshot.wallet_known,
            gold: snapshot.gold,
            gem: snapshot.gem,
            map_id: snapshot.map_id,
            zone: snapshot.zone,
            pixel_x: snapshot.pixel_x,
            pixel_y: snapshot.pixel_y,
            quota: snapshot.quota,
            bag_used: snapshot.bag_used,
            bag_max: snapshot.bag_max,
            dead: snapshot.is_dead(),
            // 2 is the client's own fighting state; every other value is ordinary.
            fighting: snapshot.state == 2,
            mounted: snapshot.mount.is_some(),
            mounts: snapshot.mounts.clone(),
            guild_name: snapshot.guild_name.clone(),
            stale: snapshot.stale,
            age_seconds: reading_age_seconds(snapshot.written_at_unix_ms),
            settings_agreed: snapshot.settings_agreed,
            auto_state: snapshot.attack_state,
            has_target: snapshot.has_target,
            stuck: snapshot.stuck,
            pickup: pickup_labels(snapshot.pickup),
            buffs: snapshot.buffs,
            materials: snapshot.materials,
            travel_state: snapshot.travel_state,
            travel_why: snapshot.travel_why,
            travel_goal: snapshot.travel_goal,
            travel_hops: snapshot.travel_hops,
            potions: snapshot.potions,
            revives: snapshot.revives,
            // ---- ENHANCE ----
            enhance_phase: snapshot.enhance_phase,
            enhance_why: snapshot.enhance_why,
            enhance_done: snapshot.enhance_done,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_state: snapshot.dungeon_state,
            dungeon_why: snapshot.dungeon_why,
            dungeon_runs: snapshot.dungeon_runs,
            dungeon_goal: snapshot.dungeon_goal,
            // ---- end DUNGEON ----
        }
    }

    /// Turns the operator's settings into the engine's own shape.
    ///
    /// Every picker is an index that is also the engine's wire value, so a selection the engine would
    /// refuse cannot be constructed here. `from_wire` still guards each one: a future option added on
    /// one side of the boundary must fall back to the engine's default rather than to whatever enum
    /// variant happens to be first.
    fn to_core_settings(settings: UiControl) -> ControlSettings {
        let settings = settings.clamped();
        ControlSettings {
            mode: match settings.mode {
                UiAutoMode::Off => AttackMode::Off,
                UiAutoMode::Stand => AttackMode::Stand,
                UiAutoMode::Move => AttackMode::Move,
            },
            spot: settings.spot.map(|spot| AttackSpot {
                map_id: spot.map_id,
                zone: spot.zone,
                pixel_x: spot.pixel_x,
                pixel_y: spot.pixel_y,
            }),
            radius: settings.radius,
            hp_on: settings.hp_on,
            hp_percent: settings.hp_percent,
            mp_on: settings.mp_on,
            mp_percent: settings.mp_percent,
            revive_on: settings.revive_on,
            // The dialog stores a picker index; the engine's wire values start at 1, so the two are
            // no longer the same number and the index has to be looked up rather than cast.
            revive: ReviveMode::ALL
                .get(settings.revive as usize)
                .copied()
                .unwrap_or_default(),
            revive_delay_seconds: settings.revive_delay_seconds,
            buffs: settings.buffs,
            item_rank: ItemRank::from_wire(settings.item_rank).unwrap_or_default(),
            potion_pickup: PotionPickup::from_wire(settings.potion_pickup).unwrap_or_default(),
            gold: GoldPickup::from_wire(settings.gold).unwrap_or_default(),
            mount: settings.mount,
            mount_template_id: settings.mount_template_id,
            medal_dialog: settings.medal_dialog,
            zone_mode: ZoneMode::from_wire(settings.zone_mode).unwrap_or_default(),
            zone_pick: settings.zone_pick,
            materials_managed: settings.materials_managed,
            materials: settings.materials,
            // Index 0 is the operator's "off"; every other index names a map through the id table,
            // and an index the table does not cover reads as off rather than as some other map.
            ring: settings.ring,
            farm_on_arrival: settings.farm_on_arrival,
            detect_spots: settings.detect_spots,
            nav_target: crate::model::NAV_TARGET_IDS
                .get(settings.nav_target as usize)
                .copied()
                .filter(|_| settings.nav_target > 0),
            // ---- ENHANCE ----
            enhance_on: settings.enhance_on,
            enhance_max_level: settings.enhance_max_level,
            enhance_charm_type: settings.enhance_charm,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_on: settings.dungeon_on,
            // Both pickers store an index and the engine stores a value, and the two are not the same
            // number: each picker's first option is a sentinel (-1) rather than a count, so the table
            // translates. `settings` is already `clamped()`, so the index cannot be past the table.
            dungeon_max: crate::model::DUNGEON_RUN_VALUES
                .get(settings.dungeon_max as usize)
                .copied()
                .unwrap_or(-1),
            dungeon_schedule: crate::model::DUNGEON_SCHEDULE_VALUES
                .get(settings.dungeon_schedule as usize)
                .copied()
                .unwrap_or(-1),
            // ---- end DUNGEON ----
            // ---- QOL --------------------------------------------------------------
            effects: 1,
            hide_players: 0,
            // ---- end QOL ----------------------------------------------------------
        }
    }

    /// Turns the engine's confirmed settings back into the operator's shape.
    /// The engine's shape for one spot the operator saved.
    fn to_core_spot(spot: UiSpot) -> AttackSpot {
        AttackSpot {
            map_id: spot.map_id,
            zone: spot.zone,
            pixel_x: spot.pixel_x,
            pixel_y: spot.pixel_y,
        }
    }

    /// The whole saved book, in the order the engine returned it.
    fn from_core_spots(book: &SpotBook) -> UiSpotBook {
        UiSpotBook {
            entries: book
                .spots()
                .iter()
                .map(|saved| UiSavedSpot {
                    name: saved.name.clone(),
                    spot: UiSpot {
                        map_id: saved.spot.map_id,
                        zone: saved.spot.zone,
                        pixel_x: saved.spot.pixel_x,
                        pixel_y: saved.spot.pixel_y,
                    },
                })
                .collect(),
        }
    }

    fn from_core_settings(settings: &ControlSettings) -> UiControl {
        UiControl {
            mode: match settings.mode {
                AttackMode::Stand => UiAutoMode::Stand,
                AttackMode::Move => UiAutoMode::Move,
                // A mode this build does not model must read as off, never as an active one.
                _ => UiAutoMode::Off,
            },
            spot: settings.spot.map(|spot| UiSpot {
                map_id: spot.map_id,
                zone: spot.zone,
                pixel_x: spot.pixel_x,
                pixel_y: spot.pixel_y,
            }),
            radius: settings.radius,
            hp_on: settings.hp_on,
            hp_percent: settings.hp_percent,
            mp_on: settings.mp_on,
            mp_percent: settings.mp_percent,
            revive_on: settings.revive_on,
            revive: ReviveMode::ALL
                .iter()
                .position(|mode| *mode == settings.revive)
                .unwrap_or(0) as u8,
            revive_delay_seconds: settings.revive_delay_seconds,
            buffs: settings.buffs,
            item_rank: settings.item_rank.as_wire(),
            potion_pickup: settings.potion_pickup.as_wire(),
            gold: settings.gold.as_wire(),
            mount: settings.mount,
            mount_template_id: settings.mount_template_id,
            medal_dialog: settings.medal_dialog,
            zone_mode: settings.zone_mode.as_wire(),
            zone_pick: settings.zone_pick,
            materials_managed: settings.materials_managed,
            materials: settings.materials,
            ring: settings.ring,
            farm_on_arrival: settings.farm_on_arrival,
            // Core carries the coordinates, not the label: the name is how the operator refers to a
            // spot, and the book is where names live. The shell resolves it from the book when it opens
            // the dialog, so there is one source of truth for where a spot is.
            spot_name: String::new(),
            // One-shot, and the mod already answered: reopening the dialog must not re-arm it.
            detect_spots: false,
            nav_target: settings
                .nav_target
                .and_then(|map| {
                    crate::model::NAV_TARGET_IDS
                        .iter()
                        .position(|candidate| *candidate == map)
                })
                .unwrap_or(0) as u8,
            // ---- ENHANCE ----
            enhance_on: settings.enhance_on,
            enhance_max_level: settings.enhance_max_level,
            enhance_charm: settings.enhance_charm_type,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_on: settings.dungeon_on,
            // The reverse of `to_core_settings`: find which picker option stands for this value. A
            // value no option names falls back to the sentinel index, the way `nav_target` above does
            // — and it cannot happen from a round trip, since the only values this writes are the ones
            // the tables carry.
            dungeon_max: crate::model::DUNGEON_RUN_VALUES
                .iter()
                .position(|candidate| *candidate == settings.dungeon_max)
                .unwrap_or(0) as u8,
            dungeon_schedule: crate::model::DUNGEON_SCHEDULE_VALUES
                .iter()
                .position(|candidate| *candidate == settings.dungeon_schedule)
                .unwrap_or(0) as u8,
            // ---- end DUNGEON ----
        }
    }

    /// Turns the three bytes read back out of the client into the client's own words.
    ///
    /// `None` for a byte the client's own tables do not name, rather than a guess: the read-back
    /// exists to say what took effect, and a mislabelled value would be worse than no label.
    fn pickup_labels(record: Option<(i8, i8, i8)>) -> Option<UiPickup> {
        let (rank, potions, gold) = record?;
        Some(UiPickup {
            item_rank: u8::try_from(rank)
                .ok()
                .and_then(ItemRank::from_wire)
                .map_or("không rõ", ItemRank::label),
            potions: u8::try_from(potions)
                .ok()
                .and_then(PotionPickup::from_wire)
                .map_or("không rõ", PotionPickup::label),
            gold: u8::try_from(gold)
                .ok()
                .and_then(GoldPickup::from_wire)
                .map_or("không rõ", GoldPickup::label),
        })
    }

    fn reading_age_seconds(written_at_unix_ms: i64) -> Option<i64> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        let now_ms = i64::try_from(now.as_millis()).ok()?;
        let age_ms = now_ms.checked_sub(written_at_unix_ms)?;
        (age_ms >= 0).then_some(age_ms / 1_000)
    }

    fn map_status(status: ManagerAccountStatus) -> UiAccountStatus {
        match status {
            ManagerAccountStatus::Idle => UiAccountStatus::Idle,
            ManagerAccountStatus::Running => UiAccountStatus::Running,
            ManagerAccountStatus::LoginFailed => UiAccountStatus::LoginFailed,
            ManagerAccountStatus::CleanupPending => UiAccountStatus::CleanupPending,
            // A future reconciled status must not silently render as something friendlier.
            _ => UiAccountStatus::Idle,
        }
    }

    fn map_rejection(rejection: ManagerRunRejection) -> UiErrorCode {
        match rejection {
            ManagerRunRejection::TaskLimitReached => UiErrorCode::LoginCapacity,
            ManagerRunRejection::AlreadyRunning => UiErrorCode::AccountBusy,
            // Any present or future rejection that is not a capacity or busy refusal means the
            // qualified target was never reached.
            _ => UiErrorCode::LoginTarget,
        }
    }

    /// Classifies a portable boot failure into the two classes the operator can act on.
    fn map_boot_failure(code: ManagerErrorCode) -> UiBootFailure {
        match code {
            ManagerErrorCode::RuntimeRejected | ManagerErrorCode::RuntimeNotFound => {
                UiBootFailure::PinnedRuntime
            }
            // Everything else failed before or during data-root repair, which is the actionable
            // instruction: run Zeus from a writable local drive.
            _ => UiBootFailure::DataRoot,
        }
    }

    /// Maps a worker admission failure by stable code only.
    fn map_worker_error(code: ManagerWorkerErrorCode) -> UiErrorCode {
        match code {
            ManagerWorkerErrorCode::NotReady | ManagerWorkerErrorCode::ThreadSpawnFailed => {
                UiErrorCode::NotReady
            }
            ManagerWorkerErrorCode::CommandQueueFull => UiErrorCode::QueueFull,
            ManagerWorkerErrorCode::Closing | ManagerWorkerErrorCode::ShutdownPending => {
                UiErrorCode::Closing
            }
            ManagerWorkerErrorCode::Closed
            | ManagerWorkerErrorCode::Disconnected
            | ManagerWorkerErrorCode::RequestIdExhausted => UiErrorCode::Closed,
            ManagerWorkerErrorCode::InputTooLong | ManagerWorkerErrorCode::InvalidInput => {
                UiErrorCode::InvalidInput
            }
            _ => UiErrorCode::InvalidInput,
        }
    }

    /// Maps an engine failure by stable code only; no backend text is ever rendered.
    fn map_core_error(code: ManagerErrorCode) -> UiErrorCode {
        match code {
            ManagerErrorCode::InvalidUsername | ManagerErrorCode::InvalidPassword => {
                UiErrorCode::InvalidInput
            }
            ManagerErrorCode::DuplicateUsername => UiErrorCode::DuplicateAccount,
            ManagerErrorCode::AccountLimitReached => UiErrorCode::AccountLimit,
            ManagerErrorCode::AccountNotFound => UiErrorCode::AccountNotFound,
            ManagerErrorCode::RevisionConflict => UiErrorCode::RevisionConflict,
            ManagerErrorCode::CredentialVaultUnavailable => UiErrorCode::CredentialUnavailable,
            ManagerErrorCode::AccountNotRunning => UiErrorCode::AccountBusy,
            ManagerErrorCode::CleanupPending => UiErrorCode::CleanupIncomplete,
            ManagerErrorCode::ControllerClosed | ManagerErrorCode::ControllerAlreadyOpen => {
                UiErrorCode::Closed
            }
            _ => UiErrorCode::InvalidInput,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_map_holds_several_named_spots_and_a_name_means_one_place() {
            let book = UiSpotBook {
                entries: vec![
                    UiSavedSpot {
                        name: "Bãi trên".to_owned(),
                        spot: UiSpot {
                            map_id: 1,
                            zone: 0,
                            pixel_x: 480,
                            pixel_y: 720,
                        },
                    },
                    UiSavedSpot {
                        name: "Bãi dưới".to_owned(),
                        spot: UiSpot {
                            map_id: 1,
                            zone: 0,
                            pixel_x: 96,
                            pixel_y: 144,
                        },
                    },
                    UiSavedSpot {
                        name: "Bãi trên".to_owned(),
                        spot: UiSpot {
                            map_id: 33,
                            zone: 2,
                            pixel_x: 12,
                            pixel_y: 24,
                        },
                    },
                ],
            };
            // Several per map: one per map was the first shape, and it made a map's second worthwhile
            // place to stand unreachable.
            assert_eq!(book.for_map(1).len(), 2);
            // Name and map together are the identity, so the same name elsewhere is another place.
            assert_eq!(book.find(1, "Bãi trên").map(|s| s.spot.pixel_x), Some(480));
            assert_eq!(book.find(33, "Bãi trên").map(|s| s.spot.pixel_x), Some(12));
            // A name the book does not hold must answer nothing rather than the nearest thing: the
            // difference decides whether the character farms where the operator chose.
            assert_eq!(book.find(1, "Bãi không có"), None);
            assert!(book.for_map(7).is_empty());
            // The reverse lookup recovers the label the wire does not carry.
            assert_eq!(
                book.name_of(UiSpot {
                    map_id: 1,
                    zone: 0,
                    pixel_x: 96,
                    pixel_y: 144,
                }),
                Some("Bãi dưới")
            );
        }

        #[test]
        fn every_destination_the_picker_offers_names_exactly_one_map() {
            use crate::model::{NAV_TARGET_IDS, NAV_TARGET_OPTIONS};
            // The two lists are addressed by the same index, so a length that drifted would send the
            // character to whatever map happened to sit at the picker's position.
            assert_eq!(NAV_TARGET_OPTIONS.len(), NAV_TARGET_IDS.len());
            // Index 0 is off. Every other entry must be a real map, named once: a repeated id would
            // give the operator two entries that do the same thing, and one of them would be a map
            // they meant to pick and did not get.
            let mut seen: Vec<u16> = NAV_TARGET_IDS[1..].to_vec();
            let offered = seen.len();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), offered, "a map id is offered twice");
            assert!(!seen.contains(&0), "map 0 is not a destination");
            // 135 has outbound edges only: no route to it exists, so offering it would be offering a
            // destination that always fails.
            assert!(!seen.contains(&135), "map 135 cannot be routed to");
            // Every label has to carry its id, because the client reuses "Đấu Trường" for 36 and 46
            // and the operator would otherwise be choosing blind between them.
            for (label, map) in NAV_TARGET_OPTIONS[1..]
                .iter()
                .zip(NAV_TARGET_IDS[1..].iter())
            {
                assert!(
                    label.contains(&format!("({map})")),
                    "label {label} does not name map {map}"
                );
            }
        }

        #[test]
        fn ui_port_reconciled_status_never_reports_a_ui_local_pending_state() {
            // Starting, Authenticating and Stopping are created from accepted commands only. If
            // reconciliation could produce one, a pending row would be indistinguishable from
            // persisted truth and the close sequence would never drain.
            for status in [
                ManagerAccountStatus::Idle,
                ManagerAccountStatus::Running,
                ManagerAccountStatus::LoginFailed,
                ManagerAccountStatus::CleanupPending,
            ] {
                let mapped = map_status(status);
                assert!(
                    !matches!(
                        mapped,
                        UiAccountStatus::Starting
                            | UiAccountStatus::Authenticating
                            | UiAccountStatus::Stopping
                    ),
                    "{status:?} reconciled into UI-local {mapped:?}"
                );
            }
            assert_eq!(
                map_status(ManagerAccountStatus::Idle),
                UiAccountStatus::Idle
            );
            assert_eq!(
                map_status(ManagerAccountStatus::Running),
                UiAccountStatus::Running
            );
            assert_eq!(
                map_status(ManagerAccountStatus::LoginFailed),
                UiAccountStatus::LoginFailed
            );
            assert_eq!(
                map_status(ManagerAccountStatus::CleanupPending),
                UiAccountStatus::CleanupPending
            );
        }

        #[test]
        fn ui_port_every_account_failure_keeps_its_own_operator_guidance() {
            // Collapsing any of these into the catch-all would tell the operator "invalid data" when
            // the real cause is a duplicate name, a full account list, or an unreadable vault.
            for (code, expected) in [
                (ManagerErrorCode::InvalidUsername, UiErrorCode::InvalidInput),
                (ManagerErrorCode::InvalidPassword, UiErrorCode::InvalidInput),
                (
                    ManagerErrorCode::DuplicateUsername,
                    UiErrorCode::DuplicateAccount,
                ),
                (
                    ManagerErrorCode::AccountLimitReached,
                    UiErrorCode::AccountLimit,
                ),
                (
                    ManagerErrorCode::AccountNotFound,
                    UiErrorCode::AccountNotFound,
                ),
                (
                    ManagerErrorCode::RevisionConflict,
                    UiErrorCode::RevisionConflict,
                ),
                (
                    ManagerErrorCode::CredentialVaultUnavailable,
                    UiErrorCode::CredentialUnavailable,
                ),
                (
                    ManagerErrorCode::AccountNotRunning,
                    UiErrorCode::AccountBusy,
                ),
                (
                    ManagerErrorCode::CleanupPending,
                    UiErrorCode::CleanupIncomplete,
                ),
                (ManagerErrorCode::ControllerClosed, UiErrorCode::Closed),
            ] {
                assert_eq!(map_core_error(code), expected, "{code:?} mapped wrongly");
            }
        }

        #[test]
        fn ui_port_admission_refusals_keep_their_retry_semantics() {
            // These four drive different operator behavior: wait, retry now, stop asking, quit.
            for (code, expected) in [
                (ManagerWorkerErrorCode::NotReady, UiErrorCode::NotReady),
                (
                    ManagerWorkerErrorCode::ThreadSpawnFailed,
                    UiErrorCode::NotReady,
                ),
                (
                    ManagerWorkerErrorCode::CommandQueueFull,
                    UiErrorCode::QueueFull,
                ),
                (ManagerWorkerErrorCode::Closing, UiErrorCode::Closing),
                (
                    ManagerWorkerErrorCode::ShutdownPending,
                    UiErrorCode::Closing,
                ),
                (ManagerWorkerErrorCode::Closed, UiErrorCode::Closed),
                (ManagerWorkerErrorCode::Disconnected, UiErrorCode::Closed),
                (
                    ManagerWorkerErrorCode::RequestIdExhausted,
                    UiErrorCode::Closed,
                ),
                (
                    ManagerWorkerErrorCode::InputTooLong,
                    UiErrorCode::InvalidInput,
                ),
                (
                    ManagerWorkerErrorCode::InvalidInput,
                    UiErrorCode::InvalidInput,
                ),
            ] {
                assert_eq!(map_worker_error(code), expected, "{code:?} mapped wrongly");
            }
        }

        #[test]
        fn ui_port_run_rejection_distinguishes_capacity_from_busy() {
            assert_eq!(
                map_rejection(ManagerRunRejection::TaskLimitReached),
                UiErrorCode::LoginCapacity
            );
            assert_eq!(
                map_rejection(ManagerRunRejection::AlreadyRunning),
                UiErrorCode::AccountBusy
            );
            // A start failure never reached a qualified window, so it is a target failure, not a
            // capacity refusal the operator could resolve by stopping something.
            assert_eq!(
                map_rejection(ManagerRunRejection::StartFailed),
                UiErrorCode::LoginTarget
            );
        }

        #[test]
        fn ui_port_boot_failure_separates_runtime_from_data_root() {
            // The two classes produce different operator instructions: re-provision the runtime, or
            // move the whole folder to a writable local drive.
            for code in [
                ManagerErrorCode::RuntimeRejected,
                ManagerErrorCode::RuntimeNotFound,
            ] {
                assert_eq!(map_boot_failure(code), UiBootFailure::PinnedRuntime);
            }
            for code in [
                ManagerErrorCode::InvalidDataRoot,
                ManagerErrorCode::DataRootNotManaged,
                ManagerErrorCode::DataRootInsecure,
                ManagerErrorCode::StorageFailure,
            ] {
                assert_eq!(map_boot_failure(code), UiBootFailure::DataRoot);
            }
        }

        #[test]
        fn ui_port_option_labels_are_the_clients_own_in_the_clients_own_order() {
            // The UI layer keeps its own copy of these so the config dialog can be laid out without a
            // running engine. The copy is only safe while it agrees with the engine's table, which is
            // itself the game's `df.gL` strings — an index that drifted would mislabel a setting and
            // send the operator's "nhặt từ đồ tím" to the client as something else.
            for (index, option) in crate::model::ITEM_RANK_OPTIONS.iter().enumerate() {
                let engine = ItemRank::ALL[index];
                assert_eq!(*option, engine.label(), "item rank {index}");
                assert_eq!(
                    index as u8,
                    engine.as_wire(),
                    "item rank {index} wire value"
                );
            }
            assert_eq!(crate::model::ITEM_RANK_OPTIONS.len(), ItemRank::ALL.len());
            for (index, option) in crate::model::POTION_PICKUP_OPTIONS.iter().enumerate() {
                let engine = PotionPickup::ALL[index];
                assert_eq!(*option, engine.label(), "potion pickup {index}");
                assert_eq!(index as u8, engine.as_wire());
            }
            assert_eq!(
                crate::model::POTION_PICKUP_OPTIONS.len(),
                PotionPickup::ALL.len()
            );
            for (index, option) in crate::model::GOLD_OPTIONS.iter().enumerate() {
                assert_eq!(*option, GoldPickup::ALL[index].label(), "gold {index}");
                assert_eq!(index as u8, GoldPickup::ALL[index].as_wire());
            }
            assert_eq!(crate::model::GOLD_OPTIONS.len(), GoldPickup::ALL.len());
            // Revive is the one picker whose index is not its wire value: the engine's modes start
            // at 1 because 0 used to mean "off", which is now its own tick box.
            for (index, option) in crate::model::REVIVE_OPTIONS.iter().enumerate() {
                assert_eq!(*option, ReviveMode::ALL[index].label(), "revive {index}");
                assert_eq!(index as u8 + 1, ReviveMode::ALL[index].as_wire());
            }
            assert_eq!(crate::model::REVIVE_OPTIONS.len(), ReviveMode::ALL.len());
            // Every id the engine accepts has a label, so no dropdown entry can render blank, and
            // the ids line up with the engine's own list rather than being a second table.
            assert_eq!(
                crate::model::MOUNT_LABELS
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
                zeus_core::MOUNT_TEMPLATE_IDS.to_vec()
            );
            for (id, _) in crate::model::MOUNT_LABELS {
                assert!(!crate::model::mount_label(id, &[]).is_empty(), "mount {id}");
            }
            // A name the client reported beats the fallback: the server owns what a mount is called.
            let carried = [(63, String::from("Ngựa bạch"))];
            assert_eq!(crate::model::mount_label(63, &carried), "Ngựa bạch");
            assert_eq!(crate::model::mount_label(65, &carried), "Ngựa xích thố");
            for (index, mode) in [AttackMode::Off, AttackMode::Stand, AttackMode::Move]
                .into_iter()
                .enumerate()
            {
                assert_eq!(crate::model::ATTACK_MODE_OPTIONS[index], mode.label());
                assert_eq!(index as u8, mode.as_wire());
            }
            for (index, option) in crate::model::ZONE_MODE_OPTIONS.iter().enumerate() {
                assert_eq!(*option, ZoneMode::ALL[index].label(), "zone mode {index}");
                assert_eq!(index as u8, ZoneMode::ALL[index].as_wire());
            }
            assert_eq!(crate::model::ZONE_MODE_OPTIONS.len(), ZoneMode::ALL.len());
            // The material order IS the wire order, measured from a traced session, and the dialog's
            // row labels are the only place the names live. A row whose label named a different
            // material would close the drop on something the operator did not choose, silently.
            assert_eq!(
                crate::model::MATERIAL_SLOTS,
                zeus_core::MATERIAL_SLOTS,
                "the material count drifted from the engine's"
            );
            for slot in 0..zeus_core::MATERIAL_SLOTS {
                let id = crate::windows::dialogs::ID_CFG_DROP_FIRST + slot as u16;
                let row = crate::windows::dialogs::CONFIG_ROWS
                    .iter()
                    .find(|row| row.id == id)
                    .unwrap_or_else(|| panic!("material {slot} has no config row"));
                let name = zeus_core::MATERIAL_LABELS[slot].to_lowercase();
                assert!(
                    row.label.to_lowercase().contains(&name),
                    "config row for material {slot} is {:?}, which does not name {name}",
                    row.label
                );
            }
        }

        #[test]
        fn ui_port_the_two_default_settings_agree_field_for_field() {
            // The UI default is only a placeholder for a row the engine has not answered for. If it
            // disagreed, the settings dialog would open on one value and be replaced by another a tick
            // later — which is exactly what happened when the engine defaulted the potion pump off
            // while the dialog showed it on.
            assert_eq!(
                from_core_settings(&ControlSettings::default()),
                UiControl::default()
            );
        }

        #[test]
        fn ui_port_settings_survive_the_round_trip_to_the_engine_and_back() {
            // The dialog edits the UI shape and the panel renders the engine's answer, so a field lost
            // in either direction would silently reset a setting the operator had chosen.
            let configured = UiControl {
                mode: UiAutoMode::Move,
                spot: Some(UiSpot {
                    map_id: 43,
                    zone: 4,
                    pixel_x: 228,
                    pixel_y: 164,
                }),
                radius: 200,
                hp_on: true,
                hp_percent: 60,
                mp_on: false,
                mp_percent: 20,
                revive_on: true,
                revive: 1,
                revive_delay_seconds: 12,
                buffs: [true, false, true],
                item_rank: 3,
                potion_pickup: 1,
                gold: 1,
                mount: true,
                mount_template_id: zeus_core::MOUNT_ANY,
                medal_dialog: false,
                zone_mode: 2,
                zone_pick: 4,
                materials_managed: true,
                materials: [true, false, true, true, false, true],
                // The picker's own index, not a map id: 1 is "Làng Sói Trắng (1)".
                nav_target: 1,
                ring: true,
                farm_on_arrival: true,
                spot_name: "Bãi trên".to_owned(),
                detect_spots: false,
                // ---- ENHANCE ----
                enhance_on: true,
                enhance_max_level: 12,
                enhance_charm: 2,
                // ---- end ENHANCE ----
                // ---- DUNGEON ----
                dungeon_on: true,
                // Both pickers on an index past their sentinel, so a round trip that forgot to
                // translate would land on the wrong value rather than accidentally agreeing at zero.
                // Index 6 is 6 runs; index 33 is slot 32, which is 16:00.
                dungeon_max: 6,
                dungeon_schedule: 33,
                // ---- end DUNGEON ----
            };
            // The name and the detect request are the tool's own: the engine stores where a spot is,
            // and the book stores what it is called, so neither survives the wire by design.
            let expected = UiControl {
                spot_name: String::new(),
                ..configured.clone()
            };
            assert_eq!(
                from_core_settings(&to_core_settings(configured.clone())),
                expected
            );
            let bare = UiControl::default();
            // A bare config has no spot, which the engine writes as off — the one field it changes.
            let returned = from_core_settings(&to_core_settings(bare.clone()));
            assert_eq!(returned, bare);
            assert_eq!(returned.mode, UiAutoMode::Off);
        }

        #[test]
        fn ui_port_clamps_before_the_engine_has_to() {
            // The engine clamps too, but silently: a threshold typed as 0 would come back as 1 and the
            // dialog would look like it had ignored the operator.
            let wild = UiControl {
                radius: 5_000,
                hp_percent: 0,
                mp_percent: 200,
                revive: 9,
                item_rank: 9,
                potion_pickup: 9,
                gold: 9,
                zone_mode: 9,
                zone_pick: 200,
                ..UiControl::default()
            }
            .clamped();
            assert_eq!(wild.radius, crate::model::RADIUS_MAX);
            assert_eq!(wild.hp_percent, 1);
            assert_eq!(wild.mp_percent, 99);
            assert_eq!(wild.revive, 1);
            assert_eq!(wild.item_rank, 5);
            assert_eq!(wild.potion_pickup, 3);
            assert_eq!(wild.gold, 1);
            assert_eq!(wild.zone_mode, 2);
            assert_eq!(wild.zone_pick, crate::model::ZONE_PICK_MAX);
            // Every clamped picker still resolves to a real engine option rather than a default.
            let core = to_core_settings(wild);
            assert_eq!(core.item_rank, ItemRank::None);
            assert_eq!(core.potion_pickup, PotionPickup::None);
            assert_eq!(core.gold, GoldPickup::Skip);
            assert_eq!(core.revive, ReviveMode::Town);
            assert_eq!(core.zone_mode, ZoneMode::Pick);
        }

        #[test]
        fn ui_port_initializes_neutral_visual_qol_defaults() {
            let core = to_core_settings(UiControl::default());
            assert_eq!(core.effects, 1);
            assert_eq!(core.hide_players, 0);
        }
    }
}

//! Platform-independent application controller.
//!
//! It owns the pure model and drives it through an [`AccountPort`]. Task 12 binds this to HWND
//! controls; nothing here touches Win32, so every rule is testable on any platform.

use crate::model::{
    AccountTableModel, MAX_BATCH_RUN, RowKey, SubmitResult, UiAccountRow, UiAutoMode,
    UiBootFailure, UiControl, UiErrorCode, UiPlayerInfo, UiRowAction, UiSavedSpot, UiSpot,
};
use crate::port::AccountPort;

/// What the shell must ask before closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosePrompt {
    /// No retained work: submit shutdown and pump until consumed.
    ShutdownImmediately,
    /// Active rows exist. The only choices are stop-and-exit or cancel; there is no leave-running.
    StopAndExitOrCancel,
}

/// Where the confirmed close sequence currently is.
///
/// The operator is never offered a leave-running option: the only choices are stop-and-exit or cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClosePhase {
    /// No close requested.
    #[default]
    Idle,
    /// Waiting for the operator to confirm stop-and-exit or cancel.
    AwaitingConfirmation,
    /// Stops submitted; pumping their results until the inventory drains.
    StoppingAccounts,
    /// Shutdown submitted; waiting for its consumed result.
    ShutdownSubmitted,
    /// The worker confirmed close; the window may be destroyed.
    Closed,
}

impl ClosePhase {
    /// Bounded Vietnamese progress copy for the status line.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Idle => None,
            Self::AwaitingConfirmation => Some("Xác nhận dừng và thoát?"),
            Self::StoppingAccounts => Some("Đang dừng các tài khoản..."),
            Self::ShutdownSubmitted => Some("Đang đóng..."),
            Self::Closed => Some("Đã đóng"),
        }
    }
}

/// Per-row outcome of one bounded batch submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchOutcome {
    pub row: RowKey,
    pub result: SubmitResult,
}

/// Drives the pure model through a port.
pub struct AccountApp<P: AccountPort> {
    port: P,
    model: AccountTableModel,
    close_phase: ClosePhase,
    /// Set once the first list has been accepted, so readiness cannot queue a second one.
    initial_list_submitted: bool,
}

impl<P: AccountPort> AccountApp<P> {
    pub fn new(port: P) -> Self {
        Self {
            port,
            model: AccountTableModel::new(),
            close_phase: ClosePhase::Idle,
            initial_list_submitted: false,
        }
    }

    pub fn model(&self) -> &AccountTableModel {
        &self.model
    }

    pub fn refresh(&mut self) -> SubmitResult {
        // The shared spot book rides along with the account list: both are what the window opens on,
        // and asking for the book only when a dialog opens would show an empty one the first time.
        // Its result is not the caller's business — the list is — so a refused read is dropped here
        // and the book simply stays as it was.
        let spots = self.port.observe_spots();
        let _ = self.model.submit_spots(spots);
        let result = self.port.list();
        self.model.submit_list(result)
    }

    /// Submits an import, keeping the dialog open when the submission is refused.
    pub fn import(&mut self, username: &str, password: Vec<u16>) -> SubmitResult {
        let result = self.port.import(username, password);
        self.model.submit_import(result)
    }

    pub fn update(
        &mut self,
        row: RowKey,
        expected: i64,
        username: &str,
        password: Option<Vec<u16>>,
    ) -> SubmitResult {
        let result = self.port.update(row, expected, username, password);
        self.model.submit_update(row, result)
    }

    pub fn delete(&mut self, row: RowKey, expected: i64) -> SubmitResult {
        let result = self.port.delete(row, expected);
        self.model.submit_delete(row, result)
    }

    /// Sets which world one account logs into.
    ///
    /// Tracked as an update because the worker answers with the same account event, so the row
    /// reconciles through one path rather than two.
    pub fn set_server(&mut self, row: RowKey, server_index: u8) -> SubmitResult {
        let result = self.port.set_server(row, server_index);
        self.model.submit_update(row, result)
    }

    /// Runs one row through the same bounded batch path a multi-row Run uses.
    pub fn run_row(&mut self, row: RowKey) -> SubmitResult {
        self.run_rows(&[row])
    }

    /// Runs the runnable subset of `rows`, capped at the batch ceiling.
    pub fn run_rows(&mut self, rows: &[RowKey]) -> SubmitResult {
        let requests = self.model.runnable_selection(rows);
        if requests.is_empty() {
            return SubmitResult::Rejected(UiErrorCode::AccountBusy);
        }
        debug_assert!(requests.len() <= MAX_BATCH_RUN);
        let keys: Vec<RowKey> = requests.iter().map(|(key, _)| *key).collect();
        let result = self.port.run_many(&requests);
        self.model.submit_run(&keys, result)
    }

    pub fn stop_row(&mut self, row: RowKey) -> SubmitResult {
        let result = self.port.stop(row);
        self.model.submit_stop(row, result)
    }

    pub fn retry_cleanup(&mut self, row: RowKey) -> SubmitResult {
        let result = self.port.retry(row);
        self.model.submit_retry_cleanup(row, result)
    }

    /// Points the character panel at one row, or clears it. Returns whether the panel changed.
    pub fn focus_player_row(&mut self, row: Option<RowKey>) -> bool {
        self.model.focus_player_row(row)
    }

    /// Asks for the focused row's character reading, if a row is focused.
    ///
    /// Called on a slow cadence rather than every tick: the client rewrites its reading about once a
    /// second, so polling faster would only re-read the same file.
    pub fn observe_focused_player(&mut self) -> Option<SubmitResult> {
        let row = self.model.player_row()?;
        let result = self.port.observe_player(row);
        Some(self.model.submit_observe_player(row, result))
    }

    /// The reading the panel should render, if any.
    pub fn player(&self) -> Option<&UiPlayerInfo> {
        self.model.player()
    }

    /// Whether any row currently owns the panel.
    pub fn player_row(&self) -> Option<RowKey> {
        self.model.player_row()
    }

    /// Mode the next press of the auto control will move one row to.
    pub fn auto_mode(&self, row: RowKey) -> UiAutoMode {
        self.model.auto_mode(row)
    }

    /// Advances one row's auto mode, keeping every other setting the operator configured.
    ///
    /// The mode is the only thing this changes: the settings it writes are the ones the engine last
    /// confirmed for this row, so a quick toggle can never quietly reset the config dialog's work.
    pub fn cycle_auto(&mut self, row: RowKey) -> SubmitResult {
        let next = self.model.auto_mode(row).next();
        let settings = UiControl {
            mode: next,
            ..self.model.control(row)
        };
        self.apply_control(row, settings)
    }

    /// Settings one row's client will read, or the defaults when it has never answered.
    pub fn control(&self, row: RowKey) -> UiControl {
        self.model.control(row)
    }

    /// The named spot on one map, or `None` when the book has no such entry.
    pub fn saved_spot(&self, map_id: u16, name: &str) -> Option<UiSpot> {
        self.model
            .spots()
            .find(map_id, name)
            .map(|saved| saved.spot)
    }

    /// Every saved spot, for the dialog to filter by whichever map its picker names.
    ///
    /// The whole book rather than one map's worth: the dialog's Map picker can name any map, and a
    /// per-map slice left every map but the live one showing nothing.
    pub fn saved_spots(&self) -> Vec<UiSavedSpot> {
        self.model.spots().entries.clone()
    }

    /// What the book calls the spot at these coordinates, or `None`.
    pub fn saved_spot_name(&self, spot: UiSpot) -> Option<String> {
        self.model.spots().name_of(spot).map(str::to_owned)
    }

    /// Walks one row's character to a map, now, without touching its settings.
    ///
    /// Separate from [`Self::apply_control`] because it is an action, not a setting: the operator
    /// pressed a button meaning "go", and a settings write refused for an unrelated reason must not
    /// swallow it. Writing only the destination leaves everything else as the engine already had it.
    pub fn walk_to_map(&mut self, row: RowKey, map_id: Option<u16>) -> SubmitResult {
        let Some(map_id) = map_id else {
            // Nothing named, nothing to do: the picker is on "do not go anywhere".
            return SubmitResult::Rejected(UiErrorCode::AutoNeedsCharacter);
        };
        let mut settings = self.model.control(row);
        settings.nav_target = crate::model::NAV_TARGET_IDS
            .iter()
            .position(|candidate| *candidate == map_id)
            .unwrap_or(0) as u8;
        // Walking is not fighting: pressing ĐI MAP must not arm auto as a side effect.
        settings.farm_on_arrival = false;
        let result = self.port.set_control(row, settings);
        self.model.submit_set_auto(result)
    }

    /// Saves one named spot, replacing any spot of the same name on the same map.
    pub fn save_spot(&mut self, spot: UiSpot, name: String) -> SubmitResult {
        let result = self.port.save_spot(spot, name);
        self.model.submit_spots(result)
    }

    /// Forgets one named spot.
    pub fn clear_spot(&mut self, map_id: u16, name: String) -> SubmitResult {
        let result = self.port.clear_spot(map_id, name);
        self.model.submit_spots(result)
    }

    /// Asks for one row's settings, so the config dialog can open on the current values.
    pub fn observe_control(&mut self, row: RowKey) -> SubmitResult {
        let result = self.port.observe_control(row);
        self.model.submit_observe_control(result)
    }

    /// Writes one row's settings, anchoring on the spot [`Self::anchor_for`] resolves.
    ///
    /// Arming with nothing to anchor on is refused locally: the engine would write the mode as off
    /// because there is no spot to hold, and the operator would see nothing happen with no
    /// explanation. A named destination whose map has a saved spot counts as something to anchor on
    /// even before the character gets there, which is what lets the walk and the fight be one press.
    pub fn apply_control(&mut self, row: RowKey, mut settings: UiControl) -> SubmitResult {
        settings = settings.clamped();
        // The mode picker IS the farm switch: choosing "đứng yên" or "di chuyển" says both that the
        // character should fight and how, so there is nothing left for a separate tick to mean. It also
        // decides what arriving means — stand walks to the exact recorded place, move only reaches the
        // bãi and roams from it.
        settings.farm_on_arrival = settings.mode != UiAutoMode::Off;
        if settings.mode != UiAutoMode::Off {
            let Some(spot) = self.anchor_for(row, &settings) else {
                // Recorded, not just returned: the operator pressed a button and nothing happened, so
                // the status line has to say why.
                return self
                    .model
                    .submit_set_auto(SubmitResult::Rejected(UiErrorCode::AutoNeedsCharacter));
            };
            settings.spot = Some(spot);
        }
        let mode = settings.mode;
        let result = self.port.set_control(row, settings.clone());
        if let SubmitResult::Accepted(_) = result {
            // Optimistic, and overwritten by the reply: the engine answers with what it actually
            // wrote, which is the value the dialog must reopen on.
            self.model.set_auto_mode(row, mode);
            self.model.set_control(row, settings);
        }
        self.model.submit_set_auto(result)
    }

    /// The spot to hold, or `None` when this row's character is not readable.
    ///
    /// Public so the settings dialog can show where the character is standing and default a position
    /// the operator has never chosen to it.
    pub fn live_spot(&self, row: RowKey) -> Option<UiSpot> {
        self.spot_for(row)
    }

    /// The spot to hold, or `None` when this row's character is not readable.
    ///
    /// Three sources, most specific first. A destination named in the picker takes the spot saved for
    /// that map, because "go to this map and farm its spot" is the whole request and the character is
    /// not there yet. Otherwise the map the character is on decides, and the book supplies the position
    /// if that map has one saved. Failing both, where the character stands is the spot.
    ///
    /// The map may now differ from the live one, which it could not before: the walker reaches another
    /// map under `nav_target`, so anchoring on a map the character has yet to arrive on is no longer
    /// arming something unreachable. The mod still refuses to fight off its own map — `attack()` bails
    /// when `fu.q.d != atkMap` — so the anchor simply waits until the walk delivers.
    fn anchor_for(&self, row: RowKey, settings: &UiControl) -> Option<UiSpot> {
        // The chosen spot is the anchor, wherever it is: the operator picked it by name from the map
        // they are sending the character to, and the walk is what gets it there. Named first because
        // the character is usually not there yet, which is the whole point of the pairing.
        if !settings.spot_name.is_empty()
            && let Some(map_id) = settings.nav_map()
            && let Some(saved) = self.saved_spot(map_id, &settings.spot_name)
        {
            return Some(saved);
        }
        let live = self.spot_for(row)?;
        // Failing that, the same name on the map underfoot: farming here without naming a destination
        // is an ordinary request and should not need the picker set.
        if !settings.spot_name.is_empty()
            && let Some(saved) = self.saved_spot(live.map_id, &settings.spot_name)
        {
            return Some(saved);
        }
        let configured = settings.spot.filter(|spot| {
            spot.map_id == live.map_id && spot.zone == live.zone && spot.pixel_x >= 0
        });
        Some(UiSpot {
            pixel_x: configured.map_or(live.pixel_x, |spot| spot.pixel_x),
            pixel_y: configured.map_or(live.pixel_y, |spot| spot.pixel_y),
            ..live
        })
    }

    /// The spot to anchor on, or `None` when this row's character is not readable.
    fn spot_for(&self, row: RowKey) -> Option<UiSpot> {
        if self.model.player_row() != Some(row) {
            return None;
        }
        let reading = self.model.player()?;
        // A reading taken while the map was loading carries the previous map's coordinates.
        if reading.stale {
            return None;
        }
        Some(UiSpot {
            map_id: reading.map_id?,
            zone: reading.zone,
            pixel_x: reading.pixel_x,
            pixel_y: reading.pixel_y,
        })
    }

    /// Stops each selected row that is actually active, one bounded command per row.
    ///
    /// A refusal on one row never blocks its siblings: each submission is independent.
    pub fn stop_rows(&mut self, rows: &[RowKey]) -> Vec<BatchOutcome> {
        let targets: Vec<RowKey> = rows
            .iter()
            .filter_map(|key| self.model.row(*key))
            .filter(|row| row.status.context_action() == UiRowAction::Stop)
            .map(|row| row.key)
            .collect();
        targets
            .into_iter()
            .map(|row| BatchOutcome {
                row,
                result: self.stop_row(row),
            })
            .collect()
    }

    /// Deletes each selected row at its own expected revision, one bounded command per row.
    pub fn delete_rows(&mut self, rows: &[RowKey]) -> Vec<BatchOutcome> {
        let targets: Vec<(RowKey, i64)> = rows
            .iter()
            .filter_map(|key| self.model.row(*key))
            .map(|row| (row.key, row.revision))
            .collect();
        targets
            .into_iter()
            .map(|(row, revision)| BatchOutcome {
                row,
                result: self.delete(row, revision),
            })
            .collect()
    }

    pub fn close_phase(&self) -> ClosePhase {
        self.close_phase
    }

    /// Submits the first list once the worker reports ready.
    ///
    /// A list submitted before readiness is refused with `NotReady` (spec section 12 admission table),
    /// so the shell must wait for the readiness event instead of asking at window-create time.
    pub fn refresh_when_ready(&mut self) -> bool {
        if !self.model.worker_ready() || self.initial_list_submitted {
            return false;
        }
        self.initial_list_submitted = matches!(self.refresh(), SubmitResult::Accepted(_));
        self.initial_list_submitted
    }

    /// The portable boot failure the worker reported, if any.
    ///
    /// The shell shows a bounded message box and exits nonzero instead of presenting an empty table
    /// over a data root it never opened.
    pub fn boot_failure(&self) -> Option<UiBootFailure> {
        self.model.boot_failure()
    }

    /// Begins the close sequence, choosing the prompt from retained work.
    pub fn request_close(&mut self) -> ClosePrompt {
        let prompt = self.close_prompt();
        self.close_phase = match prompt {
            ClosePrompt::StopAndExitOrCancel => ClosePhase::AwaitingConfirmation,
            ClosePrompt::ShutdownImmediately => ClosePhase::ShutdownSubmitted,
        };
        if self.close_phase == ClosePhase::ShutdownSubmitted {
            // Nothing is retained, so shutdown is submitted immediately.
            self.submit_shutdown();
        }
        prompt
    }

    /// Cancels an in-flight close request, leaving every row untouched.
    pub fn cancel_close(&mut self) {
        self.close_phase = ClosePhase::Idle;
    }

    /// Confirms stop-and-exit, submitting one Stop per active row.
    ///
    /// A row whose Stop is refused is not waited for: the refusal is already the answer, and the
    /// worker's own close terminates whatever is still live.
    pub fn confirm_stop_and_exit(&mut self) -> Vec<BatchOutcome> {
        self.close_phase = ClosePhase::StoppingAccounts;
        let active: Vec<RowKey> = self
            .model
            .rows()
            .iter()
            .filter(|row| row.status.is_active())
            .map(|row| row.key)
            .collect();
        let outcomes: Vec<BatchOutcome> = active
            .into_iter()
            .map(|row| BatchOutcome {
                row,
                result: self.stop_row(row),
            })
            .collect();
        self.advance_close();
        outcomes
    }

    /// Whether a close is already running, so a second request must not restart the sequence.
    pub fn close_in_flight(&self) -> bool {
        !matches!(self.close_phase, ClosePhase::Idle | ClosePhase::Closed)
    }

    /// Advances the close state machine after pumping results.
    ///
    /// Shutdown is submitted once every stop this close submitted has answered — not once the table
    /// happens to look idle. A row that answered with a failure, or that reconciled into
    /// `CleanupPending`, must not hold the window open: waiting on a status the engine no longer
    /// publishes is exactly what left the window stuck after the operator confirmed the exit. Whatever
    /// is still live is torn down by the worker's own close, which stops every retained session.
    pub fn advance_close(&mut self) -> ClosePhase {
        if self.close_phase == ClosePhase::Idle || self.close_phase == ClosePhase::Closed {
            return self.close_phase;
        }
        // A dead worker is terminal for the close regardless of the phase it was in: nothing further
        // can be submitted, and the process teardown releases every session.
        if self.model.worker_closed() {
            self.close_phase = ClosePhase::Closed;
            return self.close_phase;
        }
        if self.close_phase == ClosePhase::ShutdownSubmitted {
            return self.close_phase;
        }
        if self.model.has_pending_lifecycle() {
            self.close_phase = ClosePhase::StoppingAccounts;
            return self.close_phase;
        }
        self.submit_shutdown();
        self.close_phase
    }

    /// Submits shutdown and records the phase its result can actually reach.
    ///
    /// A refused submission never produces a result event, so waiting for one would leave the window
    /// open forever — exactly what a boot failure or an already-closed worker does. In that case the
    /// close is complete locally; dropping the port performs the real teardown.
    fn submit_shutdown(&mut self) {
        let result = self.port.shutdown();
        self.model.submit_shutdown(result);
        self.close_phase = match result {
            SubmitResult::Accepted(_) => ClosePhase::ShutdownSubmitted,
            SubmitResult::Rejected(_) => ClosePhase::Closed,
        };
    }

    /// Drains one timer tick, returning whether the table needs repainting.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        for event in self.port.poll() {
            changed |= self.model.apply(event);
        }
        // A refused lifecycle command means the table disagrees with the engine — the row it marked
        // optimistically is not what the engine holds — so the truth is re-read instead of leaving a
        // row parked on a status that offers no action.
        if self.model.take_lifecycle_failure() {
            changed |= matches!(self.refresh(), SubmitResult::Accepted(_));
        }
        if self.close_in_flight() {
            // Advanced on every drain, not only on a drain that repainted: the reply that settles the
            // close may be a shutdown result, which changes no row.
            self.advance_close();
        }
        changed
    }

    /// Decides what the close path must do.
    pub fn close_prompt(&self) -> ClosePrompt {
        if self.model.has_active_rows() {
            ClosePrompt::StopAndExitOrCancel
        } else {
            ClosePrompt::ShutdownImmediately
        }
    }

    pub fn row_action(&self, row: RowKey) -> UiRowAction {
        self.model
            .row(row)
            .map(|row| row.status.context_action())
            .unwrap_or(UiRowAction::None)
    }

    pub fn rows(&self) -> &[UiAccountRow] {
        self.model.rows()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::num::NonZeroU64;

    use super::*;
    use crate::model::{
        MAX_EVENTS_PER_POLL, UiAccountStatus, UiBatchRunItem, UiEvent, UiRequestId, UiSpotBook,
        UiSuccess,
    };
    fn key(value: u64) -> RowKey {
        RowKey::new(NonZeroU64::new(value).expect("nonzero row key"))
    }

    fn row(value: u64, username: &str, status: UiAccountStatus) -> UiAccountRow {
        UiAccountRow {
            key: key(value),
            revision: 1,
            username: username.to_owned(),
            status,
            last_run_at_unix_ms: None,
            server_index: 0,
        }
    }

    /// A reading distinguishable per row, so a misattributed panel fails loudly.
    fn reading(name: &str, level: u16) -> UiPlayerInfo {
        UiPlayerInfo {
            character_name: name.to_owned(),
            level,
            xp_permille: 0,
            xp_permille_per_hour: None,
            hp: 1,
            hp_max: 1,
            mp: 1,
            mp_max: 1,
            wallet_known: false,
            gold: 0,
            gem: 0,
            map_id: None,
            zone: -1,
            pixel_x: 0,
            pixel_y: 0,
            quota: 1,
            bag_used: None,
            bag_max: 1,
            dead: false,
            fighting: false,
            mounted: false,
            guild_name: None,
            stale: false,
            age_seconds: Some(0),
            auto_state: None,
            has_target: false,
            stuck: 0,
            pickup: None,
            buffs: [false; 3],
            materials: [None; crate::model::MATERIAL_SLOTS],
            mounts: Vec::new(),
            travel_state: 0,
            travel_why: 0,
            travel_goal: None,
            travel_hops: 0,
            potions: 0,
            revives: 0,
            settings_agreed: None,
            // ---- ENHANCE ----
            enhance_phase: 0,
            enhance_why: 0,
            enhance_done: 0,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_state: 0,
            dungeon_why: 0,
            dungeon_runs: 0,
            dungeon_goal: None,
            // ---- end DUNGEON ----
        }
    }

    /// Deterministic fake port. It proves the model and controller need no Win32 and no worker.
    #[derive(Default)]
    struct FakePort {
        next_request: UiRequestId,
        submitted: Vec<&'static str>,
        last_batch: Vec<(RowKey, i64)>,
        queued: VecDeque<UiEvent>,
        reject_with: Option<UiErrorCode>,
        last_control: Option<UiControl>,
    }

    impl FakePort {
        fn accept(&mut self, label: &'static str) -> SubmitResult {
            self.submitted.push(label);
            if let Some(code) = self.reject_with {
                return SubmitResult::Rejected(code);
            }
            self.next_request += 1;
            SubmitResult::Accepted(self.next_request)
        }

        fn enqueue(&mut self, event: UiEvent) {
            self.queued.push_back(event);
        }
    }

    impl AccountPort for FakePort {
        fn observe_spots(&mut self) -> SubmitResult {
            self.accept("observe_spots")
        }

        fn save_spot(&mut self, _spot: UiSpot, _name: String) -> SubmitResult {
            self.accept("save_spot")
        }

        fn clear_spot(&mut self, _map_id: u16, _name: String) -> SubmitResult {
            self.accept("clear_spot")
        }

        fn list(&mut self) -> SubmitResult {
            self.accept("list")
        }

        fn import(&mut self, _username: &str, password: Vec<u16>) -> SubmitResult {
            // The port owns the temporary copy; nothing retains it after submission.
            drop(password);
            self.accept("import")
        }

        fn update(
            &mut self,
            _row: RowKey,
            _expected: i64,
            _username: &str,
            password: Option<Vec<u16>>,
        ) -> SubmitResult {
            drop(password);
            self.accept("update")
        }

        fn delete(&mut self, _row: RowKey, _expected: i64) -> SubmitResult {
            self.accept("delete")
        }

        fn set_server(&mut self, _row: RowKey, _server_index: u8) -> SubmitResult {
            self.accept("set_server")
        }

        fn run_many(&mut self, rows: &[(RowKey, i64)]) -> SubmitResult {
            self.last_batch = rows.to_vec();
            self.accept("run_many")
        }

        fn stop(&mut self, _row: RowKey) -> SubmitResult {
            self.accept("stop")
        }

        fn retry(&mut self, _row: RowKey) -> SubmitResult {
            self.accept("retry")
        }

        fn observe_player(&mut self, _row: RowKey) -> SubmitResult {
            self.accept("observe_player")
        }

        fn set_control(&mut self, _row: RowKey, settings: UiControl) -> SubmitResult {
            self.last_control = Some(settings);
            self.accept("set_control")
        }

        fn observe_control(&mut self, _row: RowKey) -> SubmitResult {
            self.accept("observe_control")
        }

        fn shutdown(&mut self) -> SubmitResult {
            self.accept("shutdown")
        }

        fn poll(&mut self) -> Vec<UiEvent> {
            let mut drained = Vec::new();
            while drained.len() < MAX_EVENTS_PER_POLL {
                match self.queued.pop_front() {
                    Some(event) => drained.push(event),
                    None => break,
                }
            }
            drained
        }
    }

    fn app_with_rows(rows: Vec<UiAccountRow>) -> AccountApp<FakePort> {
        let mut app = AccountApp::new(FakePort::default());
        let SubmitResult::Accepted(request_id) = app.refresh() else {
            panic!("list must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Accounts(rows)),
        });
        assert!(app.pump());
        app
    }

    #[test]
    fn ui_model_renders_every_status_with_its_action() {
        let app = app_with_rows(vec![
            row(1, "Idle", UiAccountStatus::Idle),
            row(2, "Running", UiAccountStatus::Running),
            row(3, "Failed", UiAccountStatus::LoginFailed),
            row(4, "Cleanup", UiAccountStatus::CleanupPending),
        ]);

        let labels: Vec<&str> = app.rows().iter().map(|row| row.status.label()).collect();
        assert_eq!(
            labels,
            vec!["Chưa chạy", "Đang chạy", "Login lỗi", "Cần dọn dẹp"]
        );
        assert_eq!(app.row_action(key(1)), UiRowAction::Run);
        assert_eq!(app.row_action(key(2)), UiRowAction::Stop);
        assert_eq!(app.row_action(key(3)), UiRowAction::Run);
        assert_eq!(app.row_action(key(4)), UiRowAction::RetryCleanup);
        // An unknown row offers nothing rather than a default action.
        assert_eq!(app.row_action(key(99)), UiRowAction::None);
    }

    #[test]
    fn ui_model_pending_states_are_local_and_reconciliation_wins() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);

        // Accepted Run marks the row locally Starting.
        let SubmitResult::Accepted(run_request) = app.run_row(key(1)) else {
            panic!("run must be accepted");
        };
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Starting
        );
        assert_eq!(UiAccountStatus::Starting.label(), "Đang mở");

        // A scheduled batch result advances the row to Authenticating.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: run_request,
            result: Ok(UiSuccess::RunBatch(vec![UiBatchRunItem {
                row: key(1),
                scheduled: Ok(()),
            }])),
        });
        assert!(app.pump());
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Authenticating
        );
        assert_eq!(UiAccountStatus::Authenticating.label(), "Đang đăng nhập");

        // Terminal reconciliation overrides the local pending status.
        app.port.enqueue(UiEvent::StateChanged(row(
            1,
            "Alpha",
            UiAccountStatus::Running,
        )));
        assert!(app.pump());
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Running
        );

        // Accepted Stop marks the row locally Stopping.
        let SubmitResult::Accepted(_) = app.stop_row(key(1)) else {
            panic!("stop must be accepted");
        };
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Stopping
        );
        assert_eq!(UiAccountStatus::Stopping.label(), "Đang dừng");
    }

    #[test]
    fn ui_model_drops_a_stale_or_unknown_request_result() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);

        // A result for a request that was never submitted changes nothing.
        assert!(!app.model_mut_apply(UiEvent::RequestFinished {
            request_id: 9_999,
            result: Ok(UiSuccess::Deleted(key(1))),
        }));
        assert_eq!(app.rows().len(), 1);

        // A settled request cannot be applied twice.
        let SubmitResult::Accepted(request_id) = app.delete(key(1), 1) else {
            panic!("delete must be accepted");
        };
        assert!(app.model_mut_apply(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Deleted(key(1))),
        }));
        assert!(app.rows().is_empty());
        assert!(!app.model_mut_apply(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Deleted(key(1))),
        }));
    }

    #[test]
    fn ui_model_batch_run_is_bounded_and_isolates_one_failure() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Idle),
            row(2, "B", UiAccountStatus::Idle),
            row(3, "C", UiAccountStatus::Idle),
            row(4, "D", UiAccountStatus::Idle),
            row(5, "E", UiAccountStatus::Idle),
        ]);

        // A five-row selection is capped at the batch ceiling before submission.
        let SubmitResult::Accepted(request_id) =
            app.run_rows(&[key(1), key(2), key(3), key(4), key(5)])
        else {
            panic!("run must be accepted");
        };
        assert_eq!(app.port.last_batch.len(), MAX_BATCH_RUN);

        // A fails while B, C, and D schedule: only A reverts.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::RunBatch(vec![
                UiBatchRunItem {
                    row: key(1),
                    scheduled: Err(UiErrorCode::LoginTarget),
                },
                UiBatchRunItem {
                    row: key(2),
                    scheduled: Ok(()),
                },
                UiBatchRunItem {
                    row: key(3),
                    scheduled: Ok(()),
                },
                UiBatchRunItem {
                    row: key(4),
                    scheduled: Ok(()),
                },
            ])),
        });
        assert!(app.pump());
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Idle
        );
        for value in [2, 3, 4] {
            assert_eq!(
                app.model().row(key(value)).expect("row exists").status,
                UiAccountStatus::Authenticating,
                "row {value} must stay scheduled"
            );
        }
        assert_eq!(app.model().last_error(), Some(UiErrorCode::LoginTarget));
        // The fifth row was never submitted, so it is untouched.
        assert_eq!(
            app.model().row(key(5)).expect("row exists").status,
            UiAccountStatus::Idle
        );
    }

    #[test]
    fn ui_model_rejected_run_leaves_the_table_untouched() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);
        app.port.reject_with = Some(UiErrorCode::LoginCapacity);

        assert_eq!(
            app.run_row(key(1)),
            SubmitResult::Rejected(UiErrorCode::LoginCapacity)
        );
        // A refused submission must not leave a row stuck in a pending status.
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Idle
        );
        assert_eq!(app.model().last_error(), Some(UiErrorCode::LoginCapacity));

        // A non-runnable row is refused locally, without reaching the port.
        app.port.reject_with = None;
        let mut running = app_with_rows(vec![row(2, "Beta", UiAccountStatus::Running)]);
        assert_eq!(
            running.run_row(key(2)),
            SubmitResult::Rejected(UiErrorCode::AccountBusy)
        );
        assert!(!running.port.submitted.contains(&"run_many"));
    }

    #[test]
    fn ui_model_poll_never_exceeds_thirty_two_events_per_tick() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);
        // 33 queued events: one tick must leave exactly one for the next tick.
        for index in 0..33 {
            let status = if index % 2 == 0 {
                UiAccountStatus::Running
            } else {
                UiAccountStatus::Idle
            };
            app.port
                .enqueue(UiEvent::StateChanged(row(1, "Alpha", status)));
        }

        assert_eq!(app.port.poll_count_for_test(), MAX_EVENTS_PER_POLL);
        assert_eq!(app.port.queued.len(), 1);
        // The preserved event is still delivered on the following tick.
        assert_eq!(app.port.poll_count_for_test(), 1);
        assert!(app.port.queued.is_empty());
    }

    #[test]
    fn ui_model_every_error_code_has_bounded_vietnamese_copy() {
        for code in [
            UiErrorCode::NotReady,
            UiErrorCode::QueueFull,
            UiErrorCode::Closing,
            UiErrorCode::Closed,
            UiErrorCode::InvalidInput,
            UiErrorCode::DuplicateAccount,
            UiErrorCode::AccountLimit,
            UiErrorCode::AccountNotFound,
            UiErrorCode::RevisionConflict,
            UiErrorCode::CredentialUnavailable,
            UiErrorCode::AccountBusy,
            UiErrorCode::LoginCapacity,
            UiErrorCode::LoginTarget,
            UiErrorCode::CleanupIncomplete,
        ] {
            let label = code.label();
            assert!(!label.is_empty(), "{code:?} has no copy");
            // No backend identifier may reach the operator.
            assert!(!label.contains('_'), "{code:?} renders a backend token");
            assert!(
                !label.to_lowercase().contains("error"),
                "{code:?} renders a raw error word"
            );
        }
    }

    #[test]
    fn ui_model_worker_closed_is_terminal() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);
        assert!(!app.model().worker_closed());
        app.port.enqueue(UiEvent::WorkerClosed);
        assert!(app.pump());
        assert!(app.model().worker_closed());
    }

    #[test]
    fn batch_modal_shutdown_batch_stop_isolates_a_partial_failure() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Running),
            row(3, "C", UiAccountStatus::Idle),
        ]);

        let outcomes = app.stop_rows(&[key(1), key(2), key(3)]);
        // Only the two stoppable rows were submitted; the idle row was never touched.
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].row, key(1));
        assert_eq!(outcomes[1].row, key(2));
        assert_eq!(
            app.model().row(key(3)).expect("row exists").status,
            UiAccountStatus::Idle
        );

        // A failed stop reverts only its own row; the sibling stays Stopping.
        let SubmitResult::Accepted(first) = outcomes[0].result else {
            panic!("stop must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: first,
            result: Err(UiErrorCode::AccountNotFound),
        });
        assert!(app.pump());
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Running
        );
        assert_eq!(
            app.model().row(key(2)).expect("row exists").status,
            UiAccountStatus::Stopping
        );
    }

    #[test]
    fn batch_modal_shutdown_batch_delete_keeps_successful_rows() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Idle),
            row(2, "B", UiAccountStatus::Idle),
        ]);

        let outcomes = app.delete_rows(&[key(1), key(2)]);
        assert_eq!(outcomes.len(), 2);
        let SubmitResult::Accepted(first) = outcomes[0].result else {
            panic!("delete must be accepted");
        };
        let SubmitResult::Accepted(second) = outcomes[1].result else {
            panic!("delete must be accepted");
        };

        // The first delete fails, the second succeeds: the failure must not remove a row.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: first,
            result: Err(UiErrorCode::RevisionConflict),
        });
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: second,
            result: Ok(UiSuccess::Deleted(key(2))),
        });
        assert!(app.pump());
        assert_eq!(app.rows().len(), 1);
        assert_eq!(app.rows()[0].key, key(1));
        assert_eq!(
            app.model().last_error(),
            Some(UiErrorCode::RevisionConflict)
        );
    }

    #[test]
    fn batch_modal_shutdown_modal_drain_is_bounded_and_makes_progress() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);
        // 33 events queued while a modal dialog is open.
        for index in 0..33 {
            let status = if index == 32 {
                UiAccountStatus::Running
            } else {
                UiAccountStatus::Idle
            };
            app.port
                .enqueue(UiEvent::StateChanged(row(1, "Alpha", status)));
        }

        // The dialog timer uses the same drain: exactly 32, then the remaining one.
        assert!(app.pump());
        assert_eq!(app.port.queued.len(), 1);
        assert!(app.pump());
        assert!(app.port.queued.is_empty());
        // Worker progress was never stalled: the final event was applied.
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Running
        );
    }

    #[test]
    fn batch_modal_shutdown_close_with_no_work_submits_shutdown_immediately() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);

        assert_eq!(app.request_close(), ClosePrompt::ShutdownImmediately);
        assert_eq!(app.close_phase(), ClosePhase::ShutdownSubmitted);
        assert!(app.port.submitted.contains(&"shutdown"));

        // The window closes only after the worker confirms.
        app.port.enqueue(UiEvent::WorkerClosed);
        assert!(app.pump());
        assert_eq!(app.advance_close(), ClosePhase::Closed);
        assert_eq!(app.close_phase(), ClosePhase::Closed);
        assert!(!app.close_in_flight());
    }

    #[test]
    fn batch_modal_shutdown_close_with_active_rows_offers_only_stop_or_cancel() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Idle),
        ]);

        assert_eq!(app.request_close(), ClosePrompt::StopAndExitOrCancel);
        assert_eq!(app.close_phase(), ClosePhase::AwaitingConfirmation);
        // Nothing was submitted while the confirmation is pending.
        assert!(!app.port.submitted.contains(&"shutdown"));
        assert_eq!(
            ClosePhase::AwaitingConfirmation.label(),
            Some("Xác nhận dừng và thoát?")
        );

        // Cancel leaves every row untouched and submits nothing.
        app.cancel_close();
        assert_eq!(app.close_phase(), ClosePhase::Idle);
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Running
        );
        assert!(!app.port.submitted.contains(&"shutdown"));
    }

    #[test]
    fn batch_modal_shutdown_stop_and_exit_pumps_until_inventory_is_empty() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Running),
        ]);
        assert_eq!(app.request_close(), ClosePrompt::StopAndExitOrCancel);

        let outcomes = app.confirm_stop_and_exit();
        assert_eq!(outcomes.len(), 2);
        let ids: Vec<UiRequestId> = outcomes
            .iter()
            .map(|outcome| match outcome.result {
                SubmitResult::Accepted(id) => id,
                SubmitResult::Rejected(code) => panic!("stop must be accepted, got {code:?}"),
            })
            .collect();
        // Both stops are unanswered, so shutdown is not submitted yet.
        assert_eq!(app.close_phase(), ClosePhase::StoppingAccounts);
        assert!(!app.port.submitted.contains(&"shutdown"));
        assert_eq!(
            ClosePhase::StoppingAccounts.label(),
            Some("Đang dừng các tài khoản...")
        );

        // One stop answers: the other is still outstanding.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: ids[0],
            result: Ok(UiSuccess::Account(row(1, "A", UiAccountStatus::Idle))),
        });
        assert!(app.pump());
        assert_eq!(app.close_phase(), ClosePhase::StoppingAccounts);
        assert!(!app.port.submitted.contains(&"shutdown"));

        // The last stop answers, so shutdown is submitted exactly once.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: ids[1],
            result: Ok(UiSuccess::Account(row(2, "B", UiAccountStatus::Idle))),
        });
        assert!(app.pump());
        assert_eq!(app.close_phase(), ClosePhase::ShutdownSubmitted);
        assert_eq!(
            app.port
                .submitted
                .iter()
                .filter(|label| **label == "shutdown")
                .count(),
            1
        );
        // Both rows released their optimistic status on their own replies.
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Idle
        );
        assert_eq!(
            app.model().row(key(2)).expect("row exists").status,
            UiAccountStatus::Idle
        );
    }

    #[test]
    fn batch_modal_shutdown_proceeds_when_a_stopped_row_reconciles_to_cleanup_pending() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        assert_eq!(app.request_close(), ClosePrompt::StopAndExitOrCancel);
        app.confirm_stop_and_exit();
        let request = app.port.next_request;

        // The stop answers with a row that still needs cleanup. The regression this guards: the close
        // waited for that row to reach Idle, but a retained session publishes nothing further, so the
        // window stayed open forever after the operator had already confirmed the exit.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: request,
            result: Ok(UiSuccess::Account(row(
                1,
                "A",
                UiAccountStatus::CleanupPending,
            ))),
        });
        assert!(app.pump());
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::CleanupPending
        );
        // Shutdown is submitted on that same drain: the worker's own close tears down whatever session
        // is still retained.
        assert_eq!(app.close_phase(), ClosePhase::ShutdownSubmitted);
        assert!(app.port.submitted.contains(&"shutdown"));

        app.port.enqueue(UiEvent::WorkerClosed);
        assert!(app.pump());
        assert_eq!(app.close_phase(), ClosePhase::Closed);
    }

    #[test]
    fn batch_modal_shutdown_run_batch_reserves_one_generation_for_four_rows() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Idle),
            row(2, "B", UiAccountStatus::Idle),
            row(3, "C", UiAccountStatus::Idle),
            row(4, "D", UiAccountStatus::Idle),
        ]);

        let SubmitResult::Accepted(_) = app.run_rows(&[key(1), key(2), key(3), key(4)]) else {
            panic!("a four-row batch must be accepted");
        };
        // Exactly one bounded batch command carried all four rows.
        assert_eq!(
            app.port
                .submitted
                .iter()
                .filter(|label| **label == "run_many")
                .count(),
            1
        );
        assert_eq!(app.port.last_batch.len(), 4);
        // Every row carries its own expected revision.
        for (index, (row_key, revision)) in app.port.last_batch.iter().enumerate() {
            assert_eq!(*row_key, key(index as u64 + 1));
            assert_eq!(*revision, 1);
        }
    }

    #[test]
    fn batch_modal_shutdown_stopping_one_account_does_not_close_the_worker() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Running),
        ]);

        let SubmitResult::Accepted(request_id) = app.stop_row(key(1)) else {
            panic!("stop must be accepted");
        };
        // A confirmed stop answers with the account's reconciled row.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Account(row(1, "A", UiAccountStatus::Idle))),
        });
        assert!(app.pump());

        // The regression this guards: mapping a stop result to UiSuccess::Shutdown marked the whole
        // engine closed, so one stopped account made the UI believe the worker had exited.
        assert!(
            !app.model().worker_closed(),
            "stopping one account must not close the worker"
        );
        // The second regression: an acknowledgement left the row on its optimistic `Stopping` status,
        // whose context action is None, so the account could never be started again.
        assert_eq!(
            app.model().row(key(1)).expect("row exists").status,
            UiAccountStatus::Idle,
            "a confirmed stop must release the row on its own reply"
        );
        assert_eq!(app.row_action(key(1)), UiRowAction::Run);
        // The other row is untouched by its sibling's stop.
        assert_eq!(
            app.model().row(key(2)).expect("row exists").status,
            UiAccountStatus::Running
        );
    }

    #[test]
    fn ui_model_boot_failure_is_reported_and_terminal() {
        let mut app = app_with_rows(Vec::new());
        assert_eq!(app.boot_failure(), None);
        assert!(!app.model().worker_closed());
        app.port
            .enqueue(UiEvent::BootFailed(UiBootFailure::PinnedRuntime));
        assert!(app.pump());
        // The shell turns this into a bounded message box plus a nonzero exit, so the class must
        // survive and the worker must be considered gone.
        assert_eq!(app.boot_failure(), Some(UiBootFailure::PinnedRuntime));
        assert!(app.model().worker_closed());
    }

    #[test]
    fn ui_model_worker_ready_changes_no_row_state() {
        let mut app = app_with_rows(vec![row(1, "Alpha", UiAccountStatus::Idle)]);
        app.port.enqueue(UiEvent::WorkerReady);
        // Readiness only unblocks admission; repainting on it would be a wasted table rebuild.
        assert!(!app.pump());
        assert_eq!(app.boot_failure(), None);
        assert!(!app.model().worker_closed());
    }

    #[test]
    fn ui_model_first_list_waits_for_worker_readiness() {
        let mut app = AccountApp::new(FakePort::default());
        // Before readiness the worker refuses every command with NotReady, so asking is pure waste and
        // leaves the operator staring at an empty table that never fills.
        assert!(!app.refresh_when_ready());
        assert!(app.port.submitted.is_empty());

        app.port.enqueue(UiEvent::WorkerReady);
        app.pump();
        assert!(app.refresh_when_ready());
        // The shared spot book is asked for alongside the list: both are what the window opens on.
        assert_eq!(app.port.submitted, vec!["observe_spots", "list"]);

        // Readiness is observed once; a later drain must not queue a second list.
        assert!(!app.refresh_when_ready());
        assert_eq!(app.port.submitted, vec!["observe_spots", "list"]);
    }

    #[test]
    fn ui_model_refused_shutdown_still_finishes_the_close() {
        let mut app = app_with_rows(Vec::new());
        // A boot failure or an already-closed worker refuses shutdown, and a refused submission never
        // produces a result event. Waiting for one held the window open forever.
        app.port.reject_with = Some(UiErrorCode::Closed);
        assert_eq!(app.request_close(), ClosePrompt::ShutdownImmediately);
        assert_eq!(app.close_phase(), ClosePhase::Closed);
        assert!(!app.close_in_flight());
    }

    #[test]
    fn ui_model_accepted_shutdown_waits_for_its_result() {
        let mut app = app_with_rows(Vec::new());
        assert_eq!(app.request_close(), ClosePrompt::ShutdownImmediately);
        // An accepted shutdown must not claim completion before the worker confirms it.
        assert_eq!(app.close_phase(), ClosePhase::ShutdownSubmitted);
        assert!(app.close_in_flight());

        app.port.enqueue(UiEvent::WorkerClosed);
        assert!(app.pump());
        assert_eq!(app.advance_close(), ClosePhase::Closed);
    }

    #[test]
    fn player_panel_polls_only_the_focused_row() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Running),
        ]);

        // With nothing focused there is nothing to read, so no command is submitted at all.
        assert!(app.observe_focused_player().is_none());
        assert!(!app.port.submitted.contains(&"observe_player"));
        assert_eq!(app.player_row(), None);
        assert_eq!(app.player(), None);

        assert!(app.focus_player_row(Some(key(1))));
        assert_eq!(app.player_row(), Some(key(1)));
        // Focusing the same row again is not a change, so it cannot restart the panel.
        assert!(!app.focus_player_row(Some(key(1))));

        let Some(SubmitResult::Accepted(request_id)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Player {
                row: key(1),
                info: Some(reading("Alpha", 80)),
            }),
        });
        // A reading reaches the panel without repainting the table, which is why pump reports nothing.
        assert!(!app.pump());
        assert_eq!(
            app.player().map(|info| info.character_name.as_str()),
            Some("Alpha")
        );
    }

    #[test]
    fn player_panel_never_shows_one_account_reading_under_another_row() {
        let mut app = app_with_rows(vec![
            row(1, "A", UiAccountStatus::Running),
            row(2, "B", UiAccountStatus::Running),
        ]);
        app.focus_player_row(Some(key(1)));
        let Some(SubmitResult::Accepted(first)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };

        // The operator clicks the other row while the first read is still in flight. The panel must
        // clear immediately rather than keep showing the previous character under the new row.
        assert!(app.focus_player_row(Some(key(2))));
        assert_eq!(app.player(), None);

        app.port.enqueue(UiEvent::RequestFinished {
            request_id: first,
            result: Ok(UiSuccess::Player {
                row: key(1),
                info: Some(reading("Alpha", 80)),
            }),
        });
        // The late answer belongs to a row nobody is looking at, so it changes nothing.
        assert!(!app.pump());
        assert_eq!(app.player(), None);

        let Some(SubmitResult::Accepted(second)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: second,
            result: Ok(UiSuccess::Player {
                row: key(2),
                info: Some(reading("Beta", 12)),
            }),
        });
        assert!(!app.pump());
        assert_eq!(app.player().map(|info| info.level), Some(12));
    }

    #[test]
    fn player_panel_reading_that_did_not_change_does_not_force_a_repaint() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));
        for level in [80, 81] {
            let Some(SubmitResult::Accepted(request_id)) = app.observe_focused_player() else {
                panic!("a focused poll must be accepted");
            };
            app.port.enqueue(UiEvent::RequestFinished {
                request_id,
                result: Ok(UiSuccess::Player {
                    row: key(1),
                    info: Some(reading("Alpha", level)),
                }),
            });
            // A reading never counts as a table change, changed values included. The table renders no
            // character data, and a repaint rebuilds every item: reporting a change made the
            // once-a-second poll wipe the operator's highlight and checkboxes.
            assert!(!app.pump(), "a reading must not repaint the table");
            assert_eq!(app.player().map(|info| info.level), Some(level));
        }
    }

    #[test]
    fn player_panel_clears_when_its_row_leaves_the_table() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Idle)]);
        app.focus_player_row(Some(key(1)));
        let Some(SubmitResult::Accepted(request_id)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Player {
                row: key(1),
                info: Some(reading("Alpha", 80)),
            }),
        });
        assert!(!app.pump());
        assert!(app.player().is_some());

        // Deleting the focused row must not leave its character on screen.
        let SubmitResult::Accepted(delete) = app.delete(key(1), 1) else {
            panic!("delete must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id: delete,
            result: Ok(UiSuccess::Deleted(key(1))),
        });
        assert!(app.pump());
        assert_eq!(app.player_row(), None);
        assert_eq!(app.player(), None);

        // A row that is not in the table cannot be focused in the first place.
        assert!(!app.focus_player_row(Some(key(99))));
        assert_eq!(app.player_row(), None);
    }

    #[test]
    fn player_panel_poll_failure_is_not_reported_as_an_operator_error() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Idle)]);
        app.focus_player_row(Some(key(1)));
        let Some(SubmitResult::Accepted(request_id)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };
        // The poll repeats every second. Surfacing its failure would bury every real error under a
        // stream of noise the operator cannot act on.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Err(UiErrorCode::InvalidInput),
        });
        assert!(!app.pump());
        assert_eq!(app.model().last_error(), None);

        // A refused submission is equally silent, and leaves any existing reading in place.
        app.port.reject_with = Some(UiErrorCode::QueueFull);
        assert_eq!(
            app.observe_focused_player(),
            Some(SubmitResult::Rejected(UiErrorCode::QueueFull))
        );
        assert_eq!(app.model().last_error(), None);
    }

    #[test]
    fn player_panel_reports_a_published_absence_as_no_data() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Idle)]);
        app.focus_player_row(Some(key(1)));
        let Some(SubmitResult::Accepted(request_id)) = app.observe_focused_player() else {
            panic!("a focused poll must be accepted");
        };
        // Before a character is entered the client has published nothing. That is ordinary: the row
        // stays focused and the panel shows no data rather than an error.
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Player {
                row: key(1),
                info: None,
            }),
        });
        assert!(!app.pump());
        assert_eq!(app.player_row(), Some(key(1)));
        assert_eq!(app.player(), None);
        assert_eq!(app.model().last_error(), None);
    }

    #[test]
    fn choosing_a_mode_is_what_arms_the_farm() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));
        let mut settled = reading("Nhân vật", 80);
        settled.map_id = Some(7);
        settled.zone = 3;
        settled.pixel_x = 504;
        settled.pixel_y = 264;
        app.put_reading(key(1), settled);

        // The mode picker says both that the character should fight and how, so it is the farm switch:
        // a separate tick would be a second control meaning the same thing, and one of the two would
        // eventually disagree with the other.
        let asked = UiControl {
            mode: UiAutoMode::Stand,
            ..UiControl::default()
        };
        assert!(matches!(
            app.apply_control(key(1), asked),
            SubmitResult::Accepted(_)
        ));
        let written = app
            .port
            .last_control
            .clone()
            .expect("the port received settings");
        assert_eq!(written.mode, UiAutoMode::Stand);
        assert!(written.farm_on_arrival, "a mode means farm on arrival");

        // Off means park: ĐI MAP walks without fighting, and that has to stay expressible.
        let parked = UiControl {
            mode: UiAutoMode::Off,
            farm_on_arrival: true,
            ..UiControl::default()
        };
        let _ = app.apply_control(key(1), parked);
        let written = app
            .port
            .last_control
            .clone()
            .expect("the port received settings");
        assert!(!written.farm_on_arrival, "off must not arm the fight");
    }

    #[test]
    fn a_named_destination_anchors_on_that_maps_saved_spot() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));

        // Save a spot for map 1 through the real path, then answer it: the reply is what seeds the
        // model, and a book the model never received would make this test prove nothing.
        let saved = UiSpot {
            map_id: 1,
            zone: 0,
            pixel_x: 480,
            pixel_y: 720,
        };
        let SubmitResult::Accepted(request_id) = app.save_spot(saved, "Bãi trên".to_owned())
        else {
            panic!("saving a spot must be accepted");
        };
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Spots(UiSpotBook {
                entries: vec![UiSavedSpot {
                    name: "Bãi trên".to_owned(),
                    spot: saved,
                }],
            })),
        });
        assert!(app.pump());

        // Index 1 is "Làng Sói Trắng (1)". Naming it must anchor on map 1's saved spot even though the
        // character is nowhere near: arriving somewhere useless is the failure this prevents.
        let mut settings = UiControl {
            mode: UiAutoMode::Stand,
            nav_target: 1,
            spot_name: "Bãi trên".to_owned(),
            ..UiControl::default()
        };
        let anchor = app
            .anchor_for(key(1), &settings)
            .expect("the named spot on the named map is an anchor");
        assert_eq!(anchor.map_id, 1);
        assert_eq!((anchor.pixel_x, anchor.pixel_y), (480, 720));

        // A name the book does not hold is not an anchor: it must not fall back to another spot, or the
        // character would farm somewhere the operator never chose.
        settings.spot_name = "Bãi không có".to_owned();
        assert_eq!(app.anchor_for(key(1), &settings), None);

        // No name and no reading: nothing to anchor on at all.
        settings.spot_name = String::new();
        settings.nav_target = 0;
        assert_eq!(app.anchor_for(key(1), &settings), None);
    }

    #[test]
    fn auto_cannot_be_armed_without_a_spot_to_anchor_on() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));

        // No reading yet: arming would have no anchor, the engine would write it as off, and the
        // operator would see nothing happen with no explanation.
        assert_eq!(
            app.cycle_auto(key(1)),
            SubmitResult::Rejected(UiErrorCode::AutoNeedsCharacter)
        );
        assert!(!app.port.submitted.contains(&"set_control"));
        assert_eq!(app.auto_mode(key(1)), UiAutoMode::Off);
        assert_eq!(
            app.model().last_error(),
            Some(UiErrorCode::AutoNeedsCharacter)
        );

        // A reading taken while the map was loading carries the previous map's coordinates, so it
        // is refused for the same reason.
        let mut loading = reading("Alpha", 80);
        loading.map_id = Some(1);
        loading.stale = true;
        app.put_reading(key(1), loading);
        assert_eq!(
            app.cycle_auto(key(1)),
            SubmitResult::Rejected(UiErrorCode::AutoNeedsCharacter)
        );

        // A settled reading arms it, anchored on exactly where the character is standing.
        let mut settled = reading("Alpha", 80);
        settled.map_id = Some(7);
        settled.zone = 3;
        settled.pixel_x = 504;
        settled.pixel_y = 264;
        app.put_reading(key(1), settled);
        assert!(matches!(app.cycle_auto(key(1)), SubmitResult::Accepted(_)));
        assert_eq!(app.auto_mode(key(1)), UiAutoMode::Stand);
        let written = app.port.last_control.expect("the port received settings");
        assert_eq!(written.mode, UiAutoMode::Stand);
        let spot = written.spot.expect("the settings carried a spot");
        assert_eq!(spot.map_id, 7);
        assert_eq!(spot.zone, 3);
        assert_eq!(spot.pixel_x, 504);
        assert_eq!(spot.pixel_y, 264);
    }

    #[test]
    fn a_quick_toggle_keeps_every_setting_the_config_dialog_chose() {
        // The two controls write the same file. If the toggle rebuilt the settings from defaults it
        // would silently undo the operator's configuration the moment they pressed it.
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));
        let mut settled = reading("Alpha", 80);
        settled.map_id = Some(7);
        settled.zone = 3;
        settled.pixel_x = 504;
        settled.pixel_y = 264;
        app.put_reading(key(1), settled);

        let configured = UiControl {
            radius: 200,
            hp_percent: 70,
            mp_percent: 15,
            revive_on: true,
            revive: 1,
            buffs: [true, false, true],
            item_rank: 3,
            potion_pickup: 2,
            gold: 1,
            mount: true,
            medal_dialog: false,
            ..UiControl::default()
        };
        assert!(matches!(
            app.apply_control(key(1), configured),
            SubmitResult::Accepted(_)
        ));
        assert!(matches!(app.cycle_auto(key(1)), SubmitResult::Accepted(_)));
        let written = app.port.last_control.expect("the port received settings");
        assert_eq!(written.mode, UiAutoMode::Stand);
        assert_eq!(written.radius, 200);
        assert_eq!(written.hp_percent, 70);
        assert_eq!(written.mp_percent, 15);
        assert!(written.revive_on);
        assert_eq!(written.revive, 1);
        assert_eq!(written.buffs, [true, false, true]);
        assert_eq!(written.item_rank, 3);
        assert_eq!(written.potion_pickup, 2);
        assert_eq!(written.gold, 1);
        assert!(written.mount);
        assert!(!written.medal_dialog);
    }

    #[test]
    fn a_configured_position_is_honoured_but_a_foreign_map_is_not() {
        // Within the map the character is on, the operator's coordinates are the spot. A spot on
        // another map is not: the mod can only work the map it stands on, so arming there would leave
        // auto anchored somewhere it can never reach.
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));
        let mut settled = reading("Alpha", 80);
        settled.map_id = Some(7);
        settled.zone = 3;
        settled.pixel_x = 504;
        settled.pixel_y = 264;
        app.put_reading(key(1), settled);

        let same_map = UiControl {
            mode: UiAutoMode::Stand,
            spot: Some(UiSpot {
                map_id: 7,
                zone: 3,
                pixel_x: 600,
                pixel_y: 700,
            }),
            ..UiControl::default()
        };
        assert!(matches!(
            app.apply_control(key(1), same_map.clone()),
            SubmitResult::Accepted(_)
        ));
        let spot = app
            .port
            .last_control
            .clone()
            .expect("the port received settings")
            .spot
            .expect("the settings carried a spot");
        assert_eq!((spot.pixel_x, spot.pixel_y), (600, 700));

        let foreign_map = UiControl {
            spot: Some(UiSpot {
                map_id: 43,
                zone: 4,
                pixel_x: 228,
                pixel_y: 164,
            }),
            ..same_map
        };
        assert!(matches!(
            app.apply_control(key(1), foreign_map),
            SubmitResult::Accepted(_)
        ));
        let spot = app
            .port
            .last_control
            .clone()
            .expect("the port received settings")
            .spot
            .expect("the settings carried a spot");
        assert_eq!(spot.map_id, 7);
        assert_eq!(spot.zone, 3);
        assert_eq!((spot.pixel_x, spot.pixel_y), (504, 264));
    }

    #[test]
    fn the_engines_confirmed_settings_replace_what_was_typed_at_it() {
        // The engine clamps. The dialog must reopen on what it wrote, not on the rejected value, or
        // the operator would keep seeing a setting the client is not using.
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        let SubmitResult::Accepted(request_id) = app.observe_control(key(1)) else {
            panic!("a settings read must be accepted");
        };
        assert_eq!(app.control(key(1)), UiControl::default());
        app.port.enqueue(UiEvent::RequestFinished {
            request_id,
            result: Ok(UiSuccess::Control {
                row: key(1),
                settings: UiControl {
                    mode: UiAutoMode::Move,
                    radius: 240,
                    item_rank: 4,
                    ..UiControl::default()
                },
            }),
        });
        // Settings are not table data, so absorbing them must not force a repaint that would cost the
        // operator their selection.
        assert!(!app.pump());
        assert_eq!(app.control(key(1)).radius, 240);
        assert_eq!(app.control(key(1)).item_rank, 4);
        // The auto caption follows the engine too, so the button cannot claim a mode nobody wrote.
        assert_eq!(app.auto_mode(key(1)), UiAutoMode::Move);
    }

    #[test]
    fn the_auto_control_cycles_and_disarming_needs_no_spot() {
        let mut app = app_with_rows(vec![row(1, "A", UiAccountStatus::Running)]);
        app.focus_player_row(Some(key(1)));
        let mut settled = reading("Alpha", 80);
        settled.map_id = Some(1);
        app.put_reading(key(1), settled);

        for expected in [UiAutoMode::Stand, UiAutoMode::Move, UiAutoMode::Off] {
            assert!(matches!(app.cycle_auto(key(1)), SubmitResult::Accepted(_)));
            assert_eq!(app.auto_mode(key(1)), expected);
        }
        // Every state has copy, so the button never shows a blank caption.
        for mode in [UiAutoMode::Off, UiAutoMode::Stand, UiAutoMode::Move] {
            assert!(!mode.label().is_empty());
        }

        // Turning it off is always possible, reading or not: the operator must be able to stop.
        app.focus_player_row(None);
        app.focus_player_row(Some(key(1)));
        assert_eq!(app.auto_mode(key(1)), UiAutoMode::Off);
        assert!(matches!(app.cycle_auto(key(1)), SubmitResult::Rejected(_)));
    }

    // Test-only helpers keeping the fake's internals out of the production surface.
    impl<P: AccountPort> AccountApp<P> {
        fn model_mut_apply(&mut self, event: UiEvent) -> bool {
            self.model.apply(event)
        }

        /// Delivers one reading through the ordinary poll path, so no test reaches into the model.
        fn put_reading(&mut self, row: RowKey, info: UiPlayerInfo) {
            let Some(SubmitResult::Accepted(request_id)) = self.observe_focused_player() else {
                panic!("a focused poll must be accepted");
            };
            self.model.apply(UiEvent::RequestFinished {
                request_id,
                result: Ok(UiSuccess::Player {
                    row,
                    info: Some(info),
                }),
            });
        }
    }

    impl FakePort {
        fn poll_count_for_test(&mut self) -> usize {
            self.poll().len()
        }
    }
}

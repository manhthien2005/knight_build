//! Pure logical reconnect-stall authorization sensor (AUTO-RECONNECT-R3C).
//!
//! Detects persistent logical reconnect stalls only when the JVM/game loop and
//! reconnect-status publisher are demonstrably still healthy.
//!
//! Invariant: R3C is strictly pure logical authorization and confirmation.
//! It DOES NOT own a second restart budget, DOES NOT duplicate process stop/spawn
//! logic, and DOES NOT treat a stale last-known reconnect snapshot as sufficient
//! restart evidence.

use crate::health_observer::{HealthObservationEvent, HealthReadCondition};
use crate::reconnect_observer::{ReconnectObservationEvent, ReconnectReadCondition};
use std::time::{Duration, Instant};
use zeus_core::wire::{ReconnectState, ReconnectStatusSnapshot};

/// Supervisor policy thresholds for logical reconnect-stall detection and transport safety.
pub const RECONNECT_TRANSPORT_FRESH_MS: u64 = 10000;
pub const HEALTH_TRANSPORT_FRESH_MS: u64 = 10000;
pub const LOGICAL_LOADING_STALL_MS: u64 = 180000;
pub const LOGICAL_WORLD_SETTLE_STALL_MS: u64 = 120000;
pub const LOGICAL_STALL_CONFIRMATION_POLLS: u32 = 2;

/// Classification of logical stall recovery requests authorized by R3C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalStallRecoveryReason {
    /// LoadingScreen (fu.d) active for >= 180s with healthy progressing JVM and healthy reconnect publisher.
    ReconnectLoadingStall,
    /// GameScreen exists but authoritative world state has failed to settle for >= 120s.
    ReconnectWorldSettleStall,
}

impl LogicalStallRecoveryReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReconnectLoadingStall => "RECONNECT_LOADING_STALL",
            Self::ReconnectWorldSettleStall => "RECONNECT_WORLD_SETTLE_STALL",
        }
    }
}

/// Evaluation result from logical reconnect watchdog on a 2-second supervision tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalWatchdogEvaluation {
    /// No stall detected or strict authorization prerequisites not met (fail-closed).
    NoAction,
    /// Loading threshold reached; armed first poll confirmation.
    ArmingLoadingConfirmation { count: u32 },
    /// WorldSettle threshold reached; armed first poll confirmation.
    ArmingWorldSettleConfirmation { count: u32 },
    /// Logical reconnect stall confirmed (2 consecutive eligible polls).
    RecoveryRequested { reason: LogicalStallRecoveryReason },
}

/// In-memory state tracking logical reconnect stall confirmation for the supervised account.
#[derive(Debug, Clone)]
pub struct LogicalReconnectWatchdog {
    current_process_generation: u64,
    loading_confirmation_count: u32,
    world_settle_confirmation_count: u32,
    last_episode_id: Option<u32>,
    last_state: Option<ReconnectState>,
    last_seen_shared_backoff_epoch: u64,
    last_condition_for_transition_logging: Option<String>,
}

impl Default for LogicalReconnectWatchdog {
    fn default() -> Self {
        Self::new(0)
    }
}

impl LogicalReconnectWatchdog {
    /// Creates a new logical watchdog with clean confirmation state.
    pub fn new(shared_backoff_epoch: u64) -> Self {
        Self {
            current_process_generation: 0,
            loading_confirmation_count: 0,
            world_settle_confirmation_count: 0,
            last_episode_id: None,
            last_state: None,
            last_seen_shared_backoff_epoch: shared_backoff_epoch,
            last_condition_for_transition_logging: None,
        }
    }

    /// Resets per-generation confirmation state on a new successful process spawn.
    pub fn on_successful_spawn(&mut self, new_generation: u64, shared_backoff_epoch: u64) {
        self.current_process_generation = new_generation;
        self.loading_confirmation_count = 0;
        self.world_settle_confirmation_count = 0;
        self.last_episode_id = None;
        self.last_state = None;
        self.last_seen_shared_backoff_epoch = shared_backoff_epoch;
        self.last_condition_for_transition_logging = None;
    }

    /// Completely resets all confirmation state on explicit user stop or retirement.
    pub fn on_explicit_user_stop_or_retirement(&mut self) {
        self.current_process_generation = 0;
        self.loading_confirmation_count = 0;
        self.world_settle_confirmation_count = 0;
        self.last_episode_id = None;
        self.last_state = None;
        self.last_seen_shared_backoff_epoch = 0;
        self.last_condition_for_transition_logging = None;
    }

    /// Returns the currently tracked process generation.
    pub fn current_process_generation(&self) -> u64 {
        self.current_process_generation
    }

    /// Returns the current loading confirmation poll count.
    pub fn loading_confirmation_count(&self) -> u32 {
        self.loading_confirmation_count
    }

    /// Returns the current world-settle confirmation poll count.
    pub fn world_settle_confirmation_count(&self) -> u32 {
        self.world_settle_confirmation_count
    }

    /// Returns the last observed episode ID.
    pub fn last_episode_id(&self) -> Option<u32> {
        self.last_episode_id
    }

    /// Returns the last observed reconnect state.
    pub fn last_state(&self) -> Option<ReconnectState> {
        self.last_state
    }

    /// Returns the last observed shared backoff epoch.
    pub fn last_seen_shared_backoff_epoch(&self) -> u64 {
        self.last_seen_shared_backoff_epoch
    }

    /// Resets confirmation counts and tracked episode/state.
    fn reset_confirmations(&mut self) {
        self.loading_confirmation_count = 0;
        self.world_settle_confirmation_count = 0;
        self.last_episode_id = None;
        self.last_state = None;
    }

    /// Evaluates logical reconnect stall conditions against strict authorization rules.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        account_retiring: bool,
        account_tombstoned: bool,
        desired_state: &str,
        replacement_pending: bool,
        has_process: bool,
        is_alive: bool,
        process_generation: u64,
        health_observer_generation: Option<u64>,
        reconnect_observer_generation: Option<u64>,
        has_health_baseline: bool,
        health_read_condition: HealthReadCondition,
        health_duration_since_progress: Option<Duration>,
        last_health_event: &HealthObservationEvent,
        has_reconnect_baseline: bool,
        reconnect_read_condition: ReconnectReadCondition,
        reconnect_duration_since_progress: Option<Duration>,
        last_reconnect_event: &ReconnectObservationEvent,
        last_valid_reconnect_snapshot: Option<&ReconnectStatusSnapshot>,
        current_reconnect_episode_id: Option<u32>,
        duration_in_current_state: Option<Duration>,
        shared_backoff_epoch: u64,
        _now: Instant,
    ) -> LogicalWatchdogEvaluation {
        // 1. Shared backoff epoch transition check
        // R3C must not carry logical-stall confirmation evidence across the 30-minute external-recovery backoff.
        if shared_backoff_epoch != self.last_seen_shared_backoff_epoch {
            self.loading_confirmation_count = 0;
            self.world_settle_confirmation_count = 0;
            self.last_seen_shared_backoff_epoch = shared_backoff_epoch;
        }

        // 2. Strict account and process lifecycle authorization
        let account_process_ok = !account_retiring
            && !account_tombstoned
            && desired_state == "running"
            && !replacement_pending
            && has_process
            && is_alive
            && health_observer_generation == Some(process_generation)
            && reconnect_observer_generation == Some(process_generation)
            && process_generation == self.current_process_generation;

        if !account_process_ok {
            self.reset_confirmations();
            return LogicalWatchdogEvaluation::NoAction;
        }

        // 3. Strict health evidence (game loop must be demonstrably healthy and progressing)
        let health_freshness_threshold = Duration::from_millis(HEALTH_TRANSPORT_FRESH_MS);
        let health_progress_ok = match health_duration_since_progress {
            Some(d) => d <= health_freshness_threshold,
            None => false,
        };
        let health_event_ok = !matches!(
            last_health_event,
            HealthObservationEvent::MissingFile { .. }
                | HealthObservationEvent::InvalidOrIo { .. }
                | HealthObservationEvent::SequenceRegression { .. }
                | HealthObservationEvent::NotSupervised
        );
        let health_ok = has_health_baseline
            && health_read_condition == HealthReadCondition::Healthy
            && health_progress_ok
            && health_event_ok;

        if !health_ok {
            self.reset_confirmations();
            return LogicalWatchdogEvaluation::NoAction;
        }

        // 4. Strict reconnect transport evidence (reconnect sidecar publisher must be demonstrably healthy)
        let reconnect_freshness_threshold = Duration::from_millis(RECONNECT_TRANSPORT_FRESH_MS);
        let reconnect_progress_ok = match reconnect_duration_since_progress {
            Some(d) => d <= reconnect_freshness_threshold,
            None => false,
        };
        let reconnect_event_ok = matches!(
            last_reconnect_event,
            ReconnectObservationEvent::BaselineSample { .. }
                | ReconnectObservationEvent::Progress { .. }
                | ReconnectObservationEvent::Unchanged { .. }
        );
        let reconnect_ok = has_reconnect_baseline
            && reconnect_read_condition == ReconnectReadCondition::Healthy
            && reconnect_progress_ok
            && reconnect_event_ok;

        if !reconnect_ok {
            self.reset_confirmations();
            return LogicalWatchdogEvaluation::NoAction;
        }

        // 5. Episode evidence and authorized state check
        let snap = match last_valid_reconnect_snapshot {
            Some(s) => s,
            None => {
                self.reset_confirmations();
                return LogicalWatchdogEvaluation::NoAction;
            }
        };

        let episode_ok = snap.active
            && snap.world_seen_before_episode
            && current_reconnect_episode_id == Some(snap.episode_id)
            && duration_in_current_state.is_some();

        if !episode_ok {
            self.reset_confirmations();
            return LogicalWatchdogEvaluation::NoAction;
        }

        // 6. Reset confirmation if episode or state changed
        if self.last_episode_id != Some(snap.episode_id) || self.last_state != Some(snap.state) {
            self.loading_confirmation_count = 0;
            self.world_settle_confirmation_count = 0;
            self.last_episode_id = Some(snap.episode_id);
            self.last_state = Some(snap.state);
        }

        let state_duration = match duration_in_current_state {
            Some(d) => d,
            None => {
                self.reset_confirmations();
                return LogicalWatchdogEvaluation::NoAction;
            }
        };

        // 7. Authorized states evaluation
        match snap.state {
            ReconnectState::Loading => {
                self.world_settle_confirmation_count = 0;
                let loading_threshold = Duration::from_millis(LOGICAL_LOADING_STALL_MS);
                if state_duration >= loading_threshold {
                    self.loading_confirmation_count += 1;
                    if self.loading_confirmation_count >= LOGICAL_STALL_CONFIRMATION_POLLS {
                        LogicalWatchdogEvaluation::RecoveryRequested {
                            reason: LogicalStallRecoveryReason::ReconnectLoadingStall,
                        }
                    } else {
                        LogicalWatchdogEvaluation::ArmingLoadingConfirmation {
                            count: self.loading_confirmation_count,
                        }
                    }
                } else {
                    self.loading_confirmation_count = 0;
                    LogicalWatchdogEvaluation::NoAction
                }
            }
            ReconnectState::WorldSettle => {
                self.loading_confirmation_count = 0;
                let world_settle_threshold = Duration::from_millis(LOGICAL_WORLD_SETTLE_STALL_MS);
                if state_duration >= world_settle_threshold {
                    self.world_settle_confirmation_count += 1;
                    if self.world_settle_confirmation_count >= LOGICAL_STALL_CONFIRMATION_POLLS {
                        LogicalWatchdogEvaluation::RecoveryRequested {
                            reason: LogicalStallRecoveryReason::ReconnectWorldSettleStall,
                        }
                    } else {
                        LogicalWatchdogEvaluation::ArmingWorldSettleConfirmation {
                            count: self.world_settle_confirmation_count,
                        }
                    }
                } else {
                    self.world_settle_confirmation_count = 0;
                    LogicalWatchdogEvaluation::NoAction
                }
            }
            _ => {
                // Explicitly unauthorized states:
                // Idle, NativeWait, Login, Server, Character, Other
                self.loading_confirmation_count = 0;
                self.world_settle_confirmation_count = 0;
                LogicalWatchdogEvaluation::NoAction
            }
        }
    }
}

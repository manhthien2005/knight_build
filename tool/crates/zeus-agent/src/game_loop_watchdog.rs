//! Bounded external game-loop freeze watchdog (AUTO-RECONNECT-R3B).
//!
//! Owns pure detection confirmation and cross-generation restart-budget state.
//! Detects a JVM that is still alive while the Zeus game-loop health sequence
//! has stopped progressing, or a newly spawned JVM that produces no health sample
//! within the startup grace window, then requests bounded process-group recovery.
//!
//! Invariant: R3B is only a game-loop freeze watchdog. It MUST NOT classify
//! reconnect lifecycle states as stalled and MUST NOT use zeus-reconnect.txt
//! to authorize restart.

use crate::health_observer::{HealthObservationEvent, HealthReadCondition};
use std::time::{Duration, Instant};

/// Policy constants for game-loop freeze detection and bounded recovery budget.
pub const HEALTH_FREEZE_THRESHOLD_MS: u64 = 30000;
pub const HEALTH_STARTUP_GRACE_MS: u64 = 90000;
pub const HEALTH_CONFIRMATION_POLLS: u32 = 2;
pub const WATCHDOG_MIN_RECOVERY_INTERVAL_MS: u64 = 60000;
pub const WATCHDOG_BUDGET_WINDOW_MS: u64 = 900000; // 15 minutes
pub const WATCHDOG_MAX_RECOVERIES_PER_WINDOW: u32 = 3;
pub const WATCHDOG_BACKOFF_MS: u64 = 1800000; // 30 minutes

/// Classification of recovery requests produced by the watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogRecoveryReason {
    /// Sequence advancement ceased while JVM remained healthy and unchanged for >= 30s.
    GameLoopFreeze,
    /// New JVM failed to produce its initial health baseline within 90s grace.
    StartupNoHealth,
}

impl WatchdogRecoveryReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GameLoopFreeze => "GAME_LOOP_FREEZE",
            Self::StartupNoHealth => "STARTUP_NO_HEALTH",
        }
    }
}

/// Reason why an otherwise confirmed recovery request is suppressed by budget/rate limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitReason {
    /// Suppressed because last recovery attempt was less than 60s ago.
    MinimumIntervalActive { remaining_ms: u64 },
    /// Suppressed because 3 recoveries occurred in window and 30-minute backoff is active.
    BudgetExhaustedInBackoff { remaining_ms: u64 },
    /// Suppressed because recoveries in window reached budget limit.
    WindowBudgetExhausted,
}

/// Classification of rate limit suppression for transition-only diagnostics and logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitSuppressionClass {
    MinimumInterval,
    Backoff,
    WindowBudgetExhausted,
}

impl RateLimitReason {
    pub fn suppression_class(&self) -> RateLimitSuppressionClass {
        match self {
            RateLimitReason::MinimumIntervalActive { .. } => {
                RateLimitSuppressionClass::MinimumInterval
            }
            RateLimitReason::BudgetExhaustedInBackoff { .. } => RateLimitSuppressionClass::Backoff,
            RateLimitReason::WindowBudgetExhausted => {
                RateLimitSuppressionClass::WindowBudgetExhausted
            }
        }
    }
}

/// Evaluation of a pending replacement retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacementRetryEvaluation {
    /// Not eligible for replacement retry (e.g. not pending, not running, retiring, tombstoned, or live process exists).
    NotEligible,
    /// Bounded budget permits replacement retry attempt.
    Authorized,
    /// Replacement retry is pending and eligible, but suppressed by rate limit or backoff.
    Suppressed(RateLimitReason),
}

/// Result of evaluating the watchdog on a 2-second supervision tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchdogEvaluation {
    /// No anomaly detected or prerequisites not met (fail-closed / normal).
    NoAction,
    /// Normal freeze threshold exceeded; armed first poll confirmation.
    ArmingFreezeConfirmation { count: u32 },
    /// Startup grace exceeded; armed first poll confirmation.
    ArmingStartupConfirmation { count: u32 },
    /// Anomaly confirmed (2 polls) and budget permits process recovery.
    RecoveryRequested { reason: WatchdogRecoveryReason },
    /// Anomaly confirmed (2 polls) but suppressed by recovery budget or backoff.
    SuppressedByRateLimit { reason: RateLimitReason },
}

/// In-memory watchdog state owning per-generation confirmation and cross-generation budget.
#[derive(Debug, Clone)]
pub struct GameLoopWatchdog {
    current_process_generation: u64,
    generation_started_at: Instant,
    freeze_confirmation_count: u32,
    startup_confirmation_count: u32,
    window_started_at: Option<Instant>,
    recoveries_in_window: u32,
    last_recovery_attempt_at: Option<Instant>,
    backoff_until: Option<Instant>,
    last_decision_or_condition: Option<String>,
    replacement_pending: bool,
    last_suppression_class: Option<RateLimitSuppressionClass>,
    needs_post_backoff_reconfirmation: bool,
    long_backoff_epoch: u64,
}

impl Default for GameLoopWatchdog {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl GameLoopWatchdog {
    /// Creates a new watchdog with clean detection and budget state.
    pub fn new(now: Instant) -> Self {
        Self {
            current_process_generation: 0,
            generation_started_at: now,
            freeze_confirmation_count: 0,
            startup_confirmation_count: 0,
            window_started_at: None,
            recoveries_in_window: 0,
            last_recovery_attempt_at: None,
            backoff_until: None,
            last_decision_or_condition: None,
            replacement_pending: false,
            last_suppression_class: None,
            needs_post_backoff_reconfirmation: false,
            long_backoff_epoch: 0,
        }
    }

    /// Marks that a watchdog-owned stop reached ConfirmedStopped and replacement is pending.
    pub fn mark_replacement_pending(&mut self) {
        self.replacement_pending = true;
    }

    /// Returns whether a watchdog replacement JVM is currently pending.
    pub fn replacement_pending(&self) -> bool {
        self.replacement_pending
    }

    /// Checks whether a rate-limit suppression transition should be logged.
    ///
    /// Returns true only on entering suppression or transitioning into a different suppression class,
    /// suppressing repetitive logs on every 2-second tick while remaining-ms counts down.
    pub fn should_log_rate_limit_transition(&mut self, reason: &RateLimitReason) -> bool {
        let class = reason.suppression_class();
        if self.last_suppression_class == Some(class) {
            false
        } else {
            self.last_suppression_class = Some(class);
            true
        }
    }

    /// Clears rate-limit suppression transition tracking when exiting rate-limiting.
    pub fn clear_rate_limit_transition(&mut self) {
        self.last_suppression_class = None;
    }

    /// Resets per-generation detection state on a new successful process spawn.
    ///
    /// CRITICAL: Clears `replacement_pending` and per-generation confirmation state, but
    /// PRESERVES cross-generation restart budget (`recoveries_in_window`, `window_started_at`,
    /// `last_recovery_attempt_at`, `backoff_until`).
    pub fn on_successful_spawn(&mut self, new_generation: u64, now: Instant) {
        self.current_process_generation = new_generation;
        self.generation_started_at = now;
        self.freeze_confirmation_count = 0;
        self.startup_confirmation_count = 0;
        self.last_decision_or_condition = None;
        self.replacement_pending = false;
        self.last_suppression_class = None;
        self.needs_post_backoff_reconfirmation = false;
    }

    /// Completely resets all detection and budget state on explicit user stop or retirement.
    pub fn on_explicit_user_stop_or_retirement(&mut self) {
        self.current_process_generation = 0;
        self.generation_started_at = Instant::now();
        self.freeze_confirmation_count = 0;
        self.startup_confirmation_count = 0;
        self.window_started_at = None;
        self.recoveries_in_window = 0;
        self.last_recovery_attempt_at = None;
        self.backoff_until = None;
        self.last_decision_or_condition = None;
        self.replacement_pending = false;
        self.last_suppression_class = None;
        self.needs_post_backoff_reconfirmation = false;
        self.long_backoff_epoch = 0;
    }

    /// Evaluates whether a pending replacement retry is authorized under the watchdog budget.
    pub fn evaluate_replacement_retry(
        &mut self,
        account_retiring: bool,
        account_tombstoned: bool,
        desired_state: &str,
        has_live_process: bool,
        now: Instant,
    ) -> ReplacementRetryEvaluation {
        if !self.replacement_pending
            || account_retiring
            || account_tombstoned
            || desired_state != "running"
            || has_live_process
        {
            return ReplacementRetryEvaluation::NotEligible;
        }

        match self.check_budget(now) {
            Ok(()) => {
                self.last_suppression_class = None;
                ReplacementRetryEvaluation::Authorized
            }
            Err(reason) => ReplacementRetryEvaluation::Suppressed(reason),
        }
    }

    /// Arms watchdog recovery budget accounting immediately BEFORE calling `process_unix::stop()`.
    ///
    /// Increments `recoveries_in_window`, records `last_recovery_attempt_at`, and arms
    /// 30-minute `backoff_until` before the 3rd action.
    pub fn arm_recovery_attempt(&mut self, now: Instant) {
        self.last_recovery_attempt_at = Some(now);

        // Window maintenance
        if let Some(win_start) = self.window_started_at {
            if now.duration_since(win_start) >= Duration::from_millis(WATCHDOG_BUDGET_WINDOW_MS)
                && self.backoff_until.is_none()
            {
                self.window_started_at = Some(now);
                self.recoveries_in_window = 0;
            }
        } else {
            self.window_started_at = Some(now);
            self.recoveries_in_window = 0;
        }

        self.recoveries_in_window += 1;

        if self.recoveries_in_window >= WATCHDOG_MAX_RECOVERIES_PER_WINDOW {
            self.backoff_until = Some(now + Duration::from_millis(WATCHDOG_BACKOFF_MS));
        }

        // Reset confirmation counts after arming
        self.freeze_confirmation_count = 0;
        self.startup_confirmation_count = 0;
        self.needs_post_backoff_reconfirmation = false;
    }

    /// Returns the currently tracked process generation.
    pub fn current_process_generation(&self) -> u64 {
        self.current_process_generation
    }

    /// Returns the number of recovery attempts executed in the current budget window.
    pub fn recoveries_in_window(&self) -> u32 {
        self.recoveries_in_window
    }

    /// Returns whether the watchdog is currently in the 30-minute backoff period.
    pub fn is_in_backoff(&mut self, now: Instant) -> bool {
        self.maintain_budget_window(now);
        self.backoff_until.is_some()
    }

    /// Returns the monotonically increasing epoch of 30-minute long-backoff expirations.
    pub fn long_backoff_epoch(&self) -> u64 {
        self.long_backoff_epoch
    }

    /// Maintains window expiration and backoff expiration based on current monotonic time.
    pub fn maintain_budget_window(&mut self, now: Instant) {
        if let Some(backoff_deadline) = self.backoff_until {
            if now >= backoff_deadline {
                // Backoff expired: clear budget cycle, increment shared epoch, and arm post-backoff anomaly reconfirmation
                self.backoff_until = None;
                self.recoveries_in_window = 0;
                self.window_started_at = None;
                self.needs_post_backoff_reconfirmation = true;
                self.long_backoff_epoch += 1;
            }
        } else if let Some(win_start) = self.window_started_at {
            if now.duration_since(win_start) >= Duration::from_millis(WATCHDOG_BUDGET_WINDOW_MS) {
                // Budget window expired without backoff
                self.recoveries_in_window = 0;
                self.window_started_at = None;
            }
        }
    }

    /// Checks if budget allows an authorized recovery attempt.
    pub fn check_budget(&mut self, now: Instant) -> Result<(), RateLimitReason> {
        self.maintain_budget_window(now);

        // 1. Backoff active?
        if let Some(backoff_deadline) = self.backoff_until {
            let remaining_ms = backoff_deadline.saturating_duration_since(now).as_millis() as u64;
            return Err(RateLimitReason::BudgetExhaustedInBackoff { remaining_ms });
        }

        // 2. Minimum interval (60s) enforced?
        if let Some(last_attempt) = self.last_recovery_attempt_at {
            let min_interval = Duration::from_millis(WATCHDOG_MIN_RECOVERY_INTERVAL_MS);
            if let Some(elapsed) = now.checked_duration_since(last_attempt) {
                if elapsed < min_interval {
                    let remaining_ms = (min_interval - elapsed).as_millis() as u64;
                    return Err(RateLimitReason::MinimumIntervalActive { remaining_ms });
                }
            }
        }

        // 3. Max recoveries per window exceeded?
        if self.recoveries_in_window >= WATCHDOG_MAX_RECOVERIES_PER_WINDOW {
            return Err(RateLimitReason::WindowBudgetExhausted);
        }

        Ok(())
    }

    /// Evaluates game-loop freeze and startup health progress for the supervised account.
    ///
    /// Updates confirmation counters and applies budget checks upon confirmation.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        account_retiring: bool,
        account_tombstoned: bool,
        desired_state: &str,
        has_process: bool,
        is_alive: bool,
        process_generation: u64,
        observer_generation: Option<u64>,
        has_baseline: bool,
        duration_since_last_progress: Option<Duration>,
        read_condition: HealthReadCondition,
        last_event: &HealthObservationEvent,
        now: Instant,
    ) -> WatchdogEvaluation {
        self.maintain_budget_window(now);

        // Discard any anomaly confirmation evidence accumulated before or during long backoff.
        // For a live running JVM, long-backoff expiry requires two fresh consecutive eligible polls.
        if self.needs_post_backoff_reconfirmation {
            self.freeze_confirmation_count = 0;
            self.startup_confirmation_count = 0;
            self.needs_post_backoff_reconfirmation = false;
        }

        // Explicit fail-closed prerequisites
        if account_retiring
            || account_tombstoned
            || desired_state != "running"
            || !has_process
            || !is_alive
            || observer_generation != Some(process_generation)
            || process_generation != self.current_process_generation
        {
            self.freeze_confirmation_count = 0;
            self.startup_confirmation_count = 0;
            return WatchdogEvaluation::NoAction;
        }

        // Branch 1: Has valid baseline -> evaluate normal freeze
        if has_baseline {
            self.startup_confirmation_count = 0;

            match last_event {
                HealthObservationEvent::Progress { .. }
                | HealthObservationEvent::BaselineSample { .. } => {
                    self.freeze_confirmation_count = 0;
                    self.last_decision_or_condition = None;
                    WatchdogEvaluation::NoAction
                }
                HealthObservationEvent::MissingFile { .. }
                | HealthObservationEvent::InvalidOrIo { .. }
                | HealthObservationEvent::SequenceRegression { .. }
                | HealthObservationEvent::NotSupervised => {
                    // Fail-closed on missing/malformed/regression
                    self.freeze_confirmation_count = 0;
                    WatchdogEvaluation::NoAction
                }
                HealthObservationEvent::Unchanged { .. } => {
                    let freeze_threshold = Duration::from_millis(HEALTH_FREEZE_THRESHOLD_MS);
                    let duration_ok = duration_since_last_progress
                        .map(|d| d >= freeze_threshold)
                        .unwrap_or(false);

                    if read_condition == HealthReadCondition::Healthy && duration_ok {
                        self.freeze_confirmation_count += 1;
                        if self.freeze_confirmation_count >= HEALTH_CONFIRMATION_POLLS {
                            // Confirmed freeze! Check budget
                            match self.check_budget(now) {
                                Ok(()) => {
                                    self.last_suppression_class = None;
                                    WatchdogEvaluation::RecoveryRequested {
                                        reason: WatchdogRecoveryReason::GameLoopFreeze,
                                    }
                                }
                                Err(reason) => WatchdogEvaluation::SuppressedByRateLimit { reason },
                            }
                        } else {
                            self.last_suppression_class = None;
                            WatchdogEvaluation::ArmingFreezeConfirmation {
                                count: self.freeze_confirmation_count,
                            }
                        }
                    } else {
                        self.freeze_confirmation_count = 0;
                        WatchdogEvaluation::NoAction
                    }
                }
            }
        } else {
            // Branch 2: No valid baseline yet -> evaluate startup grace
            self.freeze_confirmation_count = 0;

            let startup_grace = Duration::from_millis(HEALTH_STARTUP_GRACE_MS);
            let age_ok = now.duration_since(self.generation_started_at) >= startup_grace;

            // Only AwaitingFirstSample or MissingFile qualify for startup recovery.
            // InvalidOrIo and SequenceRegression fail-closed.
            let condition_qualifies = matches!(
                read_condition,
                HealthReadCondition::AwaitingFirstSample | HealthReadCondition::MissingFile
            ) && !matches!(
                last_event,
                HealthObservationEvent::InvalidOrIo { .. }
                    | HealthObservationEvent::SequenceRegression { .. }
            );

            if age_ok && condition_qualifies {
                self.startup_confirmation_count += 1;
                if self.startup_confirmation_count >= HEALTH_CONFIRMATION_POLLS {
                    match self.check_budget(now) {
                        Ok(()) => {
                            self.last_suppression_class = None;
                            WatchdogEvaluation::RecoveryRequested {
                                reason: WatchdogRecoveryReason::StartupNoHealth,
                            }
                        }
                        Err(reason) => WatchdogEvaluation::SuppressedByRateLimit { reason },
                    }
                } else {
                    self.last_suppression_class = None;
                    WatchdogEvaluation::ArmingStartupConfirmation {
                        count: self.startup_confirmation_count,
                    }
                }
            } else {
                if matches!(
                    last_event,
                    HealthObservationEvent::InvalidOrIo { .. }
                        | HealthObservationEvent::SequenceRegression { .. }
                ) {
                    self.startup_confirmation_count = 0;
                }
                WatchdogEvaluation::NoAction
            }
        }
    }
}

//! Strictly observe-only game-loop progress sensor (AUTO-RECONNECT-R1B).
//!
//! Tracks health sequence advancement for the currently supervised JVM generation
//! using the agent's local monotonic `Instant` clock.
//!
//! Invariant: R1B is strictly observe-only. It NEVER concludes that a JVM is frozen,
//! NEVER calls process stop or restart, and NEVER pushes health status to cloud runtime.

use std::path::Path;
use std::time::{Duration, Instant};
use zeus_core::wire::{read_health, HealthError, HealthScreen, HealthSnapshot};

/// Observable health read conditions for transition-only diagnostics and logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthReadCondition {
    /// No process generation is currently supervised.
    Stopped,
    /// A new process generation was started and is awaiting its first valid health sample.
    AwaitingFirstSample,
    /// Health samples are being read cleanly without errors.
    Healthy,
    /// Transient observation gap: the health file does not exist on disk.
    MissingFile,
    /// Transient read error: file was malformed or encountered an I/O error.
    InvalidOrIo,
    /// Sequence regression within the same process generation (observation anomaly).
    SequenceRegression,
}

/// Events resulting from an observation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthObservationEvent {
    /// Observer is not currently supervising a process generation.
    NotSupervised,
    /// The first valid health sample of a new process generation established the baseline.
    BaselineSample {
        sequence: u64,
        screen: HealthScreen,
        dialog_open: bool,
        native_disconnect: bool,
        condition_changed: bool,
    },
    /// Sequence advanced (valid game-loop progress).
    Progress {
        sequence: u64,
        screen_changed: Option<(HealthScreen, HealthScreen)>,
        disconnect_changed: Option<(bool, bool)>,
        dialog_changed: Option<(bool, bool)>,
        condition_changed: bool,
    },
    /// Sequence unchanged (normal between-tick state; not classified as frozen).
    Unchanged {
        sequence: u64,
        condition_changed: bool,
    },
    /// Sequence regression in the same generation (anomaly; not counted as progress).
    SequenceRegression {
        last_sequence: u64,
        seen_sequence: u64,
        condition_changed: bool,
    },
    /// Health file temporarily absent (observation gap).
    MissingFile { condition_changed: bool },
    /// Malformed file or I/O error during health read.
    InvalidOrIo {
        error: String,
        condition_changed: bool,
    },
}

/// In-memory observer tracking game-loop progress across explicit JVM generations.
#[derive(Debug, Clone)]
pub struct HealthObserver {
    current_process_generation: Option<u64>,
    current_pid: Option<u32>,
    last_valid_snapshot: Option<HealthSnapshot>,
    last_sequence: Option<u64>,
    last_progress_at: Option<Instant>,
    last_read_condition: HealthReadCondition,
}

impl Default for HealthObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthObserver {
    /// Creates a new observer in stopped state.
    pub fn new() -> Self {
        Self {
            current_process_generation: None,
            current_pid: None,
            last_valid_snapshot: None,
            last_sequence: None,
            last_progress_at: None,
            last_read_condition: HealthReadCondition::Stopped,
        }
    }

    /// Resets the observer for a newly spawned JVM generation.
    ///
    /// Clears any state from the previous generation and begins awaiting
    /// the first valid health sample.
    pub fn reset_for_new_generation(&mut self, generation: u64, pid: Option<u32>) {
        self.current_process_generation = Some(generation);
        self.current_pid = pid;
        self.last_valid_snapshot = None;
        self.last_sequence = None;
        self.last_progress_at = None;
        self.last_read_condition = HealthReadCondition::AwaitingFirstSample;
    }

    /// Resets the observer to stopped/no-generation state when a process is authoritatively stopped.
    pub fn reset_stopped(&mut self) {
        self.current_process_generation = None;
        self.current_pid = None;
        self.last_valid_snapshot = None;
        self.last_sequence = None;
        self.last_progress_at = None;
        self.last_read_condition = HealthReadCondition::Stopped;
    }

    /// Returns the currently supervised process generation, if any.
    pub fn current_process_generation(&self) -> Option<u64> {
        self.current_process_generation
    }

    /// Returns the PID associated with the current generation, if known.
    pub fn current_pid(&self) -> Option<u32> {
        self.current_pid
    }

    /// Returns the latest valid health snapshot.
    pub fn last_valid_snapshot(&self) -> Option<&HealthSnapshot> {
        self.last_valid_snapshot.as_ref()
    }

    /// Returns the last valid sequence number observed for the current generation.
    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    /// Returns the monotonic instant of the last observed progress.
    pub fn last_progress_at(&self) -> Option<Instant> {
        self.last_progress_at
    }

    /// Returns the last read condition.
    pub fn last_read_condition(&self) -> HealthReadCondition {
        self.last_read_condition
    }

    /// Returns elapsed time since last observed progress, if any progress has been recorded.
    ///
    /// Pure query method for future watchdog use; does NOT trigger any action or recovery in R1B.
    pub fn duration_since_last_progress(&self, now: Instant) -> Option<Duration> {
        self.last_progress_at
            .and_then(|progress_time| now.checked_duration_since(progress_time))
    }

    /// Observes a health read result for the current generation using a given local timestamp.
    ///
    /// Strictly observe-only: preserves prior good state upon transient missing or malformed reads.
    pub fn observe_reading(
        &mut self,
        reading: Result<Option<HealthSnapshot>, HealthError>,
        now: Instant,
    ) -> HealthObservationEvent {
        if self.current_process_generation.is_none() {
            return HealthObservationEvent::NotSupervised;
        }

        match reading {
            Err(err) => {
                let condition_changed =
                    self.last_read_condition != HealthReadCondition::InvalidOrIo;
                self.last_read_condition = HealthReadCondition::InvalidOrIo;
                HealthObservationEvent::InvalidOrIo {
                    error: err.to_string(),
                    condition_changed,
                }
            }
            Ok(None) => {
                let condition_changed =
                    self.last_read_condition != HealthReadCondition::MissingFile;
                self.last_read_condition = HealthReadCondition::MissingFile;
                HealthObservationEvent::MissingFile { condition_changed }
            }
            Ok(Some(sample)) => {
                match self.last_sequence {
                    None => {
                        // First valid sample for this process generation
                        let condition_changed =
                            self.last_read_condition != HealthReadCondition::Healthy;
                        self.last_read_condition = HealthReadCondition::Healthy;
                        self.last_sequence = Some(sample.sequence);
                        self.last_progress_at = Some(now);

                        let event = HealthObservationEvent::BaselineSample {
                            sequence: sample.sequence,
                            screen: sample.screen,
                            dialog_open: sample.dialog_open,
                            native_disconnect: sample.native_disconnect,
                            condition_changed,
                        };
                        self.last_valid_snapshot = Some(sample);
                        event
                    }
                    Some(last_seq) if sample.sequence > last_seq => {
                        // Valid sequence advance -> game loop progress!
                        let condition_changed =
                            self.last_read_condition != HealthReadCondition::Healthy;
                        self.last_read_condition = HealthReadCondition::Healthy;
                        self.last_sequence = Some(sample.sequence);
                        self.last_progress_at = Some(now);

                        let (screen_changed, disconnect_changed, dialog_changed) =
                            if let Some(ref prev) = self.last_valid_snapshot {
                                (
                                    if prev.screen != sample.screen {
                                        Some((prev.screen, sample.screen))
                                    } else {
                                        None
                                    },
                                    if prev.native_disconnect != sample.native_disconnect {
                                        Some((prev.native_disconnect, sample.native_disconnect))
                                    } else {
                                        None
                                    },
                                    if prev.dialog_open != sample.dialog_open {
                                        Some((prev.dialog_open, sample.dialog_open))
                                    } else {
                                        None
                                    },
                                )
                            } else {
                                (None, None, None)
                            };

                        self.last_valid_snapshot = Some(sample.clone());
                        HealthObservationEvent::Progress {
                            sequence: sample.sequence,
                            screen_changed,
                            disconnect_changed,
                            dialog_changed,
                            condition_changed,
                        }
                    }
                    Some(last_seq) if sample.sequence == last_seq => {
                        // Sequence unchanged -> keep latest valid snapshot, do not update progress time.
                        let condition_changed =
                            self.last_read_condition != HealthReadCondition::Healthy;
                        self.last_read_condition = HealthReadCondition::Healthy;
                        HealthObservationEvent::Unchanged {
                            sequence: last_seq,
                            condition_changed,
                        }
                    }
                    Some(last_seq) => {
                        // sample.sequence < last_seq: sequence regression in same generation
                        let condition_changed =
                            self.last_read_condition != HealthReadCondition::SequenceRegression;
                        self.last_read_condition = HealthReadCondition::SequenceRegression;
                        HealthObservationEvent::SequenceRegression {
                            last_sequence: last_seq,
                            seen_sequence: sample.sequence,
                            condition_changed,
                        }
                    }
                }
            }
        }
    }

    /// Reads health from the account home directory and logs transitions cleanly.
    ///
    /// Enforces the logging policy:
    /// - Does not log every 2-second healthy sample.
    /// - Logs the first valid health sample of a new generation.
    /// - Logs screen and native_disconnect transitions.
    /// - Logs condition transitions (missing file, invalid read, sequence regression) once per condition.
    /// - Does NOT emit frozen or restart conclusions.
    pub fn observe_account_home(
        &mut self,
        home: &Path,
        account_id: &str,
        now: Instant,
    ) -> HealthObservationEvent {
        let reading = read_health(home);
        let event = self.observe_reading(reading, now);
        let generation = self.current_process_generation.unwrap_or(0);
        let pid_opt = self.current_pid;

        match &event {
            HealthObservationEvent::NotSupervised => {}
            HealthObservationEvent::BaselineSample {
                sequence,
                screen,
                dialog_open,
                native_disconnect,
                ..
            } => {
                eprintln!(
                    "[health] account={account_id} gen={generation} pid={pid_opt:?} baseline established: seq={sequence} screen={screen:?} dialog={dialog_open} disconnect={native_disconnect}"
                );
            }
            HealthObservationEvent::Progress {
                sequence,
                screen_changed,
                disconnect_changed,
                dialog_changed: _,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} recovered healthy read condition at seq={sequence}"
                    );
                }
                if let Some((old_s, new_s)) = screen_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} screen transition: {old_s:?} -> {new_s:?}"
                    );
                }
                if let Some((old_d, new_d)) = disconnect_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} native_disconnect transition: {old_d} -> {new_d}"
                    );
                }
            }
            HealthObservationEvent::Unchanged {
                sequence,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} recovered healthy read condition (seq={sequence})"
                    );
                }
                // Do not log routine unchanged sequence
            }
            HealthObservationEvent::SequenceRegression {
                last_sequence,
                seen_sequence,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} sequence regression anomaly: last={last_sequence} seen={seen_sequence}"
                    );
                }
            }
            HealthObservationEvent::MissingFile { condition_changed } => {
                if *condition_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} health file missing (observation gap)"
                    );
                }
            }
            HealthObservationEvent::InvalidOrIo {
                error,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[health] account={account_id} gen={generation} health read error: {error}"
                    );
                }
            }
        }

        event
    }
}

//! Strictly observe-only reconnect lifecycle progress and duration observer (AUTO-RECONNECT-R3A).
//!
//! Tracks reconnect state duration across an explicit JVM process generation
//! using the agent's local monotonic `Instant` clock.
//!
//! Invariant: R3A is strictly observe-only. It NEVER stops, restarts, reconciles,
//! or mutates process state or cloud runtime state based on reconnect status.

use std::path::Path;
use std::time::{Duration, Instant};
use zeus_core::wire::{
    read_reconnect_status, ReconnectState, ReconnectStatusError, ReconnectStatusSnapshot,
};

/// Observable reconnect read conditions for transition-only diagnostics and logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectReadCondition {
    /// No process generation is currently supervised.
    Stopped,
    /// A new process generation was started and is awaiting its first valid reconnect sample.
    AwaitingFirstSample,
    /// Reconnect samples are being read cleanly without errors.
    Healthy,
    /// Transient observation gap: the reconnect file does not exist on disk.
    MissingFile,
    /// Transient read error: file was malformed or encountered an I/O error.
    InvalidOrIo,
    /// Sequence regression within the same process generation (observation anomaly).
    SequenceRegression,
}

/// Events resulting from an observation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectObservationEvent {
    /// Observer is not currently supervising a process generation.
    NotSupervised,
    /// The first valid reconnect sample of a new process generation established the baseline.
    BaselineSample {
        sequence: u64,
        episode_id: u32,
        active: bool,
        state: ReconnectState,
        transitions: u32,
        world_seen_before_episode: bool,
        condition_changed: bool,
    },
    /// Sequence advanced (valid observation progress).
    Progress {
        sequence: u64,
        episode_changed: Option<(u32, u32)>,
        active_changed: Option<(bool, bool)>,
        state_changed: Option<(ReconnectState, ReconnectState)>,
        transitions_changed: Option<(u32, u32)>,
        condition_changed: bool,
    },
    /// Sequence unchanged (normal between-tick sample).
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
    /// Reconnect file temporarily absent (observation gap).
    MissingFile { condition_changed: bool },
    /// Malformed file or I/O error during reconnect status read.
    InvalidOrIo {
        error: String,
        condition_changed: bool,
    },
}

/// In-memory observer tracking reconnect lifecycle duration across explicit JVM generations.
#[derive(Debug, Clone)]
pub struct ReconnectObserver {
    current_process_generation: Option<u64>,
    current_pid: Option<u32>,
    last_valid_snapshot: Option<ReconnectStatusSnapshot>,
    last_sequence: Option<u64>,
    current_episode_id: Option<u32>,
    current_state: Option<ReconnectState>,
    state_since: Option<Instant>,
    last_read_condition: ReconnectReadCondition,
}

impl Default for ReconnectObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl ReconnectObserver {
    /// Creates a new observer in stopped state.
    pub fn new() -> Self {
        Self {
            current_process_generation: None,
            current_pid: None,
            last_valid_snapshot: None,
            last_sequence: None,
            current_episode_id: None,
            current_state: None,
            state_since: None,
            last_read_condition: ReconnectReadCondition::Stopped,
        }
    }

    /// Resets the observer for a newly spawned JVM generation.
    ///
    /// Clears any state from the previous generation and begins awaiting
    /// the first valid reconnect sample.
    pub fn reset_for_new_generation(&mut self, generation: u64, pid: Option<u32>) {
        self.current_process_generation = Some(generation);
        self.current_pid = pid;
        self.last_valid_snapshot = None;
        self.last_sequence = None;
        self.current_episode_id = None;
        self.current_state = None;
        self.state_since = None;
        self.last_read_condition = ReconnectReadCondition::AwaitingFirstSample;
    }

    /// Resets the observer to stopped/no-generation state when a process is authoritatively stopped or retired.
    pub fn reset_stopped(&mut self) {
        self.current_process_generation = None;
        self.current_pid = None;
        self.last_valid_snapshot = None;
        self.last_sequence = None;
        self.current_episode_id = None;
        self.current_state = None;
        self.state_since = None;
        self.last_read_condition = ReconnectReadCondition::Stopped;
    }

    /// Returns the currently supervised process generation, if any.
    pub fn current_process_generation(&self) -> Option<u64> {
        self.current_process_generation
    }

    /// Returns the PID associated with the current generation, if known.
    pub fn current_pid(&self) -> Option<u32> {
        self.current_pid
    }

    /// Returns the latest valid reconnect status snapshot.
    pub fn last_valid_snapshot(&self) -> Option<&ReconnectStatusSnapshot> {
        self.last_valid_snapshot.as_ref()
    }

    /// Returns the last valid sequence number observed for the current generation.
    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    /// Returns the monotonic instant when the current reconnect state was entered.
    pub fn state_since(&self) -> Option<Instant> {
        self.state_since
    }

    /// Returns elapsed duration in the current reconnect state, if known.
    ///
    /// Pure query method for duration measurement; does NOT trigger any action or recovery in R3A.
    pub fn duration_in_current_state(&self, now: Instant) -> Option<Duration> {
        self.state_since
            .and_then(|since| now.checked_duration_since(since))
    }

    /// Returns the current reconnect state, if any valid sample has been observed.
    pub fn current_state(&self) -> Option<ReconnectState> {
        self.current_state
    }

    /// Returns the current reconnect episode ID, if any valid sample has been observed.
    pub fn current_episode_id(&self) -> Option<u32> {
        self.current_episode_id
    }

    /// Returns the last read condition.
    pub fn last_read_condition(&self) -> ReconnectReadCondition {
        self.last_read_condition
    }

    /// Observes a reconnect read result for the current generation using a given local timestamp.
    ///
    /// Strictly observe-only: preserves prior good state upon transient missing or malformed reads.
    pub fn observe_reading(
        &mut self,
        reading: Result<Option<ReconnectStatusSnapshot>, ReconnectStatusError>,
        now: Instant,
    ) -> ReconnectObservationEvent {
        if self.current_process_generation.is_none() {
            return ReconnectObservationEvent::NotSupervised;
        }

        match reading {
            Err(err) => {
                let condition_changed =
                    self.last_read_condition != ReconnectReadCondition::InvalidOrIo;
                self.last_read_condition = ReconnectReadCondition::InvalidOrIo;
                ReconnectObservationEvent::InvalidOrIo {
                    error: err.to_string(),
                    condition_changed,
                }
            }
            Ok(None) => {
                let condition_changed =
                    self.last_read_condition != ReconnectReadCondition::MissingFile;
                self.last_read_condition = ReconnectReadCondition::MissingFile;
                ReconnectObservationEvent::MissingFile { condition_changed }
            }
            Ok(Some(sample)) => {
                match self.last_sequence {
                    None => {
                        // First valid sample for this process generation establishes baseline
                        let condition_changed =
                            self.last_read_condition != ReconnectReadCondition::Healthy;
                        self.last_read_condition = ReconnectReadCondition::Healthy;
                        self.last_sequence = Some(sample.sequence);
                        self.current_episode_id = Some(sample.episode_id);
                        self.current_state = Some(sample.state);
                        self.state_since = Some(now);

                        let event = ReconnectObservationEvent::BaselineSample {
                            sequence: sample.sequence,
                            episode_id: sample.episode_id,
                            active: sample.active,
                            state: sample.state,
                            transitions: sample.transitions,
                            world_seen_before_episode: sample.world_seen_before_episode,
                            condition_changed,
                        };
                        self.last_valid_snapshot = Some(sample);
                        event
                    }
                    Some(last_seq) if sample.sequence > last_seq => {
                        // Valid sequence advance -> check for state/episode/active transition
                        let condition_changed =
                            self.last_read_condition != ReconnectReadCondition::Healthy;
                        self.last_read_condition = ReconnectReadCondition::Healthy;

                        let prev_episode = self.current_episode_id;
                        let prev_active = self.last_valid_snapshot.as_ref().map(|s| s.active);
                        let prev_state = self.current_state;
                        let prev_transitions = self.last_valid_snapshot.as_ref().map(|s| s.transitions);

                        let episode_changed = if prev_episode != Some(sample.episode_id) {
                            prev_episode.map(|prev| (prev, sample.episode_id))
                        } else {
                            None
                        };
                        let active_changed = if prev_active != Some(sample.active) {
                            prev_active.map(|prev| (prev, sample.active))
                        } else {
                            None
                        };
                        let state_changed = if prev_state != Some(sample.state) {
                            prev_state.map(|prev| (prev, sample.state))
                        } else {
                            None
                        };
                        let transitions_changed = if prev_transitions != Some(sample.transitions) {
                            prev_transitions.map(|prev| (prev, sample.transitions))
                        } else {
                            None
                        };

                        if episode_changed.is_some() || active_changed.is_some() || state_changed.is_some() {
                            // Any lifecycle state transition resets state_since to local now
                            self.state_since = Some(now);
                            self.current_episode_id = Some(sample.episode_id);
                            self.current_state = Some(sample.state);
                        }
                        // Note: If episode/active/state did not change, state_since is deliberately
                        // NOT reset merely because seq advanced! This preserves logical stall duration measurement.

                        self.last_sequence = Some(sample.sequence);
                        self.last_valid_snapshot = Some(sample.clone());

                        ReconnectObservationEvent::Progress {
                            sequence: sample.sequence,
                            episode_changed,
                            active_changed,
                            state_changed,
                            transitions_changed,
                            condition_changed,
                        }
                    }
                    Some(last_seq) if sample.sequence == last_seq => {
                        // Sequence unchanged -> keep latest valid snapshot, do not update state_since
                        let condition_changed =
                            self.last_read_condition != ReconnectReadCondition::Healthy;
                        self.last_read_condition = ReconnectReadCondition::Healthy;
                        ReconnectObservationEvent::Unchanged {
                            sequence: last_seq,
                            condition_changed,
                        }
                    }
                    Some(last_seq) => {
                        // sample.sequence < last_seq: sequence regression within same generation
                        let condition_changed =
                            self.last_read_condition != ReconnectReadCondition::SequenceRegression;
                        self.last_read_condition = ReconnectReadCondition::SequenceRegression;
                        ReconnectObservationEvent::SequenceRegression {
                            last_sequence: last_seq,
                            seen_sequence: sample.sequence,
                            condition_changed,
                        }
                    }
                }
            }
        }
    }

    /// Reads reconnect status from the account home directory and logs transitions cleanly.
    ///
    /// Enforces the logging policy:
    /// - Does not log every 2-second healthy sample.
    /// - Logs first valid reconnect-status baseline for a generation.
    /// - Logs episode open/close observation transitions.
    /// - Logs reconnect state transitions once.
    /// - Logs missing/invalid/regression condition transitions once.
    /// - Does NOT emit stall or restart action conclusions.
    pub fn observe_account_home(
        &mut self,
        home: &Path,
        account_id: &str,
        now: Instant,
    ) -> ReconnectObservationEvent {
        let reading = read_reconnect_status(home);
        let event = self.observe_reading(reading, now);
        let generation = self.current_process_generation.unwrap_or(0);
        let pid_opt = self.current_pid;

        match &event {
            ReconnectObservationEvent::NotSupervised => {}
            ReconnectObservationEvent::BaselineSample {
                sequence,
                episode_id,
                active,
                state,
                transitions,
                world_seen_before_episode,
                ..
            } => {
                eprintln!(
                    "[reconnect] account={account_id} gen={generation} pid={pid_opt:?} baseline established: seq={sequence} ep={episode_id} active={active} state={state:?} trans={transitions} world_before={world_seen_before_episode}"
                );
            }
            ReconnectObservationEvent::Progress {
                sequence,
                episode_changed,
                active_changed,
                state_changed,
                transitions_changed: _,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} recovered healthy read condition at seq={sequence}"
                    );
                }
                if let Some((old_act, new_act)) = active_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} active transition: {old_act} -> {new_act}"
                    );
                }
                if let Some((old_ep, new_ep)) = episode_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} episode transition: {old_ep} -> {new_ep}"
                    );
                }
                if let Some((old_s, new_s)) = state_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} state transition: {old_s:?} -> {new_s:?}"
                    );
                }
            }
            ReconnectObservationEvent::Unchanged {
                sequence,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} recovered healthy read condition (seq={sequence})"
                    );
                }
                // Do not log routine unchanged sequence
            }
            ReconnectObservationEvent::SequenceRegression {
                last_sequence,
                seen_sequence,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} sequence regression anomaly: last={last_sequence} seen={seen_sequence}"
                    );
                }
            }
            ReconnectObservationEvent::MissingFile { condition_changed } => {
                if *condition_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} reconnect file missing (observation gap)"
                    );
                }
            }
            ReconnectObservationEvent::InvalidOrIo {
                error,
                condition_changed,
            } => {
                if *condition_changed {
                    eprintln!(
                        "[reconnect] account={account_id} gen={generation} reconnect read error: {error}"
                    );
                }
            }
        }

        event
    }
}

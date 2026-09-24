//! Safe Single-Item Enhancement Engine sidecar protocol & telemetry (ENHANCE-04).
//!
//! Provides transactional single-item enhancement command parsing, payload validation,
//! sidecar file transport (`zeus-enhance.req` and `zeus-enhance-status.json`),
//! and safe telemetry snapshot merging.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Instant;

/// Request filename written by agent for JVM enhancement engine.
pub const ENHANCE_REQUEST_FILE_NAME: &str = "zeus-enhance.req";
/// Status/telemetry filename emitted by JVM enhancement engine.
pub const ENHANCE_STATUS_FILE_NAME: &str = "zeus-enhance-status.json";
/// Cancel signal filename.
pub const ENHANCE_CANCEL_FILE_NAME: &str = "zeus-enhance.cancel";

/// Maximum bytes for enhancement status file.
pub const MAX_ENHANCE_STATUS_BYTES: u64 = 65536;

/// Default and maximum allowed attempts per single command.
pub const MIN_ATTEMPTS: u32 = 1;
pub const MAX_ATTEMPTS_BOUND: u32 = 100;

/// Wire category required for equipment enhancement candidates.
pub const EQUIPMENT_CATEGORY: i32 = 3;

/// Single-item enhancement command payload from cloud / external boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleItemEnhanceCommandPayload {
    pub captured_slot: i32,
    pub template_id: i32,
    pub category: i32,
    pub base_name: String,
    pub tier: i32,
    pub expected_level: i32,
    pub target_level: i32,
    pub charm_mode: u8,
    pub payment_type: u8,
    pub max_attempts: u32,
}

/// Request payload written to `zeus-enhance.req`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnhancementRequestPayload {
    pub request_id: String,
    pub captured_slot: i32,
    pub template_id: i32,
    pub category: i32,
    pub base_name: String,
    pub tier: i32,
    pub expected_level: i32,
    pub target_level: i32,
    pub charm_mode: u8,
    pub payment_type: u8,
    pub max_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<String>,
}

/// Enhancement state machine statuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementState {
    Idle,
    ValidatingRequest,
    WaitingGameReady,
    ValidatingTarget,
    LocatingBlacksmith,
    ApproachingBlacksmith,
    OpeningForge,
    InsertingTarget,
    ResolvingCharm,
    InsertingCharm,
    VerifyingResources,
    ReadyForAttempt,
    Attempting,
    Executing,
    WaitingResult,
    WaitingSettlement,
    FailureProtected,
    FailureDegraded,
    TargetReached,
    AttemptLimitReached,
    ItemDestroyed,
    ItemMissingOrChanged,
    AmbiguousWireTarget,
    AmbiguousCharm,
    IneligibleItem,
    CharmMissing,
    InsufficientGold,
    InsufficientGems,
    InsufficientMaterials,
    ServerRejected,
    ResultAmbiguous,
    AccountingUnsettled,
    Timeout,
    Cancelled,
    ManualReviewRequired,
}

impl EnhancementState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::TargetReached
                | Self::AttemptLimitReached
                | Self::ItemDestroyed
                | Self::ItemMissingOrChanged
                | Self::AmbiguousWireTarget
                | Self::AmbiguousCharm
                | Self::IneligibleItem
                | Self::CharmMissing
                | Self::InsufficientGold
                | Self::InsufficientGems
                | Self::InsufficientMaterials
                | Self::ServerRejected
                | Self::ResultAmbiguous
                | Self::AccountingUnsettled
                | Self::Timeout
                | Self::Cancelled
                | Self::ManualReviewRequired
        )
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::TargetReached)
    }
}

/// Enhancement status telemetry emitted by the runtime engine (Version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnhancementStatusTelemetry {
    pub version: u32,
    pub request_id: String,
    pub state: String,
    pub captured_slot: i32,
    pub template_id: i32,
    pub category: i32,
    pub base_name: String,
    pub start_level: i32,
    pub current_level: i32,
    pub target_level: i32,
    pub configured_charm_mode: u8,
    pub resolved_charm_mode: u8,
    pub payment_type: u8,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub last_result: Option<String>,
    pub quoted_gold_cost: i64,
    pub quoted_gem_cost: i64,
    pub quoted_material_requirements: Vec<i64>,
    pub actual_gold_spent: i64,
    pub actual_gem_spent: i64,
    pub actual_materials_spent: Vec<i64>,
    pub actual_charms_spent: i64,
    pub accounting_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub updated_at: String,
}

/// In-memory tracking of active enhancement in AccountState.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEnhancement {
    pub command_id: String,
    pub request_id: String,
    pub started_at: Instant,
    pub template_id: i32,
    pub category: i32,
    pub target_level: i32,
}

/// Validation error for single item enhancement payloads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadValidationError {
    #[error("category {0} is not an equipment enhancement candidate (expected 3)")]
    InvalidCategory(i32),
    #[error("expected_level {0} is out of bounds (0..14)")]
    ExpectedLevelOutOfBounds(i32),
    #[error("target_level {target} must be > expected_level {expected} and <= 15")]
    TargetLevelInvalid { expected: i32, target: i32 },
    #[error("charm_mode {0} must be 0, 1, 2, or 3")]
    InvalidCharmMode(u8),
    #[error("payment_type {0} must be 0 (Gold) or 1 (Gem)")]
    InvalidPaymentType(u8),
    #[error("max_attempts {0} must be between {MIN_ATTEMPTS} and {MAX_ATTEMPTS_BOUND}")]
    InvalidMaxAttempts(u32),
    #[error("base_name cannot be empty")]
    EmptyBaseName,
    #[error("captured_slot {0} is negative")]
    NegativeSlot(i32),
}

/// Strict validation of incoming single-item command payload before any game action.
pub fn validate_single_item_payload(
    payload: &SingleItemEnhanceCommandPayload,
) -> Result<(), PayloadValidationError> {
    if payload.category != EQUIPMENT_CATEGORY {
        return Err(PayloadValidationError::InvalidCategory(payload.category));
    }
    if payload.expected_level < 0 || payload.expected_level > 14 {
        return Err(PayloadValidationError::ExpectedLevelOutOfBounds(
            payload.expected_level,
        ));
    }
    if payload.target_level <= payload.expected_level || payload.target_level > 15 {
        return Err(PayloadValidationError::TargetLevelInvalid {
            expected: payload.expected_level,
            target: payload.target_level,
        });
    }
    if payload.charm_mode > 3 {
        return Err(PayloadValidationError::InvalidCharmMode(payload.charm_mode));
    }
    if payload.payment_type > 1 {
        return Err(PayloadValidationError::InvalidPaymentType(payload.payment_type));
    }
    if payload.max_attempts < MIN_ATTEMPTS || payload.max_attempts > MAX_ATTEMPTS_BOUND {
        return Err(PayloadValidationError::InvalidMaxAttempts(payload.max_attempts));
    }
    if payload.base_name.trim().is_empty() {
        return Err(PayloadValidationError::EmptyBaseName);
    }
    if payload.captured_slot < 0 {
        return Err(PayloadValidationError::NegativeSlot(payload.captured_slot));
    }
    Ok(())
}

/// Writes atomic request file `zeus-enhance.req`.
pub fn write_enhancement_request_file(
    home: &Path,
    request: &EnhancementRequestPayload,
) -> std::io::Result<()> {
    let req_path = home.join(ENHANCE_REQUEST_FILE_NAME);
    let tmp_path = home.join(format!("{}.tmp", ENHANCE_REQUEST_FILE_NAME));
    let json_bytes = serde_json::to_vec_pretty(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    fs::write(&tmp_path, json_bytes)?;
    fs::rename(&tmp_path, &req_path)?;
    Ok(())
}

/// Writes cancel marker file `zeus-enhance.cancel`.
pub fn write_enhancement_cancel_file(home: &Path) -> std::io::Result<()> {
    let cancel_path = home.join(ENHANCE_CANCEL_FILE_NAME);
    fs::write(cancel_path, b"cancel")?;
    Ok(())
}

/// Cleans only the request sidecar file, preserving status for telemetry.
pub fn clean_enhancement_request_file(home: &Path) {
    let _ = fs::remove_file(home.join(ENHANCE_REQUEST_FILE_NAME));
    let _ = fs::remove_file(home.join(format!("{}.tmp", ENHANCE_REQUEST_FILE_NAME)));
    let _ = fs::remove_file(home.join(ENHANCE_CANCEL_FILE_NAME));
}

/// Cleans all enhancement sidecar files for an account.
pub fn clean_enhancement_files(home: &Path) {
    let _ = fs::remove_file(home.join(ENHANCE_REQUEST_FILE_NAME));
    let _ = fs::remove_file(home.join(format!("{}.tmp", ENHANCE_REQUEST_FILE_NAME)));
    let _ = fs::remove_file(home.join(ENHANCE_STATUS_FILE_NAME));
    let _ = fs::remove_file(home.join(ENHANCE_CANCEL_FILE_NAME));
}

/// Timeout for pending enhancement before failing command (seconds).
pub const ENHANCEMENT_TIMEOUT_SECS: u64 = 120;

/// Checks if a pending enhancement has exceeded its timeout threshold.
pub fn is_pending_enhancement_timed_out(pending: &PendingEnhancement, now: Instant) -> bool {
    now.duration_since(pending.started_at).as_secs() >= ENHANCEMENT_TIMEOUT_SECS
}

impl std::str::FromStr for EnhancementState {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
    }
}

/// Outcome of polling for an enhancement status result.
#[derive(Debug, PartialEq)]
pub enum EnhancementPollOutcome {
    NoStatusYet,
    TerminalSuccess(EnhancementStatusTelemetry),
    TerminalFailure {
        state: String,
        error_message: Option<String>,
    },
    InvalidPayload(String),
}

/// Polls for completion of a pending enhancement.
pub fn poll_enhancement_status(home: &Path, expected_request_id: &str) -> EnhancementPollOutcome {
    let status_path = home.join(ENHANCE_STATUS_FILE_NAME);
    if !status_path.exists() {
        return EnhancementPollOutcome::NoStatusYet;
    }

    let status = match read_enhancement_status(&status_path) {
        Some(s) => s,
        None => {
            return EnhancementPollOutcome::InvalidPayload("cannot parse status file".to_string());
        }
    };

    if status.request_id != expected_request_id {
        return EnhancementPollOutcome::NoStatusYet;
    }

    if let Ok(state_enum) = status.state.parse::<EnhancementState>() {
        if state_enum.is_terminal() {
            if state_enum.is_success() {
                EnhancementPollOutcome::TerminalSuccess(status)
            } else {
                EnhancementPollOutcome::TerminalFailure {
                    state: status.state,
                    error_message: status.error_message,
                }
            }
        } else {
            EnhancementPollOutcome::NoStatusYet
        }
    } else if status.state == "TARGET_REACHED" {
        EnhancementPollOutcome::TerminalSuccess(status)
    } else {
        EnhancementPollOutcome::TerminalFailure {
            state: status.state,
            error_message: status.error_message,
        }
    }
}

/// Reads and validates `zeus-enhance-status.json`. Fails closed.
pub fn read_enhancement_status(path: &Path) -> Option<EnhancementStatusTelemetry> {
    if !path.exists() {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_ENHANCE_STATUS_BYTES {
        eprintln!("[enhancement] status file {:?} exceeds max bytes", path);
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let status: EnhancementStatusTelemetry = serde_json::from_str(&text).ok()?;
    if status.version != 1 {
        eprintln!("[enhancement] unsupported status version {}", status.version);
        return None;
    }
    Some(status)
}

/// Merges enhancement status telemetry into outgoing runtime snapshot.
pub fn merge_enhancement_into_snapshot(snapshot: &mut serde_json::Value, status_file: &Path) {
    if let Some(status) = read_enhancement_status(status_file) {
        if let Ok(status_val) = serde_json::to_value(status) {
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("enhancement".to_string(), status_val);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_valid_payload() -> SingleItemEnhanceCommandPayload {
        SingleItemEnhanceCommandPayload {
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            tier: 2,
            expected_level: 5,
            target_level: 7,
            charm_mode: 3,
            payment_type: 0,
            max_attempts: 10,
        }
    }

    #[test]
    fn test_valid_payload_passes_validation() {
        let p = sample_valid_payload();
        assert!(validate_single_item_payload(&p).is_ok());
    }

    #[test]
    fn test_category_must_be_equipment() {
        let mut p = sample_valid_payload();
        p.category = 4; // mount
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::InvalidCategory(4))
        ));
    }

    #[test]
    fn test_expected_level_bounds() {
        let mut p = sample_valid_payload();
        p.expected_level = -1;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::ExpectedLevelOutOfBounds(-1))
        ));

        p.expected_level = 15;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::ExpectedLevelOutOfBounds(15))
        ));
    }

    #[test]
    fn test_target_level_must_be_greater_than_expected_and_max_15() {
        let mut p = sample_valid_payload();
        p.expected_level = 7;
        p.target_level = 7; // equal
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::TargetLevelInvalid { expected: 7, target: 7 })
        ));

        p.target_level = 5; // less
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::TargetLevelInvalid { expected: 7, target: 5 })
        ));

        p.target_level = 16; // exceeds 15
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::TargetLevelInvalid { expected: 7, target: 16 })
        ));
    }

    #[test]
    fn test_charm_mode_must_be_0_to_3() {
        let mut p = sample_valid_payload();
        p.charm_mode = 4;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::InvalidCharmMode(4))
        ));
    }

    #[test]
    fn test_payment_type_must_be_0_or_1() {
        let mut p = sample_valid_payload();
        p.payment_type = 2;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::InvalidPaymentType(2))
        ));
    }

    #[test]
    fn test_max_attempts_must_be_bounded() {
        let mut p = sample_valid_payload();
        p.max_attempts = 0;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::InvalidMaxAttempts(0))
        ));

        p.max_attempts = 101;
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::InvalidMaxAttempts(101))
        ));
    }

    #[test]
    fn test_base_name_cannot_be_empty() {
        let mut p = sample_valid_payload();
        p.base_name = "   ".to_string();
        assert!(matches!(
            validate_single_item_payload(&p),
            Err(PayloadValidationError::EmptyBaseName)
        ));
    }

    #[test]
    fn test_request_file_serialization_and_cleanup() {
        let dir = tempdir().unwrap();
        let home = dir.path();

        let req = EnhancementRequestPayload {
            request_id: "cmd-uuid-1234".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            tier: 2,
            expected_level: 5,
            target_level: 7,
            charm_mode: 3,
            payment_type: 0,
            max_attempts: 10,
            requested_at: Some("2026-09-24T12:00:00Z".to_string()),
        };

        assert!(write_enhancement_request_file(home, &req).is_ok());
        let req_file = home.join(ENHANCE_REQUEST_FILE_NAME);
        assert!(req_file.exists());

        let content = fs::read_to_string(&req_file).unwrap();
        assert!(content.contains("\"request_id\": \"cmd-uuid-1234\""));
        assert!(content.contains("\"template_id\": 101"));

        clean_enhancement_files(home);
        assert!(!req_file.exists());
    }

    #[test]
    fn test_read_enhancement_status_and_merge() {
        let dir = tempdir().unwrap();
        let status_path = dir.path().join(ENHANCE_STATUS_FILE_NAME);

        let sample_status = r#"{
            "version": 1,
            "request_id": "cmd-uuid-1234",
            "state": "TARGET_REACHED",
            "captured_slot": 0,
            "template_id": 101,
            "category": 3,
            "base_name": "Kiếm ngắn",
            "start_level": 5,
            "current_level": 7,
            "target_level": 7,
            "configured_charm_mode": 3,
            "resolved_charm_mode": 1,
            "payment_type": 0,
            "attempt_count": 2,
            "max_attempts": 10,
            "last_result": "SUCCESS",
            "quoted_gold_cost": 50000,
            "quoted_gem_cost": 0,
            "quoted_material_requirements": [5, 2, 0, 0],
            "actual_gold_spent": 100000,
            "actual_gem_spent": 0,
            "actual_materials_spent": [10, 4, 0, 0],
            "actual_charms_spent": 2,
            "accounting_status": "SETTLED",
            "updated_at": "2026-09-24T12:01:00Z"
        }"#;

        fs::write(&status_path, sample_status).unwrap();

        let parsed = read_enhancement_status(&status_path).expect("must parse");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.request_id, "cmd-uuid-1234");
        assert_eq!(parsed.state, "TARGET_REACHED");
        assert_eq!(parsed.actual_gold_spent, 100000);

        let mut snapshot = serde_json::json!({
            "hp": 1000,
            "mp": 500
        });

        merge_enhancement_into_snapshot(&mut snapshot, &status_path);
        assert!(snapshot.get("enhancement").is_some());
        assert_eq!(
            snapshot["enhancement"]["state"].as_str().unwrap(),
            "TARGET_REACHED"
        );
        assert_eq!(snapshot["hp"], 1000);
    }

    #[test]
    fn test_poll_enhancement_status_in_progress_and_terminal() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        let status_path = home.join(ENHANCE_STATUS_FILE_NAME);

        // 1. Missing file -> NoStatusYet
        assert_eq!(
            poll_enhancement_status(home, "req-1"),
            EnhancementPollOutcome::NoStatusYet
        );

        // 2. In-progress state -> NoStatusYet
        let in_progress_json = r#"{
            "version": 1,
            "request_id": "req-1",
            "state": "EXECUTING",
            "captured_slot": 0,
            "template_id": 101,
            "category": 3,
            "base_name": "Sword",
            "start_level": 5,
            "current_level": 5,
            "target_level": 7,
            "configured_charm_mode": 3,
            "resolved_charm_mode": 1,
            "payment_type": 0,
            "attempt_count": 1,
            "max_attempts": 5,
            "last_result": null,
            "quoted_gold_cost": 50000,
            "quoted_gem_cost": 0,
            "quoted_material_requirements": [],
            "actual_gold_spent": 0,
            "actual_gem_spent": 0,
            "actual_materials_spent": [],
            "actual_charms_spent": 0,
            "accounting_status": "PENDING",
            "updated_at": "2026-09-24T12:00:00Z"
        }"#;
        fs::write(&status_path, in_progress_json).unwrap();
        assert_eq!(
            poll_enhancement_status(home, "req-1"),
            EnhancementPollOutcome::NoStatusYet
        );

        // 3. Mismatched request_id -> NoStatusYet
        assert_eq!(
            poll_enhancement_status(home, "req-different"),
            EnhancementPollOutcome::NoStatusYet
        );

        // 4. Terminal success -> TerminalSuccess
        let success_json = in_progress_json.replace("\"EXECUTING\"", "\"TARGET_REACHED\"");
        fs::write(&status_path, success_json).unwrap();
        match poll_enhancement_status(home, "req-1") {
            EnhancementPollOutcome::TerminalSuccess(telemetry) => {
                assert_eq!(telemetry.state, "TARGET_REACHED");
            }
            other => panic!("expected TerminalSuccess, got {:?}", other),
        }

        // 5. Terminal failure -> TerminalFailure
        let fail_json = in_progress_json
            .replace("\"EXECUTING\"", "\"ITEM_DESTROYED\"")
            .replace("\"last_result\": null", "\"last_result\": \"DESTROYED\"");
        fs::write(&status_path, fail_json).unwrap();
        match poll_enhancement_status(home, "req-1") {
            EnhancementPollOutcome::TerminalFailure { state, .. } => {
                assert_eq!(state, "ITEM_DESTROYED");
            }
            other => panic!("expected TerminalFailure, got {:?}", other),
        }

        // 6. Request cleanup preserves status
        let req_path = home.join(ENHANCE_REQUEST_FILE_NAME);
        fs::write(&req_path, b"dummy req").unwrap();
        clean_enhancement_request_file(home);
        assert!(!req_path.exists());
        assert!(status_path.exists());
    }
}

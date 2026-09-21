//! Detect spots command and sidecar protocol (R3B1).
//! Cross-platform protocol, types, validation, and snapshot injection.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

/// Request filename written by agent for JVM consumer.
pub const SPOT_REQUEST_FILE_NAME: &str = "zeus-spot.req";
/// Temporary payload file written by JVM before setting ready marker.
pub const SPOT_RESULT_PAYLOAD_FILE_NAME: &str = "zeus-spot-result.tmp";
/// Ready marker file written by JVM after payload is fully flushed.
pub const SPOT_RESULT_READY_FILE_NAME: &str = "zeus-spot-result.ready";

/// Initial target timeout for a pending scan (5 seconds).
pub const SPOT_SCAN_TIMEOUT_SECS: u64 = 5;
/// Time-to-live for a retained completed scan in AccountState (300 seconds).
pub const SPOT_SCAN_COMPLETED_TTL_SECS: u64 = 300;

/// A detected monster group / spawn candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotCandidate {
    pub x: f64,
    pub y: f64,
    pub mob_count: u32,
    pub spread_radius: f64,
    pub mob_name: String,
    pub mob_level: i32,
}

/// Request payload written to `zeus-spot.req`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotScanRequestPayload {
    pub scan_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<String>,
}

/// Result payload parsed from `zeus-spot-result.tmp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotScanResultPayload {
    pub scan_id: String,
    pub map_id: u8,
    pub captured_zone: i64,
    pub candidates: Vec<SpotCandidate>,
}

/// Cloud `spot_scan` status values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotScanStatus {
    Pending,
    Completed,
    Empty,
    Timeout,
    Error,
}

impl SpotScanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Empty => "empty",
            Self::Timeout => "timeout",
            Self::Error => "error",
        }
    }
}

/// Track an active pending scan for an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSpotScan {
    pub scan_id: String,
    pub command_id: String,
    pub started_at: Instant,
}

/// Retained latest scan stored in AccountState across telemetry pushes.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedSpotScan {
    pub scan_id: String,
    pub status: SpotScanStatus,
    pub detected_at: Option<String>,
    pub completed_at: Option<Instant>,
    pub map_id: Option<u8>,
    pub captured_zone: Option<i64>,
    pub candidates: Option<Vec<SpotCandidate>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpotScanError {
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("empty scan_id")]
    EmptyScanId,
    #[error("mismatched scan_id: expected {expected}, got {got}")]
    MismatchedScanId { expected: String, got: String },
    #[error("negative captured zone: {0}")]
    NegativeCapturedZone(i64),
    #[error("negative candidate coordinate: x={x}, y={y}")]
    NegativeCoordinate { x: f64, y: f64 },
    #[error("negative candidate spread radius: {0}")]
    NegativeSpreadRadius(f64),
}

/// Serializes a `zeus-spot.req` JSON payload.
pub fn build_spot_request_json(scan_id: &str, requested_at: Option<&str>) -> String {
    let payload = SpotScanRequestPayload {
        scan_id: scan_id.to_string(),
        requested_at: requested_at.map(|s| s.to_string()),
    };
    serde_json::to_string(&payload).unwrap_or_else(|_| format!(r#"{{"scan_id":"{scan_id}"}}"#))
}

/// Parses and validates a JVM `zeus-spot-result.tmp` payload.
pub fn parse_and_validate_spot_result(
    content: &str,
    expected_scan_id: &str,
) -> Result<SpotScanResultPayload, SpotScanError> {
    let parsed: SpotScanResultPayload = serde_json::from_str(content)?;

    if parsed.scan_id.trim().is_empty() {
        return Err(SpotScanError::EmptyScanId);
    }
    if parsed.scan_id != expected_scan_id {
        return Err(SpotScanError::MismatchedScanId {
            expected: expected_scan_id.to_string(),
            got: parsed.scan_id,
        });
    }
    if parsed.captured_zone < 0 {
        return Err(SpotScanError::NegativeCapturedZone(parsed.captured_zone));
    }
    for c in &parsed.candidates {
        if c.x < 0.0 || c.y < 0.0 {
            return Err(SpotScanError::NegativeCoordinate { x: c.x, y: c.y });
        }
        if c.spread_radius < 0.0 {
            return Err(SpotScanError::NegativeSpreadRadius(c.spread_radius));
        }
    }

    Ok(parsed)
}

/// Deletes all spot scan sidecar files in the given directory.
pub fn clean_spot_files(home: &Path) {
    let _ = std::fs::remove_file(home.join(SPOT_REQUEST_FILE_NAME));
    let _ = std::fs::remove_file(home.join(format!("{}.tmp", SPOT_REQUEST_FILE_NAME)));
    let _ = std::fs::remove_file(home.join(SPOT_RESULT_PAYLOAD_FILE_NAME));
    let _ = std::fs::remove_file(home.join(SPOT_RESULT_READY_FILE_NAME));
}

/// Writes `zeus-spot.req` using the repository's safest atomic small-file replace convention.
pub fn write_spot_request_file(
    home: &Path,
    scan_id: &str,
    requested_at: Option<&str>,
) -> std::io::Result<()> {
    use std::io::Write;
    let json_text = build_spot_request_json(scan_id, requested_at);
    let tmp_path = home.join(format!("{}.tmp", SPOT_REQUEST_FILE_NAME));
    let final_path = home.join(SPOT_REQUEST_FILE_NAME);

    let mut file = std::fs::File::create(&tmp_path)?;
    file.write_all(json_text.as_bytes())?;
    file.flush()?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Outcome of polling for a spot scan result.
#[derive(Debug)]
pub enum SpotPollOutcome {
    NoReadyMarker,
    Success(SpotScanResultPayload),
    InvalidPayload(String),
}

/// Polls for completion of a spot scan:
/// NEVER reads `zeus-spot-result.tmp` unless `zeus-spot-result.ready` exists.
pub fn poll_spot_result(home: &Path, expected_scan_id: &str) -> SpotPollOutcome {
    let ready_path = home.join(SPOT_RESULT_READY_FILE_NAME);
    if !ready_path.exists() {
        return SpotPollOutcome::NoReadyMarker;
    }

    let payload_path = home.join(SPOT_RESULT_PAYLOAD_FILE_NAME);
    let content = match std::fs::read_to_string(&payload_path) {
        Ok(c) => c,
        Err(e) => return SpotPollOutcome::InvalidPayload(format!("cannot read payload file: {e}")),
    };

    match parse_and_validate_spot_result(&content, expected_scan_id) {
        Ok(payload) => SpotPollOutcome::Success(payload),
        Err(e) => SpotPollOutcome::InvalidPayload(e.to_string()),
    }
}

/// Validates whether a new spot scan request can be accepted.
pub fn can_accept_spot_scan(pending: &Option<PendingSpotScan>) -> Result<(), &'static str> {
    if pending.is_some() {
        Err("a spot scan is already pending for this account")
    } else {
        Ok(())
    }
}

/// Checks if a pending scan has exceeded its timeout threshold.
pub fn is_pending_scan_timed_out(pending: &PendingSpotScan, now: Instant) -> bool {
    now.duration_since(pending.started_at).as_secs() >= SPOT_SCAN_TIMEOUT_SECS
}

/// Evaluates whether a retained spot scan is currently valid for inclusion in runtime snapshot.
pub fn is_valid_retained_scan(
    retained: &RetainedSpotScan,
    current_map: Option<u8>,
    now: Instant,
) -> bool {
    match retained.status {
        SpotScanStatus::Completed | SpotScanStatus::Empty => {
            // Check completed TTL (300 seconds)
            if let Some(completed_at) = retained.completed_at {
                if now.duration_since(completed_at).as_secs() >= SPOT_SCAN_COMPLETED_TTL_SECS {
                    return false;
                }
            }
            // Check map change: invalidate when current telemetry map differs from result map
            if let (Some(cur_map), Some(res_map)) = (current_map, retained.map_id) {
                if cur_map != res_map {
                    return false;
                }
            }
            true
        }
        SpotScanStatus::Pending => true,
        SpotScanStatus::Timeout | SpotScanStatus::Error => {
            // Error / timeout status retained temporarily (e.g. within completed TTL)
            if let Some(completed_at) = retained.completed_at {
                if now.duration_since(completed_at).as_secs() >= SPOT_SCAN_COMPLETED_TTL_SECS {
                    return false;
                }
            }
            true
        }
    }
}

/// Converts a `RetainedSpotScan` into a JSON Value conforming to `cloud_payload_contract`.
pub fn retained_scan_to_json(retained: &RetainedSpotScan) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "scan_id".to_string(),
        serde_json::Value::String(retained.scan_id.clone()),
    );
    obj.insert(
        "status".to_string(),
        serde_json::Value::String(retained.status.as_str().to_string()),
    );

    match retained.status {
        SpotScanStatus::Completed | SpotScanStatus::Empty => {
            if let Some(ref detected_at) = retained.detected_at {
                obj.insert(
                    "detected_at".to_string(),
                    serde_json::Value::String(detected_at.clone()),
                );
            }
            if let Some(map_id) = retained.map_id {
                obj.insert(
                    "map_id".to_string(),
                    serde_json::Value::Number(map_id.into()),
                );
            }
            if let Some(zone) = retained.captured_zone {
                obj.insert(
                    "captured_zone".to_string(),
                    serde_json::Value::Number(zone.into()),
                );
            }
            if let Some(ref candidates) = retained.candidates {
                if let Ok(c_val) = serde_json::to_value(candidates) {
                    obj.insert("candidates".to_string(), c_val);
                }
            }
        }
        _ => {}
    }

    serde_json::Value::Object(obj)
}

/// Merges retained `spot_scan` into outgoing telemetry snapshot if valid.
/// If invalid or expired, removes any existing `spot_scan` so cloud state is cleared by replacement.
pub fn merge_spot_scan_into_snapshot(
    snapshot: &mut serde_json::Value,
    retained: &Option<RetainedSpotScan>,
    current_map: Option<u8>,
    now: Instant,
) {
    if let Some(scan) = retained {
        if is_valid_retained_scan(scan, current_map, now) {
            let scan_val = retained_scan_to_json(scan);
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("spot_scan".to_string(), scan_val);
            }
            return;
        }
    }

    // Scan is missing, invalid, or expired: omit from snapshot.
    if let Some(obj) = snapshot.as_object_mut() {
        obj.remove("spot_scan");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_request_serialization_contains_exact_command_uuid_as_scan_id() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let req_json = build_spot_request_json(uuid, Some("2026-09-21T12:00:00Z"));
        let parsed: serde_json::Value = serde_json::from_str(&req_json).expect("valid json");
        assert_eq!(parsed["scan_id"], uuid);
        assert_eq!(parsed["requested_at"], "2026-09-21T12:00:00Z");
    }

    #[test]
    fn test_valid_matching_result_parses_successfully() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let payload = r#"{
            "scan_id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d",
            "map_id": 1,
            "captured_zone": 0,
            "candidates": [
                {
                    "x": 105.5,
                    "y": 200.0,
                    "mob_count": 4,
                    "spread_radius": 15.0,
                    "mob_name": "Orc Warrior",
                    "mob_level": 45
                }
            ]
        }"#;
        let res = parse_and_validate_spot_result(payload, uuid).expect("should parse");
        assert_eq!(res.scan_id, uuid);
        assert_eq!(res.map_id, 1);
        assert_eq!(res.captured_zone, 0);
        assert_eq!(res.candidates.len(), 1);
        assert_eq!(res.candidates[0].mob_name, "Orc Warrior");
    }

    #[test]
    fn test_mismatched_scan_id_is_rejected() {
        let expected_uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let different_uuid = "11111111-2222-3333-4444-555555555555";
        let payload = format!(
            r#"{{
                "scan_id": "{different_uuid}",
                "map_id": 1,
                "captured_zone": 0,
                "candidates": []
            }}"#
        );
        let res = parse_and_validate_spot_result(&payload, expected_uuid);
        assert!(matches!(res, Err(SpotScanError::MismatchedScanId { .. })));
    }

    #[test]
    fn test_empty_candidates_is_valid() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let payload = format!(
            r#"{{
                "scan_id": "{uuid}",
                "map_id": 2,
                "captured_zone": 1,
                "candidates": []
            }}"#
        );
        let res =
            parse_and_validate_spot_result(&payload, uuid).expect("empty candidates is valid");
        assert_eq!(res.scan_id, uuid);
        assert_eq!(res.candidates.len(), 0);
    }

    #[test]
    fn test_malformed_json_is_rejected() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        // Syntax error
        let bad_json = r#"{"scan_id": "bad"#;
        assert!(parse_and_validate_spot_result(bad_json, uuid).is_err());

        // Missing required field "candidates"
        let missing_candidates = format!(
            r#"{{
                "scan_id": "{uuid}",
                "map_id": 1,
                "captured_zone": 0
            }}"#
        );
        assert!(parse_and_validate_spot_result(&missing_candidates, uuid).is_err());

        // Negative coordinate
        let neg_coord = format!(
            r#"{{
                "scan_id": "{uuid}",
                "map_id": 1,
                "captured_zone": 0,
                "candidates": [
                    {{
                        "x": -5.0,
                        "y": 10.0,
                        "mob_count": 1,
                        "spread_radius": 2.0,
                        "mob_name": "Wolf",
                        "mob_level": 10
                    }}
                ]
            }}"#
        );
        assert!(matches!(
            parse_and_validate_spot_result(&neg_coord, uuid),
            Err(SpotScanError::NegativeCoordinate { .. })
        ));
    }

    #[test]
    fn test_ready_marker_absent_means_payload_is_not_consumed() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let dir_path = temp_dir.path();
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";

        // Write payload file but NO ready file
        let payload_file = dir_path.join(SPOT_RESULT_PAYLOAD_FILE_NAME);
        std::fs::write(
            &payload_file,
            format!(r#"{{"scan_id":"{uuid}","map_id":1,"captured_zone":0,"candidates":[]}}"#),
        )
        .expect("write payload");

        let outcome = poll_spot_result(dir_path, uuid);
        assert!(matches!(outcome, SpotPollOutcome::NoReadyMarker));

        // Now create ready marker
        let ready_file = dir_path.join(SPOT_RESULT_READY_FILE_NAME);
        std::fs::write(&ready_file, "ready").expect("write ready");

        let outcome_ready = poll_spot_result(dir_path, uuid);
        assert!(matches!(outcome_ready, SpotPollOutcome::Success(_)));
    }

    #[test]
    fn test_timeout_state_transitions_cleanly() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let started = Instant::now() - Duration::from_secs(6);
        let pending = PendingSpotScan {
            scan_id: uuid.to_string(),
            command_id: uuid.to_string(),
            started_at: started,
        };

        assert!(is_pending_scan_timed_out(&pending, Instant::now()));

        let retained = RetainedSpotScan {
            scan_id: pending.scan_id,
            status: SpotScanStatus::Timeout,
            detected_at: None,
            completed_at: Some(Instant::now()),
            map_id: None,
            captured_zone: None,
            candidates: None,
        };
        assert_eq!(retained.status, SpotScanStatus::Timeout);
        let json = retained_scan_to_json(&retained);
        assert_eq!(json["status"], "timeout");
        assert_eq!(json["scan_id"], uuid);
    }

    #[test]
    fn test_second_request_while_pending_is_rejected() {
        let pending = Some(PendingSpotScan {
            scan_id: "scan-1".to_string(),
            command_id: "cmd-1".to_string(),
            started_at: Instant::now(),
        });

        let res = can_accept_spot_scan(&pending);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            "a spot scan is already pending for this account"
        );

        let none_pending: Option<PendingSpotScan> = None;
        assert!(can_accept_spot_scan(&none_pending).is_ok());
    }

    #[test]
    fn test_retained_completed_result_is_merged_into_outgoing_snapshot() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let now = Instant::now();
        let retained = RetainedSpotScan {
            scan_id: uuid.to_string(),
            status: SpotScanStatus::Completed,
            detected_at: Some("2026-09-21T12:00:00Z".to_string()),
            completed_at: Some(now),
            map_id: Some(1),
            captured_zone: Some(0),
            candidates: Some(vec![SpotCandidate {
                x: 100.0,
                y: 200.0,
                mob_count: 5,
                spread_radius: 10.0,
                mob_name: "Orc".to_string(),
                mob_level: 40,
            }]),
        };

        let mut snapshot = serde_json::json!({
            "hp": 1000,
            "hpmax": 1000,
            "map": 1
        });

        merge_spot_scan_into_snapshot(&mut snapshot, &Some(retained), Some(1), now);

        let spot = &snapshot["spot_scan"];
        assert_eq!(spot["scan_id"], uuid);
        assert_eq!(spot["status"], "completed");
        assert_eq!(spot["detected_at"], "2026-09-21T12:00:00Z");
        assert_eq!(spot["map_id"], 1);
        assert_eq!(spot["captured_zone"], 0);
        assert_eq!(spot["candidates"].as_array().unwrap().len(), 1);
        // Original fields intact
        assert_eq!(snapshot["hp"], 1000);
    }

    #[test]
    fn test_expired_result_is_omitted() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let completed_time = Instant::now() - Duration::from_secs(305);
        let retained = RetainedSpotScan {
            scan_id: uuid.to_string(),
            status: SpotScanStatus::Completed,
            detected_at: Some("2026-09-21T12:00:00Z".to_string()),
            completed_at: Some(completed_time),
            map_id: Some(1),
            captured_zone: Some(0),
            candidates: Some(vec![]),
        };

        let mut snapshot = serde_json::json!({
            "hp": 1000,
            "map": 1,
            "spot_scan": { "stale": true }
        });

        merge_spot_scan_into_snapshot(&mut snapshot, &Some(retained), Some(1), Instant::now());
        assert!(snapshot.get("spot_scan").is_none());
    }

    #[test]
    fn test_result_is_invalidated_when_current_map_changes() {
        let uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
        let now = Instant::now();
        let retained = RetainedSpotScan {
            scan_id: uuid.to_string(),
            status: SpotScanStatus::Completed,
            detected_at: Some("2026-09-21T12:00:00Z".to_string()),
            completed_at: Some(now),
            map_id: Some(1), // Scan took place on map 1
            captured_zone: Some(0),
            candidates: Some(vec![]),
        };

        let mut snapshot = serde_json::json!({
            "hp": 1000,
            "map": 2 // Current map changed to 2!
        });

        // Current telemetry map is 2, while result was map 1
        merge_spot_scan_into_snapshot(&mut snapshot, &Some(retained), Some(2), now);
        assert!(snapshot.get("spot_scan").is_none());
    }
}

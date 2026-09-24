//! Read-only inventory catalog ingest and snapshot merge (ENHANCE-01).
//!
//! Zeus Java emits a self-versioned `zeus-inventory.json` sidecar beside the snapshot.
//! This module parses, validates, and safely merges it into the account runtime snapshot
//! without requiring any database schema migration.

use std::fs;
use std::path::Path;

pub const INVENTORY_FILE_NAME: &str = "zeus-inventory.json";
pub const MAX_INVENTORY_BYTES: u64 = 65536; // 64 KiB bounds full bag

/// Reads and validates the inventory catalog from disk.
/// Fails closed: returns None on missing, corrupt, or invalid schema,
/// ensuring telemetry is never poisoned.
pub fn read_inventory_catalog(path: &Path) -> Option<serde_json::Value> {
    if !path.exists() {
        return None;
    }

    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_INVENTORY_BYTES {
        eprintln!("[inventory] file {:?} exceeds max bytes ({})", path, metadata.len());
        return None;
    }

    let text = fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&text).ok()?;

    // Contract validation
    let obj = val.as_object()?;
    let version = obj.get("version")?.as_u64()?;
    if version != 1 {
        eprintln!("[inventory] unsupported inventory version {}", version);
        return None;
    }

    if !obj.contains_key("bag_capacity") || obj.get("bag_capacity").and_then(|v| v.as_i64()).is_none() {
        eprintln!("[inventory] missing or invalid bag_capacity");
        return None;
    }

    let items = obj.get("items")?.as_array()?;
    for item in items {
        let it = item.as_object()?;
        if it.get("slot").and_then(|v| v.as_i64()).is_none()
            || it.get("template_id").and_then(|v| v.as_i64()).is_none()
            || it.get("category").and_then(|v| v.as_i64()).is_none()
            || it.get("base_name").and_then(|v| v.as_str()).is_none()
            || it.get("display_name").and_then(|v| v.as_str()).is_none()
            || it.get("level").and_then(|v| v.as_i64()).is_none()
            || it.get("tier").and_then(|v| v.as_i64()).is_none()
            || it.get("count").and_then(|v| v.as_i64()).is_none()
            || it.get("candidate_for_enhancement").and_then(|v| v.as_bool()).is_none()
        {
            eprintln!("[inventory] invalid item entry in catalog: {:?}", it);
            return None;
        }
    }

    Some(val)
}

/// Merges inventory catalog into the outgoing runtime snapshot.
/// If absent or invalid, snapshot is untouched.
pub fn merge_inventory_into_snapshot(snapshot: &mut serde_json::Value, inventory_file: &Path) {
    if let Some(catalog) = read_inventory_catalog(inventory_file) {
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert("inventory".to_string(), catalog);
        }
    }
}

/// Safely removes inventory sidecar on account stop / retire.
pub fn clear_inventory(home: &Path) {
    let path = home.join(INVENTORY_FILE_NAME);
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_missing_inventory_file_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("non_existent.json");
        assert_eq!(read_inventory_catalog(&path), None);
    }

    #[test]
    fn test_valid_inventory_catalog_parses_successfully() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        let sample_json = r#"{
            "version": 1,
            "bag_capacity": 28,
            "items": [
                {
                    "slot": 0,
                    "template_id": 101,
                    "category": 3,
                    "base_name": "Kiếm ngắn",
                    "display_name": "Kiếm ngắn +5",
                    "level": 5,
                    "tier": 2,
                    "count": 1,
                    "durability": 500,
                    "bind": 1,
                    "icon": 12,
                    "candidate_for_enhancement": true
                },
                {
                    "slot": 1,
                    "template_id": 62,
                    "category": 4,
                    "base_name": "Ngựa trắng",
                    "display_name": "Ngựa trắng",
                    "level": 0,
                    "tier": 1,
                    "count": 1,
                    "durability": null,
                    "bind": 0,
                    "icon": 25,
                    "candidate_for_enhancement": false
                }
            ]
        }"#;
        fs::write(&path, sample_json).unwrap();

        let parsed = read_inventory_catalog(&path).expect("must parse valid catalog");
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["bag_capacity"], 28);
        let items = parsed["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["slot"], 0);
        assert_eq!(items[0]["candidate_for_enhancement"], true);
        assert_eq!(items[1]["slot"], 1);
        assert_eq!(items[1]["durability"], serde_json::Value::Null);
    }

    #[test]
    fn test_malformed_json_fails_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        fs::write(&path, "{ corrupt json ...").unwrap();
        assert_eq!(read_inventory_catalog(&path), None);
    }

    #[test]
    fn test_unsupported_version_fails_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        fs::write(&path, r#"{"version": 99, "bag_capacity": 28, "items": []}"#).unwrap();
        assert_eq!(read_inventory_catalog(&path), None);
    }

    #[test]
    fn test_missing_item_fields_fails_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        // missing candidate_for_enhancement
        let invalid_item = r#"{
            "version": 1,
            "bag_capacity": 28,
            "items": [
                {
                    "slot": 0,
                    "template_id": 101,
                    "category": 3,
                    "base_name": "Kiếm",
                    "display_name": "Kiếm",
                    "level": 0,
                    "tier": 1,
                    "count": 1
                }
            ]
        }"#;
        fs::write(&path, invalid_item).unwrap();
        assert_eq!(read_inventory_catalog(&path), None);
    }

    #[test]
    fn test_merge_inventory_into_snapshot_retains_existing_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        fs::write(&path, r#"{"version": 1, "bag_capacity": 28, "items": []}"#).unwrap();

        let mut snapshot = serde_json::json!({
            "v": 6,
            "name": "hero",
            "hp": 1000
        });

        merge_inventory_into_snapshot(&mut snapshot, &path);
        assert_eq!(snapshot["v"], 6);
        assert_eq!(snapshot["name"], "hero");
        assert_eq!(snapshot["hp"], 1000);
        assert_eq!(snapshot["inventory"]["version"], 1);
        assert_eq!(snapshot["inventory"]["bag_capacity"], 28);
    }

    #[test]
    fn test_merge_missing_inventory_does_not_modify_snapshot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let mut snapshot = serde_json::json!({
            "v": 6,
            "name": "hero"
        });

        merge_inventory_into_snapshot(&mut snapshot, &path);
        assert_eq!(snapshot["v"], 6);
        assert_eq!(snapshot["name"], "hero");
        assert!(snapshot.get("inventory").is_none());
    }

    #[test]
    fn test_clear_inventory_removes_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(INVENTORY_FILE_NAME);
        fs::write(&path, "{}").unwrap();
        assert!(path.exists());
        clear_inventory(dir.path());
        assert!(!path.exists());
    }
}

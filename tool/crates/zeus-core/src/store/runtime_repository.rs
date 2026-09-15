use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::runtime::{
    CapabilityState, RuntimePreflightKey, RuntimePreflightMode, RuntimePreflightValidation,
    ValidatedRuntime, runtime_matches_registry_record, validate_runtime_descriptor_full,
};

use super::CoreState;

const MAX_PAGE_LIMIT: u32 = 100;

const RUNTIME_COLUMNS: &str = "runtime_id, descriptor_sha256, descriptor_path, runtime_root, \
    target_os, target_arch, java_path, java_vendor, java_version, jre_manifest_sha256, \
    microemulator_path, microemulator_version, microemulator_sha256, game_path, game_bundle, \
    game_sha256, capability_state, validation_reason, validated_at_unix_ms, created_at_unix_ms";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeRecord {
    pub runtime_id: String,
    pub descriptor_sha256: String,
    pub descriptor_path: PathBuf,
    pub runtime_root: PathBuf,
    pub target_os: String,
    pub target_arch: String,
    pub java_path: PathBuf,
    pub java_vendor: String,
    pub java_version: String,
    pub jre_manifest_sha256: String,
    pub microemulator_path: PathBuf,
    pub microemulator_version: String,
    pub microemulator_sha256: String,
    pub game_path: PathBuf,
    pub game_bundle: String,
    pub game_sha256: String,
    pub capability_state: CapabilityState,
    pub validation_reason: String,
    pub validated_at_unix_ms: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimePage {
    pub items: Vec<RuntimeRecord>,
    pub next_cursor: Option<String>,
}

/// Records that a pinned runtime kept its identity while its content was replaced.
///
/// A re-pin is not silent: refreshing the game jar used to cost the operator every account, so the
/// registry now follows the new content instead of refusing to open. What it must never do is
/// follow it *unnoticed*, because "the jar changed under me" and "I rebuilt the jar" look identical
/// in the database and only the second one is expected. This is the audit trail that tells them
/// apart after the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeRepin {
    pub runtime_id: String,
    pub previous_descriptor_sha256: String,
    pub descriptor_sha256: String,
    pub previous_game_sha256: String,
    pub game_sha256: String,
    pub previous_microemulator_sha256: String,
    pub microemulator_sha256: String,
    pub previous_jre_manifest_sha256: String,
    pub jre_manifest_sha256: String,
    pub repinned_at_unix_ms: i64,
}

impl RuntimeRepin {
    /// True when the game jar is the only artifact that moved — the ordinary jar-rebuild case.
    pub fn game_jar_only(&self) -> bool {
        self.previous_game_sha256 != self.game_sha256
            && self.previous_microemulator_sha256 == self.microemulator_sha256
            && self.previous_jre_manifest_sha256 == self.jre_manifest_sha256
    }
}

impl CoreState {
    pub fn register_runtime_descriptor(
        &mut self,
        descriptor_path: &Path,
    ) -> CoreResult<RuntimeRecord> {
        let validation = validate_runtime_descriptor_full(descriptor_path)?;
        let record = self.register_validated_runtime(validation.runtime.clone())?;
        self.record_preflight_success(&record, validation);
        Ok(record)
    }

    /// Verifies the pinned runtime at its current exact root and re-points every stored absolute
    /// path in one transaction.
    ///
    /// The runtime is validated independently first. When the descriptor digest still matches, only
    /// paths move and every digest is held. When it does not, the row is re-pinned to the new
    /// content — see `repin_pinned_runtime` for what that refuses to follow. Runtime ID, creation
    /// time and profile bindings never change on either path.
    pub(crate) fn relocate_pinned_runtime(
        &mut self,
        descriptor_path: &Path,
    ) -> CoreResult<RuntimeRecord> {
        let validation = validate_runtime_descriptor_full(descriptor_path)?;
        let validated = validation.runtime.clone();
        validate_persistable_runtime(&validated)?;
        let stored = {
            let sql = format!("SELECT {RUNTIME_COLUMNS} FROM runtimes WHERE runtime_id = ?1");
            self.connection
                .query_row(&sql, params![validated.runtime_id], row_to_runtime)
                .optional()?
        };
        let Some(stored) = stored else {
            return self.register_runtime_descriptor(descriptor_path);
        };
        if stored.descriptor_sha256 != validated.descriptor_sha256 {
            return self.repin_pinned_runtime(stored, validation);
        }
        if runtime_matches_registry_record(&stored, &validated) {
            self.record_preflight_success(&stored, validation);
            return Ok(stored);
        }
        let relocated = RuntimeRecord {
            descriptor_path: validated.descriptor_path.clone(),
            runtime_root: validated.runtime_root.clone(),
            java_path: validated.java_path.clone(),
            microemulator_path: validated.microemulator_path.clone(),
            game_path: validated.game_path.clone(),
            ..stored
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE runtimes SET descriptor_path = ?2, runtime_root = ?3, java_path = ?4, \
                 microemulator_path = ?5, game_path = ?6 \
             WHERE runtime_id = ?1 AND descriptor_sha256 = ?7",
            params![
                relocated.runtime_id,
                path_text(&relocated.descriptor_path)?,
                path_text(&relocated.runtime_root)?,
                path_text(&relocated.java_path)?,
                path_text(&relocated.microemulator_path)?,
                path_text(&relocated.game_path)?,
                relocated.descriptor_sha256,
            ],
        )?;
        if updated != 1 {
            return Err(CoreError::RuntimeRegistryMismatch {
                runtime_id: relocated.runtime_id,
            });
        }
        let bumped = transaction.execute(
            "UPDATE schema_metadata SET global_revision = global_revision + 1 \
             WHERE singleton = 1",
            [],
        )?;
        if bumped != 1 {
            return Err(CoreError::UnmanagedDatabase);
        }
        transaction.commit()?;
        if runtime_matches_registry_record(&relocated, &validated) {
            self.record_preflight_success(&relocated, validation);
        }
        Ok(relocated)
    }

    /// Re-pins a runtime row whose identity still matches but whose content has been replaced.
    ///
    /// Why this exists: the data root stores `descriptor_sha256`, so rebuilding the game jar used to
    /// raise `RuntimeRegistryMismatch` on the next open, and the only cure was deleting the data
    /// root — which deletes every account. Following the new content is the cheaper trade, given the
    /// content is validated in full here exactly as it was at registration.
    ///
    /// What it still refuses: `target_os`, `target_arch` and `game_bundle`. Those three are what
    /// make the row *this* runtime rather than another one wearing its id, and every bound profile
    /// would silently follow them. A descriptor that changes any of them is a different runtime and
    /// belongs under a different `runtime_id`, so it fails closed as before.
    fn repin_pinned_runtime(
        &mut self,
        stored: RuntimeRecord,
        validation: RuntimePreflightValidation,
    ) -> CoreResult<RuntimeRecord> {
        let validated = validation.runtime.clone();
        if stored.target_os != validated.target_os
            || stored.target_arch != validated.target_arch
            || stored.game_bundle != validated.game_bundle
        {
            return Err(CoreError::RuntimeRegistryMismatch {
                runtime_id: stored.runtime_id,
            });
        }
        let repin = RuntimeRepin {
            runtime_id: stored.runtime_id.clone(),
            previous_descriptor_sha256: stored.descriptor_sha256.clone(),
            descriptor_sha256: validated.descriptor_sha256.clone(),
            previous_game_sha256: stored.game_sha256.clone(),
            game_sha256: validated.game_sha256.clone(),
            previous_microemulator_sha256: stored.microemulator_sha256.clone(),
            microemulator_sha256: validated.microemulator_sha256.clone(),
            previous_jre_manifest_sha256: stored.jre_manifest_sha256.clone(),
            jre_manifest_sha256: validated.jre_manifest_sha256.clone(),
            repinned_at_unix_ms: validated.validated_at_unix_ms,
        };
        // Identity and creation time are carried over from the stored row; everything the descriptor
        // describes is taken from the validation that just passed.
        let repinned = RuntimeRecord {
            descriptor_sha256: validated.descriptor_sha256.clone(),
            descriptor_path: validated.descriptor_path.clone(),
            runtime_root: validated.runtime_root.clone(),
            java_path: validated.java_path.clone(),
            java_vendor: validated.java_vendor.clone(),
            java_version: validated.java_version.clone(),
            jre_manifest_sha256: validated.jre_manifest_sha256.clone(),
            microemulator_path: validated.microemulator_path.clone(),
            microemulator_version: validated.microemulator_version.clone(),
            microemulator_sha256: validated.microemulator_sha256.clone(),
            game_path: validated.game_path.clone(),
            game_sha256: validated.game_sha256.clone(),
            capability_state: validated.capability_state,
            validation_reason: validated.validation_reason.clone(),
            validated_at_unix_ms: validated.validated_at_unix_ms,
            ..stored.clone()
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // `descriptor_sha256` is UNIQUE. Another row already holding the new digest means two
        // descriptors claim one runtime tree; report it as a conflict rather than letting the
        // constraint surface as a bare SQLite error.
        let digest_owner: Option<String> = transaction
            .query_row(
                "SELECT runtime_id FROM runtimes \
                 WHERE descriptor_sha256 = ?1 AND runtime_id <> ?2",
                params![repinned.descriptor_sha256, repinned.runtime_id],
                |row| row.get(0),
            )
            .optional()?;
        if digest_owner.is_some() {
            return Err(CoreError::RuntimeConflict {
                runtime_id: repinned.runtime_id,
            });
        }
        let updated = transaction.execute(
            "UPDATE runtimes SET descriptor_sha256 = ?2, descriptor_path = ?3, runtime_root = ?4, \
                 java_path = ?5, java_vendor = ?6, java_version = ?7, jre_manifest_sha256 = ?8, \
                 microemulator_path = ?9, microemulator_version = ?10, \
                 microemulator_sha256 = ?11, game_path = ?12, game_sha256 = ?13, \
                 capability_state = ?14, validation_reason = ?15, validated_at_unix_ms = ?16 \
             WHERE runtime_id = ?1 AND descriptor_sha256 = ?17",
            params![
                repinned.runtime_id,
                repinned.descriptor_sha256,
                path_text(&repinned.descriptor_path)?,
                path_text(&repinned.runtime_root)?,
                path_text(&repinned.java_path)?,
                repinned.java_vendor,
                repinned.java_version,
                repinned.jre_manifest_sha256,
                path_text(&repinned.microemulator_path)?,
                repinned.microemulator_version,
                repinned.microemulator_sha256,
                path_text(&repinned.game_path)?,
                repinned.game_sha256,
                capability_text(repinned.capability_state),
                repinned.validation_reason,
                repinned.validated_at_unix_ms,
                repin.previous_descriptor_sha256,
            ],
        )?;
        if updated != 1 {
            return Err(CoreError::RuntimeRegistryMismatch {
                runtime_id: repinned.runtime_id,
            });
        }
        let bumped = transaction.execute(
            "UPDATE schema_metadata SET global_revision = global_revision + 1 \
             WHERE singleton = 1",
            [],
        )?;
        if bumped != 1 {
            return Err(CoreError::UnmanagedDatabase);
        }
        transaction.commit()?;
        // The preflight key carries the digest, so the old entry could never be looked up again;
        // dropping it keeps a fingerprint for an unpinned tree from lingering in the cache.
        self.runtime_preflight_cache_mut()
            .invalidate(&RuntimePreflightKey::from_record(&stored));
        self.record_runtime_repin(repin);
        self.record_preflight_success(&repinned, validation);
        Ok(repinned)
    }

    fn record_preflight_success(
        &mut self,
        record: &RuntimeRecord,
        validation: RuntimePreflightValidation,
    ) {
        if !runtime_matches_registry_record(record, &validation.runtime) {
            return;
        }
        self.runtime_preflight_cache_mut().record_success(
            RuntimePreflightKey::from_record(record),
            validation.jre_metadata_fingerprint,
            RuntimePreflightMode::FullValidation,
            validation.content_bytes_hashed,
            validation.jre_content_bytes_hashed,
        );
    }

    pub fn list_runtimes(
        &self,
        after_runtime_id: Option<&str>,
        limit: u32,
    ) -> CoreResult<RuntimePage> {
        if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
            return Err(CoreError::InvalidPageLimit {
                maximum: MAX_PAGE_LIMIT,
            });
        }
        let cursor = after_runtime_id.unwrap_or("");
        if !cursor.is_empty() && !is_valid_runtime_id(cursor) {
            return Err(CoreError::RuntimeValidation {
                code: "runtime_cursor_invalid",
            });
        }
        let sql = format!(
            "SELECT {RUNTIME_COLUMNS} FROM runtimes \
             WHERE runtime_id > ?1 ORDER BY runtime_id LIMIT ?2"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let fetch_limit = i64::from(limit) + 1;
        let mut items = statement
            .query_map(params![cursor, fetch_limit], row_to_runtime)?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = items.len() > limit as usize;
        if has_more {
            items.pop();
        }
        let next_cursor = has_more
            .then(|| items.last().map(|runtime| runtime.runtime_id.clone()))
            .flatten();
        Ok(RuntimePage { items, next_cursor })
    }

    pub fn inspect_runtime(&self, runtime_id: &str) -> CoreResult<RuntimeRecord> {
        if !is_valid_runtime_id(runtime_id) {
            return Err(CoreError::RuntimeValidation {
                code: "runtime_id_invalid",
            });
        }
        let sql = format!("SELECT {RUNTIME_COLUMNS} FROM runtimes WHERE runtime_id = ?1");
        self.connection
            .query_row(&sql, params![runtime_id], row_to_runtime)
            .optional()?
            .ok_or_else(|| CoreError::RuntimeNotFound {
                runtime_id: runtime_id.to_owned(),
            })
    }

    fn register_validated_runtime(
        &mut self,
        validated: ValidatedRuntime,
    ) -> CoreResult<RuntimeRecord> {
        validate_persistable_runtime(&validated)?;
        let now = Self::now_unix_ms()?;
        let record = RuntimeRecord::from_validated(validated, now);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let by_id_sql = format!("SELECT {RUNTIME_COLUMNS} FROM runtimes WHERE runtime_id = ?1");
        let existing = transaction
            .query_row(&by_id_sql, params![record.runtime_id], row_to_runtime)
            .optional()?;
        if let Some(existing) = existing {
            if existing.descriptor_sha256 == record.descriptor_sha256 {
                transaction.commit()?;
                return Ok(existing);
            }
            return Err(CoreError::RuntimeConflict {
                runtime_id: record.runtime_id,
            });
        }

        let digest_owner: Option<String> = transaction
            .query_row(
                "SELECT runtime_id FROM runtimes WHERE descriptor_sha256 = ?1",
                params![record.descriptor_sha256],
                |row| row.get(0),
            )
            .optional()?;
        if digest_owner.is_some() {
            return Err(CoreError::RuntimeConflict {
                runtime_id: record.runtime_id,
            });
        }

        transaction.execute(
            "INSERT INTO runtimes (\
                runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os, \
                target_arch, java_path, java_vendor, java_version, jre_manifest_sha256, \
                microemulator_path, microemulator_version, microemulator_sha256, game_path, \
                game_bundle, game_sha256, capability_state, validation_reason, \
                validated_at_unix_ms, created_at_unix_ms\
             ) VALUES (\
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20\
             )",
            params![
                record.runtime_id,
                record.descriptor_sha256,
                path_text(&record.descriptor_path)?,
                path_text(&record.runtime_root)?,
                record.target_os,
                record.target_arch,
                path_text(&record.java_path)?,
                record.java_vendor,
                record.java_version,
                record.jre_manifest_sha256,
                path_text(&record.microemulator_path)?,
                record.microemulator_version,
                record.microemulator_sha256,
                path_text(&record.game_path)?,
                record.game_bundle,
                record.game_sha256,
                capability_text(record.capability_state),
                record.validation_reason,
                record.validated_at_unix_ms,
                record.created_at_unix_ms,
            ],
        )?;
        let updated = transaction.execute(
            "UPDATE schema_metadata SET global_revision = global_revision + 1 \
             WHERE singleton = 1",
            [],
        )?;
        if updated != 1 {
            return Err(CoreError::UnmanagedDatabase);
        }
        transaction.commit()?;
        Ok(record)
    }

    #[cfg(test)]
    fn runtime_count_for_test(&self) -> i64 {
        self.connection
            .query_row("SELECT count(*) FROM runtimes", [], |row| row.get(0))
            .expect("read runtime count")
    }

    #[cfg(test)]
    fn global_revision_for_test(&self) -> i64 {
        self.connection
            .query_row(
                "SELECT global_revision FROM schema_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("read global revision")
    }
}

impl RuntimeRecord {
    fn from_validated(runtime: ValidatedRuntime, created_at_unix_ms: i64) -> Self {
        Self {
            runtime_id: runtime.runtime_id,
            descriptor_sha256: runtime.descriptor_sha256,
            descriptor_path: runtime.descriptor_path,
            runtime_root: runtime.runtime_root,
            target_os: runtime.target_os,
            target_arch: runtime.target_arch,
            java_path: runtime.java_path,
            java_vendor: runtime.java_vendor,
            java_version: runtime.java_version,
            jre_manifest_sha256: runtime.jre_manifest_sha256,
            microemulator_path: runtime.microemulator_path,
            microemulator_version: runtime.microemulator_version,
            microemulator_sha256: runtime.microemulator_sha256,
            game_path: runtime.game_path,
            game_bundle: runtime.game_bundle,
            game_sha256: runtime.game_sha256,
            capability_state: runtime.capability_state,
            validation_reason: runtime.validation_reason,
            validated_at_unix_ms: runtime.validated_at_unix_ms,
            created_at_unix_ms,
        }
    }
}

fn validate_persistable_runtime(runtime: &ValidatedRuntime) -> CoreResult<()> {
    if runtime.capability_state != CapabilityState::NeedsValidation {
        return Err(CoreError::RuntimeValidation {
            code: "capability_state_not_allowed",
        });
    }
    if !is_valid_runtime_id(&runtime.runtime_id) {
        return Err(CoreError::RuntimeValidation {
            code: "runtime_id_invalid",
        });
    }
    for digest in [
        &runtime.descriptor_sha256,
        &runtime.jre_manifest_sha256,
        &runtime.microemulator_sha256,
        &runtime.game_sha256,
    ] {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CoreError::RuntimeValidation {
                code: "sha256_invalid",
            });
        }
    }
    if runtime.validation_reason.len() > 512 || runtime.validated_at_unix_ms <= 0 {
        return Err(CoreError::RuntimeValidation {
            code: "runtime_record_invalid",
        });
    }
    for path in [
        &runtime.descriptor_path,
        &runtime.runtime_root,
        &runtime.java_path,
        &runtime.microemulator_path,
        &runtime.game_path,
    ] {
        let text = path_text(path)?;
        if text.is_empty() || text.len() > 4096 {
            return Err(CoreError::RuntimeValidation {
                code: "artifact_path_invalid",
            });
        }
    }
    Ok(())
}

pub(crate) fn is_valid_runtime_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}

fn path_text(path: &Path) -> CoreResult<&str> {
    path.to_str().ok_or(CoreError::RuntimeValidation {
        code: "artifact_path_invalid",
    })
}

fn capability_text(capability: CapabilityState) -> &'static str {
    match capability {
        CapabilityState::Supported => "Supported",
        CapabilityState::NeedsValidation => "NeedsValidation",
        CapabilityState::Unavailable => "Unavailable",
        CapabilityState::OutOfScope => "OutOfScope",
    }
}

fn row_to_runtime(row: &Row<'_>) -> rusqlite::Result<RuntimeRecord> {
    let capability: String = row.get(16)?;
    let capability_state = match capability.as_str() {
        "Supported" => CapabilityState::Supported,
        "NeedsValidation" => CapabilityState::NeedsValidation,
        "Unavailable" => CapabilityState::Unavailable,
        "OutOfScope" => CapabilityState::OutOfScope,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(RuntimeRecord {
        runtime_id: row.get(0)?,
        descriptor_sha256: row.get(1)?,
        descriptor_path: PathBuf::from(row.get::<_, String>(2)?),
        runtime_root: PathBuf::from(row.get::<_, String>(3)?),
        target_os: row.get(4)?,
        target_arch: row.get(5)?,
        java_path: PathBuf::from(row.get::<_, String>(6)?),
        java_vendor: row.get(7)?,
        java_version: row.get(8)?,
        jre_manifest_sha256: row.get(9)?,
        microemulator_path: PathBuf::from(row.get::<_, String>(10)?),
        microemulator_version: row.get(11)?,
        microemulator_sha256: row.get(12)?,
        game_path: PathBuf::from(row.get::<_, String>(13)?),
        game_bundle: row.get(14)?,
        game_sha256: row.get(15)?,
        capability_state,
        validation_reason: row.get(17)?,
        validated_at_unix_ms: row.get(18)?,
        created_at_unix_ms: row.get(19)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use crate::runtime::{CapabilityState, ValidatedRuntime};
    use crate::{CoreError, CoreState};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("zeus-runtime-registry-{label}-{}", Uuid::new_v4())),
            )
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn registration_is_idempotent_but_runtime_id_is_immutable() {
        let root = TestDirectory::new("immutable");
        let mut core = CoreState::open_at(root.path()).expect("open Core");
        let runtime = sample_runtime("runtime-a", 1, CapabilityState::NeedsValidation);

        let first = core
            .register_validated_runtime(runtime.clone())
            .expect("register runtime");
        let repeated = core
            .register_validated_runtime(runtime)
            .expect("repeat identical registration");
        assert_eq!(first, repeated);
        assert_eq!(core.runtime_count_for_test(), 1);
        assert_eq!(core.global_revision_for_test(), 1);

        let conflict = core.register_validated_runtime(sample_runtime(
            "runtime-a",
            2,
            CapabilityState::NeedsValidation,
        ));
        assert!(matches!(
            conflict,
            Err(CoreError::RuntimeConflict { runtime_id }) if runtime_id == "runtime-a"
        ));
        assert_eq!(core.runtime_count_for_test(), 1);
        assert_eq!(core.global_revision_for_test(), 1);
    }

    #[test]
    fn foundation_refuses_to_persist_supported_capability() {
        let root = TestDirectory::new("capability");
        let mut core = CoreState::open_at(root.path()).expect("open Core");

        assert!(matches!(
            core.register_validated_runtime(sample_runtime(
                "runtime-supported",
                1,
                CapabilityState::Supported,
            )),
            Err(CoreError::RuntimeValidation {
                code: "capability_state_not_allowed"
            })
        ));
        assert_eq!(core.runtime_count_for_test(), 0);
    }

    #[test]
    fn runtime_listing_is_bounded_and_uses_a_stable_keyset_cursor() {
        let root = TestDirectory::new("page");
        let mut core = CoreState::open_at(root.path()).expect("open Core");
        for index in 0..103 {
            core.register_validated_runtime(sample_runtime(
                &format!("runtime-{index:03}"),
                index + 1,
                CapabilityState::NeedsValidation,
            ))
            .expect("register paged runtime");
        }

        let first = core.list_runtimes(None, 100).expect("first page");
        assert_eq!(first.items.len(), 100);
        assert_eq!(first.items[0].runtime_id, "runtime-000");
        assert_eq!(first.items[99].runtime_id, "runtime-099");
        assert_eq!(first.next_cursor.as_deref(), Some("runtime-099"));

        let second = core
            .list_runtimes(first.next_cursor.as_deref(), 100)
            .expect("second page");
        assert_eq!(second.items.len(), 3);
        assert_eq!(second.items[0].runtime_id, "runtime-100");
        assert_eq!(second.next_cursor, None);

        assert!(matches!(
            core.list_runtimes(None, 101),
            Err(CoreError::InvalidPageLimit { maximum: 100 })
        ));
    }

    fn sample_runtime(
        runtime_id: &str,
        digest_seed: usize,
        capability_state: CapabilityState,
    ) -> ValidatedRuntime {
        let root = if cfg!(windows) {
            PathBuf::from(format!(r"C:\runtime\{runtime_id}"))
        } else {
            PathBuf::from(format!("/runtime/{runtime_id}"))
        };
        let digest = format!("{digest_seed:064x}");
        ValidatedRuntime {
            runtime_id: runtime_id.to_owned(),
            descriptor_sha256: digest.clone(),
            descriptor_path: root.join("runtime-descriptor.json"),
            runtime_root: root.clone(),
            target_os: if cfg!(windows) { "windows" } else { "ubuntu" }.to_owned(),
            target_arch: "x64".to_owned(),
            java_path: root.join(if cfg!(windows) {
                "jre/bin/java.exe"
            } else {
                "jre/bin/java"
            }),
            java_vendor: "Eclipse Adoptium".to_owned(),
            java_version: "11.0.32+9".to_owned(),
            jre_manifest_sha256: digest.clone(),
            microemulator_path: root.join("microemulator/microemulator.jar"),
            microemulator_version: "2.0.4".to_owned(),
            microemulator_sha256: digest.clone(),
            game_path: root.join("game/KnightOnline_402.jar"),
            game_bundle: "402".to_owned(),
            game_sha256: digest,
            capability_state,
            validation_reason: "runtime probes pending".to_owned(),
            validated_at_unix_ms: 1_800_000_000_000,
        }
    }
}

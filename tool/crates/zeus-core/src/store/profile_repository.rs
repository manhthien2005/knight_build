use std::io::ErrorKind;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use uuid::{Uuid, Version};

use crate::error::{CoreError, CoreResult};
use crate::profile::{
    DEFAULT_LAUNCH_POLICY_JSON, DEFAULT_PRESENTATION_JSON, MAX_PROFILE_PAGE_LIMIT, ProfilePage,
    ProfileRecord,
};

use super::CoreState;
use super::runtime_repository::is_valid_runtime_id;

const PROFILE_COLUMNS: &str = "profile_id, revision, display_name, runtime_id, \
    launch_policy_json, presentation_json, archived_at_unix_ms, \
    created_at_unix_ms, updated_at_unix_ms";

impl CoreState {
    pub fn create_profile(
        &mut self,
        display_name: &str,
        runtime_id: &str,
    ) -> CoreResult<ProfileRecord> {
        validate_display_name(display_name)?;
        validate_runtime_id(runtime_id)?;
        if !runtime_exists(&self.connection, runtime_id)? {
            return Err(CoreError::RuntimeNotFound {
                runtime_id: runtime_id.to_owned(),
            });
        }

        let now = Self::now_unix_ms()?;
        let (profile_id, _directory) = self.allocate_profile_directory()?;
        let record = ProfileRecord {
            profile_id: profile_id.clone(),
            revision: 1,
            display_name: display_name.to_owned(),
            runtime_id: runtime_id.to_owned(),
            launch_policy_json: DEFAULT_LAUNCH_POLICY_JSON.to_owned(),
            presentation_json: DEFAULT_PRESENTATION_JSON.to_owned(),
            archived_at_unix_ms: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
        };

        let transaction = match self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
        {
            Ok(transaction) => transaction,
            Err(error) => {
                let _ = self.data_root.remove_empty_profile_directory(&profile_id);
                return Err(error.into());
            }
        };
        if let Err(error) = insert_profile(&transaction, &record) {
            drop(transaction);
            let _ = self.data_root.remove_empty_profile_directory(&profile_id);
            return Err(error);
        }
        if let Err(error) = increment_global_revision(&transaction) {
            drop(transaction);
            let _ = self.data_root.remove_empty_profile_directory(&profile_id);
            return Err(error);
        }
        // A commit I/O failure can have an ambiguous durable outcome. Keep the empty UUID
        // directory in that case so a committed profile can never lose its isolation root.
        transaction.commit()?;
        Ok(record)
    }

    pub fn inspect_profile(&self, profile_id: &str) -> CoreResult<ProfileRecord> {
        validate_profile_id(profile_id)?;
        query_profile(&self.connection, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
            profile_id: profile_id.to_owned(),
        })
    }

    pub fn list_profiles(
        &self,
        after_profile_id: Option<&str>,
        limit: u32,
        include_archived: bool,
    ) -> CoreResult<ProfilePage> {
        if !(1..=MAX_PROFILE_PAGE_LIMIT).contains(&limit) {
            return Err(CoreError::InvalidPageLimit {
                maximum: MAX_PROFILE_PAGE_LIMIT,
            });
        }
        let cursor = after_profile_id.unwrap_or("");
        if !cursor.is_empty() {
            validate_profile_id(cursor)?;
        }
        let sql = format!(
            "SELECT {PROFILE_COLUMNS} FROM profiles \
             WHERE profile_id > ?1 AND (?2 = 1 OR archived_at_unix_ms IS NULL) \
             ORDER BY profile_id LIMIT ?3"
        );
        let fetch_limit = i64::from(limit) + 1;
        let mut statement = self.connection.prepare(&sql)?;
        let mut items = statement
            .query_map(
                params![cursor, i64::from(include_archived), fetch_limit],
                row_to_profile,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = items.len() > limit as usize;
        if has_more {
            items.pop();
        }
        let next_cursor = has_more
            .then(|| items.last().map(|profile| profile.profile_id.clone()))
            .flatten();
        Ok(ProfilePage { items, next_cursor })
    }

    pub fn rename_profile(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
        display_name: &str,
    ) -> CoreResult<ProfileRecord> {
        validate_profile_id(profile_id)?;
        validate_expected_revision(expected_revision)?;
        validate_display_name(display_name)?;
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated_at = monotonic_audit_time(&transaction, profile_id, wall_clock)?;
        let updated = transaction.execute(
            "UPDATE profiles SET display_name = ?1, revision = revision + 1, \
             updated_at_unix_ms = ?2 WHERE profile_id = ?3 AND revision = ?4",
            params![display_name, updated_at, profile_id, expected_revision],
        )?;
        require_single_revision_update(&transaction, profile_id, expected_revision, updated)?;
        increment_global_revision(&transaction)?;
        let result =
            query_profile(&transaction, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
                profile_id: profile_id.to_owned(),
            })?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn bind_profile_runtime(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
        runtime_id: &str,
    ) -> CoreResult<ProfileRecord> {
        validate_profile_id(profile_id)?;
        validate_expected_revision(expected_revision)?;
        validate_runtime_id(runtime_id)?;
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !runtime_exists(&transaction, runtime_id)? {
            return Err(CoreError::RuntimeNotFound {
                runtime_id: runtime_id.to_owned(),
            });
        }
        let updated_at = monotonic_audit_time(&transaction, profile_id, wall_clock)?;
        let updated = transaction.execute(
            "UPDATE profiles SET runtime_id = ?1, revision = revision + 1, \
             updated_at_unix_ms = ?2 WHERE profile_id = ?3 AND revision = ?4",
            params![runtime_id, updated_at, profile_id, expected_revision],
        )?;
        require_single_revision_update(&transaction, profile_id, expected_revision, updated)?;
        increment_global_revision(&transaction)?;
        let result =
            query_profile(&transaction, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
                profile_id: profile_id.to_owned(),
            })?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn archive_profile(
        &mut self,
        profile_id: &str,
        expected_revision: i64,
    ) -> CoreResult<ProfileRecord> {
        validate_profile_id(profile_id)?;
        validate_expected_revision(expected_revision)?;
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            query_profile(&transaction, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
                profile_id: profile_id.to_owned(),
            })?;
        if current.revision != expected_revision {
            return Err(CoreError::RevisionConflict {
                profile_id: profile_id.to_owned(),
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if current.archived_at_unix_ms.is_some() {
            transaction.commit()?;
            return Ok(current);
        }
        let archived_at = wall_clock.max(current.updated_at_unix_ms);
        let updated = transaction.execute(
            "UPDATE profiles SET archived_at_unix_ms = ?1, revision = revision + 1, \
             updated_at_unix_ms = ?1 WHERE profile_id = ?2 AND revision = ?3 \
             AND archived_at_unix_ms IS NULL",
            params![archived_at, profile_id, expected_revision],
        )?;
        require_single_revision_update(&transaction, profile_id, expected_revision, updated)?;
        increment_global_revision(&transaction)?;
        let result =
            query_profile(&transaction, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
                profile_id: profile_id.to_owned(),
            })?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn profile_directory(&self, profile_id: &str) -> CoreResult<PathBuf> {
        validate_profile_id(profile_id)?;
        self.data_root.verified_profile_directory(profile_id)
    }

    pub(super) fn allocate_profile_directory(&self) -> CoreResult<(String, PathBuf)> {
        for _ in 0..8 {
            let profile_id = Uuid::new_v4().to_string();
            match self.data_root.create_profile_directory(&profile_id) {
                Ok(path) => return Ok((profile_id, path)),
                Err(CoreError::Io { source, .. }) if source.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(CoreError::ProfileDirectoryCollision)
    }
}

fn validate_display_name(value: &str) -> CoreResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 128
        || value.len() > 512
        || value.chars().any(char::is_control)
    {
        return Err(CoreError::InvalidProfileName);
    }
    Ok(())
}

fn validate_profile_id(value: &str) -> CoreResult<()> {
    let parsed = Uuid::parse_str(value).map_err(|_| CoreError::InvalidProfileId)?;
    if parsed.get_version() != Some(Version::Random) || parsed.hyphenated().to_string() != value {
        return Err(CoreError::InvalidProfileId);
    }
    Ok(())
}

fn validate_expected_revision(value: i64) -> CoreResult<()> {
    if !(1..i64::MAX).contains(&value) {
        return Err(CoreError::InvalidRevision);
    }
    Ok(())
}

fn validate_runtime_id(value: &str) -> CoreResult<()> {
    if !is_valid_runtime_id(value) {
        return Err(CoreError::RuntimeValidation {
            code: "runtime_id_invalid",
        });
    }
    Ok(())
}

pub(super) fn insert_profile(
    transaction: &Transaction<'_>,
    profile: &ProfileRecord,
) -> CoreResult<()> {
    transaction.execute(
        "INSERT INTO profiles (\
            profile_id, revision, display_name, runtime_id, launch_policy_json, \
            presentation_json, archived_at_unix_ms, created_at_unix_ms, updated_at_unix_ms\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            profile.profile_id,
            profile.revision,
            profile.display_name,
            profile.runtime_id,
            profile.launch_policy_json,
            profile.presentation_json,
            profile.archived_at_unix_ms,
            profile.created_at_unix_ms,
            profile.updated_at_unix_ms,
        ],
    )?;
    Ok(())
}

pub(super) fn increment_global_revision(transaction: &Transaction<'_>) -> CoreResult<()> {
    let updated = transaction.execute(
        "UPDATE schema_metadata SET global_revision = global_revision + 1 WHERE singleton = 1",
        [],
    )?;
    if updated != 1 {
        return Err(CoreError::UnmanagedDatabase);
    }
    Ok(())
}

fn runtime_exists(connection: &Connection, runtime_id: &str) -> CoreResult<bool> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtimes WHERE runtime_id = ?1)",
        params![runtime_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists == 1)
}

pub(super) fn query_profile(
    connection: &Connection,
    profile_id: &str,
) -> CoreResult<Option<ProfileRecord>> {
    let sql = format!("SELECT {PROFILE_COLUMNS} FROM profiles WHERE profile_id = ?1");
    Ok(connection
        .query_row(&sql, params![profile_id], row_to_profile)
        .optional()?)
}

fn monotonic_audit_time(
    transaction: &Transaction<'_>,
    profile_id: &str,
    wall_clock: i64,
) -> CoreResult<i64> {
    let current =
        query_profile(transaction, profile_id)?.ok_or_else(|| CoreError::ProfileNotFound {
            profile_id: profile_id.to_owned(),
        })?;
    Ok(wall_clock.max(current.updated_at_unix_ms))
}

fn require_single_revision_update(
    transaction: &Transaction<'_>,
    profile_id: &str,
    expected_revision: i64,
    updated: usize,
) -> CoreResult<()> {
    if updated == 1 {
        return Ok(());
    }
    let actual = transaction
        .query_row(
            "SELECT revision FROM profiles WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    match actual {
        Some(actual) => Err(CoreError::RevisionConflict {
            profile_id: profile_id.to_owned(),
            expected: expected_revision,
            actual,
        }),
        None => Err(CoreError::ProfileNotFound {
            profile_id: profile_id.to_owned(),
        }),
    }
}

fn row_to_profile(row: &Row<'_>) -> rusqlite::Result<ProfileRecord> {
    Ok(ProfileRecord {
        profile_id: row.get(0)?,
        revision: row.get(1)?,
        display_name: row.get(2)?,
        runtime_id: row.get(3)?,
        launch_policy_json: row.get(4)?,
        presentation_json: row.get(5)?,
        archived_at_unix_ms: row.get(6)?,
        created_at_unix_ms: row.get(7)?,
        updated_at_unix_ms: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::params;
    use uuid::Uuid;

    use crate::{CoreError, CoreState};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("zeus-profile-repository-unit-{}", Uuid::new_v4())),
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
    fn profile_mutations_are_revisioned_on_every_supported_host() {
        let root = TestDirectory::new();
        let mut core = CoreState::open_at(root.path()).expect("open Core");
        seed_runtime(&core, "runtime-a");

        let created = core
            .create_profile("Account A", "runtime-a")
            .expect("create profile");
        assert_eq!(created.revision, 1);
        assert!(
            core.profile_directory(&created.profile_id)
                .expect("private profile directory")
                .is_dir()
        );

        let renamed = core
            .rename_profile(&created.profile_id, 1, "Account A renamed")
            .expect("rename profile");
        assert_eq!(renamed.revision, 2);
        assert!(matches!(
            core.rename_profile(&created.profile_id, 1, "stale"),
            Err(CoreError::RevisionConflict {
                expected: 1,
                actual: 2,
                ..
            })
        ));
        let archived = core
            .archive_profile(&created.profile_id, 2)
            .expect("archive profile");
        assert_eq!(archived.revision, 3);
        assert_eq!(
            core.archive_profile(&created.profile_id, 3)
                .expect("idempotent archive"),
            archived
        );
        assert!(
            core.list_profiles(None, 100, false)
                .expect("list active")
                .items
                .is_empty()
        );
        assert_eq!(
            core.list_profiles(None, 100, true)
                .expect("list archived")
                .items,
            vec![archived]
        );
    }

    fn seed_runtime(core: &CoreState, runtime_id: &str) {
        let digest = "1".repeat(64);
        core.connection
            .execute(
                "INSERT INTO runtimes (\
                    runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os, \
                    target_arch, java_path, java_vendor, java_version, jre_manifest_sha256, \
                    microemulator_path, microemulator_version, microemulator_sha256, game_path, \
                    game_bundle, game_sha256, capability_state, validation_reason, \
                    validated_at_unix_ms, created_at_unix_ms\
                 ) VALUES (\
                    ?1, ?2, ?3, ?4, ?5, 'x64', ?6, 'Eclipse Adoptium', '11.0.32+9', ?2, \
                    ?7, '2.0.4', ?2, ?8, '402', ?2, 'NeedsValidation', \
                    'runtime probes pending', 1800000000000, 1800000000000\
                 )",
                params![
                    runtime_id,
                    digest,
                    if cfg!(windows) {
                        r"C:\runtime\descriptor.json"
                    } else {
                        "/runtime/descriptor.json"
                    },
                    if cfg!(windows) {
                        r"C:\runtime"
                    } else {
                        "/runtime"
                    },
                    if cfg!(windows) { "windows" } else { "ubuntu" },
                    if cfg!(windows) {
                        r"C:\runtime\jre\bin\java.exe"
                    } else {
                        "/runtime/jre/bin/java"
                    },
                    if cfg!(windows) {
                        r"C:\runtime\microemulator.jar"
                    } else {
                        "/runtime/microemulator.jar"
                    },
                    if cfg!(windows) {
                        r"C:\runtime\game.jar"
                    } else {
                        "/runtime/game.jar"
                    },
                ],
            )
            .expect("seed immutable runtime");
    }
}

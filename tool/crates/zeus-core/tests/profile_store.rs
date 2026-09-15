#![cfg(windows)]

use std::fs;
use std::path::PathBuf;

use uuid::{Uuid, Version};
use zeus_core::{CoreError, CoreState};

#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;
use runtime_fixture::RuntimeFixture;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("zeus-profile-store-{}", Uuid::new_v4())))
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
fn profiles_are_uuid_isolated_revisioned_and_bounded() {
    let runtime_fixture = RuntimeFixture::new("profile-store");
    let data_root = TestDirectory::new();
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register fixture runtime");

    for invalid in ["", " leading", "trailing ", "line\nbreak"] {
        assert!(matches!(
            core.create_profile(invalid, &runtime.runtime_id),
            Err(CoreError::InvalidProfileName)
        ));
    }
    assert!(matches!(
        core.create_profile(&"a".repeat(129), &runtime.runtime_id),
        Err(CoreError::InvalidProfileName)
    ));
    assert!(matches!(
        core.create_profile("Missing runtime", "does-not-exist"),
        Err(CoreError::RuntimeNotFound { runtime_id }) if runtime_id == "does-not-exist"
    ));

    let created = core
        .create_profile("Account A", &runtime.runtime_id)
        .expect("create profile");
    let parsed_id = Uuid::parse_str(&created.profile_id).expect("profile UUID");
    assert_eq!(parsed_id.get_version(), Some(Version::Random));
    assert_eq!(created.revision, 1);
    assert_eq!(
        created.launch_policy_json,
        r#"{"priority":0,"stagger_class":"default"}"#
    );
    assert_eq!(created.presentation_json, "{}");
    let profile_directory = core
        .profile_directory(&created.profile_id)
        .expect("derive profile directory");
    assert_eq!(
        profile_directory,
        data_root
            .0
            .canonicalize()
            .expect("canonicalize data root")
            .join("profiles")
            .join(&created.profile_id)
    );
    assert!(profile_directory.is_dir());

    assert!(matches!(
        core.bind_profile_runtime(&created.profile_id, 1, "does-not-exist"),
        Err(CoreError::RuntimeNotFound { runtime_id }) if runtime_id == "does-not-exist"
    ));
    let renamed = core
        .rename_profile(&created.profile_id, 1, "Account A renamed")
        .expect("rename profile");
    assert_eq!(renamed.revision, 2);
    assert!(matches!(
        core.rename_profile(&created.profile_id, 1, "stale write"),
        Err(CoreError::RevisionConflict {
            expected: 1,
            actual: 2,
            ..
        })
    ));
    let rebound = core
        .bind_profile_runtime(&created.profile_id, 2, &runtime.runtime_id)
        .expect("bind runtime");
    assert_eq!(rebound.revision, 3);

    let archived = core
        .archive_profile(&created.profile_id, 3)
        .expect("archive profile");
    assert_eq!(archived.revision, 4);
    assert!(archived.archived_at_unix_ms.is_some());
    let repeated = core
        .archive_profile(&created.profile_id, 4)
        .expect("repeat archive at current revision");
    assert_eq!(repeated, archived);
    assert!(matches!(
        core.archive_profile(&created.profile_id, 3),
        Err(CoreError::RevisionConflict {
            expected: 3,
            actual: 4,
            ..
        })
    ));
    assert_eq!(
        core.inspect_profile(&created.profile_id)
            .expect("inspect archived profile"),
        archived
    );
    assert!(matches!(
        core.inspect_profile("not-a-uuid"),
        Err(CoreError::InvalidProfileId)
    ));

    for index in 0..103 {
        core.create_profile(&format!("Profile {index:03}"), &runtime.runtime_id)
            .expect("create paged profile");
    }
    let first = core
        .list_profiles(None, 100, false)
        .expect("first active profile page");
    assert_eq!(first.items.len(), 100);
    assert!(first.next_cursor.is_some());
    assert!(
        first
            .items
            .iter()
            .all(|profile| profile.archived_at_unix_ms.is_none())
    );
    let second = core
        .list_profiles(first.next_cursor.as_deref(), 100, false)
        .expect("second active profile page");
    assert_eq!(second.items.len(), 3);
    assert_eq!(second.next_cursor, None);

    assert!(matches!(
        core.list_profiles(None, 0, false),
        Err(CoreError::InvalidPageLimit { maximum: 100 })
    ));
    assert!(matches!(
        core.inspect_profile(&Uuid::new_v4().to_string()),
        Err(CoreError::ProfileNotFound { .. })
    ));
}

#[test]
fn vault_key_is_exact_private_state_and_is_not_recreated_for_existing_accounts() {
    let runtime_fixture = RuntimeFixture::new("vault-key-state");
    let data_root = TestDirectory::new();
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register runtime");
    let profile = core
        .create_profile("Vault profile", &runtime.runtime_id)
        .expect("create profile");
    let key_path = data_root.0.join("vault.key");
    let key = fs::read(&key_path).expect("read vault key");
    assert_eq!(key.len(), 44);
    assert_eq!(&key[..8], b"ZEUSVLT1");
    assert_eq!(&key[8..12], &1_u32.to_le_bytes());
    assert!(core.state_files_are_private().expect("private state files"));
    drop(core);

    let connection =
        rusqlite::Connection::open(data_root.0.join("state.sqlite3")).expect("open test database");
    connection
        .execute(
            "INSERT INTO accounts (
               account_id, revision, username, username_key, profile_id,
               credential_version, password_cipher, password_nonce, password_tag,
               config_schema_version, config_revision, config_json,
               last_run_at_unix_ms, last_outcome, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (
               '00000000-0000-4000-8000-000000000001', 1, 'VaultUser', 'vaultuser',
               ?1, 1, X'01',
               X'000000000000000000000000', X'00000000000000000000000000000000',
               1, 1,
               '{\"schema\":1,\"login\":{\"task_timeout_ms\":60000,\"ready_timeout_ms\":30000,\"menu_settle_ms\":6000,\"screen_transition_ms\":2000,\"editor_settle_ms\":750,\"foreground_retry_timeout_ms\":5000}}',
               NULL, NULL, 1, 1
             )",
            [&profile.profile_id],
        )
        .expect("seed encrypted account row");
    drop(connection);
    fs::remove_file(&key_path).expect("remove key");

    let reopened = CoreState::open_at(&data_root.0).expect("metadata remains openable");
    assert!(
        !key_path.exists(),
        "vault key must not be replaced while account rows exist"
    );
    assert!(
        reopened
            .state_files_are_private()
            .expect("remaining state files private")
    );
}

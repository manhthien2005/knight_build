use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
#[cfg(windows)]
use rusqlite::params;
use uuid::Uuid;
use zeus_core::{CoreError, CoreState, MAX_DATABASE_BYTES, SCHEMA_VERSION};

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::ptr::null_mut;
#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetFileSecurityW, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SetFileSecurityW,
};

#[cfg(windows)]
#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;
#[cfg(windows)]
use runtime_fixture::RuntimeFixture;

const V1_SCHEMA_SQL: &str = r#"
CREATE TABLE schema_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    global_revision INTEGER NOT NULL CHECK (global_revision >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0)
) STRICT;

CREATE TABLE runtimes (
    runtime_id TEXT PRIMARY KEY CHECK (length(runtime_id) BETWEEN 1 AND 160),
    descriptor_sha256 TEXT NOT NULL CHECK (length(descriptor_sha256) = 64),
    descriptor_path TEXT NOT NULL CHECK (length(descriptor_path) BETWEEN 1 AND 4096),
    runtime_root TEXT NOT NULL CHECK (length(runtime_root) BETWEEN 1 AND 4096),
    target_os TEXT NOT NULL CHECK (target_os IN ('windows', 'ubuntu')),
    target_arch TEXT NOT NULL CHECK (target_arch IN ('x64')),
    java_path TEXT NOT NULL CHECK (length(java_path) BETWEEN 1 AND 4096),
    java_vendor TEXT NOT NULL CHECK (length(java_vendor) BETWEEN 1 AND 128),
    java_version TEXT NOT NULL CHECK (length(java_version) BETWEEN 1 AND 128),
    jre_manifest_sha256 TEXT NOT NULL CHECK (length(jre_manifest_sha256) = 64),
    microemulator_path TEXT NOT NULL CHECK (length(microemulator_path) BETWEEN 1 AND 4096),
    microemulator_version TEXT NOT NULL CHECK (length(microemulator_version) BETWEEN 1 AND 64),
    microemulator_sha256 TEXT NOT NULL CHECK (length(microemulator_sha256) = 64),
    game_path TEXT NOT NULL CHECK (length(game_path) BETWEEN 1 AND 4096),
    game_bundle TEXT NOT NULL CHECK (length(game_bundle) BETWEEN 1 AND 64),
    game_sha256 TEXT NOT NULL CHECK (length(game_sha256) = 64),
    capability_state TEXT NOT NULL CHECK (
        capability_state IN ('Supported', 'NeedsValidation', 'Unavailable', 'OutOfScope')
    ),
    validation_reason TEXT NOT NULL CHECK (length(validation_reason) <= 512),
    validated_at_unix_ms INTEGER NOT NULL CHECK (validated_at_unix_ms > 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),
    UNIQUE (descriptor_sha256)
) STRICT;

CREATE TABLE profiles (
    profile_id TEXT PRIMARY KEY CHECK (length(profile_id) = 36),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 128),
    runtime_id TEXT NOT NULL REFERENCES runtimes(runtime_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    launch_policy_json TEXT NOT NULL CHECK (length(launch_policy_json) BETWEEN 2 AND 2048),
    presentation_json TEXT NOT NULL CHECK (length(presentation_json) BETWEEN 2 AND 2048),
    archived_at_unix_ms INTEGER,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms),
    CHECK (archived_at_unix_ms IS NULL OR archived_at_unix_ms >= created_at_unix_ms)
) STRICT;

CREATE INDEX profiles_active_keyset
    ON profiles (archived_at_unix_ms, profile_id);
CREATE INDEX profiles_runtime_binding
    ON profiles (runtime_id, profile_id);
"#;

const V2_ACCOUNT_COLUMNS: &[&str] = &[
    "account_id",
    "revision",
    "username",
    "username_key",
    "profile_id",
    "credential_version",
    "password_cipher",
    "password_nonce",
    "password_tag",
    "config_schema_version",
    "config_revision",
    "config_json",
    "last_run_at_unix_ms",
    "last_outcome",
    "created_at_unix_ms",
    "updated_at_unix_ms",
];

const V2_ACCOUNT_DDL: &str = r#"
CREATE TABLE accounts (
    account_id TEXT PRIMARY KEY CHECK (length(account_id) = 36),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    username TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 64),
    username_key TEXT NOT NULL UNIQUE CHECK (length(username_key) BETWEEN 1 AND 64),
    profile_id TEXT NOT NULL UNIQUE REFERENCES profiles(profile_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    credential_version INTEGER NOT NULL CHECK (credential_version = 1),
    password_cipher BLOB NOT NULL CHECK (length(password_cipher) BETWEEN 1 AND 128),
    password_nonce BLOB NOT NULL CHECK (length(password_nonce) = 12),
    password_tag BLOB NOT NULL CHECK (length(password_tag) = 16),
    config_schema_version INTEGER NOT NULL CHECK (config_schema_version = 1),
    config_revision INTEGER NOT NULL CHECK (config_revision >= 1),
    config_json TEXT NOT NULL CHECK (length(config_json) BETWEEN 2 AND 2048),
    last_run_at_unix_ms INTEGER,
    last_outcome TEXT CHECK (
        last_outcome IS NULL OR last_outcome IN ('Started', 'LoginFailed')
    ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms),
    CHECK (last_run_at_unix_ms IS NULL OR last_run_at_unix_ms >= created_at_unix_ms)
) STRICT
"#;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("zeus-core-{label}-{}", Uuid::new_v4()));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn write_literal_v1_fixture(root: &Path) -> PathBuf {
    drop(CoreState::open_at(root).expect("initialize managed root and marker"));
    let database = root.join("state.sqlite3");
    let connection = Connection::open(&database).expect("open literal v1 fixture");
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;\
             DROP TABLE IF EXISTS accounts;\
             DROP TABLE IF EXISTS profiles;\
             DROP TABLE IF EXISTS runtimes;\
             DROP TABLE IF EXISTS schema_metadata;\
             DROP TABLE IF EXISTS spots;",
        )
        .expect("remove current schema from v1 fixture");
    connection
        .execute_batch(V1_SCHEMA_SQL)
        .expect("create literal committed v1 schema");
    connection
        .execute(
            "INSERT INTO schema_metadata \
             (singleton, schema_version, global_revision, created_at_unix_ms) \
             VALUES (1, 1, 41, 1700000000123)",
            [],
        )
        .expect("insert literal v1 metadata row");
    connection
        .execute_batch(
            "INSERT INTO runtimes (\
                 runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os,\
                 target_arch, java_path, java_vendor, java_version, jre_manifest_sha256,\
                 microemulator_path, microemulator_version, microemulator_sha256, game_path,\
                 game_bundle, game_sha256, capability_state, validation_reason,\
                 validated_at_unix_ms, created_at_unix_ms\
             ) VALUES (\
                 'fixture-runtime', lower(hex(zeroblob(32))), 'D:/fixture/descriptor.json',\
                 'D:/fixture', 'windows', 'x64', 'D:/fixture/java.exe', 'Temurin', '11.0.32+9',\
                 lower(hex(zeroblob(32))), 'D:/fixture/microemulator.jar', '2.0.4',\
                 lower(hex(zeroblob(32))), 'D:/fixture/game.jar', 'ko402',\
                 lower(hex(zeroblob(32))), 'NeedsValidation', 'literal-v1',\
                 1700000000123, 1700000000123\
             );\
             INSERT INTO profiles (\
                 profile_id, revision, display_name, runtime_id, launch_policy_json,\
                 presentation_json, archived_at_unix_ms, created_at_unix_ms, updated_at_unix_ms\
             ) VALUES (\
                 '00000000-0000-4000-8000-000000000001', 7, 'Literal v1 profile',\
                 'fixture-runtime', '{}', '{}', NULL, 1700000000123, 1700000000456\
             );",
        )
        .expect("insert literal v1 runtime and profile rows");
    connection
        .pragma_update(None, "user_version", 1)
        .expect("set literal v1 user version");
    drop(connection);
    database
}

fn rebuild_metadata_with_self_fk(connection: &Connection) {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;\
             CREATE TABLE schema_metadata_with_self_fk (\
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
                 schema_version INTEGER NOT NULL CHECK (schema_version = 1),\
                 global_revision INTEGER NOT NULL CHECK (global_revision >= 0),\
                 created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),\
                 FOREIGN KEY (singleton) REFERENCES schema_metadata_with_self_fk(singleton)\
                     ON UPDATE RESTRICT ON DELETE RESTRICT\
             ) STRICT;\
             INSERT INTO schema_metadata_with_self_fk \
                 (singleton, schema_version, global_revision, created_at_unix_ms) \
                 SELECT singleton, schema_version, global_revision, created_at_unix_ms \
                 FROM schema_metadata;\
             DROP TABLE schema_metadata;\
             ALTER TABLE schema_metadata_with_self_fk RENAME TO schema_metadata;",
        )
        .expect("rebuild metadata with satisfiable self foreign key");
}

fn rebuild_populated_runtime_with_self_fk(connection: &Connection) {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;\
             INSERT INTO runtimes (\
                 runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os,\
                 target_arch, java_path, java_vendor, java_version, jre_manifest_sha256,\
                 microemulator_path, microemulator_version, microemulator_sha256, game_path,\
                 game_bundle, game_sha256, capability_state, validation_reason,\
                 validated_at_unix_ms, created_at_unix_ms\
             ) VALUES (\
                 'self-fk-runtime', lower(hex(zeroblob(32))), 'D:/self-fk/descriptor.json',\
                 'D:/self-fk', 'windows', 'x64', 'D:/self-fk/java.exe', 'Temurin', '11.0.32+9',\
                 lower(hex(zeroblob(32))), 'D:/self-fk/microemulator.jar', '2.0.4',\
                 lower(hex(zeroblob(32))), 'D:/self-fk/game.jar', 'ko402',\
                 lower(hex(zeroblob(32))), 'NeedsValidation', 'self-fk',\
                 1700000000123, 1700000000123\
             );\
             CREATE TABLE runtimes_with_self_fk (\
                 runtime_id TEXT PRIMARY KEY CHECK (length(runtime_id) BETWEEN 1 AND 160),\
                 descriptor_sha256 TEXT NOT NULL CHECK (length(descriptor_sha256) = 64),\
                 descriptor_path TEXT NOT NULL CHECK (length(descriptor_path) BETWEEN 1 AND 4096),\
                 runtime_root TEXT NOT NULL CHECK (length(runtime_root) BETWEEN 1 AND 4096),\
                 target_os TEXT NOT NULL CHECK (target_os IN ('windows', 'ubuntu')),\
                 target_arch TEXT NOT NULL CHECK (target_arch IN ('x64')),\
                 java_path TEXT NOT NULL CHECK (length(java_path) BETWEEN 1 AND 4096),\
                 java_vendor TEXT NOT NULL CHECK (length(java_vendor) BETWEEN 1 AND 128),\
                 java_version TEXT NOT NULL CHECK (length(java_version) BETWEEN 1 AND 128),\
                 jre_manifest_sha256 TEXT NOT NULL CHECK (length(jre_manifest_sha256) = 64),\
                 microemulator_path TEXT NOT NULL CHECK (length(microemulator_path) BETWEEN 1 AND 4096),\
                 microemulator_version TEXT NOT NULL CHECK (length(microemulator_version) BETWEEN 1 AND 64),\
                 microemulator_sha256 TEXT NOT NULL CHECK (length(microemulator_sha256) = 64),\
                 game_path TEXT NOT NULL CHECK (length(game_path) BETWEEN 1 AND 4096),\
                 game_bundle TEXT NOT NULL CHECK (length(game_bundle) BETWEEN 1 AND 64),\
                 game_sha256 TEXT NOT NULL CHECK (length(game_sha256) = 64),\
                 capability_state TEXT NOT NULL CHECK (\
                     capability_state IN ('Supported', 'NeedsValidation', 'Unavailable', 'OutOfScope')\
                 ),\
                 validation_reason TEXT NOT NULL CHECK (length(validation_reason) <= 512),\
                 validated_at_unix_ms INTEGER NOT NULL CHECK (validated_at_unix_ms > 0),\
                 created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),\
                 UNIQUE (descriptor_sha256),\
                 FOREIGN KEY (runtime_id) REFERENCES runtimes_with_self_fk(runtime_id)\
                     ON UPDATE RESTRICT ON DELETE RESTRICT\
             ) STRICT;\
             INSERT INTO runtimes_with_self_fk (\
                 runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os,\
                 target_arch, java_path, java_vendor, java_version, jre_manifest_sha256,\
                 microemulator_path, microemulator_version, microemulator_sha256, game_path,\
                 game_bundle, game_sha256, capability_state, validation_reason,\
                 validated_at_unix_ms, created_at_unix_ms\
             ) SELECT \
                 runtime_id, descriptor_sha256, descriptor_path, runtime_root, target_os,\
                 target_arch, java_path, java_vendor, java_version, jre_manifest_sha256,\
                 microemulator_path, microemulator_version, microemulator_sha256, game_path,\
                 game_bundle, game_sha256, capability_state, validation_reason,\
                 validated_at_unix_ms, created_at_unix_ms \
             FROM runtimes;\
             DROP TABLE runtimes;\
             ALTER TABLE runtimes_with_self_fk RENAME TO runtimes;",
        )
        .expect("rebuild populated runtime with satisfiable self foreign key");
}

fn read_user_version(root: &Path) -> i64 {
    Connection::open(root.join("state.sqlite3"))
        .expect("open database to read user version")
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read user version")
}

fn assert_root_marker_v1(root: &Path) {
    assert_eq!(
        fs::read(root.join(".zeus-hso-root")).expect("read exact root marker"),
        b"ZEUS_HSO_DATA_ROOT\nschema_version=1\n"
    );
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_exact_account_schema(root: &Path) {
    let schema = Connection::open(root.join("state.sqlite3")).expect("open v2 account schema");
    let account_sql = schema
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'accounts'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read account DDL");
    assert_eq!(normalized_sql(&account_sql), normalized_sql(V2_ACCOUNT_DDL));
    assert_eq!(
        schema
            .query_row(
                "SELECT count(*) FROM sqlite_schema \
                 WHERE type = 'index' AND tbl_name = 'accounts' AND sql IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count explicit account indexes"),
        0
    );
}

#[test]
fn rejects_relative_and_unc_data_roots() {
    assert!(matches!(
        CoreState::open_at(Path::new("relative-data")),
        Err(CoreError::InvalidDataRoot { .. })
    ));

    #[cfg(windows)]
    assert!(matches!(
        CoreState::open_at(Path::new(r"\\server\share\zeus-hso")),
        Err(CoreError::InvalidDataRoot { .. })
    ));
}

#[test]
fn refuses_to_adopt_nonempty_unmanaged_directory() {
    let test_dir = TestDirectory::new("unmanaged");
    fs::create_dir_all(test_dir.path()).expect("create unmanaged root");
    fs::write(test_dir.path().join("foreign.txt"), b"do not touch").expect("write sentinel");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDataRoot)
    ));
    assert_eq!(
        fs::read(test_dir.path().join("foreign.txt")).expect("sentinel remains"),
        b"do not touch"
    );
}

#[test]
fn second_core_is_rejected_before_database_ownership() {
    if let Some(root) = std::env::var_os("ZEUS_CORE_LOCK_HELPER_ROOT") {
        run_lock_helper(Path::new(&root));
        return;
    }

    let test_dir = TestDirectory::new("lock");
    let mut helper =
        Command::new(std::env::current_exe().expect("resolve integration test binary"))
            .args([
                "--exact",
                "second_core_is_rejected_before_database_ownership",
                "--nocapture",
            ])
            .env("ZEUS_CORE_LOCK_HELPER_ROOT", test_dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn first Core helper process");
    wait_for_helper_ready(&mut helper, &test_dir.path().join("helper-ready"));

    let second = CoreState::open_at(test_dir.path());
    fs::write(test_dir.path().join("helper-release"), b"release")
        .expect("release first Core helper");
    let output = helper
        .wait_with_output()
        .expect("wait for first Core helper process");
    assert!(
        output.status.success(),
        "Core lock helper failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(matches!(second, Err(CoreError::AlreadyRunning)));
    CoreState::open_at(test_dir.path()).expect("lock is released when Core drops");
}

#[test]
fn initializes_private_bounded_rollback_journal_schema() {
    let test_dir = TestDirectory::new("schema");
    let core = CoreState::open_at(test_dir.path()).expect("open Core state");
    let invariants = core.database_invariants().expect("read invariants");

    assert_eq!(SCHEMA_VERSION, 3);
    assert_eq!(invariants.schema_version, 3);
    assert_eq!(invariants.journal_mode, "delete");
    assert!(invariants.foreign_keys);
    assert_eq!(invariants.connection_count, 1);
    assert!(invariants.page_size * invariants.max_page_count <= MAX_DATABASE_BYTES);
    assert!(core.data_root_is_private().expect("inspect data root"));
    assert!(
        core.state_files_are_private()
            .expect("inspect state-file permissions")
    );

    assert_eq!(
        core.schema_tables().expect("read exact schema tables"),
        vec![
            "accounts",
            "profiles",
            "runtimes",
            "schema_metadata",
            "spots"
        ]
    );
    assert_eq!(
        core.schema_columns("accounts")
            .expect("read exact account columns"),
        V2_ACCOUNT_COLUMNS
    );
    assert_eq!(read_user_version(test_dir.path()), 3);
    assert_root_marker_v1(test_dir.path());
    assert_exact_account_schema(test_dir.path());

    let schema = Connection::open(test_dir.path().join("state.sqlite3"))
        .expect("open fresh v2 schema fixture");
    assert!(
        schema
            .execute(
                "UPDATE schema_metadata SET schema_version = 1 WHERE singleton = 1",
                [],
            )
            .is_err(),
        "fresh metadata table must enforce CHECK (schema_version = 3)"
    );

    let forbidden = ["username", "password", "token", "secret", "credential"];
    for table in ["runtimes", "profiles"] {
        for column in core.schema_columns(table).expect("read schema columns") {
            let normalized = column.to_ascii_lowercase();
            assert!(
                forbidden.iter().all(|word| !normalized.contains(word)),
                "forbidden secret-like column {table}.{column}"
            );
        }
    }
}

#[test]
fn migrates_literal_v1_to_current_preserving_metadata_lkg_and_root_marker() {
    // A v1 root opens on the current schema in one go, through every intermediate step: an operator who
    // skipped a release must not be told their database is unmanaged.
    let test_dir = TestDirectory::new("literal-v1-to-current");
    let database = write_literal_v1_fixture(test_dir.path());
    let before = fs::read(&database).expect("snapshot literal v1 database");

    let migrated = CoreState::open_at(test_dir.path()).expect("migrate literal v1 database");
    assert_eq!(
        migrated.database_invariants().unwrap().schema_version,
        SCHEMA_VERSION
    );
    assert_eq!(
        read_user_version(test_dir.path()),
        i64::from(SCHEMA_VERSION)
    );
    assert_eq!(
        migrated.schema_tables().unwrap(),
        vec![
            "accounts",
            "profiles",
            "runtimes",
            "schema_metadata",
            "spots"
        ]
    );
    assert_eq!(
        migrated.schema_columns("accounts").unwrap(),
        V2_ACCOUNT_COLUMNS
    );
    assert_exact_account_schema(test_dir.path());
    let metadata = Connection::open(test_dir.path().join("state.sqlite3"))
        .expect("open migrated database metadata")
        .query_row(
            "SELECT singleton, schema_version, global_revision, created_at_unix_ms \
             FROM schema_metadata",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read preserved migrated metadata");
    // global_revision and created_at survive both steps: the root was created when it was created, and
    // the migration is not a new install.
    assert_eq!(metadata, (1, 3, 41, 1700000000123_i64));
    let preserved = Connection::open(test_dir.path().join("state.sqlite3"))
        .expect("open migrated preserved rows")
        .query_row(
            "SELECT r.runtime_id, p.profile_id, p.revision, p.display_name \
             FROM runtimes r JOIN profiles p ON p.runtime_id = r.runtime_id",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .expect("read preserved v1 runtime and profile");
    assert_eq!(
        preserved,
        (
            "fixture-runtime".to_owned(),
            "00000000-0000-4000-8000-000000000001".to_owned(),
            7,
            "Literal v1 profile".to_owned(),
        )
    );
    assert_root_marker_v1(test_dir.path());

    let lkg = fs::read(test_dir.path().join("state.lkg.sqlite3")).expect("read v1 LKG");
    assert_eq!(
        lkg, before,
        "LKG must be the byte-exact pre-migration v1 DB"
    );
    let backup = Connection::open_with_flags(
        test_dir.path().join("state.lkg.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open v1 LKG");
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn rejects_mismatched_v1_version_sources_without_migrating() {
    let test_dir = TestDirectory::new("v1-version-mismatch");
    let database = write_literal_v1_fixture(test_dir.path());
    let connection = Connection::open(&database).expect("open mismatched v1 fixture");
    connection
        .pragma_update(None, "user_version", 2)
        .expect("mismatch user version from metadata");
    drop(connection);
    let before = fs::read(&database).expect("snapshot mismatched fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[test]
fn rejects_future_metadata_version_without_mutating_database() {
    let test_dir = TestDirectory::new("future-metadata-version");
    drop(CoreState::open_at(test_dir.path()).expect("initialize v2 fixture"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open future metadata fixture");
    connection
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;\
             UPDATE schema_metadata SET schema_version = 4 WHERE singleton = 1;",
        )
        .expect("set future metadata version");
    drop(connection);
    let before = fs::read(&database).expect("snapshot future metadata fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnsupportedSchema {
            found: 4,
            supported: SCHEMA_VERSION
        })
    ));
    assert_eq!(fs::read(database).unwrap(), before);
}

#[test]
fn rejects_unknown_v1_table_or_column_without_migrating() {
    for alteration in [
        "CREATE TABLE foreign_state (value TEXT) STRICT;",
        "ALTER TABLE profiles ADD COLUMN foreign_value TEXT;",
    ] {
        let test_dir = TestDirectory::new("v1-unknown-shape");
        let database = write_literal_v1_fixture(test_dir.path());
        let connection = Connection::open(&database).expect("open unknown v1 fixture");
        connection
            .execute_batch(alteration)
            .expect("alter v1 fixture");
        drop(connection);
        let before = fs::read(&database).expect("snapshot unknown fixture");

        assert!(matches!(
            CoreState::open_at(test_dir.path()),
            Err(CoreError::UnmanagedDatabase)
        ));
        assert_eq!(fs::read(database).unwrap(), before);
        assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
    }
}

#[test]
fn rejects_unknown_v2_account_column_without_mutating_database() {
    let test_dir = TestDirectory::new("v2-unknown-account-column");
    drop(CoreState::open_at(test_dir.path()).expect("initialize v2 fixture"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open v2 fixture");
    connection
        .execute_batch("ALTER TABLE accounts ADD COLUMN foreign_value TEXT;")
        .expect("add unknown v2 account column");
    drop(connection);
    let before = fs::read(&database).expect("snapshot unknown v2 fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
}

#[test]
fn rejects_generated_column_in_populated_v1_before_lkg_or_migration() {
    let test_dir = TestDirectory::new("v1-generated-column");
    let database = write_literal_v1_fixture(test_dir.path());
    let connection = Connection::open(&database).expect("open v1 generated-column fixture");
    connection
        .execute_batch(
            "ALTER TABLE profiles ADD COLUMN derived_name TEXT \
             GENERATED ALWAYS AS (display_name) VIRTUAL;",
        )
        .expect("add generated v1 column");
    drop(connection);
    let before = fs::read(&database).expect("snapshot generated-column v1 fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[test]
fn rejects_generated_column_in_v2_without_mutating_database() {
    let test_dir = TestDirectory::new("v2-generated-column");
    drop(CoreState::open_at(test_dir.path()).expect("initialize v2 fixture"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open v2 generated-column fixture");
    connection
        .execute_batch(
            "ALTER TABLE accounts ADD COLUMN username_shadow TEXT \
             GENERATED ALWAYS AS (username) VIRTUAL;",
        )
        .expect("add generated v2 column");
    drop(connection);
    let before = fs::read(&database).expect("snapshot generated-column v2 fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[test]
fn rejects_unexpected_metadata_self_fk_in_populated_v1_before_lkg_or_migration() {
    let test_dir = TestDirectory::new("v1-metadata-self-fk");
    let database = write_literal_v1_fixture(test_dir.path());
    let connection = Connection::open(&database).expect("open v1 metadata self-FK fixture");
    rebuild_metadata_with_self_fk(&connection);
    drop(connection);
    let before = fs::read(&database).expect("snapshot metadata self-FK v1 fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[test]
fn rejects_unexpected_runtime_self_fk_in_populated_v2_without_mutation() {
    let test_dir = TestDirectory::new("v2-runtime-self-fk");
    drop(CoreState::open_at(test_dir.path()).expect("initialize v2 fixture"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open v2 runtime self-FK fixture");
    rebuild_populated_runtime_with_self_fk(&connection);
    drop(connection);
    let before = fs::read(&database).expect("snapshot runtime self-FK v2 fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnmanagedDatabase)
    ));
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[cfg(windows)]
#[test]
fn rejects_permissive_managed_state_file_acl_without_mutating_database() {
    for name in [".zeus-hso-root", ".core-instance.lock", "state.sqlite3"] {
        let test_dir = TestDirectory::new("state-acl-reject");
        drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
        let database = test_dir.path().join("state.sqlite3");
        let before = fs::read(&database).expect("read database before ACL rejection");
        set_everyone_full_control(&test_dir.path().join(name));

        assert!(matches!(
            CoreState::open_at(test_dir.path()),
            Err(CoreError::InsecureDataRoot)
        ));
        assert_eq!(
            fs::read(&database).expect("read database after ACL rejection"),
            before,
            "database changed while rejecting permissive {name}"
        );
    }
}

#[cfg(windows)]
#[test]
fn rejects_reparse_data_root() {
    let test_dir = TestDirectory::new("reparse");
    fs::create_dir_all(test_dir.path()).expect("create reparse fixture parent");
    let target = test_dir.path().join("target");
    let link = test_dir.path().join("link");
    fs::create_dir(&target).expect("create reparse target");
    junction::create(&target, &link).expect("create unprivileged directory junction");

    assert!(matches!(
        CoreState::open_at(&link),
        Err(CoreError::InvalidDataRoot { .. })
    ));
    junction::delete(&link).expect("delete fixture junction without following it");
}

#[test]
fn rejects_database_larger_than_hard_cap_before_sqlite_open() {
    let test_dir = TestDirectory::new("oversize");
    drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
    let database = test_dir.path().join("state.sqlite3");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&database)
        .expect("open database fixture");
    file.set_len(MAX_DATABASE_BYTES + 1)
        .expect("create sparse oversized fixture");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::DatabaseTooLarge { .. })
    ));
}

#[test]
fn rejects_newer_schema_without_mutating_it() {
    let test_dir = TestDirectory::new("newer-schema");
    drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open database fixture");
    connection
        .pragma_update(None, "user_version", 4)
        .expect("set future schema version");
    drop(connection);
    let before = fs::read(&database).expect("snapshot future database");

    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnsupportedSchema {
            found: 4,
            supported: SCHEMA_VERSION
        })
    ));
    assert_eq!(
        fs::read(&database).expect("read future database after rejection"),
        before,
        "Core mutated the future database before rejecting it"
    );
    let connection = Connection::open(&database).expect("reopen future database");
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read unchanged future version"),
        4
    );
}

#[test]
fn rejects_newer_schema_visible_only_in_live_wal_without_mutating_it() {
    let test_dir = TestDirectory::new("newer-schema-live-wal");
    drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open future WAL fixture");
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row
                .get::<_, String>(0))
            .expect("enable live WAL"),
        "wal"
    );
    connection
        .execute_batch("PRAGMA wal_autocheckpoint = 0; PRAGMA user_version = 4;")
        .expect("commit future schema version into WAL");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for suffix in ["-wal", "-shm"] {
            fs::set_permissions(
                PathBuf::from(format!("{}{suffix}", database.display())),
                fs::Permissions::from_mode(0o600),
            )
            .expect("make SQLite sidecar private");
        }
    }

    let before = fs::read(&database).expect("snapshot database before future-schema rejection");
    assert!(matches!(
        CoreState::open_at(test_dir.path()),
        Err(CoreError::UnsupportedSchema {
            found: 4,
            supported: SCHEMA_VERSION
        })
    ));
    assert_eq!(
        fs::read(&database).expect("read database after future-schema rejection"),
        before,
        "Core mutated the main database before rejecting the live future WAL"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read live future version"),
        4
    );
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .expect("read unchanged live journal mode"),
        "wal"
    );
}

/// Walks a Core-created root back to the committed v2 shape, keeping its rows.
///
/// Built by Core and then downgraded rather than written from scratch: the ACLs a root needs are Core's
/// to grant, and a hand-built directory is refused as insecure before any migration runs.
fn write_v2_fixture(root: &Path) -> PathBuf {
    drop(CoreState::open_at(root).expect("initialize managed root"));
    let database = root.join("state.sqlite3");
    let connection = Connection::open(&database).expect("open v2 fixture");
    connection
        .execute_batch(
            "DROP TABLE spots;\
             ALTER TABLE schema_metadata RENAME TO _current;\
             CREATE TABLE schema_metadata (\
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
                 schema_version INTEGER NOT NULL CHECK (schema_version = 2),\
                 global_revision INTEGER NOT NULL CHECK (global_revision >= 0),\
                 created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0)\
             ) STRICT;\
             INSERT INTO schema_metadata \
             SELECT singleton, 2, global_revision, created_at_unix_ms FROM _current;\
             DROP TABLE _current;\
             PRAGMA user_version = 2;",
        )
        .expect("walk the fixture back to v2");
    drop(connection);
    database
}

#[test]
fn creates_bounded_lkg_before_migrating_an_existing_v2_database() {
    // A v2 root is the one an operator actually upgrades from, and it is the only case where a failed
    // migration has rows to lose: the backup must exist before the migration touches anything.
    let test_dir = TestDirectory::new("lkg-v2");
    let database = write_v2_fixture(test_dir.path());
    let before = fs::read(&database).expect("snapshot the v2 database");

    let core = CoreState::open_at(test_dir.path()).expect("migrate the v2 database");
    assert_eq!(
        core.database_invariants()
            .expect("read migrated schema")
            .schema_version,
        SCHEMA_VERSION
    );

    let lkg = test_dir.path().join("state.lkg.sqlite3");
    assert_eq!(
        fs::read(&lkg).expect("LKG backup exists"),
        before,
        "the backup must be the database as it stood before the migration"
    );
    assert!(fs::metadata(&lkg).expect("LKG backup metadata").len() <= MAX_DATABASE_BYTES);
    let backup = Connection::open_with_flags(&lkg, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open LKG backup read-only");
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read LKG schema version"),
        2
    );
}

/// Opening an already-current root writes no backup: there is no migration to protect, and a rewrite
/// every launch would replace a good backup with a copy of a database that may since have gone bad.
#[test]
fn opening_a_current_database_leaves_no_lkg_backup() {
    let test_dir = TestDirectory::new("lkg-current");
    drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
    drop(CoreState::open_at(test_dir.path()).expect("reopen the current root"));
    assert!(!test_dir.path().join("state.lkg.sqlite3").exists());
}

#[test]
fn creates_bounded_lkg_before_migrating_an_existing_v0_database() {
    let test_dir = TestDirectory::new("lkg");
    drop(CoreState::open_at(test_dir.path()).expect("initialize managed root"));
    let database = test_dir.path().join("state.sqlite3");
    let connection = Connection::open(&database).expect("open database fixture");
    connection
        .execute_batch(
            "DROP TABLE accounts;\
             DROP TABLE profiles;\
             DROP TABLE runtimes;\
             DROP TABLE schema_metadata;\
             DROP TABLE spots;\
             PRAGMA user_version = 0;",
        )
        .expect("convert fixture to empty version zero database");
    drop(connection);

    let core = CoreState::open_at(test_dir.path()).expect("migrate version zero database");
    assert_eq!(
        core.database_invariants()
            .expect("read migrated schema")
            .schema_version,
        SCHEMA_VERSION
    );
    let lkg = test_dir.path().join("state.lkg.sqlite3");
    assert!(fs::metadata(&lkg).expect("LKG backup exists").len() <= MAX_DATABASE_BYTES);
    let backup = Connection::open_with_flags(&lkg, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open LKG backup read-only");
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read LKG schema version"),
        0
    );
}

#[cfg(windows)]
fn set_everyone_full_control(path: &Path) {
    let sddl: Vec<u16> = "D:P(A;;FA;;;WD)".encode_utf16().chain([0]).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the SDDL and output pointer are valid for the duration of the call.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    assert_ne!(converted, 0, "create permissive test DACL");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: both buffers are initialized and remain live through this call.
    let applied = unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    // SAFETY: the descriptor was allocated by the SDDL conversion API via LocalAlloc.
    unsafe {
        LocalFree(descriptor.cast());
    }
    assert_ne!(applied, 0, "apply permissive test DACL");
}

fn run_lock_helper(root: &Path) {
    let _core = CoreState::open_at(root).expect("helper Core owns root");
    fs::write(root.join("helper-ready"), b"ready").expect("signal helper readiness");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("helper-release").exists() {
        assert!(
            Instant::now() < deadline,
            "parent did not release lock helper"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_helper_ready(helper: &mut std::process::Child, ready: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        if let Some(status) = helper.try_wait().expect("poll Core lock helper") {
            panic!("Core lock helper exited before readiness: {status}");
        }
        if Instant::now() >= deadline {
            let _ = helper.kill();
            let _ = helper.wait();
            panic!("Core lock helper did not become ready");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
fn populate_portable_source_root(root: &Path, descriptor: &Path) -> String {
    let profile_id = {
        let mut core = CoreState::open_at(root).expect("initialize portable source root");
        let runtime = core
            .register_runtime_descriptor(descriptor)
            .expect("register source runtime");
        core.create_profile("Portable source", &runtime.runtime_id)
            .expect("create source profile")
            .profile_id
    };
    let connection = Connection::open(root.join("state.sqlite3")).expect("open source database");
    connection
        .execute(
            "INSERT INTO accounts (\
                 account_id, revision, username, username_key, profile_id, credential_version,\
                 password_cipher, password_nonce, password_tag, config_schema_version,\
                 config_revision, config_json, created_at_unix_ms, updated_at_unix_ms\
             ) VALUES (\
                 ?1, 1, 'PortableUser', 'portableuser', ?2, 1, X'0102030405060708',\
                 X'000102030405060708090a0b', X'000102030405060708090a0b0c0d0e0f', 1, 1, '{}',\
                 1700000000123, 1700000000123\
             )",
            params![Uuid::new_v4().to_string(), profile_id],
        )
        .expect("insert source account row");
    drop(connection);
    profile_id
}

#[cfg(windows)]
fn copy_tree_with_inherited_acls(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create inheriting destination directory");
    for entry in fs::read_dir(source).expect("enumerate copy source") {
        let entry = entry.expect("read copy source entry");
        let target = destination.join(entry.file_name());
        if entry
            .metadata()
            .expect("copy source entry metadata")
            .is_dir()
        {
            copy_tree_with_inherited_acls(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy source file");
        }
    }
}

#[cfg(windows)]
fn security_descriptor_text(path: &Path, requested: u32) -> String {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut needed = 0u32;
    // SAFETY: the first call intentionally passes an empty buffer to size the descriptor.
    unsafe {
        GetFileSecurityW(wide.as_ptr(), requested, null_mut(), 0, &mut needed);
    }
    assert_ne!(needed, 0, "size security descriptor of {}", path.display());
    let mut words = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    let descriptor: PSECURITY_DESCRIPTOR = words.as_mut_ptr().cast();
    // SAFETY: the aligned buffer holds at least `needed` bytes and every pointer is valid.
    let read =
        unsafe { GetFileSecurityW(wide.as_ptr(), requested, descriptor, needed, &mut needed) };
    assert_ne!(read, 0, "read security descriptor of {}", path.display());
    let mut raw = null_mut();
    let mut length = 0u32;
    // SAFETY: the descriptor is initialized and both out parameters are valid.
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            requested,
            &mut raw,
            &mut length,
        )
    };
    assert_ne!(converted, 0, "format security descriptor");
    // SAFETY: the API returned a NUL-terminated buffer of `length` code units.
    let text = unsafe {
        OsString::from_wide(std::slice::from_raw_parts(raw, length as usize))
            .to_string_lossy()
            .into_owned()
    };
    // SAFETY: the string buffer was allocated by the conversion API via LocalAlloc.
    unsafe {
        LocalFree(raw.cast());
    }
    text.trim_end_matches('\0').to_owned()
}

#[cfg(windows)]
fn owner_sid(path: &Path) -> String {
    let owner = security_descriptor_text(path, OWNER_SECURITY_INFORMATION);
    let owner = owner
        .strip_prefix("O:")
        .expect("descriptor exposes an owner")
        .to_owned();
    assert!(owner.starts_with("S-1-"), "owner must be a literal SID");
    owner
}

#[cfg(windows)]
fn assert_protected_current_user_security(path: &Path, expected_owner: &str) {
    assert_eq!(
        owner_sid(path),
        expected_owner,
        "owner of {} must be the current user",
        path.display()
    );
    let dacl = security_descriptor_text(path, DACL_SECURITY_INFORMATION)
        .strip_prefix("D:")
        .expect("descriptor exposes a DACL")
        .to_owned();
    assert!(
        dacl.starts_with('P'),
        "DACL of {} must be protected, found {dacl}",
        path.display()
    );
    // `AI` (SE_DACL_AUTO_INHERITED) is load-bearing, not cosmetic: it marks a DACL written by an
    // auto-inheritance-aware API, which propagates inheritable ACEs into existing children. Spec
    // section 6 forbids repairing unknown descendants, so the repair must use a non-propagating
    // write and this flag must stay clear.
    assert!(
        !dacl.contains("AI"),
        "DACL of {} must not be auto-inherited, found {dacl}",
        path.display()
    );
    assert_eq!(
        dacl.matches("(A;").count(),
        2,
        "DACL of {} must allow exactly the current user and System, found {dacl}",
        path.display()
    );
}

#[cfg(windows)]
fn assert_inherited_security_unchanged(path: &Path) {
    let dacl = security_descriptor_text(path, DACL_SECURITY_INFORMATION)
        .strip_prefix("D:")
        .expect("descriptor exposes a DACL")
        .to_owned();
    assert!(
        !dacl.starts_with('P'),
        "security of {} must stay inherited until repair succeeds, found {dacl}",
        path.display()
    );
}

#[cfg(windows)]
fn current_user_private_owner(label: &str) -> String {
    let reference = TestDirectory::new(label);
    drop(CoreState::open_at(reference.path()).expect("create reference private root"));
    owner_sid(reference.path())
}

#[cfg(windows)]
#[test]
fn portable_open_repairs_inherited_acl_copy_of_populated_root() {
    let runtime = RuntimeFixture::new("portable-repair");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-source");
    let profile_id = populate_portable_source_root(source.path(), runtime.descriptor_path());
    // A previously-launched profile owns the two known launch directories, and an unknown descendant
    // that repair must never touch.
    let source_profile = source.path().join("profiles").join(&profile_id);
    for child in ["microemu-home", "temp"] {
        fs::create_dir_all(source_profile.join(child)).expect("create launch directory");
    }
    let untouched_relative = Path::new("profiles")
        .join(&profile_id)
        .join("microemu-home")
        .join("operator-data.bin");
    fs::write(source.path().join(&untouched_relative), b"descendant")
        .expect("write unknown descendant");
    let copy = TestDirectory::new("portable-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    let untouched = copy.path().join(&untouched_relative);
    let untouched_before = security_descriptor_text(&untouched, DACL_SECURITY_INFORMATION);

    // An Explorer-style copy carries inherited parent ACLs, so the strict path must refuse it.
    assert!(matches!(
        CoreState::open_at(copy.path()),
        Err(CoreError::InsecureDataRoot)
    ));

    let expected_owner = current_user_private_owner("portable-owner-reference");
    {
        let core = CoreState::open_portable_at(copy.path(), &runtime_root)
            .expect("repair and open the copied root");
        assert!(core.data_root_is_private().expect("repaired root privacy"));
        assert!(
            core.state_files_are_private()
                .expect("repaired state file privacy")
        );
        let profile = core
            .inspect_profile(&profile_id)
            .expect("preserved profile");
        assert_eq!(profile.revision, 1);
        // The load-bearing outcome: a copied, previously-launched root must still launch. Before the
        // known launch directories were repaired, this failed with `InsecureDataRoot`.
        let snapshot = core
            .prepare_launch_snapshot(&profile_id, profile.revision)
            .expect("repaired copy must still produce a launch snapshot");
        // Snapshot paths are canonicalized, so compare against the canonical root.
        let canonical_copy = fs::canonicalize(copy.path()).expect("canonicalize repaired copy");
        assert!(snapshot.microemu_home().starts_with(&canonical_copy));
        assert!(snapshot.temp_directory().starts_with(&canonical_copy));
    }

    assert_protected_current_user_security(copy.path(), &expected_owner);
    for relative in ["vault.key", ".zeus-hso-root", "state.sqlite3", "profiles"] {
        assert_protected_current_user_security(&copy.path().join(relative), &expected_owner);
    }
    for child in ["microemu-home", "temp"] {
        assert_protected_current_user_security(
            &copy.path().join("profiles").join(&profile_id).join(child),
            &expected_owner,
        );
    }
    // Repair never walks unknown descendants, so this file keeps the copy's inherited security.
    assert_eq!(
        security_descriptor_text(&untouched, DACL_SECURITY_INFORMATION),
        untouched_before,
        "repair must not touch descendants below a known launch directory"
    );
    assert_protected_current_user_security(
        &copy.path().join("profiles").join(&profile_id),
        &expected_owner,
    );

    let connection =
        Connection::open(copy.path().join("state.sqlite3")).expect("open repaired copy");
    let username: String = connection
        .query_row("SELECT username FROM accounts", [], |row| row.get(0))
        .expect("preserved account row");
    assert_eq!(username, "PortableUser");
    let bound: String = connection
        .query_row("SELECT profile_id FROM accounts", [], |row| row.get(0))
        .expect("preserved account binding");
    assert_eq!(bound, profile_id);
}

#[cfg(windows)]
#[test]
fn portable_open_rejects_linked_profile_entry_before_changing_security() {
    let runtime = RuntimeFixture::new("portable-linked");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-linked-source");
    populate_portable_source_root(source.path(), runtime.descriptor_path());
    let copy = TestDirectory::new("portable-linked-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    let target = TestDirectory::new("portable-linked-target");
    fs::create_dir_all(target.path()).expect("create junction target");
    let link = copy
        .path()
        .join("profiles")
        .join(Uuid::new_v4().to_string());
    junction::create(target.path(), &link).expect("create profile junction");

    assert!(matches!(
        CoreState::open_portable_at(copy.path(), &runtime_root),
        Err(CoreError::PortableRepair { code }) if code == "linked_entry"
    ));
    // The linked entry is rejected before any security change, so the copy stays inherited.
    assert_inherited_security_unchanged(copy.path());
    assert_inherited_security_unchanged(&copy.path().join("state.sqlite3"));
    assert!(matches!(
        CoreState::open_at(copy.path()),
        Err(CoreError::InsecureDataRoot)
    ));

    junction::delete(&link).expect("delete profile junction");
}

#[cfg(windows)]
#[test]
fn portable_open_accepts_the_saved_spot_book_and_its_publish_temporary() {
    // The book is a managed root file, so a data root holding one still opens. It was not, once: an
    // unlisted root file made repair report `unknown_root_file`, Core refused to open, and the only
    // symptom the operator saw was "tool not ready" with an account list that would not load.
    let runtime = RuntimeFixture::new("portable-spots");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-spots-source");
    populate_portable_source_root(source.path(), runtime.descriptor_path());
    let copy = TestDirectory::new("portable-spots-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    fs::write(copy.path().join("zeus-spots.txt"), b"1=0,480,720\n").expect("write the spot book");
    // And a publish temporary, which a process killed between write and rename leaves behind: it must
    // not lock the operator out of their own data root either.
    fs::write(
        copy.path().join(concat!(
            "zeus-spots.txt.tmp-",
            "73667c38-7f9b-4ab1-bc75-4b278218b0a0"
        )),
        b"1=0,480,720\n",
    )
    .expect("write a publish temporary");

    let core = CoreState::open_portable_at(copy.path(), &runtime_root)
        .expect("a data root holding a spot book opens");
    drop(core);
}

#[cfg(windows)]
#[test]
fn portable_open_rejects_unknown_root_entry_shape_before_changing_security() {
    let runtime = RuntimeFixture::new("portable-unknown");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-unknown-source");
    populate_portable_source_root(source.path(), runtime.descriptor_path());
    let copy = TestDirectory::new("portable-unknown-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    fs::write(copy.path().join("operator-notes.txt"), b"unexpected")
        .expect("write unexpected root entry");

    assert!(matches!(
        CoreState::open_portable_at(copy.path(), &runtime_root),
        Err(CoreError::PortableRepair { code }) if code == "unknown_root_file"
    ));
    assert_inherited_security_unchanged(copy.path());
    assert_inherited_security_unchanged(&copy.path().join("state.sqlite3"));
    assert!(matches!(
        CoreState::open_at(copy.path()),
        Err(CoreError::InsecureDataRoot)
    ));
}

#[cfg(windows)]
#[test]
fn portable_open_accepts_an_interrupted_vault_key_publish_residue() {
    let runtime = RuntimeFixture::new("portable-residue");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-residue-source");
    populate_portable_source_root(source.path(), runtime.descriptor_path());
    let copy = TestDirectory::new("portable-residue-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    // A kill between create and rename during vault-key publish leaves this file behind. It must not
    // permanently brick an otherwise legitimate root.
    let residue = copy
        .path()
        .join(format!(".vault.key.tmp-{}", Uuid::new_v4()));
    fs::write(&residue, b"partial").expect("write interrupted publish residue");

    let expected_owner = current_user_private_owner("portable-residue-reference");
    drop(
        CoreState::open_portable_at(copy.path(), &runtime_root)
            .expect("residue must not block repair"),
    );

    assert_protected_current_user_security(&residue, &expected_owner);

    // A residue name that is not a v4 UUID stays unknown.
    let bogus = copy.path().join(".vault.key.tmp-not-a-uuid");
    fs::write(&bogus, b"partial").expect("write malformed residue");
    assert!(matches!(
        CoreState::open_portable_at(copy.path(), &runtime_root),
        Err(CoreError::PortableRepair { code }) if code == "unknown_root_file"
    ));
}

#[cfg(windows)]
#[test]
fn portable_open_rejects_nonempty_unmarked_root() {
    let runtime = RuntimeFixture::new("portable-unmanaged");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let unmanaged = TestDirectory::new("portable-unmanaged-root");
    fs::create_dir_all(unmanaged.path()).expect("create unmanaged directory");
    fs::write(unmanaged.path().join("foreign.txt"), b"foreign").expect("write foreign file");

    assert!(matches!(
        CoreState::open_portable_at(unmanaged.path(), &runtime_root),
        Err(CoreError::UnmanagedDataRoot)
    ));
    assert!(!unmanaged.path().join(".zeus-hso-root").exists());
}

#[cfg(windows)]
#[test]
fn portable_open_creates_a_missing_root_through_the_private_create_path() {
    let runtime = RuntimeFixture::new("portable-missing");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let expected_owner = current_user_private_owner("portable-missing-reference");
    let fresh = TestDirectory::new("portable-missing-root");
    assert!(!fresh.path().exists());

    {
        let core = CoreState::open_portable_at(fresh.path(), &runtime_root)
            .expect("create and open a fresh portable root");
        assert!(core.data_root_is_private().expect("fresh root privacy"));
    }
    assert_root_marker_v1(fresh.path());
    assert_protected_current_user_security(fresh.path(), &expected_owner);
}

#[cfg(windows)]
#[test]
fn portable_open_accepts_an_empty_directory_only_through_the_fresh_create_path() {
    let runtime = RuntimeFixture::new("portable-empty");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let empty = TestDirectory::new("portable-empty-root");
    fs::create_dir_all(empty.path()).expect("create inherited empty directory");

    assert!(matches!(
        CoreState::open_portable_at(empty.path(), &runtime_root),
        Err(CoreError::InsecureDataRoot)
    ));
    assert!(!empty.path().join(".zeus-hso-root").exists());

    fs::remove_dir(empty.path()).expect("remove inherited empty directory");
    let core = CoreState::open_portable_at(empty.path(), &runtime_root)
        .expect("fresh-create path accepts the removed empty directory");
    assert!(core.data_root_is_private().expect("fresh root privacy"));
}

#[cfg(windows)]
#[test]
#[ignore = "requires elevated SeTakeOwnership/SeRestore privilege to preserve a foreign owner"]
fn portable_open_repairs_a_foreign_owner_root_under_elevation() {
    let runtime = RuntimeFixture::new("portable-foreign");
    let runtime_root = runtime
        .descriptor_path()
        .parent()
        .expect("fixture runtime root")
        .to_owned();
    let source = TestDirectory::new("portable-foreign-source");
    populate_portable_source_root(source.path(), runtime.descriptor_path());
    let copy = TestDirectory::new("portable-foreign-copy");
    copy_tree_with_inherited_acls(source.path(), copy.path());
    let expected_owner = current_user_private_owner("portable-foreign-reference");
    assign_system_owner(copy.path());
    assert_ne!(owner_sid(copy.path()), expected_owner);

    {
        let core = CoreState::open_portable_at(copy.path(), &runtime_root)
            .expect("elevated repair of a foreign-owner root");
        assert!(core.data_root_is_private().expect("repaired root privacy"));
    }
    assert_protected_current_user_security(copy.path(), &expected_owner);
}

#[cfg(windows)]
fn assign_system_owner(path: &Path) {
    let sddl: Vec<u16> = "O:SY".encode_utf16().chain([0]).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the SDDL buffer and output pointer are valid for the duration of the call.
    let built = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    assert_ne!(built, 0, "build foreign-owner descriptor");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: both the path and the descriptor are valid for this call.
    let applied =
        unsafe { SetFileSecurityW(wide.as_ptr(), OWNER_SECURITY_INFORMATION, descriptor) };
    // SAFETY: the descriptor was allocated by the SDDL conversion API via LocalAlloc.
    unsafe {
        LocalFree(descriptor.cast());
    }
    assert_ne!(
        applied, 0,
        "assigning a foreign owner requires elevated privilege"
    );
}

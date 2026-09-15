#[allow(
    dead_code,
    reason = "ManagerWorker consumes the account aggregate API in Task 6"
)]
mod account_repository;
mod profile_repository;
mod runtime_repository;
pub(crate) mod schema;

use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

use crate::credential_vault::CredentialVaultState;
use crate::data_root::{
    DataRoot, atomic_replace, create_private_truncated_file, harden_existing_private_file,
    open_private_file,
};
use crate::error::{CoreError, CoreResult};
use crate::runtime::{RuntimePreflightCache, RuntimePreflightDiagnostics};

use self::schema::{ACCOUNTS_V2_SQL, SCHEMA_V2_SQL, SPOTS_V3_SQL};

#[cfg(test)]
use self::schema::SCHEMA_V1_SQL;

// Only the Windows-only manager boundary projects stored accounts into public views.
#[cfg(windows)]
pub(crate) use account_repository::{StoredAccount, StoredAccountOutcome};
pub use runtime_repository::{RuntimePage, RuntimeRecord, RuntimeRepin};

pub const SCHEMA_VERSION: u32 = 3;
pub const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const DATABASE_FILE_NAME: &str = "state.sqlite3";
const LKG_FILE_NAME: &str = "state.lkg.sqlite3";
const LKG_TEMP_FILE_NAME: &str = ".state.lkg.sqlite3.tmp";
const INSTANCE_LOCK_FILE_NAME: &str = ".core-instance.lock";
const VAULT_KEY_FILE_NAME: &str = "vault.key";
const RUNTIME_DESCRIPTOR_FILE_NAME: &str = "runtime-descriptor.json";
const OPTIONAL_PRIVATE_STATE_FILES: &[&str] = &[
    LKG_FILE_NAME,
    LKG_TEMP_FILE_NAME,
    "state.sqlite3-journal",
    "state.sqlite3-shm",
    "state.sqlite3-wal",
    // Where spots lived before they moved into the database. Still allowed and still hardened: the
    // migration reads it rather than deleting it, so a root that has upgraded keeps the file, and a root
    // that never had one never gains it.
    crate::spots::SPOT_FILE_NAME,
];

const V1_TABLES: &[&str] = &["profiles", "runtimes", "schema_metadata"];
const V2_TABLES: &[&str] = &["accounts", "profiles", "runtimes", "schema_metadata"];
const V3_TABLES: &[&str] = &[
    "accounts",
    "profiles",
    "runtimes",
    "schema_metadata",
    "spots",
];
const SPOT_COLUMNS: &[&str] = &[
    "map_id",
    "name",
    "zone",
    "pixel_x",
    "pixel_y",
    "created_at_unix_ms",
];
const METADATA_COLUMNS: &[&str] = &[
    "singleton",
    "schema_version",
    "global_revision",
    "created_at_unix_ms",
];
const RUNTIME_COLUMNS: &[&str] = &[
    "runtime_id",
    "descriptor_sha256",
    "descriptor_path",
    "runtime_root",
    "target_os",
    "target_arch",
    "java_path",
    "java_vendor",
    "java_version",
    "jre_manifest_sha256",
    "microemulator_path",
    "microemulator_version",
    "microemulator_sha256",
    "game_path",
    "game_bundle",
    "game_sha256",
    "capability_state",
    "validation_reason",
    "validated_at_unix_ms",
    "created_at_unix_ms",
];
const PROFILE_COLUMNS: &[&str] = &[
    "profile_id",
    "revision",
    "display_name",
    "runtime_id",
    "launch_policy_json",
    "presentation_json",
    "archived_at_unix_ms",
    "created_at_unix_ms",
    "updated_at_unix_ms",
];
const ACCOUNT_COLUMNS: &[&str] = &[
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
type ForeignKeyDefinition = (String, String, String, String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaState {
    Fresh,
    V1,
    V2,
    /// V2 plus `spots`: monster spots moved out of a text file beside the database and into it.
    V3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationStep {
    MetadataRenamed,
    MetadataCopied,
    AccountsCreated,
    UserVersionUpdated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseInvariants {
    pub schema_version: u32,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub page_size: u64,
    pub max_page_count: u64,
    pub connection_count: u32,
}

pub struct CoreState {
    // Connection is declared before the lock so it closes before ownership is released.
    connection: Connection,
    _instance_lock: fs::File,
    data_root: DataRoot,
    credential_vault: CredentialVaultState,
    runtime_preflight_cache: RefCell<RuntimePreflightCache>,
    last_runtime_repin: Option<RuntimeRepin>,
}

impl CoreState {
    pub fn open_at(path: &Path) -> CoreResult<Self> {
        let data_root = DataRoot::prepare_at(path)?;
        Self::open_owned(data_root)
    }

    pub fn open_default() -> CoreResult<Self> {
        let data_root = DataRoot::prepare_default()?;
        Self::open_owned(data_root)
    }

    /// Opens a data root that may have been copied or moved, then re-points the pinned runtime.
    ///
    /// Sequencing: bounded portable repair, ordinary Core open/migrate, then runtime
    /// verification/registration/relocation against `exact_runtime_root`. ManagerWorker owns the
    /// only production caller; `zeus-ui` never reaches this surface.
    #[doc(hidden)]
    pub fn open_portable_at(data_root: &Path, exact_runtime_root: &Path) -> CoreResult<Self> {
        let prepared = DataRoot::prepare_portable_at(data_root)?;
        let mut core = Self::open_owned(prepared)?;
        core.relocate_pinned_runtime(&exact_runtime_root.join(RUNTIME_DESCRIPTOR_FILE_NAME))?;
        Ok(core)
    }

    fn open_owned(data_root: DataRoot) -> CoreResult<Self> {
        let lock_path = data_root.path().join(INSTANCE_LOCK_FILE_NAME);
        let instance_lock = open_private_file(&lock_path)?;
        match instance_lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(CoreError::AlreadyRunning),
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(CoreError::io("acquire Core instance lock", error));
            }
        }

        data_root.ensure_private_child_directory("profiles")?;
        for name in OPTIONAL_PRIVATE_STATE_FILES {
            let path = data_root.path().join(name);
            match fs::symlink_metadata(&path) {
                Ok(_) => harden_existing_private_file(&path)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(CoreError::io("inspect optional state file", error)),
            }
        }
        let database_path = data_root.path().join(DATABASE_FILE_NAME);
        let database_preexisting_bytes = if database_path.exists() {
            fs::metadata(&database_path)
                .map_err(|error| CoreError::io("inspect state database", error))?
                .len()
        } else {
            0
        };
        enforce_database_size(database_preexisting_bytes)?;
        drop(open_private_file(&database_path)?);

        // Inspect the effective schema, including a live WAL, through a read-only connection.
        // Unknown, mismatched, and future schemas fail before a write connection or LKG exists.
        let schema_state = inspect_schema_read_only(&database_path)?;
        // Every root that is about to be migrated gets a backup first, stated as "not already current"
        // rather than as a list of migrating states: enumerating them meant adding V3 silently dropped
        // the backup for V2 roots, which is the one case where a failed migration is unrecoverable.
        // A root with no database has nothing to lose.
        if schema_state != SchemaState::V3 && database_preexisting_bytes > 0 {
            create_lkg_backup(data_root.path(), &database_path)?;
        }

        let mut connection = open_connection(&database_path)?;
        configure_database_cap(&connection)?;
        match schema_state {
            SchemaState::Fresh => migrate_fresh_to_v3(&mut connection, data_root.path())?,
            SchemaState::V1 => {
                migrate_v1_to_v2(&mut connection)?;
                migrate_v2_to_v3(&mut connection, data_root.path())?;
            }
            SchemaState::V2 => migrate_v2_to_v3(&mut connection, data_root.path())?,
            SchemaState::V3 => {
                if inspect_schema(&connection)? != SchemaState::V3 {
                    return Err(CoreError::UnmanagedDatabase);
                }
            }
        }

        let account_rows_exist =
            connection.query_row("SELECT EXISTS(SELECT 1 FROM accounts LIMIT 1)", [], |row| {
                row.get::<_, bool>(0)
            })?;
        let credential_vault = CredentialVaultState::open(&data_root, account_rows_exist)?;

        Ok(Self {
            connection,
            _instance_lock: instance_lock,
            data_root,
            credential_vault,
            runtime_preflight_cache: RefCell::new(RuntimePreflightCache::new()),
            last_runtime_repin: None,
        })
    }

    pub fn database_invariants(&self) -> CoreResult<DatabaseInvariants> {
        let schema_version = read_schema_version(&self.connection)?;
        let journal_mode = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        let foreign_keys = self
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?
            == 1;
        let page_size = self
            .connection
            .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?
            .try_into()
            .map_err(|_| CoreError::DatabaseTooLarge {
                limit_bytes: MAX_DATABASE_BYTES,
            })?;
        let max_page_count = self
            .connection
            .query_row("PRAGMA max_page_count", [], |row| row.get::<_, i64>(0))?
            .try_into()
            .map_err(|_| CoreError::DatabaseTooLarge {
                limit_bytes: MAX_DATABASE_BYTES,
            })?;
        Ok(DatabaseInvariants {
            schema_version,
            journal_mode,
            foreign_keys,
            page_size,
            max_page_count,
            connection_count: 1,
        })
    }

    pub fn schema_tables(&self) -> CoreResult<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let values = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values)
    }

    pub fn schema_columns(&self, table: &str) -> CoreResult<Vec<String>> {
        if !matches!(
            table,
            "schema_metadata" | "runtimes" | "profiles" | "accounts"
        ) {
            return Err(CoreError::UnmanagedDatabase);
        }
        Ok(table_columns(&self.connection, table)?
            .into_iter()
            .map(|(name, _hidden)| name)
            .collect())
    }

    pub fn data_root_is_private(&self) -> CoreResult<bool> {
        self.data_root.is_private()
    }

    pub fn state_files_are_private(&self) -> CoreResult<bool> {
        if !self
            .data_root
            .state_files_are_private(&[INSTANCE_LOCK_FILE_NAME, DATABASE_FILE_NAME])?
        {
            return Ok(false);
        }
        for name in OPTIONAL_PRIVATE_STATE_FILES {
            let path = self.data_root.path().join(name);
            match fs::symlink_metadata(path) {
                Ok(_) if !self.data_root.state_files_are_private(&[name])? => return Ok(false),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CoreError::io("inspect optional private state file", error));
                }
            }
        }
        let vault_path = self.data_root.path().join(VAULT_KEY_FILE_NAME);
        match fs::symlink_metadata(vault_path) {
            Ok(_)
                if !self
                    .data_root
                    .state_files_are_private(&[VAULT_KEY_FILE_NAME])? =>
            {
                return Ok(false);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CoreError::io("inspect credential key", error)),
        }
        Ok(true)
    }

    pub(crate) fn data_root(&self) -> &DataRoot {
        &self.data_root
    }

    pub fn runtime_preflight_diagnostics(&self) -> RuntimePreflightDiagnostics {
        self.runtime_preflight_cache.borrow().diagnostics()
    }

    /// The re-pin performed while opening this data root, if the pinned content had been replaced.
    ///
    /// `None` is the ordinary case and means the registry matched the tree byte for byte.
    pub fn last_runtime_repin(&self) -> Option<&RuntimeRepin> {
        self.last_runtime_repin.as_ref()
    }

    pub(crate) fn record_runtime_repin(&mut self, repin: RuntimeRepin) {
        self.last_runtime_repin = Some(repin);
    }

    pub(crate) fn runtime_preflight_cache(&self) -> &RefCell<RuntimePreflightCache> {
        &self.runtime_preflight_cache
    }

    pub(crate) fn runtime_preflight_cache_mut(&mut self) -> &mut RuntimePreflightCache {
        self.runtime_preflight_cache.get_mut()
    }

    pub(crate) fn now_unix_ms() -> CoreResult<i64> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CoreError::io("read system time", std::io::Error::other(error)))?;
        duration.as_millis().try_into().map_err(|_| {
            CoreError::io(
                "convert system time",
                std::io::Error::other("timestamp exceeds SQLite integer range"),
            )
        })
    }
}

fn open_connection(path: &Path) -> CoreResult<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;\
         PRAGMA journal_mode = DELETE;\
         PRAGMA synchronous = FULL;\
         PRAGMA temp_store = MEMORY;\
         PRAGMA trusted_schema = OFF;",
    )?;
    Ok(connection)
}

fn inspect_schema_read_only(path: &Path) -> CoreResult<SchemaState> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    inspect_schema(&connection)
}

fn read_schema_version(connection: &Connection) -> CoreResult<u32> {
    let value = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    value.try_into().map_err(|_| CoreError::UnsupportedSchema {
        found: u32::MAX,
        supported: SCHEMA_VERSION,
    })
}

/// Builds the current schema in an empty database.
///
/// `data_root` is here for the root whose database was deleted but whose spot file was not: the operator
/// still has those spots, and a fresh database is no reason to lose them.
fn migrate_fresh_to_v3(connection: &mut Connection, data_root: &Path) -> CoreResult<()> {
    let now = CoreState::now_unix_ms()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(SCHEMA_V2_SQL)?;
    transaction.execute_batch(SPOTS_V3_SQL)?;
    crate::spots::adopt_legacy_file(&transaction, data_root, now)?;
    transaction.execute(
        "INSERT INTO schema_metadata \
         (singleton, schema_version, global_revision, created_at_unix_ms) \
         VALUES (1, 3, 0, ?1)",
        params![now],
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    if inspect_schema(&transaction)? != SchemaState::V3 {
        return Err(CoreError::UnmanagedDatabase);
    }
    transaction.commit()?;
    Ok(())
}

/// Adds `spots` and adopts whatever the text file beside the database held.
///
/// The file was the first shape and it worked, but a spot the operator recorded deserves the same
/// durability as the account it farms with: one transaction, one file to back up, and no window where a
/// crash between write and rename loses the newest entry. The file is read once and left in place —
/// deleting the operator's data as part of an upgrade is not this migration's business.
fn migrate_v2_to_v3(connection: &mut Connection, data_root: &Path) -> CoreResult<()> {
    if inspect_schema(connection)? != SchemaState::V2 {
        return Err(CoreError::UnmanagedDatabase);
    }
    let now = CoreState::now_unix_ms()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(SPOTS_V3_SQL)?;
    // Adopted in the same transaction that creates the table, so the upgrade either takes the operator's
    // spots with it or does not happen. Once, by construction: a later run is already V3.
    crate::spots::adopt_legacy_file(&transaction, data_root, now)?;

    // schema_metadata pins its version with a CHECK, so reaching 3 means rebuilding the table: an UPDATE
    // violates the constraint the V2 table was created with. created_at is carried over rather than
    // restamped — the root was created when it was created, and the migration is not a new install.
    transaction.execute_batch("ALTER TABLE schema_metadata RENAME TO _zeus_schema_metadata_v2;")?;
    transaction.execute_batch(
        "CREATE TABLE schema_metadata (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             schema_version INTEGER NOT NULL CHECK (schema_version = 3),\
             global_revision INTEGER NOT NULL CHECK (global_revision >= 0),\
             created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0)\
         ) STRICT;",
    )?;
    if transaction.execute(
        "INSERT INTO schema_metadata \
         (singleton, schema_version, global_revision, created_at_unix_ms) \
         SELECT singleton, 3, global_revision, created_at_unix_ms \
         FROM _zeus_schema_metadata_v2",
        [],
    )? != 1
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    transaction.execute_batch("DROP TABLE _zeus_schema_metadata_v2;")?;

    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    if inspect_schema(&transaction)? != SchemaState::V3 {
        return Err(CoreError::UnmanagedDatabase);
    }
    transaction.commit()?;
    Ok(())
}

fn migrate_v1_to_v2(connection: &mut Connection) -> CoreResult<()> {
    migrate_v1_to_v2_with_hook(connection, |_| Ok(()))
}

fn migrate_v1_to_v2_with_hook<F>(connection: &mut Connection, mut after_step: F) -> CoreResult<()>
where
    F: FnMut(MigrationStep) -> CoreResult<()>,
{
    if inspect_schema(connection)? != SchemaState::V1 {
        return Err(CoreError::UnmanagedDatabase);
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch("ALTER TABLE schema_metadata RENAME TO _zeus_schema_metadata_v1;")?;
    after_step(MigrationStep::MetadataRenamed)?;

    transaction.execute_batch(
        "CREATE TABLE schema_metadata (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             schema_version INTEGER NOT NULL CHECK (schema_version = 2),\
             global_revision INTEGER NOT NULL CHECK (global_revision >= 0),\
             created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0)\
         ) STRICT;",
    )?;

    if transaction.execute(
        "INSERT INTO schema_metadata \
         (singleton, schema_version, global_revision, created_at_unix_ms) \
         SELECT singleton, 2, global_revision, created_at_unix_ms \
         FROM _zeus_schema_metadata_v1",
        [],
    )? != 1
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    after_step(MigrationStep::MetadataCopied)?;

    transaction.execute_batch("DROP TABLE _zeus_schema_metadata_v1;")?;
    transaction.execute_batch(ACCOUNTS_V2_SQL)?;
    after_step(MigrationStep::AccountsCreated)?;

    // Literal 2, not SCHEMA_VERSION: this step lands on V2 and the next one carries it to V3. Stamping
    // the newest version here would leave user_version ahead of both the metadata row and the tables,
    // which the inspector reads as a database it does not manage.
    transaction.pragma_update(None, "user_version", 2)?;
    after_step(MigrationStep::UserVersionUpdated)?;
    if inspect_schema(&transaction)? != SchemaState::V2 {
        return Err(CoreError::UnmanagedDatabase);
    }
    transaction.commit()?;
    Ok(())
}

fn inspect_schema(connection: &Connection) -> CoreResult<SchemaState> {
    let raw_user_version =
        connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    let user_version: u32 =
        raw_user_version
            .try_into()
            .map_err(|_| CoreError::UnsupportedSchema {
                found: u32::MAX,
                supported: SCHEMA_VERSION,
            })?;
    let tables = table_names(connection)?;
    let metadata = if tables.iter().any(|table| table == "schema_metadata") {
        read_metadata_version(connection)?
    } else {
        None
    };

    if user_version > SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema {
            found: user_version,
            supported: SCHEMA_VERSION,
        });
    }
    if let Some(found) = metadata.filter(|version| *version > SCHEMA_VERSION) {
        return Err(CoreError::UnsupportedSchema {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    let state = match (user_version, metadata) {
        (0, None) if tables.is_empty() => SchemaState::Fresh,
        (1, Some(1)) => SchemaState::V1,
        (2, Some(2)) => SchemaState::V2,
        (3, Some(3)) => SchemaState::V3,
        _ => return Err(CoreError::UnmanagedDatabase),
    };
    validate_schema_shape(connection, state, &tables)?;
    Ok(state)
}

fn read_metadata_version(connection: &Connection) -> CoreResult<Option<u32>> {
    let mut statement = connection.prepare(
        "SELECT singleton, schema_version, global_revision, created_at_unix_ms \
         FROM schema_metadata",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let [(singleton, version, global_revision, created_at)] = rows.as_slice() else {
        return Err(CoreError::UnmanagedDatabase);
    };
    if *singleton != 1 || *global_revision < 0 || *created_at <= 0 {
        return Err(CoreError::UnmanagedDatabase);
    }
    let version = (*version)
        .try_into()
        .map_err(|_| CoreError::UnmanagedDatabase)?;
    Ok(Some(version))
}

fn validate_schema_shape(
    connection: &Connection,
    state: SchemaState,
    tables: &[String],
) -> CoreResult<()> {
    let expected_tables = match state {
        SchemaState::Fresh => return Ok(()),
        SchemaState::V1 => V1_TABLES,
        SchemaState::V2 => V2_TABLES,
        SchemaState::V3 => V3_TABLES,
    };
    if tables != expected_tables {
        return Err(CoreError::UnmanagedDatabase);
    }
    for (table, expected_columns) in [
        ("schema_metadata", METADATA_COLUMNS),
        ("runtimes", RUNTIME_COLUMNS),
        ("profiles", PROFILE_COLUMNS),
    ] {
        validate_table(connection, table, expected_columns)?;
    }
    if matches!(state, SchemaState::V2 | SchemaState::V3) {
        validate_table(connection, "accounts", ACCOUNT_COLUMNS)?;
    }
    if state == SchemaState::V3 {
        validate_table(connection, "spots", SPOT_COLUMNS)?;
    }
    validate_foreign_keys(connection, state)?;
    if connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
        row.get::<_, i64>(0)
    })? != 0
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    Ok(())
}

fn validate_table(
    connection: &Connection,
    table: &str,
    expected_columns: &[&str],
) -> CoreResult<()> {
    let columns = table_columns(connection, table)?;
    if columns.len() != expected_columns.len()
        || columns
            .iter()
            .zip(expected_columns)
            .any(|((name, hidden), expected)| name != expected || *hidden != 0)
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    let strict = connection.query_row(
        "SELECT strict FROM pragma_table_list \
         WHERE schema = 'main' AND type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, i64>(0),
    )?;
    if strict != 1 {
        return Err(CoreError::UnmanagedDatabase);
    }
    Ok(())
}

fn validate_foreign_keys(connection: &Connection, state: SchemaState) -> CoreResult<()> {
    if !foreign_keys(connection, "schema_metadata")?.is_empty()
        || !foreign_keys(connection, "runtimes")?.is_empty()
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    let profile_keys = foreign_keys(connection, "profiles")?;
    if profile_keys
        != [(
            "runtimes".to_owned(),
            "runtime_id".to_owned(),
            "runtime_id".to_owned(),
            "RESTRICT".to_owned(),
            "RESTRICT".to_owned(),
        )]
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    if matches!(state, SchemaState::V2 | SchemaState::V3)
        && foreign_keys(connection, "accounts")?
            != [(
                "profiles".to_owned(),
                "profile_id".to_owned(),
                "profile_id".to_owned(),
                "RESTRICT".to_owned(),
                "RESTRICT".to_owned(),
            )]
    {
        return Err(CoreError::UnmanagedDatabase);
    }
    // Spots reference nothing: they are world coordinates, shared by every account, so a key into
    // accounts or profiles would be wrong rather than merely unused.
    if state == SchemaState::V3 && !foreign_keys(connection, "spots")?.is_empty() {
        return Err(CoreError::UnmanagedDatabase);
    }
    Ok(())
}

fn foreign_keys(connection: &Connection, table: &str) -> CoreResult<Vec<ForeignKeyDefinition>> {
    let mut statement = connection.prepare(
        "SELECT \"table\", \"from\", \"to\", on_update, on_delete \
         FROM pragma_foreign_key_list(?1) ORDER BY id, seq",
    )?;
    Ok(statement
        .query_map(params![table], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn table_names(connection: &Connection) -> CoreResult<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn table_columns(connection: &Connection, table: &str) -> CoreResult<Vec<(String, i64)>> {
    let mut statement =
        connection.prepare("SELECT name, hidden FROM pragma_table_xinfo(?1) ORDER BY cid")?;
    Ok(statement
        .query_map(params![table], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn configure_database_cap(connection: &Connection) -> CoreResult<()> {
    let page_size: u64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?
        .try_into()
        .map_err(|_| CoreError::DatabaseTooLarge {
            limit_bytes: MAX_DATABASE_BYTES,
        })?;
    if page_size == 0 || page_size > MAX_DATABASE_BYTES {
        return Err(CoreError::DatabaseTooLarge {
            limit_bytes: MAX_DATABASE_BYTES,
        });
    }
    let max_pages: i64 =
        (MAX_DATABASE_BYTES / page_size)
            .try_into()
            .map_err(|_| CoreError::DatabaseTooLarge {
                limit_bytes: MAX_DATABASE_BYTES,
            })?;
    connection.pragma_update(None, "max_page_count", max_pages)?;
    Ok(())
}

fn enforce_database_size(bytes: u64) -> CoreResult<()> {
    if bytes > MAX_DATABASE_BYTES {
        return Err(CoreError::DatabaseTooLarge {
            limit_bytes: MAX_DATABASE_BYTES,
        });
    }
    Ok(())
}

fn create_lkg_backup(data_root: &Path, database: &Path) -> CoreResult<()> {
    let source_size = fs::metadata(database)
        .map_err(|error| CoreError::io("inspect database before migration backup", error))?
        .len();
    enforce_database_size(source_size)?;
    let temp_path = data_root.join(LKG_TEMP_FILE_NAME);
    let destination = data_root.join(LKG_FILE_NAME);
    let source = fs::File::open(database)
        .map_err(|error| CoreError::io("open database for migration backup", error))?;
    let mut temp = create_private_truncated_file(&temp_path)?;
    let copied = std::io::copy(&mut source.take(MAX_DATABASE_BYTES + 1), &mut temp)
        .map_err(|error| CoreError::io("copy migration backup", error))?;
    if copied != source_size || copied > MAX_DATABASE_BYTES {
        return Err(CoreError::DatabaseTooLarge {
            limit_bytes: MAX_DATABASE_BYTES,
        });
    }
    temp.flush()
        .and_then(|()| temp.sync_all())
        .map_err(|error| CoreError::io("persist migration backup", error))?;
    drop(temp);
    atomic_replace(&temp_path, &destination)
}

#[cfg(test)]
mod schema_migration_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusqlite::Connection;

    use super::{MigrationStep, SCHEMA_V1_SQL, create_lkg_backup, migrate_v1_to_v2_with_hook};
    use crate::CoreError;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "zeus-schema-rollback-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migration_rolls_back_every_injected_step_and_preserves_openable_v1_lkg() {
        for failed_step in [
            MigrationStep::MetadataRenamed,
            MigrationStep::MetadataCopied,
            MigrationStep::AccountsCreated,
            MigrationStep::UserVersionUpdated,
        ] {
            let directory = TestDirectory::new();
            let database = directory.path().join("state.sqlite3");
            let connection = Connection::open(&database).unwrap();
            connection.execute_batch(SCHEMA_V1_SQL).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_metadata \
                     (singleton, schema_version, global_revision, created_at_unix_ms) \
                     VALUES (1, 1, 23, 1700000000456)",
                    [],
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
            drop(connection);
            let before = fs::read(&database).unwrap();
            create_lkg_backup(directory.path(), &database).unwrap();

            let mut connection = Connection::open(&database).unwrap();
            let result = migrate_v1_to_v2_with_hook(&mut connection, |completed| {
                if completed == failed_step {
                    Err(CoreError::UnmanagedDatabase)
                } else {
                    Ok(())
                }
            });
            assert!(matches!(result, Err(CoreError::UnmanagedDatabase)));
            drop(connection);

            let reopened = Connection::open(&database).unwrap();
            assert_eq!(
                reopened
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                reopened
                    .query_row(
                        "SELECT schema_version FROM schema_metadata WHERE singleton = 1",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
            let tables = reopened
                .prepare(
                    "SELECT name FROM sqlite_schema \
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(tables, vec!["profiles", "runtimes", "schema_metadata"]);
            drop(reopened);

            let lkg = directory.path().join("state.lkg.sqlite3");
            assert_eq!(fs::read(&lkg).unwrap(), before);
            let backup =
                Connection::open_with_flags(lkg, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .unwrap();
            assert_eq!(
                backup
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
}

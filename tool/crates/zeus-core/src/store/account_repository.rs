//! Combined account/profile aggregate operations.
//!
//! Every mutation is one Immediate transaction with exactly one global-revision increment, so an
//! account and its profile can never be observed half-created. The independently committing profile
//! methods are deliberately not reused: they would commit their own transaction and break that
//! atomicity.

use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::account::AccountConfigV1;
use crate::control::{self, AttackSpot, ControlSettings};
use crate::credential_vault::{EncryptedPasswordV1, SecretBytes};
use crate::error::{CoreError, CoreResult};
use crate::player::{self, PlayerSnapshot};
use crate::profile::{DEFAULT_LAUNCH_POLICY_JSON, DEFAULT_PRESENTATION_JSON, ProfileRecord};
use crate::rms;
use crate::spots::{self, SpotBook};

use super::CoreState;
use super::profile_repository::{increment_global_revision, insert_profile, query_profile};

/// Spec section 8 account limit.
pub(crate) const MAX_ACCOUNTS: u32 = 100;
const USERNAME_MAX_BYTES: usize = 64;
const PASSWORD_MAX_BYTES: usize = 128;
const CREDENTIAL_VERSION: i64 = 1;
const CONFIG_SCHEMA_VERSION: i64 = 1;

const ACCOUNT_SELECT_COLUMNS: &str = "account_id, revision, username, profile_id, config_json, \
    config_revision, last_run_at_unix_ms, last_outcome";

/// Terminal outcome of the most recent login attempt for one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredAccountOutcome {
    /// The complete fixed submit script was injected. This is not an authentication claim.
    Started,
    LoginFailed,
}

impl StoredAccountOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "Started",
            Self::LoginFailed => "LoginFailed",
        }
    }

    fn parse(value: &str) -> CoreResult<Self> {
        match value {
            "Started" => Ok(Self::Started),
            "LoginFailed" => Ok(Self::LoginFailed),
            _ => Err(CoreError::UnmanagedDatabase),
        }
    }
}

/// Redacted account projection. It deliberately carries no ciphertext, nonce, tag, or plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredAccount {
    pub account_id: Uuid,
    pub revision: i64,
    pub username: String,
    pub profile_id: String,
    pub config: AccountConfigV1,
    pub config_revision: i64,
    pub last_run_at_unix_ms: Option<i64>,
    pub last_outcome: Option<StoredAccountOutcome>,
}

impl CoreState {
    /// Imports one account and its private profile in a single transaction.
    pub(crate) fn create_account_with_profile(
        &mut self,
        username: &str,
        mut password: SecretBytes,
    ) -> CoreResult<StoredAccount> {
        validate_username(username)?;
        validate_password(&password)?;

        let runtime_id = self.pinned_runtime_id()?;
        let account_id = Uuid::new_v4();
        let encrypted = self
            .credential_vault
            .cipher()?
            .encrypt(account_id, &mut password)?;

        let now = Self::now_unix_ms()?;
        let (profile_id, _directory) = self.allocate_profile_directory()?;
        let profile = ProfileRecord {
            profile_id: profile_id.clone(),
            revision: 1,
            display_name: username.to_owned(),
            runtime_id,
            launch_policy_json: DEFAULT_LAUNCH_POLICY_JSON.to_owned(),
            presentation_json: DEFAULT_PRESENTATION_JSON.to_owned(),
            archived_at_unix_ms: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
        };
        let config = AccountConfigV1::defaults();

        // Any pre-commit failure must remove the uncommitted UUID directory, so a rolled-back import
        // leaves no orphan on disk.
        let outcome = (|| -> CoreResult<StoredAccount> {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if count_accounts(&transaction)? >= i64::from(MAX_ACCOUNTS) {
                return Err(CoreError::AccountLimitReached {
                    maximum: MAX_ACCOUNTS,
                });
            }
            if username_key_exists(&transaction, &username_key(username), None)? {
                return Err(CoreError::DuplicateUsername);
            }
            insert_profile(&transaction, &profile)?;
            transaction.execute(
                "INSERT INTO accounts (\
                    account_id, revision, username, username_key, profile_id, credential_version, \
                    password_cipher, password_nonce, password_tag, config_schema_version, \
                    config_revision, config_json, last_run_at_unix_ms, last_outcome, \
                    created_at_unix_ms, updated_at_unix_ms\
                 ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, NULL, NULL, ?11, ?11)",
                params![
                    account_id.hyphenated().to_string(),
                    username,
                    username_key(username),
                    profile.profile_id,
                    CREDENTIAL_VERSION,
                    encrypted.cipher,
                    encrypted.nonce.as_slice(),
                    encrypted.tag.as_slice(),
                    CONFIG_SCHEMA_VERSION,
                    config.to_canonical_json(),
                    now,
                ],
            )?;
            increment_global_revision(&transaction)?;
            transaction.commit()?;
            Ok(StoredAccount {
                account_id,
                revision: 1,
                username: username.to_owned(),
                profile_id: profile.profile_id.clone(),
                config,
                config_revision: 1,
                last_run_at_unix_ms: None,
                last_outcome: None,
            })
        })();

        if outcome.is_err() {
            let _ = self.data_root.remove_empty_profile_directory(&profile_id);
        }
        outcome
    }

    /// Renames an account and its profile together, optionally rotating the password.
    ///
    /// A rename never decrypts or re-encrypts an unchanged password, so it stays possible while the
    /// vault key is unavailable.
    pub(crate) fn update_account_and_profile(
        &mut self,
        account_id: Uuid,
        expected_revision: i64,
        username: &str,
        replacement: Option<SecretBytes>,
    ) -> CoreResult<StoredAccount> {
        validate_username(username)?;
        validate_expected_revision(expected_revision)?;
        let rotated = match replacement {
            Some(mut password) => {
                validate_password(&password)?;
                Some(
                    self.credential_vault
                        .cipher()?
                        .encrypt(account_id, &mut password)?,
                )
            }
            None => None,
        };

        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = require_account(&transaction, account_id, expected_revision)?;
        if username_key_exists(&transaction, &username_key(username), Some(account_id))? {
            return Err(CoreError::DuplicateUsername);
        }
        let updated_at = monotonic_account_time(&transaction, account_id, wall_clock)?;
        let identifier = account_id.hyphenated().to_string();

        if let Some(encrypted) = rotated {
            transaction.execute(
                "UPDATE accounts SET username = ?1, username_key = ?2, password_cipher = ?3, \
                 password_nonce = ?4, password_tag = ?5, revision = revision + 1, \
                 updated_at_unix_ms = ?6 WHERE account_id = ?7 AND revision = ?8",
                params![
                    username,
                    username_key(username),
                    encrypted.cipher,
                    encrypted.nonce.as_slice(),
                    encrypted.tag.as_slice(),
                    updated_at,
                    identifier,
                    expected_revision,
                ],
            )?;
        } else {
            transaction.execute(
                "UPDATE accounts SET username = ?1, username_key = ?2, revision = revision + 1, \
                 updated_at_unix_ms = ?3 WHERE account_id = ?4 AND revision = ?5",
                params![
                    username,
                    username_key(username),
                    updated_at,
                    identifier,
                    expected_revision,
                ],
            )?;
        }

        // The profile display name follows the account username in the same transaction.
        let profile_updated_at = query_profile(&transaction, &current.profile_id)?
            .ok_or_else(|| CoreError::ProfileNotFound {
                profile_id: current.profile_id.clone(),
            })?
            .updated_at_unix_ms
            .max(wall_clock);
        transaction.execute(
            "UPDATE profiles SET display_name = ?1, revision = revision + 1, \
             updated_at_unix_ms = ?2 WHERE profile_id = ?3",
            params![username, profile_updated_at, current.profile_id],
        )?;
        increment_global_revision(&transaction)?;
        let result =
            query_account(&transaction, account_id)?.ok_or(CoreError::AccountNotFound {
                account_id: identifier,
            })?;
        transaction.commit()?;
        Ok(result)
    }

    /// Deletes the account row and archives its profile in one transaction.
    ///
    /// This is not a secure erase: M3.1 does not enable `secure_delete`, so SQLite may retain the old
    /// ciphertext in freed pages.
    pub(crate) fn delete_account_and_archive_profile(
        &mut self,
        account_id: Uuid,
        expected_revision: i64,
    ) -> CoreResult<()> {
        validate_expected_revision(expected_revision)?;
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = require_account(&transaction, account_id, expected_revision)?;
        // The account row references the profile, so it must go first.
        transaction.execute(
            "DELETE FROM accounts WHERE account_id = ?1 AND revision = ?2",
            params![account_id.hyphenated().to_string(), expected_revision],
        )?;
        let profile = query_profile(&transaction, &current.profile_id)?.ok_or_else(|| {
            CoreError::ProfileNotFound {
                profile_id: current.profile_id.clone(),
            }
        })?;
        if profile.archived_at_unix_ms.is_none() {
            let archived_at = wall_clock.max(profile.updated_at_unix_ms);
            transaction.execute(
                "UPDATE profiles SET archived_at_unix_ms = ?1, revision = revision + 1, \
                 updated_at_unix_ms = ?1 WHERE profile_id = ?2 AND archived_at_unix_ms IS NULL",
                params![archived_at, current.profile_id],
            )?;
        }
        increment_global_revision(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    /// Resolves one account's linked profile, so lifecycle operations can find its session.
    pub(crate) fn account_profile_id(&self, account_id: Uuid) -> CoreResult<String> {
        self.connection
            .query_row(
                "SELECT profile_id FROM accounts WHERE account_id = ?1",
                params![account_id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| CoreError::AccountNotFound {
                account_id: account_id.hyphenated().to_string(),
            })
    }

    /// Current revision of one account, for callers that must confirm the row they were handed.
    pub(crate) fn account_revision(&self, account_id: Uuid) -> CoreResult<i64> {
        self.connection
            .query_row(
                "SELECT revision FROM accounts WHERE account_id = ?1",
                params![account_id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| CoreError::AccountNotFound {
                account_id: account_id.hyphenated().to_string(),
            })
    }

    /// Sets which world one account logs into, as an index into the client's server table.
    ///
    /// The index is bounded by [`rms::SERVER_COUNT`] before it is stored, so a value the client would
    /// read out of bounds can never reach the record store. Only the config counter moves: the account
    /// revision is the operator's optimistic-concurrency token and this is not an operator edit of the
    /// identity, so a concurrent rename is not invalidated by choosing a server.
    pub(crate) fn set_account_server(
        &mut self,
        account_id: Uuid,
        server_index: u8,
    ) -> CoreResult<StoredAccount> {
        if server_index >= rms::SERVER_COUNT {
            return Err(CoreError::RmsSeed {
                code: "rms_server_index_out_of_range",
            });
        }
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identifier = account_id.hyphenated().to_string();
        let current =
            query_account(&transaction, account_id)?.ok_or_else(|| CoreError::AccountNotFound {
                account_id: identifier.clone(),
            })?;
        let mut config = current.config;
        config.server_index = server_index;
        let updated_at = monotonic_account_time(&transaction, account_id, wall_clock)?;
        transaction.execute(
            "UPDATE accounts SET config_json = ?1, config_revision = config_revision + 1, \
             updated_at_unix_ms = ?2 WHERE account_id = ?3",
            params![config.to_canonical_json(), updated_at, identifier],
        )?;
        increment_global_revision(&transaction)?;
        let result =
            query_account(&transaction, account_id)?.ok_or(CoreError::AccountNotFound {
                account_id: identifier,
            })?;
        transaction.commit()?;
        Ok(result)
    }

    /// Lists every account, sorted by `username_key`, bounded by the account limit.
    pub(crate) fn list_accounts(&self) -> CoreResult<Vec<StoredAccount>> {
        let sql =
            format!("SELECT {ACCOUNT_SELECT_COLUMNS} FROM accounts ORDER BY username_key LIMIT ?1");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement
            .query_map(params![i64::from(MAX_ACCOUNTS)], row_to_account)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().collect()
    }

    /// Reads one account, so a caller that must report its current state does not re-list every row.
    pub(crate) fn account(&self, account_id: Uuid) -> CoreResult<StoredAccount> {
        self.connection
            .query_row(
                &format!("SELECT {ACCOUNT_SELECT_COLUMNS} FROM accounts WHERE account_id = ?1"),
                params![account_id.hyphenated().to_string()],
                row_to_account,
            )
            .optional()?
            .ok_or_else(|| CoreError::AccountNotFound {
                account_id: account_id.hyphenated().to_string(),
            })?
    }

    /// Writes the record stores the client's own auto-login path reads, for one account.
    ///
    /// This is the single production consumer of the credential vault's decrypt direction. The
    /// plaintext is decrypted, encoded, and dropped inside this call: it is never returned, logged, or
    /// handed to a caller, so no secret crosses the manager boundary.
    ///
    /// Must run before the JVM starts. `bs.c()` reads `user_pass` while it builds the login screen, so
    /// a store written after process start is simply never seen.
    pub(crate) fn seed_account_login(&self, account_id: Uuid) -> CoreResult<()> {
        let profile_id = self.account_profile_id(account_id)?;
        let account = self.account(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        let password = self.decrypt_account_password(account_id)?;
        // The secret is borrowed for the write and dropped at the end of this scope, which zeroes it.
        rms::seed_credentials(
            &microemu_home,
            &account.username,
            // Validated printable ASCII on the way in, so a stored password is always valid UTF-8.
            std::str::from_utf8(password.expose_for_validation())
                .map_err(|_| CoreError::InvalidPassword)?,
            account.config.server_index,
        )
    }

    /// Removes one account's seeded stores, so a stopped account leaves no credential on disk.
    pub(crate) fn clear_account_login(&self, account_id: Uuid) -> CoreResult<()> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        rms::clear_credentials(&microemu_home)
    }

    /// Reads the character snapshot the mod published for one account.
    ///
    /// `Ok(None)` is the ordinary state before a character is entered: the mod has simply not written
    /// yet. The caller supplies an account and receives values only, so no path crosses this boundary.
    pub(crate) fn account_player_snapshot(
        &self,
        account_id: Uuid,
    ) -> CoreResult<Option<PlayerSnapshot>> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        player::read_snapshot(&microemu_home)
    }

    /// Removes one account's snapshot, so a stopped account stops reporting a character.
    ///
    /// Without this the last reading would stay readable indefinitely and the panel would keep showing
    /// a live-looking character for a session that has already exited.
    pub(crate) fn clear_account_player_snapshot(&self, account_id: Uuid) -> CoreResult<()> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        player::clear_snapshot(&microemu_home)
    }

    /// Writes the attack and item settings one account's client reads while it runs.
    ///
    /// The file is also the persistence: it lives in the profile directory, so a restart of the tool
    /// finds the settings already in place rather than needing a second copy in the database that
    /// could disagree with what the mod is reading.
    pub(crate) fn set_account_control_settings(
        &self,
        account_id: Uuid,
        settings: &ControlSettings,
    ) -> CoreResult<()> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        control::write_settings(&microemu_home, settings)
    }

    /// Reads every saved monster spot.
    ///
    /// Shared by every account rather than stored per account: a spot belongs to the world, and two
    /// accounts farming the same map want the same coordinates. In the database rather than in a file
    /// beside it, so a spot has the same durability as the account that farms with it.
    pub(crate) fn saved_spots(&self) -> CoreResult<SpotBook> {
        spots::read_spots(&self.connection)
    }

    /// Saves one named spot, replacing any spot of the same name on the same map.
    ///
    /// Replacing by name is the intent: the operator stands somewhere better and saves under the name
    /// they already use, and that name now means the new place.
    pub(crate) fn save_spot(&self, spot: AttackSpot, name: &str) -> CoreResult<SpotBook> {
        let now = CoreState::now_unix_ms()?;
        spots::save_spot(&self.connection, spot, name, now)?;
        spots::read_spots(&self.connection)
    }

    /// Forgets one named spot.
    pub(crate) fn clear_spot(&self, map_id: u16, name: &str) -> CoreResult<SpotBook> {
        spots::clear_spot(&self.connection, map_id, name)?;
        spots::read_spots(&self.connection)
    }

    /// Reads back one account's settings, or `None` when none have been configured.
    pub(crate) fn account_control_settings(
        &self,
        account_id: Uuid,
    ) -> CoreResult<Option<ControlSettings>> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        control::read_settings(&microemu_home)
    }

    /// Removes one account's settings.
    ///
    /// Called on a confirmed stop so a spot captured for one session cannot silently steer the next
    /// one; the operator re-arms deliberately.
    pub(crate) fn clear_account_control_settings(&self, account_id: Uuid) -> CoreResult<()> {
        let profile_id = self.account_profile_id(account_id)?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(&profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        control::clear_settings(&microemu_home)
    }

    /// Decrypts one account's stored password. Private: the plaintext must not leave this module.
    fn decrypt_account_password(&self, account_id: Uuid) -> CoreResult<SecretBytes> {
        let (version, cipher, nonce, tag): (i64, Vec<u8>, Vec<u8>, Vec<u8>) =
            self.connection.query_row(
                "SELECT credential_version, password_cipher, password_nonce, password_tag \
                 FROM accounts WHERE account_id = ?1",
                params![account_id.hyphenated().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        if version != CREDENTIAL_VERSION {
            return Err(CoreError::UnmanagedDatabase);
        }
        let encrypted = EncryptedPasswordV1 {
            version: 1,
            cipher,
            nonce: nonce.try_into().map_err(|_| CoreError::UnmanagedDatabase)?,
            tag: tag.try_into().map_err(|_| CoreError::UnmanagedDatabase)?,
        };
        self.credential_vault
            .cipher()?
            .decrypt(account_id, &encrypted)
    }

    /// Records that the fixed submit script was injected, clearing any stale `LoginFailed`.
    pub(crate) fn mark_account_started(&mut self, account_id: Uuid) -> CoreResult<StoredAccount> {
        self.mark_account_outcome(account_id, StoredAccountOutcome::Started)
    }

    /// Records that a login stage failed for this account only.
    pub(crate) fn mark_account_login_failed(
        &mut self,
        account_id: Uuid,
    ) -> CoreResult<StoredAccount> {
        self.mark_account_outcome(account_id, StoredAccountOutcome::LoginFailed)
    }

    fn mark_account_outcome(
        &mut self,
        account_id: Uuid,
        outcome: StoredAccountOutcome,
    ) -> CoreResult<StoredAccount> {
        let wall_clock = Self::now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identifier = account_id.hyphenated().to_string();
        if query_account(&transaction, account_id)?.is_none() {
            return Err(CoreError::AccountNotFound {
                account_id: identifier,
            });
        }
        let stamp = monotonic_account_time(&transaction, account_id, wall_clock)?;
        transaction.execute(
            "UPDATE accounts SET last_outcome = ?1, last_run_at_unix_ms = ?2, \
             revision = revision + 1, updated_at_unix_ms = ?2 WHERE account_id = ?3",
            params![outcome.as_str(), stamp, identifier],
        )?;
        increment_global_revision(&transaction)?;
        let result =
            query_account(&transaction, account_id)?.ok_or(CoreError::AccountNotFound {
                account_id: identifier,
            })?;
        transaction.commit()?;
        Ok(result)
    }

    /// Resolves the single pinned runtime row every imported profile binds to.
    fn pinned_runtime_id(&self) -> CoreResult<String> {
        let runtime_id: Option<String> = self
            .connection
            .query_row("SELECT runtime_id FROM runtimes LIMIT 2", [], |row| {
                row.get(0)
            })
            .optional()?;
        runtime_id.ok_or(CoreError::RuntimeNotFound {
            runtime_id: String::new(),
        })
    }
}

/// ASCII-lowercase collation key enforcing case-insensitive uniqueness.
fn username_key(username: &str) -> String {
    username.to_ascii_lowercase()
}

/// Printable ASCII, exactly trimmed, 1..=64 bytes.
fn validate_username(value: &str) -> CoreResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > USERNAME_MAX_BYTES
        || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(CoreError::InvalidUsername);
    }
    Ok(())
}

/// Printable ASCII, preserved exactly, 1..=128 bytes. The value never reaches an error message.
fn validate_password(password: &SecretBytes) -> CoreResult<()> {
    let bytes = password.expose_for_validation();
    if bytes.is_empty()
        || bytes.len() > PASSWORD_MAX_BYTES
        || !bytes.iter().all(|byte| (0x20..=0x7e).contains(byte))
    {
        return Err(CoreError::InvalidPassword);
    }
    Ok(())
}

fn validate_expected_revision(value: i64) -> CoreResult<()> {
    if value < 1 {
        return Err(CoreError::InvalidRevision);
    }
    Ok(())
}

fn count_accounts(transaction: &Transaction<'_>) -> CoreResult<i64> {
    Ok(transaction.query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))?)
}

fn username_key_exists(
    transaction: &Transaction<'_>,
    key: &str,
    excluding: Option<Uuid>,
) -> CoreResult<bool> {
    let excluded = excluding
        .map(|id| id.hyphenated().to_string())
        .unwrap_or_default();
    let found: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM accounts WHERE username_key = ?1 AND account_id <> ?2",
            params![key, excluded],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn query_account(
    transaction: &Transaction<'_>,
    account_id: Uuid,
) -> CoreResult<Option<StoredAccount>> {
    let sql = format!("SELECT {ACCOUNT_SELECT_COLUMNS} FROM accounts WHERE account_id = ?1");
    transaction
        .query_row(
            &sql,
            params![account_id.hyphenated().to_string()],
            row_to_account,
        )
        .optional()?
        .transpose()
}

fn require_account(
    transaction: &Transaction<'_>,
    account_id: Uuid,
    expected_revision: i64,
) -> CoreResult<StoredAccount> {
    let current =
        query_account(transaction, account_id)?.ok_or_else(|| CoreError::AccountNotFound {
            account_id: account_id.hyphenated().to_string(),
        })?;
    if current.revision != expected_revision {
        return Err(CoreError::AccountRevisionConflict {
            account_id: account_id.hyphenated().to_string(),
            expected: expected_revision,
            actual: current.revision,
        });
    }
    Ok(current)
}

/// Clamps an audit stamp so a backwards system clock can never move a row's time backwards.
fn monotonic_account_time(
    transaction: &Transaction<'_>,
    account_id: Uuid,
    wall_clock: i64,
) -> CoreResult<i64> {
    let stored: i64 = transaction.query_row(
        "SELECT updated_at_unix_ms FROM accounts WHERE account_id = ?1",
        params![account_id.hyphenated().to_string()],
        |row| row.get(0),
    )?;
    Ok(wall_clock.max(stored))
}

/// Projects one row, deferring fallible parsing so SQLite mapping stays infallible.
fn row_to_account(row: &Row<'_>) -> rusqlite::Result<CoreResult<StoredAccount>> {
    let identifier: String = row.get(0)?;
    let revision: i64 = row.get(1)?;
    let username: String = row.get(2)?;
    let profile_id: String = row.get(3)?;
    let config_json: String = row.get(4)?;
    let config_revision: i64 = row.get(5)?;
    let last_run_at_unix_ms: Option<i64> = row.get(6)?;
    let last_outcome: Option<String> = row.get(7)?;
    Ok((|| {
        let account_id = Uuid::parse_str(&identifier).map_err(|_| CoreError::UnmanagedDatabase)?;
        Ok(StoredAccount {
            account_id,
            revision,
            username,
            profile_id,
            config: AccountConfigV1::parse_strict(&config_json)?,
            config_revision,
            last_run_at_unix_ms,
            last_outcome: last_outcome
                .as_deref()
                .map(StoredAccountOutcome::parse)
                .transpose()?,
        })
    })())
}

/// Reports whether the encrypted password of this account round-trips to `expected`.
#[cfg(test)]
pub(crate) fn stored_password_matches(
    core: &CoreState,
    account_id: Uuid,
    expected: &[u8],
) -> CoreResult<bool> {
    let (cipher, nonce, tag): (Vec<u8>, Vec<u8>, Vec<u8>) = core.connection.query_row(
        "SELECT password_cipher, password_nonce, password_tag FROM accounts WHERE account_id = ?1",
        params![account_id.hyphenated().to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let encrypted = EncryptedPasswordV1 {
        version: 1,
        cipher,
        nonce: nonce.try_into().map_err(|_| CoreError::UnmanagedDatabase)?,
        tag: tag.try_into().map_err(|_| CoreError::UnmanagedDatabase)?,
    };
    let secret = core
        .credential_vault
        .cipher()?
        .decrypt(account_id, &encrypted)?;
    Ok(secret.expose_for_validation() == expected)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::params;
    use uuid::Uuid;

    use super::{MAX_ACCOUNTS, StoredAccountOutcome, stored_password_matches};
    use crate::credential_vault::SecretBytes;
    use crate::{CoreError, CoreState};

    const RUNTIME_ID: &str = "windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("zeus-account-repository-unit-{}", Uuid::new_v4())),
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

    fn open_core(root: &TestDirectory) -> CoreState {
        let core = CoreState::open_at(root.path()).expect("open Core");
        seed_runtime(&core);
        core
    }

    /// Inserts the pinned runtime row directly so the unit tier needs no on-disk runtime tree.
    fn seed_runtime(core: &CoreState) {
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
                    RUNTIME_ID,
                    digest,
                    join_native("descriptor.json"),
                    native_root(),
                    if cfg!(windows) { "windows" } else { "ubuntu" },
                    join_native("java"),
                    join_native("microemulator.jar"),
                    join_native("game.jar"),
                ],
            )
            .expect("seed pinned runtime");
    }

    fn native_root() -> String {
        if cfg!(windows) {
            r"C:\runtime".to_owned()
        } else {
            "/runtime".to_owned()
        }
    }

    fn join_native(leaf: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\runtime\{leaf}")
        } else {
            format!("/runtime/{leaf}")
        }
    }

    fn secret(value: &str) -> SecretBytes {
        SecretBytes::new(value.as_bytes().to_vec())
    }

    fn global_revision(core: &CoreState) -> i64 {
        core.connection
            .query_row(
                "SELECT global_revision FROM schema_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("read global revision")
    }

    fn profile_directory_count(core: &CoreState) -> usize {
        fs::read_dir(core.data_root().path().join("profiles"))
            .expect("enumerate profiles")
            .count()
    }

    #[test]
    fn import_creates_one_account_profile_config_and_directory_in_one_revision() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let before = global_revision(&core);

        let account = core
            .create_account_with_profile("PortableUser", secret("Secret-1"))
            .expect("import account");

        assert_eq!(account.revision, 1);
        assert_eq!(account.username, "PortableUser");
        assert_eq!(account.config_revision, 1);
        assert_eq!(account.last_run_at_unix_ms, None);
        assert_eq!(account.last_outcome, None);
        // Default config round-trips through the stored canonical JSON.
        assert_eq!(account.config, crate::account::AccountConfigV1::defaults());
        // Exactly one revision increment for the whole aggregate.
        assert_eq!(global_revision(&core), before + 1);
        assert_eq!(profile_directory_count(&core), 1);

        let profile = core
            .inspect_profile(&account.profile_id)
            .expect("linked profile");
        assert_eq!(profile.display_name, "PortableUser");
        assert_eq!(profile.runtime_id, RUNTIME_ID);
        assert_eq!(profile.archived_at_unix_ms, None);
        // The private profile directory is real and verified.
        assert!(
            core.data_root()
                .verified_profile_directory(&account.profile_id)
                .is_ok()
        );
        assert!(
            stored_password_matches(&core, account.account_id, b"Secret-1")
                .expect("decrypt stored password")
        );
    }

    #[test]
    fn duplicate_username_is_rejected_ignoring_case_and_leaves_no_residue() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        core.create_account_with_profile("PortableUser", secret("Secret-1"))
            .expect("first import");
        let revision = global_revision(&core);

        assert!(matches!(
            core.create_account_with_profile("portableUSER", secret("Secret-2")),
            Err(CoreError::DuplicateUsername)
        ));

        // A rolled-back import leaves neither row nor an orphan UUID directory.
        assert_eq!(core.list_accounts().expect("list").len(), 1);
        assert_eq!(profile_directory_count(&core), 1);
        assert_eq!(global_revision(&core), revision);
    }

    #[test]
    fn import_rejects_invalid_username_and_password_bounds() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);

        for candidate in [" leading", "trailing ", "", "tab\there"] {
            assert!(matches!(
                core.create_account_with_profile(candidate, secret("Secret-1")),
                Err(CoreError::InvalidUsername)
            ));
        }
        assert!(matches!(
            core.create_account_with_profile(&"u".repeat(65), secret("Secret-1")),
            Err(CoreError::InvalidUsername)
        ));
        assert!(
            core.create_account_with_profile(&"u".repeat(64), secret("Secret-1"))
                .is_ok()
        );

        assert!(matches!(
            core.create_account_with_profile("BoundsUser", secret("")),
            Err(CoreError::InvalidPassword)
        ));
        assert!(matches!(
            core.create_account_with_profile("BoundsUser", secret(&"p".repeat(129))),
            Err(CoreError::InvalidPassword)
        ));
        assert!(matches!(
            core.create_account_with_profile("BoundsUser", secret("bad\u{7f}")),
            Err(CoreError::InvalidPassword)
        ));
        // The exact upper bound is accepted.
        assert!(
            core.create_account_with_profile("BoundsUser", secret(&"p".repeat(128)))
                .is_ok()
        );
        assert_eq!(profile_directory_count(&core), 2);
    }

    #[test]
    fn account_limit_is_enforced_at_one_hundred() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        for index in 0..MAX_ACCOUNTS {
            core.create_account_with_profile(&format!("User{index:03}"), secret("Secret-1"))
                .expect("import within the limit");
        }
        let revision = global_revision(&core);

        assert!(matches!(
            core.create_account_with_profile("Overflow", secret("Secret-1")),
            Err(CoreError::AccountLimitReached { maximum }) if maximum == MAX_ACCOUNTS
        ));
        assert_eq!(
            core.list_accounts().expect("list").len(),
            MAX_ACCOUNTS as usize
        );
        assert_eq!(profile_directory_count(&core), MAX_ACCOUNTS as usize);
        assert_eq!(global_revision(&core), revision);
    }

    #[test]
    fn edit_renames_account_and_profile_without_touching_an_unchanged_password() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("BeforeName", secret("Secret-1"))
            .expect("import account");
        let ciphertext_before: Vec<u8> = core
            .connection
            .query_row(
                "SELECT password_cipher FROM accounts WHERE account_id = ?1",
                params![account.account_id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .expect("read stored ciphertext");
        let revision = global_revision(&core);

        let renamed = core
            .update_account_and_profile(account.account_id, 1, "AfterName", None)
            .expect("rename account");

        assert_eq!(renamed.revision, 2);
        assert_eq!(renamed.username, "AfterName");
        assert_eq!(renamed.profile_id, account.profile_id);
        assert_eq!(
            core.inspect_profile(&account.profile_id)
                .expect("renamed profile")
                .display_name,
            "AfterName"
        );
        assert_eq!(global_revision(&core), revision + 1);
        // Rename never re-encrypts: the stored ciphertext is byte-identical.
        let ciphertext_after: Vec<u8> = core
            .connection
            .query_row(
                "SELECT password_cipher FROM accounts WHERE account_id = ?1",
                params![account.account_id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .expect("read stored ciphertext");
        assert_eq!(ciphertext_after, ciphertext_before);
        assert!(
            stored_password_matches(&core, account.account_id, b"Secret-1")
                .expect("password still decrypts")
        );
    }

    #[test]
    fn edit_rotates_the_password_when_a_replacement_is_supplied() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("RotateUser", secret("Secret-1"))
            .expect("import account");

        let rotated = core
            .update_account_and_profile(
                account.account_id,
                1,
                "RotateUser",
                Some(secret("Secret-2")),
            )
            .expect("rotate password");

        assert_eq!(rotated.revision, 2);
        assert!(
            stored_password_matches(&core, account.account_id, b"Secret-2")
                .expect("rotated password decrypts")
        );
        assert!(
            !stored_password_matches(&core, account.account_id, b"Secret-1")
                .expect("old password no longer matches")
        );
    }

    #[test]
    fn edit_enforces_revision_and_duplicate_username() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let first = core
            .create_account_with_profile("FirstUser", secret("Secret-1"))
            .expect("first import");
        let second = core
            .create_account_with_profile("SecondUser", secret("Secret-2"))
            .expect("second import");

        assert!(matches!(
            core.update_account_and_profile(first.account_id, 7, "Renamed", None),
            Err(CoreError::AccountRevisionConflict { expected, actual, .. })
                if expected == 7 && actual == 1
        ));
        assert!(matches!(
            core.update_account_and_profile(second.account_id, 1, "firstuser", None),
            Err(CoreError::DuplicateUsername)
        ));
        assert!(matches!(
            core.update_account_and_profile(Uuid::new_v4(), 1, "Missing", None),
            Err(CoreError::AccountNotFound { .. })
        ));
        // Renaming an account to its own username, differing only in case, stays allowed.
        assert!(
            core.update_account_and_profile(second.account_id, 1, "seconduser", None)
                .is_ok()
        );
    }

    #[test]
    fn delete_removes_the_account_and_archives_its_profile() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("DeleteUser", secret("Secret-1"))
            .expect("import account");
        let revision = global_revision(&core);

        core.delete_account_and_archive_profile(account.account_id, 1)
            .expect("delete account");

        assert!(core.list_accounts().expect("list").is_empty());
        assert!(
            core.inspect_profile(&account.profile_id)
                .expect("profile row survives")
                .archived_at_unix_ms
                .is_some()
        );
        assert_eq!(global_revision(&core), revision + 1);
        // The same username may be imported again after deletion.
        assert!(
            core.create_account_with_profile("DeleteUser", secret("Secret-1"))
                .is_ok()
        );
    }

    #[test]
    fn delete_enforces_revision_and_existence() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("GuardUser", secret("Secret-1"))
            .expect("import account");

        assert!(matches!(
            core.delete_account_and_archive_profile(account.account_id, 4),
            Err(CoreError::AccountRevisionConflict { .. })
        ));
        assert!(matches!(
            core.delete_account_and_archive_profile(Uuid::new_v4(), 1),
            Err(CoreError::AccountNotFound { .. })
        ));
        assert!(matches!(
            core.delete_account_and_archive_profile(account.account_id, 0),
            Err(CoreError::InvalidRevision)
        ));
        assert_eq!(core.list_accounts().expect("list").len(), 1);
    }

    #[test]
    fn run_outcomes_record_last_run_and_clear_a_stale_failure() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("OutcomeUser", secret("Secret-1"))
            .expect("import account");

        let failed = core
            .mark_account_login_failed(account.account_id)
            .expect("record login failure");
        assert_eq!(failed.last_outcome, Some(StoredAccountOutcome::LoginFailed));
        let first_stamp = failed.last_run_at_unix_ms.expect("failure stamps last run");

        let started = core
            .mark_account_started(account.account_id)
            .expect("record started");
        // Admission clears the stale failure and refreshes the run stamp monotonically.
        assert_eq!(started.last_outcome, Some(StoredAccountOutcome::Started));
        assert!(started.last_run_at_unix_ms.expect("started stamp") >= first_stamp);
        assert_eq!(started.revision, 3);

        assert!(matches!(
            core.mark_account_started(Uuid::new_v4()),
            Err(CoreError::AccountNotFound { .. })
        ));
    }

    #[test]
    fn audit_stamps_never_move_backwards_under_a_backwards_clock() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("ClockUser", secret("Secret-1"))
            .expect("import account");
        // Force a stored stamp far in the future, as a forward clock skew would.
        let future = 4_000_000_000_000_i64;
        core.connection
            .execute(
                "UPDATE accounts SET created_at_unix_ms = ?1, updated_at_unix_ms = ?1 \
                 WHERE account_id = ?2",
                params![future, account.account_id.hyphenated().to_string()],
            )
            .expect("skew stored stamps");

        let updated = core
            .update_account_and_profile(account.account_id, 1, "ClockUserRenamed", None)
            .expect("rename under a backwards clock");

        let stamp: i64 = core
            .connection
            .query_row(
                "SELECT updated_at_unix_ms FROM accounts WHERE account_id = ?1",
                params![updated.account_id.hyphenated().to_string()],
                |row| row.get(0),
            )
            .expect("read clamped stamp");
        assert!(stamp >= future, "stamp {stamp} moved behind {future}");
    }

    #[test]
    fn list_accounts_is_sorted_by_username_key_and_bounded() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        for username in ["zeta", "Alpha", "middle"] {
            core.create_account_with_profile(username, secret("Secret-1"))
                .expect("import account");
        }

        let listed = core.list_accounts().expect("list accounts");
        assert_eq!(
            listed
                .iter()
                .map(|account| account.username.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "middle", "zeta"]
        );
        assert!(listed.len() <= MAX_ACCOUNTS as usize);
    }

    #[test]
    fn seeding_writes_the_stores_the_client_reads_and_stopping_removes_them() {
        let root = TestDirectory::new();
        let mut core = open_core(&root);
        let account = core
            .create_account_with_profile("SeedUser", secret("Secret-1"))
            .expect("import account");

        core.seed_account_login(account.account_id)
            .expect("seeding succeeds for a stored account");
        let suite = root
            .path()
            .join("profiles")
            .join(&account.profile_id)
            .join("microemu-home")
            .join(".microemulator")
            .join("suite-null");
        let credentials = suite.join("user_pass.rs");
        let server = suite.join("isIndexServer.rs");
        assert!(
            credentials.is_file(),
            "the credential store was not written"
        );
        assert!(server.is_file(), "the server store was not written");

        // The vault's decrypt direction is exercised here: the seeded record must carry the real
        // password, obfuscated exactly as the MIDlet expects, never the ciphertext and never cleartext.
        let stored = fs::read(&credentials).expect("the credential store is readable");
        let plain: Vec<u8> = stored.iter().map(|byte| !byte).collect();
        assert!(
            plain
                .windows("Secret-1".len())
                .any(|window| window == b"Secret-1"),
            "the seeded record does not decode to the stored password"
        );
        assert!(
            !stored
                .windows("Secret-1".len())
                .any(|window| window == b"Secret-1"),
            "the password was written in cleartext"
        );
        // Default server index, complemented: the first entry of the client's table.
        assert_eq!(
            fs::read(&server)
                .expect("the server store is readable")
                .last(),
            Some(&0xffu8)
        );

        core.clear_account_login(account.account_id)
            .expect("clearing succeeds");
        assert!(
            !credentials.exists(),
            "the credential store outlived the stop"
        );
        assert!(!server.exists(), "the server store outlived the stop");

        // An unknown account cannot seed, so a deleted row can never leave a store behind.
        assert!(matches!(
            core.seed_account_login(Uuid::new_v4()),
            Err(CoreError::AccountNotFound { .. })
        ));
    }

    #[test]
    fn import_requires_a_pinned_runtime_row() {
        let root = TestDirectory::new();
        // Deliberately skip seed_runtime: no pinned runtime exists yet.
        let mut core = CoreState::open_at(root.path()).expect("open Core");

        assert!(matches!(
            core.create_account_with_profile("NoRuntime", secret("Secret-1")),
            Err(CoreError::RuntimeNotFound { .. })
        ));
        assert!(core.list_accounts().expect("list").is_empty());
        assert_eq!(profile_directory_count(&core), 0);
    }
}

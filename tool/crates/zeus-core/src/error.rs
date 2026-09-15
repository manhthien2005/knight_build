use std::error::Error;
use std::fmt;
use std::io;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug)]
pub enum CoreError {
    InvalidDataRoot {
        reason: &'static str,
    },
    UnmanagedDataRoot,
    InsecureDataRoot,
    AlreadyRunning,
    DatabaseTooLarge {
        limit_bytes: u64,
    },
    UnsupportedSchema {
        found: u32,
        supported: u32,
    },
    UnmanagedDatabase,
    RuntimeValidation {
        code: &'static str,
    },
    RuntimeConflict {
        runtime_id: String,
    },
    RuntimeRegistryMismatch {
        runtime_id: String,
    },
    ProcessLaunchSpec {
        code: &'static str,
    },
    InvalidPageLimit {
        maximum: u32,
    },
    InvalidProfileName,
    InvalidProfileId,
    InvalidRevision,
    RuntimeNotFound {
        runtime_id: String,
    },
    ProfileNotFound {
        profile_id: String,
    },
    ProfileArchived {
        profile_id: String,
    },
    RevisionConflict {
        profile_id: String,
        expected: i64,
        actual: i64,
    },
    ProfileDirectoryCollision,
    PortableRepair {
        code: &'static str,
    },
    InvalidUsername,
    InvalidPassword,
    DuplicateUsername,
    AccountLimitReached {
        maximum: u32,
    },
    AccountNotFound {
        account_id: String,
    },
    AccountRevisionConflict {
        account_id: String,
        expected: i64,
        actual: i64,
    },
    CredentialVaultUnavailable,
    RmsSeed {
        code: &'static str,
    },
    PlayerSnapshot {
        code: &'static str,
    },
    ControlSettings {
        code: &'static str,
    },
    SavedSpots {
        code: &'static str,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Database(rusqlite::Error),
}

impl CoreError {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidDataRoot { .. } => "InvalidDataRoot",
            Self::UnmanagedDataRoot => "UnmanagedDataRoot",
            Self::InsecureDataRoot => "InsecureDataRoot",
            Self::AlreadyRunning => "AlreadyRunning",
            Self::DatabaseTooLarge { .. } => "DatabaseTooLarge",
            Self::UnsupportedSchema { .. } => "UnsupportedSchema",
            Self::UnmanagedDatabase => "UnmanagedDatabase",
            Self::RuntimeValidation { code } => code,
            Self::RuntimeConflict { .. } => "RuntimeConflict",
            Self::RuntimeRegistryMismatch { .. } => "RuntimeRegistryMismatch",
            Self::ProcessLaunchSpec { code } => code,
            Self::InvalidPageLimit { .. } => "InvalidPageLimit",
            Self::InvalidProfileName => "InvalidProfileName",
            Self::InvalidProfileId => "InvalidProfileId",
            Self::InvalidRevision => "InvalidRevision",
            Self::RuntimeNotFound { .. } => "RuntimeNotFound",
            Self::ProfileNotFound { .. } => "ProfileNotFound",
            Self::ProfileArchived { .. } => "ProfileArchived",
            Self::RevisionConflict { .. } => "RevisionConflict",
            Self::ProfileDirectoryCollision => "ProfileDirectoryCollision",
            Self::PortableRepair { .. } => "PortableRepair",
            Self::InvalidUsername => "InvalidUsername",
            Self::InvalidPassword => "InvalidPassword",
            Self::DuplicateUsername => "DuplicateUsername",
            Self::AccountLimitReached { .. } => "AccountLimitReached",
            Self::AccountNotFound { .. } => "AccountNotFound",
            Self::AccountRevisionConflict { .. } => "AccountRevisionConflict",
            Self::CredentialVaultUnavailable => "CredentialVaultUnavailable",
            Self::RmsSeed { code } => code,
            Self::PlayerSnapshot { code } => code,
            Self::ControlSettings { code } => code,
            Self::SavedSpots { code } => code,
            Self::Io { .. } => "IoFailure",
            Self::Database(_) => "DatabaseFailure",
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDataRoot { reason } => write!(formatter, "invalid data root: {reason}"),
            Self::UnmanagedDataRoot => {
                formatter.write_str("data root is nonempty and is not managed by Zeus HSO")
            }
            Self::InsecureDataRoot => formatter.write_str("data root permissions are not private"),
            Self::AlreadyRunning => formatter.write_str("another Core already owns the data root"),
            Self::DatabaseTooLarge { limit_bytes } => {
                write!(
                    formatter,
                    "database reached its {limit_bytes}-byte hard limit"
                )
            }
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "database schema {found} is newer than supported schema {supported}"
            ),
            Self::UnmanagedDatabase => {
                formatter.write_str("unversioned database contains unmanaged tables")
            }
            Self::RuntimeValidation { code } => {
                write!(formatter, "runtime validation failed: {code}")
            }
            Self::RuntimeConflict { runtime_id } => {
                write!(
                    formatter,
                    "runtime ID is immutable and already registered: {runtime_id}"
                )
            }
            Self::RuntimeRegistryMismatch { runtime_id } => {
                write!(
                    formatter,
                    "runtime no longer matches its registry record: {runtime_id}"
                )
            }
            Self::ProcessLaunchSpec { code } => {
                write!(formatter, "process launch specification rejected: {code}")
            }
            Self::InvalidPageLimit { maximum } => {
                write!(formatter, "page limit must be between 1 and {maximum}")
            }
            Self::InvalidProfileName => formatter.write_str("invalid profile display name"),
            Self::InvalidProfileId => formatter.write_str("invalid profile UUID"),
            Self::InvalidRevision => formatter.write_str("expected revision must be positive"),
            Self::RuntimeNotFound { runtime_id } => {
                write!(formatter, "runtime was not found: {runtime_id}")
            }
            Self::ProfileNotFound { profile_id } => {
                write!(formatter, "profile was not found: {profile_id}")
            }
            Self::ProfileArchived { profile_id } => {
                write!(formatter, "profile is archived: {profile_id}")
            }
            Self::RevisionConflict {
                profile_id,
                expected,
                actual,
            } => write!(
                formatter,
                "profile revision conflict for {profile_id}: expected {expected}, actual {actual}"
            ),
            Self::ProfileDirectoryCollision => {
                formatter.write_str("could not allocate a unique profile directory")
            }
            Self::PortableRepair { code } => {
                write!(formatter, "portable data-root repair failed: {code}")
            }
            Self::InvalidUsername => formatter.write_str("invalid account username"),
            // Never describe the rejected password value.
            Self::InvalidPassword => formatter.write_str("invalid account password"),
            Self::DuplicateUsername => {
                formatter.write_str("account username is already used, ignoring case")
            }
            Self::AccountLimitReached { maximum } => {
                write!(formatter, "account limit of {maximum} is reached")
            }
            Self::AccountNotFound { account_id } => {
                write!(formatter, "account was not found: {account_id}")
            }
            Self::AccountRevisionConflict {
                account_id,
                expected,
                actual,
            } => write!(
                formatter,
                "account revision conflict for {account_id}: expected {expected}, actual {actual}"
            ),
            Self::CredentialVaultUnavailable => {
                formatter.write_str("portable credential vault is unavailable")
            }
            // The code names the rejected rule only; a credential value must never be described.
            Self::RmsSeed { code } => {
                write!(formatter, "record store seeding failed: {code}")
            }
            // The code names the rejected rule; a snapshot value never reaches a message.
            Self::PlayerSnapshot { code } => {
                write!(formatter, "player snapshot rejected: {code}")
            }
            // The code names the rejected rule; a spot or threshold never reaches a message.
            Self::ControlSettings { code } => {
                write!(formatter, "control settings rejected: {code}")
            }
            // Same shape: the code names the rejected rule, never the coordinates themselves.
            Self::SavedSpots { code } => {
                write!(formatter, "saved spot rejected: {code}")
            }
            Self::Io { operation, .. } => write!(formatter, "I/O operation failed: {operation}"),
            Self::Database(_) => formatter.write_str("SQLite operation failed"),
        }
    }
}

impl Error for CoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Database(source) => Some(source),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(error: rusqlite::Error) -> Self {
        if matches!(
            &error,
            rusqlite::Error::SqliteFailure(details, _)
                if details.code == rusqlite::ErrorCode::DiskFull
        ) {
            return Self::DatabaseTooLarge {
                limit_bytes: crate::MAX_DATABASE_BYTES,
            };
        }
        Self::Database(error)
    }
}

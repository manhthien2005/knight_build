#[cfg(test)]
pub(crate) const SCHEMA_V1_SQL: &str = r#"
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

pub(crate) const ACCOUNTS_V2_SQL: &str = r#"
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
) STRICT;
"#;

pub(crate) const SPOTS_V3_SQL: &str = r#"
CREATE TABLE spots (
    map_id INTEGER NOT NULL CHECK (map_id BETWEEN 0 AND 135),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 48),
    zone INTEGER NOT NULL CHECK (zone BETWEEN -1 AND 127),
    pixel_x INTEGER NOT NULL CHECK (pixel_x >= 0),
    pixel_y INTEGER NOT NULL CHECK (pixel_y >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms > 0),
    PRIMARY KEY (map_id, name)
) STRICT;
"#;

pub(crate) const SCHEMA_V2_SQL: &str = r#"
CREATE TABLE schema_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 3),
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
) STRICT;
"#;

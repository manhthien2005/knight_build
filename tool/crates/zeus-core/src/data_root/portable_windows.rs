//! Bounded Windows repair for a data root that was copied or moved between machines.
//!
//! An Explorer-style copy keeps the destination parent's inherited ACLs and, for a copy taken by
//! another account, a foreign owner. This module re-owns and re-protects exactly the managed shape
//! (data root, `profiles`, direct UUID profile children, and the known root state files) and never
//! recursively repairs unknown descendants.

use std::fs::{self, Metadata};
use std::path::Path;

use uuid::{Uuid, Version};

use crate::error::{CoreError, CoreResult};

use super::{
    ROOT_MARKER_NAME, platform, reject_linked_ancestors, validate_path_shape, validate_root_marker,
};

/// Managed directory child of the data root.
const PROFILES_DIRECTORY_NAME: &str = "profiles";
/// Managed directory children of one profile directory, created on first launch.
const KNOWN_PROFILE_CHILD_DIRECTORIES: &[&str] = &["microemu-home", "temp"];
/// Prefix of the vault-key publish temp file, which survives a kill between create and rename.
const KEY_TEMP_FILE_PREFIX: &str = ".vault.key.tmp-";
/// Prefix of the spot-book publish temp file, which survives a kill between create and rename.
///
/// Built from the file name rather than repeated, so renaming the book cannot leave this behind
/// pointing at a name that no longer exists.
const SPOT_TEMP_FILE_PREFIX: &str = concat!("zeus-spots.txt", ".tmp-");

/// Every root entry that may exist as a plain private file in a managed data root.
const KNOWN_ROOT_STATE_FILES: &[&str] = &[
    ROOT_MARKER_NAME,
    ".core-instance.lock",
    "state.sqlite3",
    "state.sqlite3-journal",
    "state.sqlite3-shm",
    "state.sqlite3-wal",
    "state.lkg.sqlite3",
    ".state.lkg.sqlite3.tmp",
    "vault.key",
    // Shared by every account, so it sits beside the database rather than inside a profile. Listed
    // here because an unlisted root file makes the whole data root read as unmanaged: repair reports
    // `unknown_root_file`, Core refuses to open, and the operator sees only "tool not ready".
    crate::spots::SPOT_FILE_NAME,
];

fn repair_error(code: &'static str) -> CoreError {
    CoreError::PortableRepair { code }
}

/// Repairs owner and DACL of a copied data root before Core opens it.
///
/// A missing root and an empty directory are left untouched for the ordinary private-create and
/// adoption paths in [`super::DataRoot::prepare_at`]. An existing root must carry the exact v1
/// marker before any security change; an unmarked nonempty root stays unmanaged.
pub(super) fn prepare_portable_root(requested: &Path) -> CoreResult<()> {
    validate_path_shape(requested)?;
    platform::ensure_local_filesystem(requested)?;
    reject_linked_ancestors(requested)?;
    if !requested.exists() {
        // The fresh-create path owns a missing root.
        return Ok(());
    }
    let metadata = fs::symlink_metadata(requested)
        .map_err(|error| CoreError::io("inspect portable data root", error))?;
    if !metadata.is_dir() || platform::is_link_or_reparse(&metadata) {
        return Err(CoreError::InvalidDataRoot {
            reason: "root is not a plain directory",
        });
    }

    let marker = requested.join(ROOT_MARKER_NAME);
    match fs::symlink_metadata(&marker) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if root_is_empty(requested)? {
                // Only the fresh-create path may adopt an empty directory.
                Ok(())
            } else {
                Err(CoreError::UnmanagedDataRoot)
            };
        }
        Err(error) => return Err(CoreError::io("inspect portable data-root marker", error)),
    }
    validate_root_marker(&marker)?;

    let shape = enumerate_managed_shape(requested)?;
    let canonical_root = fs::canonicalize(requested)
        .map_err(|error| CoreError::io("canonicalize portable data root", error))?;
    reject_linked_ancestors(&canonical_root)?;
    // Each entry is verified immediately after it is repaired, so a substituted entry is caught
    // before the walk continues rather than at the end of a trailing pass.
    for path in &shape {
        repair_entry(path)?;
        verify_repaired_entry(path, &canonical_root)?;
    }
    Ok(())
}

fn root_is_empty(root: &Path) -> CoreResult<bool> {
    let mut entries =
        fs::read_dir(root).map_err(|error| CoreError::io("enumerate portable data root", error))?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|error| CoreError::io("enumerate portable data-root entry", error))?
        .is_none())
}

/// Collects the managed shape and rejects every link, reparse point, and unknown entry before any
/// security change. The returned order repairs the root before its children.
fn enumerate_managed_shape(root: &Path) -> CoreResult<Vec<std::path::PathBuf>> {
    let mut shape = vec![root.to_owned()];
    let mut profiles = None;
    for entry in
        fs::read_dir(root).map_err(|error| CoreError::io("enumerate portable data root", error))?
    {
        let entry = entry.map_err(|error| CoreError::io("read portable data-root entry", error))?;
        let metadata = entry
            .metadata()
            .map_err(|error| CoreError::io("inspect portable data-root entry", error))?;
        reject_link(&metadata)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| repair_error("root_entry_name"))?;
        if metadata.is_dir() {
            if name != PROFILES_DIRECTORY_NAME {
                return Err(repair_error("unknown_root_directory"));
            }
            profiles = Some(entry.path());
        } else if metadata.is_file() {
            if !KNOWN_ROOT_STATE_FILES.contains(&name)
                && !is_key_temp_file_name(name)
                && !is_spot_temp_file_name(name)
            {
                return Err(repair_error("unknown_root_file"));
            }
            shape.push(entry.path());
        } else {
            return Err(repair_error("unknown_root_entry"));
        }
    }
    let Some(profiles) = profiles else {
        return Ok(shape);
    };
    shape.push(profiles.clone());
    for entry in fs::read_dir(&profiles)
        .map_err(|error| CoreError::io("enumerate portable profiles directory", error))?
    {
        let entry = entry.map_err(|error| CoreError::io("read portable profile entry", error))?;
        let metadata = entry
            .metadata()
            .map_err(|error| CoreError::io("inspect portable profile entry", error))?;
        reject_link(&metadata)?;
        if !metadata.is_dir() {
            return Err(repair_error("unknown_profile_entry"));
        }
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| repair_error("profile_entry_name"))?;
        if !is_profile_directory_name(name) {
            return Err(repair_error("unknown_profile_entry"));
        }
        let profile = entry.path();
        shape.push(profile.clone());
        // Only the two compile-time-known launch directories join the shape. Their contents and every
        // other descendant stay untouched: repair never walks unknown descendants.
        for child in KNOWN_PROFILE_CHILD_DIRECTORIES {
            let path = profile.join(child);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    reject_link(&metadata)?;
                    if !metadata.is_dir() {
                        return Err(repair_error("unknown_profile_entry"));
                    }
                    shape.push(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CoreError::io("inspect portable profile child", error));
                }
            }
        }
    }
    Ok(shape)
}

/// Accepts the vault-key publish temp file, whose UUID suffix is generated per attempt.
fn is_key_temp_file_name(name: &str) -> bool {
    name.strip_prefix(KEY_TEMP_FILE_PREFIX)
        .is_some_and(is_uuid_v4_name)
}

/// Whether a name is a spot-book publish temporary, which survives a kill between write and rename.
///
/// Tolerated for the same reason the vault key's is: refusing it would make a process killed at the
/// wrong instant leave a data root that never opens again.
fn is_spot_temp_file_name(name: &str) -> bool {
    name.strip_prefix(SPOT_TEMP_FILE_PREFIX)
        .is_some_and(is_uuid_v4_name)
}

fn reject_link(metadata: &Metadata) -> CoreResult<()> {
    if platform::is_link_or_reparse(metadata) {
        return Err(repair_error("linked_entry"));
    }
    Ok(())
}

fn is_profile_directory_name(name: &str) -> bool {
    is_uuid_v4_name(name)
}

/// Requires the exact hyphenated lowercase v4 UUID form the store persists.
fn is_uuid_v4_name(name: &str) -> bool {
    name.len() == 36
        && Uuid::parse_str(name).is_ok_and(|parsed| {
            parsed.get_version() == Some(Version::Random) && parsed.to_string() == name
        })
}

/// Assigns the current user as owner when the token permits it, then replaces inherited ACLs with
/// the protected current-user/System allowlist.
fn repair_entry(path: &Path) -> CoreResult<()> {
    if !platform::owner_is_current_user(path)?
        && !platform::assign_current_user_owner(path)?
        && !platform::owner_is_current_user(path)?
    {
        return Err(repair_error("owner_repair_denied"));
    }
    platform::enforce_private_file_mode(path).map_err(|_| repair_error("acl_repair_denied"))
}

/// Re-verifies owner, DACL, entry type, and canonical parent after repair.
fn verify_repaired_entry(path: &Path, canonical_root: &Path) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CoreError::io("inspect repaired portable entry", error))?;
    if platform::is_link_or_reparse(&metadata) {
        return Err(repair_error("linked_entry"));
    }
    if !platform::owner_is_current_user(path)? {
        return Err(repair_error("owner_not_repaired"));
    }
    let private = if metadata.is_dir() {
        platform::is_private_directory(path)?
    } else if metadata.is_file() {
        platform::is_private_file(path)?
    } else {
        return Err(repair_error("unknown_root_entry"));
    };
    if !private {
        return Err(repair_error("acl_not_repaired"));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| CoreError::io("canonicalize repaired portable entry", error))?;
    if canonical == *canonical_root {
        return Ok(());
    }
    let parent = canonical
        .parent()
        .ok_or_else(|| repair_error("entry_escaped_root"))?;
    let profiles = canonical_root.join(PROFILES_DIRECTORY_NAME);
    if parent == canonical_root || parent == profiles {
        return Ok(());
    }
    // A known launch directory sits exactly one level below its UUID profile directory.
    let is_known_launch_directory = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| KNOWN_PROFILE_CHILD_DIRECTORIES.contains(&name));
    let parent_is_profile_directory = parent.parent() == Some(profiles.as_path())
        && parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_profile_directory_name);
    if is_known_launch_directory && parent_is_profile_directory {
        return Ok(());
    }
    Err(repair_error("entry_escaped_root"))
}

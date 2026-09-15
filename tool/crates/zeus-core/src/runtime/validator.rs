use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::data_root::{
    canonical_secure_runtime_root, metadata_is_link_or_reparse, reject_linked_path,
    runtime_entry_is_secure, secure_runtime_path,
};
use crate::error::{CoreError, CoreResult};

use super::descriptor::{
    GameDescriptor, JavaDescriptor, LaunchDefaultsDescriptor, MicroEmulatorDescriptor,
    RuntimeDescriptor, ValidationDescriptor,
};
use super::validation_error;
use super::{JreMetadataFingerprint, StableMetadata};

pub const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_JAR_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_JRE_FILES: usize = 10_000;
const MAX_JRE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityState {
    Supported,
    NeedsValidation,
    Unavailable,
    OutOfScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedRuntime {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedLaunchDefaults {
    pub main_class: String,
    pub midlet_class: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub heap_initial_mib: u32,
    pub heap_max_mib: u32,
    pub quiet: bool,
    pub quit_on_midlet_destroy: bool,
}

pub(crate) struct RuntimePreflightValidation {
    pub runtime: ValidatedRuntime,
    pub launch_defaults: ValidatedLaunchDefaults,
    pub jre_metadata_fingerprint: JreMetadataFingerprint,
    pub content_bytes_hashed: u64,
    pub jre_content_bytes_hashed: u64,
}

pub fn validate_runtime_descriptor(descriptor_path: &Path) -> CoreResult<ValidatedRuntime> {
    Ok(validate_runtime_descriptor_full(descriptor_path)?.runtime)
}

pub(crate) fn validate_runtime_descriptor_full(
    descriptor_path: &Path,
) -> CoreResult<RuntimePreflightValidation> {
    validate_runtime_descriptor_preflight(descriptor_path, JreValidationMode::Full)
}

pub(crate) fn validate_runtime_descriptor_fast(
    descriptor_path: &Path,
    expected_fingerprint: &JreMetadataFingerprint,
) -> CoreResult<RuntimePreflightValidation> {
    validate_runtime_descriptor_preflight(
        descriptor_path,
        JreValidationMode::Fast(expected_fingerprint),
    )
}

enum JreValidationMode<'a> {
    Full,
    Fast(&'a JreMetadataFingerprint),
}

fn validate_runtime_descriptor_preflight(
    descriptor_path: &Path,
    mode: JreValidationMode<'_>,
) -> CoreResult<RuntimePreflightValidation> {
    // A missing or unreadable descriptor is a runtime validation failure, not a storage failure: every
    // other unreadable-descriptor branch below already reports `descriptor_path_invalid`, and mapping
    // this one to an IO error made a missing runtime surface to the operator as a data-root fault.
    let descriptor_metadata = fs::symlink_metadata(descriptor_path)
        .map_err(|_| validation_error("descriptor_path_invalid"))?;
    if !descriptor_metadata.is_file() || metadata_is_link_or_reparse(&descriptor_metadata) {
        return Err(validation_error("descriptor_path_invalid"));
    }
    if descriptor_metadata.len() > MAX_DESCRIPTOR_BYTES as u64 {
        return Err(validation_error("descriptor_too_large"));
    }

    let requested_root = descriptor_path
        .parent()
        .ok_or_else(|| validation_error("descriptor_path_invalid"))?;
    let runtime_root = canonical_secure_runtime_root(requested_root)
        .map_err(|_| validation_error("runtime_root_insecure"))?;
    let canonical_descriptor = fs::canonicalize(descriptor_path)
        .map_err(|_| validation_error("descriptor_path_invalid"))?;
    if canonical_descriptor.parent() != Some(runtime_root.as_path()) {
        return Err(validation_error("descriptor_path_invalid"));
    }
    if !secure_runtime_path(&runtime_root, &canonical_descriptor)
        .map_err(|_| validation_error("runtime_entry_insecure"))?
    {
        return Err(validation_error("runtime_entry_insecure"));
    }

    let descriptor_bytes = read_bounded_file(&canonical_descriptor, MAX_DESCRIPTOR_BYTES as u64)?;
    let mut content_bytes_hashed = descriptor_bytes.len() as u64;
    let descriptor_sha256 = sha256_bytes(&descriptor_bytes);
    let descriptor: RuntimeDescriptor = serde_json::from_slice(&descriptor_bytes)
        .map_err(|_| validation_error("descriptor_json_invalid"))?;
    validate_descriptor_fields(&descriptor)?;
    validate_host_tuple(&descriptor.platform.os, &descriptor.platform.architecture)?;

    let manifest_path = resolve_relative_file(
        &runtime_root,
        &descriptor.java.tree_manifest,
        "artifact_path_invalid",
    )?;
    let manifest_bytes = read_bounded_file(&manifest_path, MAX_MANIFEST_BYTES)?;
    content_bytes_hashed = content_bytes_hashed
        .checked_add(manifest_bytes.len() as u64)
        .ok_or_else(|| validation_error("runtime_content_too_large"))?;
    if sha256_bytes(&manifest_bytes) != normalize_sha256(&descriptor.java.tree_manifest_sha256)? {
        return Err(validation_error("manifest_hash_mismatch"));
    }
    let manifest = parse_manifest(
        &manifest_bytes,
        descriptor.java.tree_file_count,
        &descriptor.platform.os,
    )?;
    let jre_root = resolve_relative_directory(&runtime_root, "jre")?;
    let (jre_metadata_fingerprint, jre_content_bytes_hashed) =
        validate_jre_tree(&jre_root, &manifest, &descriptor.platform.os, mode)?;
    content_bytes_hashed = content_bytes_hashed
        .checked_add(jre_content_bytes_hashed)
        .ok_or_else(|| validation_error("runtime_content_too_large"))?;

    let java_path = validate_java_executables(&jre_root, &descriptor.platform.os)?;
    let microemulator_path = validate_artifact(
        &runtime_root,
        &descriptor.microemulator.jar,
        descriptor.microemulator.jar_size,
        &descriptor.microemulator.jar_sha256,
    )?;
    let game_path = validate_artifact(
        &runtime_root,
        &descriptor.game.jar,
        descriptor.game.jar_size,
        &descriptor.game.jar_sha256,
    )?;
    content_bytes_hashed = content_bytes_hashed
        .checked_add(descriptor.microemulator.jar_size)
        .and_then(|bytes| bytes.checked_add(descriptor.game.jar_size))
        .ok_or_else(|| validation_error("runtime_content_too_large"))?;

    let launch_defaults = ValidatedLaunchDefaults {
        main_class: descriptor.launch_defaults.main_class,
        midlet_class: descriptor.launch_defaults.midlet_class,
        screen_width: descriptor.launch_defaults.screen_width,
        screen_height: descriptor.launch_defaults.screen_height,
        heap_initial_mib: descriptor.launch_defaults.heap_initial_mib,
        heap_max_mib: descriptor.launch_defaults.heap_max_mib,
        quiet: descriptor.launch_defaults.quiet,
        quit_on_midlet_destroy: descriptor.launch_defaults.quit_on_midlet_destroy,
    };
    let runtime = ValidatedRuntime {
        runtime_id: descriptor.runtime_id,
        descriptor_sha256,
        descriptor_path: canonical_descriptor,
        runtime_root,
        target_os: descriptor.platform.os,
        target_arch: descriptor.platform.architecture,
        java_path,
        java_vendor: descriptor.java.vendor,
        java_version: descriptor.java.version,
        jre_manifest_sha256: normalize_sha256(&descriptor.java.tree_manifest_sha256)?,
        microemulator_path,
        microemulator_version: descriptor.microemulator.version,
        microemulator_sha256: normalize_sha256(&descriptor.microemulator.jar_sha256)?,
        game_path,
        game_bundle: descriptor.game.bundle,
        game_sha256: normalize_sha256(&descriptor.game.jar_sha256)?,
        capability_state: CapabilityState::NeedsValidation,
        validation_reason:
            "static descriptor, manifest and artifact hashes passed; runtime probes pending"
                .to_owned(),
        validated_at_unix_ms: now_unix_ms()?,
    };
    Ok(RuntimePreflightValidation {
        runtime,
        launch_defaults,
        jre_metadata_fingerprint,
        content_bytes_hashed,
        jre_content_bytes_hashed,
    })
}

fn validate_descriptor_fields(descriptor: &RuntimeDescriptor) -> CoreResult<()> {
    if descriptor.schema_version != 1 {
        return Err(validation_error("descriptor_schema_unsupported"));
    }
    if descriptor.runtime_id.is_empty()
        || descriptor.runtime_id.len() > 160
        || !descriptor
            .runtime_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(validation_error("runtime_id_invalid"));
    }
    validate_text(&descriptor.created_at_utc, 16, 64)?;
    if !descriptor.created_at_utc.contains('T') || !descriptor.created_at_utc.ends_with('Z') {
        return Err(validation_error("descriptor_field_invalid"));
    }
    if !matches!(descriptor.platform.os.as_str(), "windows" | "ubuntu")
        || descriptor.platform.architecture != "x64"
    {
        return Err(validation_error("descriptor_platform_invalid"));
    }
    validate_java_descriptor(&descriptor.java)?;
    validate_microemulator_descriptor(&descriptor.microemulator)?;
    validate_game_descriptor(&descriptor.game)?;
    validate_launch_defaults(&descriptor.launch_defaults)?;
    validate_evidence_labels(&descriptor.validation)?;
    Ok(())
}

fn validate_java_descriptor(java: &JavaDescriptor) -> CoreResult<()> {
    for value in [
        &java.vendor,
        &java.distribution,
        &java.jvm,
        &java.version,
        &java.image_type,
        &java.archive_name,
    ] {
        validate_text(value, 1, 256)?;
    }
    if java.image_type != "jre"
        || java.archive_size == 0
        || java.archive_size > MAX_ARCHIVE_BYTES
        || java.tree_file_count == 0
        || java.tree_file_count > MAX_JRE_FILES
        || !java.source.starts_with("https://")
    {
        return Err(validation_error("descriptor_field_invalid"));
    }
    validate_text(&java.source, 9, 2048)?;
    normalize_sha256(&java.archive_sha256)?;
    normalize_sha256(&java.tree_manifest_sha256)?;
    validate_relative_path_text(&java.tree_manifest)?;
    Ok(())
}

fn validate_microemulator_descriptor(value: &MicroEmulatorDescriptor) -> CoreResult<()> {
    for text in [&value.version, &value.archive_name] {
        validate_text(text, 1, 256)?;
    }
    if value.archive_size == 0
        || value.archive_size > MAX_ARCHIVE_BYTES
        || value.jar_size == 0
        || value.jar_size > MAX_JAR_BYTES
        || !value.source.starts_with("https://")
    {
        return Err(validation_error("descriptor_field_invalid"));
    }
    validate_text(&value.source, 9, 2048)?;
    normalize_sha256(&value.archive_sha256)?;
    normalize_sha256(&value.jar_sha256)?;
    validate_relative_path_text(&value.jar)?;
    if !value.optional_jars.is_empty() {
        return Err(validation_error("optional_jars_unpinned"));
    }
    Ok(())
}

fn validate_game_descriptor(game: &GameDescriptor) -> CoreResult<()> {
    for value in [
        &game.name,
        &game.bundle,
        &game.midlet_version,
        &game.profile,
        &game.configuration,
        &game.source_type,
    ] {
        validate_text(value, 1, 256)?;
    }
    if game.jar_size == 0 || game.jar_size > MAX_JAR_BYTES {
        return Err(validation_error("descriptor_field_invalid"));
    }
    validate_relative_path_text(&game.jar)?;
    normalize_sha256(&game.jar_sha256)?;
    Ok(())
}

fn validate_launch_defaults(value: &LaunchDefaultsDescriptor) -> CoreResult<()> {
    if value.mode != "classpath_midlet_main"
        || value.gc != "SerialGC"
        || value.use_perf_data
        || value.rms != "file"
        || !value.quit_on_midlet_destroy
        || !(1..=4096).contains(&value.screen_width)
        || !(1..=4096).contains(&value.screen_height)
        || value.heap_initial_mib == 0
        || value.heap_initial_mib > value.heap_max_mib
        || value.heap_max_mib > 1024
    {
        return Err(validation_error("launch_defaults_unsupported"));
    }
    validate_java_class_name(&value.main_class)?;
    validate_java_class_name(&value.midlet_class)?;
    let _diagnostic_capture_default = !value.quiet;
    Ok(())
}

fn validate_evidence_labels(value: &ValidationDescriptor) -> CoreResult<()> {
    validate_text(&value.status, 1, 256)?;
    validate_text(&value.evidence, 1, 2048)?;
    if value.passed.len() > 64 || value.pending.len() > 64 {
        return Err(validation_error("descriptor_field_invalid"));
    }
    for label in value.passed.iter().chain(&value.pending) {
        validate_text(label, 1, 128)?;
    }
    Ok(())
}

fn validate_text(value: &str, min: usize, max: usize) -> CoreResult<()> {
    if !(min..=max).contains(&value.len()) || value.chars().any(char::is_control) {
        return Err(validation_error("descriptor_field_invalid"));
    }
    Ok(())
}

fn validate_java_class_name(value: &str) -> CoreResult<()> {
    validate_text(value, 1, 256)?;
    if value.split('.').any(|segment| {
        segment.is_empty()
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
            || !segment
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$'))
    }) {
        return Err(validation_error("launch_defaults_unsupported"));
    }
    Ok(())
}

fn validate_host_tuple(os: &str, architecture: &str) -> CoreResult<()> {
    let host_os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") && host_is_ubuntu()? {
        "ubuntu"
    } else {
        "unsupported"
    };
    let host_arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    if os != host_os || architecture != host_arch {
        return Err(validation_error("host_tuple_mismatch"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn host_is_ubuntu() -> CoreResult<bool> {
    let bytes = read_bounded_file(Path::new("/etc/os-release"), 16 * 1024)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| validation_error("host_tuple_mismatch"))?;
    Ok(text.lines().any(|line| line.trim() == "ID=ubuntu"))
}

#[cfg(not(target_os = "linux"))]
fn host_is_ubuntu() -> CoreResult<bool> {
    Ok(false)
}

fn validate_artifact(
    root: &Path,
    relative: &str,
    expected_size: u64,
    expected_sha256: &str,
) -> CoreResult<PathBuf> {
    let path = resolve_relative_file(root, relative, "artifact_path_invalid")?;
    let mut file =
        File::open(&path).map_err(|error| CoreError::io("open runtime artifact", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| CoreError::io("inspect runtime artifact", error))?;
    if metadata.len() != expected_size {
        return Err(validation_error("artifact_size_mismatch"));
    }
    let actual = sha256_reader(&mut file)?;
    if actual != normalize_sha256(expected_sha256)? {
        return Err(validation_error("artifact_hash_mismatch"));
    }
    let final_metadata = file
        .metadata()
        .map_err(|error| CoreError::io("reinspect runtime artifact", error))?;
    if final_metadata.len() != metadata.len() || !final_metadata.is_file() {
        return Err(validation_error("artifact_changed_during_validation"));
    }
    Ok(path)
}

fn validate_java_executables(jre_root: &Path, target_os: &str) -> CoreResult<PathBuf> {
    if target_os == "windows" {
        let java = resolve_relative_file(jre_root, "bin/java.exe", "java_executable_missing")?;
        let javaw = resolve_relative_file(jre_root, "bin/javaw.exe", "java_executable_missing")?;
        if fs::metadata(java)
            .map_err(|error| CoreError::io("inspect Java executable", error))?
            .len()
            == 0
            || fs::metadata(&javaw)
                .map_err(|error| CoreError::io("inspect Java GUI executable", error))?
                .len()
                == 0
        {
            return Err(validation_error("java_executable_missing"));
        }
        Ok(javaw)
    } else {
        let java = resolve_relative_file(jre_root, "bin/java", "java_executable_missing")?;
        if fs::metadata(&java)
            .map_err(|error| CoreError::io("inspect Java executable", error))?
            .len()
            == 0
        {
            return Err(validation_error("java_executable_missing"));
        }
        Ok(java)
    }
}

fn parse_manifest(
    bytes: &[u8],
    expected_count: usize,
    target_os: &str,
) -> CoreResult<BTreeMap<String, ManifestEntry>> {
    let text = std::str::from_utf8(bytes).map_err(|_| validation_error("manifest_utf8_invalid"))?;
    let mut entries = BTreeMap::new();
    for raw_line in text.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            return Err(validation_error("manifest_line_invalid"));
        }
        let (hash, remainder) = line
            .split_once("  ")
            .ok_or_else(|| validation_error("manifest_line_invalid"))?;
        let (size, relative) = remainder
            .split_once("  ")
            .ok_or_else(|| validation_error("manifest_line_invalid"))?;
        let sha256 = normalize_sha256(hash)?;
        let size = size
            .parse::<u64>()
            .map_err(|_| validation_error("manifest_line_invalid"))?;
        let normalized = validate_relative_path_text(relative)?;
        let key = manifest_key(&normalized, target_os);
        if entries
            .insert(
                key,
                ManifestEntry {
                    relative: normalized,
                    size,
                    sha256,
                },
            )
            .is_some()
        {
            return Err(validation_error("manifest_duplicate_path"));
        }
    }
    if entries.len() != expected_count || entries.is_empty() {
        return Err(validation_error("manifest_file_count_mismatch"));
    }
    Ok(entries)
}

struct ManifestEntry {
    relative: String,
    size: u64,
    sha256: String,
}

fn validate_jre_tree(
    jre_root: &Path,
    manifest: &BTreeMap<String, ManifestEntry>,
    target_os: &str,
    mode: JreValidationMode<'_>,
) -> CoreResult<(JreMetadataFingerprint, u64)> {
    let actual = enumerate_plain_jre(jre_root, target_os)?;
    let expected_keys: BTreeSet<&String> = manifest.keys().collect();
    let actual_keys: BTreeSet<&String> = actual.files.keys().collect();
    if actual_keys != expected_keys {
        return Err(validation_error("jre_file_set_mismatch"));
    }

    let mut total_bytes = 0u64;
    for (key, entry) in manifest {
        let path = actual
            .files
            .get(key)
            .ok_or_else(|| validation_error("jre_file_set_mismatch"))?;
        let metadata_key = format!("f:{key}");
        let observed = actual
            .metadata
            .get(&metadata_key)
            .ok_or_else(|| validation_error("jre_file_set_mismatch"))?;
        if observed.size != entry.size {
            return Err(validation_error("manifest_entry_size_mismatch"));
        }
        total_bytes = total_bytes
            .checked_add(observed.size)
            .ok_or_else(|| validation_error("jre_tree_too_large"))?;
        if total_bytes > MAX_JRE_TOTAL_BYTES {
            return Err(validation_error("jre_tree_too_large"));
        }
        let _manifest_relative_path = &entry.relative;

        if matches!(&mode, JreValidationMode::Full) {
            let mut file = File::open(path)
                .map_err(|error| CoreError::io("open JRE manifest entry", error))?;
            let before = stable_open_file_metadata(&file)?;
            if &before != observed {
                return Err(validation_error("jre_changed_during_validation"));
            }
            if sha256_reader(&mut file)? != entry.sha256 {
                return Err(validation_error("manifest_entry_hash_mismatch"));
            }
            let after = stable_open_file_metadata(&file)?;
            if after != before {
                return Err(validation_error("jre_changed_during_validation"));
            }
        }
    }

    let fingerprint = actual.fingerprint();
    match mode {
        JreValidationMode::Full => {
            let final_fingerprint = enumerate_plain_jre(jre_root, target_os)?.fingerprint();
            if final_fingerprint != fingerprint {
                return Err(validation_error("jre_changed_during_validation"));
            }
            Ok((final_fingerprint, total_bytes))
        }
        JreValidationMode::Fast(expected) => {
            if &fingerprint != expected {
                return Err(validation_error("jre_metadata_fingerprint_mismatch"));
            }
            Ok((fingerprint, 0))
        }
    }
}

struct EnumeratedJre {
    files: BTreeMap<String, PathBuf>,
    metadata: BTreeMap<String, StableMetadata>,
}

impl EnumeratedJre {
    fn fingerprint(&self) -> JreMetadataFingerprint {
        JreMetadataFingerprint::from_records(
            self.metadata
                .iter()
                .map(|(relative, metadata)| (relative.as_str(), metadata)),
        )
    }
}

fn enumerate_plain_jre(root: &Path, target_os: &str) -> CoreResult<EnumeratedJre> {
    let mut stack = vec![root.to_owned()];
    let mut files = BTreeMap::new();
    let mut metadata_records = BTreeMap::new();
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| CoreError::io("inspect JRE root", error))?;
    if metadata_is_link_or_reparse(&root_metadata)
        || !runtime_entry_is_secure(root, &root_metadata)
            .map_err(|_| validation_error("runtime_entry_insecure"))?
    {
        return Err(validation_error("runtime_entry_insecure"));
    }
    let mut observed_entries = 0usize;
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| CoreError::io("enumerate JRE directory", error))?
        {
            let entry = entry.map_err(|error| CoreError::io("read JRE directory entry", error))?;
            observed_entries = observed_entries
                .checked_add(1)
                .ok_or_else(|| validation_error("jre_tree_too_large"))?;
            if observed_entries > MAX_JRE_FILES * 2 {
                return Err(validation_error("jre_tree_too_large"));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| CoreError::io("inspect JRE tree entry", error))?;
            if metadata_is_link_or_reparse(&metadata) {
                return Err(validation_error("artifact_reparse_rejected"));
            }
            if !runtime_entry_is_secure(&path, &metadata)
                .map_err(|_| validation_error("runtime_entry_insecure"))?
            {
                return Err(validation_error("runtime_entry_insecure"));
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| validation_error("artifact_path_invalid"))?;
                let normalized = manifest_relative_from_path(relative)?;
                let key = manifest_key(&normalized, target_os);
                let metadata_key = format!("f:{key}");
                if metadata_records
                    .insert(metadata_key, stable_file_metadata(&path, &metadata)?)
                    .is_some()
                {
                    return Err(validation_error("manifest_duplicate_path"));
                }
                if files.insert(key, path).is_some() {
                    return Err(validation_error("manifest_duplicate_path"));
                }
                if files.len() > MAX_JRE_FILES {
                    return Err(validation_error("jre_tree_too_large"));
                }
            } else {
                return Err(validation_error("artifact_path_invalid"));
            }
        }
    }
    Ok(EnumeratedJre {
        files,
        metadata: metadata_records,
    })
}

#[cfg(unix)]
fn stable_file_metadata(path: &Path, _metadata: &fs::Metadata) -> CoreResult<StableMetadata> {
    let file = File::open(path)
        .map_err(|error| CoreError::io("open JRE entry for file identity", error))?;
    stable_open_file_metadata(&file)
}

#[cfg(unix)]
fn stable_open_file_metadata(file: &File) -> CoreResult<StableMetadata> {
    let metadata = file
        .metadata()
        .map_err(|error| CoreError::io("read JRE file identity", error))?;
    Ok(StableMetadata {
        kind: 2,
        identity_primary: metadata.dev(),
        identity_secondary: metadata.ino(),
        size: metadata.len(),
        modified_primary: metadata.mtime() as u64,
        modified_secondary: metadata.mtime_nsec() as u64,
    })
}

#[cfg(windows)]
fn stable_file_metadata(path: &Path, _metadata: &fs::Metadata) -> CoreResult<StableMetadata> {
    let file = File::open(path)
        .map_err(|error| CoreError::io("open JRE entry for file identity", error))?;
    stable_open_file_metadata(&file)
}

#[cfg(windows)]
fn stable_open_file_metadata(file: &File) -> CoreResult<StableMetadata> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the File owns a valid handle and `information` is writable for the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(CoreError::io(
            "read JRE file identity",
            std::io::Error::last_os_error(),
        ));
    }
    let identity =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    let size = (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow);
    let modified = (u64::from(information.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(information.ftLastWriteTime.dwLowDateTime);
    Ok(StableMetadata {
        kind: 2,
        identity_primary: information.dwVolumeSerialNumber.into(),
        identity_secondary: identity,
        size,
        modified_primary: modified,
        modified_secondary: 0,
    })
}

fn resolve_relative_file(root: &Path, relative: &str, code: &'static str) -> CoreResult<PathBuf> {
    let normalized = validate_relative_path_text(relative)?;
    let joined = root.join(Path::new(&normalized));
    reject_linked_path(&joined).map_err(|_| validation_error("artifact_reparse_rejected"))?;
    let canonical = fs::canonicalize(&joined).map_err(|_| validation_error(code))?;
    if !canonical.starts_with(root) {
        return Err(validation_error(code));
    }
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| validation_error(code))?;
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
        return Err(validation_error(code));
    }
    if !secure_runtime_path(root, &canonical)
        .map_err(|_| validation_error("runtime_entry_insecure"))?
    {
        return Err(validation_error("runtime_entry_insecure"));
    }
    Ok(canonical)
}

fn resolve_relative_directory(root: &Path, relative: &str) -> CoreResult<PathBuf> {
    let normalized = validate_relative_path_text(relative)?;
    let joined = root.join(Path::new(&normalized));
    reject_linked_path(&joined).map_err(|_| validation_error("artifact_reparse_rejected"))?;
    let canonical =
        fs::canonicalize(joined).map_err(|_| validation_error("artifact_path_invalid"))?;
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|_| validation_error("artifact_path_invalid"))?;
    if !canonical.starts_with(root) || !metadata.is_dir() || metadata_is_link_or_reparse(&metadata)
    {
        return Err(validation_error("artifact_path_invalid"));
    }
    if !secure_runtime_path(root, &canonical)
        .map_err(|_| validation_error("runtime_entry_insecure"))?
    {
        return Err(validation_error("runtime_entry_insecure"));
    }
    Ok(canonical)
}

fn validate_relative_path_text(value: &str) -> CoreResult<String> {
    validate_text(value, 1, 512)?;
    if value.contains('\\') || value.contains(':') {
        return Err(validation_error("artifact_path_invalid"));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(validation_error("artifact_path_invalid"));
    }
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| validation_error("artifact_path_invalid"))
        })
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(components.join("/"))
}

fn manifest_relative_from_path(path: &Path) -> CoreResult<String> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .ok_or_else(|| validation_error("artifact_path_invalid")),
            _ => Err(validation_error("artifact_path_invalid")),
        })
        .collect::<CoreResult<Vec<_>>>()?;
    validate_relative_path_text(&components.join("/"))
}

fn manifest_key(relative: &str, target_os: &str) -> String {
    if target_os == "windows" {
        relative.to_ascii_lowercase()
    } else {
        relative.to_owned()
    }
}

fn normalize_sha256(value: &str) -> CoreResult<String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(validation_error("sha256_invalid"));
    }
    Ok(value.to_ascii_lowercase())
}

fn read_bounded_file(path: &Path, maximum: u64) -> CoreResult<Vec<u8>> {
    let file = File::open(path).map_err(|error| CoreError::io("open bounded file", error))?;
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CoreError::io("read bounded file", error))?;
    if bytes.len() as u64 > maximum {
        let code = if maximum == MAX_DESCRIPTOR_BYTES as u64 {
            "descriptor_too_large"
        } else {
            "bounded_file_too_large"
        };
        return Err(validation_error(code));
    }
    Ok(bytes)
}

fn sha256_reader(file: &mut File) -> CoreResult<String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| CoreError::io("rewind file for hashing", error))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| CoreError::io("hash runtime artifact", error))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_unix_ms() -> CoreResult<i64> {
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

use std::collections::VecDeque;

use sha2::{Digest, Sha256};

use crate::store::RuntimeRecord;

use super::ValidatedRuntime;

pub const MAX_RUNTIME_PREFLIGHT_CACHE_ENTRIES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePreflightMode {
    FullValidation,
    FastMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimePreflightDiagnostics {
    mode: Option<RuntimePreflightMode>,
    content_bytes_hashed: u64,
    jre_content_bytes_hashed: u64,
    cache_entries: usize,
}

impl RuntimePreflightDiagnostics {
    pub fn mode(self) -> Option<RuntimePreflightMode> {
        self.mode
    }

    pub fn content_bytes_hashed(self) -> u64 {
        self.content_bytes_hashed
    }

    pub fn jre_content_bytes_hashed(self) -> u64 {
        self.jre_content_bytes_hashed
    }

    pub fn cache_entries(self) -> usize {
        self.cache_entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JreMetadataFingerprint([u8; 32]);

impl JreMetadataFingerprint {
    pub(crate) fn from_records<'a>(
        records: impl IntoIterator<Item = (&'a str, &'a StableMetadata)>,
    ) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"zeus-hso-jre-metadata-v1\0");
        for (relative, metadata) in records {
            let relative = relative.as_bytes();
            digest.update((relative.len() as u64).to_le_bytes());
            digest.update(relative);
            digest.update([metadata.kind]);
            digest.update(metadata.identity_primary.to_le_bytes());
            digest.update(metadata.identity_secondary.to_le_bytes());
            digest.update(metadata.size.to_le_bytes());
            digest.update(metadata.modified_primary.to_le_bytes());
            digest.update(metadata.modified_secondary.to_le_bytes());
        }
        Self(digest.finalize().into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StableMetadata {
    pub(crate) kind: u8,
    pub(crate) identity_primary: u64,
    pub(crate) identity_secondary: u64,
    pub(crate) size: u64,
    pub(crate) modified_primary: u64,
    pub(crate) modified_secondary: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimePreflightKey {
    runtime_id: String,
    descriptor_sha256: String,
}

impl RuntimePreflightKey {
    pub(crate) fn from_record(record: &RuntimeRecord) -> Self {
        Self {
            runtime_id: record.runtime_id.clone(),
            descriptor_sha256: record.descriptor_sha256.clone(),
        }
    }
}

struct RuntimePreflightCacheEntry {
    key: RuntimePreflightKey,
    fingerprint: JreMetadataFingerprint,
}

pub(crate) struct RuntimePreflightCache {
    entries: VecDeque<RuntimePreflightCacheEntry>,
    last_mode: Option<RuntimePreflightMode>,
    last_content_bytes_hashed: u64,
    last_jre_content_bytes_hashed: u64,
}

impl RuntimePreflightCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_RUNTIME_PREFLIGHT_CACHE_ENTRIES),
            last_mode: None,
            last_content_bytes_hashed: 0,
            last_jre_content_bytes_hashed: 0,
        }
    }

    pub(crate) fn lookup(&mut self, key: &RuntimePreflightKey) -> Option<JreMetadataFingerprint> {
        let position = self.entries.iter().position(|entry| entry.key == *key)?;
        let entry = self.entries.remove(position)?;
        let fingerprint = entry.fingerprint.clone();
        self.entries.push_back(entry);
        Some(fingerprint)
    }

    pub(crate) fn record_success(
        &mut self,
        key: RuntimePreflightKey,
        fingerprint: JreMetadataFingerprint,
        mode: RuntimePreflightMode,
        content_bytes_hashed: u64,
        jre_content_bytes_hashed: u64,
    ) {
        self.invalidate(&key);
        if self.entries.len() == MAX_RUNTIME_PREFLIGHT_CACHE_ENTRIES {
            self.entries.pop_front();
        }
        self.entries
            .push_back(RuntimePreflightCacheEntry { key, fingerprint });
        self.last_mode = Some(mode);
        self.last_content_bytes_hashed = content_bytes_hashed;
        self.last_jre_content_bytes_hashed = jre_content_bytes_hashed;
    }

    pub(crate) fn invalidate(&mut self, key: &RuntimePreflightKey) {
        self.entries.retain(|entry| entry.key != *key);
    }

    pub(crate) fn diagnostics(&self) -> RuntimePreflightDiagnostics {
        RuntimePreflightDiagnostics {
            mode: self.last_mode,
            content_bytes_hashed: self.last_content_bytes_hashed,
            jre_content_bytes_hashed: self.last_jre_content_bytes_hashed,
            cache_entries: self.entries.len(),
        }
    }
}

pub(crate) fn runtime_matches_registry_record(
    record: &RuntimeRecord,
    revalidated: &ValidatedRuntime,
) -> bool {
    record.runtime_id == revalidated.runtime_id
        && record.descriptor_sha256 == revalidated.descriptor_sha256
        && record.descriptor_path == revalidated.descriptor_path
        && record.runtime_root == revalidated.runtime_root
        && record.target_os == revalidated.target_os
        && record.target_arch == revalidated.target_arch
        && record.java_path == revalidated.java_path
        && record.java_vendor == revalidated.java_vendor
        && record.java_version == revalidated.java_version
        && record.jre_manifest_sha256 == revalidated.jre_manifest_sha256
        && record.microemulator_path == revalidated.microemulator_path
        && record.microemulator_version == revalidated.microemulator_version
        && record.microemulator_sha256 == revalidated.microemulator_sha256
        && record.game_path == revalidated.game_path
        && record.game_bundle == revalidated.game_bundle
        && record.game_sha256 == revalidated.game_sha256
        && record.capability_state == revalidated.capability_state
        && record.validation_reason == revalidated.validation_reason
}

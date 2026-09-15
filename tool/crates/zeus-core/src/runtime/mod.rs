//! Strict, explicit validation for immutable runtime descriptors.

mod descriptor;
mod preflight;
mod validator;

pub(crate) use preflight::{
    JreMetadataFingerprint, RuntimePreflightCache, RuntimePreflightKey, StableMetadata,
    runtime_matches_registry_record,
};
pub use preflight::{
    MAX_RUNTIME_PREFLIGHT_CACHE_ENTRIES, RuntimePreflightDiagnostics, RuntimePreflightMode,
};
pub use validator::{
    CapabilityState, MAX_DESCRIPTOR_BYTES, ValidatedRuntime, validate_runtime_descriptor,
};
pub(crate) use validator::{
    RuntimePreflightValidation, ValidatedLaunchDefaults, validate_runtime_descriptor_fast,
    validate_runtime_descriptor_full,
};

pub(crate) fn validation_error(code: &'static str) -> crate::CoreError {
    crate::CoreError::RuntimeValidation { code }
}

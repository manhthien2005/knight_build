use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::data_root::reject_linked_path;
use crate::profile::ProfileRecord;
use crate::rms;
use crate::runtime::{
    RuntimePreflightKey, RuntimePreflightMode, ValidatedLaunchDefaults,
    runtime_matches_registry_record, validate_runtime_descriptor_fast,
    validate_runtime_descriptor_full,
};
use crate::store::RuntimeRecord;
use crate::{CoreError, CoreResult, CoreState};

/// The child environment is fixed, not inherited. On Windows it carries one extra entry because
/// WinSock loads its name-resolution providers through `%SystemRoot%`: without it the client resolves
/// no host and never reaches a server, while every file path it needs still works.
#[cfg(windows)]
pub(crate) const ENVIRONMENT_VARIABLE_COUNT: usize = 4;
#[cfg(not(windows))]
pub(crate) const ENVIRONMENT_VARIABLE_COUNT: usize = 3;

/// Fallback used only when the parent process has no `SystemRoot`; validation still rejects it if it
/// is not a real directory, so a wrong value fails closed rather than launching a client that cannot
/// resolve a host.
#[cfg(windows)]
const SYSTEM_ROOT_FALLBACK: &str = r"C:\Windows";

/// Resolves the machine's `SystemRoot` in the same canonical form as every other snapshot path.
#[cfg(windows)]
pub(crate) fn system_root_directory() -> PathBuf {
    // Deliberately NOT canonicalized. `fs::canonicalize` returns the `\\?\C:\WINDOWS` extended form,
    // and that prefix is not a valid `%SystemRoot%` for the loader that resolves WinSock's providers.
    // The plain absolute path is what a normal child inherits, so it is what is passed here.
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .unwrap_or_else(|| PathBuf::from(SYSTEM_ROOT_FALLBACK))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSize {
    width: u32,
    height: u32,
}

impl ScreenSize {
    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapSettings {
    initial_mib: u32,
    maximum_mib: u32,
}

impl HeapSettings {
    pub fn initial_mib(self) -> u32 {
        self.initial_mib
    }

    pub fn maximum_mib(self) -> u32 {
        self.maximum_mib
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JvmFlag {
    UseSerialGc,
    DisablePerfData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmsMode {
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchEnvironmentKey {
    Temp,
    Tmp,
    TmpDir,
    /// Windows only. Carries `%SystemRoot%` so WinSock can load its name-resolution providers; it is
    /// the one entry whose value is not the profile temp directory.
    #[cfg(windows)]
    SystemRoot,
}

impl LaunchEnvironmentKey {
    /// Whether this key's value must be the profile temp directory.
    ///
    /// Every entry except `SystemRoot` points at the profile's own temp directory, so the sealed-path
    /// checks can assert that without special-casing an index.
    pub(crate) fn is_profile_temp_directory(self) -> bool {
        match self {
            Self::Temp | Self::Tmp | Self::TmpDir => true,
            #[cfg(windows)]
            Self::SystemRoot => false,
        }
    }
}

/// The exact key order every launch environment must carry, in declaration order.
pub(crate) const ENVIRONMENT_KEY_ORDER: [LaunchEnvironmentKey; ENVIRONMENT_VARIABLE_COUNT] = [
    LaunchEnvironmentKey::Temp,
    LaunchEnvironmentKey::Tmp,
    LaunchEnvironmentKey::TmpDir,
    #[cfg(windows)]
    LaunchEnvironmentKey::SystemRoot,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnvironmentVariable {
    key: LaunchEnvironmentKey,
    value: PathBuf,
}

#[cfg(all(test, windows))]
pub(crate) fn fixed_test_environment(
    temp_directory: &Path,
) -> [LaunchEnvironmentVariable; ENVIRONMENT_VARIABLE_COUNT] {
    [
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::Temp,
            value: temp_directory.to_owned(),
        },
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::Tmp,
            value: temp_directory.to_owned(),
        },
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::TmpDir,
            value: temp_directory.to_owned(),
        },
        // The real machine value, so a test spec seals and validates exactly what production does.
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::SystemRoot,
            value: system_root_directory(),
        },
    ]
}

impl LaunchEnvironmentVariable {
    pub fn key(&self) -> LaunchEnvironmentKey {
        self.key
    }

    pub fn value(&self) -> &Path {
        &self.value
    }

    #[cfg(windows)]
    pub(crate) fn with_value_for_process(&self, value: PathBuf) -> Self {
        Self {
            key: self.key,
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSnapshot {
    session_id: Uuid,
    profile_id: Uuid,
    profile_revision: i64,
    runtime_id: String,
    descriptor_sha256: String,
    runtime_root: PathBuf,
    java_executable: PathBuf,
    microemulator_jar: PathBuf,
    game_jar: PathBuf,
    main_class: String,
    midlet_class: String,
    screen_size: ScreenSize,
    heap: HeapSettings,
    jvm_flags: [JvmFlag; 2],
    rms_mode: RmsMode,
    quiet: bool,
    quit_on_midlet_destroy: bool,
    profile_root: PathBuf,
    working_directory: PathBuf,
    microemu_home: PathBuf,
    temp_directory: PathBuf,
    environment: [LaunchEnvironmentVariable; ENVIRONMENT_VARIABLE_COUNT],
}

impl LaunchSnapshot {
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn profile_revision(&self) -> i64 {
        self.profile_revision
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn descriptor_sha256(&self) -> &str {
        &self.descriptor_sha256
    }

    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    pub fn java_executable(&self) -> &Path {
        &self.java_executable
    }

    pub fn microemulator_jar(&self) -> &Path {
        &self.microemulator_jar
    }

    pub fn game_jar(&self) -> &Path {
        &self.game_jar
    }

    pub fn main_class(&self) -> &str {
        &self.main_class
    }

    pub fn midlet_class(&self) -> &str {
        &self.midlet_class
    }

    pub fn screen_size(&self) -> ScreenSize {
        self.screen_size
    }

    pub fn heap(&self) -> HeapSettings {
        self.heap
    }

    pub fn jvm_flags(&self) -> &[JvmFlag] {
        &self.jvm_flags
    }

    pub fn rms_mode(&self) -> RmsMode {
        self.rms_mode
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    pub fn quit_on_midlet_destroy(&self) -> bool {
        self.quit_on_midlet_destroy
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn microemu_home(&self) -> &Path {
        &self.microemu_home
    }

    pub fn temp_directory(&self) -> &Path {
        &self.temp_directory
    }

    /// `%SystemRoot%` as this snapshot resolved it, so the spec seals the same value it will pass.
    #[cfg(windows)]
    pub(crate) fn system_root_directory(&self) -> &Path {
        self.environment
            .iter()
            .find(|variable| !variable.key().is_profile_temp_directory())
            .map_or_else(
                || Path::new(SYSTEM_ROOT_FALLBACK),
                |variable| variable.value(),
            )
    }

    pub fn environment(&self) -> &[LaunchEnvironmentVariable] {
        &self.environment
    }
}

impl CoreState {
    pub fn prepare_launch_snapshot(
        &self,
        profile_id: &str,
        expected_revision: i64,
    ) -> CoreResult<LaunchSnapshot> {
        if !(1..i64::MAX).contains(&expected_revision) {
            return Err(CoreError::InvalidRevision);
        }
        let profile = self.inspect_profile(profile_id)?;
        if profile.revision != expected_revision {
            return Err(CoreError::RevisionConflict {
                profile_id: profile.profile_id,
                expected: expected_revision,
                actual: profile.revision,
            });
        }
        if profile.archived_at_unix_ms.is_some() {
            return Err(CoreError::ProfileArchived {
                profile_id: profile.profile_id,
            });
        }
        let runtime = self.inspect_runtime(&profile.runtime_id)?;
        let preflight_key = RuntimePreflightKey::from_record(&runtime);
        let cached_fingerprint = self
            .runtime_preflight_cache()
            .borrow_mut()
            .lookup(&preflight_key);
        let mode = if cached_fingerprint.is_some() {
            RuntimePreflightMode::FastMetadata
        } else {
            RuntimePreflightMode::FullValidation
        };
        let revalidated = match cached_fingerprint.as_ref() {
            Some(fingerprint) => {
                validate_runtime_descriptor_fast(&runtime.descriptor_path, fingerprint)
            }
            None => validate_runtime_descriptor_full(&runtime.descriptor_path),
        };
        let revalidated = match revalidated {
            Ok(revalidated) => revalidated,
            Err(error) => {
                self.runtime_preflight_cache()
                    .borrow_mut()
                    .invalidate(&preflight_key);
                return Err(error);
            }
        };
        if !runtime_matches_registry_record(&runtime, &revalidated.runtime) {
            self.runtime_preflight_cache()
                .borrow_mut()
                .invalidate(&preflight_key);
            return Err(CoreError::RuntimeRegistryMismatch {
                runtime_id: runtime.runtime_id,
            });
        }
        self.runtime_preflight_cache().borrow_mut().record_success(
            preflight_key,
            revalidated.jre_metadata_fingerprint.clone(),
            mode,
            revalidated.content_bytes_hashed,
            revalidated.jre_content_bytes_hashed,
        );
        let profile_uuid =
            Uuid::parse_str(&profile.profile_id).map_err(|_| CoreError::InvalidProfileId)?;

        let profile_root = self.data_root().verified_profile_directory(profile_id)?;
        reject_linked_path(&profile_root)?;
        let profile_root = fs::canonicalize(profile_root)
            .map_err(|error| CoreError::io("canonicalize launch profile root", error))?;
        let microemu_home = self
            .data_root()
            .ensure_private_profile_child_directory(profile_id, rms::MICROEMU_HOME_DIRECTORY)?;
        let temp_directory = self
            .data_root()
            .ensure_private_profile_child_directory(profile_id, "temp")?;

        Ok(build_snapshot(
            profile,
            runtime,
            revalidated.launch_defaults,
            profile_uuid,
            profile_root,
            microemu_home,
            temp_directory,
        ))
    }
}

fn build_snapshot(
    profile: ProfileRecord,
    runtime: RuntimeRecord,
    launch: ValidatedLaunchDefaults,
    profile_id: Uuid,
    profile_root: PathBuf,
    microemu_home: PathBuf,
    temp_directory: PathBuf,
) -> LaunchSnapshot {
    let environment = [
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::Temp,
            value: temp_directory.clone(),
        },
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::Tmp,
            value: temp_directory.clone(),
        },
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::TmpDir,
            value: temp_directory.clone(),
        },
        // Not a profile path: WinSock resolves hosts through `%SystemRoot%`, so a child launched
        // without it connects to nothing.
        #[cfg(windows)]
        LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::SystemRoot,
            value: system_root_directory(),
        },
    ];
    LaunchSnapshot {
        session_id: Uuid::new_v4(),
        profile_id,
        profile_revision: profile.revision,
        runtime_id: runtime.runtime_id,
        descriptor_sha256: runtime.descriptor_sha256,
        runtime_root: runtime.runtime_root,
        java_executable: runtime.java_path,
        microemulator_jar: runtime.microemulator_path,
        game_jar: runtime.game_path,
        main_class: launch.main_class,
        midlet_class: launch.midlet_class,
        screen_size: ScreenSize {
            width: launch.screen_width,
            height: launch.screen_height,
        },
        heap: HeapSettings {
            initial_mib: launch.heap_initial_mib,
            maximum_mib: launch.heap_max_mib,
        },
        jvm_flags: [JvmFlag::UseSerialGc, JvmFlag::DisablePerfData],
        rms_mode: RmsMode::File,
        quiet: launch.quiet,
        quit_on_midlet_destroy: launch.quit_on_midlet_destroy,
        working_directory: profile_root.clone(),
        profile_root,
        microemu_home,
        temp_directory,
        environment,
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::fs;

    use super::{LaunchEnvironmentKey, LaunchEnvironmentVariable};
    use crate::process_launch_spec::windows::local_drive_invocation_path;

    #[test]
    fn process_value_remap_preserves_environment_key_and_canonical_identity() {
        let canonical = fs::canonicalize(std::env::temp_dir()).unwrap();
        let invocation = local_drive_invocation_path(&canonical).unwrap();
        let original = LaunchEnvironmentVariable {
            key: LaunchEnvironmentKey::Temp,
            value: canonical.clone(),
        };

        let remapped = original.with_value_for_process(invocation);

        assert_eq!(remapped.key(), original.key());
        assert_ne!(remapped.value(), original.value());
        assert_eq!(fs::canonicalize(remapped.value()).unwrap(), canonical);
    }
}

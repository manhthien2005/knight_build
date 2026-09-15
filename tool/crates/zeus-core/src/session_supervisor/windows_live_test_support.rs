use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::{Uuid, Version};
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, HWND, LPARAM};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, GetProcessTimes,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
    WM_NULL,
};

use crate::CoreState;
use crate::data_root::metadata_is_link_or_reparse;
use crate::process_adapter::ProcessBirthId;
use crate::process_adapter::test_support::{
    ProcessObservation, open_identity_checked_process_for_metrics,
};
use crate::runtime::CapabilityState;

use super::SessionSupervisor;

pub(super) const LIVE_APPROVAL_TOKEN: &str = "windows-live-runtime-bridge-v1-approved";
pub(crate) const LIVE_RUNTIME_ID: &str = "windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402";
pub(super) const LIVE_DESCRIPTOR_SHA256: &str =
    "3e39b998bac686d8d61b8fb2da1b7f207efac463097d643341d1990525c7eac9";
pub(super) const LIVE_TEST_BASE: &str = "zeus-hso-live-runtime-bridge-v1";
pub(super) const START_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const WINDOW_READY_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const STOP_DEADLINE: Duration = Duration::from_secs(10);
pub(super) const WINDOW_POLL_CADENCE: Duration = Duration::from_millis(50);
pub(super) const WINDOW_RESPONSE_TIMEOUT_MS: u32 = 1_000;
pub(super) const STABILIZATION_DURATION: Duration = Duration::from_secs(15);
pub(super) const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const SAMPLE_COUNT: usize = 13;
pub(super) const CONCURRENT_SESSION_COUNT: usize = 4;
pub(super) const REGISTERED_PROFILE_COUNT: usize = 5;
pub(super) const PER_SESSION_MEMORY_LIMIT: u64 = 256 * 1024 * 1024;
pub(super) const AGGREGATE_MEMORY_LIMIT: u64 = 768 * 1024 * 1024;
pub(super) const PER_SESSION_HANDLE_LIMIT: u32 = 1_200;
pub(super) const AGGREGATE_HANDLE_LIMIT: u32 = 4_800;
pub(super) const AGGREGATE_CPU_X100_LIMIT: u64 = 10_000;
pub(super) const AGGREGATE_GROWTH_LIMIT: i64 = 128 * 1024 * 1024;

const LIVE_GAME_SHA256: &str = "6608bb0c77f03749e46165f711e9566dca4e172ce232256497b35faafe74c259";

static LIVE_TEST_BASE_MUTATION: Mutex<()> = Mutex::new(());

fn lock_live_test_base_mutation() -> MutexGuard<'static, ()> {
    LIVE_TEST_BASE_MUTATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

const REQUIRED_RUNTIME_FILES: &[&str] = &[
    "runtime-descriptor.json",
    "jre-files.sha256",
    "jre/bin/javaw.exe",
    "microemulator/microemulator.jar",
    "game/KnightOnline_402.jar",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiveGateFailure {
    stage: &'static str,
    index: Option<usize>,
    limit: Option<u64>,
    win32_code: Option<u32>,
}

impl LiveGateFailure {
    pub(crate) fn stage(stage: &'static str) -> Self {
        Self {
            stage,
            index: None,
            limit: None,
            win32_code: None,
        }
    }

    fn io(stage: &'static str, error: &io::Error) -> Self {
        Self {
            stage,
            index: None,
            limit: None,
            win32_code: error
                .raw_os_error()
                .and_then(|code| u32::try_from(code).ok()),
        }
    }

    fn last_win32(stage: &'static str) -> Self {
        Self::io(stage, &io::Error::last_os_error())
    }

    pub(super) fn stage_name(self) -> &'static str {
        self.stage
    }
}

impl fmt::Display for LiveGateFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LiveGateFailure(stage={}", self.stage)?;
        if let Some(index) = self.index {
            write!(formatter, ", index={index}")?;
        }
        if let Some(limit) = self.limit {
            write!(formatter, ", limit={limit}")?;
        }
        if let Some(code) = self.win32_code {
            write!(formatter, ", win32_code={code}")?;
        }
        formatter.write_str(")")
    }
}

impl std::error::Error for LiveGateFailure {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct ProcessResourceSample {
    pub(super) working_set_bytes: u64,
    pub(super) private_bytes: u64,
    pub(super) handle_count: u32,
    pub(super) cpu_time_100ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct SessionPerformanceEvidenceV1 {
    pub(super) index: u32,
    pub(super) max_working_set_bytes: u64,
    pub(super) final_working_set_bytes: u64,
    pub(super) max_private_bytes: u64,
    pub(super) final_private_bytes: u64,
    pub(super) max_handle_count: u32,
    pub(super) cpu_percent_one_core_x100: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct AggregatePerformanceEvidenceV1 {
    pub(super) first_working_set_bytes: u64,
    pub(super) max_working_set_bytes: u64,
    pub(super) final_working_set_bytes: u64,
    pub(super) working_set_growth_bytes: i64,
    pub(super) first_private_bytes: u64,
    pub(super) max_private_bytes: u64,
    pub(super) final_private_bytes: u64,
    pub(super) private_growth_bytes: i64,
    pub(super) max_handle_count: u32,
    pub(super) cpu_percent_one_core_x100: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct LivePerformanceEvidenceV1 {
    pub(super) schema_version: u32,
    pub(super) runtime_id: &'static str,
    pub(super) concurrent_sessions: u32,
    pub(super) stabilization_seconds: u64,
    pub(super) observation_seconds: u64,
    pub(super) sample_interval_seconds: u64,
    pub(super) samples_per_session: u32,
    pub(super) representative_of_1gib_target: bool,
    pub(super) capacity_rejection_confirmed: bool,
    pub(super) all_windows_responsive: bool,
    pub(super) cleanup_confirmed: bool,
    pub(super) start_to_window_ms: [u64; CONCURRENT_SESSION_COUNT],
    pub(super) per_session: [SessionPerformanceEvidenceV1; CONCURRENT_SESSION_COUNT],
    pub(super) aggregate: AggregatePerformanceEvidenceV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReadyWindow {
    hwnd: HWND,
}

pub(crate) struct LiveTestDirectory {
    canonical_temp_root: PathBuf,
    canonical_base: PathBuf,
    canonical_path: PathBuf,
    leaf: String,
    cleanup_confirmed: bool,
}

impl LiveTestDirectory {
    pub(crate) fn create() -> Result<Self, LiveGateFailure> {
        let _base_mutation = lock_live_test_base_mutation();
        let canonical_temp_root = fs::canonicalize(std::env::temp_dir())
            .map_err(|error| LiveGateFailure::io("live_temp_root", &error))?;
        let base = canonical_temp_root.join(LIVE_TEST_BASE);
        fs::create_dir_all(&base).map_err(|error| LiveGateFailure::io("live_test_base", &error))?;
        let canonical_base = fs::canonicalize(&base)
            .map_err(|error| LiveGateFailure::io("live_test_base", &error))?;
        if canonical_base.parent() != Some(canonical_temp_root.as_path())
            || canonical_base.file_name() != Some(OsStr::new(LIVE_TEST_BASE))
        {
            return Err(LiveGateFailure::stage("live_test_base_identity"));
        }

        let leaf = Uuid::new_v4().hyphenated().to_string();
        let path = canonical_base.join(&leaf);
        let core =
            CoreState::open_at(&path).map_err(|_| LiveGateFailure::stage("live_test_data_root"))?;
        drop(core);
        let canonical_path = fs::canonicalize(&path)
            .map_err(|error| LiveGateFailure::io("live_test_path", &error))?;
        let metadata = fs::symlink_metadata(&canonical_path)
            .map_err(|error| LiveGateFailure::io("live_test_path", &error))?;
        if canonical_path.parent() != Some(canonical_base.as_path())
            || canonical_path.file_name() != Some(OsStr::new(&leaf))
            || !metadata.is_dir()
            || metadata_is_link_or_reparse(&metadata)
        {
            return Err(LiveGateFailure::stage("live_test_path_identity"));
        }

        Ok(Self {
            canonical_temp_root,
            canonical_base,
            canonical_path,
            leaf,
            cleanup_confirmed: false,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.canonical_path
    }

    pub(super) fn base(&self) -> &Path {
        &self.canonical_base
    }

    pub(super) fn leaf(&self) -> &str {
        &self.leaf
    }

    pub(super) fn mark_cleanup_confirmed(&mut self) {
        self.cleanup_confirmed = true;
    }

    pub(crate) fn remove_after_cleanup_confirmation(mut self) -> Result<(), LiveGateFailure> {
        self.cleanup_confirmed = true;
        let result = self.cleanup();
        self.cleanup_confirmed = false;
        result
    }

    fn cleanup(&self) -> Result<(), LiveGateFailure> {
        let _base_mutation = lock_live_test_base_mutation();
        let canonical_temp_root = fs::canonicalize(std::env::temp_dir())
            .map_err(|error| LiveGateFailure::io("cleanup_temp_root", &error))?;
        if canonical_temp_root != self.canonical_temp_root {
            return Err(LiveGateFailure::stage("cleanup_temp_identity"));
        }
        let resolved = fs::canonicalize(&self.canonical_path)
            .map_err(|error| LiveGateFailure::io("cleanup_path", &error))?;
        let metadata = fs::symlink_metadata(&self.canonical_path)
            .map_err(|error| LiveGateFailure::io("cleanup_path", &error))?;
        if resolved != self.canonical_path
            || resolved.parent() != Some(self.canonical_base.as_path())
            || resolved.file_name() != Some(OsStr::new(&self.leaf))
            || !metadata.is_dir()
            || metadata_is_link_or_reparse(&metadata)
        {
            return Err(LiveGateFailure::stage("cleanup_path_identity"));
        }
        fs::remove_dir_all(&resolved)
            .map_err(|error| LiveGateFailure::io("cleanup_remove", &error))?;
        match fs::remove_dir(&self.canonical_base) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(LiveGateFailure::io("cleanup_base", &error)),
        }
        Ok(())
    }
}

pub(super) struct LiveRuntimeFixture {
    pub(super) directory: LiveTestDirectory,
    pub(super) supervisor: SessionSupervisor,
    pub(super) profile_id: String,
    pub(super) profile_revision: i64,
    pub(super) profile_root: PathBuf,
}

pub(super) struct LiveChildRuntimeFixture {
    pub(super) supervisor: SessionSupervisor,
    pub(super) profile_id: String,
    pub(super) profile_revision: i64,
    pub(super) profile_root: PathBuf,
}

pub(crate) struct LiveManagerCoreFixture {
    directory: LiveTestDirectory,
    core: CoreState,
    profile_id: String,
    profile_revision: i64,
    profile_root: PathBuf,
}

impl LiveManagerCoreFixture {
    pub(crate) fn into_parts(self) -> (LiveTestDirectory, CoreState, String, i64, PathBuf) {
        (
            self.directory,
            self.core,
            self.profile_id,
            self.profile_revision,
            self.profile_root,
        )
    }
}

pub(crate) fn prepare_live_manager_core_fixture(
    display_name: &str,
) -> Result<LiveManagerCoreFixture, LiveGateFailure> {
    let runtime_root = require_live_runtime_root();
    let directory = LiveTestDirectory::create()?;
    match prepare_live_runtime_core_at(directory.path(), &runtime_root, display_name) {
        Ok((core, profile_id, profile_revision, profile_root)) => Ok(LiveManagerCoreFixture {
            directory,
            core,
            profile_id,
            profile_revision,
            profile_root,
        }),
        Err(original) => match directory.remove_after_cleanup_confirmation() {
            Ok(()) => Err(original),
            Err(cleanup) => Err(cleanup),
        },
    }
}

pub(super) fn prepare_live_child_runtime_fixture(
    guarded_root: &Path,
    runtime_root: &Path,
    display_name: &str,
) -> Result<LiveChildRuntimeFixture, LiveGateFailure> {
    let (supervisor, profile_id, profile_revision, profile_root) =
        prepare_live_runtime_fixture_at(guarded_root, runtime_root, display_name)?;
    Ok(LiveChildRuntimeFixture {
        supervisor,
        profile_id,
        profile_revision,
        profile_root,
    })
}

#[derive(Debug)]
pub(super) struct LiveProfileFixture {
    pub(super) profile_id: String,
    pub(super) profile_revision: i64,
    pub(super) profile_root: PathBuf,
}

pub(super) struct LiveMultiProfileFixture {
    pub(super) directory: LiveTestDirectory,
    pub(super) supervisor: SessionSupervisor,
    pub(super) profiles: [LiveProfileFixture; REGISTERED_PROFILE_COUNT],
}

pub(super) fn prepare_live_multi_profile_fixture()
-> Result<LiveMultiProfileFixture, LiveGateFailure> {
    let runtime_root = require_live_runtime_root();
    let directory = LiveTestDirectory::create()?;
    match prepare_live_multi_profile_fixture_in(&directory, &runtime_root) {
        Ok((supervisor, profiles)) => Ok(LiveMultiProfileFixture {
            directory,
            supervisor,
            profiles,
        }),
        Err(original) => match directory.remove_after_cleanup_confirmation() {
            Ok(()) => Err(original),
            Err(cleanup) => Err(cleanup),
        },
    }
}

fn prepare_live_multi_profile_fixture_in(
    directory: &LiveTestDirectory,
    runtime_root: &Path,
) -> Result<
    (
        SessionSupervisor,
        [LiveProfileFixture; REGISTERED_PROFILE_COUNT],
    ),
    LiveGateFailure,
> {
    let mut core = CoreState::open_at(directory.path())
        .map_err(|_| LiveGateFailure::stage("fixture_core_open"))?;
    let descriptor = runtime_root.join("runtime-descriptor.json");
    let runtime = core
        .register_runtime_descriptor(&descriptor)
        .map_err(|_| LiveGateFailure::stage("fixture_runtime_register"))?;
    if runtime.runtime_id != LIVE_RUNTIME_ID
        || runtime.descriptor_sha256 != LIVE_DESCRIPTOR_SHA256
        || runtime.game_sha256 != LIVE_GAME_SHA256
        || runtime.capability_state != CapabilityState::NeedsValidation
        || runtime.runtime_root != runtime_root
        || runtime.descriptor_path != descriptor
    {
        return Err(LiveGateFailure::stage("fixture_runtime_identity"));
    }

    let mut profiles = Vec::with_capacity(REGISTERED_PROFILE_COUNT);
    for index in 1..=REGISTERED_PROFILE_COUNT {
        let profile = core
            .create_profile(&format!("Live Bridge {index}"), &runtime.runtime_id)
            .map_err(|_| LiveGateFailure::stage("fixture_profile_create"))?;
        let profile_uuid = Uuid::parse_str(&profile.profile_id)
            .map_err(|_| LiveGateFailure::stage("fixture_profile_identity"))?;
        if profile.revision != 1
            || profile_uuid.get_version() != Some(Version::Random)
            || profile_uuid.hyphenated().to_string() != profile.profile_id
        {
            return Err(LiveGateFailure::stage("fixture_profile_identity"));
        }
        let profile_root = core
            .profile_directory(&profile.profile_id)
            .map_err(|_| LiveGateFailure::stage("fixture_profile_root"))?;
        let profile_root = fs::canonicalize(profile_root)
            .map_err(|error| LiveGateFailure::io("fixture_profile_root", &error))?;
        let profile_metadata = fs::symlink_metadata(&profile_root)
            .map_err(|error| LiveGateFailure::io("fixture_profile_root", &error))?;
        let expected_profile_root = directory.path().join("profiles").join(&profile.profile_id);
        if profile_root != expected_profile_root
            || !profile_root.starts_with(directory.path())
            || !profile_metadata.is_dir()
            || metadata_is_link_or_reparse(&profile_metadata)
        {
            return Err(LiveGateFailure::stage("fixture_profile_root_identity"));
        }
        profiles.push(LiveProfileFixture {
            profile_id: profile.profile_id,
            profile_revision: profile.revision,
            profile_root,
        });
    }
    let profiles = profiles
        .try_into()
        .map_err(|_| LiveGateFailure::stage("fixture_profile_count"))?;
    Ok((SessionSupervisor::new(core), profiles))
}

pub(super) fn prepare_live_runtime_fixture(
    display_name: &str,
) -> Result<LiveRuntimeFixture, LiveGateFailure> {
    let runtime_root = require_live_runtime_root();
    let directory = LiveTestDirectory::create()?;
    match prepare_live_runtime_fixture_in(&directory, &runtime_root, display_name) {
        Ok((supervisor, profile_id, profile_revision, profile_root)) => Ok(LiveRuntimeFixture {
            directory,
            supervisor,
            profile_id,
            profile_revision,
            profile_root,
        }),
        Err(original) => match directory.remove_after_cleanup_confirmation() {
            Ok(()) => Err(original),
            Err(cleanup) => Err(cleanup),
        },
    }
}

fn prepare_live_runtime_fixture_in(
    directory: &LiveTestDirectory,
    runtime_root: &Path,
    display_name: &str,
) -> Result<(SessionSupervisor, String, i64, PathBuf), LiveGateFailure> {
    prepare_live_runtime_fixture_at(directory.path(), runtime_root, display_name)
}

fn prepare_live_runtime_fixture_at(
    guarded_root: &Path,
    runtime_root: &Path,
    display_name: &str,
) -> Result<(SessionSupervisor, String, i64, PathBuf), LiveGateFailure> {
    let (core, profile_id, profile_revision, profile_root) =
        prepare_live_runtime_core_at(guarded_root, runtime_root, display_name)?;
    Ok((
        SessionSupervisor::new(core),
        profile_id,
        profile_revision,
        profile_root,
    ))
}

fn prepare_live_runtime_core_at(
    guarded_root: &Path,
    runtime_root: &Path,
    display_name: &str,
) -> Result<(CoreState, String, i64, PathBuf), LiveGateFailure> {
    let mut core = CoreState::open_at(guarded_root)
        .map_err(|_| LiveGateFailure::stage("fixture_core_open"))?;
    let descriptor = runtime_root.join("runtime-descriptor.json");
    let runtime = core
        .register_runtime_descriptor(&descriptor)
        .map_err(|_| LiveGateFailure::stage("fixture_runtime_register"))?;
    if runtime.runtime_id != LIVE_RUNTIME_ID
        || runtime.descriptor_sha256 != LIVE_DESCRIPTOR_SHA256
        || runtime.game_sha256 != LIVE_GAME_SHA256
        || runtime.capability_state != CapabilityState::NeedsValidation
        || runtime.runtime_root != runtime_root
        || runtime.descriptor_path != descriptor
    {
        return Err(LiveGateFailure::stage("fixture_runtime_identity"));
    }

    let profile = core
        .create_profile(display_name, &runtime.runtime_id)
        .map_err(|_| LiveGateFailure::stage("fixture_profile_create"))?;
    let profile_uuid = Uuid::parse_str(&profile.profile_id)
        .map_err(|_| LiveGateFailure::stage("fixture_profile_identity"))?;
    if profile.revision != 1
        || profile_uuid.get_version() != Some(Version::Random)
        || profile_uuid.hyphenated().to_string() != profile.profile_id
    {
        return Err(LiveGateFailure::stage("fixture_profile_identity"));
    }
    let profile_root = core
        .profile_directory(&profile.profile_id)
        .map_err(|_| LiveGateFailure::stage("fixture_profile_root"))?;
    let profile_root = fs::canonicalize(profile_root)
        .map_err(|error| LiveGateFailure::io("fixture_profile_root", &error))?;
    let profile_metadata = fs::symlink_metadata(&profile_root)
        .map_err(|error| LiveGateFailure::io("fixture_profile_root", &error))?;
    let expected_profile_root = guarded_root.join("profiles").join(&profile.profile_id);
    if profile_root != expected_profile_root
        || !profile_root.starts_with(guarded_root)
        || !profile_metadata.is_dir()
        || metadata_is_link_or_reparse(&profile_metadata)
    {
        return Err(LiveGateFailure::stage("fixture_profile_root_identity"));
    }

    Ok((core, profile.profile_id, profile.revision, profile_root))
}

impl Drop for LiveTestDirectory {
    fn drop(&mut self) {
        if self.cleanup_confirmed {
            let _ = self.cleanup();
        }
    }
}

pub(crate) fn require_live_runtime_root() -> PathBuf {
    let approval = std::env::var_os("ZEUS_LIVE_RUNTIME_BRIDGE");
    if approval.as_deref() != Some(OsStr::new(LIVE_APPROVAL_TOKEN)) {
        panic!("{}", LiveGateFailure::stage("live_approval"));
    }
    let runtime_root = std::env::var_os("ZEUS_EXACT_RUNTIME_ROOT").map(PathBuf::from);
    require_live_runtime_root_from_inputs(approval.as_deref(), runtime_root.as_deref())
        .unwrap_or_else(|error| panic!("{error}"))
}

pub(super) fn validate_parent_crash_guarded_root(
    guarded_root: &Path,
    child_token: &str,
) -> Result<PathBuf, LiveGateFailure> {
    let token =
        Uuid::parse_str(child_token).map_err(|_| LiveGateFailure::stage("parent_child_token"))?;
    if token.get_version() != Some(Version::Random) || token.hyphenated().to_string() != child_token
    {
        return Err(LiveGateFailure::stage("parent_child_token"));
    }
    let canonical_root = fs::canonicalize(guarded_root)
        .map_err(|error| LiveGateFailure::io("parent_guarded_root", &error))?;
    let canonical_base = canonical_root
        .parent()
        .ok_or_else(|| LiveGateFailure::stage("parent_live_base"))?;
    let resolved_base = fs::canonicalize(canonical_base)
        .map_err(|error| LiveGateFailure::io("parent_live_base", &error))?;
    let base_metadata = fs::symlink_metadata(canonical_base)
        .map_err(|error| LiveGateFailure::io("parent_live_base", &error))?;
    let root_metadata = fs::symlink_metadata(&canonical_root)
        .map_err(|error| LiveGateFailure::io("parent_guarded_root", &error))?;
    if canonical_base.as_os_str().as_encoded_bytes() != resolved_base.as_os_str().as_encoded_bytes()
        || !parent_crash_guarded_root_shape_is_valid(
            guarded_root,
            &canonical_root,
            canonical_base,
            child_token,
            base_metadata.is_dir() && !metadata_is_link_or_reparse(&base_metadata),
            root_metadata.is_dir() && !metadata_is_link_or_reparse(&root_metadata),
        )
    {
        return Err(LiveGateFailure::stage("parent_guarded_root_identity"));
    }
    Ok(canonical_root)
}

fn parent_crash_guarded_root_shape_is_valid(
    supplied_root: &Path,
    canonical_root: &Path,
    canonical_base: &Path,
    child_token: &str,
    base_is_safe_directory: bool,
    root_is_safe_directory: bool,
) -> bool {
    supplied_root.as_os_str().as_encoded_bytes() == canonical_root.as_os_str().as_encoded_bytes()
        && canonical_base.file_name() == Some(OsStr::new(LIVE_TEST_BASE))
        && canonical_root.parent().is_some_and(|parent| {
            parent.as_os_str().as_encoded_bytes() == canonical_base.as_os_str().as_encoded_bytes()
        })
        && canonical_root.file_name() == Some(OsStr::new(child_token))
        && base_is_safe_directory
        && root_is_safe_directory
}

fn require_live_runtime_root_from_inputs(
    approval: Option<&OsStr>,
    runtime_root: Option<&Path>,
) -> Result<PathBuf, LiveGateFailure> {
    if approval != Some(OsStr::new(LIVE_APPROVAL_TOKEN)) {
        return Err(LiveGateFailure::stage("live_approval"));
    }
    let runtime_root = runtime_root.ok_or_else(|| LiveGateFailure::stage("live_runtime_root"))?;
    let canonical_root = fs::canonicalize(runtime_root)
        .map_err(|error| LiveGateFailure::io("live_runtime_root", &error))?;
    let root_metadata = fs::symlink_metadata(&canonical_root)
        .map_err(|error| LiveGateFailure::io("live_runtime_root", &error))?;
    if !root_metadata.is_dir() || metadata_is_link_or_reparse(&root_metadata) {
        return Err(LiveGateFailure::stage("live_runtime_root_type"));
    }

    for relative in REQUIRED_RUNTIME_FILES {
        verify_runtime_file(&canonical_root, Path::new(relative))?;
    }
    Ok(canonical_root)
}

fn verify_runtime_file(root: &Path, relative: &Path) -> Result<(), LiveGateFailure> {
    let candidate = root.join(relative);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| LiveGateFailure::io("live_runtime_file", &error))?;
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
        return Err(LiveGateFailure::stage("live_runtime_file_type"));
    }
    let resolved = fs::canonicalize(&candidate)
        .map_err(|error| LiveGateFailure::io("live_runtime_file", &error))?;
    if !resolved.starts_with(root) || resolved != candidate {
        return Err(LiveGateFailure::stage("live_runtime_containment"));
    }
    Ok(())
}

pub(super) fn sample_offsets() -> [Duration; SAMPLE_COUNT] {
    std::array::from_fn(|index| {
        SAMPLE_INTERVAL
            .checked_mul(u32::try_from(index).expect("sample index should fit u32"))
            .expect("sample offset should fit Duration")
    })
}

struct WindowSearch {
    expected_pid: u32,
    first_visible: HWND,
}

unsafe extern "system" fn enumerate_window(hwnd: HWND, context: LPARAM) -> i32 {
    if context == 0 {
        return 0;
    }
    // SAFETY: EnumWindows receives the address of a live WindowSearch stack value, keeps the
    // callback synchronous, and the callback never stores the reference or unwinds.
    let search = unsafe { &mut *(context as *mut WindowSearch) };
    if !search.first_visible.is_null() {
        return 0;
    }
    let mut owner_pid = 0u32;
    // SAFETY: `owner_pid` is correctly sized writable storage live for this call; HWND is supplied
    // by EnumWindows and is used only for observation.
    unsafe { GetWindowThreadProcessId(hwnd, &mut owner_pid) };
    // SAFETY: HWND is supplied by EnumWindows and is used only for a visibility query.
    if owner_pid == search.expected_pid && unsafe { IsWindowVisible(hwnd) } != 0 {
        search.first_visible = hwnd;
        return 0;
    }
    1
}

fn first_visible_window(expected_pid: u32) -> Result<Option<ReadyWindow>, LiveGateFailure> {
    let mut search = WindowSearch {
        expected_pid,
        first_visible: std::ptr::null_mut(),
    };
    // SAFETY: `search` remains live for the synchronous enumeration, the callback has the required
    // ABI and does not unwind, and the pointer is used only during this call.
    let enumerated = unsafe {
        EnumWindows(
            Some(enumerate_window),
            (&mut search as *mut WindowSearch) as LPARAM,
        )
    };
    if !search.first_visible.is_null() {
        return Ok(Some(ReadyWindow {
            hwnd: search.first_visible,
        }));
    }
    if enumerated == 0 {
        return Err(LiveGateFailure::last_win32("window_enumeration"));
    }
    Ok(None)
}

fn window_matches_owner(window: &ReadyWindow, expected_pid: u32) -> bool {
    let mut owner_pid = 0u32;
    // SAFETY: ReadyWindow holds an opaque observed HWND; owner_pid is writable storage live for the
    // call. No window content or input is accessed.
    let thread_id = unsafe { GetWindowThreadProcessId(window.hwnd, &mut owner_pid) };
    if thread_id == 0 || owner_pid != expected_pid {
        return false;
    }
    // SAFETY: ReadyWindow holds an opaque observed HWND used only for a visibility query.
    unsafe { IsWindowVisible(window.hwnd) != 0 }
}

fn window_is_responsive_with_timeout(
    window: &ReadyWindow,
    expected_pid: u32,
    timeout_ms: u32,
) -> bool {
    if timeout_ms == 0 || !window_matches_owner(window, expected_pid) {
        return false;
    }
    let mut message_result = 0usize;
    // SAFETY: the opaque window is revalidated against its owner immediately above; WM_NULL carries
    // no data or input, the timeout is finite, and message_result is writable storage for the call.
    unsafe {
        SendMessageTimeoutW(
            window.hwnd,
            WM_NULL,
            0,
            0,
            SMTO_ABORTIFHUNG,
            timeout_ms,
            &mut message_result,
        ) != 0
    }
}

pub(super) fn window_is_responsive(
    window: &ReadyWindow,
    expected_pid: u32,
) -> Result<bool, LiveGateFailure> {
    Ok(window_is_responsive_with_timeout(
        window,
        expected_pid,
        WINDOW_RESPONSE_TIMEOUT_MS,
    ))
}

pub(super) fn wait_for_ready_window(
    observation: &ProcessObservation,
    deadline: Instant,
) -> Result<ReadyWindow, LiveGateFailure> {
    loop {
        if observation
            .is_signaled()
            .map_err(|error| LiveGateFailure::io("window_process_wait", &error))?
        {
            return Err(LiveGateFailure::stage("window_process_exited"));
        }
        if let Some(window) = first_visible_window(observation.pid)? {
            let now = Instant::now();
            let Some(remaining) = deadline.checked_duration_since(now) else {
                return Err(LiveGateFailure::stage("window_ready_deadline"));
            };
            let timeout_ms = u32::try_from(remaining.as_millis())
                .unwrap_or(u32::MAX)
                .min(WINDOW_RESPONSE_TIMEOUT_MS);
            if timeout_ms == 0 {
                return Err(LiveGateFailure::stage("window_ready_deadline"));
            }
            if window_is_responsive_with_timeout(&window, observation.pid, timeout_ms) {
                return Ok(window);
            }
        }

        let now = Instant::now();
        let Some(remaining) = deadline.checked_duration_since(now) else {
            return Err(LiveGateFailure::stage("window_ready_deadline"));
        };
        thread::sleep(remaining.min(WINDOW_POLL_CADENCE));
    }
}

pub(crate) fn require_qualified_ready_window(
    observation: &ProcessObservation,
    profile_root: &Path,
    deadline: Instant,
) -> Result<(), LiveGateFailure> {
    let window = wait_for_ready_window(observation, deadline)?;
    let config = wait_for_profile_config(profile_root, deadline)?;
    if !config.starts_with(profile_root) {
        return Err(LiveGateFailure::stage("qualified_config_containment"));
    }
    if !window_is_responsive(&window, observation.pid)? {
        return Err(LiveGateFailure::stage("qualified_ready_window"));
    }
    Ok(())
}

pub(super) fn wait_for_profile_config(
    profile_root: &Path,
    deadline: Instant,
) -> Result<PathBuf, LiveGateFailure> {
    let canonical_profile_root = fs::canonicalize(profile_root)
        .map_err(|error| LiveGateFailure::io("config_profile_root", &error))?;
    if canonical_profile_root != profile_root {
        return Err(LiveGateFailure::stage("config_profile_root_identity"));
    }
    let profile_metadata = fs::symlink_metadata(profile_root)
        .map_err(|error| LiveGateFailure::io("config_profile_root", &error))?;
    if !profile_metadata.is_dir() || metadata_is_link_or_reparse(&profile_metadata) {
        return Err(LiveGateFailure::stage("config_profile_root_type"));
    }

    let config = profile_root.join("microemu-home/.microemulator/config2.xml");
    loop {
        match fs::symlink_metadata(&config) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
                    return Err(LiveGateFailure::stage("config_file_type"));
                }
                let canonical_config = fs::canonicalize(&config)
                    .map_err(|error| LiveGateFailure::io("config_file", &error))?;
                if canonical_config != config || !canonical_config.starts_with(profile_root) {
                    return Err(LiveGateFailure::stage("config_file_containment"));
                }
                return Ok(canonical_config);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(LiveGateFailure::io("config_file", &error)),
        }

        let now = Instant::now();
        let Some(remaining) = deadline.checked_duration_since(now) else {
            return Err(LiveGateFailure::stage("config_ready_deadline"));
        };
        thread::sleep(remaining.min(WINDOW_POLL_CADENCE));
    }
}

pub(crate) fn wait_for_process_signal(
    observation: &ProcessObservation,
    deadline: Instant,
) -> Result<(), LiveGateFailure> {
    loop {
        if observation
            .is_signaled()
            .map_err(|error| LiveGateFailure::io("cleanup_process_wait", &error))?
        {
            return Ok(());
        }
        let now = Instant::now();
        let Some(remaining) = deadline.checked_duration_since(now) else {
            return Err(LiveGateFailure::stage("cleanup_process_deadline"));
        };
        thread::sleep(remaining.min(WINDOW_POLL_CADENCE));
    }
}

pub(super) fn sample_process_resources(
    observation: &ProcessObservation,
) -> Result<ProcessResourceSample, LiveGateFailure> {
    let counter_size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>())
        .map_err(|_| LiveGateFailure::stage("metrics_memory_size"))?;
    if counter_size == 0 {
        return Err(LiveGateFailure::stage("metrics_memory_size"));
    }
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: counter_size,
        ..Default::default()
    };
    // SAFETY: the observation retains an identity-checked process handle with query and VM-read
    // rights; counters is correctly sized writable storage and ownership is not transferred.
    let memory_queried = unsafe {
        K32GetProcessMemoryInfo(
            observation.raw_handle(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counter_size,
        )
    };
    if memory_queried == 0 {
        return Err(LiveGateFailure::last_win32("metrics_memory"));
    }
    if counters.cb != counter_size || counters.WorkingSetSize == 0 || counters.PrivateUsage == 0 {
        return Err(LiveGateFailure::stage("metrics_memory_value"));
    }
    let working_set_bytes = u64::try_from(counters.WorkingSetSize)
        .map_err(|_| LiveGateFailure::stage("metrics_memory_overflow"))?;
    let private_bytes = u64::try_from(counters.PrivateUsage)
        .map_err(|_| LiveGateFailure::stage("metrics_memory_overflow"))?;

    let mut handle_count = 0u32;
    // SAFETY: the observation retains a live queryable process handle and handle_count is writable
    // storage for the complete call.
    if unsafe { GetProcessHandleCount(observation.raw_handle(), &mut handle_count) } == 0 {
        return Err(LiveGateFailure::last_win32("metrics_handles"));
    }
    if handle_count == 0 {
        return Err(LiveGateFailure::stage("metrics_handle_value"));
    }

    let (creation_time_100ns, kernel_time_100ns, user_time_100ns) =
        query_process_times(observation.raw_handle())?;
    if creation_time_100ns != observation.creation_time_100ns {
        return Err(LiveGateFailure::stage("metrics_birth_identity"));
    }
    let cpu_time_100ns = kernel_time_100ns
        .checked_add(user_time_100ns)
        .ok_or_else(|| LiveGateFailure::stage("metrics_cpu_overflow"))?;
    if cpu_time_100ns == 0 {
        return Err(LiveGateFailure::stage("metrics_cpu_value"));
    }

    Ok(ProcessResourceSample {
        working_set_bytes,
        private_bytes,
        handle_count,
        cpu_time_100ns,
    })
}

fn query_process_times(handle: HANDLE) -> Result<(u64, u64, u64), LiveGateFailure> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the caller supplies a live process handle with query access and all FILETIME values
    // are correctly sized writable storage for the complete call.
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(LiveGateFailure::last_win32("metrics_times"));
    }
    Ok((
        filetime_to_u64(creation),
        filetime_to_u64(kernel),
        filetime_to_u64(user),
    ))
}

fn filetime_to_u64(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn current_process_birth_id_for_test() -> Result<ProcessBirthId, LiveGateFailure> {
    // SAFETY: GetCurrentProcess returns a non-owning pseudo-handle valid for the current process;
    // it is borrowed only for the immediately following query and is never closed or adopted.
    let handle = unsafe { GetCurrentProcess() };
    let (creation_time_100ns, _, _) = query_process_times(handle)?;
    Ok(ProcessBirthId::for_test(
        std::process::id(),
        creation_time_100ns,
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use serde_json::Value;
    use uuid::{Uuid, Version};

    use super::*;

    struct SyntheticRuntimeFixture {
        canonical_base: PathBuf,
        canonical_path: PathBuf,
        leaf: String,
    }

    impl SyntheticRuntimeFixture {
        fn create() -> Self {
            let base = std::env::temp_dir().join("zeus-hso-live-runtime-fixtures-v1");
            fs::create_dir_all(&base).expect("synthetic fixture base should be created");
            let canonical_base =
                fs::canonicalize(base).expect("synthetic fixture base should canonicalize");
            let leaf = Uuid::new_v4().hyphenated().to_string();
            let path = canonical_base.join(&leaf);
            fs::create_dir(&path).expect("synthetic runtime root should be created");
            fs::create_dir_all(path.join("jre/bin")).expect("synthetic JRE should be created");
            fs::create_dir_all(path.join("microemulator"))
                .expect("synthetic MicroEmulator directory should be created");
            fs::create_dir_all(path.join("game"))
                .expect("synthetic game directory should be created");
            for relative in [
                "runtime-descriptor.json",
                "jre-files.sha256",
                "jre/bin/javaw.exe",
                "microemulator/microemulator.jar",
                "game/KnightOnline_402.jar",
            ] {
                fs::write(path.join(relative), b"synthetic source-test fixture")
                    .expect("synthetic runtime file should be written");
            }
            let canonical_path =
                fs::canonicalize(path).expect("synthetic runtime root should canonicalize");
            assert_eq!(canonical_path.parent(), Some(canonical_base.as_path()));
            assert_eq!(canonical_path.file_name(), Some(OsStr::new(&leaf)));
            Self {
                canonical_base,
                canonical_path,
                leaf,
            }
        }
    }

    impl Drop for SyntheticRuntimeFixture {
        fn drop(&mut self) {
            let Ok(resolved) = fs::canonicalize(&self.canonical_path) else {
                return;
            };
            if resolved != self.canonical_path
                || resolved.parent() != Some(self.canonical_base.as_path())
                || resolved.file_name() != Some(OsStr::new(&self.leaf))
            {
                return;
            }
            fs::remove_dir_all(&resolved).expect("synthetic runtime fixture should be removed");
            match fs::remove_dir(&self.canonical_base) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => panic!("synthetic fixture base cleanup failed: {error}"),
            }
        }
    }

    fn assert_no_string_values(value: &Value) {
        match value {
            Value::String(_) => panic!("live performance evidence must not contain strings"),
            Value::Array(values) => values.iter().for_each(assert_no_string_values),
            Value::Object(values) => values.values().for_each(assert_no_string_values),
            _ => {}
        }
    }

    fn remove_abandoned_live_directory(base: &Path, path: &Path, leaf: &str) {
        let _base_mutation = lock_live_test_base_mutation();
        let resolved = fs::canonicalize(path).expect("abandoned live directory should exist");
        assert_eq!(resolved, path);
        assert_eq!(resolved.parent(), Some(base));
        assert_eq!(resolved.file_name(), Some(OsStr::new(leaf)));
        fs::remove_dir_all(resolved).expect("guarded abandoned live directory cleanup should work");
        match fs::remove_dir(base) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => panic!("live test base cleanup failed: {error}"),
        }
    }

    #[test]
    fn approval_is_rejected_before_runtime_root_is_considered() {
        let missing = require_live_runtime_root_from_inputs(None, None)
            .expect_err("missing approval must fail before runtime lookup");
        assert_eq!(missing.stage_name(), "live_approval");

        let wrong = require_live_runtime_root_from_inputs(Some(OsStr::new("wrong-token")), None)
            .expect_err("wrong approval must fail before runtime lookup");
        assert_eq!(wrong.stage_name(), "live_approval");
    }

    #[test]
    fn live_test_base_mutation_coordinator_excludes_competing_mutations() {
        let first_guard = lock_live_test_base_mutation();
        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let competitor = std::thread::spawn(move || {
            attempted_tx
                .send(())
                .expect("competitor should announce its lock attempt");
            let competing_guard = lock_live_test_base_mutation();
            acquired_tx
                .send(())
                .expect("competitor should announce lock acquisition");
            drop(competing_guard);
        });

        attempted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("competitor should reach the lock attempt");
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout),
            "competing base mutation must remain excluded while the first guard is held"
        );

        drop(first_guard);
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("competitor should acquire after the first guard releases");
        competitor.join().expect("competitor should exit cleanly");
    }

    #[test]
    fn approved_synthetic_runtime_root_resolves_required_descriptor_beneath_fixture() {
        let fixture = SyntheticRuntimeFixture::create();
        let resolved = require_live_runtime_root_from_inputs(
            Some(OsStr::new(LIVE_APPROVAL_TOKEN)),
            Some(&fixture.canonical_path),
        )
        .expect("explicit synthetic runtime-shaped fixture should pass source admission");

        assert_eq!(resolved, fixture.canonical_path);
        let descriptor = fs::canonicalize(resolved.join("runtime-descriptor.json"))
            .expect("synthetic descriptor should canonicalize");
        assert!(descriptor.starts_with(&resolved));
        assert_eq!(descriptor.parent(), Some(resolved.as_path()));
    }

    #[test]
    fn live_test_directory_requires_explicit_cleanup_confirmation() {
        let abandoned = LiveTestDirectory::create().expect("live test directory should be created");
        let abandoned_base = abandoned.base().to_owned();
        let abandoned_path = abandoned.path().to_owned();
        let abandoned_leaf = abandoned.leaf().to_owned();
        assert_eq!(abandoned_path.parent(), Some(abandoned_base.as_path()));
        assert_eq!(
            abandoned_path.file_name(),
            Some(OsStr::new(&abandoned_leaf))
        );
        let parsed = Uuid::parse_str(&abandoned_leaf).expect("leaf should be a UUID");
        assert_eq!(parsed.get_version(), Some(Version::Random));
        assert_eq!(parsed.hyphenated().to_string(), abandoned_leaf);

        drop(abandoned);
        assert!(abandoned_path.is_dir());
        remove_abandoned_live_directory(&abandoned_base, &abandoned_path, &abandoned_leaf);

        let mut confirmed =
            LiveTestDirectory::create().expect("second live test directory should be created");
        let confirmed_path = confirmed.path().to_owned();
        confirmed.mark_cleanup_confirmed();
        drop(confirmed);
        assert!(!confirmed_path.exists());
    }

    #[test]
    fn sampling_constants_produce_exact_thirteen_offsets_through_sixty_seconds() {
        assert_eq!(
            LIVE_RUNTIME_ID,
            "windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402"
        );
        assert_eq!(
            LIVE_DESCRIPTOR_SHA256,
            "3e39b998bac686d8d61b8fb2da1b7f207efac463097d643341d1990525c7eac9"
        );
        assert_eq!(LIVE_TEST_BASE, "zeus-hso-live-runtime-bridge-v1");
        assert_eq!(START_DEADLINE, Duration::from_secs(30));
        assert_eq!(WINDOW_READY_DEADLINE, Duration::from_secs(30));
        assert_eq!(STOP_DEADLINE, Duration::from_secs(10));
        assert_eq!(WINDOW_POLL_CADENCE, Duration::from_millis(50));
        assert_eq!(WINDOW_RESPONSE_TIMEOUT_MS, 1_000);
        assert_eq!(STABILIZATION_DURATION, Duration::from_secs(15));
        assert_eq!(SAMPLE_INTERVAL, Duration::from_secs(5));
        assert_eq!(SAMPLE_COUNT, 13);

        let offsets = sample_offsets();
        assert_eq!(offsets.len(), 13);
        for (index, offset) in offsets.iter().enumerate() {
            assert_eq!(*offset, Duration::from_secs((index as u64) * 5));
        }
        assert_eq!(offsets[12], Duration::from_secs(60));
    }

    #[test]
    fn performance_evidence_is_numeric_only_and_has_no_forbidden_properties() {
        let evidence = serde_json::to_value(ProcessResourceSample {
            working_set_bytes: 1,
            private_bytes: 2,
            handle_count: 3,
            cpu_time_100ns: 4,
        })
        .expect("performance evidence should serialize");
        let object = evidence
            .as_object()
            .expect("performance evidence should be an object");
        assert_eq!(object.len(), 4);
        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "cpu_time_100ns",
                "handle_count",
                "private_bytes",
                "working_set_bytes",
            ]
        );
        for forbidden in [
            "pid",
            "path",
            "title",
            "argv",
            "environment",
            "username",
            "host",
        ] {
            assert!(!object.contains_key(forbidden));
        }
        assert_no_string_values(&evidence);
    }

    #[test]
    fn live_performance_record_has_the_exact_bounded_v1_shape() {
        let session = SessionPerformanceEvidenceV1 {
            index: 1,
            max_working_set_bytes: 2,
            final_working_set_bytes: 3,
            max_private_bytes: 4,
            final_private_bytes: 5,
            max_handle_count: 6,
            cpu_percent_one_core_x100: 7,
        };
        let aggregate = AggregatePerformanceEvidenceV1 {
            first_working_set_bytes: 8,
            max_working_set_bytes: 9,
            final_working_set_bytes: 10,
            working_set_growth_bytes: -1,
            first_private_bytes: 11,
            max_private_bytes: 12,
            final_private_bytes: 13,
            private_growth_bytes: -2,
            max_handle_count: 14,
            cpu_percent_one_core_x100: 15,
        };
        let evidence = LivePerformanceEvidenceV1 {
            schema_version: 1,
            runtime_id: LIVE_RUNTIME_ID,
            concurrent_sessions: 4,
            stabilization_seconds: 15,
            observation_seconds: 60,
            sample_interval_seconds: 5,
            samples_per_session: 13,
            representative_of_1gib_target: false,
            capacity_rejection_confirmed: true,
            all_windows_responsive: true,
            cleanup_confirmed: true,
            start_to_window_ms: [1, 2, 3, 4],
            per_session: [session; CONCURRENT_SESSION_COUNT],
            aggregate,
        };
        let value = serde_json::to_value(evidence).expect("v1 evidence should serialize");
        let object = value.as_object().expect("v1 evidence should be an object");
        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "aggregate",
                "all_windows_responsive",
                "capacity_rejection_confirmed",
                "cleanup_confirmed",
                "concurrent_sessions",
                "observation_seconds",
                "per_session",
                "representative_of_1gib_target",
                "runtime_id",
                "sample_interval_seconds",
                "samples_per_session",
                "schema_version",
                "stabilization_seconds",
                "start_to_window_ms",
            ]
        );
        assert_eq!(object["runtime_id"], Value::String(LIVE_RUNTIME_ID.into()));
        assert_eq!(object["start_to_window_ms"].as_array().unwrap().len(), 4);
        assert_eq!(object["per_session"].as_array().unwrap().len(), 4);
        let session_object = object["per_session"].as_array().unwrap()[0]
            .as_object()
            .unwrap();
        assert_eq!(
            session_object
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "cpu_percent_one_core_x100",
                "final_private_bytes",
                "final_working_set_bytes",
                "index",
                "max_handle_count",
                "max_private_bytes",
                "max_working_set_bytes",
            ]
        );
        let aggregate_object = object["aggregate"].as_object().unwrap();
        assert_eq!(
            aggregate_object
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "cpu_percent_one_core_x100",
                "final_private_bytes",
                "final_working_set_bytes",
                "first_private_bytes",
                "first_working_set_bytes",
                "max_handle_count",
                "max_private_bytes",
                "max_working_set_bytes",
                "private_growth_bytes",
                "working_set_growth_bytes",
            ]
        );
        for forbidden in [
            "pid",
            "creation_time",
            "path",
            "title",
            "argv",
            "environment",
            "username",
            "hostname",
            "machine_id",
        ] {
            assert!(!serde_json::to_string(&value).unwrap().contains(forbidden));
        }
    }

    #[test]
    fn later_live_tests_can_reuse_the_bounded_support_surface() {
        let _: fn() -> PathBuf = require_live_runtime_root;
        let _: fn(&ProcessObservation, Instant) -> Result<ReadyWindow, LiveGateFailure> =
            wait_for_ready_window;
        let _: fn(&ReadyWindow, u32) -> Result<bool, LiveGateFailure> = window_is_responsive;
    }

    #[test]
    fn current_process_metrics_preserve_birth_identity_and_are_nonzero() {
        let birth_id = current_process_birth_id_for_test()
            .expect("current process birth identity should be available");
        let observation = open_identity_checked_process_for_metrics(
            birth_id.pid(),
            birth_id.creation_time_100ns(),
        )
        .expect("current process should open for bounded metrics");
        assert_eq!(observation.birth_id(), birth_id);
        assert!(
            !observation
                .is_signaled()
                .expect("zero-time wait should work")
        );

        let cpu_accounting_deadline = Instant::now() + Duration::from_secs(2);
        let mut accumulator = 0u64;
        let sample = loop {
            match sample_process_resources(&observation) {
                Ok(sample) => break sample,
                Err(error)
                    if error.stage_name() == "metrics_cpu_value"
                        && Instant::now() < cpu_accounting_deadline =>
                {
                    for value in 0..100_000u64 {
                        accumulator = accumulator.wrapping_add(value.rotate_left(7));
                    }
                    std::hint::black_box(accumulator);
                }
                Err(error) => panic!("current process metrics should be sampled: {error}"),
            }
        };
        assert!(sample.working_set_bytes > 0);
        assert!(sample.private_bytes > 0);
        assert!(sample.handle_count > 0);
        assert!(sample.cpu_time_100ns > 0);
    }

    #[test]
    fn parent_crash_root_shape_is_independent_of_ambient_temp_and_reparse_safe() {
        let token = "01234567-89ab-4def-8123-456789abcdef";
        let base = Path::new(r"\\?\C:\bounded\zeus-hso-live-runtime-bridge-v1");
        let root = base.join(token);
        assert!(parent_crash_guarded_root_shape_is_valid(
            &root, &root, base, token, true, true,
        ));
        let aliased = PathBuf::from(format!(
            r"{}\..\{}\{}",
            base.display(),
            LIVE_TEST_BASE,
            token
        ));
        assert!(!parent_crash_guarded_root_shape_is_valid(
            &aliased, &root, base, token, true, true,
        ));
        assert!(!parent_crash_guarded_root_shape_is_valid(
            &root, &root, base, token, false, true,
        ));
        assert!(!parent_crash_guarded_root_shape_is_valid(
            &root, &root, base, token, true, false,
        ));
    }
}

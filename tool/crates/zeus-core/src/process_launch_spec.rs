use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use uuid::Uuid;

use crate::data_root::metadata_is_link_or_reparse;
use crate::launch_snapshot::{ENVIRONMENT_KEY_ORDER, ENVIRONMENT_VARIABLE_COUNT};
use crate::{CoreError, CoreResult, LaunchEnvironmentVariable, LaunchSnapshot};

#[cfg(windows)]
pub(crate) mod windows;

pub const PROCESS_ARGV_SCHEMA_VERSION: u32 = 1;
pub const MAX_PROCESS_ARGUMENTS: usize = 32;
pub const MAX_PROCESS_ARGUMENT_NATIVE_UNITS: usize = 4_096;
pub const MAX_WINDOWS_COMMAND_LINE_UNITS: usize = 32_767;

/// System property naming where the mod publishes its read-only character snapshot.
///
/// Passed on every launch, including a vanilla game jar: an unrecognised `-D` property is inert to the
/// JVM, so one argv shape serves both jars and swapping the pinned jar needs no launch change.
const PLAYER_SNAPSHOT_PROPERTY: &str = "-Dzeus.player.out=";

/// System property naming where the tool writes the attack and item settings.
///
/// Passed unconditionally for the same reason as the snapshot property, and pinned to the same
/// private directory: the settings carry the monster spot, so a redirected file would let another
/// process steer the character.
const CONTROL_SETTINGS_PROPERTY: &str = "-Dzeus.ctl.in=";

/// Fixed argv positions of the typed Java paths.
///
/// The builder below and the adapter-boundary revalidation both index the same constants, so inserting
/// an argument cannot leave one of them checking the wrong slot.
#[cfg(windows)]
mod java_argv {
    pub(super) const USER_HOME: usize = 0;
    pub(super) const TEMP_DIRECTORY: usize = 1;
    pub(super) const PLAYER_SNAPSHOT: usize = 2;
    pub(super) const CONTROL_SETTINGS: usize = 3;
    pub(super) const ERROR_FILE: usize = 8;
    pub(super) const CLASSPATH_FLAG: usize = 9;
    pub(super) const CLASSPATH: usize = 10;
    /// Every position above must be present before the shape is accepted.
    pub(super) const MINIMUM_LENGTH: usize = CLASSPATH + 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStdio {
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessArgumentShape {
    JavaRuntime,
    #[cfg(all(test, windows))]
    WindowsTestProbe,
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedWindowsPaths {
    executable: PathBuf,
    working_directory: PathBuf,
    temp_directory: PathBuf,
    /// `%SystemRoot%`, sealed like every other path so the adapter cannot be handed a different one.
    system_root: PathBuf,
    java: Option<SealedWindowsJavaPaths>,
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedWindowsJavaPaths {
    user_home: PathBuf,
    error_file_parent: PathBuf,
    classpath: [PathBuf; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunchSpec {
    session_id: Uuid,
    profile_id: Uuid,
    argv_schema_version: u32,
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: [LaunchEnvironmentVariable; ENVIRONMENT_VARIABLE_COUNT],
    inherit_environment: bool,
    stdin: ProcessStdio,
    stdout: ProcessStdio,
    stderr: ProcessStdio,
    argument_shape: ProcessArgumentShape,
    #[cfg(windows)]
    sealed_windows_paths: SealedWindowsPaths,
}

impl ProcessLaunchSpec {
    #[cfg(all(test, windows))]
    pub(crate) fn for_windows_test(
        executable: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        temp_directory: PathBuf,
    ) -> CoreResult<Self> {
        if !canonical_file(&executable) {
            return Err(spec_error("process_executable_invalid"));
        }
        if !canonical_directory(&working_directory) {
            return Err(spec_error("process_working_directory_invalid"));
        }
        if !canonical_directory(&temp_directory)
            || temp_directory.parent() != Some(working_directory.as_path())
        {
            return Err(spec_error("process_environment_invalid"));
        }
        let sealed_windows_paths = SealedWindowsPaths {
            executable: executable.clone(),
            working_directory: working_directory.clone(),
            temp_directory: temp_directory.clone(),
            system_root: crate::launch_snapshot::system_root_directory(),
            java: None,
        };
        let executable = windows::local_drive_invocation_path(&executable)?;
        let working_directory = windows::local_drive_invocation_path(&working_directory)?;
        let invocation_temp = windows::local_drive_invocation_path(&temp_directory)?;
        // Only the profile-scoped entries are remapped to the invocation temp path. `SystemRoot` is
        // not a profile path, so rewriting it would both break validation and strip the one variable
        // WinSock needs.
        let environment =
            crate::launch_snapshot::fixed_test_environment(&temp_directory).map(|variable| {
                if variable.key().is_profile_temp_directory() {
                    variable.with_value_for_process(invocation_temp.clone())
                } else {
                    variable
                }
            });
        let spec = Self {
            session_id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            argv_schema_version: PROCESS_ARGV_SCHEMA_VERSION,
            executable,
            arguments,
            working_directory,
            environment,
            inherit_environment: false,
            stdin: ProcessStdio::Null,
            stdout: ProcessStdio::Null,
            stderr: ProcessStdio::Null,
            argument_shape: ProcessArgumentShape::WindowsTestProbe,
            sealed_windows_paths,
        };
        spec.revalidate_for_adapter()?;
        Ok(spec)
    }

    #[cfg_attr(
        not(windows),
        allow(
            dead_code,
            reason = "live adapter-boundary revalidation is Windows-only in v1"
        )
    )]
    pub(crate) fn revalidate_for_adapter(&self) -> CoreResult<()> {
        if self.argv_schema_version != PROCESS_ARGV_SCHEMA_VERSION {
            return Err(spec_error("process_argv_schema_invalid"));
        }
        #[cfg(windows)]
        let executable_valid = exact_invocation_path(
            &self.executable,
            &self.sealed_windows_paths.executable,
            false,
        );
        #[cfg(not(windows))]
        let executable_valid = canonical_file(&self.executable);
        if !executable_valid {
            return Err(spec_error("process_executable_invalid"));
        }
        #[cfg(windows)]
        let working_directory_valid = exact_invocation_path(
            &self.working_directory,
            &self.sealed_windows_paths.working_directory,
            true,
        );
        #[cfg(not(windows))]
        let working_directory_valid = canonical_directory(&self.working_directory);
        if !working_directory_valid {
            return Err(spec_error("process_working_directory_invalid"));
        }
        if self
            .environment
            .iter()
            .map(LaunchEnvironmentVariable::key)
            .ne(ENVIRONMENT_KEY_ORDER)
        {
            return Err(spec_error("process_environment_invalid"));
        }
        let temp_directory = self.environment[0].value();
        #[cfg(windows)]
        let temp_directory_valid = exact_invocation_path(
            temp_directory,
            &self.sealed_windows_paths.temp_directory,
            true,
        );
        #[cfg(not(windows))]
        let temp_directory_valid = canonical_directory(temp_directory);
        // Every profile-scoped entry must be the one profile temp directory. `SystemRoot` is the one
        // entry that is deliberately not a profile path, so it is checked as a real directory instead
        // of against the temp directory it must not equal.
        if !temp_directory_valid
            || temp_directory.parent() != Some(self.working_directory.as_path())
            || self.environment.iter().any(|variable| {
                variable.key().is_profile_temp_directory() && variable.value() != temp_directory
            })
        {
            return Err(spec_error("process_environment_invalid"));
        }
        // `SystemRoot` is checked against its sealed value and as a real directory, but NOT against
        // the canonical `\\?\` form the other paths use: the extended prefix is not a usable
        // `%SystemRoot%` for the loader, so the plain absolute path is the correct value here.
        #[cfg(windows)]
        if self
            .environment
            .iter()
            .filter(|variable| !variable.key().is_profile_temp_directory())
            .any(|variable| {
                let value = variable.value();
                !value.is_absolute()
                    || native_units(value.as_os_str()).is_none()
                    || !fs::symlink_metadata(value).is_ok_and(|metadata| {
                        metadata.is_dir() && !metadata_is_link_or_reparse(&metadata)
                    })
                    || value != self.sealed_windows_paths.system_root
            })
        {
            return Err(spec_error("process_environment_invalid"));
        }
        if self.inherit_environment {
            return Err(spec_error("process_environment_inheritance_invalid"));
        }
        if self.stdin != ProcessStdio::Null
            || self.stdout != ProcessStdio::Null
            || self.stderr != ProcessStdio::Null
        {
            return Err(spec_error("process_stdio_invalid"));
        }
        #[cfg(windows)]
        if self.argument_shape == ProcessArgumentShape::JavaRuntime {
            let sealed_java = self
                .sealed_windows_paths
                .java
                .as_ref()
                .ok_or_else(|| spec_error("process_argument_shape_invalid"))?;
            validate_java_invocation_paths(
                &self.arguments,
                &self.working_directory,
                temp_directory,
                &self.sealed_windows_paths.temp_directory,
                sealed_java,
            )?;
        } else if self.sealed_windows_paths.java.is_some() {
            return Err(spec_error("process_argument_shape_invalid"));
        }
        validate_argument_bounds(&self.executable, &self.arguments)
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn argv_schema_version(&self) -> u32 {
        self.argv_schema_version
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn environment(&self) -> &[LaunchEnvironmentVariable] {
        &self.environment
    }

    pub fn inherit_environment(&self) -> bool {
        self.inherit_environment
    }

    pub fn stdin(&self) -> ProcessStdio {
        self.stdin
    }

    pub fn stdout(&self) -> ProcessStdio {
        self.stdout
    }

    pub fn stderr(&self) -> ProcessStdio {
        self.stderr
    }
}

impl LaunchSnapshot {
    pub fn process_launch_spec(&self) -> CoreResult<ProcessLaunchSpec> {
        validate_snapshot_paths(self)?;
        if path_contains_classpath_separator(self.microemulator_jar())
            || path_contains_classpath_separator(self.game_jar())
        {
            return Err(spec_error("process_classpath_path_invalid"));
        }
        #[cfg(windows)]
        let executable = windows::local_drive_invocation_path(self.java_executable())?;
        #[cfg(not(windows))]
        let executable = self.java_executable().to_owned();
        #[cfg(windows)]
        let working_directory = windows::local_drive_invocation_path(self.working_directory())?;
        #[cfg(not(windows))]
        let working_directory = self.working_directory().to_owned();
        #[cfg(windows)]
        let environment = {
            let temp_directory = windows::local_drive_invocation_path(self.temp_directory())?;
            // Only the profile-temp entries are remapped to the invocation form of the temp
            // directory. `SystemRoot` names a machine path outside the profile, so rewriting it here
            // would point the child at the profile temp directory and break host resolution again.
            std::array::from_fn(|index| {
                let variable = &self.environment()[index];
                if variable.key().is_profile_temp_directory() {
                    variable.with_value_for_process(temp_directory.clone())
                } else {
                    variable.clone()
                }
            })
        };
        #[cfg(not(windows))]
        let environment = std::array::from_fn(|index| self.environment()[index].clone());

        #[cfg(windows)]
        let user_home = invocation_prefixed_path("-Duser.home=", self.microemu_home())?;
        #[cfg(not(windows))]
        let user_home = prefixed_path("-Duser.home=", self.microemu_home());
        #[cfg(windows)]
        let temp_directory = invocation_prefixed_path("-Djava.io.tmpdir=", self.temp_directory())?;
        #[cfg(not(windows))]
        let temp_directory = prefixed_path("-Djava.io.tmpdir=", self.temp_directory());
        // The snapshot is written inside the profile's own `microemu-home`, the same private directory
        // `-Duser.home` already points at, so publishing needs no additional writable path.
        #[cfg(windows)]
        let player_snapshot = invocation_prefixed_child_path(
            PLAYER_SNAPSHOT_PROPERTY,
            self.microemu_home(),
            OsStr::new(crate::player::SNAPSHOT_FILE_NAME),
        )?;
        #[cfg(not(windows))]
        let player_snapshot = prefixed_path(
            PLAYER_SNAPSHOT_PROPERTY,
            &self.microemu_home().join(crate::player::SNAPSHOT_FILE_NAME),
        );
        // Same private directory and the same pinning: the settings carry the monster spot, so a
        // redirected file would let another process steer the character.
        #[cfg(windows)]
        let control_settings = invocation_prefixed_child_path(
            CONTROL_SETTINGS_PROPERTY,
            self.microemu_home(),
            OsStr::new(crate::control::CONTROL_FILE_NAME),
        )?;
        #[cfg(not(windows))]
        let control_settings = prefixed_path(
            CONTROL_SETTINGS_PROPERTY,
            &self.microemu_home().join(crate::control::CONTROL_FILE_NAME),
        );
        #[cfg(windows)]
        let error_file = invocation_prefixed_child_path(
            "-XX:ErrorFile=",
            self.profile_root(),
            OsStr::new("hs_err_pid%p.log"),
        )?;
        #[cfg(not(windows))]
        let error_file = prefixed_path(
            "-XX:ErrorFile=",
            &self.profile_root().join("hs_err_pid%p.log"),
        );
        #[cfg(windows)]
        let classpath = invocation_classpath(self.microemulator_jar(), self.game_jar())?;
        #[cfg(not(windows))]
        let classpath = {
            let mut value = OsString::from(self.microemulator_jar().as_os_str());
            value.push(":");
            value.push(self.game_jar().as_os_str());
            value
        };

        let heap = self.heap();
        let screen = self.screen_size();
        let mut arguments = vec![
            user_home,
            temp_directory,
            player_snapshot,
            control_settings,
            OsString::from(format!("-Xms{}m", heap.initial_mib())),
            OsString::from(format!("-Xmx{}m", heap.maximum_mib())),
            OsString::from("-XX:+UseSerialGC"),
            OsString::from("-XX:-UsePerfData"),
            error_file,
            OsString::from("-cp"),
            classpath,
            OsString::from(self.main_class()),
            OsString::from("--resizableDevice"),
            OsString::from(screen.width().to_string()),
            OsString::from(screen.height().to_string()),
            OsString::from("--rms"),
            OsString::from("file"),
            OsString::from("--id"),
            OsString::from(self.profile_id().to_string()),
        ];
        if self.quiet() {
            arguments.push(OsString::from("--quiet"));
        }
        if self.quit_on_midlet_destroy() {
            arguments.push(OsString::from("--quit"));
        }
        arguments.push(OsString::from(self.midlet_class()));
        validate_argument_bounds(&executable, &arguments)?;

        #[cfg(windows)]
        let sealed_windows_paths = SealedWindowsPaths {
            executable: self.java_executable().to_owned(),
            working_directory: self.working_directory().to_owned(),
            temp_directory: self.temp_directory().to_owned(),
            system_root: self.system_root_directory().to_owned(),
            java: Some(SealedWindowsJavaPaths {
                user_home: self.microemu_home().to_owned(),
                error_file_parent: self.profile_root().to_owned(),
                classpath: [
                    self.microemulator_jar().to_owned(),
                    self.game_jar().to_owned(),
                ],
            }),
        };
        let spec = ProcessLaunchSpec {
            session_id: self.session_id(),
            profile_id: self.profile_id(),
            argv_schema_version: PROCESS_ARGV_SCHEMA_VERSION,
            executable,
            arguments,
            working_directory,
            environment,
            inherit_environment: false,
            stdin: ProcessStdio::Null,
            stdout: ProcessStdio::Null,
            stderr: ProcessStdio::Null,
            argument_shape: ProcessArgumentShape::JavaRuntime,
            #[cfg(windows)]
            sealed_windows_paths,
        };
        spec.revalidate_for_adapter()?;
        Ok(spec)
    }
}

fn validate_snapshot_paths(snapshot: &LaunchSnapshot) -> CoreResult<()> {
    if !canonical_directory(snapshot.runtime_root()) {
        return Err(spec_error("process_runtime_path_invalid"));
    }
    for artifact in [
        snapshot.java_executable(),
        snapshot.microemulator_jar(),
        snapshot.game_jar(),
    ] {
        if !artifact.starts_with(snapshot.runtime_root()) || !canonical_file(artifact) {
            return Err(spec_error("process_artifact_path_invalid"));
        }
    }
    if !canonical_directory(snapshot.profile_root())
        || snapshot.working_directory() != snapshot.profile_root()
        || !canonical_directory(snapshot.working_directory())
    {
        return Err(spec_error("process_working_directory_invalid"));
    }
    for writable in [snapshot.microemu_home(), snapshot.temp_directory()] {
        if !writable.starts_with(snapshot.profile_root()) || !canonical_directory(writable) {
            return Err(spec_error("process_writable_path_invalid"));
        }
    }
    if snapshot.microemu_home() == snapshot.temp_directory() {
        return Err(spec_error("process_writable_path_invalid"));
    }
    Ok(())
}

fn canonical_file(path: &Path) -> bool {
    canonical_path(path, false)
}

fn canonical_directory(path: &Path) -> bool {
    canonical_path(path, true)
}

#[cfg(windows)]
fn exact_invocation_path(path: &Path, sealed_canonical: &Path, directory: bool) -> bool {
    if !path.is_absolute()
        || native_units(path.as_os_str()).is_none()
        || !canonical_path(sealed_canonical, directory)
    {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata_is_link_or_reparse(&metadata)
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return false;
    }
    fs::canonicalize(path)
        .ok()
        .is_some_and(|canonical| exact_path_units(&canonical, sealed_canonical))
        && windows::local_drive_invocation_path(sealed_canonical)
            .ok()
            .is_some_and(|rendered| exact_path_units(&rendered, path))
}

fn canonical_path(path: &Path, directory: bool) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata_is_link_or_reparse(&metadata)
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return false;
    }
    fs::canonicalize(path).is_ok_and(|canonical| exact_path_units(&canonical, path))
}

#[cfg(windows)]
fn exact_path_units(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .encode_wide()
        .eq(right.as_os_str().encode_wide())
}

#[cfg(unix)]
fn exact_path_units(left: &Path, right: &Path) -> bool {
    left.as_os_str().as_bytes() == right.as_os_str().as_bytes()
}

fn path_contains_classpath_separator(path: &Path) -> bool {
    #[cfg(windows)]
    {
        path.as_os_str().encode_wide().any(|unit| unit == 59u16)
    }
    #[cfg(unix)]
    {
        path.as_os_str().as_bytes().contains(&b':')
    }
}

fn validate_argument_bounds(executable: &Path, arguments: &[OsString]) -> CoreResult<()> {
    if arguments.len() > MAX_PROCESS_ARGUMENTS {
        return Err(spec_error("process_argument_count_exceeded"));
    }
    native_units(executable.as_os_str()).ok_or_else(|| spec_error("process_executable_invalid"))?;
    for argument in arguments {
        let units = native_units(argument.as_os_str())
            .ok_or_else(|| spec_error("process_argument_invalid"))?;
        if units > MAX_PROCESS_ARGUMENT_NATIVE_UNITS {
            return Err(spec_error("process_argument_too_long"));
        }
    }
    #[cfg(windows)]
    {
        windows::encode_application_name(executable)?;
        windows::encode_command_line(executable, arguments)?;
    }
    Ok(())
}

#[cfg(windows)]
fn native_units(value: &OsStr) -> Option<usize> {
    let mut count = 0usize;
    for unit in value.encode_wide() {
        if unit == 0 {
            return None;
        }
        count = count.checked_add(1)?;
    }
    Some(count)
}

#[cfg(unix)]
fn native_units(value: &OsStr) -> Option<usize> {
    let bytes = value.as_bytes();
    (!bytes.contains(&0)).then_some(bytes.len())
}

fn spec_error(code: &'static str) -> CoreError {
    CoreError::ProcessLaunchSpec { code }
}

fn prefixed_path(prefix: &str, path: &Path) -> OsString {
    let mut value = OsString::from(prefix);
    value.push(path.as_os_str());
    value
}

#[cfg(windows)]
fn invocation_prefixed_path(prefix: &str, canonical: &Path) -> CoreResult<OsString> {
    Ok(prefixed_path(
        prefix,
        &windows::local_drive_invocation_path(canonical)?,
    ))
}

#[cfg(windows)]
fn invocation_prefixed_child_path(
    prefix: &str,
    canonical_parent: &Path,
    child: &OsStr,
) -> CoreResult<OsString> {
    let child_path = Path::new(child);
    let mut components = child_path.components();
    if native_units(child).is_none()
        || !matches!(
            components.next(),
            Some(std::path::Component::Normal(value)) if value == child
        )
        || components.next().is_some()
    {
        return Err(spec_error("process_invocation_path_invalid"));
    }
    Ok(prefixed_path(
        prefix,
        &windows::local_drive_invocation_path(canonical_parent)?.join(child),
    ))
}

#[cfg(windows)]
fn invocation_classpath(first: &Path, second: &Path) -> CoreResult<OsString> {
    let first = windows::local_drive_invocation_path(first)?;
    let second = windows::local_drive_invocation_path(second)?;
    let mut classpath = OsString::from(first.as_os_str());
    classpath.push(";");
    classpath.push(second.as_os_str());
    Ok(classpath)
}

#[cfg(windows)]
fn validate_java_invocation_paths(
    arguments: &[OsString],
    working_directory: &Path,
    temp_directory: &Path,
    sealed_temp_directory: &Path,
    sealed: &SealedWindowsJavaPaths,
) -> CoreResult<()> {
    if arguments.len() < java_argv::MINIMUM_LENGTH
        || arguments[java_argv::CLASSPATH_FLAG] != OsStr::new("-cp")
    {
        return Err(spec_error("process_argument_shape_invalid"));
    }
    let user_home = prefixed_argument_path(&arguments[java_argv::USER_HOME], "-Duser.home=")
        .ok_or_else(|| spec_error("process_argument_path_invalid"))?;
    let java_temp =
        prefixed_argument_path(&arguments[java_argv::TEMP_DIRECTORY], "-Djava.io.tmpdir=")
            .ok_or_else(|| spec_error("process_argument_path_invalid"))?;
    let player_snapshot = prefixed_argument_path(
        &arguments[java_argv::PLAYER_SNAPSHOT],
        PLAYER_SNAPSHOT_PROPERTY,
    )
    .ok_or_else(|| spec_error("process_argument_path_invalid"))?;
    let control_settings = prefixed_argument_path(
        &arguments[java_argv::CONTROL_SETTINGS],
        CONTROL_SETTINGS_PROPERTY,
    )
    .ok_or_else(|| spec_error("process_argument_path_invalid"))?;
    let error_file = prefixed_argument_path(&arguments[java_argv::ERROR_FILE], "-XX:ErrorFile=")
        .ok_or_else(|| spec_error("process_argument_path_invalid"))?;
    if !exact_invocation_path(&user_home, &sealed.user_home, true)
        || user_home.parent() != Some(working_directory)
        || user_home == temp_directory
        || !exact_invocation_path(&java_temp, sealed_temp_directory, true)
        || java_temp != temp_directory
        // The snapshot must land in the already-verified private home and nowhere else, so the
        // published file cannot be redirected out of the profile by a mutated argument.
        || player_snapshot.parent() != Some(user_home.as_path())
        || player_snapshot.file_name() != Some(OsStr::new(crate::player::SNAPSHOT_FILE_NAME))
        // The settings carry the monster spot, so the same pinning applies in the other direction.
        || control_settings.parent() != Some(user_home.as_path())
        || control_settings.file_name() != Some(OsStr::new(crate::control::CONTROL_FILE_NAME))
        || error_file.parent() != Some(working_directory)
        || error_file
            .parent()
            .is_none_or(|parent| !exact_invocation_path(parent, &sealed.error_file_parent, true))
        || error_file.file_name() != Some(OsStr::new("hs_err_pid%p.log"))
    {
        return Err(spec_error("process_argument_path_invalid"));
    }
    let (first, second) = split_windows_classpath(&arguments[java_argv::CLASSPATH])
        .ok_or_else(|| spec_error("process_classpath_path_invalid"))?;
    if !exact_invocation_path(&first, &sealed.classpath[0], false)
        || !exact_invocation_path(&second, &sealed.classpath[1], false)
    {
        return Err(spec_error("process_classpath_path_invalid"));
    }
    Ok(())
}

#[cfg(windows)]
fn prefixed_argument_path(argument: &OsStr, prefix: &str) -> Option<PathBuf> {
    let units = argument.encode_wide().collect::<Vec<_>>();
    let prefix = prefix.encode_utf16().collect::<Vec<_>>();
    if !units.starts_with(&prefix) || units.len() == prefix.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&units[prefix.len()..])))
}

#[cfg(windows)]
fn split_windows_classpath(argument: &OsStr) -> Option<(PathBuf, PathBuf)> {
    let units = argument.encode_wide().collect::<Vec<_>>();
    let separators = units
        .iter()
        .enumerate()
        .filter_map(|(index, unit)| (*unit == b';' as u16).then_some(index))
        .collect::<Vec<_>>();
    if separators.len() != 1 || separators[0] == 0 || separators[0] + 1 == units.len() {
        return None;
    }
    let separator = separators[0];
    Some((
        PathBuf::from(OsString::from_wide(&units[..separator])),
        PathBuf::from(OsString::from_wide(&units[separator + 1..])),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreError;
    #[cfg(windows)]
    use crate::data_root::DataRoot;

    #[cfg(windows)]
    const SPEC_TEST_ROOT: &str = "zeus-hso-wcpv1-spec-tests";

    #[cfg(windows)]
    struct WindowsSpecTestDirectory {
        canonical_base: PathBuf,
        canonical_path: PathBuf,
        leaf: String,
        temp_directory: PathBuf,
    }

    #[cfg(windows)]
    impl WindowsSpecTestDirectory {
        fn create() -> Self {
            let base = std::env::temp_dir().join(SPEC_TEST_ROOT);
            fs::create_dir_all(&base).unwrap();
            let canonical_base = fs::canonicalize(base).unwrap();
            let leaf = Uuid::new_v4().to_string();
            let path = canonical_base.join(&leaf);
            let data_root = DataRoot::prepare_at(&path).unwrap();
            let canonical_path = fs::canonicalize(data_root.path()).unwrap();
            let mut directory = Self {
                canonical_base,
                canonical_path,
                leaf,
                temp_directory: PathBuf::new(),
            };
            assert_eq!(
                directory.canonical_path.parent(),
                Some(directory.canonical_base.as_path())
            );
            assert_eq!(
                directory.canonical_path.file_name(),
                Some(OsStr::new(&directory.leaf))
            );
            let temp_directory = data_root.ensure_private_child_directory("temp").unwrap();
            let temp_directory = fs::canonicalize(temp_directory).unwrap();
            assert_eq!(
                temp_directory.parent(),
                Some(directory.canonical_path.as_path())
            );
            assert!(data_root.is_private().unwrap());
            directory.temp_directory = temp_directory;
            directory
        }
    }

    #[cfg(windows)]
    impl Drop for WindowsSpecTestDirectory {
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
            let _ = fs::remove_dir_all(resolved);
        }
    }

    #[cfg(windows)]
    fn valid_windows_test_spec() -> (ProcessLaunchSpec, WindowsSpecTestDirectory) {
        let workspace = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap(),
        )
        .unwrap();
        let executable = fs::canonicalize(
            workspace
                .join(".devtools/rustup/toolchains/1.98.0-x86_64-pc-windows-gnu/bin/rustc.exe"),
        )
        .unwrap();
        let directory = WindowsSpecTestDirectory::create();

        let spec = ProcessLaunchSpec::for_windows_test(
            executable,
            vec![OsString::from("report")],
            directory.canonical_path.clone(),
            directory.temp_directory.clone(),
        )
        .unwrap();
        (spec, directory)
    }

    #[cfg(windows)]
    fn valid_windows_java_test_spec() -> (ProcessLaunchSpec, WindowsSpecTestDirectory, PathBuf) {
        let (mut spec, directory) = valid_windows_test_spec();
        let canonical_home = directory.canonical_path.join("microemu-home");
        fs::create_dir(&canonical_home).unwrap();
        let canonical_home = fs::canonicalize(canonical_home).unwrap();
        let workspace = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap(),
        )
        .unwrap();
        let first = fs::canonicalize(workspace.join("Cargo.toml")).unwrap();
        let second = fs::canonicalize(workspace.join("crates/zeus-core/Cargo.toml")).unwrap();
        spec.arguments = vec![
            invocation_prefixed_path("-Duser.home=", &canonical_home).unwrap(),
            invocation_prefixed_path("-Djava.io.tmpdir=", &directory.temp_directory).unwrap(),
            invocation_prefixed_child_path(
                PLAYER_SNAPSHOT_PROPERTY,
                &canonical_home,
                OsStr::new(crate::player::SNAPSHOT_FILE_NAME),
            )
            .unwrap(),
            invocation_prefixed_child_path(
                CONTROL_SETTINGS_PROPERTY,
                &canonical_home,
                OsStr::new(crate::control::CONTROL_FILE_NAME),
            )
            .unwrap(),
            OsString::from("-Xms16m"),
            OsString::from("-Xmx128m"),
            OsString::from("-XX:+UseSerialGC"),
            OsString::from("-XX:-UsePerfData"),
            invocation_prefixed_child_path(
                "-XX:ErrorFile=",
                &directory.canonical_path,
                OsStr::new("hs_err_pid%p.log"),
            )
            .unwrap(),
            OsString::from("-cp"),
            invocation_classpath(&first, &second).unwrap(),
        ];
        spec.argument_shape = ProcessArgumentShape::JavaRuntime;
        spec.sealed_windows_paths.java = Some(SealedWindowsJavaPaths {
            user_home: canonical_home.clone(),
            error_file_parent: directory.canonical_path.clone(),
            classpath: [first, second],
        });
        spec.revalidate_for_adapter().unwrap();
        (spec, directory, canonical_home)
    }

    #[test]
    fn argument_count_over_thirty_two_is_rejected() {
        let arguments = vec![OsString::from("x"); 33];

        assert!(matches!(
            validate_argument_bounds(Path::new("java"), &arguments),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_count_exceeded"
            })
        ));
    }

    #[test]
    fn one_argument_over_four_thousand_ninety_six_native_units_is_rejected() {
        let arguments = vec![OsString::from("x".repeat(4_097))];

        assert!(matches!(
            validate_argument_bounds(Path::new("java"), &arguments),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_too_long"
            })
        ));
    }

    #[test]
    #[cfg(unix)]
    fn portable_validation_does_not_apply_windows_aggregate_limit() {
        let arguments = vec![OsString::from("x".repeat(4_096)); 4];

        assert!(validate_argument_bounds(Path::new("/java"), &arguments).is_ok());
    }

    #[test]
    fn native_nul_is_rejected() {
        let arguments = vec![OsString::from("bad\0argument")];

        assert!(matches!(
            validate_argument_bounds(Path::new("java"), &arguments),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_invalid"
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_test_fixture_is_sealed_and_revalidates() {
        let (spec, directory) = valid_windows_test_spec();

        assert!(!spec.session_id().is_nil());
        assert!(!spec.profile_id().is_nil());
        assert_eq!(spec.argv_schema_version(), PROCESS_ARGV_SCHEMA_VERSION);
        assert_eq!(spec.arguments(), &[OsString::from("report")]);
        assert_eq!(
            spec.environment()
                .iter()
                .map(LaunchEnvironmentVariable::key)
                .collect::<Vec<_>>(),
            ENVIRONMENT_KEY_ORDER
        );
        // Exactly one entry is not the profile temp directory, and it is the machine `SystemRoot`.
        let non_temp: Vec<&LaunchEnvironmentVariable> = spec
            .environment()
            .iter()
            .filter(|variable| !variable.key().is_profile_temp_directory())
            .collect();
        assert_eq!(non_temp.len(), 1);
        assert_eq!(
            non_temp[0].value(),
            crate::launch_snapshot::system_root_directory()
        );
        assert!(!spec.inherit_environment());
        assert_eq!(spec.stdin(), ProcessStdio::Null);
        assert_eq!(spec.stdout(), ProcessStdio::Null);
        assert_eq!(spec.stderr(), ProcessStdio::Null);
        assert_ne!(spec.working_directory(), directory.canonical_path);
        assert_eq!(
            fs::canonicalize(spec.working_directory()).unwrap(),
            directory.canonical_path
        );
        assert_ne!(spec.environment()[0].value(), directory.temp_directory);
        assert_eq!(
            fs::canonicalize(spec.environment()[0].value()).unwrap(),
            directory.temp_directory
        );
        assert_eq!(
            spec.environment()[0].value().parent(),
            Some(spec.working_directory())
        );
        assert!(spec.revalidate_for_adapter().is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn typed_java_path_arguments_render_each_canonical_element_for_invocation() {
        let workspace = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap(),
        )
        .unwrap();
        let first = fs::canonicalize(workspace.join("Cargo.toml")).unwrap();
        let second = fs::canonicalize(workspace.join("crates/zeus-core/Cargo.toml")).unwrap();
        let directory = WindowsSpecTestDirectory::create();

        let home = invocation_prefixed_path("-Duser.home=", &directory.canonical_path).unwrap();
        let error_file = invocation_prefixed_child_path(
            "-XX:ErrorFile=",
            &directory.canonical_path,
            OsStr::new("hs_err_pid%p.log"),
        )
        .unwrap();
        let classpath = invocation_classpath(&first, &second).unwrap();
        let invocation_home =
            windows::local_drive_invocation_path(&directory.canonical_path).unwrap();
        let invocation_first = windows::local_drive_invocation_path(&first).unwrap();
        let invocation_second = windows::local_drive_invocation_path(&second).unwrap();

        let mut expected_home = OsString::from("-Duser.home=");
        expected_home.push(&invocation_home);
        assert_eq!(home, expected_home);
        let mut expected_error = OsString::from("-XX:ErrorFile=");
        expected_error.push(invocation_home.join("hs_err_pid%p.log"));
        assert_eq!(error_file, expected_error);
        let mut expected_classpath = OsString::from(invocation_first.as_os_str());
        expected_classpath.push(";");
        expected_classpath.push(invocation_second.as_os_str());
        assert_eq!(classpath, expected_classpath);
    }

    #[cfg(windows)]
    #[test]
    fn windows_adapter_revalidation_rejects_mutated_internal_invariants() {
        use crate::launch_snapshot::fixed_test_environment;

        let (mut wrong_schema, _directory) = valid_windows_test_spec();
        wrong_schema.argv_schema_version += 1;
        assert!(matches!(
            wrong_schema.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argv_schema_invalid"
            })
        ));

        let (mut wrong_executable, _directory) = valid_windows_test_spec();
        wrong_executable.executable = PathBuf::from("relative-probe.exe");
        assert!(matches!(
            wrong_executable.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_executable_invalid"
            })
        ));

        let (mut executable_is_directory, directory) = valid_windows_test_spec();
        executable_is_directory.executable = directory.canonical_path.clone();
        assert!(matches!(
            executable_is_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_executable_invalid"
            })
        ));

        let (mut wrong_working_directory, _directory) = valid_windows_test_spec();
        wrong_working_directory.working_directory = PathBuf::from("relative-working-directory");
        assert!(matches!(
            wrong_working_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_working_directory_invalid"
            })
        ));

        let (mut verbatim_working_directory, directory) = valid_windows_test_spec();
        verbatim_working_directory.working_directory = directory.canonical_path.clone();
        assert!(matches!(
            verbatim_working_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_working_directory_invalid"
            })
        ));

        let (mut parent_alias_working_directory, _directory) = valid_windows_test_spec();
        parent_alias_working_directory.working_directory = parent_alias_working_directory
            .working_directory
            .join("temp")
            .join("..");
        assert!(matches!(
            parent_alias_working_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_working_directory_invalid"
            })
        ));

        let (mut working_directory_is_file, _directory) = valid_windows_test_spec();
        working_directory_is_file.working_directory = working_directory_is_file.executable.clone();
        assert!(matches!(
            working_directory_is_file.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_working_directory_invalid"
            })
        ));

        let (mut wrong_environment_order, _directory) = valid_windows_test_spec();
        wrong_environment_order.environment.swap(0, 1);
        assert!(matches!(
            wrong_environment_order.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_invalid"
            })
        ));

        let (mut wrong_environment_path, _directory) = valid_windows_test_spec();
        wrong_environment_path.environment =
            fixed_test_environment(Path::new("relative-temp-directory"));
        assert!(matches!(
            wrong_environment_path.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_invalid"
            })
        ));

        let (mut temp_equals_working_directory, _directory) = valid_windows_test_spec();
        temp_equals_working_directory.environment =
            fixed_test_environment(temp_equals_working_directory.working_directory.as_path());
        assert!(matches!(
            temp_equals_working_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_invalid"
            })
        ));

        let (mut temp_outside_working_directory, directory) = valid_windows_test_spec();
        temp_outside_working_directory.environment =
            fixed_test_environment(directory.canonical_base.as_path());
        assert!(matches!(
            temp_outside_working_directory.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_invalid"
            })
        ));

        let (mut environment_values_differ, _directory) = valid_windows_test_spec();
        let different = fixed_test_environment(environment_values_differ.working_directory());
        environment_values_differ.environment[2] = different[2].clone();
        assert!(matches!(
            environment_values_differ.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_invalid"
            })
        ));

        let (mut inherited_environment, _directory) = valid_windows_test_spec();
        inherited_environment.inherit_environment = true;
        assert!(matches!(
            inherited_environment.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_environment_inheritance_invalid"
            })
        ));

        let (mut excessive_arguments, _directory) = valid_windows_test_spec();
        excessive_arguments.arguments = vec![OsString::from("x"); MAX_PROCESS_ARGUMENTS + 1];
        assert!(matches!(
            excessive_arguments.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_count_exceeded"
            })
        ));

        let (mut oversized_argument, _directory) = valid_windows_test_spec();
        oversized_argument.arguments = vec![OsString::from(
            "x".repeat(MAX_PROCESS_ARGUMENT_NATIVE_UNITS + 1),
        )];
        assert!(matches!(
            oversized_argument.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_too_long"
            })
        ));

        let (mut nul_argument, _directory) = valid_windows_test_spec();
        nul_argument.arguments = vec![OsString::from("bad\0argument")];
        assert!(matches!(
            nul_argument.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_invalid"
            })
        ));

        let (mut excessive_command_line, _directory) = valid_windows_test_spec();
        excessive_command_line.arguments =
            vec![OsString::from("x".repeat(MAX_PROCESS_ARGUMENT_NATIVE_UNITS)); 8];
        assert!(matches!(
            excessive_command_line.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_command_line_too_long"
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_java_revalidation_rejects_non_deterministic_typed_path_renderings() {
        let (mut verbatim_home, _directory, canonical_home) = valid_windows_java_test_spec();
        verbatim_home.arguments[0] = prefixed_path("-Duser.home=", &canonical_home);
        assert!(matches!(
            verbatim_home.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        let (mut parent_alias_home, _directory, _canonical_home) = valid_windows_java_test_spec();
        let aliased_home = parent_alias_home
            .working_directory
            .join("temp")
            .join("..")
            .join("microemu-home");
        parent_alias_home.arguments[0] = prefixed_path("-Duser.home=", &aliased_home);
        assert!(matches!(
            parent_alias_home.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        let (mut wrong_case_home, _directory, _canonical_home) = valid_windows_java_test_spec();
        let home = prefixed_argument_path(&wrong_case_home.arguments[0], "-Duser.home=").unwrap();
        let mut units = home.as_os_str().encode_wide().collect::<Vec<_>>();
        units[0] = if (b'A' as u16..=b'Z' as u16).contains(&units[0]) {
            units[0] + u16::from(b'a' - b'A')
        } else {
            units[0] - u16::from(b'a' - b'A')
        };
        wrong_case_home.arguments[0] =
            prefixed_path("-Duser.home=", Path::new(&OsString::from_wide(&units)));
        assert!(matches!(
            wrong_case_home.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        let (mut verbatim_classpath, _directory, _canonical_home) = valid_windows_java_test_spec();
        let workspace = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap(),
        )
        .unwrap();
        let mut classpath = OsString::from(workspace.join("Cargo.toml").as_os_str());
        classpath.push(";");
        classpath.push(workspace.join("crates/zeus-core/Cargo.toml"));
        verbatim_classpath.arguments[java_argv::CLASSPATH] = classpath;
        assert!(matches!(
            verbatim_classpath.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_classpath_path_invalid"
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_java_revalidation_pins_the_player_snapshot_inside_the_private_home() {
        // A redirected snapshot argument would publish the character outside the profile, where a
        // sibling session or any other program could read it or replace it with values this tool would
        // then render as fact.
        let (mut escaped_home, directory, _canonical_home) = valid_windows_java_test_spec();
        escaped_home.arguments[java_argv::PLAYER_SNAPSHOT] = invocation_prefixed_child_path(
            PLAYER_SNAPSHOT_PROPERTY,
            &directory.canonical_path,
            OsStr::new(crate::player::SNAPSHOT_FILE_NAME),
        )
        .unwrap();
        assert!(matches!(
            escaped_home.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        let (mut wrong_file_name, _directory, canonical_home) = valid_windows_java_test_spec();
        wrong_file_name.arguments[java_argv::PLAYER_SNAPSHOT] = invocation_prefixed_child_path(
            PLAYER_SNAPSHOT_PROPERTY,
            &canonical_home,
            OsStr::new("not-the-snapshot.txt"),
        )
        .unwrap();
        assert!(matches!(
            wrong_file_name.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        // A dropped property would silently disable publishing, so the slot must still hold it.
        let (mut replaced_property, _directory, _canonical_home) = valid_windows_java_test_spec();
        replaced_property.arguments[java_argv::PLAYER_SNAPSHOT] =
            OsString::from("-Dunrelated.property=1");
        assert!(matches!(
            replaced_property.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        // The settings argument is pinned the same way, and for a stronger reason: it carries the
        // monster spot, so a redirected file would let another process steer the character.
        let (mut escaped_settings, directory, _canonical_home) = valid_windows_java_test_spec();
        escaped_settings.arguments[java_argv::CONTROL_SETTINGS] = invocation_prefixed_child_path(
            CONTROL_SETTINGS_PROPERTY,
            &directory.canonical_path,
            OsStr::new(crate::control::CONTROL_FILE_NAME),
        )
        .unwrap();
        assert!(matches!(
            escaped_settings.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));

        let (mut swapped_settings, _directory, canonical_home) = valid_windows_java_test_spec();
        swapped_settings.arguments[java_argv::CONTROL_SETTINGS] = invocation_prefixed_child_path(
            CONTROL_SETTINGS_PROPERTY,
            &canonical_home,
            OsStr::new(crate::player::SNAPSHOT_FILE_NAME),
        )
        .unwrap();
        assert!(matches!(
            swapped_settings.revalidate_for_adapter(),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_path_invalid"
            })
        ));
    }
}

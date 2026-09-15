use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output};
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ,
    TerminateProcess, WaitForSingleObject,
};

use crate::data_root::DataRoot;

use super::{ProcessBirthId, wait_millis};

pub(crate) const PROBE_TEST_ROOT: &str = "zeus-hso-wcpv1-tests";
pub(crate) const REPORT_RECORD_TYPE: u32 = 1;
pub(crate) const SLEEP_RECORD_TYPE: u32 = 2;
pub(crate) const DESCENDANT_RECORD_TYPE: u32 = 3;
pub(crate) const REPORT_NUMERIC_FIELD_COUNT: usize = 8;
pub(crate) const SLEEP_NUMERIC_FIELD_COUNT: usize = 2;
pub(crate) const DESCENDANT_NUMERIC_FIELD_COUNT: usize = 4;
pub(crate) const MAX_PROBE_RECORD_BYTES: usize = 1_048_576;
pub(crate) const MAX_PROBE_RECORD_ITEMS: usize = 128;
pub(crate) const MAX_PROBE_STRING_UNITS: usize = 32_767;
pub(crate) const LIVE_OWNER_RECORD_BYTES: usize = 24;
const LIVE_OWNER_MAGIC: [u8; 8] = *b"ZHSLIVE1";
const LIVE_OWNER_SCHEMA_VERSION: u32 = 1;

pub(crate) fn live_owner_ready_path(root: &Path) -> PathBuf {
    root.join("live-owner.ready")
}

fn live_owner_partial_path(root: &Path) -> PathBuf {
    root.join("live-owner.partial")
}

pub(crate) fn encode_live_owner_record(
    pid: u32,
    creation_time_100ns: u64,
) -> io::Result<[u8; LIVE_OWNER_RECORD_BYTES]> {
    if pid == 0 || creation_time_100ns == 0 {
        return Err(invalid_live_owner_record("live owner identity is zero"));
    }
    let mut bytes = [0u8; LIVE_OWNER_RECORD_BYTES];
    bytes[..8].copy_from_slice(&LIVE_OWNER_MAGIC);
    bytes[8..12].copy_from_slice(&LIVE_OWNER_SCHEMA_VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&pid.to_le_bytes());
    bytes[16..24].copy_from_slice(&creation_time_100ns.to_le_bytes());
    Ok(bytes)
}

pub(crate) fn decode_live_owner_record_bytes(bytes: &[u8]) -> io::Result<ProcessBirthId> {
    if bytes.len() != LIVE_OWNER_RECORD_BYTES {
        return Err(invalid_live_owner_record("live owner record size mismatch"));
    }
    if bytes[..8] != LIVE_OWNER_MAGIC {
        return Err(invalid_live_owner_record(
            "live owner record magic mismatch",
        ));
    }
    let schema = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| invalid_live_owner_record("live owner schema width"))?,
    );
    if schema != LIVE_OWNER_SCHEMA_VERSION {
        return Err(invalid_live_owner_record("live owner schema mismatch"));
    }
    let pid = u32::from_le_bytes(
        bytes[12..16]
            .try_into()
            .map_err(|_| invalid_live_owner_record("live owner pid width"))?,
    );
    let creation_time_100ns = u64::from_le_bytes(
        bytes[16..24]
            .try_into()
            .map_err(|_| invalid_live_owner_record("live owner creation width"))?,
    );
    if pid == 0 || creation_time_100ns == 0 {
        return Err(invalid_live_owner_record("live owner identity is zero"));
    }
    Ok(ProcessBirthId::for_test(pid, creation_time_100ns))
}

pub(crate) fn publish_live_owner_record(
    root: &Path,
    pid: u32,
    creation_time_100ns: u64,
) -> io::Result<()> {
    let bytes = encode_live_owner_record(pid, creation_time_100ns)?;
    let partial = live_owner_partial_path(root);
    let ready = live_owner_ready_path(root);
    if ready.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "live owner ready record already exists",
        ));
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(partial, ready)
}

pub(crate) fn read_live_owner_record(root: &Path) -> io::Result<ProcessBirthId> {
    let ready = live_owner_ready_path(root);
    let mut file = fs::File::open(ready)?;
    if file.metadata()?.len() != LIVE_OWNER_RECORD_BYTES as u64 {
        return Err(invalid_live_owner_record("live owner record size mismatch"));
    }
    let mut bytes = [0u8; LIVE_OWNER_RECORD_BYTES];
    file.read_exact(&mut bytes)?;
    decode_live_owner_record_bytes(&bytes)
}

fn invalid_live_owner_record(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) struct ProbeTestDirectory {
    pub(crate) canonical_base: PathBuf,
    pub(crate) canonical_path: PathBuf,
    pub(crate) leaf: String,
    pub(crate) temp_directory: PathBuf,
}

impl ProbeTestDirectory {
    pub(crate) fn create() -> io::Result<Self> {
        let base = std::env::temp_dir().join(PROBE_TEST_ROOT);
        fs::create_dir_all(&base)?;
        let canonical_base = fs::canonicalize(&base)?;
        let leaf = Uuid::new_v4().to_string();
        let path = canonical_base.join(&leaf);
        let data_root = DataRoot::prepare_at(&path)
            .map_err(|error| io::Error::other(format!("create private test root: {error}")))?;
        let canonical_path = fs::canonicalize(data_root.path())?;
        let mut directory = Self {
            canonical_base,
            canonical_path,
            leaf,
            temp_directory: PathBuf::new(),
        };
        if directory.canonical_path.parent() != Some(directory.canonical_base.as_path())
            || directory.canonical_path.file_name() != Some(OsStr::new(&directory.leaf))
        {
            return Err(io::Error::other("probe test directory escaped its base"));
        }
        let temp_directory = data_root
            .ensure_private_child_directory("temp")
            .map_err(|error| io::Error::other(format!("create private test temp: {error}")))?;
        if !data_root
            .is_private()
            .map_err(|error| io::Error::other(format!("validate private test root: {error}")))?
        {
            return Err(io::Error::other("probe test root is not private"));
        }
        let temp_directory = fs::canonicalize(temp_directory)?;
        if temp_directory.parent() != Some(directory.canonical_path.as_path()) {
            return Err(io::Error::other("probe test directory escaped its base"));
        }
        directory.temp_directory = temp_directory;
        Ok(directory)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.canonical_path
    }

    pub(crate) fn temp_path(&self) -> &Path {
        &self.temp_directory
    }
}

impl Drop for ProbeTestDirectory {
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

pub(crate) struct CompiledProbe {
    pub(crate) _directory: ProbeTestDirectory,
    pub(crate) executable: PathBuf,
    pub(crate) compiler: PathBuf,
    pub(crate) linker: PathBuf,
    pub(crate) version: String,
}

pub(crate) struct ProbeChild {
    pub(crate) child: Option<Child>,
    pub(crate) deadline: Instant,
}

pub(crate) struct ProcessObservation {
    pub(crate) handle: OwnedHandle,
    pub(crate) pid: u32,
    pub(crate) creation_time_100ns: u64,
}

impl ProcessObservation {
    pub(crate) fn birth_id(&self) -> ProcessBirthId {
        ProcessBirthId::for_test(self.pid, self.creation_time_100ns)
    }

    pub(crate) fn raw_handle(&self) -> HANDLE {
        self.handle.as_raw_handle() as HANDLE
    }

    pub(crate) fn is_signaled(&self) -> io::Result<bool> {
        // SAFETY: `self` retains sole ownership of a process handle opened with synchronization
        // access for the complete zero-time wait; ownership is not transferred.
        match unsafe { WaitForSingleObject(self.raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other("unexpected zero-time process wait result")),
        }
    }
}

impl ProbeChild {
    pub(crate) fn spawn(command: &mut Command, deadline: Instant) -> io::Result<Self> {
        command.spawn().map(|child| Self {
            child: Some(child),
            deadline,
        })
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("probe child should still be owned")
    }

    pub(crate) fn wait_until_exit(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.child_mut().try_wait()? {
                self.child.take();
                return Ok(status);
            }
            let now = Instant::now();
            if now >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "probe child did not exit before the absolute deadline",
                ));
            }
            thread::sleep((self.deadline - now).min(Duration::from_millis(10)));
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        loop {
            let now = Instant::now();
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if now < self.deadline => {
                    thread::sleep((self.deadline - now).min(Duration::from_millis(10)));
                }
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[derive(Debug)]
pub(crate) struct DecodedProbeRecord {
    pub(crate) magic: [u8; 8],
    pub(crate) schema_version: u32,
    pub(crate) record_type: u32,
    pub(crate) strings: Vec<OsString>,
    pub(crate) numeric_fields: Vec<u64>,
}

pub(crate) fn canonical_workspace_root() -> PathBuf {
    let manifest = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .expect("zeus-core manifest directory should canonicalize");
    fs::canonicalize(
        manifest
            .parent()
            .and_then(Path::parent)
            .expect("workspace root should be two levels above zeus-core"),
    )
    .expect("workspace root should canonicalize")
}

pub(crate) fn canonical_invocation_path(canonical: &Path) -> PathBuf {
    const VERBATIM_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];

    let units: Vec<u16> = canonical.as_os_str().encode_wide().collect();
    let invocation = if units.starts_with(&VERBATIM_PREFIX) {
        PathBuf::from(OsString::from_wide(&units[VERBATIM_PREFIX.len()..]))
    } else {
        canonical.to_owned()
    };
    assert!(invocation.is_absolute());
    assert_eq!(
        fs::canonicalize(&invocation).expect("tool invocation path should canonicalize"),
        canonical
    );
    invocation
}

pub(crate) fn compile_probe() -> CompiledProbe {
    let workspace = canonical_workspace_root();
    let toolchain = fs::canonicalize(
        workspace.join(".devtools/rustup/toolchains/1.98.0-x86_64-pc-windows-gnu"),
    )
    .expect("pinned Windows GNU toolchain should exist");
    let canonical_compiler =
        fs::canonicalize(toolchain.join("bin/rustc.exe")).expect("pinned rustc.exe should exist");
    let canonical_linker =
        fs::canonicalize(toolchain.join("lib/rustlib/x86_64-pc-windows-gnu/bin/rust-lld.exe"))
            .expect("pinned rust-lld.exe should exist");
    assert!(canonical_compiler.starts_with(&toolchain));
    assert!(canonical_linker.starts_with(&toolchain));
    let compiler = canonical_invocation_path(&canonical_compiler);
    let linker = canonical_invocation_path(&canonical_linker);

    let version_output = Command::new(&compiler)
        .arg("-vV")
        .output()
        .expect("absolute pinned rustc should run");
    assert_command_succeeded(&version_output, "rustc -vV");
    let version = String::from_utf8(version_output.stdout)
        .expect("pinned rustc version output should be UTF-8");
    assert!(version.lines().any(|line| line == "release: 1.98.0"));
    assert!(
        version
            .lines()
            .any(|line| line == "host: x86_64-pc-windows-gnu")
    );

    let source =
        fs::canonicalize(workspace.join("crates/zeus-core/tests/support/windows_process_probe.rs"))
            .expect("Windows process probe source should exist");
    let directory = ProbeTestDirectory::create().expect("probe test directory should be created");
    let executable = directory.path().join("windows process probe.exe");
    let compile_output = Command::new(&compiler)
        .arg(&source)
        .arg("--edition=2024")
        .arg("-C")
        .arg(format!("linker={}", linker.display()))
        .arg("-C")
        .arg("linker-flavor=ld.lld")
        .arg("-o")
        .arg(&executable)
        .current_dir(&workspace)
        .output()
        .expect("absolute pinned rustc should compile the probe");
    assert_command_succeeded(&compile_output, "compile Windows process probe");
    let executable =
        fs::canonicalize(executable).expect("compiled Windows process probe should canonicalize");

    CompiledProbe {
        _directory: directory,
        executable,
        compiler,
        linker,
        version,
    }
}

pub(crate) fn assert_command_succeeded(output: &Output, stage: &str) {
    assert!(
        output.status.success(),
        "{stage} failed: {}",
        String::from_utf8(output.stderr.clone())
            .unwrap_or_else(|_| "<non-UTF-8 compiler diagnostic>".to_owned())
    );
}

pub(crate) fn inheritable_sentinel_event() -> OwnedHandle {
    let security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: `security_attributes` is fully initialized and live for the call, the event is
    // unnamed, and the returned non-null handle is immediately transferred to one owner.
    let raw = unsafe { CreateEventW(&security_attributes, 1, 0, std::ptr::null()) };
    assert!(
        !raw.is_null(),
        "CreateEventW failed: {}",
        io::Error::last_os_error()
    );
    // SAFETY: CreateEventW returned a fresh non-null owned handle, and this is its sole adoption.
    unsafe { OwnedHandle::from_raw_handle(raw) }
}

pub(crate) fn wait_for_probe_ready(
    child: &mut Child,
    ready: &Path,
    deadline: Instant,
) -> io::Result<()> {
    loop {
        if ready.is_file() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "probe exited with {status} before publishing readiness"
            )));
        }
        let now = Instant::now();
        if now >= deadline {
            if ready.is_file() {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "probe readiness did not appear before the absolute deadline",
            ));
        }
        thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}

pub(crate) fn wait_for_ready_path(ready: &Path, deadline: Instant) -> io::Result<()> {
    loop {
        if ready.is_file() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            if ready.is_file() {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "probe readiness did not appear before the absolute deadline",
            ));
        }
        thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}

pub(crate) fn open_identity_checked_process(
    pid: u32,
    creation_time_100ns: u64,
) -> ProcessObservation {
    open_identity_checked_process_with_access(
        pid,
        creation_time_100ns,
        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
    )
}

pub(crate) fn open_identity_checked_process_for_termination(
    pid: u32,
    creation_time_100ns: u64,
) -> ProcessObservation {
    open_identity_checked_process_with_access(
        pid,
        creation_time_100ns,
        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
    )
}

pub(crate) fn open_identity_checked_process_for_termination_result(
    pid: u32,
    creation_time_100ns: u64,
) -> io::Result<ProcessObservation> {
    open_identity_checked_process_with_access_result(
        pid,
        creation_time_100ns,
        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
    )
}

pub(crate) fn child_process_birth_id(child: &Child) -> io::Result<ProcessBirthId> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: Child retains ownership of a live process handle and every FILETIME is writable for
    // the complete query; ownership of the handle is not transferred.
    if unsafe {
        GetProcessTimes(
            child.as_raw_handle() as HANDLE,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let creation_time_100ns =
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    if child.id() == 0 || creation_time_100ns == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "child process identity is zero",
        ));
    }
    Ok(ProcessBirthId::for_test(child.id(), creation_time_100ns))
}

pub(crate) fn terminate_process_observation(
    observation: &ProcessObservation,
    exit_code: u32,
) -> io::Result<()> {
    // SAFETY: observation owns an identity-checked handle opened with PROCESS_TERMINATE; only that
    // retained handle is terminated and ownership is not transferred.
    if unsafe { TerminateProcess(observation.raw_handle(), exit_code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn open_identity_checked_process_for_metrics(
    pid: u32,
    creation_time_100ns: u64,
) -> io::Result<ProcessObservation> {
    open_identity_checked_process_with_access_result(
        pid,
        creation_time_100ns,
        PROCESS_SYNCHRONIZE | PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
    )
}

fn open_identity_checked_process_with_access_result(
    pid: u32,
    creation_time_100ns: u64,
    access: u32,
) -> io::Result<ProcessObservation> {
    // SAFETY: the access mask is fixed by the metrics caller, handle inheritance is disabled,
    // and a successful non-null result is transferred into exactly one OwnedHandle below.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcess returned a fresh non-null owned handle, and this is its sole adoption.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let observed_creation_time = process_creation_time_result(&handle)?;
    if observed_creation_time != creation_time_100ns {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process birth identity mismatch",
        ));
    }
    Ok(ProcessObservation {
        handle,
        pid,
        creation_time_100ns,
    })
}

fn open_identity_checked_process_with_access(
    pid: u32,
    creation_time_100ns: u64,
    access: u32,
) -> ProcessObservation {
    // SAFETY: the requested access mask is bounded by the two test-only callers;
    // no handle is inherited, and a successful non-null result is adopted exactly once below.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    assert!(
        !raw.is_null(),
        "OpenProcess failed for pid={pid}: {}",
        io::Error::last_os_error()
    );
    // SAFETY: OpenProcess returned a fresh non-null owned handle, and this is its sole adoption.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let observed_creation_time = process_creation_time(&handle);
    assert_eq!(
        observed_creation_time, creation_time_100ns,
        "published process birth identity must match the opened process before its handle is evidence"
    );
    ProcessObservation {
        handle,
        pid,
        creation_time_100ns,
    }
}

pub(crate) fn process_creation_time(process: &OwnedHandle) -> u64 {
    process_creation_time_result(process)
        .unwrap_or_else(|error| panic!("GetProcessTimes failed: {error}"))
}

fn process_creation_time_result(process: &OwnedHandle) -> io::Result<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `process` owns a live handle opened with process-query access; all four FILETIME
    // values are correctly sized writable storage and remain live for the complete call.
    let queried = unsafe {
        GetProcessTimes(
            process.as_raw_handle() as HANDLE,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if queried == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

pub(crate) fn wait_for_process_observation(observation: &ProcessObservation, deadline: Instant) {
    let timeout = wait_millis(deadline, Instant::now());
    // SAFETY: `observation` keeps the process handle valid for the complete finite wait; the
    // handle was opened with synchronization access and ownership is not transferred.
    let result =
        unsafe { WaitForSingleObject(observation.handle.as_raw_handle() as HANDLE, timeout) };
    assert_eq!(
        result, WAIT_OBJECT_0,
        "process observation did not signal before deadline: pid={}",
        observation.pid
    );
}

pub(crate) fn process_observation_exit_code(observation: &ProcessObservation) -> u32 {
    let mut exit_code = 0u32;
    // SAFETY: `observation` keeps the signaled process handle valid with process-query access;
    // `exit_code` is correctly sized writable storage live for the call.
    let queried = unsafe {
        windows_sys::Win32::System::Threading::GetExitCodeProcess(
            observation.handle.as_raw_handle() as HANDLE,
            &mut exit_code,
        )
    };
    assert_ne!(
        queried,
        0,
        "GetExitCodeProcess failed for pid={}: {}",
        observation.pid,
        io::Error::last_os_error()
    );
    exit_code
}

pub(crate) fn decode_probe_record(path: &Path) -> io::Result<DecodedProbeRecord> {
    let file = fs::File::open(path)?;
    let byte_count = usize::try_from(file.metadata()?.len())
        .map_err(|_| invalid_probe_record("probe record size does not fit usize"))?;
    if byte_count > MAX_PROBE_RECORD_BYTES {
        return Err(invalid_probe_record("probe record exceeds hard byte bound"));
    }
    let mut bytes = Vec::with_capacity(byte_count);
    file.take((MAX_PROBE_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PROBE_RECORD_BYTES {
        return Err(invalid_probe_record("probe record exceeds hard byte bound"));
    }
    decode_probe_bytes(&bytes)
}

pub(crate) fn decode_probe_bytes(bytes: &[u8]) -> io::Result<DecodedProbeRecord> {
    if bytes.len() > MAX_PROBE_RECORD_BYTES {
        return Err(invalid_probe_record("probe record exceeds hard byte bound"));
    }
    let mut offset = 0usize;
    let magic = take_array::<8>(bytes, &mut offset)?;
    if magic != *b"ZHSOPRB1" {
        return Err(invalid_probe_record("probe record magic mismatch"));
    }
    let schema_version = u32::from_le_bytes(take_array::<4>(bytes, &mut offset)?);
    if schema_version != 1 {
        return Err(invalid_probe_record("probe record schema mismatch"));
    }
    let record_type = u32::from_le_bytes(take_array::<4>(bytes, &mut offset)?);
    let numeric_field_count = match record_type {
        REPORT_RECORD_TYPE => REPORT_NUMERIC_FIELD_COUNT,
        SLEEP_RECORD_TYPE => SLEEP_NUMERIC_FIELD_COUNT,
        DESCENDANT_RECORD_TYPE => DESCENDANT_NUMERIC_FIELD_COUNT,
        _ => return Err(invalid_probe_record("probe record type is unsupported")),
    };
    let item_count = u32::from_le_bytes(take_array::<4>(bytes, &mut offset)?) as usize;
    if item_count > MAX_PROBE_RECORD_ITEMS || (record_type != REPORT_RECORD_TYPE && item_count != 0)
    {
        return Err(invalid_probe_record("probe record item count is invalid"));
    }
    let mut strings = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        let unit_count = u32::from_le_bytes(take_array::<4>(bytes, &mut offset)?) as usize;
        if unit_count > MAX_PROBE_STRING_UNITS {
            return Err(invalid_probe_record("probe string exceeds hard unit bound"));
        }
        let byte_count = unit_count
            .checked_mul(size_of::<u16>())
            .ok_or_else(|| invalid_probe_record("probe string size overflow"))?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| invalid_probe_record("probe string offset overflow"))?;
        let encoded = bytes
            .get(offset..end)
            .ok_or_else(|| invalid_probe_record("probe string is truncated"))?;
        let mut units = Vec::with_capacity(unit_count);
        let (encoded_units, remainder) = encoded.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(invalid_probe_record("probe string unit width is invalid"));
        }
        for encoded_unit in encoded_units {
            units.push(u16::from_le_bytes(*encoded_unit));
        }
        offset = end;
        strings.push(OsString::from_wide(&units));
    }
    let numeric_bytes = numeric_field_count
        .checked_mul(size_of::<u64>())
        .ok_or_else(|| invalid_probe_record("probe numeric size overflow"))?;
    if bytes.len().checked_sub(offset) != Some(numeric_bytes) {
        return Err(invalid_probe_record("probe record size is invalid"));
    }
    let mut numeric_fields = Vec::with_capacity(numeric_field_count);
    for _ in 0..numeric_field_count {
        numeric_fields.push(u64::from_le_bytes(take_array::<8>(bytes, &mut offset)?));
    }
    if record_type == REPORT_RECORD_TYPE
        && numeric_fields[0].checked_add(numeric_fields[1]) != u64::try_from(strings.len()).ok()
    {
        return Err(invalid_probe_record(
            "probe report counts do not match items",
        ));
    }
    Ok(DecodedProbeRecord {
        magic,
        schema_version,
        record_type,
        strings,
        numeric_fields,
    })
}

pub(crate) fn take_array<const N: usize>(bytes: &[u8], offset: &mut usize) -> io::Result<[u8; N]> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| invalid_probe_record("probe record offset overflow"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| invalid_probe_record("probe record is truncated"))?
        .try_into()
        .map_err(|_| invalid_probe_record("probe field width is invalid"))?;
    *offset = end;
    Ok(value)
}

pub(crate) fn invalid_probe_record(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

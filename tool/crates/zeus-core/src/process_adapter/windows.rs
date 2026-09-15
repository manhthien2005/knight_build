use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::time::{Duration, Instant};

#[cfg(test)]
use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
use windows_sys::Win32::Foundation::{
    FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectAssociateCompletionPortInformation,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
#[cfg(test)]
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessTimes, INFINITE,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::process_launch_spec::windows::{encode_application_name, encode_command_line};
use crate::{LaunchEnvironmentKey, LaunchEnvironmentVariable, ProcessLaunchSpec};

const COMPLETION_KEY: usize = 1;
const STARTUP_FAILURE_EXIT_CODE: u32 = 0xffff_fffe;
const HARD_STOP_EXIT_CODE: u32 = 0xffff_fffd;
const HARD_STOP_QUERY_CADENCE: Duration = Duration::from_millis(50);
const NUL_NAME: [u16; 4] = [b'N' as u16, b'U' as u16, b'L' as u16, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessBirthId {
    pid: u32,
    creation_time_100ns: u64,
}

#[allow(
    dead_code,
    reason = "numeric birth diagnostics are exercised by crate-private live tests"
)]
impl ProcessBirthId {
    pub(crate) fn pid(self) -> u32 {
        self.pid
    }

    pub(crate) fn creation_time_100ns(self) -> u64 {
        self.creation_time_100ns
    }

    #[cfg(test)]
    pub(crate) fn for_test(pid: u32, creation_time_100ns: u64) -> Self {
        Self {
            pid,
            creation_time_100ns,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RootExit {
    identity: ProcessBirthId,
    exit_code: u32,
}

impl RootExit {
    pub(crate) fn identity(self) -> ProcessBirthId {
        self.identity
    }

    pub(crate) fn exit_code(self) -> u32 {
        self.exit_code
    }

    #[cfg(test)]
    pub(crate) fn for_test(identity: ProcessBirthId, exit_code: u32) -> Self {
        Self {
            identity,
            exit_code,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsProcessStage {
    ValidateSpec,
    EncodeApplication,
    EncodeCommandLine,
    EncodeEnvironment,
    CreateCompletionPort,
    CreateJob,
    ConfigureJob,
    AssociateCompletionPort,
    OpenNullStdin,
    OpenNullStdout,
    OpenNullStderr,
    InitializeAttributeList,
    CreateProcess,
    ReadBirthIdentity,
    AssignJob,
    ResumeThread,
    TerminateRoot,
    TerminateJob,
    WaitRoot,
    QueryJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsProcessErrorKind {
    LaunchFailed,
    DeadlineExpired,
    QueryFailed,
    CleanupUnconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowsProcessError {
    kind: WindowsProcessErrorKind,
    stage: WindowsProcessStage,
    os_code: Option<u32>,
    cleanup_stage: Option<WindowsProcessStage>,
    cleanup_os_code: Option<u32>,
}

#[allow(
    dead_code,
    reason = "bounded diagnostic fields remain available to crate-private lifecycle callers"
)]
impl WindowsProcessError {
    pub(crate) fn kind(self) -> WindowsProcessErrorKind {
        self.kind
    }

    pub(crate) fn stage(self) -> WindowsProcessStage {
        self.stage
    }

    pub(crate) fn os_code(self) -> Option<u32> {
        self.os_code
    }

    pub(crate) fn cleanup_stage(self) -> Option<WindowsProcessStage> {
        self.cleanup_stage
    }

    pub(crate) fn cleanup_os_code(self) -> Option<u32> {
        self.cleanup_os_code
    }
}

impl fmt::Display for WindowsProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} at {:?} (os_code={:?}, cleanup_stage={:?}, cleanup_os_code={:?})",
            self.kind, self.stage, self.os_code, self.cleanup_stage, self.cleanup_os_code
        )
    }
}

impl std::error::Error for WindowsProcessError {}

#[derive(Debug)]
pub(crate) enum WindowsSpawnFailure {
    Rejected(WindowsProcessError),
    CleanupUnconfirmed {
        error: WindowsProcessError,
        owner: UnconfirmedWindowsProcess,
    },
}

impl fmt::Display for WindowsSpawnFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => write!(formatter, "contained process rejected: {error}"),
            Self::CleanupUnconfirmed { error, .. } => {
                write!(formatter, "contained process cleanup unconfirmed: {error}")
            }
        }
    }
}

impl std::error::Error for WindowsSpawnFailure {}

impl From<WindowsProcessError> for WindowsSpawnFailure {
    fn from(error: WindowsProcessError) -> Self {
        Self::Rejected(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Membership {
    PreAssignment,
    Contained,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum TestFailurePoint {
    BeforeAssign,
    AfterAssignBeforeResume,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TestTerminationRequest {
    Root,
    Job,
}

struct CompletionPortHandle(OwnedHandle);
struct JobHandle(OwnedHandle);
struct ProcessHandle(OwnedHandle);
struct ThreadHandle(OwnedHandle);
struct NullHandle(OwnedHandle);
#[cfg(test)]
struct ProcessObservationHandle(OwnedHandle);

macro_rules! raw_handle {
    ($type:ty) => {
        impl $type {
            fn raw(&self) -> HANDLE {
                self.0.as_raw_handle() as HANDLE
            }
        }
    };
}

raw_handle!(CompletionPortHandle);
raw_handle!(JobHandle);
raw_handle!(ProcessHandle);
raw_handle!(ThreadHandle);
raw_handle!(NullHandle);
#[cfg(test)]
raw_handle!(ProcessObservationHandle);

pub(crate) struct WindowsContainedProcess {
    completion_port: CompletionPortHandle,
    root: ProcessHandle,
    identity: ProcessBirthId,
    tree_empty_confirmed: bool,
    #[cfg(test)]
    last_active_processes: Option<u32>,
    // Rust drops fields in declaration order after `Drop::drop`; the sole Job owner is declared
    // last so completion/root ownership closes normally before kill-on-Job-close is released.
    job: JobHandle,
}

pub(crate) struct UnconfirmedWindowsProcess {
    membership: Membership,
    root: Option<ProcessHandle>,
    thread: Option<ThreadHandle>,
    completion_port: Option<CompletionPortHandle>,
    job: Option<JobHandle>,
    confirmed: bool,
}

impl fmt::Debug for UnconfirmedWindowsProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnconfirmedWindowsProcess")
            .field("membership", &self.membership)
            .field("has_root", &self.root.is_some())
            .field("has_thread", &self.thread.is_some())
            .field("has_completion_port", &self.completion_port.is_some())
            .field("has_job", &self.job.is_some())
            .field("confirmed", &self.confirmed)
            .finish()
    }
}

impl UnconfirmedWindowsProcess {
    fn process(&self) -> &ProcessHandle {
        self.root
            .as_ref()
            .expect("created process owner must contain the root")
    }

    fn thread(&self) -> &ThreadHandle {
        self.thread
            .as_ref()
            .expect("created process owner must contain the primary thread")
    }

    fn job(&self) -> &JobHandle {
        self.job
            .as_ref()
            .expect("created process owner must contain the configured Job")
    }

    pub(crate) fn retry_cleanup(&mut self, deadline: Instant) -> Result<(), WindowsProcessError> {
        self.cleanup(deadline, None, false)
    }

    fn cleanup(
        &mut self,
        deadline: Instant,
        #[cfg(test)] trace: Option<&mut TestStartupTrace>,
        #[cfg(not(test))] _trace: Option<&mut ()>,
        suppress_confirmation: bool,
    ) -> Result<(), WindowsProcessError> {
        if self.confirmed {
            return Ok(());
        }
        let result = match self.membership {
            Membership::PreAssignment => self.cleanup_pre_assignment(
                deadline,
                #[cfg(test)]
                trace,
                suppress_confirmation,
            ),
            Membership::Contained => self.cleanup_contained(
                deadline,
                #[cfg(test)]
                trace,
                suppress_confirmation,
            ),
        };
        if result.is_ok() {
            self.release_confirmed();
        }
        result
    }

    fn cleanup_pre_assignment(
        &mut self,
        deadline: Instant,
        #[cfg(test)] mut trace: Option<&mut TestStartupTrace>,
        suppress_confirmation: bool,
    ) -> Result<(), WindowsProcessError> {
        let Some(root) = self.root.as_ref() else {
            return Err(cleanup_error(WindowsProcessStage::TerminateRoot, None));
        };
        // SAFETY: the semantic root wrapper owns a valid process handle. TerminateProcess does
        // not transfer ownership; the handle remains owned for the subsequent observation.
        let terminated = unsafe { TerminateProcess(root.raw(), STARTUP_FAILURE_EXIT_CODE) };
        let terminate_os_code = (terminated == 0).then(last_os_code).flatten();
        #[cfg(test)]
        if let Some(trace) = trace.as_deref_mut() {
            trace.termination_request = Some(TestTerminationRequest::Root);
        }
        if suppress_confirmation {
            return Err(cleanup_error(
                WindowsProcessStage::TerminateRoot,
                terminate_os_code,
            ));
        }

        match wait_for_root_signal(root, deadline) {
            Ok(()) => {
                #[cfg(test)]
                if let Some(trace) = trace {
                    trace.root_signaled_before_return = true;
                }
                Ok(())
            }
            Err(observation_error) => Err(prefer_termination_error(
                WindowsProcessStage::TerminateRoot,
                terminate_os_code,
                observation_error,
            )),
        }
    }

    fn cleanup_contained(
        &mut self,
        deadline: Instant,
        #[cfg(test)] mut trace: Option<&mut TestStartupTrace>,
        suppress_confirmation: bool,
    ) -> Result<(), WindowsProcessError> {
        let Some(root) = self.root.as_ref() else {
            return Err(cleanup_error(WindowsProcessStage::WaitRoot, None));
        };
        let Some(job) = self.job.as_ref() else {
            return Err(cleanup_error(WindowsProcessStage::TerminateJob, None));
        };
        // SAFETY: the semantic Job wrapper owns a valid configured Job handle. The call only
        // requests termination and does not transfer or invalidate ownership.
        let terminated = unsafe { TerminateJobObject(job.raw(), STARTUP_FAILURE_EXIT_CODE) };
        let terminate_os_code = (terminated == 0).then(last_os_code).flatten();
        #[cfg(test)]
        if let Some(trace) = trace.as_deref_mut() {
            trace.termination_request = Some(TestTerminationRequest::Job);
        }
        if suppress_confirmation {
            return Err(cleanup_error(
                WindowsProcessStage::TerminateJob,
                terminate_os_code,
            ));
        }

        let root_observation = wait_for_root_signal(root, deadline);
        #[cfg(test)]
        if root_observation.is_ok()
            && let Some(trace) = trace.as_deref_mut()
        {
            trace.root_signaled_before_return = true;
        }
        let active_processes = query_active_processes(job)
            .map_err(|os_code| cleanup_error(WindowsProcessStage::QueryJob, os_code));
        #[cfg(test)]
        if let Ok(active_processes) = active_processes
            && let Some(trace) = trace
        {
            trace.active_processes_before_return = Some(active_processes);
        }

        match (root_observation, active_processes) {
            (Ok(()), Ok(0)) => Ok(()),
            (Err(error), _) => Err(prefer_termination_error(
                WindowsProcessStage::TerminateJob,
                terminate_os_code,
                error,
            )),
            (Ok(()), Err(error)) => Err(prefer_termination_error(
                WindowsProcessStage::TerminateJob,
                terminate_os_code,
                error,
            )),
            (Ok(()), Ok(_)) => Err(prefer_termination_error(
                WindowsProcessStage::TerminateJob,
                terminate_os_code,
                cleanup_error(WindowsProcessStage::QueryJob, None),
            )),
        }
    }

    fn release_confirmed(&mut self) {
        drop(self.thread.take());
        drop(self.root.take());
        drop(self.completion_port.take());
        drop(self.job.take());
        self.confirmed = true;
    }
}

impl Drop for UnconfirmedWindowsProcess {
    fn drop(&mut self) {
        if self.confirmed {
            return;
        }
        match self.membership {
            Membership::PreAssignment => {
                if let Some(root) = self.root.as_ref() {
                    // SAFETY: the root wrapper owns a valid process handle. Drop performs exactly
                    // one non-blocking best-effort termination request and keeps ownership.
                    unsafe {
                        TerminateProcess(root.raw(), STARTUP_FAILURE_EXIT_CODE);
                    }
                }
            }
            Membership::Contained => {
                if let Some(job) = self.job.as_ref() {
                    // SAFETY: the Job wrapper owns a valid Job handle. Drop performs exactly one
                    // non-blocking best-effort termination request and keeps ownership.
                    unsafe {
                        TerminateJobObject(job.raw(), STARTUP_FAILURE_EXIT_CODE);
                    }
                }
            }
        }
        drop(self.thread.take());
        drop(self.root.take());
        drop(self.completion_port.take());
        drop(self.job.take());
    }
}

#[cfg(test)]
pub(super) struct TestStartupTrace {
    root_observation: Option<ProcessObservationHandle>,
    termination_request: Option<TestTerminationRequest>,
    root_signaled_before_return: bool,
    active_processes_before_return: Option<u32>,
}

#[cfg(test)]
impl TestStartupTrace {
    fn new() -> Self {
        Self {
            root_observation: None,
            termination_request: None,
            root_signaled_before_return: false,
            active_processes_before_return: None,
        }
    }

    pub(super) fn root_observation_is_signaled(&self) -> bool {
        let observation = self
            .root_observation
            .as_ref()
            .expect("post-create test trace must own a duplicated root observation handle");
        // SAFETY: the observation wrapper owns a valid duplicated process handle for the entire
        // zero-time query; no ownership is transferred.
        unsafe { WaitForSingleObject(observation.raw(), 0) == WAIT_OBJECT_0 }
    }
}

struct SpawnControl<'a> {
    #[cfg(test)]
    failure_point: Option<TestFailurePoint>,
    #[cfg(test)]
    suppress_confirmation_once: bool,
    #[cfg(test)]
    trace: Option<&'a mut TestStartupTrace>,
    #[cfg(not(test))]
    _marker: std::marker::PhantomData<&'a ()>,
}

impl SpawnControl<'_> {
    fn production() -> Self {
        Self {
            #[cfg(test)]
            failure_point: None,
            #[cfg(test)]
            suppress_confirmation_once: false,
            #[cfg(test)]
            trace: None,
            #[cfg(not(test))]
            _marker: std::marker::PhantomData,
        }
    }
}

struct ProcThreadAttributeList {
    storage: Vec<usize>,
    initialized: bool,
}

impl ProcThreadAttributeList {
    fn with_handle_list(handles: &[HANDLE; 3]) -> Result<Self, WindowsProcessError> {
        let mut required_bytes = 0usize;
        // SAFETY: a null attribute-list pointer is required for the size query; `required_bytes`
        // is a valid writable usize and all other size-query arguments follow the API contract.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut required_bytes);
        }
        if required_bytes == 0 {
            return Err(last_error(WindowsProcessStage::InitializeAttributeList));
        }
        let word_count = required_bytes
            .checked_add(size_of::<usize>() - 1)
            .and_then(|bytes| bytes.checked_div(size_of::<usize>()))
            .ok_or_else(|| launch_error(WindowsProcessStage::InitializeAttributeList, None))?;
        let mut owner = Self {
            storage: vec![0usize; word_count],
            initialized: false,
        };
        let list = owner.storage.as_mut_ptr().cast::<c_void>();
        // SAFETY: `storage` is non-empty, aligned for usize, large enough for `required_bytes`,
        // and will not be reallocated while the initialized list exists. The size pointer is
        // valid and writable for the call.
        let initialized =
            unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut required_bytes) };
        if initialized == 0 {
            return Err(last_error(WindowsProcessStage::InitializeAttributeList));
        }
        owner.initialized = true;
        // SAFETY: `list` names the initialized stable storage owned by `owner`; `handles` is a
        // correctly sized contiguous array of three valid inheritable handles and stays live for
        // the call. No previous-value buffers are requested.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast::<c_void>(),
                size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(last_error(WindowsProcessStage::InitializeAttributeList));
        }
        Ok(owner)
    }

    fn raw(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast::<c_void>()
    }
}

impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        if !self.initialized {
            return;
        }
        // SAFETY: successful initialization set `initialized`; storage remains allocated,
        // aligned, stable, and uniquely borrowed until deletion returns, before Vec is freed.
        unsafe {
            DeleteProcThreadAttributeList(self.storage.as_mut_ptr().cast::<c_void>());
        }
    }
}

pub(crate) fn spawn(
    spec: ProcessLaunchSpec,
    deadline: Instant,
) -> Result<WindowsContainedProcess, WindowsSpawnFailure> {
    spawn_inner(spec, deadline, SpawnControl::production())
}

#[cfg(test)]
pub(super) fn spawn_for_test(
    spec: ProcessLaunchSpec,
    deadline: Instant,
    failure_point: TestFailurePoint,
    suppress_confirmation_once: bool,
) -> (
    Result<WindowsContainedProcess, WindowsSpawnFailure>,
    TestStartupTrace,
) {
    let mut trace = TestStartupTrace::new();
    let result = spawn_inner(
        spec,
        deadline,
        SpawnControl {
            failure_point: Some(failure_point),
            suppress_confirmation_once,
            trace: Some(&mut trace),
        },
    );
    (result, trace)
}

fn spawn_inner(
    spec: ProcessLaunchSpec,
    deadline: Instant,
    mut control: SpawnControl<'_>,
) -> Result<WindowsContainedProcess, WindowsSpawnFailure> {
    if spec.revalidate_for_adapter().is_err() {
        return Err(launch_error(WindowsProcessStage::ValidateSpec, None).into());
    }
    check_deadline(deadline, WindowsProcessStage::ValidateSpec)?;

    let application = encode_application_name(spec.executable())
        .map_err(|_| launch_error(WindowsProcessStage::EncodeApplication, None))?;
    let mut command_line = encode_command_line(spec.executable(), spec.arguments())
        .map_err(|_| launch_error(WindowsProcessStage::EncodeCommandLine, None))?;
    let environment = encode_environment_block(spec.environment())?;
    let mut working_directory: Vec<u16> =
        spec.working_directory().as_os_str().encode_wide().collect();
    working_directory.push(0);

    // SAFETY: INVALID_HANDLE_VALUE requests a new completion port; no existing port or
    // completion-key pointer is supplied, and the returned handle is checked and adopted once.
    let completion_port_raw =
        unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
    // SAFETY: the raw result has not been adopted; the helper rejects null and takes sole
    // ownership of a successful CreateIoCompletionPort result.
    let completion_port = CompletionPortHandle(unsafe {
        adopt_non_null_handle(
            completion_port_raw,
            WindowsProcessStage::CreateCompletionPort,
        )?
    });

    // SAFETY: null attributes and name request a fresh unnamed Job; the returned handle is
    // checked and immediately adopted exactly once.
    let job_raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    // SAFETY: the raw result has not been adopted; the helper rejects null and takes sole
    // ownership of a successful CreateJobObjectW result.
    let job = JobHandle(unsafe { adopt_non_null_handle(job_raw, WindowsProcessStage::CreateJob)? });

    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `job` is a valid owned Job handle and `limits` is the correct initialized
    // structure, size, and immutable pointer for JobObjectExtendedLimitInformation.
    let configured = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return Err(last_error(WindowsProcessStage::ConfigureJob).into());
    }

    let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: ptr::without_provenance_mut(COMPLETION_KEY),
        CompletionPort: completion_port.raw(),
    };
    // SAFETY: the Job is still empty, both owned handles are valid, and `association` has the
    // exact structure and size expected. Its fixed nonzero key is opaque numeric data and is
    // never dereferenced.
    let associated = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectAssociateCompletionPortInformation,
            (&raw const association).cast::<c_void>(),
            size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
        )
    };
    if associated == 0 {
        return Err(last_error(WindowsProcessStage::AssociateCompletionPort).into());
    }

    let security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: 1,
        ..Default::default()
    };
    let stdin = open_null_handle(
        GENERIC_READ,
        &security_attributes,
        WindowsProcessStage::OpenNullStdin,
    )?;
    let stdout = open_null_handle(
        GENERIC_WRITE,
        &security_attributes,
        WindowsProcessStage::OpenNullStdout,
    )?;
    let stderr = open_null_handle(
        GENERIC_WRITE,
        &security_attributes,
        WindowsProcessStage::OpenNullStderr,
    )?;
    let inherited_handles = [stdin.raw(), stdout.raw(), stderr.raw()];
    let mut attribute_list = ProcThreadAttributeList::with_handle_list(&inherited_handles)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin.raw();
    startup.StartupInfo.hStdOutput = stdout.raw();
    startup.StartupInfo.hStdError = stderr.raw();
    startup.lpAttributeList = attribute_list.raw();

    check_deadline(deadline, WindowsProcessStage::CreateProcess)?;
    let mut process_information = PROCESS_INFORMATION::default();
    // SAFETY: application and cwd are immutable NUL-terminated UTF-16 buffers; command_line is
    // a uniquely borrowed mutable NUL-terminated buffer; environment is a live double-NUL UTF-16
    // block. STARTUPINFOEXW has its full size, valid standard handles, and a live initialized
    // attribute list containing exactly those handles. Output storage is correctly sized and
    // writable. Security pointers are null, inheritance is constrained by the handle list, and
    // no pointer outlives this call.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast::<c_void>(),
            working_directory.as_ptr(),
            &startup.StartupInfo,
            &mut process_information,
        )
    };
    if created == 0 {
        return Err(last_error(WindowsProcessStage::CreateProcess).into());
    }

    let process = if process_information.hProcess.is_null() {
        None
    } else {
        // SAFETY: successful CreateProcessW returned this fresh non-null process handle, it has
        // not been adopted or closed, and this transfers sole ownership exactly once.
        Some(ProcessHandle(unsafe {
            OwnedHandle::from_raw_handle(process_information.hProcess)
        }))
    };
    let thread = if process_information.hThread.is_null() {
        None
    } else {
        // SAFETY: successful CreateProcessW returned this fresh non-null primary-thread handle,
        // it has not been adopted or closed, and this transfers sole ownership exactly once.
        Some(ThreadHandle(unsafe {
            OwnedHandle::from_raw_handle(process_information.hThread)
        }))
    };
    let mut startup_process = UnconfirmedWindowsProcess {
        membership: Membership::PreAssignment,
        root: process,
        thread,
        completion_port: Some(completion_port),
        job: Some(job),
        confirmed: false,
    };
    if startup_process.root.is_none() || startup_process.thread.is_none() {
        let origin = launch_error(WindowsProcessStage::CreateProcess, None);
        return Err(rollback_spawn_failure(
            origin,
            startup_process,
            deadline,
            &mut control,
        ));
    }

    #[cfg(test)]
    if let Some(trace) = control.trace.as_deref_mut() {
        match duplicate_process_observation(startup_process.process()) {
            Ok(observation) => trace.root_observation = Some(observation),
            Err(error) => {
                return Err(rollback_spawn_failure(
                    error,
                    startup_process,
                    deadline,
                    &mut control,
                ));
            }
        }
    }

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the process wrapper owns a valid live process handle and all four FILETIME values
    // are correctly sized writable storage that remains live for the call.
    let read_times = unsafe {
        GetProcessTimes(
            startup_process.process().raw(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if read_times == 0 {
        let origin = last_error(WindowsProcessStage::ReadBirthIdentity);
        return Err(rollback_spawn_failure(
            origin,
            startup_process,
            deadline,
            &mut control,
        ));
    }
    let identity = ProcessBirthId {
        pid: process_information.dwProcessId,
        creation_time_100ns: (u64::from(creation.dwHighDateTime) << 32)
            | u64::from(creation.dwLowDateTime),
    };

    #[cfg(test)]
    if matches!(control.failure_point, Some(TestFailurePoint::BeforeAssign)) {
        return Err(rollback_spawn_failure(
            launch_error(WindowsProcessStage::AssignJob, None),
            startup_process,
            deadline,
            &mut control,
        ));
    }
    if let Err(error) = check_deadline(deadline, WindowsProcessStage::AssignJob) {
        return Err(rollback_spawn_failure(
            error,
            startup_process,
            deadline,
            &mut control,
        ));
    }
    // SAFETY: both wrappers own valid handles; the root is still suspended and the Job is empty
    // and already fully configured with its completion port.
    let assigned = unsafe {
        AssignProcessToJobObject(startup_process.job().raw(), startup_process.process().raw())
    };
    if assigned == 0 {
        let origin = last_error(WindowsProcessStage::AssignJob);
        return Err(rollback_spawn_failure(
            origin,
            startup_process,
            deadline,
            &mut control,
        ));
    }
    startup_process.membership = Membership::Contained;

    #[cfg(test)]
    if matches!(
        control.failure_point,
        Some(TestFailurePoint::AfterAssignBeforeResume)
    ) {
        return Err(rollback_spawn_failure(
            launch_error(WindowsProcessStage::ResumeThread, None),
            startup_process,
            deadline,
            &mut control,
        ));
    }
    if let Err(error) = check_deadline(deadline, WindowsProcessStage::ResumeThread) {
        return Err(rollback_spawn_failure(
            error,
            startup_process,
            deadline,
            &mut control,
        ));
    }
    // SAFETY: the thread wrapper owns the live suspended primary-thread handle. ResumeThread
    // does not transfer ownership; the wrapper remains valid after the call.
    let previous_suspend_count = unsafe { ResumeThread(startup_process.thread().raw()) };
    if previous_suspend_count != 1 {
        let os_code = (previous_suspend_count == u32::MAX)
            .then(last_os_code)
            .flatten();
        return Err(rollback_spawn_failure(
            launch_error(WindowsProcessStage::ResumeThread, os_code),
            startup_process,
            deadline,
            &mut control,
        ));
    }

    let root = startup_process
        .root
        .take()
        .expect("startup owner must contain the resumed root");
    drop(startup_process.thread.take());
    drop(attribute_list);
    drop(stderr);
    drop(stdout);
    drop(stdin);
    let completion_port = startup_process
        .completion_port
        .take()
        .expect("startup owner must contain the completion port");
    let job = startup_process
        .job
        .take()
        .expect("startup owner must contain the configured Job");
    startup_process.confirmed = true;
    drop(startup_process);

    Ok(WindowsContainedProcess {
        completion_port,
        root,
        identity,
        tree_empty_confirmed: false,
        #[cfg(test)]
        last_active_processes: None,
        job,
    })
}

fn rollback_spawn_failure(
    origin: WindowsProcessError,
    mut owner: UnconfirmedWindowsProcess,
    deadline: Instant,
    _control: &mut SpawnControl<'_>,
) -> WindowsSpawnFailure {
    #[cfg(test)]
    let suppress_confirmation = _control.suppress_confirmation_once;
    #[cfg(not(test))]
    let suppress_confirmation = false;
    let cleanup_result = owner.cleanup(
        deadline,
        #[cfg(test)]
        _control.trace.as_deref_mut(),
        #[cfg(not(test))]
        None,
        suppress_confirmation,
    );
    match cleanup_result {
        Ok(()) => WindowsSpawnFailure::Rejected(origin),
        Err(cleanup) => WindowsSpawnFailure::CleanupUnconfirmed {
            error: cleanup_unconfirmed_error(origin, cleanup),
            owner,
        },
    }
}

fn wait_for_root_signal(
    root: &ProcessHandle,
    deadline: Instant,
) -> Result<(), WindowsProcessError> {
    loop {
        let wait = wait_millis(deadline, Instant::now());
        // SAFETY: the semantic root wrapper owns a valid process handle for the complete wait;
        // the finite timeout includes zero for the final already-expired observation.
        let wait_result = unsafe { WaitForSingleObject(root.raw(), wait) };
        match wait_result {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT if Instant::now() < deadline => continue,
            WAIT_TIMEOUT => {
                return Err(cleanup_error(WindowsProcessStage::WaitRoot, None));
            }
            WAIT_FAILED => {
                return Err(cleanup_error(WindowsProcessStage::WaitRoot, last_os_code()));
            }
            _ => return Err(cleanup_error(WindowsProcessStage::WaitRoot, None)),
        }
    }
}

fn query_active_processes(job: &JobHandle) -> Result<u32, Option<u32>> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: the semantic Job wrapper owns a valid Job handle; `accounting` is correctly sized,
    // aligned writable storage for the requested information class and remains live for the call.
    let queried = unsafe {
        QueryInformationJobObject(
            job.raw(),
            JobObjectBasicAccountingInformation,
            (&raw mut accounting).cast::<c_void>(),
            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            ptr::null_mut(),
        )
    };
    if queried == 0 {
        Err(last_os_code())
    } else {
        Ok(accounting.ActiveProcesses)
    }
}

#[derive(Clone, Copy)]
struct TreeObservation {
    root_signaled: bool,
    active_processes: u32,
}

fn observe_root_nonblocking(root: &ProcessHandle) -> Result<bool, WindowsProcessError> {
    // SAFETY: the semantic root wrapper owns a valid process handle for this zero-time query; no
    // ownership is transferred and the handle remains live after the call.
    let wait_result = unsafe { WaitForSingleObject(root.raw(), 0) };
    match wait_result {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(query_error(WindowsProcessStage::WaitRoot, last_os_code())),
        _ => Err(query_error(WindowsProcessStage::WaitRoot, None)),
    }
}

fn wait_for_completion_wake(
    completion_port: &CompletionPortHandle,
    timeout_millis: u32,
) -> Result<(), WindowsProcessError> {
    let mut transferred = 0u32;
    let mut completion_key = 0usize;
    let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
    // SAFETY: the semantic completion-port wrapper owns a valid port handle; all three output
    // pointers refer to correctly sized writable storage live for the complete finite wait. The
    // returned message fields are intentionally ignored because completion is only a wake hint.
    let dequeued = unsafe {
        GetQueuedCompletionStatus(
            completion_port.raw(),
            &mut transferred,
            &mut completion_key,
            &mut overlapped,
            timeout_millis,
        )
    };
    if dequeued != 0 || !overlapped.is_null() {
        return Ok(());
    }
    let os_code = last_os_code();
    if os_code == Some(WAIT_TIMEOUT) {
        Ok(())
    } else {
        Err(query_error(WindowsProcessStage::QueryJob, os_code))
    }
}

fn query_root_exit(
    root: &ProcessHandle,
    identity: ProcessBirthId,
) -> Result<RootExit, WindowsProcessError> {
    let mut exit_code = 0u32;
    // SAFETY: the semantic root wrapper owns a valid signaled process handle, and `exit_code` is
    // correctly sized writable storage live for the call. No ownership is transferred.
    let queried = unsafe { GetExitCodeProcess(root.raw(), &mut exit_code) };
    if queried == 0 {
        return Err(query_error(WindowsProcessStage::WaitRoot, last_os_code()));
    }
    Ok(RootExit {
        identity,
        exit_code,
    })
}

#[cfg(test)]
fn duplicate_process_observation(
    process: &ProcessHandle,
) -> Result<ProcessObservationHandle, WindowsProcessError> {
    let mut duplicate = ptr::null_mut();
    // SAFETY: GetCurrentProcess supplies valid source and target pseudo-handles for this call;
    // `process` owns the valid source process handle; `duplicate` is writable output storage.
    // DUPLICATE_SAME_ACCESS requests an independent non-inheritable handle without changing the
    // source, and a successful raw result is adopted exactly once below.
    let duplicated = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            process.raw(),
            GetCurrentProcess(),
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if duplicated == 0 || duplicate.is_null() {
        return Err(last_error(WindowsProcessStage::ReadBirthIdentity));
    }
    // SAFETY: successful DuplicateHandle returned a fresh non-null handle in `duplicate`; it has
    // not been adopted or closed and ownership transfers exactly once here.
    Ok(ProcessObservationHandle(unsafe {
        OwnedHandle::from_raw_handle(duplicate)
    }))
}

impl WindowsContainedProcess {
    pub(crate) fn birth_id(&self) -> ProcessBirthId {
        self.identity
    }

    pub(crate) fn try_wait_root(&mut self) -> Result<Option<RootExit>, WindowsProcessError> {
        // SAFETY: `root` owns a valid process handle for this single zero-time observation; no
        // ownership is transferred and the handle remains live after the call.
        let wait_result = unsafe { WaitForSingleObject(self.root.raw(), 0) };
        match wait_result {
            WAIT_OBJECT_0 => query_root_exit(&self.root, self.identity).map(Some),
            WAIT_TIMEOUT => Ok(None),
            WAIT_FAILED => Err(query_error(WindowsProcessStage::WaitRoot, last_os_code())),
            _ => Err(query_error(WindowsProcessStage::WaitRoot, None)),
        }
    }

    #[allow(
        dead_code,
        reason = "blocking root-only wait remains adapter regression-test support"
    )]
    pub(crate) fn wait_root(&mut self, deadline: Instant) -> Result<RootExit, WindowsProcessError> {
        loop {
            let wait = wait_millis(deadline, Instant::now());
            // SAFETY: `root` owns a valid process handle for the complete wait. The timeout is
            // finite, including zero for the required already-expired non-blocking observation.
            let wait_result = unsafe { WaitForSingleObject(self.root.raw(), wait) };
            match wait_result {
                WAIT_OBJECT_0 => break,
                WAIT_TIMEOUT if Instant::now() < deadline => continue,
                WAIT_TIMEOUT => return Err(deadline_error(WindowsProcessStage::WaitRoot)),
                WAIT_FAILED => {
                    return Err(query_error(WindowsProcessStage::WaitRoot, last_os_code()));
                }
                _ => return Err(query_error(WindowsProcessStage::WaitRoot, None)),
            }
        }

        query_root_exit(&self.root, self.identity)
    }

    pub(crate) fn terminate_tree_and_wait(
        &mut self,
        deadline: Instant,
    ) -> Result<RootExit, WindowsProcessError> {
        if self.tree_empty_confirmed {
            return query_root_exit(&self.root, self.identity);
        }

        // SAFETY: the semantic Job wrapper owns the valid configured Job handle. This is the
        // method's sole termination request; it neither transfers nor invalidates ownership.
        let terminated = unsafe { TerminateJobObject(self.job.raw(), HARD_STOP_EXIT_CODE) };
        let termination_error = (terminated == 0)
            .then(|| query_error(WindowsProcessStage::TerminateJob, last_os_code()));

        let mut observation_error = None;
        match self.observe_tree() {
            Ok(observation) if self.confirm_observation(observation) => {
                return query_root_exit(&self.root, self.identity);
            }
            Ok(_) => {}
            Err(error) => observation_error = Some(error),
        }
        let mut next_query = Instant::now() + HARD_STOP_QUERY_CADENCE;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return match self.observe_tree() {
                    Ok(observation) if self.confirm_observation(observation) => {
                        query_root_exit(&self.root, self.identity)
                    }
                    Ok(observation) => Err(termination_error.unwrap_or_else(|| {
                        if observation.root_signaled {
                            deadline_error(WindowsProcessStage::QueryJob)
                        } else {
                            deadline_error(WindowsProcessStage::WaitRoot)
                        }
                    })),
                    Err(error) => Err(termination_error.or(observation_error).unwrap_or(error)),
                };
            }

            if now >= next_query {
                match self.observe_tree() {
                    Ok(observation) if self.confirm_observation(observation) => {
                        return query_root_exit(&self.root, self.identity);
                    }
                    Ok(_) => observation_error = None,
                    Err(error) => observation_error = Some(error),
                }
                next_query = Instant::now() + HARD_STOP_QUERY_CADENCE;
                continue;
            }

            let wake_deadline = deadline.min(next_query);
            let timeout = wait_millis(wake_deadline, now).min(50);
            if let Err(error) = wait_for_completion_wake(&self.completion_port, timeout) {
                return Err(termination_error.unwrap_or(error));
            }
        }
    }

    fn observe_tree(&mut self) -> Result<TreeObservation, WindowsProcessError> {
        let root_signaled = observe_root_nonblocking(&self.root);
        let active_processes = query_active_processes(&self.job)
            .map_err(|os_code| query_error(WindowsProcessStage::QueryJob, os_code));
        #[cfg(test)]
        if let Ok(active_processes) = active_processes {
            self.last_active_processes = Some(active_processes);
        }
        match (root_signaled, active_processes) {
            (Ok(root_signaled), Ok(active_processes)) => Ok(TreeObservation {
                root_signaled,
                active_processes,
            }),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }

    fn confirm_observation(&mut self, observation: TreeObservation) -> bool {
        if observation.root_signaled && observation.active_processes == 0 {
            self.tree_empty_confirmed = true;
        }
        self.tree_empty_confirmed
    }

    #[cfg(test)]
    fn last_active_processes_for_test(&self) -> Option<u32> {
        self.last_active_processes
    }
}

impl Drop for WindowsContainedProcess {
    fn drop(&mut self) {
        if !self.tree_empty_confirmed {
            // SAFETY: the semantic Job wrapper owns a valid configured Job handle. Drop makes one
            // non-blocking best-effort termination request without transferring ownership; normal
            // field destruction then closes completion/root handles before the Job owner last.
            unsafe {
                TerminateJobObject(self.job.raw(), HARD_STOP_EXIT_CODE);
            }
        }
    }
}

fn open_null_handle(
    desired_access: u32,
    security_attributes: &SECURITY_ATTRIBUTES,
    stage: WindowsProcessStage,
) -> Result<NullHandle, WindowsProcessError> {
    // SAFETY: NUL_NAME is immutable and NUL-terminated; security_attributes is fully initialized,
    // correctly sized, inheritable, and live for the call. OPEN_EXISTING and shared read/write
    // access are valid for the NUL device. The returned file handle is adopted below exactly once.
    let raw = unsafe {
        CreateFileW(
            NUL_NAME.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            security_attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    // SAFETY: the CreateFileW result has not been adopted; this helper rejects both null and
    // INVALID_HANDLE_VALUE before transferring a successful result exactly once.
    unsafe { adopt_file_handle(raw, stage).map(NullHandle) }
}

/// Adopts sole ownership of a Win32 handle whose API uses NULL as its failure sentinel.
///
/// # Safety
/// `raw` must be either NULL or a fresh owned handle that has not been adopted or closed.
unsafe fn adopt_non_null_handle(
    raw: HANDLE,
    stage: WindowsProcessStage,
) -> Result<OwnedHandle, WindowsProcessError> {
    if raw.is_null() {
        return Err(last_error(stage));
    }
    // SAFETY: the caller guarantees a fresh, valid, solely owned non-null Win32 handle, and this
    // is the single ownership transfer for it.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// Adopts sole ownership of a Win32 file handle after checking both documented failure sentinels.
///
/// # Safety
/// `raw` must be NULL, INVALID_HANDLE_VALUE, or a fresh owned handle not yet adopted or closed.
unsafe fn adopt_file_handle(
    raw: HANDLE,
    stage: WindowsProcessStage,
) -> Result<OwnedHandle, WindowsProcessError> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(last_error(stage));
    }
    // SAFETY: the caller guarantees a fresh, valid, solely owned file handle after both sentinel
    // checks, and this is the single ownership transfer for it.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn check_deadline(
    deadline: Instant,
    stage: WindowsProcessStage,
) -> Result<(), WindowsProcessError> {
    if Instant::now() >= deadline {
        Err(deadline_error(stage))
    } else {
        Ok(())
    }
}

fn launch_error(stage: WindowsProcessStage, os_code: Option<u32>) -> WindowsProcessError {
    WindowsProcessError {
        kind: WindowsProcessErrorKind::LaunchFailed,
        stage,
        os_code,
        cleanup_stage: None,
        cleanup_os_code: None,
    }
}

fn deadline_error(stage: WindowsProcessStage) -> WindowsProcessError {
    WindowsProcessError {
        kind: WindowsProcessErrorKind::DeadlineExpired,
        stage,
        os_code: None,
        cleanup_stage: None,
        cleanup_os_code: None,
    }
}

fn query_error(stage: WindowsProcessStage, os_code: Option<u32>) -> WindowsProcessError {
    WindowsProcessError {
        kind: WindowsProcessErrorKind::QueryFailed,
        stage,
        os_code,
        cleanup_stage: None,
        cleanup_os_code: None,
    }
}

fn cleanup_error(stage: WindowsProcessStage, os_code: Option<u32>) -> WindowsProcessError {
    WindowsProcessError {
        kind: WindowsProcessErrorKind::CleanupUnconfirmed,
        stage,
        os_code,
        cleanup_stage: Some(stage),
        cleanup_os_code: os_code,
    }
}

fn prefer_termination_error(
    termination_stage: WindowsProcessStage,
    termination_os_code: Option<u32>,
    observation_error: WindowsProcessError,
) -> WindowsProcessError {
    if termination_os_code.is_some() {
        cleanup_error(termination_stage, termination_os_code)
    } else {
        observation_error
    }
}

fn cleanup_unconfirmed_error(
    origin: WindowsProcessError,
    cleanup: WindowsProcessError,
) -> WindowsProcessError {
    WindowsProcessError {
        kind: WindowsProcessErrorKind::CleanupUnconfirmed,
        stage: origin.stage,
        os_code: origin.os_code,
        cleanup_stage: cleanup.cleanup_stage.or(Some(cleanup.stage)),
        cleanup_os_code: cleanup.cleanup_os_code.or(cleanup.os_code),
    }
}

fn last_error(stage: WindowsProcessStage) -> WindowsProcessError {
    launch_error(stage, last_os_code())
}

fn last_os_code() -> Option<u32> {
    io::Error::last_os_error()
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
}

fn encode_environment_block(
    environment: &[LaunchEnvironmentVariable],
) -> Result<Vec<u16>, WindowsProcessError> {
    if environment.len() != crate::launch_snapshot::ENVIRONMENT_VARIABLE_COUNT {
        return Err(environment_error());
    }

    let mut entries: Vec<(&'static str, &LaunchEnvironmentVariable)> = environment
        .iter()
        .map(|variable| (environment_name(variable.key()), variable))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    // Sorted, so this is the exact set the child may receive: the three temp names plus SystemRoot,
    // which WinSock needs to resolve a host at all.
    if entries
        .iter()
        .map(|(name, _)| *name)
        .ne(["SystemRoot", "TEMP", "TMP", "TMPDIR"])
    {
        return Err(environment_error());
    }

    let mut output = Vec::new();
    for (name, variable) in entries {
        output.extend(name.encode_utf16());
        output.push(b'=' as u16);
        for unit in variable.value().as_os_str().encode_wide() {
            if unit == 0 {
                return Err(environment_error());
            }
            output.push(unit);
        }
        output.push(0);
    }
    output.push(0);
    Ok(output)
}

fn environment_name(key: LaunchEnvironmentKey) -> &'static str {
    match key {
        LaunchEnvironmentKey::Temp => "TEMP",
        LaunchEnvironmentKey::Tmp => "TMP",
        LaunchEnvironmentKey::TmpDir => "TMPDIR",
        LaunchEnvironmentKey::SystemRoot => "SystemRoot",
    }
}

fn environment_error() -> WindowsProcessError {
    launch_error(WindowsProcessStage::EncodeEnvironment, None)
}

fn wait_millis(deadline: Instant, now: Instant) -> u32 {
    let remaining = deadline.saturating_duration_since(now);
    if remaining.is_zero() {
        return 0;
    }

    let rounded_millis = remaining.as_millis().saturating_add(u128::from(
        !remaining.subsec_nanos().is_multiple_of(1_000_000),
    ));
    rounded_millis.min(u128::from(INFINITE - 1)) as u32
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, GetProcessTimes, TerminateProcess,
};

use crate::process_adapter::test_support::{
    CompiledProbe, DESCENDANT_NUMERIC_FIELD_COUNT, DESCENDANT_RECORD_TYPE, ProbeChild,
    ProbeTestDirectory, compile_probe, decode_probe_record, open_identity_checked_process,
    open_identity_checked_process_for_termination, wait_for_process_observation,
    wait_for_ready_path,
};
use crate::{CoreState, ProcessLaunchSpec};

use super::{SessionObservation, SessionSupervisor};

fn sleep_spec(
    probe: &CompiledProbe,
    directory: &ProbeTestDirectory,
    record_name: &str,
    lifetime_millis: u64,
) -> (ProcessLaunchSpec, std::path::PathBuf) {
    let record_base = directory.path().join(record_name);
    let ready_path = record_base.with_extension("ready");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("sleep"),
            record_base.into_os_string(),
            OsString::from(lifetime_millis.to_string()),
        ],
        directory.path().to_owned(),
        directory.temp_path().to_owned(),
    )
    .expect("live Supervisor sleep spec should seal");
    (spec, ready_path)
}

fn descendant_spec(
    executable: &Path,
    directory: &ProbeTestDirectory,
    record_name: &str,
) -> (ProcessLaunchSpec, PathBuf) {
    let record_base = directory.path().join(record_name);
    let ready_path = record_base.with_extension("ready");
    let spec = ProcessLaunchSpec::for_windows_test(
        executable.to_owned(),
        vec![
            OsString::from("descendant"),
            record_base.into_os_string(),
            OsString::from("300000"),
        ],
        directory.path().to_owned(),
        directory.temp_path().to_owned(),
    )
    .expect("live Supervisor descendant spec should seal");
    (spec, ready_path)
}

fn descendant_births(ready_path: &Path) -> ((u32, u64), (u32, u64)) {
    let record = decode_probe_record(ready_path).expect("descendant readiness should decode");
    assert_eq!(record.record_type, DESCENDANT_RECORD_TYPE);
    assert_eq!(record.numeric_fields.len(), DESCENDANT_NUMERIC_FIELD_COUNT);
    (
        (
            u32::try_from(record.numeric_fields[0]).expect("root PID should fit u32"),
            record.numeric_fields[1],
        ),
        (
            u32::try_from(record.numeric_fields[2]).expect("descendant PID should fit u32"),
            record.numeric_fields[3],
        ),
    )
}

#[test]
fn live_supervisor_owns_two_distinct_probe_sessions() {
    let probe = compile_probe();
    let core_directory = ProbeTestDirectory::create().expect("create live Supervisor Core root");
    let first_directory = ProbeTestDirectory::create().expect("create first live session root");
    let second_directory = ProbeTestDirectory::create().expect("create second live session root");
    let first_path = first_directory.path().to_owned();
    let second_path = second_directory.path().to_owned();
    let core_path = core_directory.path().to_owned();
    let core = CoreState::open_at(core_directory.path()).expect("open live Supervisor Core");
    let mut supervisor = SessionSupervisor::new(core);
    let (first_spec, first_ready) = sleep_spec(&probe, &first_directory, "first", 300_000);
    let (second_spec, second_ready) = sleep_spec(&probe, &second_directory, "second", 300_000);
    let first_session_id = first_spec.session_id();
    let first_profile_id = first_spec.profile_id();
    let second_session_id = second_spec.session_id();
    let second_profile_id = second_spec.profile_id();
    let deadline = Instant::now() + Duration::from_secs(10);

    let first = supervisor
        .start_spec_for_test(first_spec, deadline)
        .expect("start first live Supervisor session");
    let second = supervisor
        .start_spec_for_test(second_spec, deadline)
        .expect("start second live Supervisor session");
    wait_for_ready_path(&first_ready, deadline)
        .expect("first sleep probe should publish readiness");
    wait_for_ready_path(&second_ready, deadline)
        .expect("second sleep probe should publish readiness");

    assert_eq!(first.metadata().session_id(), first_session_id);
    assert_eq!(first.metadata().profile_id(), first_profile_id);
    assert_eq!(second.metadata().session_id(), second_session_id);
    assert_eq!(second.metadata().profile_id(), second_profile_id);
    assert_ne!(first_session_id, second_session_id);
    assert_ne!(first_profile_id, second_profile_id);
    assert_ne!(first.birth_id(), second.birth_id());
    let first_observation = open_identity_checked_process(
        first.birth_id().pid(),
        first.birth_id().creation_time_100ns(),
    );
    let second_observation = open_identity_checked_process(
        second.birth_id().pid(),
        second.birth_id().creation_time_100ns(),
    );
    assert_eq!(supervisor.active_session_count(), 2);

    supervisor
        .stop_session(first_session_id, deadline)
        .expect("stop first live Supervisor session");
    assert_eq!(supervisor.active_session_count(), 1);
    supervisor
        .stop_session(second_session_id, deadline)
        .expect("stop second live Supervisor session");
    assert_eq!(supervisor.active_session_count(), 0);
    wait_for_process_observation(&first_observation, deadline);
    wait_for_process_observation(&second_observation, deadline);

    drop(supervisor);
    drop(first_directory);
    drop(second_directory);
    drop(core_directory);
    assert!(!first_path.exists());
    assert!(!second_path.exists());
    assert!(!core_path.exists());
}

#[test]
fn live_supervisor_hard_stop_confirms_root_and_descendant_exit() {
    let probe = compile_probe();
    let core_directory = ProbeTestDirectory::create().expect("create hard-stop Core root");
    let session_directory = ProbeTestDirectory::create().expect("create hard-stop session root");
    let session_path = session_directory.path().to_owned();
    let core_path = core_directory.path().to_owned();
    let core = CoreState::open_at(core_directory.path()).expect("open hard-stop Core");
    let mut supervisor = SessionSupervisor::new(core);
    let (spec, ready_path) = descendant_spec(&probe.executable, &session_directory, "hard-stop");
    let session_id = spec.session_id();
    let deadline = Instant::now() + Duration::from_secs(10);
    let started = supervisor
        .start_spec_for_test(spec, deadline)
        .expect("start hard-stop descendant session");
    wait_for_ready_path(&ready_path, deadline).expect("descendant should publish readiness");
    let ((root_pid, root_creation), (descendant_pid, descendant_creation)) =
        descendant_births(&ready_path);
    assert_eq!(started.birth_id().pid(), root_pid);
    assert_eq!(started.birth_id().creation_time_100ns(), root_creation);
    let root_observation = open_identity_checked_process(root_pid, root_creation);
    let descendant_observation = open_identity_checked_process(descendant_pid, descendant_creation);

    let exit = supervisor
        .stop_session(session_id, deadline)
        .expect("hard stop should confirm empty Job");
    assert_eq!(exit.birth_id(), started.birth_id());
    assert_eq!(supervisor.active_session_count(), 0);
    wait_for_process_observation(&root_observation, deadline);
    wait_for_process_observation(&descendant_observation, deadline);

    drop(supervisor);
    drop(session_directory);
    drop(core_directory);
    assert!(!session_path.exists());
    assert!(!core_path.exists());
}

#[test]
fn live_supervisor_drop_is_non_blocking_and_kills_all_owned_trees() {
    let probe = compile_probe();
    let core_directory = ProbeTestDirectory::create().expect("create drop Core root");
    let first_directory = ProbeTestDirectory::create().expect("create first drop session root");
    let second_directory = ProbeTestDirectory::create().expect("create second drop session root");
    let first_path = first_directory.path().to_owned();
    let second_path = second_directory.path().to_owned();
    let core_path = core_directory.path().to_owned();
    let core = CoreState::open_at(core_directory.path()).expect("open drop Core");
    let mut supervisor = SessionSupervisor::new(core);
    let (first_spec, first_ready) =
        descendant_spec(&probe.executable, &first_directory, "drop-first");
    let (second_spec, second_ready) =
        descendant_spec(&probe.executable, &second_directory, "drop-second");
    let deadline = Instant::now() + Duration::from_secs(10);
    supervisor
        .start_spec_for_test(first_spec, deadline)
        .expect("start first dropped tree");
    supervisor
        .start_spec_for_test(second_spec, deadline)
        .expect("start second dropped tree");
    wait_for_ready_path(&first_ready, deadline).expect("first dropped tree should be ready");
    wait_for_ready_path(&second_ready, deadline).expect("second dropped tree should be ready");
    let (first_root, first_descendant) = descendant_births(&first_ready);
    let (second_root, second_descendant) = descendant_births(&second_ready);
    let observations = [first_root, first_descendant, second_root, second_descendant]
        .map(|(pid, creation)| open_identity_checked_process(pid, creation));

    let before_drop = Instant::now();
    drop(supervisor);
    assert!(before_drop.elapsed() < Duration::from_secs(1));
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    for observation in &observations {
        wait_for_process_observation(observation, exit_deadline);
    }
    drop(first_directory);
    drop(second_directory);
    drop(core_directory);
    assert!(!first_path.exists());
    assert!(!second_path.exists());
    assert!(!core_path.exists());
}

const CHILD_PROBE_ENV: &str = "ZHSO_SUPERVISOR_CHILD_PROBE";
const CHILD_ROOT_ENV: &str = "ZHSO_SUPERVISOR_CHILD_ROOT";
const OWNER_MAGIC: [u8; 8] = *b"ZHSOWNR1";

fn owner_record_path(root: &Path) -> PathBuf {
    root.join("supervisor-owner.ready")
}

fn publish_owner_record(path: &Path, pid: u32, creation_time_100ns: u64) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&OWNER_MAGIC)?;
    file.write_all(&pid.to_le_bytes())?;
    file.write_all(&creation_time_100ns.to_le_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)
}

fn read_owner_record(path: &Path) -> io::Result<(u32, u64)> {
    let bytes = fs::read(path)?;
    if bytes.len() != 20 || bytes[..8] != OWNER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Supervisor owner record",
        ));
    }
    let pid = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid owner PID"))?,
    );
    let creation =
        u64::from_le_bytes(bytes[12..20].try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid owner creation time")
        })?);
    Ok((pid, creation))
}

fn child_creation_time(child: &Child) -> u64 {
    let mut creation = windows_sys::Win32::Foundation::FILETIME::default();
    let mut exit = windows_sys::Win32::Foundation::FILETIME::default();
    let mut kernel = windows_sys::Win32::Foundation::FILETIME::default();
    let mut user = windows_sys::Win32::Foundation::FILETIME::default();
    // SAFETY: Child owns a live process handle and all FILETIME outputs are writable for the call.
    let queried = unsafe {
        GetProcessTimes(
            child.as_raw_handle() as HANDLE,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    assert_ne!(queried, 0, "query child process creation time");
    (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
}

#[test]
#[ignore = "child-only harness for the Supervisor parent-crash test"]
fn supervisor_parent_crash_child() {
    let (Some(probe), Some(root)) = (
        std::env::var_os(CHILD_PROBE_ENV),
        std::env::var_os(CHILD_ROOT_ENV),
    ) else {
        return;
    };
    let probe = fs::canonicalize(PathBuf::from(probe)).expect("canonical child probe path");
    let root = fs::canonicalize(PathBuf::from(root)).expect("canonical child root path");
    let temp = fs::canonicalize(root.join("temp")).expect("canonical child temp path");
    let core = CoreState::open_at(&root).expect("open parent-crash child Core");
    let mut supervisor = SessionSupervisor::new(core);
    let record_base = root.join("parent-crash-descendant");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe,
        vec![
            OsString::from("descendant"),
            record_base.into_os_string(),
            OsString::from("300000"),
        ],
        root.clone(),
        temp,
    )
    .expect("seal parent-crash child spec");
    let started = supervisor
        .start_spec_for_test(spec, Instant::now() + Duration::from_secs(10))
        .expect("start parent-crash child session");
    publish_owner_record(
        &owner_record_path(&root),
        started.birth_id().pid(),
        started.birth_id().creation_time_100ns(),
    )
    .expect("publish parent-crash owner record");
    loop {
        thread::park_timeout(Duration::from_secs(1));
    }
}

#[test]
fn supervisor_parent_process_death_kills_root_and_descendant() {
    let probe = compile_probe();
    let directory = ProbeTestDirectory::create().expect("create parent-crash root");
    let root_path = directory.path().to_owned();
    let owner_path = owner_record_path(directory.path());
    let descendant_ready = directory
        .path()
        .join("parent-crash-descendant")
        .with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    let current_exe = fs::canonicalize(std::env::current_exe().expect("current test executable"))
        .expect("canonical current test executable");
    let mut command = Command::new(current_exe);
    command
        .arg("--exact")
        .arg("session_supervisor::windows_tests::supervisor_parent_crash_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env_clear()
        .env(CHILD_PROBE_ENV, &probe.executable)
        .env(CHILD_ROOT_ENV, directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = ProbeChild::spawn(&mut command, deadline).expect("spawn parent-crash owner");
    let child_pid = child.child_mut().id();
    let child_creation = child_creation_time(child.child_mut());
    let child_observation =
        open_identity_checked_process_for_termination(child_pid, child_creation);
    wait_for_ready_path(&owner_path, deadline).expect("child should publish owner record");
    wait_for_ready_path(&descendant_ready, deadline)
        .expect("child descendant should publish readiness");
    let (owned_root_pid, owned_root_creation) =
        read_owner_record(&owner_path).expect("decode owner record");
    let (root_birth, descendant_birth) = descendant_births(&descendant_ready);
    assert_eq!(root_birth, (owned_root_pid, owned_root_creation));
    let root_observation = open_identity_checked_process(root_birth.0, root_birth.1);
    let descendant_observation =
        open_identity_checked_process(descendant_birth.0, descendant_birth.1);

    // SAFETY: the identity-checked observation owns a live handle to only the spawned child owner.
    let terminated = unsafe {
        TerminateProcess(
            child_observation.handle.as_raw_handle() as HANDLE,
            0xffff_fffc,
        )
    };
    let termination_error = (terminated == 0).then(io::Error::last_os_error);
    assert_ne!(
        terminated, 0,
        "terminate identity-checked child owner: {termination_error:?}"
    );
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    wait_for_process_observation(&root_observation, exit_deadline);
    wait_for_process_observation(&descendant_observation, exit_deadline);
    wait_for_process_observation(&child_observation, exit_deadline);
    child
        .wait_until_exit()
        .expect("reap terminated parent-crash child");

    drop(directory);
    assert!(!root_path.exists());
}

#[test]
#[ignore = "run as the dedicated serialized Windows Supervisor handle-leak gate"]
fn windows_supervisor_handle_count_stays_bounded() {
    let probe = compile_probe();

    run_supervisor_natural_handle_cycle(&probe, "supervisor-handle-warmup-natural");
    run_supervisor_stop_handle_cycle(&probe, "supervisor-handle-warmup-stop");
    let baseline = current_process_handle_count();

    let mut natural_samples = Vec::with_capacity(32);
    for cycle in 0..32 {
        run_supervisor_natural_handle_cycle(&probe, &format!("supervisor-handle-natural-{cycle}"));
        natural_samples.push(current_process_handle_count());
    }
    assert_handle_samples_bounded("natural", baseline, &natural_samples);
    let final_natural = *natural_samples
        .last()
        .expect("natural handle samples should not be empty");

    let mut stop_samples = Vec::with_capacity(32);
    for cycle in 0..32 {
        run_supervisor_stop_handle_cycle(&probe, &format!("supervisor-handle-stop-{cycle}"));
        stop_samples.push(current_process_handle_count());
    }
    assert_handle_samples_bounded("stop", baseline, &stop_samples);
    let final_stop = *stop_samples
        .last()
        .expect("stop handle samples should not be empty");

    println!(
        "baseline_handles={baseline} final_natural_handles={final_natural} final_stop_handles={final_stop}"
    );
}

fn run_supervisor_natural_handle_cycle(probe: &CompiledProbe, label: &str) {
    let core_directory = ProbeTestDirectory::create().expect("create natural-cycle Core root");
    let session_directory =
        ProbeTestDirectory::create().expect("create natural-cycle session root");
    let core = CoreState::open_at(core_directory.path()).expect("open natural-cycle Core");
    let mut supervisor = SessionSupervisor::new(core);
    let (spec, _) = sleep_spec(probe, &session_directory, label, 0);
    let session_id = spec.session_id();
    let deadline = Instant::now() + Duration::from_secs(10);
    supervisor
        .start_spec_for_test(spec, deadline)
        .expect("start natural handle-cycle session");

    loop {
        match supervisor
            .observe_session(session_id, deadline)
            .expect("observe natural handle-cycle session")
        {
            SessionObservation::Running { .. } => {
                assert!(
                    Instant::now() < deadline,
                    "natural handle-cycle root did not exit before deadline"
                );
                thread::sleep(Duration::from_millis(5));
            }
            SessionObservation::Exited(_) => break,
            SessionObservation::CleanupPending { .. } => {
                panic!("natural handle-cycle session cannot be cleanup-pending")
            }
        }
    }
    assert_eq!(supervisor.active_session_count(), 0);
    drop(supervisor);
    drop(session_directory);
    drop(core_directory);
}

fn run_supervisor_stop_handle_cycle(probe: &CompiledProbe, label: &str) {
    let core_directory = ProbeTestDirectory::create().expect("create stop-cycle Core root");
    let session_directory = ProbeTestDirectory::create().expect("create stop-cycle session root");
    let core = CoreState::open_at(core_directory.path()).expect("open stop-cycle Core");
    let mut supervisor = SessionSupervisor::new(core);
    let (spec, ready_path) = sleep_spec(probe, &session_directory, label, 300_000);
    let session_id = spec.session_id();
    let deadline = Instant::now() + Duration::from_secs(10);
    supervisor
        .start_spec_for_test(spec, deadline)
        .expect("start stop handle-cycle session");
    wait_for_ready_path(&ready_path, deadline).expect("stop handle-cycle readiness");
    supervisor
        .stop_session(session_id, deadline)
        .expect("stop handle-cycle session");
    assert_eq!(supervisor.active_session_count(), 0);
    drop(supervisor);
    drop(session_directory);
    drop(core_directory);
}

fn current_process_handle_count() -> u32 {
    let mut count = 0u32;
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle and count is writable for the call.
    let queried = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    assert_ne!(
        queried,
        0,
        "GetProcessHandleCount failed: {}",
        io::Error::last_os_error()
    );
    count
}

fn assert_handle_samples_bounded(phase: &str, baseline: u32, samples: &[u32]) {
    let limit = baseline + 2;
    if let Some((cycle, observed)) = samples
        .iter()
        .copied()
        .enumerate()
        .find(|(_, observed)| *observed > limit)
    {
        panic!(
            "{phase} Supervisor handle count first exceeded its bound after cycle {}: baseline={baseline}, limit={limit}, observed={observed}",
            cycle + 1
        );
    }
    assert!(
        samples.last().is_some_and(|observed| *observed <= limit),
        "{phase} Supervisor final handle count exceeded baseline={baseline} limit={limit}"
    );
}

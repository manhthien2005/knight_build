use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, INFINITE, WaitForSingleObject,
};

use crate::ProcessLaunchSpec;
use crate::launch_snapshot::fixed_test_environment;

use super::test_support::*;
use super::{
    TestFailurePoint, TestTerminationRequest, WindowsProcessErrorKind, WindowsProcessStage,
    WindowsSpawnFailure, encode_environment_block, spawn, spawn_for_test, wait_millis,
};

#[test]
fn probe_compiles_with_pinned_toolchain() {
    let probe = compile_probe();
    let report_path = probe._directory.path().join("report.bin");
    let temp_value = probe._directory.temp_path().as_os_str();
    let output = Command::new(&probe.executable)
        .arg("report")
        .arg(&report_path)
        .arg("0")
        .arg("23")
        .arg("--")
        .arg("alpha")
        .arg("Đường dẫn")
        .env_clear()
        .env("TEMP", temp_value)
        .env("TMP", temp_value)
        .env("TMPDIR", temp_value)
        .env("ZHSO_PROBE_TEST", "Unicode-✓")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("compiled probe should run directly");

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"ZHSO-PROBE-STDOUT\n");
    assert_eq!(output.stderr, b"ZHSO-PROBE-STDERR\n");
    let record = decode_probe_record(&report_path).expect("valid report record should decode");
    assert_eq!(record.magic, *b"ZHSOPRB1");
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.record_type, REPORT_RECORD_TYPE);
    assert_eq!(record.numeric_fields.len(), REPORT_NUMERIC_FIELD_COUNT);
    assert_eq!(&record.strings[..2], &["alpha", "Đường dẫn"]);
    assert_eq!(record.numeric_fields[0], 2);
    assert_eq!(record.numeric_fields[1], 4);
    assert_eq!(record.strings.len(), 6);
    for expected in [
        environment_entry("TEMP", probe._directory.temp_path().as_os_str()),
        environment_entry("TMP", probe._directory.temp_path().as_os_str()),
        environment_entry("TMPDIR", probe._directory.temp_path().as_os_str()),
        OsString::from("ZHSO_PROBE_TEST=Unicode-✓"),
    ] {
        assert!(record.strings[2..].contains(&expected));
    }
    assert_eq!(&record.numeric_fields[2..7], &[1, 1, 1, 0, 23]);
    assert_ne!(record.numeric_fields[7], 0);
    assert!(probe.compiler.is_absolute());
    assert!(probe.linker.is_absolute());
    assert!(probe.version.lines().any(|line| line == "release: 1.98.0"));
    println!("probe_compiler={}", probe.compiler.display());
    println!("probe_linker={}", probe.linker.display());
    println!("probe_output={}", probe.executable.display());
}

fn environment_entry(name: &str, value: &OsStr) -> OsString {
    let mut entry = OsString::from(name);
    entry.push("=");
    entry.push(value);
    entry
}

fn invocation_path(canonical: &Path) -> PathBuf {
    let units = canonical.as_os_str().encode_wide().collect::<Vec<_>>();
    let prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    assert!(units.starts_with(&prefix));
    let invocation = PathBuf::from(OsString::from_wide(&units[prefix.len()..]));
    assert!(
        !invocation
            .as_os_str()
            .encode_wide()
            .collect::<Vec<_>>()
            .starts_with(&prefix)
    );
    assert_eq!(fs::canonicalize(&invocation).unwrap(), canonical);
    invocation
}

#[test]
fn normal_exit_round_trips_isolated_process_state() {
    let parent_path = std::env::var_os("PATH").expect("test parent PATH must exist");
    assert!(
        !parent_path.is_empty(),
        "test parent PATH must not be empty"
    );

    let probe = compile_probe();
    assert!(
        probe
            .executable
            .as_os_str()
            .encode_wide()
            .any(|unit| unit == b' ' as u16),
        "probe executable path must contain a space"
    );
    let report_path = probe._directory.path().join("contained report.bin");
    let sentinel = inheritable_sentinel_event();
    let sentinel_value = (sentinel.as_raw_handle() as usize).to_string();
    let payload = vec![
        OsString::from(""),
        OsString::from("value with spaces"),
        OsString::from("value\twith\ttabs"),
        OsString::from(r#"literal"quote"#),
        OsString::from(r#"trailing backslash\"#),
        OsString::from(r#"before\\\"after"#),
        OsString::from("Đường dẫn Unicode ✓"),
    ];
    let mut arguments = vec![
        OsString::from("report"),
        report_path.as_os_str().to_owned(),
        OsString::from(sentinel_value),
        OsString::from("37"),
        OsString::from("--"),
    ];
    arguments.extend(payload.iter().cloned());
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        arguments,
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("contained probe spec should seal");
    let deadline = Instant::now() + Duration::from_secs(10);

    let mut process = spawn(spec, deadline).expect("contained probe should spawn");
    let root_exit = process
        .wait_root(deadline)
        .expect("contained probe root should exit");

    assert_eq!(root_exit.exit_code, 37);
    assert_ne!(root_exit.identity.pid, 0);
    assert_ne!(root_exit.identity.creation_time_100ns, 0);
    let record = decode_probe_record(&report_path).expect("probe report should decode");
    assert_eq!(record.record_type, REPORT_RECORD_TYPE);
    assert_eq!(record.numeric_fields.len(), REPORT_NUMERIC_FIELD_COUNT);
    assert_eq!(record.numeric_fields[0], payload.len() as u64);
    assert_eq!(&record.strings[..payload.len()], payload.as_slice());
    // Four, not three: the child also receives `SystemRoot`, without which WinSock cannot load its
    // name-resolution providers and the client never reaches a server. Observed in the real child's
    // own environment, so this is the end of the chain rather than a claim about the spec.
    assert_eq!(
        record.numeric_fields[1],
        crate::launch_snapshot::ENVIRONMENT_VARIABLE_COUNT as u64
    );
    let mut child_environment = record.strings[payload.len()..].to_vec();
    child_environment.sort();
    let canonical_temp = probe._directory.temp_path();
    let invocation_temp = invocation_path(canonical_temp);
    let system_root = crate::launch_snapshot::system_root_directory();
    let mut expected_environment = vec![
        environment_entry("TEMP", invocation_temp.as_os_str()),
        environment_entry("TMP", invocation_temp.as_os_str()),
        environment_entry("TMPDIR", invocation_temp.as_os_str()),
        environment_entry("SystemRoot", system_root.as_os_str()),
    ];
    expected_environment.sort();
    assert_eq!(child_environment, expected_environment);
    // Every profile-scoped value resolves to the one profile temp directory. `SystemRoot` is the one
    // entry that must not, so it is asserted separately: it points at the machine directory the
    // loader needs, and nothing inside the profile.
    for entry in &child_environment {
        let units = entry.as_os_str().encode_wide().collect::<Vec<_>>();
        let separator = units.iter().position(|unit| *unit == b'=' as u16).unwrap();
        let name = OsString::from_wide(&units[..separator]);
        let value = PathBuf::from(OsString::from_wide(&units[separator + 1..]));
        if name == "SystemRoot" {
            assert_eq!(value, system_root);
            assert_ne!(fs::canonicalize(&value).unwrap(), canonical_temp);
        } else {
            assert_eq!(fs::canonicalize(value).unwrap(), canonical_temp);
        }
    }
    assert!(
        !record.strings[payload.len()..].iter().any(|entry| {
            entry
                .as_os_str()
                .encode_wide()
                .take(5)
                .eq("PATH=".encode_utf16())
        }),
        "contained child must not inherit parent PATH"
    );
    assert_eq!(&record.numeric_fields[2..6], &[1, 1, 1, 0]);
    assert_eq!(record.numeric_fields[6], 37);
    assert_eq!(record.numeric_fields[7], u64::from(root_exit.identity.pid));
    // SAFETY: the sentinel owner keeps this valid event handle alive for the non-blocking wait.
    let sentinel_wait = unsafe { WaitForSingleObject(sentinel.as_raw_handle() as HANDLE, 0) };
    assert_eq!(sentinel_wait, WAIT_TIMEOUT);
}

#[test]
fn hard_stop_confirms_root_and_descendant_exit() {
    let probe = compile_probe();
    let record_base = probe._directory.path().join("contained descendant");
    let ready_path = record_base.with_extension("ready");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("descendant"),
            record_base.as_os_str().to_owned(),
            OsString::from("300000"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("contained descendant probe spec should seal");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut process = spawn(spec, deadline).expect("contained descendant probe should spawn");

    wait_for_ready_path(&ready_path, deadline)
        .expect("descendant readiness should appear before the absolute deadline");
    let record = decode_probe_record(&ready_path).expect("descendant readiness should decode");
    assert_eq!(record.record_type, DESCENDANT_RECORD_TYPE);
    assert_eq!(record.numeric_fields.len(), DESCENDANT_NUMERIC_FIELD_COUNT);
    let root_pid = u32::try_from(record.numeric_fields[0]).expect("root PID must fit u32");
    let root_creation_time = record.numeric_fields[1];
    let descendant_pid =
        u32::try_from(record.numeric_fields[2]).expect("descendant PID must fit u32");
    let descendant_creation_time = record.numeric_fields[3];
    assert_eq!(process.identity.pid, root_pid);
    assert_eq!(process.identity.creation_time_100ns, root_creation_time);
    let root_observation = open_identity_checked_process(root_pid, root_creation_time);
    let descendant_observation =
        open_identity_checked_process(descendant_pid, descendant_creation_time);

    let root_exit = process
        .terminate_tree_and_wait(deadline)
        .expect("hard stop must confirm both root exit and an empty Job");

    assert_eq!(root_exit.identity.pid, root_pid);
    assert_eq!(root_exit.identity.creation_time_100ns, root_creation_time);
    wait_for_process_observation(&root_observation, deadline);
    wait_for_process_observation(&descendant_observation, deadline);
    assert_eq!(process.last_active_processes_for_test(), Some(0));
    println!(
        "hard_stop_root_pid={} hard_stop_root_creation_time_100ns={} hard_stop_descendant_pid={} hard_stop_descendant_creation_time_100ns={} hard_stop_exit_code={} hard_stop_active_processes=0",
        root_observation.pid,
        root_observation.creation_time_100ns,
        descendant_observation.pid,
        descendant_observation.creation_time_100ns,
        root_exit.exit_code,
    );
}

#[test]
fn drop_is_non_blocking_and_kills_root_and_descendant() {
    let probe = compile_probe();
    let record_base = probe._directory.path().join("dropped descendant");
    let ready_path = record_base.with_extension("ready");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("descendant"),
            record_base.as_os_str().to_owned(),
            OsString::from("300000"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("dropped descendant probe spec should seal");
    let ready_deadline = Instant::now() + Duration::from_secs(10);
    let owner = spawn(spec, ready_deadline).expect("dropped descendant probe should spawn");

    wait_for_ready_path(&ready_path, ready_deadline)
        .expect("descendant readiness should appear before the absolute deadline");
    let record = decode_probe_record(&ready_path).expect("descendant readiness should decode");
    assert_eq!(record.record_type, DESCENDANT_RECORD_TYPE);
    assert_eq!(record.numeric_fields.len(), DESCENDANT_NUMERIC_FIELD_COUNT);
    let root_pid = u32::try_from(record.numeric_fields[0]).expect("root PID must fit u32");
    let root_creation_time = record.numeric_fields[1];
    let descendant_pid =
        u32::try_from(record.numeric_fields[2]).expect("descendant PID must fit u32");
    let descendant_creation_time = record.numeric_fields[3];
    assert_eq!(owner.identity.pid, root_pid);
    assert_eq!(owner.identity.creation_time_100ns, root_creation_time);
    let root_observation = open_identity_checked_process(root_pid, root_creation_time);
    let descendant_observation =
        open_identity_checked_process(descendant_pid, descendant_creation_time);

    let started = Instant::now();
    drop(owner);
    let drop_elapsed = started.elapsed();
    assert!(drop_elapsed < Duration::from_secs(1));

    let observation_deadline = Instant::now() + Duration::from_secs(5);
    wait_for_process_observation(&root_observation, observation_deadline);
    wait_for_process_observation(&descendant_observation, observation_deadline);
    let root_exit_code = process_observation_exit_code(&root_observation);
    let descendant_exit_code = process_observation_exit_code(&descendant_observation);
    assert_eq!(root_exit_code, 0xffff_fffd);
    assert_eq!(descendant_exit_code, 0xffff_fffd);
    println!(
        "drop_root_pid={} drop_root_creation_time_100ns={} drop_descendant_pid={} drop_descendant_creation_time_100ns={} drop_elapsed_millis={} drop_root_exit_code={} drop_descendant_exit_code={}",
        root_observation.pid,
        root_observation.creation_time_100ns,
        descendant_observation.pid,
        descendant_observation.creation_time_100ns,
        drop_elapsed.as_millis(),
        root_exit_code,
        descendant_exit_code,
    );
}

fn current_process_handle_count() -> u32 {
    let mut count = 0u32;
    // SAFETY: GetCurrentProcess returns the current-process pseudo-handle, which remains valid for
    // this call, and `count` is correctly sized writable storage live for the complete call.
    let queried = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    assert_ne!(
        queried,
        0,
        "GetProcessHandleCount failed: {}",
        io::Error::last_os_error()
    );
    count
}

fn run_normal_handle_cycle(probe: &CompiledProbe, record_name: &str) {
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("sleep"),
            probe._directory.path().join(record_name).into_os_string(),
            OsString::from("0"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("normal handle-cycle probe spec should seal");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut process = spawn(spec, deadline).expect("normal handle-cycle probe should spawn");
    process
        .wait_root(deadline)
        .expect("normal handle-cycle probe should exit");
    drop(process);
}

fn run_terminate_handle_cycle(probe: &CompiledProbe, record_name: &str) {
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("sleep"),
            probe._directory.path().join(record_name).into_os_string(),
            OsString::from("300000"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("terminate handle-cycle probe spec should seal");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut process = spawn(spec, deadline).expect("terminate handle-cycle probe should spawn");
    process
        .terminate_tree_and_wait(deadline)
        .expect("terminate handle-cycle probe should be synchronously terminated");
    drop(process);
}

#[test]
fn try_wait_root_is_non_blocking_and_preserves_birth_identity() {
    let probe = compile_probe();
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("sleep"),
            probe
                ._directory
                .path()
                .join("try-wait-root")
                .into_os_string(),
            OsString::from("300000"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("try-wait-root probe spec should seal");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut owner = spawn(spec, deadline).expect("spawn contained sleep probe");

    let birth = owner.birth_id();
    assert_ne!(birth.pid(), 0);
    assert_ne!(birth.creation_time_100ns(), 0);
    assert_eq!(owner.try_wait_root().expect("observe live root"), None);

    let exit = owner
        .terminate_tree_and_wait(deadline)
        .expect("terminate observed root");
    assert_eq!(exit.identity(), birth);
    assert_eq!(exit.exit_code(), 0xffff_fffd);
    assert_eq!(
        owner.try_wait_root().expect("observe exited root"),
        Some(exit)
    );
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
            "{phase} handle count first exceeded its bound after cycle {}: baseline={baseline}, limit={limit}, observed={observed}",
            cycle + 1
        );
    }
    let final_count = samples
        .last()
        .copied()
        .expect("handle-count phase must retain at least one sample");
    assert!(
        final_count <= limit,
        "{phase} final handle count exceeded its bound: baseline={baseline}, limit={limit}, observed={final_count}"
    );
}

#[test]
#[ignore = "run as the dedicated serialized Windows handle-leak gate"]
fn windows_handle_count_stays_bounded() {
    let probe = compile_probe();

    run_normal_handle_cycle(&probe, "handle-warmup-normal");
    run_terminate_handle_cycle(&probe, "handle-warmup-terminate");
    let baseline = current_process_handle_count();

    let mut normal_samples = Vec::with_capacity(64);
    for cycle in 0..64 {
        run_normal_handle_cycle(&probe, &format!("handle-normal-{cycle}"));
        normal_samples.push(current_process_handle_count());
    }
    assert_handle_samples_bounded("normal", baseline, &normal_samples);
    let normal_count = *normal_samples
        .last()
        .expect("normal samples must not be empty");

    let mut terminate_samples = Vec::with_capacity(64);
    for cycle in 0..64 {
        run_terminate_handle_cycle(&probe, &format!("handle-terminate-{cycle}"));
        terminate_samples.push(current_process_handle_count());
    }
    assert_handle_samples_bounded("terminate", baseline, &terminate_samples);
    let terminate_count = *terminate_samples
        .last()
        .expect("terminate samples must not be empty");

    println!(
        "baseline_handles={baseline} final_normal_handles={normal_count} final_terminate_handles={terminate_count}"
    );
}

fn startup_failure_spec(probe: &CompiledProbe, record_name: &str) -> ProcessLaunchSpec {
    ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("sleep"),
            probe._directory.path().join(record_name).into_os_string(),
            OsString::from("300000"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("startup-failure probe spec should seal")
}

#[test]
fn startup_failure_before_assignment_confirms_root_termination_before_rejection() {
    let probe = compile_probe();
    let spec = startup_failure_spec(&probe, "pre-assignment failure");
    let deadline = Instant::now() + Duration::from_secs(10);

    let (result, trace) = spawn_for_test(spec, deadline, TestFailurePoint::BeforeAssign, false);

    assert!(matches!(
        result,
        Err(WindowsSpawnFailure::Rejected(error))
            if error.kind == WindowsProcessErrorKind::LaunchFailed
                && error.stage == WindowsProcessStage::AssignJob
    ));
    assert_eq!(
        trace.termination_request,
        Some(TestTerminationRequest::Root)
    );
    assert!(trace.root_signaled_before_return);
    assert!(trace.root_observation_is_signaled());
    assert_eq!(trace.active_processes_before_return, None);
}

#[test]
fn startup_failure_after_assignment_confirms_root_and_empty_job_before_rejection() {
    let probe = compile_probe();
    let spec = startup_failure_spec(&probe, "contained failure");
    let deadline = Instant::now() + Duration::from_secs(10);

    let (result, trace) = spawn_for_test(
        spec,
        deadline,
        TestFailurePoint::AfterAssignBeforeResume,
        false,
    );

    assert!(matches!(
        result,
        Err(WindowsSpawnFailure::Rejected(error))
            if error.kind == WindowsProcessErrorKind::LaunchFailed
                && error.stage == WindowsProcessStage::ResumeThread
    ));
    assert_eq!(trace.termination_request, Some(TestTerminationRequest::Job));
    assert!(trace.root_signaled_before_return);
    assert!(trace.root_observation_is_signaled());
    assert_eq!(trace.active_processes_before_return, Some(0));
}

#[test]
fn startup_failure_suppressed_confirmation_returns_live_owner_for_retry() {
    let probe = compile_probe();
    let spec = startup_failure_spec(&probe, "suppressed confirmation");
    let first_deadline = Instant::now() + Duration::from_secs(10);

    let (result, trace) = spawn_for_test(
        spec,
        first_deadline,
        TestFailurePoint::AfterAssignBeforeResume,
        true,
    );

    let mut owner = match result {
        Err(WindowsSpawnFailure::CleanupUnconfirmed { error, owner }) => {
            assert_eq!(error.kind, WindowsProcessErrorKind::CleanupUnconfirmed);
            assert_eq!(error.stage, WindowsProcessStage::ResumeThread);
            owner
        }
        _ => panic!("suppressed confirmation must return the live cleanup owner"),
    };
    assert_eq!(trace.termination_request, Some(TestTerminationRequest::Job));
    assert!(!trace.root_signaled_before_return);
    assert_eq!(trace.active_processes_before_return, None);

    owner
        .retry_cleanup(Instant::now() + Duration::from_secs(10))
        .expect("retry must confirm the already-terminated contained root and empty Job");
    assert!(trace.root_observation_is_signaled());
}

#[test]
fn expired_spawn_deadline_rejects_before_probe_output() {
    let probe = compile_probe();
    let report_path = probe._directory.path().join("expired report.bin");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("report"),
            report_path.as_os_str().to_owned(),
            OsString::from("0"),
            OsString::from("0"),
            OsString::from("--"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("expired-deadline probe spec should seal");

    let result = spawn(spec, Instant::now() - Duration::from_secs(1));

    assert!(matches!(
        result,
        Err(WindowsSpawnFailure::Rejected(error))
            if error.kind == WindowsProcessErrorKind::DeadlineExpired
    ));
    assert!(!report_path.exists());
    assert!(!report_path.with_extension("ready").exists());
}

#[test]
fn missing_sealed_executable_is_rejected_during_live_revalidation() {
    let probe = compile_probe();
    let report_path = probe._directory.path().join("revalidation report.bin");
    let spec = ProcessLaunchSpec::for_windows_test(
        probe.executable.clone(),
        vec![
            OsString::from("report"),
            report_path.as_os_str().to_owned(),
            OsString::from("0"),
            OsString::from("0"),
            OsString::from("--"),
        ],
        probe._directory.path().to_owned(),
        probe._directory.temp_path().to_owned(),
    )
    .expect("live-revalidation probe spec should seal");
    fs::remove_file(&probe.executable).expect("temporary probe executable should be removable");

    let result = spawn(spec, Instant::now() + Duration::from_secs(5));

    assert!(matches!(
        result,
        Err(WindowsSpawnFailure::Rejected(error))
            if error.kind == WindowsProcessErrorKind::LaunchFailed
                && error.stage == WindowsProcessStage::ValidateSpec
    ));
    assert!(!report_path.exists());
    assert!(!report_path.with_extension("ready").exists());
}

fn literal_sleep_record() -> Vec<u8> {
    let mut bytes = b"ZHSOPRB1".to_vec();
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&41u64.to_le_bytes());
    bytes.extend_from_slice(&42u64.to_le_bytes());
    bytes
}

#[test]
fn probe_decoder_rejects_unbounded_or_malformed_records() {
    let valid = literal_sleep_record();
    assert!(decode_probe_bytes(&valid).is_ok());

    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    assert!(decode_probe_bytes(&bad_magic).is_err());

    let mut bad_schema = valid.clone();
    bad_schema[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_probe_bytes(&bad_schema).is_err());

    let mut bad_type = valid.clone();
    bad_type[12..16].copy_from_slice(&99u32.to_le_bytes());
    assert!(decode_probe_bytes(&bad_type).is_err());

    let mut bad_sleep_item_count = valid.clone();
    bad_sleep_item_count[16..20].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode_probe_bytes(&bad_sleep_item_count).is_err());

    assert!(decode_probe_bytes(&valid[..valid.len() - 1]).is_err());
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(decode_probe_bytes(&trailing).is_err());

    let mut unbounded_items = b"ZHSOPRB1".to_vec();
    unbounded_items.extend_from_slice(&1u32.to_le_bytes());
    unbounded_items.extend_from_slice(&REPORT_RECORD_TYPE.to_le_bytes());
    unbounded_items.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_probe_bytes(&unbounded_items).is_err());

    let mut unbounded_string = b"ZHSOPRB1".to_vec();
    unbounded_string.extend_from_slice(&1u32.to_le_bytes());
    unbounded_string.extend_from_slice(&REPORT_RECORD_TYPE.to_le_bytes());
    unbounded_string.extend_from_slice(&1u32.to_le_bytes());
    unbounded_string.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_probe_bytes(&unbounded_string).is_err());
}

#[test]
fn probe_decoder_preserves_utf16_units_and_validates_report_counts() {
    let units = [0x0041u16, 0xD800, 0x0042];
    let mut bytes = b"ZHSOPRB1".to_vec();
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&REPORT_RECORD_TYPE.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    for value in [1u64, 1, 1, 0, 23, 99] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let record = decode_probe_bytes(&bytes).expect("well-shaped report should decode");
    assert_eq!(record.strings[0].encode_wide().collect::<Vec<_>>(), units);

    bytes[20 + 4 + units.len() * 2..20 + 4 + units.len() * 2 + 8]
        .copy_from_slice(&2u64.to_le_bytes());
    assert!(decode_probe_bytes(&bytes).is_err());
}

#[test]
fn probe_sleep_readiness_is_atomic_and_bounded() {
    let probe = compile_probe();
    let record_base = probe._directory.path().join("sleep-record");
    let ready_path = record_base.with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut command = Command::new(&probe.executable);
    command
        .arg("sleep")
        .arg(&record_base)
        .arg("250")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child =
        ProbeChild::spawn(&mut command, deadline).expect("sleep probe should spawn directly");

    wait_for_probe_ready(child.child_mut(), &ready_path, deadline)
        .expect("sleep readiness should appear before the absolute deadline");
    assert_eq!(child.child_mut().try_wait().unwrap(), None);
    let status = child
        .wait_until_exit()
        .expect("sleep probe should be reaped before the same absolute deadline");
    assert!(status.success());
    assert!(!record_base.with_extension("partial").exists());
    let record =
        decode_probe_record(&ready_path).expect("valid sleep readiness record should decode");
    assert_eq!(record.magic, *b"ZHSOPRB1");
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.record_type, SLEEP_RECORD_TYPE);
    assert!(record.strings.is_empty());
    assert_eq!(record.numeric_fields.len(), 2);
    assert_ne!(record.numeric_fields[0], 0);
    assert_ne!(record.numeric_fields[1], 0);
}

#[test]
fn probe_descendant_readiness_records_live_birth_identity() {
    let probe = compile_probe();
    let record_base = probe._directory.path().join("descendant-record");
    let ready_path = record_base.with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut command = Command::new(&probe.executable);
    command
        .arg("descendant")
        .arg(&record_base)
        .arg("250")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child =
        ProbeChild::spawn(&mut command, deadline).expect("descendant probe should spawn directly");

    wait_for_probe_ready(child.child_mut(), &ready_path, deadline)
        .expect("descendant readiness should appear before the absolute deadline");
    assert_eq!(child.child_mut().try_wait().unwrap(), None);
    let status = child
        .wait_until_exit()
        .expect("descendant probe should be reaped before the same absolute deadline");
    assert!(status.success());
    assert!(!record_base.with_extension("partial").exists());
    let record =
        decode_probe_record(&ready_path).expect("valid descendant readiness record should decode");
    assert_eq!(record.magic, *b"ZHSOPRB1");
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.record_type, DESCENDANT_RECORD_TYPE);
    assert!(record.strings.is_empty());
    assert_eq!(record.numeric_fields.len(), 4);
    assert_ne!(record.numeric_fields[0], 0);
    assert_ne!(record.numeric_fields[1], 0);
    assert_ne!(record.numeric_fields[2], 0);
    assert_ne!(record.numeric_fields[3], 0);
    assert_ne!(record.numeric_fields[0], record.numeric_fields[2]);
}

#[test]
fn probe_rejects_lifetime_above_hard_bound_before_readiness() {
    let probe = compile_probe();
    let record_base = probe._directory.path().join("unbounded-sleep-record");
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut command = Command::new(&probe.executable);
    command
        .arg("sleep")
        .arg(&record_base)
        .arg("300001")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child =
        ProbeChild::spawn(&mut command, deadline).expect("sleep probe should spawn directly");

    let status = child
        .wait_until_exit()
        .expect("out-of-range lifetime should be rejected without sleeping");
    assert_eq!(status.code(), Some(254));
    assert!(!record_base.with_extension("partial").exists());
    assert!(!record_base.with_extension("ready").exists());
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

#[test]
fn pure_environment_names_are_sorted_and_block_is_double_nul_terminated() {
    let environment = fixed_test_environment(Path::new(r"C:\Profile Temp"));
    let system_root = crate::launch_snapshot::system_root_directory();
    let system_root = system_root.to_str().expect("SystemRoot is Unicode");

    let encoded = encode_environment_block(&environment).unwrap();

    // Sorted, so `SystemRoot` leads. It is the one entry that is not the profile temp directory:
    // WinSock loads its name-resolution providers from there, and a child without it resolves no host.
    assert_eq!(
        encoded,
        wide(&format!(
            "SystemRoot={system_root}\0TEMP=C:\\Profile Temp\0TMP=C:\\Profile Temp\0TMPDIR=C:\\Profile Temp\0\0"
        ))
    );
    assert_eq!(&encoded[encoded.len() - 2..], &[0, 0]);
    assert_ne!(encoded[encoded.len() - 3], 0);
}

#[test]
fn pure_environment_value_with_nul_is_rejected_without_echoing_value() {
    let environment = fixed_test_environment(Path::new("C:\\secret-marker\0tail"));

    let error = encode_environment_block(&environment).unwrap_err();

    assert_eq!(error.stage, WindowsProcessStage::EncodeEnvironment);
    assert!(!format!("{error:?}").contains("secret-marker"));
    assert!(!error.to_string().contains("secret-marker"));
}

#[test]
fn pure_expired_deadline_maps_to_zero_milliseconds() {
    let now = Instant::now();

    assert_eq!(wait_millis(now - Duration::from_secs(1), now), 0);
}

#[test]
fn pure_positive_sub_millisecond_deadline_rounds_up() {
    let now = Instant::now();

    assert_eq!(wait_millis(now + Duration::from_nanos(1), now), 1);
}

#[test]
fn pure_deadline_above_largest_finite_wait_is_capped() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(u64::from(INFINITE) + 1);

    assert_eq!(wait_millis(deadline, now), INFINITE - 1);
}

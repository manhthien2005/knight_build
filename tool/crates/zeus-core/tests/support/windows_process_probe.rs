use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const PROBE_MAGIC: [u8; 8] = *b"ZHSOPRB1";
const PROBE_SCHEMA_VERSION: u32 = 1;
const REPORT_RECORD_TYPE: u32 = 1;
const SLEEP_RECORD_TYPE: u32 = 2;
const DESCENDANT_RECORD_TYPE: u32 = 3;
const DEFAULT_MAX_LIFETIME_MILLIS: u64 = 300_000;

#[repr(C)]
struct FileTime {
    low_date_time: u32,
    high_date_time: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn GetProcessTimes(
        process: isize,
        creation_time: *mut FileTime,
        exit_time: *mut FileTime,
        kernel_time: *mut FileTime,
        user_time: *mut FileTime,
    ) -> i32;
    fn SetEvent(event: isize) -> i32;
}

fn main() {
    let exit_code = match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("probe error: {error}");
            254
        }
    };
    process::exit(exit_code as i32);
}

fn run() -> io::Result<u32> {
    let mut arguments = env::args_os();
    let _executable = arguments.next();
    let mode = required_argument(&mut arguments, "mode")?;
    if mode == OsStr::new("report") {
        run_report(arguments)
    } else if mode == OsStr::new("sleep") {
        run_sleep(arguments)
    } else if mode == OsStr::new("descendant") {
        run_descendant(arguments)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown probe mode",
        ))
    }
}

fn run_report(mut arguments: impl Iterator<Item = OsString>) -> io::Result<u32> {
    let output = PathBuf::from(required_argument(&mut arguments, "report output")?);
    let sentinel = parse_ascii_u64(&required_argument(&mut arguments, "sentinel handle")?)?;
    let exit_code = parse_ascii_u32(&required_argument(&mut arguments, "exit code")?)?;
    if required_argument(&mut arguments, "payload separator")? != OsStr::new("--") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing payload separator",
        ));
    }

    let payload: Vec<OsString> = arguments.collect();
    let environment: Vec<OsString> = env::vars_os()
        .map(|(key, value)| {
            let mut entry = key;
            entry.push("=");
            entry.push(value);
            entry
        })
        .collect();
    let mut strings = Vec::with_capacity(payload.len() + environment.len());
    strings.extend(payload.iter().cloned());
    strings.extend(environment.iter().cloned());

    let mut input = [0u8; 1];
    let stdin_eof = matches!(io::stdin().read(&mut input), Ok(0));
    let stdout_ok = write_marker(io::stdout(), b"ZHSO-PROBE-STDOUT\n");
    let stderr_ok = write_marker(io::stderr(), b"ZHSO-PROBE-STDERR\n");
    let sentinel_handle = isize::try_from(sentinel)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "sentinel handle overflow"))?;
    // SAFETY: The numeric handle is untrusted test input by design; SetEvent validates it, and
    // the probe only records success or failure without taking ownership of the handle.
    let sentinel_signaled = unsafe { SetEvent(sentinel_handle) } != 0;

    let numeric_fields = [
        payload.len() as u64,
        environment.len() as u64,
        u64::from(stdin_eof),
        u64::from(stdout_ok),
        u64::from(stderr_ok),
        u64::from(sentinel_signaled),
        u64::from(exit_code),
        u64::from(process::id()),
    ];
    let mut file = File::create(output)?;
    write_record(&mut file, REPORT_RECORD_TYPE, &strings, &numeric_fields)?;
    file.flush()?;
    file.sync_all()?;
    Ok(exit_code)
}

fn run_sleep(mut arguments: impl Iterator<Item = OsString>) -> io::Result<u32> {
    let output = PathBuf::from(required_argument(&mut arguments, "sleep output")?);
    let lifetime = optional_lifetime(arguments.next())?;
    let (pid, creation_time) = current_process_identity()?;
    publish_ready(
        &output,
        SLEEP_RECORD_TYPE,
        &[],
        &[u64::from(pid), creation_time],
    )?;
    thread::sleep(lifetime);
    Ok(0)
}

fn run_descendant(mut arguments: impl Iterator<Item = OsString>) -> io::Result<u32> {
    let output = PathBuf::from(required_argument(&mut arguments, "descendant output")?);
    let lifetime = optional_lifetime(arguments.next())?;
    let current_executable = fs::canonicalize(env::current_exe()?)?;
    if !current_executable.is_absolute() {
        return Err(io::Error::other("current probe executable is not absolute"));
    }
    let child_output = child_output_path(&output)?;
    let mut child = Command::new(&current_executable)
        .arg("sleep")
        .arg(&child_output)
        .arg(lifetime.as_millis().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let child_pid = child.id();
    let child_creation_time = child_creation_time(&child)?;
    let (root_pid, root_creation_time) = current_process_identity()?;
    publish_ready(
        &output,
        DESCENDANT_RECORD_TYPE,
        &[],
        &[
            u64::from(root_pid),
            root_creation_time,
            u64::from(child_pid),
            child_creation_time,
        ],
    )?;

    thread::sleep(lifetime);
    let _ = child.kill();
    let _ = child.wait();
    Ok(0)
}

fn required_argument(
    arguments: &mut impl Iterator<Item = OsString>,
    name: &str,
) -> io::Result<OsString> {
    arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing probe control argument: {name}"),
        )
    })
}

fn optional_lifetime(value: Option<OsString>) -> io::Result<Duration> {
    let millis = match value {
        Some(value) => parse_ascii_u64(&value)?,
        None => DEFAULT_MAX_LIFETIME_MILLIS,
    };
    if millis > DEFAULT_MAX_LIFETIME_MILLIS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "probe lifetime exceeds hard bound",
        ));
    }
    Ok(Duration::from_millis(millis))
}

fn parse_ascii_u32(value: &OsStr) -> io::Result<u32> {
    u32::try_from(parse_ascii_u64(value)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "numeric value exceeds u32"))
}

fn parse_ascii_u64(value: &OsStr) -> io::Result<u64> {
    let mut result = 0u64;
    let mut any = false;
    for unit in value.encode_wide() {
        if !(b'0' as u16..=b'9' as u16).contains(&unit) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "numeric value is not ASCII decimal",
            ));
        }
        any = true;
        result = result
            .checked_mul(10)
            .and_then(|current| current.checked_add(u64::from(unit - b'0' as u16)))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "numeric value overflow"))?;
    }
    if !any {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "numeric value is empty",
        ));
    }
    Ok(result)
}

fn write_marker(mut output: impl Write, marker: &[u8]) -> bool {
    output
        .write_all(marker)
        .and_then(|()| output.flush())
        .is_ok()
}

fn publish_ready(
    output: &Path,
    record_type: u32,
    strings: &[OsString],
    numeric_fields: &[u64],
) -> io::Result<()> {
    let partial = output.with_extension("partial");
    let ready = output.with_extension("ready");
    let mut file = File::create(&partial)?;
    write_record(&mut file, record_type, strings, numeric_fields)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(partial, ready)
}

fn write_record(
    output: &mut impl Write,
    record_type: u32,
    strings: &[OsString],
    numeric_fields: &[u64],
) -> io::Result<()> {
    output.write_all(&PROBE_MAGIC)?;
    output.write_all(&PROBE_SCHEMA_VERSION.to_le_bytes())?;
    output.write_all(&record_type.to_le_bytes())?;
    let item_count = u32::try_from(strings.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many probe strings"))?;
    output.write_all(&item_count.to_le_bytes())?;
    for value in strings {
        let units: Vec<u16> = value.as_os_str().encode_wide().collect();
        let unit_count = u32::try_from(units.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "probe string is too long"))?;
        output.write_all(&unit_count.to_le_bytes())?;
        for unit in units {
            output.write_all(&unit.to_le_bytes())?;
        }
    }
    for field in numeric_fields {
        output.write_all(&field.to_le_bytes())?;
    }
    Ok(())
}

fn child_output_path(output: &Path) -> io::Result<PathBuf> {
    let file_name = output
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output has no file name"))?;
    let mut child_name = file_name.to_os_string();
    child_name.push("-child");
    Ok(output.with_file_name(child_name))
}

fn current_process_identity() -> io::Result<(u32, u64)> {
    // SAFETY: GetCurrentProcess returns a process pseudo-handle valid in this process; it is used
    // only for the duration of GetProcessTimes and is never closed or transferred.
    let handle = unsafe { GetCurrentProcess() };
    creation_time(handle).map(|creation_time| (process::id(), creation_time))
}

fn child_creation_time(child: &Child) -> io::Result<u64> {
    creation_time(child.as_raw_handle() as isize)
}

fn creation_time(process_handle: isize) -> io::Result<u64> {
    let mut creation = FileTime {
        low_date_time: 0,
        high_date_time: 0,
    };
    let mut exit = FileTime {
        low_date_time: 0,
        high_date_time: 0,
    };
    let mut kernel = FileTime {
        low_date_time: 0,
        high_date_time: 0,
    };
    let mut user = FileTime {
        low_date_time: 0,
        high_date_time: 0,
    };
    // SAFETY: The handle is a live current-process pseudo-handle or a live Child process handle.
    // All four FILETIME pointers are valid, writable, correctly sized, and live for the call.
    let succeeded = unsafe {
        GetProcessTimes(
            process_handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(creation.high_date_time) << 32) | u64::from(creation.low_date_time))
}

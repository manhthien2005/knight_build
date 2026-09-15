use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::{CoreResult, MAX_PROCESS_ARGUMENT_NATIVE_UNITS, MAX_PROCESS_ARGUMENTS};

use super::{MAX_WINDOWS_COMMAND_LINE_UNITS, spec_error};

const VERBATIM_LOCAL_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];

pub(crate) fn local_drive_invocation_path(canonical: &Path) -> CoreResult<PathBuf> {
    let units = canonical.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.contains(&0)
        || !units.starts_with(&VERBATIM_LOCAL_PREFIX)
        || units.len() < 7
        || !matches!(units[4], 65..=90 | 97..=122)
        || units[5] != b':' as u16
        || units[6] != b'\\' as u16
    {
        return Err(spec_error("process_invocation_path_invalid"));
    }
    let invocation = PathBuf::from(OsString::from_wide(&units[VERBATIM_LOCAL_PREFIX.len()..]));
    let canonical_identity =
        fs::canonicalize(canonical).map_err(|_| spec_error("process_invocation_path_invalid"))?;
    let invocation_identity =
        fs::canonicalize(&invocation).map_err(|_| spec_error("process_invocation_path_invalid"))?;
    if !invocation.is_absolute()
        || !exact_path_units(&canonical_identity, canonical)
        || !exact_path_units(&invocation_identity, canonical)
    {
        return Err(spec_error("process_invocation_path_invalid"));
    }
    Ok(invocation)
}

fn exact_path_units(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .encode_wide()
        .eq(right.as_os_str().encode_wide())
}

pub(crate) fn encode_application_name(executable: &Path) -> CoreResult<Vec<u16>> {
    let mut encoded = encode_without_nul(executable.as_os_str(), "process_executable_invalid")?;
    encoded
        .len()
        .checked_add(1)
        .ok_or_else(|| spec_error("process_command_line_too_long"))?;
    encoded.push(0);
    Ok(encoded)
}

pub(crate) fn encode_command_line(
    executable: &Path,
    arguments: &[OsString],
) -> CoreResult<Vec<u16>> {
    if arguments.len() > MAX_PROCESS_ARGUMENTS {
        return Err(spec_error("process_argument_count_exceeded"));
    }

    let executable = encode_without_nul(executable.as_os_str(), "process_executable_invalid")?;
    if executable.contains(&(b'"' as u16)) {
        return Err(spec_error("process_executable_invalid"));
    }

    let mut output = Vec::new();
    let quote_argv_zero = executable
        .iter()
        .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16);
    if quote_argv_zero {
        push_repeated(&mut output, b'"' as u16, 1)?;
    }
    push_slice(&mut output, &executable)?;
    if quote_argv_zero {
        push_repeated(&mut output, b'"' as u16, 1)?;
    }

    for argument in arguments {
        push_repeated(&mut output, b' ' as u16, 1)?;
        let encoded = encode_one_argument(argument.as_os_str())?;
        push_slice(&mut output, &encoded)?;
    }

    output.push(0);
    Ok(output)
}

fn encode_one_argument(value: &OsStr) -> CoreResult<Vec<u16>> {
    let units = encode_without_nul(value, "process_argument_invalid")?;
    if units.len() > MAX_PROCESS_ARGUMENT_NATIVE_UNITS {
        return Err(spec_error("process_argument_too_long"));
    }

    let quote = units.is_empty()
        || units
            .iter()
            .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16 || *unit == b'"' as u16);
    if !quote {
        return Ok(units);
    }

    let mut output = Vec::new();
    push_repeated(&mut output, b'"' as u16, 1)?;
    let mut index = 0usize;
    while index < units.len() {
        let slash_start = index;
        while index < units.len() && units[index] == b'\\' as u16 {
            index += 1;
        }
        let slash_count = index - slash_start;

        if index == units.len() {
            let doubled = slash_count
                .checked_mul(2)
                .ok_or_else(|| spec_error("process_command_line_too_long"))?;
            push_repeated(&mut output, b'\\' as u16, doubled)?;
            break;
        }

        if units[index] == b'"' as u16 {
            let escaped = slash_count
                .checked_mul(2)
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| spec_error("process_command_line_too_long"))?;
            push_repeated(&mut output, b'\\' as u16, escaped)?;
            push_repeated(&mut output, b'"' as u16, 1)?;
        } else {
            push_repeated(&mut output, b'\\' as u16, slash_count)?;
            push_repeated(&mut output, units[index], 1)?;
        }
        index += 1;
    }
    push_repeated(&mut output, b'"' as u16, 1)?;
    Ok(output)
}

fn encode_without_nul(value: &OsStr, error_code: &'static str) -> CoreResult<Vec<u16>> {
    let encoded: Vec<u16> = value.encode_wide().collect();
    if encoded.contains(&0) {
        return Err(spec_error(error_code));
    }
    Ok(encoded)
}

fn push_slice(output: &mut Vec<u16>, value: &[u16]) -> CoreResult<()> {
    let final_len = output
        .len()
        .checked_add(value.len())
        .ok_or_else(|| spec_error("process_command_line_too_long"))?;
    if final_len >= MAX_WINDOWS_COMMAND_LINE_UNITS {
        return Err(spec_error("process_command_line_too_long"));
    }
    output.extend_from_slice(value);
    Ok(())
}

fn push_repeated(output: &mut Vec<u16>, unit: u16, count: usize) -> CoreResult<()> {
    let final_len = output
        .len()
        .checked_add(count)
        .ok_or_else(|| spec_error("process_command_line_too_long"))?;
    if final_len >= MAX_WINDOWS_COMMAND_LINE_UNITS {
        return Err(spec_error("process_command_line_too_long"));
    }
    output.resize(final_len, unit);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use crate::CoreError;

    use super::{
        encode_application_name, encode_command_line, encode_one_argument,
        local_drive_invocation_path,
    };

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }

    #[test]
    fn argv_zero_with_spaces_is_quoted_as_one_token() {
        let encoded =
            encode_command_line(Path::new(r"C:\Program Files\Probe\probe.exe"), &[]).unwrap();

        assert_eq!(encoded, wide("\"C:\\Program Files\\Probe\\probe.exe\"\0"));
    }

    #[test]
    fn ordinary_arguments_follow_crt_escaping_rules() {
        let cases = [
            ("", "\"\""),
            ("plain", "plain"),
            ("two words", "\"two words\""),
            ("tab\tvalue", "\"tab\tvalue\""),
            ("ends\\", "ends\\"),
            ("quote\"here", "\"quote\\\"here\""),
            ("slash\\\"quote", "\"slash\\\\\\\"quote\""),
        ];

        for (input, expected) in cases {
            assert_eq!(
                encode_one_argument(OsStr::new(input)).unwrap(),
                wide(expected)
            );
        }
    }

    #[test]
    fn exact_create_process_limit_counts_final_nul() {
        let accepted = format!(r"C:\{}", "x".repeat(32_763));
        assert_eq!(
            encode_command_line(Path::new(&accepted), &[])
                .unwrap()
                .len(),
            32_767
        );

        let rejected = format!(r"C:\{}", "x".repeat(32_764));
        assert!(matches!(
            encode_command_line(Path::new(&rejected), &[]),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_command_line_too_long"
            })
        ));
    }

    #[test]
    fn executable_quote_is_rejected() {
        assert!(matches!(
            encode_command_line(Path::new("C:\\bad\"probe.exe"), &[]),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_executable_invalid"
            })
        ));
    }

    #[test]
    fn embedded_nul_in_executable_or_argument_is_rejected() {
        assert!(matches!(
            encode_application_name(Path::new("C:\\bad\0probe.exe")),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_executable_invalid"
            })
        ));
        assert!(matches!(
            encode_command_line(Path::new(r"C:\Probe\probe.exe"), &["bad\0argument".into()]),
            Err(CoreError::ProcessLaunchSpec {
                code: "process_argument_invalid"
            })
        ));
    }

    #[test]
    fn quoted_trailing_backslashes_are_doubled() {
        assert_eq!(
            encode_one_argument(OsStr::new("two words\\")).unwrap(),
            wide("\"two words\\\\\"")
        );
    }

    #[test]
    fn non_ascii_utf16_is_preserved() {
        assert_eq!(
            encode_one_argument(OsStr::new("Đường dẫn")).unwrap(),
            wide("\"Đường dẫn\"")
        );
    }

    #[test]
    fn encoded_buffers_have_exactly_one_terminal_nul() {
        let application = encode_application_name(Path::new(r"C:\Probe\probe.exe")).unwrap();
        let command_line = encode_command_line(
            Path::new(r"C:\Probe\probe.exe"),
            &["".into(), "two words".into()],
        )
        .unwrap();

        for encoded in [application, command_line] {
            assert_eq!(encoded.last(), Some(&0));
            assert!(!encoded[..encoded.len() - 1].contains(&0));
        }
    }

    #[test]
    fn verbatim_local_drive_path_renders_to_exact_dos_invocation_identity() {
        let canonical = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
        let invocation = local_drive_invocation_path(&canonical).unwrap();
        let units = invocation.as_os_str().encode_wide().collect::<Vec<_>>();

        assert!(invocation.is_absolute());
        assert!(!units.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]));
        assert_eq!(fs::canonicalize(&invocation).unwrap(), canonical);
    }

    #[test]
    fn non_local_or_malformed_device_paths_fail_closed() {
        for rejected in [
            r"\\?\UNC\server\share\payload.jar",
            r"\\.\C:\payload.jar",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\payload.jar",
            r"\\?\C:relative\payload.jar",
            "\\\\?\\C:\\bad\0payload.jar",
        ] {
            assert!(matches!(
                local_drive_invocation_path(Path::new(rejected)),
                Err(CoreError::ProcessLaunchSpec {
                    code: "process_invocation_path_invalid"
                })
            ));
        }
    }
}

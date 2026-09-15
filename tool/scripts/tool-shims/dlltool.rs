use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("cannot locate dlltool shim: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(devtools) = current_exe.parent().and_then(|directory| directory.parent()) else {
        eprintln!("dlltool shim is outside the expected .devtools/bin directory");
        return ExitCode::FAILURE;
    };
    let zig = devtools.join("zig-0.16.0").join("zig.exe");

    match Command::new(zig)
        .arg("dlltool")
        .args(std::env::args_os().skip(1))
        .status()
    {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!("cannot execute Zig dlltool: {error}");
            ExitCode::FAILURE
        }
    }
}

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use serde_json::{Value, json};
use zeus_core::{COMMAND_SCHEMA_VERSION, CoreError, CoreState};

const MAX_ARGUMENT_COUNT: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 4096;
const EXIT_USAGE: u8 = 64;
const EXIT_DOMAIN: u8 = 65;
const EXIT_INTERNAL: u8 = 70;

const USAGE: &str = "zeus-core foundation diagnostic commands:\n\
  [--data-root PATH] init\n\
  [--data-root PATH] runtime register --descriptor PATH\n\
  [--data-root PATH] runtime list [--after ID] [--limit 1..100]\n\
  [--data-root PATH] runtime inspect --runtime-id ID\n\
  [--data-root PATH] profile create --name NAME --runtime-id ID\n\
  [--data-root PATH] profile list [--after UUID] [--limit 1..100] [--include-archived]\n\
  [--data-root PATH] profile inspect --profile-id UUID\n\
  [--data-root PATH] profile rename --profile-id UUID --expected-revision N --name NAME\n\
  [--data-root PATH] profile bind-runtime --profile-id UUID --expected-revision N --runtime-id ID\n\
  [--data-root PATH] profile archive --profile-id UUID --expected-revision N\n\n\
No launch, stop, session, IPC, UI or log commands exist in this milestone.";

fn main() -> ExitCode {
    match execute() {
        Ok(document) => match write_json(io::stdout().lock(), &document) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(EXIT_INTERNAL),
        },
        Err(failure) => {
            let document = json!({
                "schema_version": COMMAND_SCHEMA_VERSION,
                "ok": false,
                "error": {
                    "code": failure.code,
                    "message": failure.message,
                }
            });
            let _ = write_json(io::stderr().lock(), &document);
            ExitCode::from(failure.exit_code)
        }
    }
}

fn execute() -> Result<Value, CliFailure> {
    let mut arguments = read_arguments()?;
    if arguments.is_empty() || matches!(arguments[0].as_str(), "help" | "--help" | "-h") {
        if arguments.len() > 1 {
            return Err(CliFailure::invalid_arguments());
        }
        return success(json!({ "usage": USAGE }));
    }

    let data_root = if arguments
        .first()
        .is_some_and(|value| value == "--data-root")
    {
        if arguments.len() < 3 {
            return Err(CliFailure::invalid_arguments());
        }
        let path = PathBuf::from(arguments[1].clone());
        arguments.drain(0..2);
        Some(path)
    } else {
        None
    };

    let top_level = arguments
        .first()
        .map(String::as_str)
        .ok_or_else(CliFailure::invalid_arguments)?;
    match top_level {
        "init" => execute_init(data_root.as_deref(), &arguments[1..]),
        "runtime" => execute_runtime(data_root.as_deref(), &arguments[1..]),
        "profile" => execute_profile(data_root.as_deref(), &arguments[1..]),
        _ => Err(CliFailure::unknown_command()),
    }
}

fn execute_init(data_root: Option<&Path>, arguments: &[String]) -> Result<Value, CliFailure> {
    if !arguments.is_empty() {
        return Err(CliFailure::invalid_arguments());
    }
    let core = open_core(data_root)?;
    let invariants = core.database_invariants()?;
    success(json!({
        "schema_version": invariants.schema_version,
        "journal_mode": invariants.journal_mode,
        "foreign_keys": invariants.foreign_keys,
        "page_size": invariants.page_size,
        "max_page_count": invariants.max_page_count,
        "connection_count": invariants.connection_count,
    }))
}

fn execute_runtime(data_root: Option<&Path>, arguments: &[String]) -> Result<Value, CliFailure> {
    let action = arguments
        .first()
        .map(String::as_str)
        .ok_or_else(CliFailure::invalid_arguments)?;
    match action {
        "register" => {
            let options = Options::parse(&arguments[1..], &["--descriptor"], &[])?;
            let descriptor = options.required("--descriptor")?;
            let mut core = open_core(data_root)?;
            success_value(core.register_runtime_descriptor(Path::new(descriptor))?)
        }
        "list" => {
            let options = Options::parse(&arguments[1..], &["--after", "--limit"], &[])?;
            let limit = options.parse_u32("--limit", 100)?;
            let core = open_core(data_root)?;
            success_value(core.list_runtimes(options.optional("--after"), limit)?)
        }
        "inspect" => {
            let options = Options::parse(&arguments[1..], &["--runtime-id"], &[])?;
            let runtime_id = options.required("--runtime-id")?;
            let core = open_core(data_root)?;
            success_value(core.inspect_runtime(runtime_id)?)
        }
        _ => Err(CliFailure::unknown_command()),
    }
}

fn execute_profile(data_root: Option<&Path>, arguments: &[String]) -> Result<Value, CliFailure> {
    let action = arguments
        .first()
        .map(String::as_str)
        .ok_or_else(CliFailure::invalid_arguments)?;
    match action {
        "create" => {
            let options = Options::parse(&arguments[1..], &["--name", "--runtime-id"], &[])?;
            let mut core = open_core(data_root)?;
            success_value(core.create_profile(
                options.required("--name")?,
                options.required("--runtime-id")?,
            )?)
        }
        "list" => {
            let options = Options::parse(
                &arguments[1..],
                &["--after", "--limit"],
                &["--include-archived"],
            )?;
            let limit = options.parse_u32("--limit", 100)?;
            let core = open_core(data_root)?;
            success_value(core.list_profiles(
                options.optional("--after"),
                limit,
                options.present("--include-archived"),
            )?)
        }
        "inspect" => {
            let options = Options::parse(&arguments[1..], &["--profile-id"], &[])?;
            let core = open_core(data_root)?;
            success_value(core.inspect_profile(options.required("--profile-id")?)?)
        }
        "rename" => {
            let options = Options::parse(
                &arguments[1..],
                &["--profile-id", "--expected-revision", "--name"],
                &[],
            )?;
            let revision = options.parse_i64_required("--expected-revision")?;
            let mut core = open_core(data_root)?;
            success_value(core.rename_profile(
                options.required("--profile-id")?,
                revision,
                options.required("--name")?,
            )?)
        }
        "bind-runtime" => {
            let options = Options::parse(
                &arguments[1..],
                &["--profile-id", "--expected-revision", "--runtime-id"],
                &[],
            )?;
            let revision = options.parse_i64_required("--expected-revision")?;
            let mut core = open_core(data_root)?;
            success_value(core.bind_profile_runtime(
                options.required("--profile-id")?,
                revision,
                options.required("--runtime-id")?,
            )?)
        }
        "archive" => {
            let options = Options::parse(
                &arguments[1..],
                &["--profile-id", "--expected-revision"],
                &[],
            )?;
            let revision = options.parse_i64_required("--expected-revision")?;
            let mut core = open_core(data_root)?;
            success_value(core.archive_profile(options.required("--profile-id")?, revision)?)
        }
        _ => Err(CliFailure::unknown_command()),
    }
}

fn open_core(data_root: Option<&Path>) -> Result<CoreState, CliFailure> {
    match data_root {
        Some(path) => Ok(CoreState::open_at(path)?),
        None => Ok(CoreState::open_default()?),
    }
}

fn read_arguments() -> Result<Vec<String>, CliFailure> {
    let raw = std::env::args_os().skip(1).collect::<Vec<_>>();
    if raw.len() > MAX_ARGUMENT_COUNT {
        return Err(CliFailure::invalid_arguments());
    }
    raw.into_iter()
        .map(|argument| {
            let argument = argument
                .into_string()
                .map_err(|_| CliFailure::invalid_arguments())?;
            if argument.len() > MAX_ARGUMENT_BYTES || argument.contains('\0') {
                return Err(CliFailure::invalid_arguments());
            }
            Ok(argument)
        })
        .collect()
}

fn success(data: Value) -> Result<Value, CliFailure> {
    Ok(json!({
        "schema_version": COMMAND_SCHEMA_VERSION,
        "ok": true,
        "data": data,
    }))
}

fn success_value<T: Serialize>(data: T) -> Result<Value, CliFailure> {
    let data = serde_json::to_value(data).map_err(|_| CliFailure::internal())?;
    success(data)
}

fn write_json(mut writer: impl Write, document: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut writer, document)?;
    writer.write_all(b"\n")
}

struct Options {
    values: BTreeMap<String, Option<String>>,
}

impl Options {
    fn parse(
        arguments: &[String],
        value_options: &[&str],
        switches: &[&str],
    ) -> Result<Self, CliFailure> {
        let mut values = BTreeMap::new();
        let mut index = 0;
        while index < arguments.len() {
            let name = arguments[index].as_str();
            if values.contains_key(name) {
                return Err(CliFailure::invalid_arguments());
            }
            if value_options.contains(&name) {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(CliFailure::invalid_arguments)?;
                values.insert(name.to_owned(), Some(value.clone()));
                index += 2;
            } else if switches.contains(&name) {
                values.insert(name.to_owned(), None);
                index += 1;
            } else {
                return Err(CliFailure::invalid_arguments());
            }
        }
        Ok(Self { values })
    }

    fn required(&self, name: &str) -> Result<&str, CliFailure> {
        self.optional(name)
            .ok_or_else(CliFailure::invalid_arguments)
    }

    fn optional(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(Option::as_deref)
    }

    fn present(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    fn parse_u32(&self, name: &str, default: u32) -> Result<u32, CliFailure> {
        match self.optional(name) {
            Some(value) => value
                .parse::<u32>()
                .map_err(|_| CliFailure::invalid_arguments()),
            None => Ok(default),
        }
    }

    fn parse_i64_required(&self, name: &str) -> Result<i64, CliFailure> {
        self.required(name)?
            .parse::<i64>()
            .map_err(|_| CliFailure::invalid_arguments())
    }
}

struct CliFailure {
    code: &'static str,
    message: String,
    exit_code: u8,
}

impl CliFailure {
    fn invalid_arguments() -> Self {
        Self {
            code: "InvalidArguments",
            message: "arguments do not match a bounded foundation command; use help".to_owned(),
            exit_code: EXIT_USAGE,
        }
    }

    fn unknown_command() -> Self {
        Self {
            code: "UnknownCommand",
            message: "command is not available in the foundation milestone".to_owned(),
            exit_code: EXIT_USAGE,
        }
    }

    fn internal() -> Self {
        Self {
            code: "InternalFailure",
            message: "internal serialization failed".to_owned(),
            exit_code: EXIT_INTERNAL,
        }
    }
}

impl From<CoreError> for CliFailure {
    fn from(error: CoreError) -> Self {
        let exit_code = if matches!(error, CoreError::Io { .. } | CoreError::Database(_)) {
            EXIT_INTERNAL
        } else {
            EXIT_DOMAIN
        };
        Self {
            code: error.code(),
            message: error.to_string(),
            exit_code,
        }
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read_source(relative: &str) -> String {
    fs::read_to_string(manifest().join(relative))
        .unwrap_or_else(|error| panic!("failed to read repository source {relative}: {error}"))
        .replace("\r\n", "\n")
}

fn workspace_root() -> PathBuf {
    manifest()
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_owned()
}

fn native_ui_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut pending = vec![root.join("crates/zeus-ui/src")];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).unwrap_or_else(|error| {
            panic!(
                "failed to read native UI sources {}: {error}",
                directory.display()
            )
        });
        for entry in entries {
            let entry = entry.expect("native UI source entry");
            let path = entry.path();
            if entry.file_type().expect("native UI source kind").is_dir() {
                pending.push(path);
                continue;
            }
            if !path.extension().is_some_and(|extension| extension == "rs") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .expect("native UI source must remain inside workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| {
                    panic!("failed to read native UI source {relative}: {error}")
                })
                .replace("\r\n", "\n");
            sources.push((relative, source));
        }
    }
    assert!(
        !sources.is_empty(),
        "the native UI package must declare Rust sources"
    );
    sources
}

#[test]
fn foundation_binary_rejects_every_lifecycle_command_with_bounded_json() {
    for command in ["launch", "stop", "session"] {
        let output = Command::new(env!("CARGO_BIN_EXE_zeus-core"))
            .arg(command)
            .output()
            .expect("zeus-core should start");
        assert!(
            !output.status.success(),
            "foundation must reject lifecycle command {command}"
        );
        assert!(output.stdout.is_empty(), "CLI errors must not write stdout");
        let error: Value =
            serde_json::from_slice(&output.stderr).expect("CLI error must be bounded JSON");
        assert_eq!(error["schema_version"], 1);
        assert_eq!(error["ok"], false);
        assert_eq!(error["error"]["code"], "UnknownCommand");
    }
}

#[test]
fn production_process_sources_do_not_use_commands_shells_or_live_admission() {
    let production_sources = [
        "src/process_adapter/mod.rs",
        "src/process_adapter/windows.rs",
        "src/process_launch_spec/windows.rs",
        "src/session_supervisor/mod.rs",
        "src/session_supervisor/backend.rs",
    ];
    let forbidden_tokens = [
        "std::process::Command",
        "Command::new",
        "cmd.exe",
        "powershell",
        "/bin/sh",
        "sh -c",
        "ZEUS_LIVE_RUNTIME_BRIDGE",
    ];

    for relative in production_sources {
        let source = read_source(relative);
        for token in forbidden_tokens {
            assert!(
                !source.contains(token),
                "{relative} contains a forbidden production launch-boundary token"
            );
        }
    }
}

#[test]
fn manager_is_the_exact_windows_public_boundary_and_lower_layers_stay_private() {
    let lib = read_source("src/lib.rs");
    for (module, exact_declaration) in [
        ("process_adapter", "#[cfg(windows)]\nmod process_adapter;"),
        (
            "session_supervisor",
            "#[cfg(windows)]\nmod session_supervisor;",
        ),
    ] {
        assert_eq!(lib.matches(module).count(), 1);
        assert_eq!(lib.matches(exact_declaration).count(), 1);
    }
    assert_eq!(lib.matches("#[cfg(windows)]\nmod manager;").count(), 1);
    assert_eq!(
        lib.matches("#[cfg(windows)]\npub use manager::{").count(),
        1
    );
    for required in [
        "ManagerController",
        "ManagerError",
        "ManagerObservation",
        "ManagerProfileView",
        "ManagerRuntimeView",
        "ManagerSessionView",
    ] {
        assert!(
            lib.contains(required),
            "manager export is missing {required}"
        );
    }
    for forbidden in [
        "pub mod process_adapter",
        "pub use process_adapter",
        "pub mod session_supervisor",
        "pub use session_supervisor",
    ] {
        assert!(
            !lib.contains(forbidden),
            "lower-level process ownership escaped the crate boundary"
        );
    }
    for forbidden in [
        "windows_live_test_support",
        "windows_live_runtime_tests",
        "SessionSupervisor",
        "ProcessObservation",
        "ProcessResourceSample",
        "LiveGateFailure",
        "ReadyWindow",
        "LivePerformanceEvidenceV1",
    ] {
        assert!(
            !lib.contains(forbidden),
            "crate root exposes a forbidden lifecycle or live-observation surface"
        );
    }
    assert_eq!(lib.matches("pub mod runtime;").count(), 1);
    assert!(lib.contains("CoreState"));
    assert!(lib.contains("RuntimePreflightDiagnostics"));
}

#[test]
fn manager_production_sources_exclude_identity_shell_live_ui_ipc_and_workers() {
    let production_sources = [
        "src/manager/mod.rs",
        "src/manager/error.rs",
        "src/manager/types.rs",
    ];
    let forbidden_categories = [
        ("shell", "std::process"),
        ("shell", "command::new"),
        ("shell", "cmd.exe"),
        ("shell", "powershell"),
        ("shell", "/bin/sh"),
        ("live approval", "zeus_"),
        ("process identity", "processbirthid"),
        ("process identity", "creation_time"),
        ("process identity", "exit_code"),
        ("process identity", "os_code"),
        ("worker", "std::thread"),
        ("worker", "thread::"),
        ("worker", "tokio"),
        ("IPC", "named_pipe"),
        ("IPC", "std::sync::mpsc"),
        ("UI", "windowsandmessaging"),
        ("UI", "sendmessagetimeout"),
        ("UI", "enumwindows"),
        ("unsafe", "unsafe {"),
    ];

    for relative in production_sources {
        let source = read_source(relative).to_lowercase();
        for (category, token) in forbidden_categories {
            if relative == "src/manager/mod.rs" && token == "processbirthid" {
                continue;
            }
            assert!(
                !source.contains(token),
                "{relative} contains forbidden {category} token {token}"
            );
        }
    }
}

#[test]
fn manager_worker_sources_keep_the_public_boundary_narrow_and_test_identity_private() {
    // The exact readiness import the worker engine may make. Pinning the whole block, rather than
    // stripping the bare word `login`, is what stops a future automation path from being admitted by
    // renaming it at the import boundary: adding an item here fails the assertions below.
    const APPROVED_READINESS_IMPORT: &str = concat!(
        "use super::login::{\n",
        "    LoginCompletion as ReadinessCompletion, LoginOutcome as ReadinessOutcome,\n",
        "    MAX_LOGIN_TASKS as MAX_READINESS_TASKS,\n",
        "};\n"
    );
    let engine_source = read_source("src/manager/worker/engine.rs");
    assert_eq!(
        engine_source.matches("super::login::").count(),
        1,
        "the worker engine may reference the readiness module exactly once, in its pinned import"
    );
    assert!(
        engine_source.contains(APPROVED_READINESS_IMPORT),
        "the worker engine's readiness import drifted from the approved block"
    );

    let production_sources = [
        "src/manager/worker/mod.rs",
        "src/manager/worker/engine.rs",
        "src/manager/worker/error.rs",
        "src/manager/worker/types.rs",
    ];
    let thread_transport_sources = ["src/manager/worker/mod.rs", "src/manager/worker/engine.rs"];
    let forbidden_categories = [
        ("lower layer", "supervisorcore"),
        ("lower layer", "sessionsupervisor"),
        ("lower layer", "processbackend"),
        ("lower layer", "windowsprocessbackend"),
        ("lower layer", "process_adapter"),
        ("lower layer", "session_supervisor"),
        ("shell", "std::process::command"),
        ("shell", "command::new"),
        ("shell", "cmd.exe"),
        ("shell", "powershell"),
        ("shell", "/bin/sh"),
        ("async", "tokio"),
        ("async", "async fn"),
        ("async", ".await"),
        ("IPC", "named_pipe"),
        ("network", "std::net"),
        ("network", "tcpstream"),
        ("UI", "windowsandmessaging"),
        ("UI", "sendmessagetimeout"),
        ("UI", "enumwindows"),
        ("unsafe", "unsafe {"),
        ("live approval", "zeus_live_runtime_bridge"),
        ("live approval", "windows-live-runtime-bridge-v1-approved"),
        ("credential", "credential"),
        ("credential", "password"),
        ("log", "telemetry"),
        ("log", "tracing"),
        ("log", "log::"),
        ("log", "log!"),
        ("log", "println!"),
        ("log", "eprintln!"),
        ("log", "print!"),
        ("log", "eprint!"),
        ("log", "dbg!"),
        ("log", "env_logger"),
        ("log", "log_message"),
        ("log", "logger"),
        ("log", "logging"),
        ("automation", "sendinput"),
        ("automation", "keybd_event"),
        ("automation", "mouse_event"),
        ("automation", "uiautomation"),
        ("automation", "screenshot"),
        ("automation", "capture_screen"),
        ("automation", "login"),
        ("process identity", "creation_time"),
        ("process identity", "exit_code"),
        ("OS detail", "os_code"),
        ("OS detail", "win32"),
        ("OS detail", "getlasterror"),
    ];

    for relative in production_sources {
        let source = read_source(relative);
        let production_only = source
            .replace(
                "#[cfg(all(test, windows))]\nuse crate::process_adapter::ProcessBirthId;\n",
                "",
            )
            .replace("mod account;\n", "")
            // The module declaration and its aliased import name the readiness engine; the
            // automation-token ban still applies to every other line, including anything that would
            // inject input from here.
            .replace("mod login;\n", "")
            .replace(APPROVED_READINESS_IMPORT, "")
            .replace("pub use account::ManagerAccountPassword;\n", "");
        // Spec section 11 mandates the exact parameter names `password` and `replacement_password`
        // carrying the opaque `ManagerAccountPassword`. Approve only lines that declare or move that
        // typed carrier, so the plaintext-credential ban still applies to everything else.
        let production_only = strip_typed_password_carrier(relative, &production_only);
        let lowercase = production_only.to_lowercase();
        for (category, token) in forbidden_categories {
            assert!(
                !lowercase.contains(token),
                "{relative} contains forbidden {category} token {token}"
            );
        }
        if !thread_transport_sources.contains(&relative) {
            for token in ["std::thread", "std::sync::mpsc"] {
                assert!(
                    !source.contains(token),
                    "{relative} contains worker transport token {token}"
                );
            }
        }
    }

    let engine = read_source("src/manager/worker/engine.rs");
    let worker = read_source("src/manager/worker/mod.rs");
    let worker_struct_start = worker
        .find("pub struct ManagerWorker {")
        .expect("public ManagerWorker struct declaration");
    let worker_struct_end = worker[worker_struct_start..]
        .find("\n}\n\nimpl ManagerWorker")
        .map(|offset| worker_struct_start + offset + 3)
        .expect("public ManagerWorker struct boundary");
    let worker_struct = &worker[worker_struct_start..worker_struct_end];
    assert_eq!(
        worker_struct
            .matches("_not_sync: PhantomData<Cell<()>>,")
            .count(),
        1,
        "ManagerWorker must retain its exact !Sync ownership marker"
    );
    let worker_declaration_start = worker[..worker_struct_start]
        .rfind("\n\n")
        .map(|offset| offset + 2)
        .unwrap_or(0);
    let worker_declaration = &worker[worker_declaration_start..worker_struct_end];
    assert!(
        !worker_declaration.contains("Clone"),
        "ManagerWorker declaration must remain non-cloneable"
    );
    // Defence in depth for the worker source text only. The compiler proof in
    // `manager_worker_handle_stays_send_never_sync_and_never_cloneable` below is the enforced
    // contract. Comment lines are dropped and every whitespace run collapses to a single space, so
    // all path spellings of the trait normalise to one needle and only a space or a path separator
    // may precede it. That rejects `unsafe impl std::marker::Sync for ManagerWorker` while leaving
    // `impl AsyncSync for ManagerWorker` alone: that spelling does contain the needle, but the
    // character in front of it belongs to the trait identifier. The preceding-character rule is the
    // only thing separating those two cases, so do not simplify it away.
    let worker_code = worker
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ");
    for (forbidden_trait, ownership) in [("Sync", "!Sync"), ("Clone", "non-cloneable")] {
        let needle = format!("{forbidden_trait} for ManagerWorker");
        for (offset, _) in worker_code.match_indices(&needle) {
            assert!(
                !matches!(worker_code[..offset].chars().next_back(), Some(' ' | ':')),
                "ManagerWorker must stay {ownership}: worker source implements {forbidden_trait}"
            );
        }
    }
    for guarded in [
        "#[cfg(all(test, windows))]\nuse crate::process_adapter::ProcessBirthId;",
        "#[cfg(all(test, windows))]\n    BirthForLiveTest {",
        "#[cfg(all(test, windows))]\n    fn running_birth_id_for_live_test(&self, _session: &str) -> Option<ProcessBirthId>",
        "#[cfg(all(test, windows))]\n    fn running_birth_id_for_live_test(&self, session: &str) -> Option<ProcessBirthId>",
    ] {
        assert_eq!(
            engine.matches(guarded).count(),
            1,
            "worker engine live identity must have one exact test-only Windows guard"
        );
    }
    assert_eq!(engine.matches("ProcessBirthId").count(), 4);
    assert_eq!(engine.matches("BirthForLiveTest").count(), 2);
    assert_eq!(
        engine
            .matches(
                "#[cfg(all(test, windows))]\n            WorkerCommand::BirthForLiveTest { session_id, reply } => {"
            )
            .count(),
        1,
        "worker live identity command handling must have one exact test-only Windows guard"
    );
    assert_eq!(
        worker
            .matches("#[cfg(all(test, windows))]\n    fn running_birth_id_for_live_test(")
            .count(),
        1,
        "worker live identity method must have one exact test-only Windows guard"
    );
    assert_eq!(worker.matches("ProcessBirthId").count(), 2);
    assert_eq!(worker.matches("WorkerCommand::BirthForLiveTest").count(), 1);
    assert_eq!(worker.matches("sync_channel(1)").count(), 1);
    assert_eq!(
        worker
            .matches("recv_timeout(Duration::from_secs(10))")
            .count(),
        1
    );

    let command_start = engine
        .find("pub(super) enum WorkerCommand {")
        .expect("worker command enum declaration");
    // The boundary is the enum's own closing brace at column zero, not whatever attribute happens to
    // follow it: keying on a neighbouring `#[allow(` silently widened this surface to the whole
    // control trait once that attribute was deleted.
    let command_end = engine[command_start..]
        .find("\n}\n")
        .map(|offset| command_start + offset + 3)
        .expect("worker command enum boundary");
    let command_surface = engine[command_start..command_end].replace(
        "    #[cfg(all(test, windows))]\n    BirthForLiveTest {\n        session_id: String,\n        reply: SyncSender<Option<ProcessBirthId>>,\n    },\n",
        "",
    );
    let event_types = read_source("src/manager/worker/types.rs");
    let account_secret = read_source("src/manager/worker/account.rs");
    for forbidden in [
        "#[derive(Clone",
        "impl Clone for ManagerAccountPassword",
        "pub fn as_str",
        "pub fn as_bytes",
        "println!",
        "eprintln!",
        "dbg!",
        "tracing",
        "log::",
    ] {
        assert!(
            !account_secret.contains(forbidden),
            "account secret source exposes forbidden token {forbidden}"
        );
    }
    let event_start = event_types
        .find("pub enum ManagerWorkerEvent {")
        .expect("worker event enum declaration");
    let event_end = event_types[event_start..]
        .find("\n}\n")
        .map(|offset| event_start + offset + 3)
        .expect("worker event enum boundary");
    let event_surface = &event_types[event_start..event_end];
    let forbidden_command_event_tokens = [
        ("thread identity", "threadid"),
        ("thread identity", "thread_id"),
        ("native handle", "nativehandle"),
        ("native handle", "rawhandle"),
        ("native handle", "ownedhandle"),
        ("native handle", "native_handle"),
        ("native handle", "handle:"),
        ("process identity", "processbirthid"),
        ("process identity", "process_id"),
        ("process identity", "pid:"),
        ("process identity", "creation_time_100ns"),
        ("process identity", "exit_code"),
        ("descriptor digest", "descriptor_sha256"),
        ("descriptor digest", "descriptor_digest"),
        ("descriptor digest", "descriptor_hash"),
        ("channel implementation", "syncsender"),
        ("channel implementation", "sender<"),
        ("channel implementation", "receiver<"),
        ("channel implementation", "sync_channel"),
        ("channel implementation", "mpsc"),
        ("channel implementation", "try_send"),
        ("channel implementation", "try_recv"),
        ("channel implementation", "channel:"),
        ("panic payload", "panic_payload"),
        ("panic payload", "panic:"),
        ("panic payload", "panic_message"),
        ("panic payload", "backtrace"),
        ("platform detail", "os_code"),
        ("platform detail", "win32_stage"),
    ];
    for (surface_name, surface) in [
        ("worker command", command_surface.as_str()),
        ("public worker event", event_surface),
    ] {
        let lowercase_surface = surface.to_lowercase();
        for (category, token) in forbidden_command_event_tokens {
            assert!(
                !lowercase_surface.contains(token),
                "{surface_name} exposes forbidden {category} token {token}"
            );
        }
    }
    assert_eq!(
        worker
            .matches("#[cfg(all(test, windows))]\nmod windows_live_runtime_tests;")
            .count(),
        1
    );

    let manager = read_source("src/manager/mod.rs");
    assert_eq!(
        manager
            .matches("#[cfg(all(test, windows))]\nuse crate::process_adapter::ProcessBirthId;")
            .count(),
        1
    );
    assert_eq!(
        manager
            .matches("#[cfg(all(test, windows))]\n    pub(super) fn running_birth_id_for_test(")
            .count(),
        1
    );
    assert_eq!(manager.matches("ProcessBirthId").count(), 2);

    let lib = read_source("src/lib.rs");
    for required in [
        "ManagerRequestId",
        "ManagerAccountPassword",
        "ManagerWorker",
        "ManagerWorkerError",
        "ManagerWorkerErrorCode",
        "ManagerWorkerEvent",
        "ManagerWorkerOperation",
        "ManagerWorkerResult",
        "ManagerWorkerState",
    ] {
        assert!(
            lib.contains(required),
            "worker export is missing {required}"
        );
    }
    for forbidden in [
        "running_birth_id_for_live_test",
        "BirthForLiveTest",
        "ProcessBirthId",
        "windows_live_runtime_tests",
    ] {
        assert!(
            !lib.contains(forbidden),
            "crate root exposes private worker live helper {forbidden}"
        );
    }

    let worker_live = "src/manager/worker/windows_live_runtime_tests.rs";
    assert!(
        manifest().join(worker_live).is_file(),
        "worker live source file is missing: {worker_live}"
    );
}

/// Compiler-backed ownership contract for the single-owner Windows worker handle.
///
/// `OwnershipProof` is implemented once for every type at marker `()`, once more for every
/// `Sync` type and once more for every `Clone` type. While `ManagerWorker` is neither, exactly
/// one implementation applies to it and the elided marker below infers to `()`. The moment any
/// spelling — qualified path, `use` alias, `derive`, macro expansion or blanket implementation —
/// makes the handle `Sync` or `Clone`, a second implementation applies, the marker stops
/// inferring, and this test target fails to compile naming the violated marker type.
#[cfg(windows)]
mod worker_ownership {
    use zeus_core::ManagerWorker;

    #[allow(dead_code)]
    struct ManagerWorkerMustStayNotSync;

    #[allow(dead_code)]
    struct ManagerWorkerMustStayNotClone;

    trait OwnershipProof<Forbidden> {
        fn proof() {}
    }

    impl<T: ?Sized> OwnershipProof<()> for T {}

    impl<T: ?Sized + Sync> OwnershipProof<ManagerWorkerMustStayNotSync> for T {}

    impl<T: Clone> OwnershipProof<ManagerWorkerMustStayNotClone> for T {}

    fn assert_send<T: Send>() {}

    pub(super) fn assert_send_only_single_owner_handle() {
        assert_send::<ManagerWorker>();
        let _ = <ManagerWorker as OwnershipProof<_>>::proof;
    }
}

#[cfg(windows)]
#[test]
fn manager_worker_handle_stays_send_never_sync_and_never_cloneable() {
    worker_ownership::assert_send_only_single_owner_handle();
}

#[test]
fn windows_live_modules_are_test_only_with_narrow_manager_fixture_sharing() {
    let supervisor = read_source("src/session_supervisor/mod.rs");
    let exact_declarations = "#[cfg(all(test, windows))]\nmod windows_live_test_support;\n\n\
#[cfg(all(test, windows))]\nmod windows_live_runtime_tests;";
    assert_eq!(supervisor.matches(exact_declarations).count(), 1);
    assert_eq!(supervisor.matches("windows_live_runtime_tests").count(), 1);
    assert_eq!(supervisor.matches("windows_live_test_support").count(), 2);
    assert!(
        supervisor.contains(
            "#[cfg(all(test, windows))]\npub(crate) use self::windows_live_test_support::{"
        )
    );
    for relative in [
        "src/session_supervisor/windows_live_test_support.rs",
        "src/session_supervisor/windows_live_runtime_tests.rs",
    ] {
        assert!(
            manifest().join(relative).is_file(),
            "fixed live source file is missing: {relative}"
        );
    }
}

#[test]
fn locked_metadata_has_eleven_targets_and_no_live_or_helper_target() {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .output()
        .expect("run locked Cargo metadata");
    assert!(output.status.success(), "locked Cargo metadata failed");
    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("parse Cargo metadata JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages array");
    let package_names = packages
        .iter()
        .map(|package| package["name"].as_str().expect("metadata package name"))
        .collect::<Vec<_>>();
    assert_eq!(package_names, vec!["zeus-core", "zeus-ui"]);
    let targets = packages
        .iter()
        .flat_map(|package| {
            package["targets"]
                .as_array()
                .expect("metadata targets array")
        })
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 11, "M3.1 metadata target count changed");
    assert_eq!(
        targets
            .iter()
            .filter(|target| target["name"].as_str() == Some("zeus-ui"))
            .count(),
        1,
        "the native UI package must declare exactly one Cargo target"
    );

    let forbidden = [
        "windows_live_test_support",
        "windows_live_runtime_tests",
        "windows_manager_control_live",
        "windows_process_probe",
        "session_supervisor_parent",
        "windows_manager_worker_live",
    ];
    let live_sources = [
        "crates/zeus-core/src/manager/windows_live_runtime_tests.rs",
        "crates/zeus-core/src/manager/worker/windows_live_runtime_tests.rs",
        "crates/zeus-core/src/session_supervisor/windows_live_test_support.rs",
        "crates/zeus-core/src/session_supervisor/windows_live_runtime_tests.rs",
    ];
    for target in targets {
        let name = target["name"].as_str().expect("target name").to_lowercase();
        let source = Path::new(target["src_path"].as_str().expect("target source path"));
        let relative_source = source
            .strip_prefix(workspace_root())
            .expect("target source must remain inside workspace")
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        for token in forbidden {
            assert!(
                !name.contains(token) && !relative_source.contains(token),
                "forbidden helper or live source leaked into Cargo metadata"
            );
        }
        assert!(
            !live_sources.contains(&relative_source.as_str()),
            "live source became a declared Cargo target"
        );
        if relative_source.starts_with("crates/zeus-ui/") {
            assert_eq!(
                relative_source, "crates/zeus-ui/src/main.rs",
                "the native UI package must declare no test or helper Cargo target"
            );
            let kinds = target["kind"]
                .as_array()
                .expect("target kind array")
                .iter()
                .map(|kind| kind.as_str().expect("target kind"))
                .collect::<Vec<_>>();
            assert_eq!(
                kinds,
                vec!["bin"],
                "the native UI target must stay a binary"
            );
        }
    }
}

#[test]
fn native_ui_sources_exclude_core_internals_web_shells_and_async_runtimes() {
    for (relative, source) in native_ui_sources() {
        for token in [
            "CoreState",
            "ManagerController",
            "SessionSupervisor",
            "rusqlite",
            "Cryptography",
            "process_adapter",
            "std::process::Command",
            "Command::new",
            "tokio",
            "async fn",
            ".await",
            "WebView",
            "Electron",
            "tauri",
        ] {
            assert!(
                !source.contains(token),
                "forbidden surface {token} leaked into native UI source {relative}"
            );
        }
    }
}

#[test]
fn portable_windows_repair_stays_crate_private_and_out_of_the_native_ui() {
    let data_root = read_source("src/data_root.rs");
    assert_eq!(
        data_root
            .matches("#[cfg(windows)]\nmod portable_windows;")
            .count(),
        1,
        "portable repair must be one Windows-only private module"
    );
    for forbidden in [
        "pub mod portable_windows",
        "pub use portable_windows",
        "pub(crate) mod portable_windows",
    ] {
        assert!(
            !data_root.contains(forbidden),
            "portable repair escaped the data-root module boundary"
        );
    }
    assert_eq!(
        data_root
            .matches("pub(crate) fn prepare_portable_at(requested: &Path) -> CoreResult<Self>")
            .count(),
        1,
        "the portable entry point must stay crate-visible only"
    );

    let portable = read_source("src/data_root/portable_windows.rs");
    assert_eq!(
        portable
            .matches("pub(super) fn prepare_portable_root(requested: &Path) -> CoreResult<()>")
            .count(),
        1,
        "the repair operation must stay visible to the data-root module only"
    );
    for forbidden in ["\npub fn ", "\npub struct ", "\npub(crate) fn "] {
        assert!(
            !portable.contains(forbidden),
            "portable repair must expose no wider surface than the data-root module"
        );
    }

    let lib = read_source("src/lib.rs");
    for forbidden in [
        "portable_windows",
        "prepare_portable_at",
        "open_portable_at",
        "relocate_pinned_runtime",
    ] {
        assert!(
            !lib.contains(forbidden),
            "portable repair surface {forbidden} escaped the crate root"
        );
    }

    for (relative, source) in native_ui_sources() {
        for forbidden in [
            "open_portable_at",
            "prepare_portable_at",
            "portable_windows",
            "relocate_pinned_runtime",
            "PortableRepair",
        ] {
            assert!(
                !source.contains(forbidden),
                "portable repair surface {forbidden} leaked into native UI source {relative}"
            );
        }
    }
}

#[test]
fn production_runtime_capability_stays_needs_validation() {
    let validator = read_source("src/runtime/validator.rs");
    assert!(validator.contains("capability_state: CapabilityState::NeedsValidation"));
    assert!(!validator.contains("capability_state: CapabilityState::Supported"));
}

#[cfg(windows)]
mod account_password_ownership {
    use zeus_core::ManagerAccountPassword;

    struct MustNotSync;
    struct MustNotClone;
    trait OwnershipProof<Forbidden> {
        fn proof() {}
    }
    impl<T: ?Sized> OwnershipProof<()> for T {}
    impl<T: ?Sized + Sync> OwnershipProof<MustNotSync> for T {}
    impl<T: Clone> OwnershipProof<MustNotClone> for T {}

    fn assert_send<T: Send>() {}

    pub(super) fn prove() {
        assert_send::<ManagerAccountPassword>();
        let _ = <ManagerAccountPassword as OwnershipProof<_>>::proof;
    }
}

#[cfg(windows)]
#[test]
fn manager_account_password_is_send_never_sync_noncloneable_and_redacted() {
    use zeus_core::{ManagerAccountPassword, ManagerWorkerOperation};

    account_password_ownership::prove();
    let sentinel = "FoundationSecret-19!";
    let secret = ManagerAccountPassword::try_from_utf16(
        ManagerWorkerOperation::ImportAccount,
        sentinel.encode_utf16().collect(),
    )
    .expect("valid secret");
    assert_eq!(format!("{secret:?}"), "ManagerAccountPassword([REDACTED])");
    assert_eq!(format!("{secret}"), "ManagerAccountPassword([REDACTED])");
    assert!(!format!("{secret:?}{secret}").contains(sentinel));
}

/// Spec section 11: the public account boundary carries only ID/revision/username/status/last-run.
#[cfg(windows)]
#[test]
fn public_account_boundary_stays_redacted_and_opaque() {
    use std::hash::Hash;

    use zeus_core::ManagerAccountId;

    let account = read_source("src/manager/account.rs");

    // `ManagerAccountView` must expose exactly the five approved public fields.
    let view = account
        .split("pub struct ManagerAccountView {")
        .nth(1)
        .expect("the view struct is declared")
        .split('}')
        .next()
        .expect("the view struct is closed");
    let fields = view
        .lines()
        .filter_map(|line| line.trim().strip_suffix(','))
        .filter_map(|line| line.strip_prefix("pub "))
        .filter_map(|line| line.split(':').next())
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            "account_id",
            "revision",
            "username",
            "status",
            "last_run_at_unix_ms",
            // The operator's own world choice. It is a bounded index into the client's public server
            // table, not an internal identity, so it belongs on this surface.
            "server_index"
        ],
        "the public account view gained or lost a field"
    );

    // No credential, config, profile, session, runtime, or process identity may appear on the
    // public account surface.
    for forbidden in [
        "password",
        "cipher",
        "nonce",
        "tag",
        "config",
        "profile_id:",
        "session_id",
        "runtime_id",
        "pid",
        "hwnd",
        "birth",
    ] {
        assert!(
            !view.contains(forbidden),
            "the public account view exposes {forbidden}"
        );
    }

    // `ManagerAccountId` is an opaque row key: no public constructor, Display, or string accessor.
    // Every impl block is inspected, not just the first: a second block could otherwise reopen the
    // type and add a public accessor without failing this gate.
    let account_id_impls: Vec<&str> = account
        .split("impl ManagerAccountId {")
        .skip(1)
        .map(|block| {
            block
                .split("\n}")
                .next()
                .expect("the ManagerAccountId impl is closed")
        })
        .collect();
    assert_eq!(
        account_id_impls.len(),
        1,
        "ManagerAccountId must have exactly one impl block"
    );
    for block in &account_id_impls {
        for forbidden in [
            "pub fn new(",
            "pub fn get(",
            "pub fn as_str(",
            "pub fn to_string(",
        ] {
            assert!(
                !block.contains(forbidden),
                "ManagerAccountId exposes {forbidden}"
            );
        }
    }
    assert!(
        !account.contains("impl fmt::Display for ManagerAccountId"),
        "ManagerAccountId must not be renderable"
    );

    // Copy + Eq + Ord + Hash, so a UI row model can key on it. Asserted statically because the type
    // deliberately has no public constructor for a test to call.
    fn assert_row_key<T: Copy + Eq + Ord + Hash>() {}
    assert_row_key::<ManagerAccountId>();

    // The four public status values are the complete reconciled state space; UI-local pending states
    // must not leak in.
    for forbidden in ["Starting,", "Authenticating,", "Stopping,"] {
        assert!(
            !account.contains(forbidden),
            "UI-local pending state {forbidden} leaked into the public status enum"
        );
    }
    for expected in ["Idle,", "Running,", "LoginFailed,", "CleanupPending,"] {
        assert!(
            account.contains(expected),
            "the public status enum lost {expected}"
        );
    }
}

/// Removes the lines that move the opaque `ManagerAccountPassword` carrier, failing if any other line
/// mentions a password. This keeps the credential-token ban strict while allowing the exact public
/// signatures spec section 11 requires.
fn strip_typed_password_carrier(relative: &str, source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // The outer gate lowercases before matching, and `ManagerAccountPassword` itself contains
            // the token, so this filter must be case-insensitive too.
            if !trimmed.to_lowercase().contains("password") {
                return true;
            }
            // Every approved line must either name the opaque carrier type or be one of the exact
            // field-shorthand lines spec section 11's signatures require. The previous shape heuristic
            // ("contains `password)` and ends with a paren") would have approved
            // `eprintln!("{}", password)`, and because approved lines are removed before the outer
            // token scan, such a leak would have bypassed both gates.
            // Exact full-line allowlist, not a shape heuristic. The previous rule approved any line
            // containing `password)` that ended with a paren, which would have waved through
            // `eprintln!("{}", password)`; because approved lines are removed before the outer token
            // scan, such a leak would have bypassed both gates. Every entry below either names the
            // opaque carrier type or moves it by field shorthand between the signatures spec section 11
            // fixes by name.
            const APPROVED_CARRIER_LINES: &[&str] = &[
                "password,",
                "replacement_password,",
                "self.import_account(username, password)",
                "result: controller.import_account(&username, password),",
                "self.inner.import_account(username, password)",
                ".create_account_with_profile(username, password.into_secret())",
            ];
            let approved = trimmed.contains("ManagerAccountPassword")
                || APPROVED_CARRIER_LINES.contains(&trimmed);
            assert!(
                approved,
                "{relative} mentions a password outside the approved typed carrier: {trimmed}"
            );
            false
        })
        .collect::<Vec<_>>()
        .join("\n")
}

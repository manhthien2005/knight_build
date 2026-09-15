use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use uuid::Uuid;
use zeus_core::SCHEMA_VERSION;

#[cfg(windows)]
#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;
#[cfg(windows)]
use runtime_fixture::RuntimeFixture;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("zeus-core-cli-{label}-{}", Uuid::new_v4())))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn init_is_json_bounded_and_session_commands_fail_closed() {
    let root = TestDirectory::new("init");
    let initialized = run(&root.0, &["init"]);
    let document = success_json(initialized);
    assert_eq!(document["schema_version"], 1);
    assert_eq!(document["ok"], true);
    assert_eq!(document["data"]["schema_version"], SCHEMA_VERSION);
    assert_eq!(document["data"]["connection_count"], 1);

    for forbidden in ["launch", "stop", "session"] {
        let output = Command::new(env!("CARGO_BIN_EXE_zeus-core"))
            .arg(forbidden)
            .output()
            .expect("start zeus-core");
        let error = error_json(output);
        assert_eq!(error["error"]["code"], "UnknownCommand");
    }

    let output = run(&root.0, &["runtime", "list", "--limit", "101"]);
    let error = error_json(output);
    assert_eq!(error["error"]["code"], "InvalidPageLimit");
}

#[cfg(windows)]
#[test]
fn runtime_and_profile_diagnostic_commands_round_trip_with_fixture() {
    let root = TestDirectory::new("round-trip");
    let runtime = RuntimeFixture::new("cli-round-trip");
    let descriptor_text = runtime
        .descriptor_path()
        .to_str()
        .expect("Unicode descriptor path");

    let registered = success_json(run(
        &root.0,
        &["runtime", "register", "--descriptor", descriptor_text],
    ));
    let runtime_id = registered["data"]["runtime_id"]
        .as_str()
        .expect("runtime ID")
        .to_owned();
    assert_eq!(registered["data"]["capability_state"], "NeedsValidation");

    let listed = success_json(run(&root.0, &["runtime", "list", "--limit", "1"]));
    assert_eq!(listed["data"]["items"].as_array().unwrap().len(), 1);
    let inspected = success_json(run(
        &root.0,
        &["runtime", "inspect", "--runtime-id", &runtime_id],
    ));
    assert_eq!(inspected["data"]["runtime_id"], runtime_id);

    let created_a = success_json(run(
        &root.0,
        &[
            "profile",
            "create",
            "--name",
            "Account A",
            "--runtime-id",
            &runtime_id,
        ],
    ));
    let profile_a = created_a["data"]["profile_id"]
        .as_str()
        .expect("profile ID")
        .to_owned();
    assert_eq!(created_a["data"]["revision"], 1);
    let created_b = success_json(run(
        &root.0,
        &[
            "profile",
            "create",
            "--name",
            "Account B",
            "--runtime-id",
            &runtime_id,
        ],
    ));
    assert_eq!(created_b["data"]["revision"], 1);

    let renamed = success_json(run(
        &root.0,
        &[
            "profile",
            "rename",
            "--profile-id",
            &profile_a,
            "--expected-revision",
            "1",
            "--name",
            "Account A renamed",
        ],
    ));
    assert_eq!(renamed["data"]["revision"], 2);
    let stale = error_json(run(
        &root.0,
        &[
            "profile",
            "rename",
            "--profile-id",
            &profile_a,
            "--expected-revision",
            "1",
            "--name",
            "stale",
        ],
    ));
    assert_eq!(stale["error"]["code"], "RevisionConflict");

    let rebound = success_json(run(
        &root.0,
        &[
            "profile",
            "bind-runtime",
            "--profile-id",
            &profile_a,
            "--expected-revision",
            "2",
            "--runtime-id",
            &runtime_id,
        ],
    ));
    assert_eq!(rebound["data"]["revision"], 3);
    let archived = success_json(run(
        &root.0,
        &[
            "profile",
            "archive",
            "--profile-id",
            &profile_a,
            "--expected-revision",
            "3",
        ],
    ));
    assert_eq!(archived["data"]["revision"], 4);
    assert!(archived["data"]["archived_at_unix_ms"].is_number());

    let inspected_profile = success_json(run(
        &root.0,
        &["profile", "inspect", "--profile-id", &profile_a],
    ));
    assert_eq!(
        inspected_profile["data"]["display_name"],
        "Account A renamed"
    );
    let all_profiles = success_json(run(
        &root.0,
        &["profile", "list", "--limit", "100", "--include-archived"],
    ));
    assert_eq!(all_profiles["data"]["items"].as_array().unwrap().len(), 2);
}

fn run(data_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zeus-core"))
        .arg("--data-root")
        .arg(data_root)
        .args(arguments)
        .output()
        .expect("start zeus-core")
}

fn success_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "success must not write stderr");
    serde_json::from_slice(&output.stdout).expect("valid success JSON")
}

fn error_json(output: Output) -> Value {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    assert!(output.stdout.is_empty(), "errors must not write stdout");
    serde_json::from_slice(&output.stderr).expect("valid error JSON")
}

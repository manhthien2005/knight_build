use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeus_core::runtime::{CapabilityState, MAX_DESCRIPTOR_BYTES, validate_runtime_descriptor};
use zeus_core::{CoreError, CoreState};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::ptr::null_mut;
#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SetFileSecurityW,
};

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("zeus-runtime-{label}-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct RuntimeFixture {
    _directory: TestDirectory,
    root: PathBuf,
    descriptor_path: PathBuf,
}

impl RuntimeFixture {
    fn new(label: &str) -> Self {
        let directory = TestDirectory::new(label);
        let root = directory.path.clone();
        drop(CoreState::open_at(&root).expect("create private fixture root"));

        let jre = root.join("jre");
        fs::create_dir_all(jre.join("bin")).expect("create fixture JRE");
        let java_relative = if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        };
        fs::write(jre.join(java_relative), b"fixture-java").expect("write fixture java");
        let mut jre_files = vec![java_relative.to_owned()];
        if cfg!(windows) {
            fs::write(jre.join("bin/javaw.exe"), b"fixture-javaw").expect("write fixture javaw");
            jre_files.push("bin/javaw.exe".to_owned());
        }
        jre_files.sort();

        fs::create_dir_all(root.join("microemulator")).expect("create MicroEmulator directory");
        fs::create_dir_all(root.join("game")).expect("create game directory");
        let micro_path = root.join("microemulator/microemulator.jar");
        let game_path = root.join("game/KnightOnline_402.jar");
        fs::write(&micro_path, b"fixture-microemulator").expect("write MicroEmulator fixture");
        fs::write(&game_path, b"fixture-game-402").expect("write game fixture");

        let manifest = jre_files
            .iter()
            .map(|relative| {
                let bytes = fs::read(jre.join(relative)).expect("read JRE fixture");
                format!("{}  {}  {relative}\n", sha256(&bytes), bytes.len())
            })
            .collect::<String>();
        let manifest_path = root.join("jre-files.sha256");
        fs::write(&manifest_path, manifest.as_bytes()).expect("write JRE manifest");

        let (target_os, target_arch) = host_tuple();
        let descriptor = json!({
            "schema_version": 1,
            "runtime_id": format!("{target_os}-{target_arch}_fixture-java11_microemu204_ko402"),
            "created_at_utc": "2026-08-22T12:04:19.283Z",
            "platform": {
                "os": target_os,
                "architecture": target_arch
            },
            "java": {
                "vendor": "Fixture Vendor",
                "distribution": "Fixture JRE",
                "jvm": "HotSpot",
                "version": "11.0.32+9",
                "image_type": "jre",
                "archive_name": "fixture-jre.zip",
                "archive_size": 1024,
                "archive_sha256": sha256(b"fixture-jre-archive"),
                "source": "https://example.invalid/fixture-jre.zip",
                "tree_manifest": "jre-files.sha256",
                "tree_file_count": jre_files.len(),
                "tree_manifest_sha256": sha256(manifest.as_bytes())
            },
            "microemulator": {
                "version": "2.0.4",
                "archive_name": "microemulator-2.0.4.zip",
                "archive_size": 2048,
                "archive_sha256": sha256(b"fixture-micro-archive"),
                "source": "https://example.invalid/microemulator.zip",
                "jar": "microemulator/microemulator.jar",
                "jar_size": fs::metadata(&micro_path).expect("micro metadata").len(),
                "jar_sha256": sha256_path(&micro_path),
                "optional_jars": []
            },
            "game": {
                "name": "KnightOnline",
                "bundle": "402",
                "midlet_version": "1.8.2",
                "profile": "MIDP-2.0",
                "configuration": "CLDC-1.0",
                "jar": "game/KnightOnline_402.jar",
                "jar_size": fs::metadata(&game_path).expect("game metadata").len(),
                "jar_sha256": sha256_path(&game_path),
                "source_type": "local_verified_copy"
            },
            "launch_defaults": {
                "mode": "classpath_midlet_main",
                "main_class": "org.microemu.app.Main",
                "midlet_class": "com.silverknight.TemMidlet",
                "screen_width": 240,
                "screen_height": 320,
                "heap_initial_mib": 16,
                "heap_max_mib": 128,
                "gc": "SerialGC",
                "use_perf_data": false,
                "rms": "file",
                "quiet": false,
                "quit_on_midlet_destroy": true
            },
            "validation": {
                "status": "fixture_static_only",
                "evidence": "evidence.json",
                "passed": ["artifact_checksums"],
                "pending": ["isolation", "containment", "resource", "ubuntu_tuple"]
            }
        });
        let descriptor_path = root.join("runtime-descriptor.json");
        write_json(&descriptor_path, &descriptor);

        Self {
            _directory: directory,
            root,
            descriptor_path,
        }
    }

    fn mutate_descriptor(&self, mutation: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_slice(
            &fs::read(&self.descriptor_path).expect("read descriptor for mutation"),
        )
        .expect("parse descriptor for mutation");
        mutation(&mut value);
        write_json(&self.descriptor_path, &value);
    }

    fn refresh_manifest_metadata(&self) {
        let manifest = fs::read(self.root.join("jre-files.sha256")).expect("read manifest");
        self.mutate_descriptor(|value| {
            value["java"]["tree_manifest_sha256"] = Value::String(sha256(&manifest));
        });
    }
}

#[test]
fn validates_bounded_static_fixture_but_keeps_runtime_needs_validation() {
    let fixture = RuntimeFixture::new("valid");
    let result = validate_runtime_descriptor(&fixture.descriptor_path).expect("validate fixture");

    assert_eq!(result.capability_state, CapabilityState::NeedsValidation);
    assert_eq!(result.game_bundle, "402");
    assert_eq!(result.microemulator_version, "2.0.4");
    assert_eq!(result.java_version, "11.0.32+9");
    assert_eq!(result.descriptor_sha256.len(), 64);
    assert!(result.validation_reason.contains("runtime probes pending"));
}

#[test]
fn rejects_oversized_and_unknown_descriptor_input() {
    let oversized = RuntimeFixture::new("oversized");
    fs::write(
        &oversized.descriptor_path,
        vec![b' '; MAX_DESCRIPTOR_BYTES + 1],
    )
    .expect("write oversized descriptor");
    assert_validation_code(
        validate_runtime_descriptor(&oversized.descriptor_path),
        "descriptor_too_large",
    );

    let unknown = RuntimeFixture::new("unknown");
    unknown.mutate_descriptor(|value| value["unexpected"] = json!(true));
    assert_validation_code(
        validate_runtime_descriptor(&unknown.descriptor_path),
        "descriptor_json_invalid",
    );

    // An absent descriptor is a runtime rejection, never a storage failure: the portable UI turns the
    // two into different operator instructions, so misclassifying this told the operator to move the
    // whole data folder when only the runtime was missing.
    let absent = RuntimeFixture::new("absent");
    fs::remove_file(&absent.descriptor_path).expect("remove descriptor");
    assert_validation_code(
        validate_runtime_descriptor(&absent.descriptor_path),
        "descriptor_path_invalid",
    );
}

#[test]
fn rejects_schema_runtime_id_and_host_tuple_mismatch() {
    let schema = RuntimeFixture::new("schema");
    schema.mutate_descriptor(|value| value["schema_version"] = json!(2));
    assert_validation_code(
        validate_runtime_descriptor(&schema.descriptor_path),
        "descriptor_schema_unsupported",
    );

    let runtime_id = RuntimeFixture::new("runtime-id");
    runtime_id.mutate_descriptor(|value| value["runtime_id"] = json!("../runtime"));
    assert_validation_code(
        validate_runtime_descriptor(&runtime_id.descriptor_path),
        "runtime_id_invalid",
    );

    let host = RuntimeFixture::new("host");
    let wrong_os = if cfg!(windows) { "ubuntu" } else { "windows" };
    host.mutate_descriptor(|value| value["platform"]["os"] = json!(wrong_os));
    assert_validation_code(
        validate_runtime_descriptor(&host.descriptor_path),
        "host_tuple_mismatch",
    );
}

#[test]
fn rejects_escaped_absolute_and_unpinned_artifact_paths() {
    let escaped = RuntimeFixture::new("escaped");
    escaped.mutate_descriptor(|value| value["game"]["jar"] = json!("../game.jar"));
    assert_validation_code(
        validate_runtime_descriptor(&escaped.descriptor_path),
        "artifact_path_invalid",
    );

    let absolute = RuntimeFixture::new("absolute");
    absolute.mutate_descriptor(|value| {
        value["microemulator"]["jar"] = json!(r"C:\shared\microemulator.jar")
    });
    assert_validation_code(
        validate_runtime_descriptor(&absolute.descriptor_path),
        "artifact_path_invalid",
    );

    let optional = RuntimeFixture::new("optional");
    optional.mutate_descriptor(|value| {
        value["microemulator"]["optional_jars"] = json!(["unhashed.jar"])
    });
    assert_validation_code(
        validate_runtime_descriptor(&optional.descriptor_path),
        "optional_jars_unpinned",
    );
}

#[test]
fn rejects_artifact_size_hash_and_manifest_corruption() {
    let bad_hash = RuntimeFixture::new("bad-hash");
    bad_hash.mutate_descriptor(|value| value["game"]["jar_sha256"] = json!("0".repeat(64)));
    assert_validation_code(
        validate_runtime_descriptor(&bad_hash.descriptor_path),
        "artifact_hash_mismatch",
    );

    let bad_size = RuntimeFixture::new("bad-size");
    bad_size.mutate_descriptor(|value| value["microemulator"]["jar_size"] = json!(1));
    assert_validation_code(
        validate_runtime_descriptor(&bad_size.descriptor_path),
        "artifact_size_mismatch",
    );

    let malformed = RuntimeFixture::new("manifest-malformed");
    fs::write(malformed.root.join("jre-files.sha256"), b"malformed\n")
        .expect("write malformed manifest");
    malformed.refresh_manifest_metadata();
    assert_validation_code(
        validate_runtime_descriptor(&malformed.descriptor_path),
        "manifest_line_invalid",
    );
}

#[test]
fn rejects_runtime_artifact_writable_by_other_os_users() {
    let fixture = RuntimeFixture::new("insecure-entry");
    make_artifact_writable_by_other_users(&fixture.root.join("game/KnightOnline_402.jar"));

    assert_validation_code(
        validate_runtime_descriptor(&fixture.descriptor_path),
        "runtime_entry_insecure",
    );
}

#[test]
fn rejects_jre_extra_file_and_reparse_tree_entry() {
    let extra = RuntimeFixture::new("extra-jre");
    fs::write(extra.root.join("jre/extra.bin"), b"extra").expect("write extra JRE file");
    assert_validation_code(
        validate_runtime_descriptor(&extra.descriptor_path),
        "jre_file_set_mismatch",
    );

    #[cfg(windows)]
    {
        let linked = RuntimeFixture::new("linked-jre");
        let target = linked.root.join("junction-target");
        fs::create_dir(&target).expect("create junction target");
        fs::write(target.join("file.bin"), b"linked").expect("write junction target file");
        let junction_path = linked.root.join("jre/linked");
        junction::create(&target, &junction_path).expect("create JRE junction fixture");
        assert_validation_code(
            validate_runtime_descriptor(&linked.descriptor_path),
            "artifact_reparse_rejected",
        );
        junction::delete(&junction_path).expect("delete JRE junction fixture");
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires provisioned exact Windows runtime; run scripts/Invoke-ExactRuntimeTests.ps1"]
fn validates_exact_local_descriptor_as_needs_validation() {
    let runtime_root = std::env::var_os("ZEUS_EXACT_RUNTIME_ROOT")
        .map(PathBuf::from)
        .expect("Invoke-ExactRuntimeTests.ps1 must provide ZEUS_EXACT_RUNTIME_ROOT")
        .canonicalize()
        .expect("canonicalize selected exact runtime root");
    let descriptor = runtime_root.join("runtime-descriptor.json");
    let result = validate_runtime_descriptor(&descriptor).expect("validate exact local runtime");

    assert_eq!(
        result.runtime_id,
        "windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402"
    );
    assert_eq!(result.capability_state, CapabilityState::NeedsValidation);
    assert_eq!(
        result.game_sha256,
        "6608bb0c77f03749e46165f711e9566dca4e172ce232256497b35faafe74c259"
    );
}

fn host_tuple() -> (&'static str, &'static str) {
    let os = if cfg!(windows) { "windows" } else { "ubuntu" };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    (os, arch)
}

#[cfg(unix)]
fn make_artifact_writable_by_other_users(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o666))
        .expect("make Unix artifact broadly writable");
}

#[cfg(windows)]
fn make_artifact_writable_by_other_users(path: &Path) {
    let sddl: Vec<u16> = "D:P(A;;FA;;;WD)".encode_utf16().chain([0]).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the SDDL buffer and output pointer are valid for this call.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    assert_ne!(converted, 0, "create permissive artifact DACL");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the path and descriptor buffers remain valid through this call.
    let applied = unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    // SAFETY: the SDDL conversion allocated this descriptor via LocalAlloc.
    unsafe {
        LocalFree(descriptor.cast());
    }
    assert_ne!(applied, 0, "apply permissive artifact DACL");
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_path(path: &Path) -> String {
    sha256(&fs::read(path).expect("read hash fixture"))
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize fixture descriptor"),
    )
    .expect("write fixture descriptor");
}

fn assert_validation_code<T>(result: Result<T, CoreError>, expected: &'static str) {
    match result {
        Err(CoreError::RuntimeValidation { code }) => assert_eq!(code, expected),
        Err(other) => panic!("expected RuntimeValidation({expected}), got {other:?}"),
        Ok(_) => panic!("expected RuntimeValidation({expected}), got success"),
    }
}

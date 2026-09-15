use std::ffi::OsString;
use std::fs::{self, FileTimes, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::{Uuid, Version};
use zeus_core::runtime::CapabilityState;
use zeus_core::{
    CoreError, CoreState, JvmFlag, LaunchEnvironmentKey, ProcessStdio, ProfileRecord, RmsMode,
    RuntimePreflightMode,
};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
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

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("zeus-launch-{label}-{}", Uuid::new_v4())))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.0);
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
        Self::new_with_directory_label(label, label)
    }

    fn new_with_directory_label(label: &str, directory_label: &str) -> Self {
        let directory = TestDirectory::new(directory_label);
        let root = directory.0.clone();
        drop(CoreState::open_at(&root).expect("create private fixture root"));

        let jre = root.join("jre");
        fs::create_dir_all(jre.join("bin")).expect("create fixture JRE");
        let java_relative = if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        };
        fs::write(jre.join(java_relative), b"not-an-executable-java")
            .expect("write inert Java fixture");
        let mut jre_files = vec![java_relative.to_owned()];
        if cfg!(windows) {
            fs::write(jre.join("bin/javaw.exe"), b"not-an-executable-javaw")
                .expect("write inert Javaw fixture");
            jre_files.push("bin/javaw.exe".to_owned());
        }
        jre_files.sort();

        fs::create_dir_all(root.join("microemulator")).expect("create MicroEmulator directory");
        fs::create_dir_all(root.join("game")).expect("create game directory");
        let microemulator_path = root.join("microemulator/microemulator.jar");
        let game_path = root.join("game/KnightOnline_402.jar");
        fs::write(&microemulator_path, b"fixture-microemulator")
            .expect("write MicroEmulator fixture");
        fs::write(&game_path, b"fixture-game-402").expect("write game fixture");

        let manifest = jre_files
            .iter()
            .map(|relative| {
                let bytes = fs::read(jre.join(relative)).expect("read JRE fixture");
                format!("{}  {}  {relative}\n", sha256(&bytes), bytes.len())
            })
            .collect::<String>();
        fs::write(root.join("jre-files.sha256"), manifest.as_bytes()).expect("write JRE manifest");

        let target_os = if cfg!(windows) { "windows" } else { "ubuntu" };
        let descriptor = json!({
            "schema_version": 1,
            "runtime_id": format!("{target_os}-x64_fixture-java11_microemu204_ko402_{label}"),
            "created_at_utc": "2026-08-22T12:04:19.283Z",
            "platform": {
                "os": target_os,
                "architecture": "x64"
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
                "jar_size": fs::metadata(&microemulator_path).expect("micro metadata").len(),
                "jar_sha256": sha256_path(&microemulator_path),
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
        fs::write(
            &descriptor_path,
            serde_json::to_vec_pretty(&descriptor).expect("serialize descriptor"),
        )
        .expect("write descriptor");

        Self {
            _directory: directory,
            root,
            descriptor_path,
        }
    }

    fn mutate_descriptor(&self, mutation: impl FnOnce(&mut Value)) {
        let mut descriptor: Value = serde_json::from_slice(
            &fs::read(&self.descriptor_path).expect("read descriptor for mutation"),
        )
        .expect("parse descriptor for mutation");
        mutation(&mut descriptor);
        fs::write(
            &self.descriptor_path,
            serde_json::to_vec_pretty(&descriptor).expect("serialize mutated descriptor"),
        )
        .expect("write mutated descriptor");
    }
}

#[test]
fn portable_snapshot_is_structured_private_and_does_not_execute_java() {
    let fixture = RuntimeFixture::new("portable");
    let data_root = TestDirectory::new("data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Visible Account Name", &runtime.runtime_id)
        .expect("create profile");

    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare launch snapshot without executing inert Java fixture");

    assert_eq!(snapshot.session_id().get_version(), Some(Version::Random));
    assert_eq!(snapshot.profile_id().to_string(), profile.profile_id);
    assert_eq!(snapshot.profile_revision(), 1);
    assert_eq!(snapshot.runtime_id(), runtime.runtime_id);
    assert_eq!(snapshot.descriptor_sha256(), runtime.descriptor_sha256);
    assert_eq!(snapshot.java_executable(), runtime.java_path);
    assert_eq!(snapshot.microemulator_jar(), runtime.microemulator_path);
    assert_eq!(snapshot.game_jar(), runtime.game_path);
    assert_eq!(snapshot.main_class(), "org.microemu.app.Main");
    assert_eq!(snapshot.midlet_class(), "com.silverknight.TemMidlet");
    assert_eq!(snapshot.screen_size().width(), 240);
    assert_eq!(snapshot.screen_size().height(), 320);
    assert_eq!(snapshot.heap().initial_mib(), 16);
    assert_eq!(snapshot.heap().maximum_mib(), 128);
    assert_eq!(
        snapshot.jvm_flags(),
        &[JvmFlag::UseSerialGc, JvmFlag::DisablePerfData]
    );
    assert_eq!(snapshot.rms_mode(), RmsMode::File);
    assert!(!snapshot.quiet());
    assert!(snapshot.quit_on_midlet_destroy());
    assert_eq!(snapshot.working_directory(), snapshot.profile_root());
    assert_eq!(
        snapshot.microemu_home(),
        snapshot.profile_root().join("microemu-home")
    );
    assert_eq!(
        snapshot.temp_directory(),
        snapshot.profile_root().join("temp")
    );
    for path in [
        snapshot.runtime_root(),
        snapshot.java_executable(),
        snapshot.microemulator_jar(),
        snapshot.game_jar(),
        snapshot.profile_root(),
        snapshot.working_directory(),
        snapshot.microemu_home(),
        snapshot.temp_directory(),
    ] {
        assert!(path.is_absolute());
        assert_eq!(path.canonicalize().expect("canonical snapshot path"), path);
    }
    for path in [
        snapshot.java_executable(),
        snapshot.microemulator_jar(),
        snapshot.game_jar(),
    ] {
        assert!(path.starts_with(snapshot.runtime_root()));
    }
    for path in [
        snapshot.working_directory(),
        snapshot.microemu_home(),
        snapshot.temp_directory(),
    ] {
        assert!(path.starts_with(snapshot.profile_root()));
    }
    assert!(snapshot.microemu_home().is_dir());
    assert!(snapshot.temp_directory().is_dir());
    #[cfg(unix)]
    for directory in [snapshot.microemu_home(), snapshot.temp_directory()] {
        assert_eq!(
            fs::metadata(directory)
                .expect("inspect private launch directory")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    // On Windows the fixed environment also carries `SystemRoot`, because WinSock resolves hosts
    // through it: a child without it connects to nothing, which is a silent failure rather than an
    // error. Every other entry is still the profile's own temp directory.
    let expected_environment_len = if cfg!(windows) { 4 } else { 3 };
    assert_eq!(snapshot.environment().len(), expected_environment_len);
    let mut expected_keys = vec![
        LaunchEnvironmentKey::Temp,
        LaunchEnvironmentKey::Tmp,
        LaunchEnvironmentKey::TmpDir,
    ];
    if cfg!(windows) {
        expected_keys.push(LaunchEnvironmentKey::SystemRoot);
    }
    assert_eq!(
        snapshot
            .environment()
            .iter()
            .map(|entry| entry.key())
            .collect::<Vec<_>>(),
        expected_keys
    );
    assert!(
        snapshot
            .environment()
            .iter()
            .filter(|entry| entry.key() != LaunchEnvironmentKey::SystemRoot)
            .all(|entry| entry.value() == snapshot.temp_directory())
    );
    // The one non-profile entry must be a real directory outside the profile.
    for entry in snapshot
        .environment()
        .iter()
        .filter(|entry| entry.key() == LaunchEnvironmentKey::SystemRoot)
    {
        assert!(entry.value().is_dir());
        assert_ne!(entry.value(), snapshot.temp_directory());
    }
    assert_eq!(
        core.inspect_runtime(&runtime.runtime_id)
            .expect("inspect unchanged runtime state")
            .capability_state,
        CapabilityState::NeedsValidation
    );
    assert_eq!(
        fixture.root.canonicalize().expect("canonical fixture root"),
        runtime.runtime_root
    );
}

#[test]
fn process_launch_spec_maps_validated_snapshot_to_ordered_native_argv() {
    let fixture = RuntimeFixture::new("process-spec-order");
    let data_root = TestDirectory::new("process-spec-order-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Process spec", &runtime.runtime_id)
        .expect("create profile");
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare snapshot");

    let spec = snapshot
        .process_launch_spec()
        .expect("materialize process launch spec");
    let invocation_java = invocation_path(snapshot.java_executable());
    let invocation_working_directory = invocation_path(snapshot.working_directory());
    let invocation_home = invocation_path(snapshot.microemu_home());
    let invocation_temp = invocation_path(snapshot.temp_directory());
    let invocation_microemulator = invocation_path(snapshot.microemulator_jar());
    let invocation_game = invocation_path(snapshot.game_jar());
    let mut classpath = OsString::from(invocation_microemulator.as_os_str());
    classpath.push(if cfg!(windows) { ";" } else { ":" });
    classpath.push(invocation_game.as_os_str());
    let expected = vec![
        prefixed_os("-Duser.home=", &invocation_home),
        prefixed_os("-Djava.io.tmpdir=", &invocation_temp),
        // The mod publishes its read-only character snapshot here, inside the profile's own
        // `microemu-home`. The name is pinned so the writer and the tool's reader cannot drift.
        prefixed_os(
            "-Dzeus.player.out=",
            &invocation_home.join("zeus-player.txt"),
        ),
        // The tool writes the attack and item settings here, in the same private directory.
        prefixed_os("-Dzeus.ctl.in=", &invocation_home.join("zeus-control.txt")),
        OsString::from("-Xms16m"),
        OsString::from("-Xmx128m"),
        OsString::from("-XX:+UseSerialGC"),
        OsString::from("-XX:-UsePerfData"),
        prefixed_os(
            "-XX:ErrorFile=",
            &invocation_working_directory.join("hs_err_pid%p.log"),
        ),
        OsString::from("-cp"),
        classpath,
        OsString::from("org.microemu.app.Main"),
        OsString::from("--resizableDevice"),
        OsString::from("240"),
        OsString::from("320"),
        OsString::from("--rms"),
        OsString::from("file"),
        OsString::from("--id"),
        OsString::from(profile.profile_id.as_str()),
        OsString::from("--quit"),
        OsString::from("com.silverknight.TemMidlet"),
    ];

    assert_eq!(spec.argv_schema_version(), 1);
    assert_eq!(spec.session_id(), snapshot.session_id());
    assert_eq!(spec.profile_id(), snapshot.profile_id());
    assert_eq!(spec.executable(), invocation_java);
    assert_eq!(spec.arguments(), expected.as_slice());
    for writable in [
        snapshot.microemu_home(),
        snapshot.temp_directory(),
        &snapshot.profile_root().join("hs_err_pid%p.log"),
    ] {
        assert!(writable.starts_with(snapshot.profile_root()));
    }
    assert_eq!(spec.working_directory(), invocation_working_directory);
    assert_native_invocation_identity(spec.executable(), snapshot.java_executable());
    assert_native_invocation_identity(spec.working_directory(), snapshot.working_directory());
    assert_native_invocation_identity(&invocation_home, snapshot.microemu_home());
    assert_native_invocation_identity(&invocation_temp, snapshot.temp_directory());
    assert_native_invocation_identity(&invocation_microemulator, snapshot.microemulator_jar());
    assert_native_invocation_identity(&invocation_game, snapshot.game_jar());
    assert!(!spec.inherit_environment());
    assert_eq!(spec.stdin(), ProcessStdio::Null);
    assert_eq!(spec.stdout(), ProcessStdio::Null);
    assert_eq!(spec.stderr(), ProcessStdio::Null);
}

#[test]
fn process_launch_spec_uses_only_profile_temp_environment_without_inheritance() {
    let fixture = RuntimeFixture::new("process-spec-env");
    let data_root = TestDirectory::new("process spec env with spaces");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Environment", &runtime.runtime_id)
        .expect("create profile");
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare snapshot");

    let spec = snapshot
        .process_launch_spec()
        .expect("materialize process launch spec");

    assert!(!spec.inherit_environment());
    let expected_spec_environment_len = if cfg!(windows) { 4 } else { 3 };
    assert_eq!(spec.environment().len(), expected_spec_environment_len);
    let mut expected_spec_keys = vec![
        LaunchEnvironmentKey::Temp,
        LaunchEnvironmentKey::Tmp,
        LaunchEnvironmentKey::TmpDir,
    ];
    if cfg!(windows) {
        expected_spec_keys.push(LaunchEnvironmentKey::SystemRoot);
    }
    assert_eq!(
        spec.environment()
            .iter()
            .map(|entry| entry.key())
            .collect::<Vec<_>>(),
        expected_spec_keys
    );
    let invocation_temp = invocation_path(snapshot.temp_directory());
    // Only the profile entries are remapped to the invocation form; `SystemRoot` keeps the plain
    // absolute path, because the extended `\\?\` prefix is not a usable `%SystemRoot%`.
    assert!(
        spec.environment()
            .iter()
            .filter(|entry| entry.key() != LaunchEnvironmentKey::SystemRoot)
            .all(|entry| entry.value() == invocation_temp)
    );
    for entry in spec
        .environment()
        .iter()
        .filter(|entry| entry.key() != LaunchEnvironmentKey::SystemRoot)
    {
        assert_native_invocation_identity(entry.value(), snapshot.temp_directory());
    }
    for entry in spec
        .environment()
        .iter()
        .filter(|entry| entry.key() == LaunchEnvironmentKey::SystemRoot)
    {
        assert!(entry.value().is_dir());
        assert!(!entry.value().to_string_lossy().starts_with(r"\\?\"));
    }
    let invocation_home = invocation_path(snapshot.microemu_home());
    let home = prefixed_os("-Duser.home=", &invocation_home);
    assert!(home.to_string_lossy().contains(' '));
    assert_eq!(
        spec.arguments()
            .iter()
            .filter(|argument| *argument == &home)
            .count(),
        1
    );
    // 21 arguments: the 19 the JVM and emulator need, plus the character snapshot and the
    // attack/item settings.
    assert_eq!(spec.arguments().len(), 21);
}

#[test]
fn process_launch_spec_emits_quiet_and_quit_only_from_snapshot_settings() {
    let fixture = RuntimeFixture::new("process-spec-switches");
    fixture.mutate_descriptor(|descriptor| {
        descriptor["launch_defaults"]["quiet"] = json!(true);
    });
    let data_root = TestDirectory::new("process-spec-switches-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Switches", &runtime.runtime_id)
        .expect("create profile");
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare snapshot");

    let spec = snapshot
        .process_launch_spec()
        .expect("materialize process launch spec");

    assert!(spec.arguments().contains(&OsString::from("--quiet")));
    assert!(spec.arguments().contains(&OsString::from("--quit")));
    assert_eq!(
        spec.arguments().last(),
        Some(&OsString::from("com.silverknight.TemMidlet"))
    );
}

#[test]
fn process_launch_specs_do_not_share_profile_writable_paths() {
    let fixture = RuntimeFixture::new("process-spec-isolation");
    let data_root = TestDirectory::new("process-spec-isolation-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let first = core
        .create_profile("First", &runtime.runtime_id)
        .expect("create first profile");
    let second = core
        .create_profile("Second", &runtime.runtime_id)
        .expect("create second profile");
    let first_snapshot = core
        .prepare_launch_snapshot(&first.profile_id, first.revision)
        .expect("prepare first snapshot");
    let second_snapshot = core
        .prepare_launch_snapshot(&second.profile_id, second.revision)
        .expect("prepare second snapshot");

    let first_spec = first_snapshot
        .process_launch_spec()
        .expect("materialize first spec");
    let second_spec = second_snapshot
        .process_launch_spec()
        .expect("materialize second spec");

    assert_ne!(
        first_spec.working_directory(),
        second_spec.working_directory()
    );
    assert_ne!(first_spec.environment(), second_spec.environment());
    for first_argument in [
        prefixed_os("-Duser.home=", first_snapshot.microemu_home()),
        prefixed_os("-Djava.io.tmpdir=", first_snapshot.temp_directory()),
        // Two accounts running at once must not publish over each other's character snapshot, nor
        // read each other's monster spot.
        prefixed_os(
            "-Dzeus.player.out=",
            &first_snapshot.microemu_home().join("zeus-player.txt"),
        ),
        prefixed_os(
            "-Dzeus.ctl.in=",
            &first_snapshot.microemu_home().join("zeus-control.txt"),
        ),
        prefixed_os(
            "-XX:ErrorFile=",
            &first_snapshot.profile_root().join("hs_err_pid%p.log"),
        ),
    ] {
        assert!(
            !second_spec
                .arguments()
                .iter()
                .any(|argument| argument == &first_argument)
        );
    }
}

#[test]
fn process_launch_spec_rejects_artifact_removed_after_snapshot() {
    let fixture = RuntimeFixture::new("process-spec-removed-artifact");
    let data_root = TestDirectory::new("process-spec-removed-artifact-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Removed artifact", &runtime.runtime_id)
        .expect("create profile");
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare snapshot");
    fs::remove_file(snapshot.game_jar()).expect("remove game JAR after snapshot");

    assert!(matches!(
        snapshot.process_launch_spec(),
        Err(CoreError::ProcessLaunchSpec {
            code: "process_artifact_path_invalid"
        })
    ));
}

#[test]
fn process_launch_spec_rejects_classpath_separator_inside_artifact_path() {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let fixture = RuntimeFixture::new_with_directory_label(
        "process-spec-classpath",
        &format!("process-spec{separator}classpath"),
    );
    let data_root = TestDirectory::new("process-spec-classpath-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Classpath separator", &runtime.runtime_id)
        .expect("create profile");
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare snapshot");

    assert!(matches!(
        snapshot.process_launch_spec(),
        Err(CoreError::ProcessLaunchSpec {
            code: "process_classpath_path_invalid"
        })
    ));
}

#[test]
fn first_snapshot_in_core_epoch_is_full_then_second_uses_metadata_fast_path() {
    let fixture = RuntimeFixture::new("cold-fast");
    let data_root = TestDirectory::new("cold-fast-data");
    let profile = {
        let mut core = CoreState::open_at(&data_root.0).expect("open registration Core");
        let runtime = core
            .register_runtime_descriptor(&fixture.descriptor_path)
            .expect("register fixture runtime");
        core.create_profile("Cold then fast", &runtime.runtime_id)
            .expect("create profile")
    };
    let core = CoreState::open_at(&data_root.0).expect("open fresh Core epoch");

    let cold_started = Instant::now();
    core.prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("cold launch snapshot");
    let cold_elapsed = cold_started.elapsed();
    let cold = core.runtime_preflight_diagnostics();

    let fast_started = Instant::now();
    core.prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("fast launch snapshot");
    let fast_elapsed = fast_started.elapsed();
    let fast = core.runtime_preflight_diagnostics();

    assert_eq!(cold.mode(), Some(RuntimePreflightMode::FullValidation));
    assert!(cold.jre_content_bytes_hashed() > 0);
    assert!(cold.content_bytes_hashed() > fast.content_bytes_hashed());
    assert_eq!(fast.mode(), Some(RuntimePreflightMode::FastMetadata));
    assert_eq!(fast.jre_content_bytes_hashed(), 0);
    assert_eq!(cold.cache_entries(), 1);
    assert_eq!(fast.cache_entries(), 1);
    eprintln!(
        "preflight-measurement cold_us={} cold_bytes={} cold_jre_bytes={} fast_us={} fast_bytes={} fast_jre_bytes={}",
        cold_elapsed.as_micros(),
        cold.content_bytes_hashed(),
        cold.jre_content_bytes_hashed(),
        fast_elapsed.as_micros(),
        fast.content_bytes_hashed(),
        fast.jre_content_bytes_hashed()
    );
}

#[test]
fn runtime_preflight_cache_is_bounded_to_sixteen_entries() {
    let data_root = TestDirectory::new("cache-cap-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let fixtures = (0..17)
        .map(|index| RuntimeFixture::new(&format!("cache-{index:02}")))
        .collect::<Vec<_>>();

    let first_runtime = core
        .register_runtime_descriptor(&fixtures[0].descriptor_path)
        .expect("register first cache fixture");
    let first_profile = core
        .create_profile("Evicted runtime", &first_runtime.runtime_id)
        .expect("create first runtime profile");
    for fixture in &fixtures[1..] {
        core.register_runtime_descriptor(&fixture.descriptor_path)
            .expect("register remaining cache fixture");
    }

    assert_eq!(core.runtime_preflight_diagnostics().cache_entries(), 16);
    core.prepare_launch_snapshot(&first_profile.profile_id, first_profile.revision)
        .expect("evicted runtime full-validates again");
    let diagnostics = core.runtime_preflight_diagnostics();
    assert_eq!(
        diagnostics.mode(),
        Some(RuntimePreflightMode::FullValidation)
    );
    assert!(diagnostics.jre_content_bytes_hashed() > 0);
    assert_eq!(diagnostics.cache_entries(), 16);
}

#[test]
fn fast_path_rejects_same_size_jre_content_modification_by_metadata() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-modified");
    let java = fixture_java_path(&fixture);
    let mut bytes = fs::read(&java).expect("read Java fixture");
    bytes[0] ^= 0xff;
    fs::write(&java, bytes).expect("modify Java fixture with same size");
    OpenOptions::new()
        .write(true)
        .open(&java)
        .expect("open modified Java fixture")
        .set_times(FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("force distinct Java modification time");

    assert_preflight_failure_evicts(&core, &profile, "jre_metadata_fingerprint_mismatch");
}

#[test]
fn fast_path_rejects_same_size_jre_file_replacement_by_identity() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-replaced");
    let java = fixture_java_path(&fixture);
    let bytes = fs::read(&java).expect("read Java fixture");
    fs::remove_file(&java).expect("remove registered Java fixture");
    fs::write(&java, bytes).expect("replace Java fixture with same-size file");

    assert_preflight_failure_evicts(&core, &profile, "jre_metadata_fingerprint_mismatch");
}

#[test]
fn fast_path_rejects_jre_size_change() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-size");
    OpenOptions::new()
        .append(true)
        .open(fixture_java_path(&fixture))
        .expect("open Java fixture for append")
        .write_all(b"x")
        .expect("increase Java fixture size");

    assert_preflight_failure_evicts(&core, &profile, "manifest_entry_size_mismatch");
}

#[test]
fn fast_path_rejects_jre_file_set_change() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-file-set");
    fs::write(fixture.root.join("jre/unregistered.bin"), b"unregistered")
        .expect("add unregistered JRE file");

    assert_preflight_failure_evicts(&core, &profile, "jre_file_set_mismatch");
}

#[test]
fn fast_path_rejects_linked_jre_entry() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-linked");
    let target = fixture.root.join("linked-target");
    let linked = fixture.root.join("jre/linked");
    fs::create_dir(&target).expect("create linked JRE target");
    fs::write(target.join("linked.bin"), b"linked").expect("write linked JRE target");
    #[cfg(unix)]
    symlink(&target, &linked).expect("create JRE symlink");
    #[cfg(windows)]
    junction::create(&target, &linked).expect("create JRE junction");

    assert_preflight_failure_evicts(&core, &profile, "artifact_reparse_rejected");

    #[cfg(windows)]
    junction::delete(&linked).expect("delete JRE junction");
}

#[test]
fn fast_path_rejects_insecure_jre_permission() {
    let (fixture, _data_root, core, profile) = primed_fast_path("jre-permission");
    make_artifact_writable_by_other_users(&fixture_java_path(&fixture));

    assert_preflight_failure_evicts(&core, &profile, "runtime_entry_insecure");
}

#[test]
fn fast_path_hashes_manifest_content() {
    let (fixture, _data_root, core, profile) = primed_fast_path("manifest-hash");
    let manifest = fixture.root.join("jre-files.sha256");
    let mut bytes = fs::read(&manifest).expect("read manifest fixture");
    bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
    fs::write(&manifest, bytes).expect("change manifest with same size");

    assert_preflight_failure_evicts(&core, &profile, "manifest_hash_mismatch");
}

#[test]
fn fast_path_hashes_same_size_microemulator_change() {
    let (fixture, _data_root, core, profile) = primed_fast_path("micro-hash");
    mutate_file_same_size(&fixture.root.join("microemulator/microemulator.jar"));

    assert_preflight_failure_evicts(&core, &profile, "artifact_hash_mismatch");
}

#[test]
fn fast_path_hashes_same_size_game_change() {
    let (fixture, _data_root, core, profile) = primed_fast_path("game-hash");
    mutate_file_same_size(&fixture.root.join("game/KnightOnline_402.jar"));

    assert_preflight_failure_evicts(&core, &profile, "artifact_hash_mismatch");
}

#[test]
fn stale_profile_revision_is_rejected_before_launch_directories_are_created() {
    let fixture = RuntimeFixture::new("stale");
    let data_root = TestDirectory::new("stale-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Stale profile", &runtime.runtime_id)
        .expect("create profile");

    let result = core.prepare_launch_snapshot(&profile.profile_id, profile.revision + 1);

    assert!(matches!(
        result,
        Err(CoreError::RevisionConflict {
            expected: 2,
            actual: 1,
            ..
        })
    ));
    let profile_root = core
        .profile_directory(&profile.profile_id)
        .expect("inspect profile root");
    assert!(!profile_root.join("microemu-home").exists());
    assert!(!profile_root.join("temp").exists());
}

#[test]
fn archived_profile_is_rejected() {
    let fixture = RuntimeFixture::new("archived");
    let data_root = TestDirectory::new("archived-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Archived profile", &runtime.runtime_id)
        .expect("create profile");
    let archived = core
        .archive_profile(&profile.profile_id, profile.revision)
        .expect("archive profile");

    let result = core.prepare_launch_snapshot(&archived.profile_id, archived.revision);

    assert!(matches!(
        result,
        Err(CoreError::ProfileArchived { profile_id }) if profile_id == archived.profile_id
    ));
}

#[test]
fn descriptor_changed_after_registration_is_rejected_as_registry_mismatch() {
    let fixture = RuntimeFixture::new("descriptor-change");
    let data_root = TestDirectory::new("descriptor-change-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Descriptor change", &runtime.runtime_id)
        .expect("create profile");
    fixture.mutate_descriptor(|descriptor| {
        descriptor["launch_defaults"]["main_class"] = Value::String("changed.Main".to_owned());
    });

    let result = core.prepare_launch_snapshot(&profile.profile_id, profile.revision);

    assert!(matches!(
        result,
        Err(CoreError::RuntimeRegistryMismatch { runtime_id }) if runtime_id == runtime.runtime_id
    ));
}

#[test]
fn snapshot_revision_argument_is_positive_and_bounded() {
    let fixture = RuntimeFixture::new("invalid-revision");
    let data_root = TestDirectory::new("invalid-revision-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Invalid revision", &runtime.runtime_id)
        .expect("create profile");

    assert!(matches!(
        core.prepare_launch_snapshot(&profile.profile_id, 0),
        Err(CoreError::InvalidRevision)
    ));
    assert!(matches!(
        core.prepare_launch_snapshot(&profile.profile_id, i64::MAX),
        Err(CoreError::InvalidRevision)
    ));
}

#[test]
fn missing_profile_is_rejected() {
    let data_root = TestDirectory::new("missing-profile-data");
    let core = CoreState::open_at(&data_root.0).expect("open Core");
    let missing = Uuid::new_v4().to_string();

    assert!(matches!(
        core.prepare_launch_snapshot(&missing, 1),
        Err(CoreError::ProfileNotFound { profile_id }) if profile_id == missing
    ));
}

#[test]
fn runtime_artifact_changed_after_registration_is_rejected() {
    let fixture = RuntimeFixture::new("artifact-change");
    let data_root = TestDirectory::new("artifact-change-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Artifact change", &runtime.runtime_id)
        .expect("create profile");
    fs::write(&runtime.game_path, b"changed-game-artifact")
        .expect("replace registered game artifact");

    assert!(matches!(
        core.prepare_launch_snapshot(&profile.profile_id, profile.revision),
        Err(CoreError::RuntimeValidation {
            code: "artifact_size_mismatch"
        })
    ));
    assert_eq!(
        core.inspect_runtime(&runtime.runtime_id)
            .expect("runtime record remains registered")
            .capability_state,
        CapabilityState::NeedsValidation
    );
}

#[test]
fn profile_uuid_not_display_name_owns_distinct_launch_directories() {
    let fixture = RuntimeFixture::new("profile-isolation");
    let data_root = TestDirectory::new("profile-isolation-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let first = core
        .create_profile("Same visible name", &runtime.runtime_id)
        .expect("create first profile");
    let second = core
        .create_profile("Same visible name", &runtime.runtime_id)
        .expect("create second profile");

    let first_snapshot = core
        .prepare_launch_snapshot(&first.profile_id, first.revision)
        .expect("prepare first snapshot");
    let second_snapshot = core
        .prepare_launch_snapshot(&second.profile_id, second.revision)
        .expect("prepare second snapshot");

    assert_ne!(
        first_snapshot.working_directory(),
        second_snapshot.working_directory()
    );
    assert_ne!(
        first_snapshot.microemu_home(),
        second_snapshot.microemu_home()
    );
    assert_ne!(
        first_snapshot.temp_directory(),
        second_snapshot.temp_directory()
    );
    assert_eq!(
        first_snapshot
            .profile_root()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(first.profile_id.as_str())
    );
    assert_eq!(
        second_snapshot
            .profile_root()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(second.profile_id.as_str())
    );
    for path in [
        first_snapshot.profile_root(),
        first_snapshot.microemu_home(),
        first_snapshot.temp_directory(),
        second_snapshot.profile_root(),
        second_snapshot.microemu_home(),
        second_snapshot.temp_directory(),
    ] {
        assert!(!path.to_string_lossy().contains("Same visible name"));
    }
}

#[test]
fn linked_microemu_home_is_rejected() {
    let fixture = RuntimeFixture::new("linked-home");
    let data_root = TestDirectory::new("linked-home-data");
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fixture runtime");
    let profile = core
        .create_profile("Linked home", &runtime.runtime_id)
        .expect("create profile");
    let profile_root = core
        .profile_directory(&profile.profile_id)
        .expect("profile root");
    let link = profile_root.join("microemu-home");
    let target = data_root.0.join("linked-home-target");
    fs::create_dir(&target).expect("create linked-home target");

    #[cfg(unix)]
    symlink(&target, &link).expect("create microemu-home symlink");
    #[cfg(windows)]
    junction::create(&target, &link).expect("create microemu-home junction");

    assert!(matches!(
        core.prepare_launch_snapshot(&profile.profile_id, profile.revision),
        Err(CoreError::InsecureDataRoot)
    ));

    #[cfg(windows)]
    junction::delete(&link).expect("delete microemu-home junction");
}

#[cfg(windows)]
#[test]
#[ignore = "requires provisioned exact Windows runtime; run scripts/Invoke-ExactRuntimeTests.ps1"]
fn exact_windows_runtime_produces_a_needs_validation_snapshot() {
    let runtime_root = std::env::var_os("ZEUS_EXACT_RUNTIME_ROOT")
        .map(PathBuf::from)
        .expect("Invoke-ExactRuntimeTests.ps1 must provide ZEUS_EXACT_RUNTIME_ROOT")
        .canonicalize()
        .expect("canonicalize selected exact runtime root");
    let descriptor = runtime_root.join("runtime-descriptor.json");
    let data_root = TestDirectory::new("exact-windows-data");
    let profile = {
        let mut core = CoreState::open_at(&data_root.0).expect("open registration Core");
        let runtime = core
            .register_runtime_descriptor(&descriptor)
            .expect("register exact Windows runtime");
        core.create_profile("Exact Windows runtime", &runtime.runtime_id)
            .expect("create profile")
    };
    let core = CoreState::open_at(&data_root.0).expect("open fresh Core epoch");

    let cold_started = Instant::now();
    let snapshot = core
        .prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare exact Windows snapshot");
    let cold_elapsed = cold_started.elapsed();
    let cold = core.runtime_preflight_diagnostics();
    let fast_started = Instant::now();
    core.prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prepare exact Windows fast snapshot");
    let fast_elapsed = fast_started.elapsed();
    let fast = core.runtime_preflight_diagnostics();

    assert_eq!(
        snapshot.runtime_id(),
        "windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402"
    );
    assert_eq!(
        snapshot.game_jar(),
        runtime_root.join("game/KnightOnline_402.jar")
    );
    assert_eq!(
        core.inspect_runtime(snapshot.runtime_id())
            .expect("inspect exact runtime")
            .capability_state,
        CapabilityState::NeedsValidation
    );
    assert_eq!(cold.mode(), Some(RuntimePreflightMode::FullValidation));
    assert!(cold.jre_content_bytes_hashed() > 0);
    assert_eq!(fast.mode(), Some(RuntimePreflightMode::FastMetadata));
    assert_eq!(fast.jre_content_bytes_hashed(), 0);
    eprintln!(
        "exact-windows-preflight cold_us={} cold_bytes={} cold_jre_bytes={} fast_us={} fast_bytes={} fast_jre_bytes={}",
        cold_elapsed.as_micros(),
        cold.content_bytes_hashed(),
        cold.jre_content_bytes_hashed(),
        fast_elapsed.as_micros(),
        fast.content_bytes_hashed(),
        fast.jre_content_bytes_hashed()
    );
}

fn primed_fast_path(label: &str) -> (RuntimeFixture, TestDirectory, CoreState, ProfileRecord) {
    let fixture = RuntimeFixture::new(label);
    let data_root = TestDirectory::new(&format!("{label}-data"));
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");
    let runtime = core
        .register_runtime_descriptor(&fixture.descriptor_path)
        .expect("register fast-path fixture");
    let profile = core
        .create_profile("Fast preflight", &runtime.runtime_id)
        .expect("create fast-path profile");
    core.prepare_launch_snapshot(&profile.profile_id, profile.revision)
        .expect("prime fast preflight cache");
    assert_eq!(
        core.runtime_preflight_diagnostics().mode(),
        Some(RuntimePreflightMode::FastMetadata)
    );
    (fixture, data_root, core, profile)
}

fn fixture_java_path(fixture: &RuntimeFixture) -> PathBuf {
    fixture.root.join(if cfg!(windows) {
        "jre/bin/javaw.exe"
    } else {
        "jre/bin/java"
    })
}

fn assert_preflight_failure_evicts(
    core: &CoreState,
    profile: &ProfileRecord,
    expected_code: &'static str,
) {
    match core.prepare_launch_snapshot(&profile.profile_id, profile.revision) {
        Err(CoreError::RuntimeValidation { code }) => assert_eq!(code, expected_code),
        Err(other) => panic!("expected RuntimeValidation({expected_code}), got {other:?}"),
        Ok(_) => panic!("expected RuntimeValidation({expected_code}), got success"),
    }
    assert_eq!(core.runtime_preflight_diagnostics().cache_entries(), 0);
}

fn mutate_file_same_size(path: &Path) {
    let mut bytes = fs::read(path).expect("read same-size mutation fixture");
    bytes[0] ^= 0xff;
    fs::write(path, bytes).expect("write same-size mutation fixture");
}

fn prefixed_os(prefix: &str, path: &Path) -> OsString {
    let mut value = OsString::from(prefix);
    value.push(path.as_os_str());
    value
}

#[cfg(windows)]
fn invocation_path(canonical: &Path) -> PathBuf {
    let units = canonical.as_os_str().encode_wide().collect::<Vec<_>>();
    let verbatim_prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    assert!(units.starts_with(&verbatim_prefix));
    let invocation = PathBuf::from(OsString::from_wide(&units[verbatim_prefix.len()..]));
    assert_native_invocation_identity(&invocation, canonical);
    invocation
}

#[cfg(not(windows))]
fn invocation_path(canonical: &Path) -> PathBuf {
    canonical.to_owned()
}

#[cfg(windows)]
fn assert_native_invocation_identity(invocation: &Path, canonical: &Path) {
    let units = invocation.as_os_str().encode_wide().collect::<Vec<_>>();
    assert!(!units.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]));
    assert_eq!(fs::canonicalize(invocation).unwrap(), canonical);
}

#[cfg(not(windows))]
fn assert_native_invocation_identity(invocation: &Path, canonical: &Path) {
    assert_eq!(invocation, canonical);
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

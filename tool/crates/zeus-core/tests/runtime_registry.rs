#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;
use zeus_core::runtime::CapabilityState;
use zeus_core::{CoreError, CoreState};

#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;
use runtime_fixture::RuntimeFixture;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "zeus-runtime-registry-integration-{}",
            Uuid::new_v4()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
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
fn validated_fixture_registers_idempotently_and_remains_needs_validation() {
    let runtime_fixture = RuntimeFixture::new("runtime-registry");
    let data_root = TestDirectory::new();
    let mut core = CoreState::open_at(&data_root.0).expect("open Core");

    let first = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("register fixture runtime");
    let repeated = core
        .register_runtime_descriptor(runtime_fixture.descriptor_path())
        .expect("repeat fixture registration");

    assert_eq!(first, repeated);
    assert_eq!(first.capability_state, CapabilityState::NeedsValidation);
    let page = core.list_runtimes(None, 100).expect("list runtimes");
    assert_eq!(page.items, vec![first]);
    assert_eq!(page.next_cursor, None);
}

/// Copies a runtime tree into a fresh private root so the copy carries inherited child ACLs.
fn copy_runtime_tree(source: &Path, destination: &Path) {
    drop(CoreState::open_at(destination).expect("create private relocation root"));
    copy_contents(source, destination);
}

fn copy_contents(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source).expect("enumerate runtime source") {
        let entry = entry.expect("read runtime source entry");
        let target = destination.join(entry.file_name());
        if entry
            .metadata()
            .expect("runtime source entry metadata")
            .is_dir()
        {
            if !target.exists() {
                fs::create_dir(&target).expect("create runtime copy directory");
            }
            copy_contents(&entry.path(), &target);
        } else if !target.exists() {
            fs::copy(entry.path(), &target).expect("copy runtime file");
        }
    }
}

#[test]
fn relocated_runtime_tree_repoints_every_stored_path_and_keeps_identity() {
    let source = RuntimeFixture::new("runtime-relocated");
    let original_root = fs::canonicalize(
        source
            .descriptor_path()
            .parent()
            .expect("source runtime root"),
    )
    .expect("canonicalize source runtime root");
    let data_root = TestDirectory::new();
    let registered = {
        let mut core = CoreState::open_at(data_root.path()).expect("open Core");
        core.register_runtime_descriptor(source.descriptor_path())
            .expect("register source runtime")
    };
    assert_eq!(registered.runtime_root, original_root);

    let moved = TestDirectory::new();
    copy_runtime_tree(&original_root, moved.path());
    let moved_root = fs::canonicalize(moved.path()).expect("canonicalize moved runtime root");

    let core = CoreState::open_portable_at(data_root.path(), &moved_root)
        .expect("open portable Core against the moved runtime");
    let relocated = core
        .inspect_runtime(&registered.runtime_id)
        .expect("inspect relocated runtime");

    assert_eq!(relocated.runtime_id, registered.runtime_id);
    assert_eq!(relocated.descriptor_sha256, registered.descriptor_sha256);
    assert_eq!(
        relocated.jre_manifest_sha256,
        registered.jre_manifest_sha256
    );
    assert_eq!(relocated.game_sha256, registered.game_sha256);
    assert_eq!(relocated.capability_state, CapabilityState::NeedsValidation);
    assert_eq!(relocated.created_at_unix_ms, registered.created_at_unix_ms);

    for path in [
        &relocated.descriptor_path,
        &relocated.runtime_root,
        &relocated.java_path,
        &relocated.microemulator_path,
        &relocated.game_path,
    ] {
        assert!(
            path.starts_with(&moved_root),
            "stored path {} must point under the moved root",
            path.display()
        );
        assert!(
            !path.starts_with(&original_root),
            "stored path {} still references the original root",
            path.display()
        );
    }

    // Spec section 6 requires proving the old path is never launched, not merely never stored: build a
    // real launch spec and sweep every path and argument it carries.
    let profile = {
        let mut core = core;
        let profile = core
            .create_profile("Relocated Account", &registered.runtime_id)
            .expect("bind a profile to the relocated runtime");
        let snapshot = core
            .prepare_launch_snapshot(&profile.profile_id, profile.revision)
            .expect("prepare a snapshot from the relocated runtime");
        for path in [
            snapshot.runtime_root(),
            snapshot.java_executable(),
            snapshot.microemulator_jar(),
            snapshot.game_jar(),
            snapshot.working_directory(),
            snapshot.microemu_home(),
            snapshot.temp_directory(),
            snapshot.profile_root(),
        ] {
            assert!(
                !path.starts_with(&original_root),
                "snapshot path {} still references the original root",
                path.display()
            );
        }
        let spec = snapshot
            .process_launch_spec()
            .expect("build a launch spec from the relocated runtime");
        assert!(
            !spec.executable().starts_with(&original_root),
            "launch executable {} still references the original root",
            spec.executable().display()
        );
        let original_text = original_root.as_os_str().to_string_lossy().to_lowercase();
        for argument in spec.arguments() {
            assert!(
                !argument
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&original_text),
                "launch argument still references the original root"
            );
        }
        profile
    };
    assert_eq!(profile.revision, 1);
    assert_eq!(relocated.runtime_root, moved_root);
}

/// The scenario that used to cost the operator every account: rebuild the jar, reopen, keep working.
#[test]
fn refreshed_game_jar_repins_the_runtime_and_keeps_the_bound_profile() {
    let source = RuntimeFixture::new("runtime-repin");
    let runtime_root = fs::canonicalize(
        source
            .descriptor_path()
            .parent()
            .expect("source runtime root"),
    )
    .expect("canonicalize source runtime root");
    let data_root = TestDirectory::new();
    let (registered, profile) = {
        let mut core = CoreState::open_at(data_root.path()).expect("open Core");
        let registered = core
            .register_runtime_descriptor(source.descriptor_path())
            .expect("register source runtime");
        let profile = core
            .create_profile("Repinned Account", &registered.runtime_id)
            .expect("bind a profile before the rebuild");
        (registered, profile)
    };

    source.rebuild_game_jar(b"fixture-game-402-rebuilt-with-a-new-module");

    let core = CoreState::open_portable_at(data_root.path(), &runtime_root)
        .expect("a rebuilt game jar must not block opening the data root");
    let repinned = core
        .inspect_runtime(&registered.runtime_id)
        .expect("inspect re-pinned runtime");

    // Identity holds, so every profile binding survives untouched.
    assert_eq!(repinned.runtime_id, registered.runtime_id);
    assert_eq!(repinned.created_at_unix_ms, registered.created_at_unix_ms);
    assert_eq!(repinned.target_os, registered.target_os);
    assert_eq!(repinned.target_arch, registered.target_arch);
    assert_eq!(repinned.game_bundle, registered.game_bundle);
    assert_eq!(repinned.capability_state, CapabilityState::NeedsValidation);

    // Content follows the tree that just validated.
    assert_ne!(repinned.descriptor_sha256, registered.descriptor_sha256);
    assert_ne!(repinned.game_sha256, registered.game_sha256);
    assert_eq!(
        repinned.microemulator_sha256,
        registered.microemulator_sha256
    );
    assert_eq!(repinned.jre_manifest_sha256, registered.jre_manifest_sha256);

    let repin = core
        .last_runtime_repin()
        .expect("a re-pin must be reported, not silent");
    assert_eq!(repin.runtime_id, registered.runtime_id);
    assert_eq!(
        repin.previous_descriptor_sha256,
        registered.descriptor_sha256
    );
    assert_eq!(repin.descriptor_sha256, repinned.descriptor_sha256);
    assert_eq!(repin.previous_game_sha256, registered.game_sha256);
    assert_eq!(repin.game_sha256, repinned.game_sha256);
    assert!(
        repin.game_jar_only(),
        "rebuilding only the game jar must be reported as such"
    );

    // The profile is still bound and still launchable against the refreshed content.
    let reloaded = core
        .inspect_profile(&profile.profile_id)
        .expect("the bound profile must survive a re-pin");
    assert_eq!(reloaded.runtime_id, registered.runtime_id);
    assert_eq!(reloaded.revision, profile.revision);
    let snapshot = core
        .prepare_launch_snapshot(&reloaded.profile_id, reloaded.revision)
        .expect("prepare a snapshot from the re-pinned runtime");
    assert_eq!(snapshot.runtime_root(), runtime_root);
}

/// A re-pin follows content, never identity: the bundle names which game the row is for.
#[test]
fn repin_refuses_a_descriptor_that_renames_the_game_bundle() {
    let source = RuntimeFixture::new("runtime-repin-bundle");
    let runtime_root = fs::canonicalize(
        source
            .descriptor_path()
            .parent()
            .expect("source runtime root"),
    )
    .expect("canonicalize source runtime root");
    let data_root = TestDirectory::new();
    let registered = {
        let mut core = CoreState::open_at(data_root.path()).expect("open Core");
        core.register_runtime_descriptor(source.descriptor_path())
            .expect("register source runtime")
    };

    source.set_game_bundle("500");

    assert!(matches!(
        CoreState::open_portable_at(data_root.path(), &runtime_root),
        Err(CoreError::RuntimeRegistryMismatch { runtime_id }) if runtime_id == registered.runtime_id
    ));

    let core = CoreState::open_at(data_root.path()).expect("reopen Core after the bounded failure");
    let unchanged = core
        .inspect_runtime(&registered.runtime_id)
        .expect("inspect unchanged runtime");
    assert_eq!(unchanged, registered);
    assert!(
        core.last_runtime_repin().is_none(),
        "a refused re-pin must report nothing"
    );
}

/// A descriptor whose stated digest does not match the artifact on disk still fails validation.
#[test]
fn repin_still_rejects_a_descriptor_that_disagrees_with_its_own_artifacts() {
    let source = RuntimeFixture::new("runtime-repin-corrupt");
    let runtime_root = fs::canonicalize(
        source
            .descriptor_path()
            .parent()
            .expect("source runtime root"),
    )
    .expect("canonicalize source runtime root");
    let data_root = TestDirectory::new();
    let registered = {
        let mut core = CoreState::open_at(data_root.path()).expect("open Core");
        core.register_runtime_descriptor(source.descriptor_path())
            .expect("register source runtime")
    };

    // The jar changes but the descriptor keeps claiming the old size and digest.
    fs::write(
        runtime_root.join("game").join("KnightOnline_402.jar"),
        b"fixture-game-402-swapped-behind-the-descriptor",
    )
    .expect("swap the game jar without restating the descriptor");

    assert!(matches!(
        CoreState::open_portable_at(data_root.path(), &runtime_root),
        Err(CoreError::RuntimeValidation { .. })
    ));

    let core = CoreState::open_at(data_root.path()).expect("reopen Core after the bounded failure");
    assert_eq!(
        core.inspect_runtime(&registered.runtime_id)
            .expect("inspect unchanged runtime"),
        registered
    );
}

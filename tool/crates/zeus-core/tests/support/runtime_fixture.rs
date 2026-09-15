use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeus_core::CoreState;

pub(crate) struct RuntimeFixture {
    root: PathBuf,
    descriptor_path: PathBuf,
}

impl RuntimeFixture {
    pub(crate) fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("zeus-shared-runtime-{label}-{}", Uuid::new_v4()));
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

        let microemulator_path = root.join("microemulator/microemulator.jar");
        let game_path = root.join("game/KnightOnline_402.jar");
        fs::create_dir_all(
            microemulator_path
                .parent()
                .expect("MicroEmulator parent directory"),
        )
        .expect("create MicroEmulator directory");
        fs::create_dir_all(game_path.parent().expect("game parent directory"))
            .expect("create game directory");
        fs::write(&microemulator_path, b"fixture-microemulator")
            .expect("write MicroEmulator fixture");
        fs::write(&game_path, b"fixture-game-402").expect("write game fixture");

        let manifest = jre_files
            .iter()
            .map(|relative| {
                let path = jre.join(relative);
                let bytes = fs::read(&path).expect("read JRE fixture");
                format!("{}  {}  {relative}\n", sha256(&bytes), bytes.len())
            })
            .collect::<String>();
        fs::write(root.join("jre-files.sha256"), manifest.as_bytes()).expect("write JRE manifest");

        let (target_os, target_arch) = host_tuple();
        let runtime_id = format!("{target_os}-{target_arch}_fixture-java11_microemu204_ko402");
        let descriptor = json!({
            "schema_version": 1,
            "runtime_id": runtime_id,
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
                "jar_size": file_size(&microemulator_path),
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
                "jar_size": file_size(&game_path),
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
            serde_json::to_vec_pretty(&descriptor).expect("serialize fixture descriptor"),
        )
        .expect("write fixture descriptor");

        Self {
            root,
            descriptor_path,
        }
    }

    pub(crate) fn descriptor_path(&self) -> &Path {
        &self.descriptor_path
    }

    /// Replaces the game jar in place and re-states its size and digest in the descriptor.
    ///
    /// This is what a jar rebuild looks like to the registry: the runtime id, platform and bundle
    /// hold, while the game digest — and therefore the descriptor digest — move.
    #[allow(dead_code, reason = "used by the re-pin tests only")]
    pub(crate) fn rebuild_game_jar(&self, bytes: &[u8]) {
        let game_path = self.root.join("game").join("KnightOnline_402.jar");
        fs::write(&game_path, bytes).expect("rewrite fixture game jar");
        self.patch_descriptor(|descriptor| {
            let game = &mut descriptor["game"];
            game["jar_size"] = json!(file_size(&game_path));
            game["jar_sha256"] = json!(sha256_path(&game_path));
        });
    }

    /// Renames the bundle without touching any artifact, so only the identity field moves.
    #[allow(dead_code, reason = "used by the re-pin tests only")]
    pub(crate) fn set_game_bundle(&self, bundle: &str) {
        self.patch_descriptor(|descriptor| {
            descriptor["game"]["bundle"] = json!(bundle);
        });
    }

    fn patch_descriptor(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let text = fs::read_to_string(&self.descriptor_path).expect("read fixture descriptor");
        let mut descriptor: serde_json::Value =
            serde_json::from_str(&text).expect("parse fixture descriptor");
        edit(&mut descriptor);
        fs::write(
            &self.descriptor_path,
            serde_json::to_vec_pretty(&descriptor).expect("serialize patched fixture descriptor"),
        )
        .expect("write patched fixture descriptor");
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        if self.root.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn host_tuple() -> (&'static str, &'static str) {
    let os = if cfg!(windows) { "windows" } else { "ubuntu" };
    let architecture = if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    (os, architecture)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).expect("fixture metadata").len()
}

fn sha256_path(path: &Path) -> String {
    sha256(&fs::read(path).expect("read fixture for hashing"))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

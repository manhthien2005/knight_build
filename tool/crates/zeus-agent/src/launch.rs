//! Builds the JVM invocation for one account.
//!
//! Target path: `Tool/tool/crates/zeus-agent/src/launch.rs`
//!
//! The argv shape below is not a guess. Three forms were measured in the `knight-potato`
//! container on 2026-09-12 (JDK 11.0.32, the same build as production):
//!
//! | form | result |
//! |---|---|
//! | `-jar me.jar … game.jar TemMidlet` | FAIL — `MalformedURLException: Unable to find class com.silverknight.TemMidlet URL` |
//! | `-jar me.jar … game.jar` (what `docker-build/bin/entrypoint.sh` uses) | `openJar` succeeds, **the MIDlet never starts**, no snapshot is published |
//! | `-cp me.jar:game.jar org.microemu.app.Main … TemMidlet` | **PASS** — MIDlet runs, snapshot published, `ctl=1` |
//!
//! The reason is the classpath: with `-jar`, the game jar is not on the classpath, so
//! `MIDletClassLoader` cannot resolve the MIDlet class and `Main` falls through to the launcher
//! that waits for a human to pick a suite. Over noVNC that looks like "it needs a click"; in a
//! headless container it is a process that runs forever and does nothing.
//!
//! `docker-build/bin/entrypoint.sh` therefore has never started the MIDlet unattended. Its README
//! claims two tabs reached the game menu, which either involved a click through noVNC or a
//! different condition than the one measured. Re-measure after A2 lands.
//!
//! The authoritative source for these values is `zeus_core::wire` plus the runtime descriptor;
//! this module only assembles them.

use std::path::{Path, PathBuf};
use std::process::Command;

use zeus_core::wire::{CONTROL_FILE_NAME, SNAPSHOT_FILE_NAME};

/// MicroEmulator's entry point. From `runtime-descriptor.json`: `"main_class"`.
pub const MAIN_CLASS: &str = "org.microemu.app.Main";

/// The MIDlet inside the game jar. From `runtime-descriptor.json`: `"midlet_class"`.
///
/// Required as the trailing argument. With `-cp` it resolves; with `-jar` it does not.
pub const MIDLET_CLASS: &str = "com.silverknight.TemMidlet";

/// Paint throttle control file. A third transport, deliberately separate from the control
/// contract: it is fail-SAFE (unreadable keeps the current mode) rather than fail-closed, and
/// folding it into `zeus-control.txt` would mean bumping `CTL_VERSION` for something that is not
/// game logic. See docs/full_spec/tool/WIRE-CONTRACT.md §7.
pub const POTATO_FILE_NAME: &str = "potato.ctl";

/// Per-account filesystem layout.
///
/// `home` is what `-Duser.home` points at, which is also `zeus-core`'s notion of
/// `microemu_home`: MicroEmulator creates `.microemulator/` (config2.xml + the record stores)
/// inside it, and both transport files live directly in it. Every `wire::*` function takes this
/// same directory, so the writer and the reader cannot disagree about where the files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPaths {
    /// `/opt/knight/accounts/<slot>/home` — chmod 0700.
    pub home: PathBuf,
    /// `/opt/knight/accounts/<slot>/tmp`.
    pub tmp: PathBuf,
}

impl AccountPaths {
    /// Build paths for a given slot index. Layout: `/opt/knight/accounts/<slot>/home`.
    pub fn for_slot(slot_index: i32) -> Self {
        let base = PathBuf::from(format!("/opt/knight/accounts/{slot_index}"));
        Self {
            home: base.join("home"),
            tmp: base.join("tmp"),
        }
    }

    pub fn control_file(&self) -> PathBuf {
        self.home.join(CONTROL_FILE_NAME)
    }
    pub fn snapshot_file(&self) -> PathBuf {
        self.home.join(SNAPSHOT_FILE_NAME)
    }
    pub fn potato_file(&self) -> PathBuf {
        self.home.join(POTATO_FILE_NAME)
    }
    pub fn spot_request_file(&self) -> PathBuf {
        self.home.join(crate::spot_scan::SPOT_REQUEST_FILE_NAME)
    }
    pub fn spot_result_payload_file(&self) -> PathBuf {
        self.home.join(crate::spot_scan::SPOT_RESULT_PAYLOAD_FILE_NAME)
    }
    pub fn spot_result_ready_file(&self) -> PathBuf {
        self.home.join(crate::spot_scan::SPOT_RESULT_READY_FILE_NAME)
    }
    pub fn inventory_file(&self) -> PathBuf {
        self.home.join(crate::inventory::INVENTORY_FILE_NAME)
    }
    pub fn enhancement_request_file(&self) -> PathBuf {
        self.home.join(crate::enhancement::ENHANCE_REQUEST_FILE_NAME)
    }
    pub fn enhancement_status_file(&self) -> PathBuf {
        self.home.join(crate::enhancement::ENHANCE_STATUS_FILE_NAME)
    }
    pub fn enhancement_cancel_file(&self) -> PathBuf {
        self.home.join(crate::enhancement::ENHANCE_CANCEL_FILE_NAME)
    }
}

/// Heap and GC settings. Defaults are the ones measured in `docker-build/README.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapConfig {
    pub initial_mib: u32,
    pub maximum_mib: u32,
    pub stack_kib: u32,
    pub reserved_code_cache_mib: u32,
    pub max_metaspace_mib: u32,
    /// The pair below is load-bearing. `trim` issues a full GC through jattach, and only with
    /// `MaxHeapFreeRatio` set does SerialGC actually *uncommit* the pages so RSS drops. Measured:
    /// without the pair, 281 MB → 281 MB (heap "used" fell, RSS did not); with it, 254 → 194 MB.
    /// Calling GC alone is useless.
    pub min_heap_free_ratio: u32,
    pub max_heap_free_ratio: u32,
}

impl Default for HeapConfig {
    fn default() -> Self {
        Self {
            initial_mib: 8,
            maximum_mib: 320,
            stack_kib: 512,
            reserved_code_cache_mib: 32,
            max_metaspace_mib: 96,
            min_heap_free_ratio: 10,
            max_heap_free_ratio: 25,
        }
    }
}

/// Everything needed to build one invocation.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub java: PathBuf,
    pub microemulator_jar: PathBuf,
    pub game_jar: PathBuf,
    pub paths: AccountPaths,
    pub device_width: u32,
    pub device_height: u32,
    pub heap: HeapConfig,
    /// Run with no display at all. Cuts the Swing window, X11, the blit and VNC encoding.
    ///
    /// It does **not** cut rendering inside the JVM: `NoUiDisplayComponent.repaintRequest` still
    /// calls `paintDisplayable` into an offscreen image with no early return. Use
    /// `potato.ctl` for that. See docs/full_spec/build-docker/RUNTIME-SPEC.md §4.
    pub headless: bool,
    /// 1-based account character slot (1, 2, or 3).
    pub character_slot: i16,
}

impl LaunchSpec {
    /// Builds a LaunchSpec with production defaults for the Docker container layout.
    ///
    /// JRE at `/opt/knight/jre/bin/java`, jars in `/opt/knight/game/`.
    pub fn default_for_paths(paths: AccountPaths) -> Self {
        Self {
            // Paths must match Dockerfile exactly — Issue #02
            java: PathBuf::from("/opt/java/bin/java"),
            microemulator_jar: PathBuf::from("/opt/microemulator-2.0.4/microemulator.jar"),
            game_jar: PathBuf::from("/opt/knight/game/Zeus_Knight.jar"),
            paths,
            device_width: 360,
            device_height: 480,
            heap: HeapConfig::default(),
            headless: false,
            character_slot: 1,
        }
    }

    /// Assembles argv in the exact measured order.
    ///
    /// Order is not free: JVM flags must precede `-cp`, the main class follows the classpath,
    /// emulator options follow the main class, and the MIDlet class is last.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.java);

        // ── system properties ────────────────────────────────────────────────
        // These are the contract between this process and the mod inside the jar. The mod
        // fails closed when they are absent, so a missing property is "every module off",
        // not "defaults".
        command
            .arg(format!("-Duser.home={}", self.paths.home.display()))
            .arg(format!("-Djava.io.tmpdir={}", self.paths.tmp.display()))
            .arg(format!(
                "-Dzeus.player.out={}",
                self.paths.snapshot_file().display()
            ))
            .arg(format!(
                "-Dzeus.ctl.in={}",
                self.paths.control_file().display()
            ))
            .arg(format!(
                "-Dpotato.ctl={}",
                self.paths.potato_file().display()
            ))
            .arg(format!(
                "-Dzeus.auth.slot={}",
                self.character_slot - 1
            ));

        if self.headless {
            command.arg("-Djava.awt.headless=true");
        }

        // ── heap + GC ────────────────────────────────────────────────────────
        let heap = self.heap;
        command
            .arg(format!("-Xms{}m", heap.initial_mib))
            .arg(format!("-Xmx{}m", heap.maximum_mib))
            .arg(format!("-Xss{}k", heap.stack_kib))
            .arg("-XX:+UseSerialGC")
            .arg("-XX:-UsePerfData")
            .arg(format!(
                "-XX:ReservedCodeCacheSize={}m",
                heap.reserved_code_cache_mib
            ))
            .arg(format!("-XX:MaxMetaspaceSize={}m", heap.max_metaspace_mib))
            .arg(format!("-XX:MinHeapFreeRatio={}", heap.min_heap_free_ratio))
            .arg(format!("-XX:MaxHeapFreeRatio={}", heap.max_heap_free_ratio))
            // Crash dumps land in the account's own tmp, not a shared directory, so one
            // account's hs_err cannot be mistaken for another's.
            .arg(format!(
                "-XX:ErrorFile={}/hs_err_%p.log",
                self.paths.tmp.display()
            ));

        // Cap glibc arenas from the environment side instead — see `environment()`.

        // ── classpath, then main class ───────────────────────────────────
        // `:` is the Linux separator. This is the line that makes the MIDlet resolvable.
        command
            .arg("-Xshare:auto") // Activate CDS archive built in Dockerfile Stage 1 — Issue #107
            .arg("-cp")
            .arg(format!(
                "{}:{}",
                self.microemulator_jar.display(),
                self.game_jar.display()
            ))
            .arg(MAIN_CLASS);

        // ── emulator options ─────────────────────────────────────────────────
        // Per-account user.home provides MicroEmulator isolation, so no `--id` is passed.
        // Omitting `--id` ensures MicroEmulator config root is `<user.home>/.microemulator/`
        // and its RMS directory is `<user.home>/.microemulator/suite-null/`, aligning with Zeus RMS.
        command
            .arg("--resizableDevice")
            .arg(self.device_width.to_string())
            .arg(self.device_height.to_string())
            .arg("--rms")
            .arg("file");

        // `--quit` is what makes supervision work: the JVM exits when the MIDlet is destroyed
        // instead of lingering as an empty window, so `waitpid` reports the death.
        // `--quiet` keeps the emulator's own chatter out of the account log.
        command.arg("--quiet").arg("--quit");

        // ── MIDlet class, last ───────────────────────────────────────────────
        command.arg(MIDLET_CLASS);

        command
    }

    /// Environment for the child. Kept minimal on purpose: an inherited environment can carry
    /// a `DISPLAY` or a `_JAVA_OPTIONS` that changes behaviour in ways the argv cannot show.
    pub fn environment(&self, display: &str) -> Vec<(&'static str, String)> {
        vec![
            ("DISPLAY", display.to_owned()),
            ("HOME", self.paths.home.to_string_lossy().into_owned()),
            // Caps glibc malloc arenas. Without it, native RSS drifts upward over a multi-day
            // session, which reads as a leak and is not one.
            ("MALLOC_ARENA_MAX", "2".to_owned()),
        ]
    }
}

/// Resolves the runtime layout inside the container.
///
/// Paths are fixed rather than configured: the image owns them, and a configurable path here
/// would be a second place for the transport contract to disagree with itself.
pub fn container_paths(slot: &str) -> AccountPaths {
    let root = Path::new("/opt/knight/accounts").join(slot);
    AccountPaths {
        home: root.join("home"),
        tmp: root.join("tmp"),
    }
}

/// Creates the per-account directories with the permissions the credential path requires.
///
/// 0700 on `home` because the record stores hold the plaintext password the client must be able
/// to read, and because the agent's own key material lives under `/opt/knight/state`. The threat
/// model here is narrow and deliberate: the node itself is allowed to be compromised, but one
/// compromised node must not yield any other user's credentials.
pub fn prepare_directories(paths: &AccountPaths) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for dir in [&paths.home, &paths.tmp] {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> LaunchSpec {
        LaunchSpec {
            java: PathBuf::from("/opt/java/bin/java"),
            microemulator_jar: PathBuf::from("/opt/microemulator-2.0.4/microemulator.jar"),
            game_jar: PathBuf::from("/opt/knight/game/Zeus_Knight.jar"),
            paths: container_paths("acc1"),
            device_width: 360,
            device_height: 480,
            heap: HeapConfig::default(),
            headless: false,
            character_slot: 1,
        }
    }

    fn argv(s: &LaunchSpec) -> Vec<String> {
        s.command()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// The regression that matters: `-cp`, never `-jar`. A `-jar` invocation loads the suite and
    /// then waits for a human, which in a container is an immortal no-op.
    #[test]
    fn uses_classpath_and_never_the_jar_switch() {
        let args = argv(&spec());
        assert!(
            !args.iter().any(|a| a == "-jar"),
            "-jar cannot resolve the MIDlet"
        );

        let cp = args.iter().position(|a| a == "-cp").expect("no -cp");
        assert_eq!(
            args[cp + 1],
            "/opt/microemulator-2.0.4/microemulator.jar:/opt/knight/game/Zeus_Knight.jar"
        );
        assert_eq!(args[cp + 2], MAIN_CLASS);
        assert_eq!(args.last().unwrap(), MIDLET_CLASS);
    }

    /// Both transport properties must be present and must point at the same directory the
    /// writer uses. If they ever diverge, the mod fails closed and reports `ctl=-1`.
    #[test]
    fn transport_properties_point_at_the_account_home() {
        let args = argv(&spec());
        let home = "/opt/knight/accounts/acc1/home";
        assert!(args.contains(&format!("-Duser.home={home}")));
        assert!(args.contains(&format!("-Dzeus.ctl.in={home}/{CONTROL_FILE_NAME}")));
        assert!(args.contains(&format!("-Dzeus.player.out={home}/{SNAPSHOT_FILE_NAME}")));
        assert!(args.contains(&format!("-Dpotato.ctl={home}/{POTATO_FILE_NAME}")));
    }

    /// `--quit` is what lets the supervisor notice a death. Without it the JVM outlives the
    /// MIDlet and `waitpid` never returns.
    #[test]
    fn quits_when_the_midlet_is_destroyed() {
        let args = argv(&spec());
        assert!(args.contains(&"--quit".to_string()));
        assert!(args.contains(&"--rms".to_string()));
        assert!(args.contains(&"file".to_string()));
    }

    /// Regression test: `--id` must NOT be passed to MicroEmulator.
    ///
    /// MicroEmulator `--id <id>` moves its config root from `<user.home>/.microemulator/`
    /// to `<user.home>/.microemulator/<id>/`, which broke auto-login because the RMS stores
    /// were written directly into `<user.home>/.microemulator/suite-null/`.
    /// Per-account `-Duser.home` already provides full isolation across account slots,
    /// so `--id` is omitted and reader/writer agree on one canonical RMS root.
    #[test]
    fn launch_omits_emulator_id_and_aligns_with_canonical_rms_path() {
        let s = spec();
        let args = argv(&s);
        assert!(
            !args.iter().any(|a| a == "--id"),
            "--id must be omitted so MicroEmulator uses <user.home>/.microemulator/suite-null directly"
        );
        let rms_idx = args.iter().position(|a| a == "--rms").expect("missing --rms flag");
        assert_eq!(args.get(rms_idx + 1).map(|s| s.as_str()), Some("file"));

        // Canonical RMS root asserted against pure path helper from wire contract:
        let expected_rms_dir = suite_directory(&s.paths.home);
        assert_eq!(
            expected_rms_dir,
            PathBuf::from("/opt/knight/accounts/acc1/home/.microemulator/suite-null")
        );
    }

    /// The heap-free-ratio pair is what makes the RSS trim actually reclaim. Dropping one of
    /// them silently turns the trim into a no-op that still pauses the JVM.
    #[test]
    fn keeps_the_heap_free_ratio_pair() {
        let args = argv(&spec());
        assert!(args.contains(&"-XX:MinHeapFreeRatio=10".to_string()));
        assert!(args.contains(&"-XX:MaxHeapFreeRatio=25".to_string()));
        assert!(args.contains(&"-XX:+UseSerialGC".to_string()));
    }

    #[test]
    fn headless_adds_only_the_awt_property() {
        let mut headless = spec();
        headless.headless = true;
        let with = argv(&headless);
        let without = argv(&spec());
        assert_eq!(with.len(), without.len() + 1);
        assert!(with.contains(&"-Djava.awt.headless=true".to_string()));
    }

    #[test]
    fn launch_spec_sets_zeus_auth_slot_correctly() {
        let mut s1 = spec();
        s1.character_slot = 1;
        let args1 = argv(&s1);
        assert!(args1.contains(&"-Dzeus.auth.slot=0".to_string()), "Slot 1 -> -Dzeus.auth.slot=0");
        assert_eq!(
            args1.iter().filter(|a| a.starts_with("-Dzeus.auth.slot=")).count(),
            1,
            "No duplicate -Dzeus.auth.slot"
        );

        let mut s2 = spec();
        s2.character_slot = 2;
        let args2 = argv(&s2);
        assert!(args2.contains(&"-Dzeus.auth.slot=1".to_string()), "Slot 2 -> -Dzeus.auth.slot=1");
        assert_eq!(
            args2.iter().filter(|a| a.starts_with("-Dzeus.auth.slot=")).count(),
            1,
            "No duplicate -Dzeus.auth.slot"
        );

        let mut s3 = spec();
        s3.character_slot = 3;
        let args3 = argv(&s3);
        assert!(args3.contains(&"-Dzeus.auth.slot=2".to_string()), "Slot 3 -> -Dzeus.auth.slot=2");
        assert_eq!(
            args3.iter().filter(|a| a.starts_with("-Dzeus.auth.slot=")).count(),
            1,
            "No duplicate -Dzeus.auth.slot"
        );

        // Verify position: system property must appear before -cp and MAIN_CLASS
        let slot_pos = args1.iter().position(|a| a == "-Dzeus.auth.slot=0").unwrap();
        let cp_pos = args1.iter().position(|a| a == "-cp").unwrap();
        assert!(slot_pos < cp_pos, "System property must precede -cp");
    }
}


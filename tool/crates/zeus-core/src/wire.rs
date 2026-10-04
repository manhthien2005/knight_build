//! Public surface for out-of-crate consumers of the two transport files and the record stores.
//!
//! Nothing here adds logic. It exists because `control.rs`, `player.rs` and `rms.rs` are the single
//! implementation of a fail-closed wire format, and a second crate must reuse them rather than
//! re-derive them. Two independent implementations of 35 keys will drift, and drift shows up as
//! the mod silently turning every module off — which is what actually happened on 2026-09-12
//! (docs/full_spec/cross-domain/VERIFY-RESULTS.md §2).
//!
//! If you find yourself writing a `fn` in this file, stop: the logic belongs in the module that
//! owns the format, and its tests are the only tests of that behaviour.
//!
//! Target path: `Tool/tool/crates/zeus-core/src/wire.rs`
//! Requires: the visibility changes in `../ZEUS-CORE-CHANGES.md` §2.

// ── control: driver → jar ────────────────────────────────────────────────────────
//
// `zeus-control.txt`, 35 lines, `CTL_VERSION 13`. The jar polls it every 500 ms against
// `dx.a()` (wall clock) and fails CLOSED: an unknown key, a missing key, a repeat or an
// out-of-range value turns every module off while the JVM keeps running.
//
// `write_settings` is atomic (unique temp beside the target, then replace) and runs
// `clamped()` internally, so a correct caller can never emit a file the jar must refuse.
pub use crate::control::{clear_settings, control_path, parse_settings, read_settings, write_settings};

// ── snapshot: jar → driver ───────────────────────────────────────────────────────
//
// `zeus-player.txt`, 48 keys, `v=6`. `read_snapshot` returns `Ok(None)` when the mod has not
// published yet — the ordinary state before a character is entered. Anything present but
// malformed is an `Err`, and parsing REJECTS rather than clamps, so a jar and a driver that
// disagree about the key set cannot quietly agree on a wrong value.
pub use crate::player::{clear_snapshot, parse_snapshot, read_snapshot, snapshot_path};

// ── health: jar → driver ──────────────────────────────────────────────────────────
//
// `zeus-health.txt`, 6 keys, `HEALTH_VERSION 1`. `read_health` returns `Ok(None)` when the mod
// has not published yet. Anything present but malformed is an `Err`.
pub use crate::health::{clear_health, health_path, parse_health, read_health};

// ── reconnect status: jar → driver ──────────────────────────────────────────────
//
// `zeus-reconnect.txt`, 8 keys, `RECONNECT_STATUS_VERSION 1`. `read_reconnect_status` returns `Ok(None)`
// when the mod has not published yet. Anything present but malformed is an `Err`.
pub use crate::reconnect_status::{
    clear_reconnect_status, parse_reconnect_status, read_reconnect_status, reconnect_status_path,
};

// ── record stores: seeded before the JVM starts ──────────────────────────────────
//
// `user_pass` and `isIndexServer`, written byte-for-byte in the client's own format: every
// payload bitwise complemented (because `com.silverknight.TemMidlet` complements on write and
// again on read), strings in Java *modified* UTF-8 (NUL takes two bytes, a supplementary
// character travels as two surrogates).
//
// This is why plaintext credentials must exist on the node: `bs.c()` reads `user_pass` while
// building the login screen and submits opcode 1 itself. The driver does not drive login.
//
// `seed_credentials` writes the server store FIRST — a present `user_pass` with a stale server
pub use crate::rms::{clear_credentials, seed_credentials, suite_directory};

// ── shared plumbing ──────────────────────────────────────────────────────────────

/// Atomic replace, the same primitive both transport writers use. Needed for `potato.ctl`,
/// which is a third file with its own (fail-SAFE) semantics and deliberately does not join
/// the control contract — see docs/full_spec/tool/WIRE-CONTRACT.md §7.
pub use crate::data_root::atomic_replace;

// ── file names ───────────────────────────────────────────────────────────────────
//
// Exposed so a caller can assert the path it passes to `-Dzeus.ctl.in` is the path the writer
// will use. They can never disagree, which is the point of the launch spec passing this same
// constant through.
pub use crate::control::CONTROL_FILE_NAME;
pub use crate::health::HEALTH_FILE_NAME;
pub use crate::player::SNAPSHOT_FILE_NAME;
pub use crate::reconnect_status::RECONNECT_STATUS_FILE_NAME;

// ── contract constants ───────────────────────────────────────────────────────────
//
// These are what the version gate compares against, and what the CI test in
// `../ZEUS-CORE-CHANGES.md` §4 asserts. The rule they encode: the version number is a
// FUNCTION OF THE KEY SET. Change any key, bump the version, in the same commit.
pub use crate::control::{
    CONTROL_VERSION, CONTROL_VERSION_V13, CONTROL_VERSION_V14, CTL_KEY_COUNT, CTL_KEY_COUNT_V13,
    CTL_KEY_COUNT_V14, CTL_KEY_NAMES, CTL_KEY_NAMES_V13, CTL_KEY_NAMES_V14, MAX_CONTROL_BYTES,
};
pub use crate::health::{HEALTH_VERSION, MAX_HEALTH_BYTES};
pub use crate::player::{MAX_SNAPSHOT_BYTES, SUPPORTED_VERSION};
pub use crate::reconnect_status::{MAX_RECONNECT_STATUS_BYTES, RECONNECT_STATUS_VERSION};

// ── value ranges ─────────────────────────────────────────────────────────────────
//
// Needed by the version gate and by anything that validates before writing. The web form
// hard-codes the same numbers in TypeScript (web/control-schema-v13.ts); this list is the
// authority those numbers were copied from.
pub use crate::control::{
    DUNGEON_MAX_MINUTE, DUNGEON_MIN_MINUTE, DUNGEON_RUNS_MAX, DUNGEON_SCHEDULE_SLOTS,
    DUNGEON_UNSCHEDULED, MAX_NAV_TARGET, MAX_RADIUS, MAX_ZONE_PICK, MIN_RADIUS, MIN_ZONE_PICK,
};

// ── types ────────────────────────────────────────────────────────────────────────
//
// Already re-exported by lib.rs; repeated here so a caller can `use zeus_core::wire::*` and
// get a complete surface without also knowing about the crate root.
pub use crate::control::{
    AttackMode, AttackSpot, ControlSettings, GoldPickup, ItemRank, PotionPickup, ReviveMode,
    ZoneMode,
};
pub use crate::error::{CoreError, CoreResult};
pub use crate::health::{HealthError, HealthScreen, HealthSnapshot};
pub use crate::player::PlayerSnapshot;
pub use crate::reconnect_status::{ReconnectState, ReconnectStatusError, ReconnectStatusSnapshot};
pub use crate::rms::SERVER_NAMES;
pub use crate::control::{
    BUFF_SLOTS, ENHANCE_CHARM_MAX, ENHANCE_LEVEL_MAX, ENHANCE_LEVEL_MIN, MATERIAL_LABELS,
    MATERIAL_SLOTS, MOUNT_ANY, MOUNT_TEMPLATE_IDS, REVIVE_DELAY_MAX,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The facade is a promise that these paths resolve. If a future refactor renames or
    /// re-privatises one, this test fails at compile time with the name in the error — which
    /// is cheaper than discovering it when the agent does not build on Linux.
    #[test]
    fn facade_resolves_every_reused_entry_point() {
        let home = std::path::Path::new("/tmp/wire-facade-probe");

        // Paths are pure; calling them proves visibility without touching the filesystem.
        assert_eq!(control_path(home).file_name().unwrap(), CONTROL_FILE_NAME);
        assert_eq!(snapshot_path(home).file_name().unwrap(), SNAPSHOT_FILE_NAME);
        assert_eq!(health_path(home).file_name().unwrap(), HEALTH_FILE_NAME);
        assert_eq!(reconnect_status_path(home).file_name().unwrap(), RECONNECT_STATUS_FILE_NAME);

        // Functions are referenced, not called, so no I/O happens.
        let _: fn(&std::path::Path, &ControlSettings) -> CoreResult<()> = write_settings;
        let _: fn(&std::path::Path) -> CoreResult<Option<ControlSettings>> = read_settings;
        let _: fn(&std::path::Path) -> CoreResult<()> = clear_settings;
        let _: fn(&str) -> CoreResult<ControlSettings> = parse_settings;
        let _: fn(&std::path::Path) -> CoreResult<Option<PlayerSnapshot>> = read_snapshot;
        let _: fn(&std::path::Path) -> CoreResult<()> = clear_snapshot;
        let _: fn(&str) -> CoreResult<PlayerSnapshot> = parse_snapshot;
        let _: fn(&std::path::Path) -> Result<Option<HealthSnapshot>, HealthError> = read_health;
        let _: fn(&std::path::Path) -> Result<(), HealthError> = clear_health;
        let _: fn(&str) -> Result<HealthSnapshot, HealthError> = parse_health;
        let _: fn(&std::path::Path) -> Result<Option<ReconnectStatusSnapshot>, ReconnectStatusError> =
            read_reconnect_status;
        let _: fn(&std::path::Path) -> Result<(), ReconnectStatusError> = clear_reconnect_status;
        let _: fn(&str) -> Result<ReconnectStatusSnapshot, ReconnectStatusError> =
            parse_reconnect_status;
        let _: fn(&std::path::Path, &str, &str, u8) -> CoreResult<()> = seed_credentials;
        let _: fn(&std::path::Path) -> CoreResult<()> = clear_credentials;
        let _: fn(&std::path::Path) -> std::path::PathBuf = suite_directory;
        let _: fn(&std::path::Path, &std::path::Path) -> CoreResult<()> = atomic_replace;
    }

    /// The two version numbers an agent reports to the cloud, and the two key counts the gate
    /// compares. If any of these moves without the contract doc moving with it, this is the
    /// cheapest place to notice.
    #[test]
    fn contract_numbers_match_the_documented_v15_v6() {
        assert_eq!(CONTROL_VERSION, 15);
        assert_eq!(CTL_KEY_COUNT, 38);
        assert_eq!(CONTROL_VERSION_V14, 14);
        assert_eq!(CTL_KEY_COUNT_V14, 37);
        assert_eq!(CONTROL_VERSION_V13, 13);
        assert_eq!(CTL_KEY_COUNT_V13, 35);
        assert_eq!(SUPPORTED_VERSION, 6);
        assert_eq!(HEALTH_VERSION, 1);
        assert_eq!(HEALTH_FILE_NAME, "zeus-health.txt");
        assert_eq!(MAX_HEALTH_BYTES, 1024);
        assert_eq!(RECONNECT_STATUS_VERSION, 1);
        assert_eq!(RECONNECT_STATUS_FILE_NAME, "zeus-reconnect.txt");
        assert_eq!(MAX_RECONNECT_STATUS_BYTES, 1024);
        assert_eq!(CTL_KEY_NAMES.len(), CTL_KEY_COUNT);
        assert_eq!(CTL_KEY_NAMES[0], "v");
        assert_eq!(CTL_KEY_NAMES[34], "dungeon.startMin");
        assert_eq!(CTL_KEY_NAMES[35], "dungeon.endMin");
        assert_eq!(CTL_KEY_NAMES[36], "ui.effects");
        assert_eq!(CTL_KEY_NAMES[37], "ui.hidePlayers");
    }
}

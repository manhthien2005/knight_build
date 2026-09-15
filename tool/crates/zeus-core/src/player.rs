//! Reads the read-only player snapshot Zeus_Knight publishes.
//!
//! The mod runs inside the game's JVM and cannot share memory with this process, so it
//! writes one small `key=value` file into the profile directory and this module reads it.
//! `docs/core/11-player-transport.md` records why a file rather than a socket: a socket
//! inside the client would be an unauthenticated local endpoint any process could drive.
//!
//! The parser is deliberately strict. An unknown key, a missing key, a value outside its
//! documented range, or an unexpected format version is a rejection rather than a
//! best-effort read, because a silently mis-parsed snapshot would be rendered to the
//! operator as fact. A *missing* file is not an error: it means the mod has not published
//! yet, which is the normal state before login.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult};

/// File name the mod writes inside `microemu-home`.
///
/// The launch specification passes this same path to the JVM as `-Dzeus.player.out`, so the writer
/// and the reader can never disagree about where the snapshot lives.
pub const SNAPSHOT_FILE_NAME: &str = "zeus-player.txt";

/// Format version this parser accepts. A different value is rejected outright.
pub const SUPPORTED_VERSION: u32 = 6;

/// The real snapshot is around 300 bytes; this bounds a hostile or corrupt file.
pub const MAX_SNAPSHOT_BYTES: u64 = 4096;

/// Longest accepted character or guild name, matching the mod's own clamp.
const MAX_NAME_CHARS: usize = 64;

/// One published reading of the live character. Every field is a plain value: no path,
/// no session identity, and never a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSnapshot {
    /// Mod-side wall clock when the snapshot was written, in Unix milliseconds.
    pub written_at_unix_ms: i64,
    pub character_name: String,
    pub level: u16,
    /// Progress through the current level in permille, 0..=1000.
    ///
    /// Not absolute experience: the client only ever knows the percentage, so a total or
    /// a per-level requirement cannot be derived from it.
    pub xp_permille: u16,
    pub hp: i64,
    pub hp_max: i64,
    pub mp: i64,
    pub mp_max: i64,
    /// Whether the wallet has been delivered yet. While false, `gold` and `gem` are
    /// meaningless and must render as unknown rather than as zero.
    pub wallet_known: bool,
    pub gold: i64,
    pub gem: i64,
    /// Map id, or `None` before the map is known.
    pub map_id: Option<u16>,
    pub zone: i16,
    pub pixel_x: i32,
    pub pixel_y: i32,
    /// Remaining attack quota. At zero or below the client silently stops auto-attacking.
    pub quota: i64,
    /// Occupied bag slots, or `None` before the inventory is parsed.
    pub bag_used: Option<i64>,
    pub bag_max: i64,
    /// Raw client state: 4 is dead and 2 is fighting.
    pub state: i32,
    /// Mount id, or `None` when not mounted.
    pub mount: Option<i32>,
    /// Mounts the bag holds, as `(template id, server-given name)`.
    ///
    /// Published by the mod because there is no id-to-name table anywhere in the client to read
    /// instead: an empty list means the character is carrying none, not that the parse failed.
    pub mounts: Vec<(u16, String)>,
    pub guild_name: Option<String>,
    /// XP gain in permille per hour, or `None` when not measurable yet.
    pub xp_permille_per_hour: Option<i64>,
    /// True when the scene was not settled, so these are last-known values.
    pub stale: bool,
    /// The client's own auto phase: -1 off, 0 armed, 1 attacking.
    pub attack_phase: i8,
    /// The mod's own state: `None` when it owns nothing, else 0 fighting, 1 returning, 2 settling.
    pub attack_state: Option<u8>,
    /// Whether a live monster is currently held as the target.
    pub has_target: bool,
    /// 0 fine, 1 no monsters in range, 2 monsters that cannot be reached or hit.
    pub stuck: u8,
    /// Potions drunk since the client started.
    pub potions: i64,
    /// Times this session put the character back on its feet.
    pub revives: i64,
    /// The pickup record actually in force, read back from the client: rank threshold, potion mode,
    /// gold mode. `None` means the client's collector is off.
    ///
    /// Read back rather than echoed from the settings, because `co.b()` and its own read-back
    /// disagree about which byte carries which value: only the client can say what took effect.
    pub pickup: Option<(i8, i8, i8)>,
    /// Which buff slots the client will really cast, after its own learned check.
    pub buffs: [bool; 3],
    /// Each material's close-drop state as the server confirmed it: `None` never confirmed,
    /// `Some(true)` closed. Read back rather than echoed, so a flip that never landed shows as
    /// unknown instead of as applied.
    pub materials: [Option<bool>; 6],
    /// Where TRAVEL is: 0 off, 1 idle, 2 waiting on a teleport stone, 3 walking, 4 arrived,
    /// 5 stopped. Anything but 4 while the character is off its map means it is still on the way.
    pub travel_state: u8,
    /// Why TRAVEL stopped: 0 not stopped, 1 no route, 2 the map has no usable exit or the walk
    /// stalled, 3 the stone refused the selection, 4 the hop cap, 5 an exception.
    ///
    /// Separate from `travel_state` because "stopped" without a reason is the failure mode this key
    /// exists to prevent: a route that quietly gives up looks the same as one still walking.
    pub travel_why: u8,
    /// Destination map, or `None` when travel is off.
    pub travel_goal: Option<u16>,
    /// Map borders crossed since the current route started.
    pub travel_hops: u16,
    /// Whether the tool and the mod agree about the settings: `None` when the client was launched
    /// without a settings path, else false when nothing readable was found and true when the last
    /// read parsed.
    ///
    /// Failing closed is silent by design, so without this "auto does nothing" and "the settings
    /// file is one key out of date" look identical from outside the client.
    pub settings_agreed: Option<bool>,
    /// Which step of an enhancement run the mod is on: 0 idle, 1 walking to the blacksmith,
    /// 2 opening the NPC, 3 in its menu, 4 choosing the item, 5 choosing the charm, 6 confirming,
    /// 7 waiting for the result.
    pub enhance_phase: u8,
    /// Why an enhancement run is not progressing: 0 fine, 1 no blacksmith reachable, 2 out of
    /// charms, 3 the item it was working on is gone, 4 the target level is reached.
    pub enhance_why: u8,
    /// Items finished this session. Not reset by a settings change, so a run that is switched off
    /// and back on still shows what it already did.
    pub enhance_done: u16,
    // ---- DUNGEON ----
    /// Where the dungeon loop is: 0 off, 1 idle, 2 walking to the officer, 3 talking to it,
    /// 4 inside a run, 5 a run finished. Anything but 4 while a run was asked for means the
    /// character has not got into the dungeon yet.
    pub dungeon_state: u8,
    /// Why the dungeon loop is not progressing: 0 fine, 1 no officer or the character is off the
    /// officer's map, 2 its menu would not open, 3 the walk stalled, 4 the run limit is reached.
    ///
    /// Separate from `dungeon_state` for the same reason as [`Self::travel_why`]: a loop that has
    /// quietly given up must not read the same as one still waiting for a menu reply.
    pub dungeon_why: u8,
    /// Runs completed this session. Not reset by a settings change, so a loop switched off and
    /// back on still shows what it already did.
    pub dungeon_runs: u16,
    /// The dungeon's own map id while the loop is armed, or `None` when it is off. Mirrors
    /// [`Self::travel_goal`]: the mod publishes -1 rather than omitting the key, so an un-armed
    /// loop is a value rather than a missing one.
    pub dungeon_goal: Option<u16>,
    // ---- end DUNGEON ----
}

impl PlayerSnapshot {
    /// Whether the character was dead at publication.
    pub fn is_dead(&self) -> bool {
        self.state == 4
    }

    /// HP as a percentage, or `None` when the maximum is not yet known.
    ///
    /// `hp_max` is genuinely zero on entry, before the stats packet arrives, so every
    /// ratio here is guarded rather than assumed positive.
    pub fn hp_percent(&self) -> Option<u32> {
        percent(self.hp, self.hp_max)
    }

    pub fn mp_percent(&self) -> Option<u32> {
        percent(self.mp, self.mp_max)
    }

    /// Whether the attack quota is exhausted, which stops auto-attack with no message.
    pub fn quota_exhausted(&self) -> bool {
        self.quota <= 0
    }

    /// Whether the mod is holding the client's combat fields.
    ///
    /// Derived from the mod's own state rather than the client's phase: the client raises its phase
    /// for a manual attack too, so the phase alone cannot say whether auto is running.
    pub fn auto_running(&self) -> bool {
        self.attack_state.is_some()
    }
}

fn percent(value: i64, maximum: i64) -> Option<u32> {
    if maximum <= 0 {
        return None;
    }
    let clamped = value.clamp(0, maximum);
    u32::try_from(clamped.saturating_mul(100) / maximum).ok()
}

fn snapshot_error(code: &'static str) -> CoreError {
    CoreError::PlayerSnapshot { code }
}

/// Path of the snapshot for one profile.
pub fn snapshot_path(microemu_home: &Path) -> PathBuf {
    microemu_home.join(SNAPSHOT_FILE_NAME)
}

/// Reads and parses one profile's snapshot.
///
/// `Ok(None)` means the mod has not published yet, which is the ordinary state before a
/// character is entered. Anything present but malformed is an error.
pub fn read_snapshot(microemu_home: &Path) -> CoreResult<Option<PlayerSnapshot>> {
    let path = snapshot_path(microemu_home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CoreError::io("inspect player snapshot", error)),
    };
    // A snapshot is a plain file the mod wrote. A link here would mean something else is
    // redirecting the read, so it fails closed rather than following it.
    if !metadata.is_file() || crate::data_root::metadata_is_link_or_reparse(&metadata) {
        return Err(snapshot_error("player_snapshot_not_a_file"));
    }
    if metadata.len() > MAX_SNAPSHOT_BYTES {
        return Err(snapshot_error("player_snapshot_too_large"));
    }
    let bytes = fs::read(&path).map_err(|error| CoreError::io("read player snapshot", error))?;
    let text = String::from_utf8(bytes).map_err(|_| snapshot_error("player_snapshot_not_utf8"))?;
    parse_snapshot(&text).map(Some)
}

/// Parses the snapshot body. Public so the `wire` facade can hand the agent the exact reader the
/// tool itself uses, and so tests can drive it without a file.
pub fn parse_snapshot(text: &str) -> CoreResult<PlayerSnapshot> {
    let mut fields: Vec<(&str, &str)> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| snapshot_error("player_snapshot_line_invalid"))?;
        // A repeated key would make the result depend on which one won.
        if fields.iter().any(|(existing, _)| *existing == key) {
            return Err(snapshot_error("player_snapshot_duplicate_key"));
        }
        fields.push((key, value));
    }

    let take = |key: &str| -> CoreResult<&str> {
        fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
            .ok_or_else(|| snapshot_error("player_snapshot_key_missing"))
    };

    if parse_u32(take("v")?)? != SUPPORTED_VERSION {
        return Err(snapshot_error("player_snapshot_version_unsupported"));
    }

    let level = parse_u32(take("lv")?)?;
    if level > 255 {
        return Err(snapshot_error("player_snapshot_level_out_of_range"));
    }
    let xp_permille = parse_u32(take("xp")?)?;
    if xp_permille > 1000 {
        return Err(snapshot_error("player_snapshot_xp_out_of_range"));
    }
    let zone = parse_i64(take("zone")?)?;
    if !(-1..=127).contains(&zone) {
        return Err(snapshot_error("player_snapshot_zone_out_of_range"));
    }
    let map = parse_i64(take("map")?)?;
    if !(-1..=255).contains(&map) {
        return Err(snapshot_error("player_snapshot_map_out_of_range"));
    }
    let bag_used = parse_i64(take("bag")?)?;
    let xp_rate = parse_i64(take("xprate")?)?;
    let attack_phase = parse_i64(take("atkphase")?)?;
    if !(-1..=1).contains(&attack_phase) {
        return Err(snapshot_error("player_snapshot_attack_phase_out_of_range"));
    }
    let attack_state = parse_i64(take("atkstate")?)?;
    if !(-1..=2).contains(&attack_state) {
        return Err(snapshot_error("player_snapshot_attack_state_out_of_range"));
    }
    let control_state = parse_i64(take("ctl")?)?;
    if !(-1..=1).contains(&control_state) {
        return Err(snapshot_error("player_snapshot_control_state_out_of_range"));
    }
    let stuck = parse_u32(take("stuck")?)?;
    // Range-checked rather than trusted: the mod's state ids and the tool's must not drift apart
    // silently, and an unknown state rendered as a label is worse than a rejected snapshot.
    let travel_state = parse_u32(take("travel")?)?;
    if travel_state > 5 {
        return Err(snapshot_error("player_snapshot_travel_state_out_of_range"));
    }
    let travel_why = parse_u32(take("travelwhy")?)?;
    if travel_why > 5 {
        return Err(snapshot_error("player_snapshot_travel_why_out_of_range"));
    }
    let travel_goal = parse_i64(take("travelgoal")?)?;
    if !(-1..=255).contains(&travel_goal) {
        return Err(snapshot_error("player_snapshot_travel_goal_out_of_range"));
    }
    let travel_hops = parse_u32(take("travelhops")?)?;
    if travel_hops > 1_000 {
        return Err(snapshot_error("player_snapshot_travel_hops_out_of_range"));
    }
    // Range-checked rather than trusted, like the travel keys above: the mod's phase and reason
    // ids and the tool's must not drift apart silently, and an unknown one rendered as a label is
    // worse than a rejected snapshot.
    let enhance_phase = parse_u32(take("enhancephase")?)?;
    if enhance_phase > 7 {
        return Err(snapshot_error("player_snapshot_enhance_phase_out_of_range"));
    }
    let enhance_why = parse_u32(take("enhancewhy")?)?;
    if enhance_why > 4 {
        return Err(snapshot_error("player_snapshot_enhance_why_out_of_range"));
    }
    let enhance_done = parse_u32(take("enhancedone")?)?;
    if enhance_done > 9_999 {
        return Err(snapshot_error("player_snapshot_enhance_done_out_of_range"));
    }
    // ---- DUNGEON ----
    // Range-checked rather than trusted, like the travel keys above and the enhancement ones just
    // before these: a state or reason id the tool does not model is a mod newer than the tool, and
    // guessing would render a wrong label as fact.
    let dungeon_state = parse_u32(take("dungeonstate")?)?;
    if dungeon_state > 5 {
        return Err(snapshot_error("player_snapshot_dungeon_state_out_of_range"));
    }
    let dungeon_why = parse_u32(take("dungeonwhy")?)?;
    if dungeon_why > 4 {
        return Err(snapshot_error("player_snapshot_dungeon_why_out_of_range"));
    }
    let dungeon_runs = parse_u32(take("dungeonruns")?)?;
    if dungeon_runs > 1_000 {
        return Err(snapshot_error("player_snapshot_dungeon_runs_out_of_range"));
    }
    let dungeon_goal = parse_i32(take("dungeongoal")?)?;
    if !(-1..=255).contains(&dungeon_goal) {
        return Err(snapshot_error("player_snapshot_dungeon_goal_out_of_range"));
    }
    // ---- end DUNGEON ----
    // The three pickup bytes come back together: the client either has a record or has none, and a
    // half-present one would mean the reader and the mod disagree about the shape.
    let rank = parse_i32(take("pkrank")?)?;
    let mphp = parse_i32(take("pkmphp")?)?;
    let gold = parse_i32(take("pkgold")?)?;
    if !(-1..=5).contains(&rank) || !(-1..=3).contains(&mphp) || !(-1..=1).contains(&gold) {
        return Err(snapshot_error("player_snapshot_pickup_out_of_range"));
    }
    let pickup_record =
        (rank >= 0 || mphp >= 0 || gold >= 0).then_some((rank as i8, mphp as i8, gold as i8));
    let buffs_text = take("buffs")?;
    if buffs_text.len() != 3 {
        return Err(snapshot_error("player_snapshot_buffs_invalid"));
    }
    let mut buff_slots = [false; 3];
    for (slot, character) in buff_slots.iter_mut().zip(buffs_text.chars()) {
        *slot = match character {
            '0' => false,
            '1' => true,
            _ => return Err(snapshot_error("player_snapshot_buffs_invalid")),
        };
    }
    let materials_text = take("drops")?;
    if materials_text.len() != 6 {
        return Err(snapshot_error("player_snapshot_drops_invalid"));
    }
    let mut material_slots = [None; 6];
    for (slot, character) in material_slots.iter_mut().zip(materials_text.chars()) {
        *slot = match character {
            // A dash is the honest reading for a material the mod never got a confirmation about.
            '-' => None,
            '0' => Some(false),
            '1' => Some(true),
            _ => return Err(snapshot_error("player_snapshot_drops_invalid")),
        };
    }
    if stuck > 2 {
        return Err(snapshot_error("player_snapshot_stuck_out_of_range"));
    }

    let snapshot = PlayerSnapshot {
        written_at_unix_ms: parse_i64(take("t")?)?,
        character_name: parse_name(take("name")?)?,
        level: level as u16,
        xp_permille: xp_permille as u16,
        hp: parse_non_negative(take("hp")?)?,
        hp_max: parse_non_negative(take("hpmax")?)?,
        mp: parse_non_negative(take("mp")?)?,
        mp_max: parse_non_negative(take("mpmax")?)?,
        wallet_known: parse_flag(take("wallet")?)?,
        gold: parse_non_negative(take("gold")?)?,
        gem: parse_non_negative(take("gem")?)?,
        // The mod reports -1 when the map is not resolved yet.
        map_id: (map >= 0).then_some(map as u16),
        zone: zone as i16,
        pixel_x: parse_i32(take("px")?)?,
        pixel_y: parse_i32(take("py")?)?,
        quota: parse_i64(take("quota")?)?,
        bag_used: (bag_used >= 0).then_some(bag_used),
        bag_max: parse_non_negative(take("bagmax")?)?,
        state: parse_i32(take("state")?)?,
        // -1 is the client's own "not mounted" value.
        mount: {
            let mount = parse_i32(take("mount")?)?;
            (mount >= 0).then_some(mount)
        },
        mounts: parse_mounts(take("mounts")?)?,
        guild_name: {
            let guild = parse_name(take("guild")?)?;
            (!guild.is_empty()).then_some(guild)
        },
        xp_permille_per_hour: (xp_rate >= 0).then_some(xp_rate),
        stale: parse_flag(take("stale")?)?,
        attack_phase: attack_phase as i8,
        // -1 is the mod reporting that it owns nothing, which is different from phase 0.
        attack_state: (attack_state >= 0).then_some(attack_state as u8),
        has_target: parse_flag(take("target")?)?,
        stuck: stuck as u8,
        potions: parse_non_negative(take("potions")?)?,
        revives: parse_non_negative(take("revives")?)?,
        pickup: pickup_record,
        buffs: buff_slots,
        materials: material_slots,
        travel_state: travel_state as u8,
        travel_why: travel_why as u8,
        travel_goal: (travel_goal >= 0).then_some(travel_goal as u16),
        travel_hops: travel_hops as u16,
        settings_agreed: (control_state >= 0).then_some(control_state == 1),
        enhance_phase: enhance_phase as u8,
        enhance_why: enhance_why as u8,
        enhance_done: enhance_done as u16,
        // ---- DUNGEON ----
        dungeon_state: dungeon_state as u8,
        dungeon_why: dungeon_why as u8,
        dungeon_runs: dungeon_runs as u16,
        dungeon_goal: (dungeon_goal >= 0).then_some(dungeon_goal as u16),
        // ---- end DUNGEON ----
    };

    // Every key must be one this parser knows: an unrecognised key means the mod and the
    // tool disagree about the format, and guessing would render a wrong number as fact.
    const KNOWN_KEYS: &[&str] = &[
        "v",
        "t",
        "name",
        "lv",
        "xp",
        "hp",
        "hpmax",
        "mp",
        "mpmax",
        "wallet",
        "gold",
        "gem",
        "map",
        "zone",
        "px",
        "py",
        "quota",
        "bag",
        "bagmax",
        "state",
        "mount",
        "mounts",
        "guild",
        "xprate",
        "stale",
        "atkphase",
        "atkstate",
        "target",
        "stuck",
        "potions",
        "revives",
        "pkrank",
        "pkmphp",
        "pkgold",
        "buffs",
        "ctl",
        "drops",
        "travel",
        "travelwhy",
        "travelgoal",
        "travelhops",
        "enhancephase",
        "enhancewhy",
        "enhancedone",
        // ---- DUNGEON ----
        "dungeonstate",
        "dungeonwhy",
        "dungeonruns",
        "dungeongoal",
        // ---- end DUNGEON ----
    ];
    if let Some((unknown, _)) = fields.iter().find(|(key, _)| !KNOWN_KEYS.contains(key)) {
        let _ = unknown;
        return Err(snapshot_error("player_snapshot_unknown_key"));
    }

    Ok(snapshot)
}

fn parse_u32(value: &str) -> CoreResult<u32> {
    value
        .parse::<u32>()
        .map_err(|_| snapshot_error("player_snapshot_number_invalid"))
}

fn parse_i64(value: &str) -> CoreResult<i64> {
    value
        .parse::<i64>()
        .map_err(|_| snapshot_error("player_snapshot_number_invalid"))
}

fn parse_i32(value: &str) -> CoreResult<i32> {
    value
        .parse::<i32>()
        .map_err(|_| snapshot_error("player_snapshot_number_invalid"))
}

fn parse_non_negative(value: &str) -> CoreResult<i64> {
    let parsed = parse_i64(value)?;
    if parsed < 0 {
        return Err(snapshot_error("player_snapshot_negative"));
    }
    Ok(parsed)
}

fn parse_flag(value: &str) -> CoreResult<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(snapshot_error("player_snapshot_flag_invalid")),
    }
}

/// Accepts a display name, rejecting the delimiters the format reserves.
fn parse_name(value: &str) -> CoreResult<String> {
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(snapshot_error("player_snapshot_name_too_long"));
    }
    if value.contains(['\r', '\n', '=']) {
        return Err(snapshot_error("player_snapshot_name_invalid"));
    }
    Ok(value.to_owned())
}

/// Parses `mounts`: `id:name` pairs joined by `|`, empty when the bag holds none.
///
/// A malformed entry fails the whole snapshot rather than being skipped. The list drives which
/// mount the operator picks, and a silently shortened one would offer a choice that does not match
/// what is being carried.
fn parse_mounts(value: &str) -> CoreResult<Vec<(u16, String)>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut mounts = Vec::new();
    for entry in value.split('|') {
        let (id, name) = entry
            .split_once(':')
            .ok_or_else(|| snapshot_error("player_snapshot_mounts_invalid"))?;
        let id = id
            .parse::<u16>()
            .map_err(|_| snapshot_error("player_snapshot_mounts_invalid"))?;
        mounts.push((id, parse_name(name)?));
    }
    Ok(mounts)
}

pub fn clear_snapshot(microemu_home: &Path) -> CoreResult<()> {
    let path = snapshot_path(microemu_home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::io("remove player snapshot", error)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SNAPSHOT_BYTES, SNAPSHOT_FILE_NAME, clear_snapshot, parse_snapshot, read_snapshot,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    /// The exact body the mod wrote during a live run, with a synthetic character name.
    ///
    /// Kept verbatim so a format drift on either side fails here rather than in the UI.
    const LIVE_SNAPSHOT: &str = concat!(
        "v=6\nt=1788240611417\nname=TestChar\nlv=80\nxp=105\n",
        "hp=45991\nhpmax=45991\nmp=9406\nmpmax=9406\n",
        "wallet=1\ngold=44916\ngem=0\nmap=1\nzone=13\npx=504\npy=264\n",
        "quota=30000\nbag=37\nbagmax=42\nstate=0\nmount=-1\nmounts=62:Ngua trang|64:Tuan loc\nguild=\n",
        "xprate=0\nstale=0\n",
        "atkphase=1\nctl=1\natkstate=0\ntarget=1\nstuck=0\npotions=3\nrevives=1\n",
        "pkrank=1\npkmphp=0\npkgold=0\nbuffs=101\ndrops=1-0110\n",
        "travel=3\ntravelwhy=0\ntravelgoal=43\ntravelhops=2\n",
        "enhancephase=0\nenhancewhy=0\nenhancedone=0\n",
        "dungeonstate=0\ndungeonwhy=0\ndungeonruns=0\ndungeongoal=-1\n",
    );

    struct TestHome(PathBuf);

    impl TestHome {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!("zeus-player-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("a temporary home is creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, body: &str) {
            fs::write(self.0.join(SNAPSHOT_FILE_NAME), body).expect("the snapshot is writable");
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn the_live_snapshot_parses_into_the_values_the_game_displayed() {
        let snapshot = parse_snapshot(LIVE_SNAPSHOT).expect("the captured snapshot parses");
        assert_eq!(snapshot.character_name, "TestChar");
        assert_eq!(snapshot.level, 80);
        // 105 permille renders as "10,5%", which is what the client drew on screen.
        assert_eq!(snapshot.xp_permille, 105);
        assert_eq!(snapshot.hp, 45_991);
        assert_eq!(snapshot.hp_max, 45_991);
        assert_eq!(snapshot.hp_percent(), Some(100));
        assert!(snapshot.wallet_known);
        assert_eq!(snapshot.gold, 44_916);
        assert_eq!(snapshot.gem, 0);
        assert_eq!(snapshot.map_id, Some(1));
        assert_eq!(snapshot.zone, 13);
        assert_eq!(snapshot.quota, 30_000);
        assert!(!snapshot.quota_exhausted());
        assert_eq!(snapshot.bag_used, Some(37));
        assert_eq!(snapshot.bag_max, 42);
        assert!(!snapshot.is_dead());
        // -1 is the client's not-mounted value and must not surface as mount 0.
        assert_eq!(snapshot.mount, None);
        assert_eq!(snapshot.guild_name, None);
        assert_eq!(snapshot.xp_permille_per_hour, Some(0));
        assert!(!snapshot.stale);
        // The two modules report what they are doing, so a silent module is distinguishable
        // from a working one without opening the game.
        assert_eq!(snapshot.attack_phase, 1);
        assert_eq!(snapshot.attack_state, Some(0));
        assert!(snapshot.auto_running());
        assert!(snapshot.has_target);
        assert_eq!(snapshot.stuck, 0);
        assert_eq!(snapshot.revives, 1);
        // The pickup record is read back from the client, so the panel can show what took effect
        // rather than what was asked for.
        assert_eq!(snapshot.pickup, Some((1, 0, 0)));
        assert_eq!(snapshot.buffs, [true, false, true]);
        // Read back per material: a dash is a material the mod never got a confirmation about, and
        // it has to stay distinguishable from one confirmed open.
        assert_eq!(
            snapshot.materials,
            [
                Some(true),
                None,
                Some(false),
                Some(true),
                Some(true),
                Some(false)
            ]
        );
        assert_eq!(snapshot.potions, 3);
        // Travel's four keys, which together say where a route is and whether it is still moving.
        assert_eq!(snapshot.travel_state, 3);
        assert_eq!(snapshot.travel_why, 0);
        assert_eq!(snapshot.travel_goal, Some(43));
        assert_eq!(snapshot.travel_hops, 2);
        assert_eq!(snapshot.settings_agreed, Some(true));
    }

    #[test]
    fn travel_off_is_told_apart_from_a_route_that_stopped() {
        // -1 is the mod's own "no destination", and it must not read as map 0: one means travel is
        // off, the other would name a real map to walk to.
        let off = LIVE_SNAPSHOT
            .replace("travel=3", "travel=0")
            .replace("travelgoal=43", "travelgoal=-1");
        let snapshot = parse_snapshot(&off).expect("a travel-off snapshot parses");
        assert_eq!(snapshot.travel_state, 0);
        assert_eq!(snapshot.travel_goal, None);

        // A stop keeps its reason: without it the panel cannot tell a stall from a give-up.
        let stopped = LIVE_SNAPSHOT
            .replace("travel=3", "travel=5")
            .replace("travelwhy=0", "travelwhy=3");
        let snapshot = parse_snapshot(&stopped).expect("a stopped route parses");
        assert_eq!(snapshot.travel_state, 5);
        assert_eq!(snapshot.travel_why, 3);

        // Out-of-range values are refused rather than clamped: a byte this parser does not model is
        // a mod newer than the tool, and guessing would show the operator a wrong reason.
        for wire in [
            "travel=6",
            "travelwhy=6",
            "travelgoal=256",
            "travelhops=1001",
            "enhancephase=8",
            "enhancewhy=5",
            "enhancedone=10000",
            "dungeonstate=6",
            "dungeonwhy=5",
            "dungeonruns=1001",
            "dungeongoal=256",
        ] {
            let (key, _) = wire.split_once('=').expect("the probe is a key=value pair");
            let body = LIVE_SNAPSHOT.replace(
                &format!(
                    "{key}={}",
                    match key {
                        "travel" => "3",
                        "travelwhy" => "0",
                        "travelgoal" => "43",
                        "travelhops" => "2",
                        "enhancephase" | "enhancewhy" | "enhancedone" => "0",
                        "dungeonstate" | "dungeonwhy" | "dungeonruns" => "0",
                        "dungeongoal" => "-1",
                        _ => unreachable!("the probe list and this match are one list"),
                    }
                ),
                wire,
            );
            assert_ne!(body, LIVE_SNAPSHOT, "{wire} matched nothing");
            assert!(parse_snapshot(&body).is_err(), "{wire} was accepted");
        }
    }

    #[test]
    fn an_idle_module_is_told_apart_from_a_working_one() {
        // The client raises its own phase for a manual attack too, so the phase alone cannot say
        // whether auto is running: the mod's own state is what reports ownership.
        let body = LIVE_SNAPSHOT
            .replace("atkstate=0", "atkstate=-1")
            .replace("atkphase=1", "atkphase=-1")
            .replace("target=1", "target=0");
        let snapshot = parse_snapshot(&body).expect("an idle module parses");
        assert_eq!(snapshot.attack_state, None);
        assert!(!snapshot.auto_running());
        assert_eq!(snapshot.attack_phase, -1);
        assert!(!snapshot.has_target);

        // Both stuck diagnoses survive, because they call for different fixes.
        for (wire, expected) in [("stuck=1", 1u8), ("stuck=2", 2)] {
            let body = LIVE_SNAPSHOT.replace("stuck=0", wire);
            assert_eq!(
                parse_snapshot(&body).expect("a stuck reading parses").stuck,
                expected
            );
        }

        // Failing closed is silent, so the three settings states must stay distinguishable: never
        // configured, unreadable, and agreed.
        for (wire, expected) in [
            ("ctl=-1", None),
            ("ctl=0", Some(false)),
            ("ctl=1", Some(true)),
        ] {
            let body = LIVE_SNAPSHOT.replace("ctl=1", wire);
            assert_eq!(
                parse_snapshot(&body)
                    .expect("a settings state parses")
                    .settings_agreed,
                expected
            );
        }
    }

    #[test]
    fn a_zero_maximum_yields_no_percentage_instead_of_dividing_by_zero() {
        // hp_max and mp_max are genuinely 0 between entering the game and the stats packet.
        let body = LIVE_SNAPSHOT
            .replace("hpmax=45991", "hpmax=0")
            .replace("mpmax=9406", "mpmax=0")
            .replace("hp=45991", "hp=0")
            .replace("mp=9406", "mp=0");
        let snapshot = parse_snapshot(&body).expect("a zero maximum still parses");
        assert_eq!(snapshot.hp_percent(), None);
        assert_eq!(snapshot.mp_percent(), None);
    }

    #[test]
    fn an_undelivered_wallet_is_distinguishable_from_being_broke() {
        let body = LIVE_SNAPSHOT
            .replace("wallet=1", "wallet=0")
            .replace("gold=44916", "gold=0");
        let snapshot = parse_snapshot(&body).expect("an unknown wallet parses");
        // The caller must be able to render a dash: 0 gold and unknown gold are different.
        assert!(!snapshot.wallet_known);
        assert_eq!(snapshot.gold, 0);
    }

    #[test]
    fn an_unmeasurable_xp_rate_is_absent_rather_than_negative() {
        let body = LIVE_SNAPSHOT.replace("xprate=0", "xprate=-1");
        let snapshot = parse_snapshot(&body).expect("an unmeasured rate parses");
        assert_eq!(snapshot.xp_permille_per_hour, None);
    }

    #[test]
    fn a_stale_snapshot_is_flagged_so_it_is_not_treated_as_live() {
        let body = LIVE_SNAPSHOT.replace("stale=0", "stale=1");
        assert!(
            parse_snapshot(&body)
                .expect("a stale snapshot parses")
                .stale
        );
    }

    #[test]
    fn a_dead_character_and_an_exhausted_quota_are_both_reported() {
        // `\nstate=` and not `state=`: the latter also matches `atkstate=`, which is a different
        // key. The parser splits each line on its first `=`, so the keys themselves never collide.
        let body = LIVE_SNAPSHOT
            .replace("\nstate=0", "\nstate=4")
            .replace("quota=30000", "quota=0");
        let snapshot = parse_snapshot(&body).expect("a dead snapshot parses");
        assert!(snapshot.is_dead());
        // Quota at zero is exactly why auto-attack stops with no message.
        assert!(snapshot.quota_exhausted());
    }

    #[test]
    fn a_mounted_character_and_a_guild_survive_the_round_trip() {
        let body = LIVE_SNAPSHOT
            .replace("mount=-1", "mount=62")
            .replace("guild=\n", "guild=Zeus Clan\n");
        let snapshot = parse_snapshot(&body).expect("a mounted snapshot parses");
        assert_eq!(snapshot.mount, Some(62));
        assert_eq!(snapshot.guild_name.as_deref(), Some("Zeus Clan"));
    }

    #[test]
    fn every_malformed_snapshot_is_refused_rather_than_half_read() {
        let cases: &[(&str, String)] = &[
            // Both directions of drift, because both happen: a portable folder still holding the
            // previous jar writes the older version, and a newer jar dropped beside an older tool
            // writes a later one. Neither may be half-read.
            (
                "a jar older than this parser",
                LIVE_SNAPSHOT.replace("v=6", "v=5"),
            ),
            (
                "a jar newer than this parser",
                LIVE_SNAPSHOT.replace("v=6", "v=7"),
            ),
            ("missing key", LIVE_SNAPSHOT.replace("quota=30000\n", "")),
            ("unknown key", format!("{LIVE_SNAPSHOT}somethingelse=1\n")),
            ("duplicate key", format!("{LIVE_SNAPSHOT}lv=1\n")),
            ("no separator", LIVE_SNAPSHOT.replace("stale=0", "stale")),
            (
                "xp past permille",
                LIVE_SNAPSHOT.replace("xp=105", "xp=1001"),
            ),
            (
                "level past a byte",
                LIVE_SNAPSHOT.replace("lv=80", "lv=256"),
            ),
            (
                "zone past the table",
                LIVE_SNAPSHOT.replace("zone=13", "zone=128"),
            ),
            ("negative hp", LIVE_SNAPSHOT.replace("hp=45991", "hp=-1")),
            (
                "non-numeric level",
                LIVE_SNAPSHOT.replace("lv=80", "lv=eighty"),
            ),
            (
                "non-flag wallet",
                LIVE_SNAPSHOT.replace("wallet=1", "wallet=2"),
            ),
            (
                "name carrying a separator",
                LIVE_SNAPSHOT.replace("name=TestChar", "name=a=b"),
            ),
            (
                "name too long",
                LIVE_SNAPSHOT.replace("name=TestChar", &format!("name={}", "x".repeat(65))),
            ),
            (
                "phase past the client's own",
                LIVE_SNAPSHOT.replace("atkphase=1", "atkphase=2"),
            ),
            (
                "state past the last one",
                LIVE_SNAPSHOT.replace("atkstate=0", "atkstate=3"),
            ),
            (
                "stuck past the last diagnosis",
                LIVE_SNAPSHOT.replace("stuck=0", "stuck=3"),
            ),
            (
                "pick rank past the client's own list",
                LIVE_SNAPSHOT.replace("pkrank=1", "pkrank=6"),
            ),
            (
                "a buff string that is not one character per slot",
                LIVE_SNAPSHOT.replace("buffs=101", "buffs=10"),
            ),
            (
                "a buff character that is neither on nor off",
                LIVE_SNAPSHOT.replace("buffs=101", "buffs=1x1"),
            ),
            (
                "material states too short",
                LIVE_SNAPSHOT.replace("drops=1-0110", "drops=1-011"),
            ),
            (
                "a material character that is none of the three states",
                LIVE_SNAPSHOT.replace("drops=1-0110", "drops=1-011x"),
            ),
        ];
        for (label, body) in cases {
            assert!(
                parse_snapshot(body).is_err(),
                "accepted a malformed snapshot: {label}"
            );
        }
        // A truncated file, as a reader racing the writer would see, must also fail.
        let truncated = &LIVE_SNAPSHOT[..LIVE_SNAPSHOT.len() / 2];
        assert!(
            parse_snapshot(truncated).is_err(),
            "accepted a truncated snapshot"
        );
    }

    #[test]
    fn a_missing_snapshot_is_no_data_rather_than_a_failure() {
        let home = TestHome::new("absent");
        // Before login the mod has published nothing, which is ordinary, not an error.
        assert_eq!(
            read_snapshot(home.path()).expect("an absent snapshot is not an error"),
            None
        );
    }

    #[test]
    fn a_present_snapshot_is_read_cleared_and_then_absent_again() {
        let home = TestHome::new("roundtrip");
        home.write(LIVE_SNAPSHOT);
        let snapshot = read_snapshot(home.path())
            .expect("a written snapshot reads")
            .expect("a written snapshot is present");
        assert_eq!(snapshot.level, 80);

        clear_snapshot(home.path()).expect("clearing succeeds");
        assert_eq!(read_snapshot(home.path()).expect("cleared reads"), None);
        // Clearing twice is not an error, so stopping an account cannot fail on it.
        clear_snapshot(home.path()).expect("clearing is idempotent");
    }

    #[test]
    fn an_oversized_snapshot_is_refused_without_being_parsed() {
        let home = TestHome::new("oversized");
        let padding = "x".repeat(usize::try_from(MAX_SNAPSHOT_BYTES).expect("bound fits") + 1);
        home.write(&format!("{LIVE_SNAPSHOT}pad={padding}\n"));
        let error = read_snapshot(home.path()).expect_err("an oversized snapshot fails");
        assert_eq!(error.code(), "player_snapshot_too_large");
    }
}

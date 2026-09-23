//! Writes the settings the Zeus_Knight attack and item modules read.
//!
//! This is the reverse of [`crate::player`]: the same profile directory, the same
//! `key=value` shape, the same atomic replace, but the tool writes and the mod reads.
//! `docs/core/12-control-transport.md` records why a polled file rather than a socket,
//! and why not RMS: the record stores are only read while the client builds its login
//! screen, so a setting changed mid-session would never be seen.
//!
//! The mod fails closed. Anything it cannot parse — a missing key, an unknown key, a
//! value out of range — turns every module off rather than acting on a half-read file,
//! because a partial read could carry another account's spot or a nonsense threshold.
//! This module's job is therefore to never emit such a file: every value is clamped or
//! rejected here, before it reaches disk.
//!
//! Most of these settings are not the tool's invention. The client already owns auto-pickup,
//! the potion gate, the HP/MP thresholds and the buff slots in native fields that it syncs to
//! the server itself, and it has a menu for them. So the shapes below are the client's shapes —
//! an item rank *threshold* rather than five colour flags, a three-way potion filter, a two-way
//! gold flag — and the option names are the game's own strings from `df.java:820`. Inventing a
//! different shape here would mean two mechanisms disagreeing about one setting.

use std::fs;
use std::path::{Path, PathBuf};

use crate::data_root::atomic_replace;
use crate::error::{CoreError, CoreResult};

/// File name the tool writes inside `microemu-home`.
///
/// The launch specification passes this same path to the JVM as `-Dzeus.ctl.in`, so the
/// writer and the reader can never disagree about where the settings live.
pub const CONTROL_FILE_NAME: &str = "zeus-control.txt";

/// Format version the mod accepts. A different value turns every module off.
///
/// Bumped to 2 when the pickup settings moved to the client's own shape, and to 3 when zone
/// switching and the material close-drop arrived: a stale jar paired with a new tool then reports a
/// version it does not know instead of an unexplained unknown key. 5 added `nav.target`, 6 `ui.ring`
/// with `atk.farmOnArrival`, 8 dropped `atk.travel` for one destination, and 9 added
/// `nav.detectSpots`, 10 `atk.reviveDelay`, 11 moved reviving to its own `revive.*` keys with a
/// switch of its own, 12 did the same for the mount — `mount.on`/`mount.id`, with 0 meaning
/// "any mount in the bag", and 13 split the single `atk.potion` gate into `atk.hpOn` and
/// `atk.mpOn`, so HP and MP each get their own toggle.
/// The dungeon keys — `dungeon.on`, `dungeon.max` and `dungeon.schedule` — are written at 13 as
/// well. Their bump is deliberately deferred to a batch step, so the three ship under a version
/// the mod already accepts; a jar that has not learned them refuses the whole file and turns every
/// module off, which is the failure this constant exists to make legible.
pub const CONTROL_VERSION: u32 = 14;

/// Previous Control version supported for backward-compatible ingestion.
pub const CONTROL_VERSION_V13: u32 = 13;

/// Number of lines `to_wire()` emits, including the `v=` line.
pub const CTL_KEY_COUNT: usize = 37;

/// Canonical key count for legacy Control v13.
pub const CTL_KEY_COUNT_V13: usize = 35;

/// Key names for legacy Control v13 in canonical wire order.
pub const CTL_KEY_NAMES_V13: [&str; CTL_KEY_COUNT_V13] = [
    "v", "atk.mode", "atk.map", "atk.zone", "atk.x", "atk.y", "atk.radius",
    "atk.hpOn", "atk.hpPct", "atk.mpOn", "atk.mpPct", "revive.mode", "atk.buffs",
    "atk.zoneMode", "atk.zonePick", "item.rank", "item.mphp", "item.gold",
    "mount.on", "mount.id", "item.medalDialog", "item.dropsOn", "item.drops",
    "nav.target", "ui.ring", "atk.farmOnArrival", "nav.detectSpots",
    "revive.delay", "revive.on", "enhance.on", "enhance.maxLv", "enhance.charm",
    "dungeon.on", "dungeon.max", "dungeon.schedule",
];

/// Key names in the exact order `to_wire()` emits them. The jar's parser is order-insensitive
/// (it `take()`s by name), but the CI test asserts order so that a drift in either direction —
/// adding a key without bumping the count, or reordering without updating this array — is caught.
pub const CTL_KEY_NAMES: [&str; CTL_KEY_COUNT] = [
    "v", "atk.mode", "atk.map", "atk.zone", "atk.x", "atk.y", "atk.radius",
    "atk.hpOn", "atk.hpPct", "atk.mpOn", "atk.mpPct", "revive.mode", "atk.buffs",
    "atk.zoneMode", "atk.zonePick", "item.rank", "item.mphp", "item.gold",
    "mount.on", "mount.id", "item.medalDialog", "item.dropsOn", "item.drops",
    "nav.target", "ui.ring", "atk.farmOnArrival", "nav.detectSpots",
    "revive.delay", "revive.on", "enhance.on", "enhance.maxLv", "enhance.charm",
    "dungeon.on", "dungeon.max", "dungeon.schedule",
    "ui.effects", "ui.hidePlayers",
];

/// Buff slots the operator can address. The client's own count is `ah.b`.
pub const BUFF_SLOTS: usize = 3;

/// Material kinds whose drop the server can close, in the order the client's own menu lists them.
///
/// The order is the wire order, measured: a traced session toggled all six and the confirmation
/// dialog named each one, so index 0 is "mề đay trắng" and index 5 is "lửa tinh tú".
/// `docs/core/09-module-item.md` §12 records the packets and the table.
pub const MATERIAL_SLOTS: usize = 6;

/// Names of the six, for the operator. The client has no string table for these — the wording only
/// exists in the server's confirmation dialog — so these are this tool's own labels.
pub const MATERIAL_LABELS: [&str; MATERIAL_SLOTS] = [
    "Mề đay trắng",
    "Mề đay vàng",
    "Mề đay tím",
    "Mề đay xanh",
    "Nguyên liệu tinh tú",
    "Lửa tinh tú",
];

/// Target-scan radius the mod writes into `cn.g.bi` while auto is on.
///
/// The client's own default is 140, which scans 210 px. The narrower 120 keeps the character
/// on its spot; both are accepted so the operator can widen it back.
pub const MIN_RADIUS: u16 = 60;
pub const MAX_RADIUS: u16 = 240;

/// Mount template ids the client's own menu recognises (`fr.java:591-614`).
pub const MOUNT_TEMPLATE_IDS: [u16; 5] = [62, 63, 64, 65, 66];

/// `mount_template_id` value that means "whichever mount is in the bag".
///
/// The default, because it is the answer that works for every account: the mod rides what it finds
/// instead of holding out for one id the character may not carry.
pub const MOUNT_ANY: u16 = 0;

/// Longest revive delay the jar accepts, in seconds. Past five minutes on the ground the setting
/// is a mistake rather than a choice, and the jar refuses the whole file rather than clamping it.
pub const REVIVE_DELAY_MAX: u16 = 300;

/// The level an enhancement run stops at. The upper bound is the client's own: the enhancement
/// menu offers nothing past +15, so a larger target could never be reached and the run would only
/// keep spending charms.
pub const ENHANCE_LEVEL_MIN: u8 = 1;
pub const ENHANCE_LEVEL_MAX: u8 = 15;
/// Charm kinds the enhancement menu offers, as an index into its own list. 0 means "use none",
/// so the count is one past the last charm rather than the number of charms.
pub const ENHANCE_CHARM_MAX: u8 = 3;
// ---- DUNGEON ----
/// Most runs the dungeon loop may be asked to complete. -1 is unlimited, so this is a ceiling
/// rather than a pair of bounds: a larger target could never be reached and the loop would only
/// keep re-entering the dungeon.
pub const DUNGEON_RUNS_MAX: i8 = 10;
/// Half-hour slots in a day, which is the resolution the mod's own schedule uses. -1 is "no
/// timer", so slot 0 is 00:00 and the last is 23:30.
pub const DUNGEON_SCHEDULE_SLOTS: i8 = 48;
// ---- end DUNGEON ----

/// Which equipment the client's collector keeps, as a threshold rather than a set of flags.
///
/// The client skips a drop when `fa.ct < bq.q.a` (`bq.java:541`), so "blue" means blue and better.
/// Labels are the game's own, from `df.gL[0]`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemRank {
    #[default]
    All,
    BlueUp,
    YellowUp,
    PurpleUp,
    OrangeUp,
    /// Stored by the client as −1, not as index 5.
    None,
}

impl ItemRank {
    pub fn as_wire(self) -> u8 {
        match self {
            Self::All => 0,
            Self::BlueUp => 1,
            Self::YellowUp => 2,
            Self::PurpleUp => 3,
            Self::OrangeUp => 4,
            Self::None => 5,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::All,
            1 => Self::BlueUp,
            2 => Self::YellowUp,
            3 => Self::PurpleUp,
            4 => Self::OrangeUp,
            5 => Self::None,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "nhặt tất cả",
            Self::BlueUp => "nhặt từ đồ xanh",
            Self::YellowUp => "nhặt từ đồ vàng",
            Self::PurpleUp => "nhặt từ đồ tím",
            Self::OrangeUp => "nhặt từ đồ cam",
            Self::None => "không nhặt",
        }
    }

    /// Every option in the client's own order, for a picker.
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::BlueUp,
        Self::YellowUp,
        Self::PurpleUp,
        Self::OrangeUp,
        Self::None,
    ];
}

/// Which potions the client's collector keeps. Labels from `df.gL[1]`.
///
/// The client tells the two apart by `fa.ct` on the drop: 0 and 1 are the two potion kinds
/// (`bq.java:546-552`), which is why this is a three-way choice and not two flags.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PotionPickup {
    #[default]
    All,
    HpOnly,
    MpOnly,
    None,
}

impl PotionPickup {
    pub fn as_wire(self) -> u8 {
        match self {
            Self::All => 0,
            Self::HpOnly => 1,
            Self::MpOnly => 2,
            Self::None => 3,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::All,
            1 => Self::HpOnly,
            2 => Self::MpOnly,
            3 => Self::None,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "nhặt tất cả",
            Self::HpOnly => "chỉ nhặt HP",
            Self::MpOnly => "chỉ nhặt MP",
            Self::None => "không nhặt",
        }
    }

    pub const ALL: [Self; 4] = [Self::All, Self::HpOnly, Self::MpOnly, Self::None];
}

/// Whether the client's collector keeps gold. Labels from `df.gL[2]`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoldPickup {
    #[default]
    Pick,
    Skip,
}

impl GoldPickup {
    pub fn as_wire(self) -> u8 {
        match self {
            Self::Pick => 0,
            Self::Skip => 1,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Pick,
            1 => Self::Skip,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pick => "nhặt",
            Self::Skip => "không nhặt",
        }
    }

    pub const ALL: [Self; 2] = [Self::Pick, Self::Skip];
}

/// Which way to get back up. Whether to get up at all is [`ControlSettings::revive_on`].
///
/// Only the ticket puts the character back where it fell, which is the one path that lets auto
/// carry on without travelling. Town works but leaves the character off its spot, and walking
/// back is a later round; running out of tickets falls through to town rather than lying there.
///
/// There is deliberately no "off" variant. It used to be `Stay`, which meant the same state as
/// the switch being off and gave the tool and the jar two ways to disagree about it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviveMode {
    #[default]
    Ticket,
    Town,
}

impl ReviveMode {
    pub fn as_wire(self) -> u8 {
        match self {
            Self::Ticket => 1,
            Self::Town => 2,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Ticket,
            2 => Self::Town,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ticket => "dùng vé tại chỗ",
            Self::Town => "về làng",
        }
    }

    pub const ALL: [Self; 2] = [Self::Ticket, Self::Town];
}

/// How the attack module holds position.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttackMode {
    /// Auto is off. The mod touches no combat field.
    #[default]
    Off,
    /// Pin the character on its spot every tick, and drift no further than 30 px.
    ///
    /// Pinning makes the client's own movement deltas zero, which turns the periodic
    /// resync into the only way the server learns the position. That is a deliberate
    /// consequence, recorded in `docs/core/08-module-attack.md` section 4.1.
    Stand,
    /// Follow the target, returning to the spot only past 280 px.
    Move,
}

impl AttackMode {
    /// Wire value the mod parses.
    ///
    /// Matched exhaustively on purpose: adding a mode must fail to compile here rather than
    /// fall through a wildcard and reach the mod as "off" or, worse, as an active mode.
    pub fn as_wire(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Stand => 1,
            Self::Move => 2,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Off,
            1 => Self::Stand,
            2 => Self::Move,
            _ => return None,
        })
    }

    /// Next mode in the operator's single three-state control.
    ///
    /// One control rather than two toggles: the mod the client shipped left its
    /// move-and-attack action unreachable because two separate entries drifted apart.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Stand,
            Self::Stand => Self::Move,
            Self::Move => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Tắt",
            Self::Stand => "Đứng yên",
            Self::Move => "Di chuyển",
        }
    }
}

/// The monster spot: a map, a zone, and a pixel position inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackSpot {
    pub map_id: u16,
    pub zone: i16,
    pub pixel_x: i32,
    pub pixel_y: i32,
}

/// What to do about the zone the character is standing in.
///
/// Switching needs no travel: a traced probe sent the board interaction from 678 pixels away and the
/// server answered with the menu, so the mod talks to the board from wherever it stands
/// (`docs/core/12-control-transport.md` §10.1).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoneMode {
    /// Leave the zone alone.
    #[default]
    Keep,
    /// Read the player count out of the board's own menu and move to the emptiest zone.
    Emptiest,
    /// Move to the zone the operator named.
    Pick,
}

impl ZoneMode {
    pub fn as_wire(self) -> u8 {
        match self {
            Self::Keep => 0,
            Self::Emptiest => 1,
            Self::Pick => 2,
        }
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Keep,
            1 => Self::Emptiest,
            2 => Self::Pick,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "giữ khu hiện tại",
            Self::Emptiest => "tự chọn khu ít người",
            Self::Pick => "khu tự chọn",
        }
    }

    pub const ALL: [Self; 3] = [Self::Keep, Self::Emptiest, Self::Pick];
}

/// Zone numbers the operator may name. The board's menu on the traced map listed six.
pub const MIN_ZONE_PICK: u8 = 1;
pub const MAX_ZONE_PICK: u8 = 99;

/// One past the last routable map id. The mod's adjacency table is `int[136][]`, so this bounds what
/// a destination may be — a larger id would index outside it there.
pub const MAX_NAV_TARGET: i32 = 136;

/// Every setting the two modules read. Plain values: no path and no identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlSettings {
    pub mode: AttackMode,
    /// `None` until the operator captures a spot, which turns auto off at the mod.
    pub spot: Option<AttackSpot>,
    pub radius: u16,
    /// HP potion gate: the mod drinks HP potions when this is on and HP is below the threshold.
    pub hp_on: bool,
    /// HP percentage below which the mod drinks, 1..=99.
    pub hp_percent: u8,
    /// MP potion gate: the mod drinks MP potions when this is on and MP is below the threshold.
    pub mp_on: bool,
    pub mp_percent: u8,
    /// Whether the mod gets the character back up at all.
    ///
    /// Its own switch, independent of [`Self::mode`]: the revive module runs from the mod's tick
    /// whether or not a spot is armed, so a character can be left to revive without ever being set
    /// to fight. [`Self::revive`] only picks how.
    pub revive_on: bool,
    pub revive: ReviveMode,
    /// Seconds on the ground before the mod's first revive attempt, 0..=`REVIVE_DELAY_MAX`.
    ///
    /// Not a throttle: the mod already spaces its attempts. This is the operator's own pause, so a
    /// death can be looked at — or a fight nearby allowed to end — before the corpse is given up.
    /// 0 revives on the first tick dead, which is what every account had before this existed.
    pub revive_delay_seconds: u16,
    /// Buff 1, 2 and 3. A slot the character has not learned stays off whatever this says.
    pub buffs: [bool; BUFF_SLOTS],
    pub item_rank: ItemRank,
    pub potion_pickup: PotionPickup,
    pub gold: GoldPickup,
    /// Whether the mod rides a mount when the character is not already on one.
    pub mount: bool,
    /// [`MOUNT_ANY`] for any mount in the bag, or one id from [`MOUNT_TEMPLATE_IDS`].
    pub mount_template_id: u16,
    pub medal_dialog: bool,
    pub zone_mode: ZoneMode,
    /// Zone the operator named, used only by [`ZoneMode::Pick`].
    pub zone_pick: u8,
    /// Whether the tool drives the material close-drop at all.
    ///
    /// Off by default and deliberately a separate switch: the interaction is a *toggle*, not a set,
    /// so touching it always changes something. An operator who has not asked for it must not have
    /// their own choices flipped the first time they arm auto.
    pub materials_managed: bool,
    /// Desired state per material: true means the drop is closed.
    pub materials: [bool; MATERIAL_SLOTS],
    /// The map to walk to, or `None` for nowhere. The only destination there is.
    ///
    /// There were two, and that was the bug: `travel` walked to wherever the spot was saved while this
    /// walked to a named map, so with both set the walker arrived at one and was dragged toward the
    /// other — the operator watched it reach the right map and leave again. The mod switches this off
    /// on arrival rather than holding the character there; whether to fight is `farm_on_arrival`.
    pub nav_target: Option<u16>,
    /// Whether the mod draws the attack-range overlay in the game window.
    ///
    /// Purely local: it draws, it never sends. Every threshold this tool works in is a distance in
    /// world pixels with nothing on screen to judge it against, so drawn they are obvious and as
    /// numbers they are guesses. Off by default — it changes what the operator sees.
    pub ring: bool,
    /// Whether arriving at the spot arms the fight, or only parks the character there.
    ///
    /// Off by default on the operator's own instruction: reaching a spot must never start farming by
    /// itself. Walking somewhere and fighting there are two decisions, and this is the second.
    pub farm_on_arrival: bool,
    /// One-shot: ask the mod to report the monster clusters it can see.
    ///
    /// A request carried on the settings because that is the only channel to the mod. It answers into
    /// its trace — a scene reading is not a value this tool stores — and the tool writes it back false
    /// on the next save, so one press cannot fire twice.
    pub detect_spots: bool,
    /// Whether the mod drives the blacksmith's enhancement menu at all.
    ///
    /// Its own switch, like [`Self::revive_on`]: enhancing is a separate module in the mod's tick
    /// and shares no field with ATTACK, so a character can be left enhancing without ever being
    /// armed to fight. Off by default, because it spends charms and gold the operator has to have
    /// chosen to spend.
    pub enhance_on: bool,
    /// The level to enhance up to, `ENHANCE_LEVEL_MIN`..=`ENHANCE_LEVEL_MAX`.
    pub enhance_max_level: u8,
    /// Which charm the menu is driven to pick, 0..=`ENHANCE_CHARM_MAX`; 0 is "use none".
    pub enhance_charm_type: u8,
    // ---- DUNGEON ----
    /// Whether the mod runs the "Ngã tư tử thần" dungeon loop at all.
    ///
    /// Off by default, and unlike [`Self::revive_on`] this is not inert while ATTACK is disarmed:
    /// the module walks the character to the dungeon officer and talks to it on its own, so arming
    /// it moves a character the operator may have deliberately parked somewhere.
    pub dungeon_on: bool,
    /// Runs to complete before stopping, at most `DUNGEON_RUNS_MAX`. -1 keeps going without a
    /// limit.
    pub dungeon_max: i8,
    /// Half-hour slot of the day to start at, 0 being 00:00 and the last being 23:30. -1 means no
    /// timer, so the loop starts as soon as it is armed.
    pub dungeon_schedule: i8,
    // ---- end DUNGEON ----
    // ---- QOL --------------------------------------------------------------
    /// Visual effects rendering switch: 1 = enabled (client fa.ch = 0), 0 = disabled (fa.ch = 1).
    pub effects: u8,
    /// Player rendering mode: 0 = show all (cn.aN=false, cn.aO=false), 1 = hide other players (cn.aN=true, cn.aO=false), 2 = hide all players (cn.aN=false, cn.aO=true).
    pub hide_players: u8,
    // ---- end QOL ----------------------------------------------------------
}

impl Default for ControlSettings {
    fn default() -> Self {
        Self {
            mode: AttackMode::Off,
            spot: None,
            radius: 120,
            // The potion pumps are inert until auto is armed — the mod reaches them only after
            // `atk.mode == 0` has returned early — so `true` is safe and is what an operator arming
            // a spot wants: a run that drinks nothing is not a run.
            hp_on: true,
            // 50 is the client's own default, and the live run showed it drinking almost without
            // pause at a spot that out-damaged it. 35 leaves room to react without emptying a bag.
            hp_percent: 35,
            mp_on: true,
            mp_percent: 35,
            // Off, because reviving is no longer inert: the module runs whether or not auto is
            // armed, so a default of `true` would get a character up that the operator had left
            // lying there deliberately. It is one tick box away.
            revive_on: false,
            revive: ReviveMode::Ticket,
            // Immediate, which is what every account had before this setting existed.
            revive_delay_seconds: 0,
            buffs: [false; BUFF_SLOTS],
            item_rank: ItemRank::All,
            potion_pickup: PotionPickup::All,
            gold: GoldPickup::Pick,
            mount: false,
            // Any mount in the bag: the setting that works whatever the character happens to carry.
            mount_template_id: MOUNT_ANY,
            medal_dialog: true,
            zone_mode: ZoneMode::Keep,
            zone_pick: 1,
            materials_managed: false,
            materials: [false; MATERIAL_SLOTS],
            nav_target: None,
            ring: false,
            farm_on_arrival: false,
            detect_spots: false,
            enhance_on: false,
            // The client's own default target: far enough to be worth the charms, short of the
            // +15 ceiling where each further step costs more than the one before.
            enhance_max_level: 10,
            enhance_charm_type: 0,
            // ---- DUNGEON ----
            dungeon_on: false,
            dungeon_max: -1,
            dungeon_schedule: -1,
            // ---- end DUNGEON ----
            // ---- QOL --------------------------------------------------------------
            effects: 1,
            hide_players: 0,
            // ---- end QOL ----------------------------------------------------------
        }
    }
}

impl ControlSettings {
    /// Clamps every value into the range the mod accepts.
    ///
    /// Called before rendering rather than trusted from the caller: the mod turns every module
    /// off when a value is out of range, so an unclamped write would silently disable auto
    /// instead of reporting the bad value.
    pub fn clamped(mut self) -> Self {
        self.radius = self.radius.clamp(MIN_RADIUS, MAX_RADIUS);
        self.hp_percent = self.hp_percent.clamp(1, 99);
        self.mp_percent = self.mp_percent.clamp(1, 99);
        self.zone_pick = self.zone_pick.clamp(MIN_ZONE_PICK, MAX_ZONE_PICK);
        self.revive_delay_seconds = self.revive_delay_seconds.min(REVIVE_DELAY_MAX);
        self.enhance_max_level = self
            .enhance_max_level
            .clamp(ENHANCE_LEVEL_MIN, ENHANCE_LEVEL_MAX);
        self.enhance_charm_type = self.enhance_charm_type.min(ENHANCE_CHARM_MAX);
        // ---- DUNGEON ----
        self.dungeon_max = self.dungeon_max.clamp(-1, DUNGEON_RUNS_MAX);
        self.dungeon_schedule = self.dungeon_schedule.clamp(-1, DUNGEON_SCHEDULE_SLOTS - 1);
        // ---- end DUNGEON ----
        if self.mount_template_id != MOUNT_ANY
            && !MOUNT_TEMPLATE_IDS.contains(&self.mount_template_id)
        {
            self.mount_template_id = MOUNT_ANY;
        }
        // Auto with no spot has no anchor to hold, so the mode is not written as active.
        if self.spot.is_none() {
            self.mode = AttackMode::Off;
        }
        self.effects = self.effects.min(1);
        self.hide_players = self.hide_players.min(2);
        self
    }

    /// Renders the exact body the mod parses.
    pub fn to_wire(&self) -> String {
        let settings = self.clone().clamped();
        // An absent spot is written as the client's own not-known values rather than omitted:
        // a missing key turns every module off, which would hide the real reason.
        let spot = settings.spot.unwrap_or(AttackSpot {
            map_id: 0,
            zone: -1,
            pixel_x: -1,
            pixel_y: -1,
        });
        let mut buffs = String::with_capacity(BUFF_SLOTS);
        for enabled in settings.buffs {
            buffs.push(if enabled { '1' } else { '0' });
        }
        let mut materials = String::with_capacity(MATERIAL_SLOTS);
        for closed in settings.materials {
            materials.push(if closed { '1' } else { '0' });
        }
        let mut body = String::with_capacity(320);
        body.push_str(&format!("v={CONTROL_VERSION}\n"));
        body.push_str(&format!("atk.mode={}\n", settings.mode.as_wire()));
        body.push_str(&format!("atk.map={}\n", spot.map_id));
        body.push_str(&format!("atk.zone={}\n", spot.zone));
        body.push_str(&format!("atk.x={}\n", spot.pixel_x));
        body.push_str(&format!("atk.y={}\n", spot.pixel_y));
        body.push_str(&format!("atk.radius={}\n", settings.radius));
        body.push_str(&format!("atk.hpOn={}\n", flag(settings.hp_on)));
        body.push_str(&format!("atk.hpPct={}\n", settings.hp_percent));
        body.push_str(&format!("atk.mpOn={}\n", flag(settings.mp_on)));
        body.push_str(&format!("atk.mpPct={}\n", settings.mp_percent));
        body.push_str(&format!("revive.mode={}\n", settings.revive.as_wire()));
        body.push_str(&format!("atk.buffs={buffs}\n"));
        body.push_str(&format!("atk.zoneMode={}\n", settings.zone_mode.as_wire()));
        body.push_str(&format!("atk.zonePick={}\n", settings.zone_pick));
        body.push_str(&format!("item.rank={}\n", settings.item_rank.as_wire()));
        body.push_str(&format!("item.mphp={}\n", settings.potion_pickup.as_wire()));
        body.push_str(&format!("item.gold={}\n", settings.gold.as_wire()));
        body.push_str(&format!("mount.on={}\n", flag(settings.mount)));
        body.push_str(&format!("mount.id={}\n", settings.mount_template_id));
        body.push_str(&format!(
            "item.medalDialog={}\n",
            flag(settings.medal_dialog)
        ));
        body.push_str(&format!(
            "item.dropsOn={}\n",
            flag(settings.materials_managed)
        ));
        body.push_str(&format!("item.drops={materials}\n"));
        // -1 is off. The mod's own sentinel, so an absent destination is a value rather than a
        // missing key: a missing key turns every module off, which would hide the real reason.
        body.push_str(&format!(
            "nav.target={}\n",
            match settings.nav_target {
                Some(map) => i32::from(map),
                None => -1,
            }
        ));
        body.push_str(&format!("ui.ring={}\n", u8::from(settings.ring)));
        body.push_str(&format!(
            "atk.farmOnArrival={}\n",
            u8::from(settings.farm_on_arrival)
        ));
        body.push_str(&format!(
            "nav.detectSpots={}\n",
            u8::from(settings.detect_spots)
        ));
        body.push_str(&format!("revive.delay={}\n", settings.revive_delay_seconds));
        body.push_str(&format!("revive.on={}\n", flag(settings.revive_on)));
        body.push_str(&format!("enhance.on={}\n", flag(settings.enhance_on)));
        body.push_str(&format!("enhance.maxLv={}\n", settings.enhance_max_level));
        body.push_str(&format!("enhance.charm={}\n", settings.enhance_charm_type));
        // ---- DUNGEON ----
        body.push_str(&format!("dungeon.on={}\n", flag(settings.dungeon_on)));
        body.push_str(&format!("dungeon.max={}\n", settings.dungeon_max));
        body.push_str(&format!("dungeon.schedule={}\n", settings.dungeon_schedule));
        // ---- end DUNGEON ----
        // ---- QOL --------------------------------------------------------------
        body.push_str(&format!("ui.effects={}\n", settings.effects));
        body.push_str(&format!("ui.hidePlayers={}\n", settings.hide_players));
        // ---- end QOL ----------------------------------------------------------
        body
    }
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}

/// Path of the control file for one profile.
pub fn control_path(microemu_home: &Path) -> PathBuf {
    microemu_home.join(CONTROL_FILE_NAME)
}

/// Replaces one profile's control file.
///
/// Atomic: a uniquely named temporary beside the target, then a replace, so the mod can never
/// read a half-written body and turn everything off because of it.
pub fn write_settings(microemu_home: &Path, settings: &ControlSettings) -> CoreResult<()> {
    let path = control_path(microemu_home);
    let temporary = microemu_home.join(format!("{CONTROL_FILE_NAME}.tmp-{}", uuid::Uuid::new_v4()));
    let body = settings.to_wire();
    let result = fs::write(&temporary, body.as_bytes())
        .map_err(|error| CoreError::io("write control settings", error))
        .and_then(|()| atomic_replace(&temporary, &path));
    if result.is_err() {
        // A failed replace must not leave a stray temporary the managed-shape check would
        // later reject as an unknown descendant.
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Removes one profile's control file, so a stopped account carries no settings forward.
pub fn clear_settings(microemu_home: &Path) -> CoreResult<()> {
    let path = control_path(microemu_home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::io("remove control settings", error)),
    }
}

/// Largest control file this reader will look at. The real body is around 300 bytes.
pub const MAX_CONTROL_BYTES: u64 = 4096;

/// Reads back the settings written for one profile.
///
/// The file is the persistence: it lives in the profile directory, so settings survive a restart of
/// the tool without a second copy in the database that could disagree with what the mod reads.
///
/// `Ok(None)` means nothing has been configured yet. Anything present but malformed is an error,
/// because the tool has a UI to report it — unlike the mod, which can only fail closed.
pub fn read_settings(microemu_home: &Path) -> CoreResult<Option<ControlSettings>> {
    let path = control_path(microemu_home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CoreError::io("inspect control settings", error)),
    };
    if !metadata.is_file() || crate::data_root::metadata_is_link_or_reparse(&metadata) {
        return Err(control_error("control_settings_not_a_file"));
    }
    if metadata.len() > MAX_CONTROL_BYTES {
        return Err(control_error("control_settings_too_large"));
    }
    let bytes = fs::read(&path).map_err(|error| CoreError::io("read control settings", error))?;
    let text = String::from_utf8(bytes).map_err(|_| control_error("control_settings_not_utf8"))?;
    parse_settings(&text).map(Some)
}

fn control_error(code: &'static str) -> CoreError {
    CoreError::ControlSettings { code }
}

/// Parses a control body. Public so the `wire` facade can hand the agent the exact reader the
/// tool itself uses, and so tests can drive it without a file.
pub fn parse_settings(text: &str) -> CoreResult<ControlSettings> {
    let mut fields: Vec<(&str, &str)> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| control_error("control_settings_line_invalid"))?;
        if fields.iter().any(|(existing, _)| *existing == key) {
            return Err(control_error("control_settings_duplicate_key"));
        }
        fields.push((key, value));
    }
    let take = |key: &str| -> CoreResult<&str> {
        fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
            .ok_or_else(|| control_error("control_settings_key_missing"))
    };

    let version = number::<u32>(take("v")?)?;
    if version != CONTROL_VERSION && version != CONTROL_VERSION_V13 {
        return Err(control_error("control_settings_version_unsupported"));
    }
    let mode = AttackMode::from_wire(number(take("atk.mode")?)?)
        .ok_or_else(|| control_error("control_settings_mode_invalid"))?;
    let pixel_x = number::<i32>(take("atk.x")?)?;
    let pixel_y = number::<i32>(take("atk.y")?)?;
    let map = number::<i32>(take("atk.map")?)?;
    let zone = number::<i32>(take("atk.zone")?)?;
    if !(0..=255).contains(&map) || !(-1..=127).contains(&zone) {
        return Err(control_error("control_settings_spot_out_of_range"));
    }
    // The spot is absent exactly when its position is the client's own not-known value, which is
    // how `to_wire` writes it. Half a spot is a rejection rather than a guess.
    let spot = match (pixel_x >= 0, pixel_y >= 0) {
        (true, true) => Some(AttackSpot {
            map_id: map as u16,
            zone: zone as i16,
            pixel_x,
            pixel_y,
        }),
        (false, false) => None,
        _ => return Err(control_error("control_settings_spot_out_of_range")),
    };
    let buffs_text = take("atk.buffs")?;
    if buffs_text.len() != BUFF_SLOTS {
        return Err(control_error("control_settings_buffs_invalid"));
    }
    let mut buffs = [false; BUFF_SLOTS];
    for (slot, character) in buffs.iter_mut().zip(buffs_text.chars()) {
        *slot = match character {
            '0' => false,
            '1' => true,
            _ => return Err(control_error("control_settings_buffs_invalid")),
        };
    }
    let materials_text = take("item.drops")?;
    if materials_text.len() != MATERIAL_SLOTS {
        return Err(control_error("control_settings_materials_invalid"));
    }
    let mut materials = [false; MATERIAL_SLOTS];
    for (slot, character) in materials.iter_mut().zip(materials_text.chars()) {
        *slot = match character {
            '0' => false,
            '1' => true,
            _ => return Err(control_error("control_settings_materials_invalid")),
        };
    }
    let zone_pick = number::<u8>(take("atk.zonePick")?)?;
    if !(MIN_ZONE_PICK..=MAX_ZONE_PICK).contains(&zone_pick) {
        return Err(control_error("control_settings_zone_pick_out_of_range"));
    }
    let radius = number::<u16>(take("atk.radius")?)?;
    let hp_percent = number::<u8>(take("atk.hpPct")?)?;
    let mp_percent = number::<u8>(take("atk.mpPct")?)?;
    let mount_template_id = number::<u16>(take("mount.id")?)?;
    if !(MIN_RADIUS..=MAX_RADIUS).contains(&radius)
        || !(1..=99).contains(&hp_percent)
        || !(1..=99).contains(&mp_percent)
        || (mount_template_id != MOUNT_ANY && !MOUNT_TEMPLATE_IDS.contains(&mount_template_id))
    {
        return Err(control_error("control_settings_value_out_of_range"));
    }
    // -1 is off; every other value has to be a map the mod can route on. Its adjacency table is
    // bounded at 136, so a larger id would index outside it.
    let nav = number::<i32>(take("nav.target")?)?;
    if !(-1..MAX_NAV_TARGET).contains(&nav) {
        return Err(control_error("control_settings_nav_target_out_of_range"));
    }
    let nav_target = if nav < 0 { None } else { Some(nav as u16) };
    let ring = parse_flag(take("ui.ring")?)?;
    let farm_on_arrival = parse_flag(take("atk.farmOnArrival")?)?;
    let detect_spots = parse_flag(take("nav.detectSpots")?)?;
    let revive_delay_seconds = number::<u16>(take("revive.delay")?)?;
    let revive_on = parse_flag(take("revive.on")?)?;
    if revive_delay_seconds > REVIVE_DELAY_MAX {
        return Err(control_error("control_settings_revive_delay_out_of_range"));
    }
    let enhance_on = parse_flag(take("enhance.on")?)?;
    let enhance_max_level = number::<u8>(take("enhance.maxLv")?)?;
    let enhance_charm_type = number::<u8>(take("enhance.charm")?)?;
    // Bounded rather than clamped, like every other range here: the mod rejects the whole file
    // when a value is out of range, so a body this crate could not have written is an error to
    // report rather than a setting to quietly correct.
    if !(ENHANCE_LEVEL_MIN..=ENHANCE_LEVEL_MAX).contains(&enhance_max_level) {
        return Err(control_error("control_settings_enhance_level_out_of_range"));
    }
    if enhance_charm_type > ENHANCE_CHARM_MAX {
        return Err(control_error("control_settings_enhance_charm_out_of_range"));
    }
    // ---- DUNGEON ----
    let dungeon_on = parse_flag(take("dungeon.on")?)?;
    let dungeon_max = number::<i8>(take("dungeon.max")?)?;
    let dungeon_schedule = number::<i8>(take("dungeon.schedule")?)?;
    // Bounded rather than clamped, for the same reason as the enhancement above, and with the same
    // care around the sentinel: -1 is a setting the operator chose on both of these, so a value
    // outside the range is a body this crate could not have written.
    if !(-1..=DUNGEON_RUNS_MAX).contains(&dungeon_max) {
        return Err(control_error("control_settings_dungeon_max_out_of_range"));
    }
    if !(-1..=DUNGEON_SCHEDULE_SLOTS - 1).contains(&dungeon_schedule) {
        return Err(control_error(
            "control_settings_dungeon_schedule_out_of_range",
        ));
    }
    // ---- end DUNGEON ----
    let (effects, hide_players) = if version == CONTROL_VERSION_V13 {
        (1u8, 0u8)
    } else {
        let eff = number::<u8>(take("ui.effects")?)?;
        if eff > 1 {
            return Err(control_error("control_settings_effects_invalid"));
        }
        let hp = number::<u8>(take("ui.hidePlayers")?)?;
        if hp > 2 {
            return Err(control_error("control_settings_hide_players_invalid"));
        }
        (eff, hp)
    };

    let settings = ControlSettings {
        mode,
        spot,
        radius,
        hp_on: parse_flag(take("atk.hpOn")?)?,
        hp_percent,
        mp_on: parse_flag(take("atk.mpOn")?)?,
        mp_percent,
        revive_on,
        revive: ReviveMode::from_wire(number(take("revive.mode")?)?)
            .ok_or_else(|| control_error("control_settings_revive_invalid"))?,
        revive_delay_seconds,
        buffs,
        item_rank: ItemRank::from_wire(number(take("item.rank")?)?)
            .ok_or_else(|| control_error("control_settings_rank_invalid"))?,
        potion_pickup: PotionPickup::from_wire(number(take("item.mphp")?)?)
            .ok_or_else(|| control_error("control_settings_potion_pickup_invalid"))?,
        gold: GoldPickup::from_wire(number(take("item.gold")?)?)
            .ok_or_else(|| control_error("control_settings_gold_invalid"))?,
        mount: parse_flag(take("mount.on")?)?,
        mount_template_id,
        medal_dialog: parse_flag(take("item.medalDialog")?)?,
        zone_mode: ZoneMode::from_wire(number(take("atk.zoneMode")?)?)
            .ok_or_else(|| control_error("control_settings_zone_mode_invalid"))?,
        zone_pick,
        materials_managed: parse_flag(take("item.dropsOn")?)?,
        materials,
        nav_target,
        ring,
        farm_on_arrival,
        detect_spots,
        enhance_on,
        enhance_max_level,
        enhance_charm_type,
        // ---- DUNGEON ----
        dungeon_on,
        dungeon_max,
        dungeon_schedule,
        // ---- end DUNGEON ----
        // ---- QOL --------------------------------------------------------------
        effects,
        hide_players,
        // ---- end QOL ----------------------------------------------------------
    };

    // Every key must be one this parser knows for the respective version: an unrecognised key
    // means the tool and the mod disagree about the format, and the mod turns everything off.
    let known_keys: &[&str] = if version == CONTROL_VERSION_V13 {
        &CTL_KEY_NAMES_V13
    } else {
        &CTL_KEY_NAMES
    };
    if fields.iter().any(|(key, _)| !known_keys.contains(key)) {
        return Err(control_error("control_settings_unknown_key"));
    }
    if fields.len() != known_keys.len() {
        return Err(control_error("control_settings_key_missing"));
    }
    Ok(settings)
}

fn number<T: std::str::FromStr>(value: &str) -> CoreResult<T> {
    value
        .parse::<T>()
        .map_err(|_| control_error("control_settings_number_invalid"))
}

fn parse_flag(value: &str) -> CoreResult<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(control_error("control_settings_flag_invalid")),
    }
}

#[cfg(test)]
mod wire_shape {
    use super::*;

    fn spotted() -> ControlSettings {
        ControlSettings {
            mode: AttackMode::Stand,
            spot: Some(AttackSpot {
                map_id: 43,
                zone: 4,
                pixel_x: 228,
                pixel_y: 164,
            }),
            ..ControlSettings::default()
        }
    }

    #[test]
    fn control_body_is_the_exact_shape_the_mod_parses() {
        let body = spotted().to_wire();
        assert!(body.starts_with("v=14\n"));
        for line in [
            "atk.mode=1",
            "atk.map=43",
            "atk.zone=4",
            "atk.x=228",
            "atk.y=164",
            "atk.radius=120",
            "atk.hpOn=1",
            "atk.hpPct=35",
            "atk.mpOn=1",
            "atk.mpPct=35",
            "revive.mode=1",
            "atk.buffs=000",
            "atk.zoneMode=0",
            "atk.zonePick=1",
            "item.rank=0",
            "item.mphp=0",
            "item.gold=0",
            "mount.on=0",
            "mount.id=0",
            "item.medalDialog=1",
            "item.dropsOn=0",
            "item.drops=000000",
            "revive.delay=0",
            "revive.on=0",
            "enhance.on=0",
            "enhance.maxLv=10",
            "enhance.charm=0",
            // ---- DUNGEON ----
            "dungeon.on=0",
            "dungeon.max=-1",
            "dungeon.schedule=-1",
            // ---- end DUNGEON ----
            "ui.effects=1",
            "ui.hidePlayers=0",
        ] {
            assert!(body.contains(&format!("{line}\n")), "missing {line}");
        }
        for line in body.lines() {
            assert!(line.contains('='), "unparsable line {line}");
        }
        // 37 keys, asserted so an added key cannot ship without the mod learning it: an unknown
        // key turns every module off.
        assert_eq!(body.lines().count(), CTL_KEY_COUNT);
    }
}

#[cfg(test)]
mod settings {
    use super::*;

    fn spotted() -> ControlSettings {
        ControlSettings {
            mode: AttackMode::Stand,
            spot: Some(AttackSpot {
                map_id: 43,
                zone: 4,
                pixel_x: 228,
                pixel_y: 164,
            }),
            ..ControlSettings::default()
        }
    }

    #[test]
    fn the_pickup_settings_use_the_clients_own_shape_and_labels() {
        // Five independent colour flags is not how the client thinks: bq.java:541 skips a drop when
        // fa.ct < bq.q.a, so the setting is a threshold and blue means blue and better. Getting this
        // wrong means the tool and the game's own menu disagree about one setting.
        assert_eq!(ItemRank::ALL.len(), 6);
        assert_eq!(ItemRank::All.as_wire(), 0);
        assert_eq!(ItemRank::BlueUp.as_wire(), 1);
        assert_eq!(ItemRank::None.as_wire(), 5);
        assert_eq!(ItemRank::BlueUp.label(), "nhặt từ đồ xanh");
        assert_eq!(PotionPickup::HpOnly.label(), "chỉ nhặt HP");
        assert_eq!(GoldPickup::Skip.label(), "không nhặt");
        // Every option round-trips, so a picker index cannot land on a value the mod rejects.
        for rank in ItemRank::ALL {
            assert_eq!(ItemRank::from_wire(rank.as_wire()), Some(rank));
            assert!(!rank.label().is_empty());
        }
        for potion in PotionPickup::ALL {
            assert_eq!(PotionPickup::from_wire(potion.as_wire()), Some(potion));
            assert!(!potion.label().is_empty());
        }
        for gold in GoldPickup::ALL {
            assert_eq!(GoldPickup::from_wire(gold.as_wire()), Some(gold));
        }
        for revive in ReviveMode::ALL {
            assert_eq!(ReviveMode::from_wire(revive.as_wire()), Some(revive));
            assert!(!revive.label().is_empty());
        }
        // One past the last option is refused rather than wrapping to the first.
        assert_eq!(ItemRank::from_wire(6), None);
        assert_eq!(PotionPickup::from_wire(4), None);
        assert_eq!(GoldPickup::from_wire(2), None);
        assert_eq!(ReviveMode::from_wire(3), None);
        assert_eq!(AttackMode::from_wire(3), None);
    }

    #[test]
    fn the_default_threshold_is_lower_than_the_clients_own() {
        // A live run at the client's default of 50 drank 190 potions and still died, because the
        // spot out-damaged the drink rate. 35 leaves room to react without emptying a bag.
        let settings = ControlSettings::default();
        assert_eq!(settings.hp_percent, 35);
        assert_eq!(settings.mp_percent, 35);
        // Drinking and getting back up are on by default. Both are unreachable while the mode is
        // off, so this arms nothing on its own; it means a spot armed with one press behaves.
        assert!(settings.hp_on);
        assert!(settings.mp_on);
        assert_eq!(settings.revive, ReviveMode::Ticket);
        assert_eq!(settings.mode, AttackMode::Off);
        assert_eq!(settings.buffs, [false; BUFF_SLOTS]);
    }

    #[test]
    fn every_setting_survives_the_round_trip_the_tool_and_the_mod_share() {
        let written = ControlSettings {
            mode: AttackMode::Move,
            spot: Some(AttackSpot {
                map_id: 92,
                zone: 0,
                pixel_x: 1_200,
                pixel_y: 3_456,
            }),
            radius: 200,
            hp_on: true,
            mp_on: false,
            hp_percent: 65,
            mp_percent: 30,
            revive_on: true,
            revive: ReviveMode::Town,
            revive_delay_seconds: 12,
            buffs: [true, false, true],
            item_rank: ItemRank::PurpleUp,
            potion_pickup: PotionPickup::MpOnly,
            gold: GoldPickup::Skip,
            mount: true,
            mount_template_id: 66,
            medal_dialog: false,
            zone_mode: ZoneMode::Pick,
            zone_pick: 4,
            materials_managed: true,
            materials: [true, false, true, true, false, true],
            nav_target: Some(43),
            ring: true,
            farm_on_arrival: true,
            detect_spots: true,
            enhance_on: true,
            enhance_max_level: 12,
            enhance_charm_type: 2,
            // ---- DUNGEON ----
            dungeon_on: true,
            dungeon_max: 7,
            dungeon_schedule: 25,
            // ---- end DUNGEON ----
            effects: 0,
            hide_players: 2,
        };
        assert_eq!(
            parse_settings(&written.to_wire()).expect("the body this crate wrote parses"),
            written
        );
        let bare = ControlSettings::default();
        assert_eq!(
            parse_settings(&bare.to_wire()).expect("defaults parse"),
            bare
        );
        assert_eq!(parse_settings(&bare.to_wire()).unwrap().spot, None);
    }

    #[test]
    fn each_material_slot_keeps_its_own_position() {
        // The six are addressed by menu position, measured from a traced session: index 0 is
        // "mề đay trắng" and index 5 is "lửa tinh tú". A reversed or shifted string would close the
        // drop on a material the operator did not name, and the operator would have no way to tell.
        let body = ControlSettings {
            materials_managed: true,
            materials: [false, false, true, false, false, false],
            ..spotted()
        }
        .to_wire();
        assert!(body.contains("item.drops=001000\n"));
        assert!(body.contains("item.dropsOn=1\n"));
        assert_eq!(MATERIAL_LABELS.len(), MATERIAL_SLOTS);
        assert_eq!(MATERIAL_LABELS[0], "Mề đay trắng");
        assert_eq!(MATERIAL_LABELS[5], "Lửa tinh tú");
        for label in MATERIAL_LABELS {
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn the_zone_mode_options_round_trip_and_stop_at_the_last_one() {
        for mode in ZoneMode::ALL {
            assert_eq!(ZoneMode::from_wire(mode.as_wire()), Some(mode));
            assert!(!mode.label().is_empty());
        }
        assert_eq!(ZoneMode::from_wire(3), None);
        assert_eq!(ZoneMode::default(), ZoneMode::Keep);
        // Managing the material drop is off by default: the interaction is a toggle, so anything
        // else would flip the operator's own choices the first time they armed auto.
        assert!(!ControlSettings::default().materials_managed);
    }

    #[test]
    fn each_buff_slot_keeps_its_own_position() {
        let body = ControlSettings {
            buffs: [false, true, false],
            ..spotted()
        }
        .to_wire();
        // Buff 1, 2, 3 in order. A reversed string would silently cast the wrong one.
        assert!(body.contains("atk.buffs=010\n"));
    }

    #[test]
    fn auto_without_a_spot_is_written_as_off() {
        let body = ControlSettings {
            mode: AttackMode::Move,
            spot: None,
            // Travel asked for with no spot has no destination: the goal *is* the spot's map, so
            // leaving it on would send the character walking toward map 0.
            ..ControlSettings::default()
        }
        .to_wire();
        assert!(body.contains("atk.mode=0\n"));
        assert!(body.contains("atk.x=-1\n"));
        assert!(body.contains("atk.zone=-1\n"));
    }

    #[test]
    fn out_of_range_values_are_clamped_rather_than_written_and_rejected() {
        let clamped = ControlSettings {
            radius: 5_000,
            hp_percent: 0,
            mp_percent: 200,
            mount_template_id: 999,
            ..spotted()
        }
        .clamped();
        assert_eq!(clamped.radius, MAX_RADIUS);
        assert_eq!(clamped.hp_percent, 1);
        assert_eq!(clamped.mp_percent, 99);
        // Back to "any", not to a fixed id: the operator's mount is gone, and the setting that still
        // works is the one that rides whatever is carried.
        assert_eq!(clamped.mount_template_id, MOUNT_ANY);
        assert_eq!(
            ControlSettings {
                radius: 1,
                ..spotted()
            }
            .clamped()
            .radius,
            MIN_RADIUS
        );
    }

    #[test]
    fn a_reader_refuses_every_malformed_body_rather_than_half_reading_it() {
        let body = spotted().to_wire();
        let cases: &[(&str, String)] = &[
            ("unsupported version", body.replace("v=14", "v=12")),
            ("missing key", body.replace("atk.radius=120\n", "")),
            ("unknown key", format!("{body}nav.mode=1\n")),
            ("duplicate key", format!("{body}atk.mode=0\n")),
            ("no separator", body.replace("atk.hpOn=1", "atk.hpOn")),
            (
                "mode past the last one",
                body.replace("atk.mode=1", "atk.mode=3"),
            ),
            (
                "revive mode past the last one",
                body.replace("revive.mode=1", "revive.mode=3"),
            ),
            (
                "rank past the last one",
                body.replace("item.rank=0", "item.rank=6"),
            ),
            (
                "potion mode past the last one",
                body.replace("item.mphp=0", "item.mphp=4"),
            ),
            (
                "gold past the last one",
                body.replace("item.gold=0", "item.gold=2"),
            ),
            (
                "radius below the floor",
                body.replace("atk.radius=120", "atk.radius=10"),
            ),
            (
                "radius past the ceiling",
                body.replace("atk.radius=120", "atk.radius=900"),
            ),
            (
                "threshold at zero",
                body.replace("atk.hpPct=35", "atk.hpPct=0"),
            ),
            (
                "threshold at a hundred",
                body.replace("atk.mpPct=35", "atk.mpPct=100"),
            ),
            ("map past a byte", body.replace("atk.map=43", "atk.map=256")),
            (
                "zone past the table",
                body.replace("atk.zone=4", "atk.zone=128"),
            ),
            (
                "mount outside the set the client matches",
                body.replace("mount.id=0", "mount.id=70"),
            ),
            (
                "buffs too short",
                body.replace("atk.buffs=000", "atk.buffs=00"),
            ),
            (
                "buffs not a flag string",
                body.replace("atk.buffs=000", "atk.buffs=01x"),
            ),
            (
                "flag that is not a flag",
                body.replace("mount.on=0", "mount.on=2"),
            ),
            (
                "non-numeric threshold",
                body.replace("atk.hpPct=35", "atk.hpPct=fifty"),
            ),
            // Half a spot is a rejection: one axis known and the other not cannot be an anchor.
            ("half a spot", body.replace("atk.y=164", "atk.y=-1")),
            (
                "zone mode past the last one",
                body.replace("atk.zoneMode=0", "atk.zoneMode=3"),
            ),
            (
                "zone pick at zero",
                body.replace("atk.zonePick=1", "atk.zonePick=0"),
            ),
            (
                "zone pick past the ceiling",
                body.replace("atk.zonePick=1", "atk.zonePick=100"),
            ),
            (
                "materials too short",
                body.replace("item.drops=000000", "item.drops=00000"),
            ),
            (
                "materials not a flag string",
                body.replace("item.drops=000000", "item.drops=0000x0"),
            ),
            // -1 is off and 0..135 are maps; 136 is one past the mod's adjacency table.
            (
                "nav target past the routable ids",
                body.replace("nav.target=-1", "nav.target=136"),
            ),
            (
                "nav target below the off sentinel",
                body.replace("nav.target=-1", "nav.target=-2"),
            ),
            // ---- DUNGEON ----
            // -1 is a chosen setting on both, so it is the pair of bounds around each that has to
            // hold rather than a single ceiling.
            (
                "dungeon runs past the ceiling",
                body.replace("dungeon.max=-1", "dungeon.max=11"),
            ),
            (
                "dungeon runs below the unlimited sentinel",
                body.replace("dungeon.max=-1", "dungeon.max=-2"),
            ),
            (
                "dungeon slot past the last half hour",
                body.replace("dungeon.schedule=-1", "dungeon.schedule=48"),
            ),
            // ---- end DUNGEON ----
            // ---- QOL ----
            (
                "effects past the ceiling",
                body.replace("ui.effects=1", "ui.effects=2"),
            ),
            (
                "hide players past the ceiling",
                body.replace("ui.hidePlayers=0", "ui.hidePlayers=3"),
            ),
            ("missing effects", body.replace("ui.effects=1\n", "")),
            (
                "missing hide players",
                body.replace("ui.hidePlayers=0\n", ""),
            ),
            // ---- end QOL ----
        ];
        for (label, malformed) in cases {
            // A replace that matched nothing leaves a valid body, and the case would then be
            // asserting that valid input is refused. Changing a default has silently disarmed one of
            // these before, so the mutation itself is checked first.
            assert_ne!(malformed, &body, "case mutated nothing: {label}");
            assert!(
                parse_settings(malformed).is_err(),
                "accepted a malformed body: {label}"
            );
        }
        assert!(parse_settings(&body[..body.len() / 2]).is_err());
    }

    #[test]
    fn writing_reading_then_clearing_leaves_the_profile_as_it_was() {
        let home = std::env::temp_dir().join(format!("zeus-control-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&home).expect("a temporary home is creatable");
        let path = control_path(&home);

        assert_eq!(
            read_settings(&home).expect("an absent file is not an error"),
            None
        );
        write_settings(&home, &spotted()).expect("settings are writable");
        assert_eq!(
            read_settings(&home)
                .expect("written settings read")
                .expect("written settings are present"),
            spotted().clamped()
        );

        // A rewrite replaces rather than appends, so the mod never sees a duplicate key.
        write_settings(&home, &ControlSettings::default()).expect("settings are rewritable");
        let rewritten = fs::read_to_string(&path).expect("the control file is readable");
        assert!(rewritten.contains("atk.mode=0\n"));
        assert_eq!(rewritten.lines().count(), CTL_KEY_COUNT);

        let padding = "x".repeat(usize::try_from(MAX_CONTROL_BYTES).expect("bound fits") + 1);
        fs::write(&path, padding).expect("the control file is rewritable");
        assert_eq!(
            read_settings(&home)
                .expect_err("an oversized file fails")
                .code(),
            "control_settings_too_large"
        );

        clear_settings(&home).expect("clearing succeeds");
        assert!(!path.exists());
        clear_settings(&home).expect("clearing is idempotent");
        let leftovers: Vec<_> = fs::read_dir(&home)
            .expect("the home is listable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(leftovers.is_empty(), "left {leftovers:?} behind");
        let _ = fs::remove_dir_all(&home);
    }
}

// ── contract CI tests ────────────────────────────────────────────────────────────
//
// These exist because the 2026-09-12 incident proved that version alone is not enough:
// source and jar both said CTL_VERSION=13 but differed by 6 keys. The tests below catch
// both kinds of drift — within the Rust crate, and (via the shell companion) against the jar.

#[cfg(test)]
mod contract {
    use super::*;

    #[test]
    fn wire_contract_is_one_shape() {
        let body = ControlSettings::default().to_wire();
        let lines: Vec<&str> = body.lines().collect();

        // 1. line count matches the published constant
        assert_eq!(lines.len(), CTL_KEY_COUNT, "to_wire() changed line count");

        // 2. key names match the published array, in order
        let names: Vec<&str> = lines.iter().map(|l| l.split('=').next().unwrap()).collect();
        assert_eq!(names, CTL_KEY_NAMES, "key names/order diverged from CTL_KEY_NAMES");

        // 3. version line matches the published constant
        assert_eq!(lines[0], format!("v={CONTROL_VERSION}"));

        // 4. round-trip: parse what we just wrote
        assert_eq!(
            parse_settings(&body).unwrap(),
            ControlSettings::default().clamped(),
            "to_wire → parse_settings round-trip failed"
        );

        // 5. parse accepts its own output (guards against KNOWN_KEYS drifting from to_wire)
        assert!(parse_settings(&body).is_ok());
    }

    #[test]
    fn every_key_has_a_range_the_writer_clamps_to() {
        let wild = ControlSettings {
            radius: u16::MAX,
            hp_percent: 200,
            mp_percent: 0,
            zone_pick: 250,
            revive_delay_seconds: u16::MAX,
            enhance_max_level: 250,
            enhance_charm_type: 200,
            dungeon_max: 100,
            dungeon_schedule: 100,
            mount_template_id: 9999,
            ..ControlSettings::default()
        }
        .clamped();

        assert_eq!(wild.radius, MAX_RADIUS);
        assert_eq!(wild.hp_percent, 99);
        assert_eq!(wild.mp_percent, 1);
        assert_eq!(wild.zone_pick, MAX_ZONE_PICK);
        assert_eq!(wild.revive_delay_seconds, REVIVE_DELAY_MAX);
        assert_eq!(wild.enhance_max_level, ENHANCE_LEVEL_MAX);
        assert_eq!(wild.enhance_charm_type, ENHANCE_CHARM_MAX);
        assert_eq!(wild.dungeon_max, DUNGEON_RUNS_MAX);
        assert_eq!(wild.dungeon_schedule, DUNGEON_SCHEDULE_SLOTS - 1);
        assert_eq!(wild.mount_template_id, MOUNT_ANY);
    }

    #[test]
    fn v14_control_contract_and_v13_compatibility() {
        assert_eq!(CONTROL_VERSION, 14);
        assert_eq!(CTL_KEY_COUNT, 37);
        assert_eq!(CTL_KEY_NAMES.len(), 37);
        assert_eq!(CTL_KEY_NAMES[35], "ui.effects");
        assert_eq!(CTL_KEY_NAMES[36], "ui.hidePlayers");
        assert_eq!(CONTROL_VERSION_V13, 13);
        assert_eq!(CTL_KEY_COUNT_V13, 35);
        assert_eq!(CTL_KEY_NAMES_V13.len(), 35);
        assert!(!CTL_KEY_NAMES_V13.contains(&"ui.effects"));
        assert!(!CTL_KEY_NAMES_V13.contains(&"ui.hidePlayers"));

        // v13 body with neutral defaults
        let v13_body = ControlSettings::default()
            .to_wire()
            .replace("v=14\n", "v=13\n")
            .replace("ui.effects=1\n", "")
            .replace("ui.hidePlayers=0\n", "");
        assert_eq!(v13_body.lines().count(), 35);
        let parsed_v13 = parse_settings(&v13_body).expect("v13 body parses under compatibility");
        assert_eq!(parsed_v13.effects, 1);
        assert_eq!(parsed_v13.hide_players, 0);

        // Normalized v13 emits canonical v14 wire
        let v14_from_v13 = parsed_v13.to_wire();
        assert_eq!(v14_from_v13.lines().count(), 37);
        assert!(v14_from_v13.starts_with("v=14\n"));
        assert!(v14_from_v13.contains("ui.effects=1\n"));
        assert!(v14_from_v13.contains("ui.hidePlayers=0\n"));

        // v13 schema rejects v14 keys as unknown
        let malformed_v13 = format!("{v13_body}ui.effects=1\n");
        assert!(parse_settings(&malformed_v13).is_err());
        let malformed_v13_hp = format!("{v13_body}ui.hidePlayers=0\n");
        assert!(parse_settings(&malformed_v13_hp).is_err());

        // v14 round-trip preserves configured QoL values
        let mut custom = ControlSettings::default();
        custom.effects = 0;
        custom.hide_players = 2;
        let custom_wire = custom.to_wire();
        assert!(custom_wire.contains("ui.effects=0\n"));
        assert!(custom_wire.contains("ui.hidePlayers=2\n"));
        let parsed_custom = parse_settings(&custom_wire).expect("custom v14 round-trip");
        assert_eq!(parsed_custom.effects, 0);
        assert_eq!(parsed_custom.hide_players, 2);

        // invalid QoL values fail
        assert!(parse_settings(&custom_wire.replace("ui.effects=0", "ui.effects=2")).is_err());
        assert!(parse_settings(&custom_wire.replace("ui.effects=0", "ui.effects=-1")).is_err());
        assert!(parse_settings(&custom_wire.replace("ui.hidePlayers=2", "ui.hidePlayers=3")).is_err());
        assert!(parse_settings(&custom_wire.replace("ui.hidePlayers=2", "ui.hidePlayers=-1")).is_err());
    }
}

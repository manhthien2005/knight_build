//! Local, platform-independent account UI state.
//!
//! Everything here is pure: no Win32 call, no Core type, no worker handle. That keeps every status
//! transition, Vietnamese string, and stale-result rule testable on any platform.

use std::num::NonZeroU64;

/// Local row identity. The hidden account identity never reaches this layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowKey(NonZeroU64);

impl RowKey {
    pub fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Local request identity, paired with one submitted command.
pub type UiRequestId = u64;

/// Hard cap on events drained per timer tick, so one tick can never stall the message loop.
pub const MAX_EVENTS_PER_POLL: usize = 32;

/// Maximum accounts in one batch Run.
pub const MAX_BATCH_RUN: usize = 4;

/// Row status as the operator sees it.
///
/// `Starting`, `Authenticating`, and `Stopping` are UI-local: they are created from accepted commands
/// and never come from reconciliation, so a pending row cannot be mistaken for persisted truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAccountStatus {
    Idle,
    Starting,
    Authenticating,
    Running,
    Stopping,
    LoginFailed,
    CleanupPending,
}

impl UiAccountStatus {
    /// Bounded Vietnamese copy for this status.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Chưa chạy",
            Self::Starting => "Đang mở",
            Self::Authenticating => "Đang đăng nhập",
            Self::Running => "Đang chạy",
            Self::Stopping => "Đang dừng",
            Self::LoginFailed => "Login lỗi",
            Self::CleanupPending => "Cần dọn dẹp",
        }
    }

    /// Reports whether this row still holds work that blocks a clean exit.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Authenticating | Self::Running | Self::Stopping
        )
    }

    /// The single context action offered for this row.
    pub fn context_action(self) -> UiRowAction {
        match self {
            Self::Idle | Self::LoginFailed => UiRowAction::Run,
            Self::Starting | Self::Authenticating | Self::Running => UiRowAction::Stop,
            // A stopping row has no further action; cleanup is explicit operator work.
            Self::Stopping => UiRowAction::None,
            Self::CleanupPending => UiRowAction::RetryCleanup,
        }
    }
}

/// Action a row offers in its context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiRowAction {
    Run,
    Stop,
    RetryCleanup,
    None,
}

impl UiRowAction {
    #[allow(dead_code)]
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Run => Some("Chạy"),
            Self::Stop => Some("Dừng"),
            Self::RetryCleanup => Some("Dọn dẹp lại"),
            Self::None => None,
        }
    }
}

/// Where the character is standing, as the operator's live reading reports it.
///
/// Carried from the panel to the engine when auto is armed: the spot is whatever the character was
/// on at that moment, which is the only definition that matches "start here".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiSpot {
    pub map_id: u16,
    pub zone: i16,
    pub pixel_x: i32,
    pub pixel_y: i32,
}

/// How the operator wants the character to hold its position while fighting.
///
/// A single three-state control rather than two toggles: the mod the client shipped left its
/// move-and-attack action unreachable because two separate menu entries drifted apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiAutoMode {
    #[default]
    Off,
    /// Stay pinned on the spot and fight what comes into range.
    Stand,
    /// Follow the target, returning to the spot only when it drifts far.
    Move,
}

impl UiAutoMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Stand,
            Self::Stand => Self::Move,
            Self::Move => Self::Off,
        }
    }

    /// The mode one picker index means, or `None` when the index is not one.
    ///
    /// The picker's own order is the wire order, so an index maps straight through; this exists so the
    /// mapping lives in one place rather than at every read site.
    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Off),
            1 => Some(Self::Stand),
            2 => Some(Self::Move),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Tự đánh: tắt",
            Self::Stand => "Tự đánh: đứng yên",
            Self::Move => "Tự đánh: di chuyển",
        }
    }
}

/// Options the client's own menus offer, in the client's own order and words.
///
/// Copied into this layer rather than re-exported from the engine, so the model stays free of Core
/// types and the config dialog can be laid out and asserted without a running engine. The copy is not
/// left on trust: `port.rs` has a test that fails if any label or its position ever diverges from the
/// engine's table, which is itself taken from the game's `df.gL` strings.
///
/// The index of an option *is* its wire value, so a picker's selection needs no translation table.
pub const ITEM_RANK_OPTIONS: [&str; 6] = [
    "nhặt tất cả",
    "nhặt từ đồ xanh",
    "nhặt từ đồ vàng",
    "nhặt từ đồ tím",
    "nhặt từ đồ cam",
    "không nhặt",
];

/// MP and HP drops. The client tells the two apart itself, which is why this is one three-way choice.
pub const POTION_PICKUP_OPTIONS: [&str; 4] =
    ["nhặt tất cả", "chỉ nhặt HP", "chỉ nhặt MP", "không nhặt"];

pub const GOLD_OPTIONS: [&str; 2] = ["nhặt", "không nhặt"];

/// How to get back up. Whether to get up at all is its own tick box, so there is no "off" option
/// here: two ways to express the same state is two ways for the tool and the mod to disagree.
pub const REVIVE_OPTIONS: [&str; 2] = ["dùng vé tại chỗ", "về làng"];

/// Fallback label per mount template id, for a mount the account is not carrying.
///
/// The client has no id-to-name table to read — a mount's name arrives with the item itself — so
/// these are the names observed from live bags, and `Thú #<id>` where none has been seen yet. A live
/// reading always wins: [`UiPlayerInfo::mounts`] replaces the label for anything in the bag, so a
/// placeholder is what an id looks like only until an account carries it once.
///
/// The dropdown lists all five regardless, because picking one is a standing instruction: ride this
/// mount when it is to hand. A mount absent from the bag simply means nothing to ride yet.
pub const MOUNT_LABELS: [(u16, &str); 5] = [
    (62, "Thú #62"),
    (63, "Ngựa trắng"),
    (64, "Thú #64"),
    (65, "Ngựa xích thố"),
    (66, "Ngựa đen"),
];

/// The label for one mount id: the name the client reported, else the fallback.
pub fn mount_label(id: u16, live: &[(u16, String)]) -> String {
    live.iter()
        .find(|(carried, _)| *carried == id)
        .map(|(_, name)| name.clone())
        .or_else(|| {
            MOUNT_LABELS
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, name)| (*name).to_owned())
        })
        .unwrap_or_else(|| format!("Thú #{id}"))
}

/// How the character holds position while fighting.
pub const ATTACK_MODE_OPTIONS: [&str; 3] = ["Tắt", "Đứng yên", "Di chuyển"];

/// What to do about the zone. Switching needs no travel: the board answers from across the map.
pub const ZONE_MODE_OPTIONS: [&str; 3] =
    ["giữ khu hiện tại", "tự chọn khu ít người", "khu tự chọn"];

/// Zone numbers the operator may name.
pub const ZONE_PICK_MIN: u8 = 1;
pub const ZONE_PICK_MAX: u8 = 99;

/// Where the walker can be sent, as the operator picks it: a label and the id it means.
///
/// Two lists rather than one table because a picker's value is its index, and the index has to map
/// back to a map id the mod knows. Index 0 is off; the rest are every map the graph has an inbound
/// edge to, sorted by name. Map 135 is deliberately absent: it has outbound edges only, so no route
/// to it exists and offering it would be offering a destination that always fails.
///
/// The names are the client's own (`df.gE[]`, 92 of them) plus the eight ids it does not cover. The
/// id rides in the label because the client reuses "Đấu Trường" for both 36 and 46 — without it the
/// operator could not tell which one they were choosing.
/// Most spots one map may hold, matching the engine's own cap: the picker is sized for it.
pub const MAX_SPOTS_PER_MAP: i32 = 32;

pub const NAV_TARGET_OPTIONS: [&str; 76] = [
    "— không đi —",
    "Làng Sói Trắng (1)",
    "Khu mỏ (2)",
    "Bìa Rừng (3)",
    "Hang Lửa (4)",
    "Rừng Ảo Giác (5)",
    "Khe Vực (6)",
    "Cánh Đồng Sói (7)",
    "Thung Lũng Kỳ Bí (8)",
    "Hồ Ký Ức (9)",
    "Bãi Đất Trống (10)",
    "Bờ Biển (11)",
    "Vực Đá (12)",
    "Rặng Đá Ngầm (13)",
    "Nghĩa Địa Tàu Đắm (14)",
    "Đầm Lầy (15)",
    "Đền Cổ (16)",
    "Hang Dơi (17)",
    "Hang Sói Quỷ (18)",
    "Cửa Biển (19)",
    "Sa Mạc (20)",
    "Đồi Cát (21)",
    "Vực Lún (22)",
    "Hố Tử Thần (23)",
    "Nghĩa địa cát (24)",
    "Rừng Chết (25)",
    "Suối Ma (26)",
    "Thung Lũng Đá (27)",
    "Boss Guardian (28)",
    "Hầm Mộ Tầng 1 (29)",
    "Hầm Mộ Tầng 2 (30)",
    "Hầm Mộ Tầng 3 (31)",
    "Hầm mộ quái vật (32)",
    "Thành Phố Kho Báu (33)",
    "Khu phía Tây (34)",
    "Khu phía Đông (35)",
    "Đấu Trường (36)",
    "Rừng cao nguyên (37)",
    "Con đường hiểm trở (38)",
    "Vách đá cheo leo (39)",
    "Núi Cầu Vòng (40)",
    "Lối lên Thượng giới (41)",
    "Đèo hoang sơ (42)",
    "Đường xuống lòng đất (43)",
    "Cây cầu ma ám (44)",
    "Cổng vào Hạ giới (45)",
    "Đấu Trường (46)",
    "Đồi xác chết (47)",
    "Khu vườn (50)",
    "Cổng thiên đàng (51)",
    "Cổng địa ngục (52)",
    "Địa ngục tầng 1 (62)",
    "Rừng medusa (63)",
    "Rừng Chimera (64)",
    "Rừng quái vật (65)",
    "Thác reo (66)",
    "Thành phố cảng (67)",
    "Khu bờ nam (68)",
    "Khu bờ bắc (69)",
    "Khu bờ tây (70)",
    "Rừng chuột (71)",
    "Rừng hoa đỏ (72)",
    "Vịnh Caribe (73)",
    "Mê cung (74)",
    "Mê cung tầng 1 (75)",
    "Mê cung tầng 2 (76)",
    "Mê cung tầng 3 (77)",
    "Mê cung tầng 4 (78)",
    "Mê cung tầng cuối (79)",
    "Cổng trắng (92)",
    "Thị trấn mùa đông (93)",
    "Thung lũng băng giá (94)",
    "Chân núi tuyết (95)",
    "Đèo băng giá (96)",
    "Vực thẳm sương mù (97)",
    "Trạm núi tuyết (98)",
];

/// The map id each [`NAV_TARGET_OPTIONS`] entry means. Index 0 is off, so its id is unused.
pub const NAV_TARGET_IDS: [u16; 76] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    50, 51, 52, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 92, 93,
    94, 95, 96, 97, 98,
];

/// Materials whose drop the server can close, in the order the client's own menu lists them.
///
/// The order is the wire order and it was measured, not guessed: a traced session toggled all six
/// and the confirmation dialog named each one. The names themselves are not repeated here — the
/// config dialog's own row labels carry them, and `port.rs` ties those labels to the engine's table
/// so a renamed or reordered material cannot pass unnoticed.
pub const MATERIAL_SLOTS: usize = 6;

/// Scan-radius bounds the engine accepts. Outside these the settings are clamped, never refused.
pub const RADIUS_MIN: u16 = 60;
pub const RADIUS_MAX: u16 = 240;

/// Potion threshold bounds, as a percentage of the maximum.
pub const PERCENT_MIN: u8 = 1;
pub const PERCENT_MAX: u8 = 99;

/// Revive-delay bounds, in seconds. The engine's own ceiling, mirrored so the dialog can bound the
/// field rather than let a rejected value reach the mod.
pub const REVIVE_DELAY_MIN: u16 = 0;
pub const REVIVE_DELAY_MAX: u16 = zeus_core::REVIVE_DELAY_MAX;

/// Buff slots each character has.
pub const BUFF_SLOTS: usize = 3;

// ---- ENHANCE --------------------------------------------------------
/// Lowest and highest level an enhancement run may target. The ceiling is the client's own: its
/// enhancement menu offers nothing past +15.
pub const ENHANCE_LEVEL_MIN: u8 = zeus_core::ENHANCE_LEVEL_MIN;
pub const ENHANCE_LEVEL_MAX: u8 = zeus_core::ENHANCE_LEVEL_MAX;
/// The levels the picker offers, in order. Index 0 is `ENHANCE_LEVEL_MIN`, so unlike every other
/// picker here the selection is *not* its wire value: the setting stores the level itself, and the
/// dialog adds `ENHANCE_LEVEL_MIN` when it reads the picker and subtracts it when it opens one.
pub const ENHANCE_LEVEL_OPTIONS: [&str; 15] = [
    "+1", "+2", "+3", "+4", "+5", "+6", "+7", "+8", "+9", "+10", "+11", "+12", "+13", "+14", "+15",
];
/// The charms the enhancement menu offers, by index. 0 is "use none", so the index *is* the wire
/// value and needs no translation.
pub const ENHANCE_CHARM_OPTIONS: [&str; 4] = ["Không dùng", "Cỏ 3 lá", "Cỏ 4 lá", "Thông minh"];
// ---- end ENHANCE ----------------------------------------------------
// ---- DUNGEON --------------------------------------------------------
/// The run counts the picker offers, in order. Index 0 is "no limit", index `n` is `n` runs.
pub const DUNGEON_RUN_OPTIONS: [&str; 11] = [
    "Liên tục",
    "1 lượt",
    "2 lượt",
    "3 lượt",
    "4 lượt",
    "5 lượt",
    "6 lượt",
    "7 lượt",
    "8 lượt",
    "9 lượt",
    "10 lượt",
];
/// The start times the picker offers, in order. Index 0 is "no timer", index `k` is slot `k - 1`.
///
/// Formatted the way the mod's own reference module formats a slot — the hour zero-padded and the
/// minute either `00` or `30` — so the dialog and the in-game menu cannot disagree about what a slot
/// means. Slot 47 is `23:30`.
pub const DUNGEON_SCHEDULE_OPTIONS: [&str; 49] = [
    "Không hẹn",
    "00:00",
    "00:30",
    "01:00",
    "01:30",
    "02:00",
    "02:30",
    "03:00",
    "03:30",
    "04:00",
    "04:30",
    "05:00",
    "05:30",
    "06:00",
    "06:30",
    "07:00",
    "07:30",
    "08:00",
    "08:30",
    "09:00",
    "09:30",
    "10:00",
    "10:30",
    "11:00",
    "11:30",
    "12:00",
    "12:30",
    "13:00",
    "13:30",
    "14:00",
    "14:30",
    "15:00",
    "15:30",
    "16:00",
    "16:30",
    "17:00",
    "17:30",
    "18:00",
    "18:30",
    "19:00",
    "19:30",
    "20:00",
    "20:30",
    "21:00",
    "21:30",
    "22:00",
    "22:30",
    "23:00",
    "23:30",
];
/// The run count each picker index stands for, in order. Index 0 is the "keep going" sentinel.
///
/// A table rather than a subtraction, like [`NAV_TARGET_IDS`]: the sentinel sits at index 0 and every
/// other entry is its own index minus one, so the arithmetic that connects them would have to be
/// repeated at each of the two places that translate a selection, and one of them would eventually
/// get it wrong.
pub const DUNGEON_RUN_VALUES: [i8; 11] = [-1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
/// The schedule slot each picker index stands for, in order. Index 0 is the "no timer" sentinel.
pub const DUNGEON_SCHEDULE_VALUES: [i8; 49] = [
    -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
];
// ---- end DUNGEON ----------------------------------------------------

/// One account's automation settings, as the operator edits them.
///
/// A deliberate mirror of the engine's settings rather than a re-export, for the same reason
/// [`UiPlayerInfo`] is one. Every picker is stored as its option index, which is also its wire value,
/// so a selection cannot land on a value the engine would reject.
///
/// Most of these are not this tool's inventions: the client already owns the collector, the potion
/// gate and the buff slots in native fields it syncs to the server itself, and it has its own menu
/// for them. The tool drives those fields rather than reimplementing them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiControl {
    pub mode: UiAutoMode,
    /// Where to fight. `None` means "wherever the character is standing when auto is armed".
    pub spot: Option<UiSpot>,
    pub radius: u16,
    pub hp_on: bool,
    pub hp_percent: u8,
    pub mp_on: bool,
    pub mp_percent: u8,
    /// Whether the mod revives at all. Independent of [`Self::mode`]: reviving is its own module.
    pub revive_on: bool,
    /// Index into [`REVIVE_OPTIONS`].
    pub revive: u8,
    /// Seconds on the ground before the mod's first revive attempt. 0 revives at once.
    pub revive_delay_seconds: u16,
    pub buffs: [bool; BUFF_SLOTS],
    /// Index into [`ITEM_RANK_OPTIONS`].
    pub item_rank: u8,
    /// Index into [`POTION_PICKUP_OPTIONS`].
    pub potion_pickup: u8,
    /// Index into [`GOLD_OPTIONS`].
    pub gold: u8,
    /// Whether the mod rides a mount when the character is off one.
    pub mount: bool,
    /// [`zeus_core::MOUNT_ANY`] for any mount in the bag, or one template id.
    pub mount_template_id: u16,
    pub medal_dialog: bool,
    /// Index into [`ZONE_MODE_OPTIONS`].
    pub zone_mode: u8,
    /// Zone the operator named, used only by the "khu tự chọn" mode.
    pub zone_pick: u8,
    /// Whether the tool drives the material close-drop at all.
    ///
    /// Off by default, and a separate switch because the interaction is a *toggle*, not a set:
    /// touching it always changes something, so an operator who never asked must not have their own
    /// choices flipped the first time they arm auto.
    pub materials_managed: bool,
    /// Desired state per material: true means the drop is closed.
    pub materials: [bool; MATERIAL_SLOTS],
    /// Index into [`NAV_TARGET_OPTIONS`], which is also what the picker stores. 0 is off.
    ///
    /// An index rather than the map id: the operator picks a place by name, and asking them for an
    /// id they cannot see anywhere in the game is asking them to guess.
    pub nav_target: u8,
    /// Whether the mod draws the attack-range overlay in the game window.
    ///
    /// Off by default: it changes what the operator sees, and the thresholds it draws are the ones
    /// they cannot otherwise judge — `atk.radius`, and the leash the walker measures from the spot.
    pub ring: bool,
    /// Whether arriving at the chosen spot arms the fight, or only parks the character there.
    ///
    /// Derived from [`Self::mode`] rather than set on its own: choosing a mode says both that the
    /// character should fight and how, and ĐI MAP is the control that walks without fighting. Kept as
    /// its own wire key because the mod reads the two at different moments.
    pub farm_on_arrival: bool,
    /// Name of the spot to farm, empty when none is chosen.
    ///
    /// A name rather than an index into the picker: the list is rebuilt whenever the book changes, and
    /// an index would then point at whatever moved into that row.
    pub spot_name: String,
    /// One-shot: ask the mod to report the monster clusters it can see, then clear itself.
    ///
    /// A request rather than a mode, carried on the settings because that is the only channel to the
    /// mod. The mod clears it after answering, so it cannot fire twice from one press.
    pub detect_spots: bool,
    // ---- ENHANCE ----
    /// Whether the mod drives the blacksmith's enhancement menu at all.
    ///
    /// Its own switch, like [`Self::revive_on`]: the mod's enhancement module runs from its tick
    /// whether or not a spot is armed, so a character can be left enhancing without ever being set
    /// to fight.
    pub enhance_on: bool,
    /// The level to enhance up to, `ENHANCE_LEVEL_MIN`..=`ENHANCE_LEVEL_MAX`.
    ///
    /// A level rather than a picker index, because the picker's first option is +1: storing the
    /// index would make every reader remember an off-by-one that means nothing to the operator.
    pub enhance_max_level: u8,
    /// Index into [`ENHANCE_CHARM_OPTIONS`], which is also its wire value.
    pub enhance_charm: u8,
    // ---- end ENHANCE ----
    // ---- DUNGEON ----
    /// Whether the mod runs the "Ngã tư tử thần" loop at all.
    ///
    /// Its own switch, like [`Self::revive_on`]: the module runs from the mod's tick whether or not a
    /// spot is armed. It is not inert while ATTACK is disarmed either, the way reviving is — arming it
    /// walks the character to the dungeon officer and talks to it, so it moves a character the
    /// operator may have deliberately parked somewhere.
    pub dungeon_on: bool,
    /// Index into [`DUNGEON_RUN_OPTIONS`]. Not the run count: the first option is the "keep going"
    /// sentinel, so [`DUNGEON_RUN_VALUES`] translates it.
    pub dungeon_max: u8,
    /// Index into [`DUNGEON_SCHEDULE_OPTIONS`]. Not the slot: the first option is the "no timer"
    /// sentinel, so [`DUNGEON_SCHEDULE_VALUES`] translates it.
    pub dungeon_schedule: u8,
    // ---- end DUNGEON ----
}

impl Default for UiControl {
    /// Mirrors the engine's own defaults, field for field.
    ///
    /// This value is only a placeholder for a row the engine has not answered for yet, so any
    /// disagreement would show the operator one value and then silently replace it a tick later with
    /// another. `port.rs` has a test that fails if the two ever drift.
    fn default() -> Self {
        Self {
            mode: UiAutoMode::Off,
            spot: None,
            radius: 120,
            hp_on: true,
            hp_percent: 35,
            mp_on: true,
            mp_percent: 35,
            // Off, and the ticket path when it is turned on: the module now runs whether or not auto
            // is armed, so it must not get a character up that was left lying there deliberately.
            revive_on: false,
            revive: 0,
            revive_delay_seconds: 0,
            buffs: [false; BUFF_SLOTS],
            item_rank: 0,
            potion_pickup: 0,
            gold: 0,
            mount: false,
            // Any mount in the bag: the setting that works whatever the character carries.
            mount_template_id: zeus_core::MOUNT_ANY,
            medal_dialog: true,
            zone_mode: 0,
            zone_pick: 1,
            materials_managed: false,
            materials: [false; MATERIAL_SLOTS],
            nav_target: 0,
            ring: false,
            farm_on_arrival: false,
            spot_name: String::new(),
            detect_spots: false,
            // ---- ENHANCE ----
            enhance_on: false,
            enhance_max_level: 10,
            enhance_charm: 0,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_on: false,
            // Both pickers on their sentinel, which is "no limit" and "no timer": the loop does what
            // it says on the switch and stops only when the operator stops it.
            dungeon_max: 0,
            dungeon_schedule: 0,
            // ---- end DUNGEON ----
        }
    }
}

impl UiControl {
    /// Brings every value into the range the engine accepts.
    ///
    /// Applied before submitting rather than trusted: a picker index past the last option, or a
    /// threshold typed as 0, would otherwise reach the engine and be silently corrected there, so the
    /// dialog would show something different from what the client reads.
    /// The map the picker is pointing at, or `None` when it is off.
    ///
    /// The picker stores an index, not a map id, so every reader would otherwise repeat the table
    /// lookup and one of them would eventually forget that index 0 means "do not go anywhere".
    pub fn nav_map(&self) -> Option<u16> {
        if self.nav_target == 0 {
            return None;
        }
        NAV_TARGET_IDS.get(self.nav_target as usize).copied()
    }

    pub fn clamped(mut self) -> Self {
        self.radius = self.radius.clamp(RADIUS_MIN, RADIUS_MAX);
        self.hp_percent = self.hp_percent.clamp(PERCENT_MIN, PERCENT_MAX);
        self.mp_percent = self.mp_percent.clamp(PERCENT_MIN, PERCENT_MAX);
        self.revive = self.revive.min(REVIVE_OPTIONS.len() as u8 - 1);
        self.revive_delay_seconds = self.revive_delay_seconds.min(REVIVE_DELAY_MAX);
        self.item_rank = self.item_rank.min(ITEM_RANK_OPTIONS.len() as u8 - 1);
        self.potion_pickup = self
            .potion_pickup
            .min(POTION_PICKUP_OPTIONS.len() as u8 - 1);
        self.gold = self.gold.min(GOLD_OPTIONS.len() as u8 - 1);
        self.zone_mode = self.zone_mode.min(ZONE_MODE_OPTIONS.len() as u8 - 1);
        self.zone_pick = self.zone_pick.clamp(ZONE_PICK_MIN, ZONE_PICK_MAX);
        // A picked mount the operator no longer carries falls back to "any" rather than to a fixed
        // id: holding out for a mount that is not in the bag is silence, which is the failure this
        // module had.
        if self.mount_template_id != zeus_core::MOUNT_ANY
            && !zeus_core::MOUNT_TEMPLATE_IDS.contains(&self.mount_template_id)
        {
            self.mount_template_id = zeus_core::MOUNT_ANY;
        }
        self.nav_target = self.nav_target.min(NAV_TARGET_OPTIONS.len() as u8 - 1);
        // ---- ENHANCE ----
        self.enhance_max_level = self
            .enhance_max_level
            .clamp(ENHANCE_LEVEL_MIN, ENHANCE_LEVEL_MAX);
        self.enhance_charm = self
            .enhance_charm
            .min(ENHANCE_CHARM_OPTIONS.len() as u8 - 1);
        // ---- end ENHANCE ----
        // ---- DUNGEON ----
        self.dungeon_max = self.dungeon_max.min(DUNGEON_RUN_OPTIONS.len() as u8 - 1);
        self.dungeon_schedule = self
            .dungeon_schedule
            .min(DUNGEON_SCHEDULE_OPTIONS.len() as u8 - 1);
        // ---- end DUNGEON ----
        self
    }
}

/// One rendered row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiAccountRow {
    pub key: RowKey,
    pub revision: i64,
    pub username: String,
    pub status: UiAccountStatus,
    pub last_run_at_unix_ms: Option<i64>,
    /// World this account logs into, as an index into the client's server table.
    pub server_index: u8,
}

/// The client's own collector settings, in the client's own words.
///
/// Words rather than codes because the words are the client's, not this tool's: they come from
/// `df.gL`, the same table the in-game menu draws, so the panel and that menu can never disagree
/// about what "nhặt từ đồ xanh" means. The adapter resolves them once; nothing here re-derives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiPickup {
    pub item_rank: &'static str,
    pub potions: &'static str,
    pub gold: &'static str,
}

/// One published character reading, as the UI models it.
///
/// A deliberate copy of the engine's snapshot rather than a re-export: this module holds no Core type,
/// which is what keeps every rendering rule testable on any platform. Numbers stay numbers here and are
/// formatted exactly once, in the panel projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiPlayerInfo {
    pub character_name: String,
    pub level: u16,
    /// Progress through the current level in permille, 0..=1000. Not absolute experience: the client
    /// only ever knows the percentage.
    pub xp_permille: u16,
    /// Level progress per hour in permille, or `None` while it cannot be measured yet.
    pub xp_permille_per_hour: Option<i64>,
    pub hp: i64,
    pub hp_max: i64,
    pub mp: i64,
    pub mp_max: i64,
    /// While false, `gold` and `gem` are meaningless and must render as unknown, never as zero.
    pub wallet_known: bool,
    pub gold: i64,
    pub gem: i64,
    pub map_id: Option<u16>,
    pub zone: i16,
    pub pixel_x: i32,
    pub pixel_y: i32,
    /// Remaining attack quota. At zero or below the client silently stops auto-attacking.
    pub quota: i64,
    pub bag_used: Option<i64>,
    pub bag_max: i64,
    pub dead: bool,
    pub fighting: bool,
    pub mounted: bool,
    /// Mounts the bag holds, as `(template id, name)`, so the dialog can offer them by name.
    pub mounts: Vec<(u16, String)>,
    pub guild_name: Option<String>,
    /// True when the scene was not settled, so these are last-known rather than current values.
    pub stale: bool,
    /// How long ago the client wrote this reading, in whole seconds. `None` when it cannot be
    /// computed, which is treated as unknown rather than as fresh.
    pub age_seconds: Option<i64>,
    /// `None` when the mod owns no combat field, else 0 fighting, 1 returning, 2 settling.
    pub auto_state: Option<u8>,
    /// Whether a live monster is currently held as the target.
    pub has_target: bool,
    /// 0 fine, 1 no monsters in range, 2 monsters that cannot be reached or hit.
    pub stuck: u8,
    /// The client's own collector settings as they really are inside the running client, already
    /// in the client's own words. `None` means the collector is off.
    ///
    /// Read back rather than echoed from what the tool asked for: the point of showing it is to
    /// tell the operator what took effect, and a panel that renders the request cannot do that.
    pub pickup: Option<UiPickup>,
    /// Buff 1, 2 and 3 as the client will really cast them, after its own learned check.
    pub buffs: [bool; 3],
    /// Each material's close-drop state as the server confirmed it: `None` never confirmed.
    ///
    /// Read back rather than echoed from what was asked for: the interaction is a toggle, so
    /// "asked for" and "in force" are genuinely different facts.
    pub materials: [Option<bool>; MATERIAL_SLOTS],
    /// Where TRAVEL is: 0 off, 1 idle, 2 waiting on a teleport stone, 3 walking, 4 arrived,
    /// 5 stopped.
    pub travel_state: u8,
    /// Why TRAVEL stopped: 0 not stopped, 1 no route, 2 no usable exit or the walk stalled, 3 the
    /// stone refused the selection, 4 the hop cap, 5 an exception.
    ///
    /// Separate from `travel_state` because "stopped" with no reason is the failure this key exists
    /// to explain: a walk that never arrives and a walk that gave up two maps ago look identical
    /// from outside.
    pub travel_why: u8,
    /// Destination map, or `None` when travel is off.
    pub travel_goal: Option<u16>,
    /// Map borders crossed on the way, so a route that loops is visible rather than merely slow.
    pub travel_hops: u16,
    pub potions: i64,
    /// Times this session put the character back on its feet.
    pub revives: i64,
    /// Whether the tool and the mod agree about the settings. `None` when the client was launched
    /// without a settings path.
    pub settings_agreed: Option<bool>,
    // ---- ENHANCE ----
    /// Which step of an enhancement run the mod is on: 0 idle, 1 walking, 2 opening the NPC,
    /// 3 in its menu, 4 choosing the item, 5 choosing the charm, 6 confirming, 7 waiting.
    pub enhance_phase: u8,
    /// Why an enhancement run is not progressing: 0 fine, 1 no blacksmith, 2 out of charms,
    /// 3 the item is gone, 4 the target level is reached.
    pub enhance_why: u8,
    /// Items finished this session.
    pub enhance_done: u16,
    // ---- end ENHANCE ----
    // ---- DUNGEON ----
    /// Where the dungeon loop is: 0 off, 1 idle, 2 walking to the officer, 3 talking to it,
    /// 4 inside a run, 5 a run finished.
    pub dungeon_state: u8,
    /// Why the loop is not progressing: 0 fine, 1 no officer or the character is off the officer's
    /// map, 2 its menu would not open, 3 the walk stalled, 4 the run limit is reached.
    ///
    /// Separate from `dungeon_state` for the same reason as [`Self::travel_why`]: a loop that quietly
    /// gave up and one still walking look identical from outside, and "not moving" without a reason is
    /// the failure this field exists to prevent.
    pub dungeon_why: u8,
    /// Runs completed this session.
    pub dungeon_runs: u16,
    /// The dungeon's own map id while the loop is armed, or `None` when it is off. Mirrors
    /// [`Self::travel_goal`]: the mod publishes `-1` rather than omitting the key, so an un-armed loop
    /// is a value the panel can name rather than a reading it has to guess at.
    pub dungeon_goal: Option<u16>,
    // ---- end DUNGEON ----
}

impl UiPlayerInfo {
    /// HP as a percentage, or `None` before the maximum is known.
    ///
    /// `hp_max` is genuinely zero between entering the game and the stats packet arriving, so the
    /// ratio is guarded rather than assumed positive.
    pub fn hp_percent(&self) -> Option<u32> {
        percent(self.hp, self.hp_max)
    }

    pub fn mp_percent(&self) -> Option<u32> {
        percent(self.mp, self.mp_max)
    }

    /// Whether the attack quota is spent, which stops auto-attack with no message on screen.
    pub fn quota_exhausted(&self) -> bool {
        self.quota <= 0
    }
}

fn percent(value: i64, maximum: i64) -> Option<u32> {
    if maximum <= 0 {
        return None;
    }
    let clamped = value.clamp(0, maximum);
    u32::try_from(clamped.saturating_mul(100) / maximum).ok()
}

/// Stable failure classes the UI can render. Backend `Debug`/`Display` text is never shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiErrorCode {
    NotReady,
    QueueFull,
    Closing,
    Closed,
    InvalidInput,
    DuplicateAccount,
    AccountLimit,
    AccountNotFound,
    RevisionConflict,
    CredentialUnavailable,
    AccountBusy,
    LoginCapacity,
    LoginTarget,
    CleanupIncomplete,
    /// Auto was armed with no readable character, so there is no spot to anchor on.
    AutoNeedsCharacter,
}

impl UiErrorCode {
    /// Bounded Vietnamese copy for this failure class.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotReady => "Chưa sẵn sàng, vui lòng thử lại",
            Self::QueueFull => "Đang xử lý nhiều lệnh, vui lòng thử lại",
            Self::Closing => "Đang đóng ứng dụng",
            Self::Closed => "Ứng dụng đã đóng",
            Self::InvalidInput => "Dữ liệu không hợp lệ",
            Self::DuplicateAccount => "Tên đăng nhập đã tồn tại",
            Self::AccountLimit => "Đã đạt giới hạn 100 tài khoản",
            Self::AccountNotFound => "Không tìm thấy tài khoản",
            Self::RevisionConflict => "Tài khoản vừa thay đổi, vui lòng tải lại",
            Self::CredentialUnavailable => "Không mở được kho mật khẩu",
            Self::AccountBusy => "Tài khoản đang chạy",
            Self::LoginCapacity => "Chỉ chạy được 4 tài khoản cùng lúc",
            Self::LoginTarget => "Không tìm thấy cửa sổ game hợp lệ",
            Self::CleanupIncomplete => "Dọn dẹp chưa hoàn tất",
            Self::AutoNeedsCharacter => "Chưa vào game, chưa chọn được bãi",
        }
    }
}

/// Result of submitting one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitResult {
    Accepted(UiRequestId),
    Rejected(UiErrorCode),
}

/// Per-row outcome inside one batch Run result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiBatchRunItem {
    pub row: RowKey,
    pub scheduled: Result<(), UiErrorCode>,
}

/// Why the portable boot failed, in the only two classes the operator can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiBootFailure {
    /// The data root could not be opened or repaired.
    DataRoot,
    /// The pinned runtime could not be verified or relocated.
    PinnedRuntime,
}

/// One saved place to stand, with the name the operator gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSavedSpot {
    /// What the operator calls it. Unique within its map.
    pub name: String,
    /// Where it is.
    pub spot: UiSpot,
}

/// Every saved monster spot, in the order the engine returned them.
///
/// Shared by every account, so it carries no account handle: a monster spot belongs to the world, and
/// two accounts farming the same map want the same coordinates rather than two copies that can drift
/// apart. Several per map, each named — one per map was the first shape, and it made a map's second
/// worthwhile place to stand unreachable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiSpotBook {
    pub entries: Vec<UiSavedSpot>,
}

impl UiSpotBook {
    /// Every spot saved for one map, in book order.
    ///
    /// Used by the port tests to state the invariant the dialog relies on: a map's spots are exactly
    /// those filed under it, so filtering by the chosen map can never surface another map's.
    pub fn for_map(&self, map_id: u16) -> Vec<&UiSavedSpot> {
        self.entries
            .iter()
            .filter(|saved| saved.spot.map_id == map_id)
            .collect()
    }

    /// The name of the spot saved at these exact coordinates, or `None`.
    ///
    /// The reverse lookup, for reopening the dialog: the engine stores where a spot is, and the book
    /// stores what it is called, so the label is recovered rather than duplicated onto the wire.
    pub fn name_of(&self, spot: UiSpot) -> Option<&str> {
        self.entries
            .iter()
            .find(|saved| {
                saved.spot.map_id == spot.map_id
                    && saved.spot.pixel_x == spot.pixel_x
                    && saved.spot.pixel_y == spot.pixel_y
            })
            .map(|saved| saved.name.as_str())
    }

    /// The spot with this name on this map, or `None`.
    ///
    /// Name and map together are the identity: two maps may both hold a "Bãi trên", and they are
    /// different places.
    pub fn find(&self, map_id: u16, name: &str) -> Option<&UiSavedSpot> {
        self.entries
            .iter()
            .find(|saved| saved.spot.map_id == map_id && saved.name == name)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiSuccess {
    Accounts(Vec<UiAccountRow>),
    Account(UiAccountRow),
    Deleted(RowKey),
    RunBatch(Vec<UiBatchRunItem>),
    /// One row's published character reading. `None` means the client has not published yet, which is
    /// ordinary before a character is entered and must render as "no data", not as an error.
    Player {
        row: RowKey,
        info: Option<UiPlayerInfo>,
    },
    /// One row's automation settings as the engine wrote them, clamped.
    ///
    /// The reply to both writing and reading them, because it is the same fact either way: what the
    /// client will actually read. The dialog opens on this rather than on what was last typed, so a
    /// value the engine corrected is visible instead of silently disagreeing.
    Control {
        row: RowKey,
        settings: UiControl,
    },
    Shutdown,
    /// Every saved monster spot, keyed by map, after reading, saving or clearing one.
    ///
    /// One variant for all three because the reply is the whole book either way: a caller that just
    /// saved wants to see the result, not reconcile a delta against what it hoped it wrote.
    Spots(UiSpotBook),
}

/// One event delivered to the UI.
///
/// `RequestFinished` is much the largest variant, because a character reading is two dozen fields.
/// Boxing it would trade a move for an allocation and a free on the once-a-second poll path, for an
/// event that is drained, matched once, and dropped — so the size difference is deliberate.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiEvent {
    RequestFinished {
        request_id: UiRequestId,
        result: Result<UiSuccess, UiErrorCode>,
    },
    /// Terminal asynchronous reconciliation for one row.
    StateChanged(UiAccountRow),
    /// The worker finished portable boot and accepts commands.
    WorkerReady,
    /// Portable boot failed, so no window may be shown.
    BootFailed(UiBootFailure),
    WorkerClosed,
}

/// Which command a pending request represents, so its result can be applied correctly.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingKind {
    List,
    Import,
    Update(RowKey),
    Delete(RowKey),
    Run(Vec<RowKey>),
    Stop(RowKey),
    RetryCleanup(RowKey),
    ObservePlayer(RowKey),
    SetAuto,
    /// A settings read submitted to fill the config dialog. Kept apart from a write because a failed
    /// read is not operator-actionable — the dialog simply opens on the defaults.
    ObserveControl,
    /// A read or write of the shared spot book. Carries no row: the book belongs to the world.
    Spots,
    Shutdown,
}

/// Pure account table with UI-local pending state.
#[derive(Debug, Default)]
pub struct AccountTableModel {
    rows: Vec<UiAccountRow>,
    pending: Vec<(UiRequestId, PendingKind)>,
    last_error: Option<UiErrorCode>,
    worker_closed: bool,
    worker_ready: bool,
    boot_failure: Option<UiBootFailure>,
    /// Row the character panel is showing, which is the highlighted row rather than a checked one.
    player_row: Option<RowKey>,
    /// Most recent reading for [`Self::player_row`], or `None` when nothing has been published.
    player: Option<UiPlayerInfo>,
    /// Attack mode the operator last asked for, per row.
    ///
    /// UI-local, like the pending statuses: the engine answers with the settings it wrote, and the
    /// mod's own reported state is what the panel renders, so this only drives the next press.
    auto_modes: Vec<(RowKey, UiAutoMode)>,
    /// Settings the engine last confirmed for each row, so the config dialog opens on the truth.
    ///
    /// Seeded by every `Control` reply, from a read as well as a write. A row missing here has never
    /// answered, and the dialog opens on the defaults rather than on another row's values.
    controls: Vec<(RowKey, UiControl)>,
    /// Every saved monster spot, keyed by map, as the engine last returned it.
    ///
    /// Not per row: the book is shared by every account, because a spot belongs to the world. Empty
    /// until the first read answers, which is also the state a fresh installation is really in.
    spots: UiSpotBook,
    /// Set when a Run, Stop or cleanup retry was refused, so the caller re-reads the account list.
    ///
    /// A refusal means the row this model marked optimistically is not the row the engine holds, and
    /// the engine sends no notification for a command it never ran.
    lifecycle_failed: bool,
}

impl AccountTableModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every saved monster spot, keyed by map.
    ///
    /// Shared by every account: a spot belongs to the world, not to a login.
    pub fn spots(&self) -> &UiSpotBook {
        &self.spots
    }

    pub fn rows(&self) -> &[UiAccountRow] {
        &self.rows
    }

    pub fn last_error(&self) -> Option<UiErrorCode> {
        self.last_error
    }

    pub fn worker_closed(&self) -> bool {
        self.worker_closed
    }

    /// Whether the worker finished portable boot and accepts commands.
    pub fn worker_ready(&self) -> bool {
        self.worker_ready
    }

    /// The boot failure class, or `None` while boot is still pending or succeeded.
    pub fn boot_failure(&self) -> Option<UiBootFailure> {
        self.boot_failure
    }

    pub fn row(&self, key: RowKey) -> Option<&UiAccountRow> {
        self.rows.iter().find(|row| row.key == key)
    }

    /// Row the character panel is showing, if any.
    pub fn player_row(&self) -> Option<RowKey> {
        self.player_row
    }

    /// Most recent character reading for the panel's row.
    pub fn player(&self) -> Option<&UiPlayerInfo> {
        self.player.as_ref()
    }

    /// Attack mode last asked for on one row.
    pub fn auto_mode(&self, key: RowKey) -> UiAutoMode {
        self.auto_modes
            .iter()
            .find(|(row, _)| *row == key)
            .map(|(_, mode)| *mode)
            .unwrap_or_default()
    }

    /// Records the mode one row was just switched to.
    pub fn set_auto_mode(&mut self, key: RowKey, mode: UiAutoMode) {
        match self.auto_modes.iter_mut().find(|(row, _)| *row == key) {
            Some(entry) => entry.1 = mode,
            None => self.auto_modes.push((key, mode)),
        }
    }

    /// Settings one row's client will read, or the defaults when it has never answered.
    pub fn control(&self, key: RowKey) -> UiControl {
        self.controls
            .iter()
            .find(|(row, _)| *row == key)
            .map(|(_, settings)| settings.clone())
            .unwrap_or_default()
    }

    /// Records the settings one row confirmed.
    pub fn set_control(&mut self, key: RowKey, settings: UiControl) {
        match self.controls.iter_mut().find(|(row, _)| *row == key) {
            Some(entry) => entry.1 = settings,
            None => self.controls.push((key, settings)),
        }
    }

    /// Points the character panel at one row, or clears it.
    ///
    /// Any previous reading is dropped on a change, because showing the last character while the new
    /// row's reading is still in flight would attribute one account's character to another.
    /// Returns whether the panel changed.
    pub fn focus_player_row(&mut self, key: Option<RowKey>) -> bool {
        let key = key.filter(|key| self.row(*key).is_some());
        if self.player_row == key {
            return false;
        }
        self.player_row = key;
        self.player = None;
        true
    }

    /// Reports whether any row still holds work, which decides the close prompt.
    pub fn has_active_rows(&self) -> bool {
        self.rows.iter().any(|row| row.status.is_active())
    }

    /// Whether a Stop or a cleanup retry is still waiting for its answer.
    ///
    /// This, not the row statuses, is what the close sequence waits on: a status is the engine's last
    /// word about a session, and a stopped session publishes nothing further, so waiting for a row to
    /// leave `Stopping` on its own would wait forever.
    pub fn has_pending_lifecycle(&self) -> bool {
        self.pending
            .iter()
            .any(|(_, kind)| matches!(kind, PendingKind::Stop(_) | PendingKind::RetryCleanup(_)))
    }

    /// Consumes the refused-lifecycle flag, reporting whether the account list must be re-read.
    pub fn take_lifecycle_failure(&mut self) -> bool {
        std::mem::take(&mut self.lifecycle_failed)
    }

    /// Rows the operator may Run, capped at the batch ceiling.
    pub fn runnable_selection(&self, selected: &[RowKey]) -> Vec<(RowKey, i64)> {
        selected
            .iter()
            .filter_map(|key| self.row(*key))
            .filter(|row| row.status.context_action() == UiRowAction::Run)
            .take(MAX_BATCH_RUN)
            .map(|row| (row.key, row.revision))
            .collect()
    }

    /// Records a submitted List and clears any stale error.
    pub fn submit_list(&mut self, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::List)
    }

    pub fn submit_import(&mut self, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::Import)
    }

    pub fn submit_update(&mut self, row: RowKey, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::Update(row))
    }

    pub fn submit_delete(&mut self, row: RowKey, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::Delete(row))
    }

    /// Records a submitted Run and marks each requested row locally `Starting`.
    ///
    /// The local status is applied only on acceptance, so a rejected Run leaves the table untouched.
    pub fn submit_run(&mut self, rows: &[RowKey], result: SubmitResult) -> SubmitResult {
        if let SubmitResult::Accepted(_) = result {
            for key in rows {
                self.set_status(*key, UiAccountStatus::Starting);
            }
        }
        self.record(result, PendingKind::Run(rows.to_vec()))
    }

    pub fn submit_stop(&mut self, row: RowKey, result: SubmitResult) -> SubmitResult {
        if let SubmitResult::Accepted(_) = result {
            self.set_status(row, UiAccountStatus::Stopping);
        }
        self.record(result, PendingKind::Stop(row))
    }

    pub fn submit_retry_cleanup(&mut self, row: RowKey, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::RetryCleanup(row))
    }

    pub fn submit_observe_player(&mut self, row: RowKey, result: SubmitResult) -> SubmitResult {
        // A refused poll must not surface as the operator's last error: the panel simply keeps its
        // previous reading and the next tick asks again.
        if let SubmitResult::Accepted(request_id) = result {
            self.pending
                .push((request_id, PendingKind::ObservePlayer(row)));
        }
        result
    }

    /// Records a submitted auto-mode change. A refusal is an error the operator should see.
    pub fn submit_set_auto(&mut self, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::SetAuto)
    }

    /// Records a submitted settings read.
    ///
    /// A refusal is deliberately not the operator's last error: the read is a convenience that fills
    /// the dialog, and the dialog still opens — on the defaults — when it is refused.
    pub fn submit_observe_control(&mut self, result: SubmitResult) -> SubmitResult {
        if let SubmitResult::Accepted(request_id) = result {
            self.pending.push((request_id, PendingKind::ObserveControl));
        }
        result
    }

    /// Records a spot-book request, so its reply is not dropped as unsolicited.
    pub fn submit_spots(&mut self, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::Spots)
    }

    pub fn submit_shutdown(&mut self, result: SubmitResult) -> SubmitResult {
        self.record(result, PendingKind::Shutdown)
    }

    /// Applies one event, returning whether the table changed.
    pub fn apply(&mut self, event: UiEvent) -> bool {
        match event {
            UiEvent::RequestFinished { request_id, result } => {
                // A result whose request was never submitted, or was already settled, is dropped: it
                // refers to state this model does not own.
                let Some(index) = self
                    .pending
                    .iter()
                    .position(|(pending, _)| *pending == request_id)
                else {
                    return false;
                };
                let (_, kind) = self.pending.remove(index);
                self.apply_result(kind, result)
            }
            UiEvent::StateChanged(row) => {
                // Terminal reconciliation always wins over a UI-local pending status.
                self.upsert(row);
                true
            }
            // Readiness carries no row data; it only unblocks the first list request.
            UiEvent::WorkerReady => {
                self.worker_ready = true;
                false
            }
            UiEvent::BootFailed(failure) => {
                self.boot_failure = Some(failure);
                self.worker_closed = true;
                true
            }
            UiEvent::WorkerClosed => {
                self.worker_closed = true;
                true
            }
        }
    }

    fn apply_result(&mut self, kind: PendingKind, result: Result<UiSuccess, UiErrorCode>) -> bool {
        match result {
            Ok(success) => self.apply_success(success),
            Err(code) => {
                // A failed character poll is not operator-actionable: it repeats every second, so
                // surfacing it would bury the last real error under a stream of noise. A failed
                // settings read is the same: the dialog still opens, on the defaults.
                if matches!(
                    kind,
                    PendingKind::ObservePlayer(_) | PendingKind::ObserveControl
                ) {
                    return false;
                }
                // A refused book read leaves the model with whatever it already had, which is the
                // honest state; a refused save is worth reporting, so only the read is silent.
                if matches!(kind, PendingKind::Spots) {
                    self.last_error = Some(code);
                    return false;
                }
                self.last_error = Some(code);
                // A failed command must release the rows it optimistically marked, so one account's
                // failure never freezes another row. A lifecycle failure also means the table and the
                // engine disagree about this row, so it is flagged for a re-read: reverting alone would
                // restore a status the engine may no longer hold.
                match kind {
                    PendingKind::Run(rows) => {
                        for key in rows {
                            self.revert_pending(key);
                        }
                        self.lifecycle_failed = true;
                    }
                    PendingKind::Stop(row) => {
                        self.revert_pending(row);
                        self.lifecycle_failed = true;
                    }
                    PendingKind::RetryCleanup(_) => self.lifecycle_failed = true,
                    _ => {}
                }
                true
            }
        }
    }

    fn apply_success(&mut self, success: UiSuccess) -> bool {
        match success {
            // The book replaces whatever the model held: one spot per map, and the engine just
            // returned the whole of it, so there is no delta to reconcile.
            UiSuccess::Spots(book) => {
                self.spots = book;
                true
            }
            UiSuccess::Accounts(rows) => {
                self.rows = rows;
                // A row that vanished from the refreshed list can no longer own the panel.
                self.prune_player_focus();
                true
            }
            UiSuccess::Account(row) => {
                self.upsert(row);
                true
            }
            UiSuccess::Player { row, info } => {
                // A reading for a row the operator has since left is dropped rather than shown under
                // the newly focused account's name.
                if self.player_row != Some(row) {
                    return false;
                }
                self.player = info;
                // Deliberately never a table change. The table renders no character data, and a
                // repaint rebuilds every item: reporting a change here made the once-a-second poll
                // wipe the operator's highlight and checkboxes.
                false
            }
            UiSuccess::Control { row, settings } => {
                // Recorded whatever the engine said, including a value it clamped: the dialog must
                // open on what the client will read, not on what was typed at it.
                let mode = settings.mode;
                self.set_control(row, settings);
                self.set_auto_mode(row, mode);
                // Not a table change, for the same reason a reading is not: the table shows no
                // settings, and a repaint would cost the operator their selection.
                false
            }
            UiSuccess::Deleted(key) => {
                let before = self.rows.len();
                self.rows.retain(|row| row.key != key);
                self.prune_player_focus();
                self.rows.len() != before
            }
            UiSuccess::RunBatch(items) => {
                for item in items {
                    match item.scheduled {
                        // An admitted row is authenticating until reconciliation says otherwise.
                        Ok(()) => self.set_status(item.row, UiAccountStatus::Authenticating),
                        Err(code) => {
                            // Per-row isolation: one rejection reverts only its own row.
                            self.last_error = Some(code);
                            self.revert_pending(item.row);
                        }
                    }
                }
                true
            }
            UiSuccess::Shutdown => {
                self.worker_closed = true;
                true
            }
        }
    }

    fn record(&mut self, result: SubmitResult, kind: PendingKind) -> SubmitResult {
        match result {
            SubmitResult::Accepted(request_id) => {
                self.last_error = None;
                self.pending.push((request_id, kind));
            }
            SubmitResult::Rejected(code) => self.last_error = Some(code),
        }
        result
    }

    fn set_status(&mut self, key: RowKey, status: UiAccountStatus) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.key == key) {
            row.status = status;
        }
    }

    /// Returns an optimistically pending row to a resting status.
    fn revert_pending(&mut self, key: RowKey) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.key == key) {
            if matches!(
                row.status,
                UiAccountStatus::Starting | UiAccountStatus::Authenticating
            ) {
                row.status = UiAccountStatus::Idle;
            } else if row.status == UiAccountStatus::Stopping {
                row.status = UiAccountStatus::Running;
            }
        }
    }

    fn upsert(&mut self, incoming: UiAccountRow) {
        match self.rows.iter_mut().find(|row| row.key == incoming.key) {
            Some(existing) => *existing = incoming,
            None => self.rows.push(incoming),
        }
    }

    /// Drops the panel focus, and its reading, when the focused row is no longer in the table.
    ///
    /// Per-row settings and auto modes go with it. Row keys are never reused within a session, so a
    /// surviving entry could not be misattributed — but it would keep a deleted account's spot alive
    /// in memory for no reason.
    fn prune_player_focus(&mut self) {
        if self.player_row.is_some_and(|key| self.row(key).is_none()) {
            self.player_row = None;
            self.player = None;
        }
        let keys: Vec<RowKey> = self.rows.iter().map(|row| row.key).collect();
        self.controls.retain(|(row, _)| keys.contains(row));
        self.auto_modes.retain(|(row, _)| keys.contains(row));
    }
}

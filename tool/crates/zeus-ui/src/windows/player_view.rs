//! Character panel projection.
//!
//! Pure text: given one reading, it produces the exact lines the panel shows. No Win32 call and no
//! Core type, so every rendering rule below — the unknown wallet, the guarded percentage, the stale
//! marker — is assertable without a window and without a running game.
//!
//! Two values are easy to render wrongly and are handled deliberately:
//!
//! - Experience is permille of the *current level*, not absolute points. The client itself draws
//!   `bA/10 + "," + bA%10 + "%"`, so 105 is `10,5%` and there is no total to show.
//! - Gold and gem arrive only with the inventory packet. Before it lands both read zero, which is
//!   indistinguishable from a broke account by value alone, so an undelivered wallet renders as a dash.

use crate::model::{ITEM_RANK_OPTIONS, REVIVE_OPTIONS, ZONE_MODE_OPTIONS, UiControl, UiPlayerInfo};

/// Panel heading.
#[allow(dead_code)]
pub const PANEL_TITLE: &str = "THÔNG TIN NHÂN VẬT & CẤU HÌNH";

/// Live player card heading.
pub const INFO_PANEL_TITLE: &str = "👤 THÔNG TIN NHÂN VẬT (LIVE)";

/// Auto config summary card heading.
pub const CONFIG_PANEL_TITLE: &str = "⚙️ CẤU HÌNH AUTO ĐANG CHẠY";

/// Shown when no row owns the panel.
pub const NO_SELECTION: &str = "Chọn một tài khoản để xem thông tin nhân vật.";

/// Shown when a row is focused but the client has published nothing yet.
pub const NO_READING: &str = "Chưa có dữ liệu.\r\nChạy tài khoản và vào game để xem.";

/// Rendered for any value the client has not delivered yet.
const UNKNOWN: &str = "—";

/// Line separator. Static controls break on CRLF, which is what the panel relies on.
const LINE_BREAK: &str = "\r\n";

/// Builds the body for the Live Player Information card.
pub fn player_info_text(focused: bool, reading: Option<&UiPlayerInfo>) -> String {
    if !focused {
        return format!("{INFO_PANEL_TITLE}{LINE_BREAK}{LINE_BREAK}{NO_SELECTION}");
    }
    let Some(reading) = reading else {
        return format!("{INFO_PANEL_TITLE}{LINE_BREAK}{LINE_BREAK}{NO_READING}");
    };
    let mut body = String::with_capacity(512);
    body.push_str(INFO_PANEL_TITLE);
    body.push_str(LINE_BREAK);
    body.push_str("----------------------------------------");
    body.push_str(LINE_BREAK);
    for line in reading_lines(reading) {
        body.push_str(&line);
        body.push_str(LINE_BREAK);
    }
    body
}

/// Builds the body for the Auto Configuration card.
pub fn player_config_text(focused: bool, control: Option<&UiControl>) -> String {
    if !focused {
        return format!("{CONFIG_PANEL_TITLE}{LINE_BREAK}{LINE_BREAK}Chưa chọn tài khoản.");
    }
    let Some(ctrl) = control else {
        return format!("{CONFIG_PANEL_TITLE}{LINE_BREAK}{LINE_BREAK}Chưa có dữ liệu cấu hình.");
    };
    let mut body = String::with_capacity(512);
    body.push_str(CONFIG_PANEL_TITLE);
    body.push_str(LINE_BREAK);
    body.push_str("----------------------------------------");
    body.push_str(LINE_BREAK);
    body.push_str(&format!("• Chế độ đánh  : {}\r\n", ctrl.mode.label()));
    body.push_str(&format!("• Bán kính/Chết : {}m | {}\r\n", ctrl.radius, REVIVE_OPTIONS.get(usize::from(ctrl.revive)).copied().unwrap_or("Về làng")));
    body.push_str(&format!("• Bơm HP/MP    : HP ({} - {}%) | MP ({} - {}%)\r\n", 
        if ctrl.hp_on { "Bật" } else { "Tắt" }, ctrl.hp_percent,
        if ctrl.mp_on { "Bật" } else { "Tắt" }, ctrl.mp_percent
    ));
    body.push_str(&format!("• Nhặt & Buff  : Rank ({}) | Buff [{},{},{}]\r\n",
        ITEM_RANK_OPTIONS.get(usize::from(ctrl.item_rank)).copied().unwrap_or("Tất cả"),
        if ctrl.buffs[0] { "1" } else { "-" },
        if ctrl.buffs[1] { "2" } else { "-" },
        if ctrl.buffs[2] { "3" } else { "-" },
    ));
    body.push_str(&format!("• Di chuyển     : {}\r\n", if ctrl.nav_target > 0 { "Bật (Tự đi map)" } else { "Tắt" }));
    body.push_str(&format!("• Đổi khu       : {}\r\n", ZONE_MODE_OPTIONS.get(usize::from(ctrl.zone_mode)).copied().unwrap_or("Tắt")));
    body
}

/// Builds the whole panel body for one focused row, its reading, and its current config.
#[allow(dead_code)]
pub fn panel_text(focused: bool, reading: Option<&UiPlayerInfo>, control: Option<&UiControl>) -> String {
    if !focused {
        return format!("{PANEL_TITLE}{LINE_BREAK}{LINE_BREAK}{NO_SELECTION}");
    }
    let Some(reading) = reading else {
        let mut body = String::with_capacity(512);
        body.push_str(PANEL_TITLE);
        body.push_str(LINE_BREAK);
        body.push_str(LINE_BREAK);
        body.push_str(NO_READING);
        if let Some(ctrl) = control {
            body.push_str(LINE_BREAK);
            body.push_str(LINE_BREAK);
            body.push_str("⚙️ CẤU HÌNH AUTO ĐANG CÀI:");
            body.push_str(LINE_BREAK);
            body.push_str(&format!("• Chế độ  : {}\r\n", ctrl.mode.label()));
            body.push_str(&format!("• Bán kính: {}m | HP: {}% | MP: {}%\r\n", ctrl.radius, ctrl.hp_percent, ctrl.mp_percent));
        }
        return body;
    };
    let mut body = String::with_capacity(768);
    body.push_str("👤 THÔNG TIN NHÂN VẬT LIVE");
    body.push_str(LINE_BREAK);
    body.push_str("----------------------------------------");
    body.push_str(LINE_BREAK);
    for line in reading_lines(reading) {
        body.push_str(&line);
        body.push_str(LINE_BREAK);
    }

    if let Some(ctrl) = control {
        body.push_str(LINE_BREAK);
        body.push_str("⚙️ CẤU HÌNH AUTO ĐANG BẬT");
        body.push_str(LINE_BREAK);
        body.push_str("----------------------------------------");
        body.push_str(LINE_BREAK);
        body.push_str(&format!("• Chế độ đánh : {}\r\n", ctrl.mode.label()));
        body.push_str(&format!("• Bán kính/Chết: {}m | {}\r\n", ctrl.radius, REVIVE_OPTIONS.get(usize::from(ctrl.revive)).copied().unwrap_or("Về làng")));
        body.push_str(&format!("• Bơm HP/MP   : HP ({} - {}%) | MP ({} - {}%)\r\n", 
            if ctrl.hp_on { "Bật" } else { "Tắt" }, ctrl.hp_percent,
            if ctrl.mp_on { "Bật" } else { "Tắt" }, ctrl.mp_percent
        ));
        body.push_str(&format!("• Nhặt & Buff : Rank ({}) | Buff [{},{},{}]\r\n",
            ITEM_RANK_OPTIONS.get(usize::from(ctrl.item_rank)).copied().unwrap_or("Tất cả"),
            if ctrl.buffs[0] { "1" } else { "-" },
            if ctrl.buffs[1] { "2" } else { "-" },
            if ctrl.buffs[2] { "3" } else { "-" },
        ));
        body.push_str(&format!("• Dịch chuyển  : {}\r\n", if ctrl.nav_target > 0 { "Bật (Tự đi map)" } else { "Tắt" }));
    }
    body
}

/// The labelled lines of one reading, in display order.
pub fn reading_lines(reading: &UiPlayerInfo) -> Vec<String> {
    vec![
        labelled("Tên", &display_name(reading)),
        labelled(
            "Cấp",
            &format!(
                "{} — {}",
                reading.level,
                permille(reading.xp_permille.into())
            ),
        ),
        labelled("Tăng cấp", &experience_rate(reading.xp_permille_per_hour)),
        labelled("HP", &bar(reading.hp, reading.hp_max, reading.hp_percent())),
        labelled("MP", &bar(reading.mp, reading.mp_max, reading.mp_percent())),
        labelled("Vàng", &wallet(reading.wallet_known, reading.gold)),
        labelled("Ngọc", &wallet(reading.wallet_known, reading.gem)),
        labelled("Bản đồ", &location(reading.map_id, reading.zone)),
        labelled(
            "Toạ độ",
            &format!("{}, {}", reading.pixel_x, reading.pixel_y),
        ),
        labelled("Túi", &bag(reading.bag_used, reading.bag_max)),
        labelled("Lượt đánh", &attack_quota(reading)),
        labelled("Bang hội", reading.guild_name.as_deref().unwrap_or(UNKNOWN)),
        labelled("Thú cưỡi", if reading.mounted { "Có" } else { "Không" }),
        labelled("Trạng thái", condition(reading)),
        labelled("Tự đánh", &automation(reading)),
        labelled("Nhặt", &collector(reading)),
        labelled("Buff", &buffs(reading.buffs)),
        labelled("Đóng rớt", &closed_drops(reading.materials)),
        labelled("Đi map", &travel(reading)),
        // ---- ENHANCE ----
        labelled("Cường hóa", &enhance(reading)),
        // ---- end ENHANCE ----
        // ---- DUNGEON ----
        labelled("Phó bản", &dungeon(reading)),
        // ---- end DUNGEON ----
        labelled("Đã uống", &format!("{} bình", grouped(reading.potions))),
        labelled("Hồi sinh", &format!("{} lần", grouped(reading.revives))),
        labelled("Số liệu", &currency(reading.stale, reading.age_seconds)),
    ]
}

fn labelled(label: &str, value: &str) -> String {
    format!("{label}: {value}")
}

/// A character with no name yet is the client's own level-0 placeholder, not a real character.
fn display_name(reading: &UiPlayerInfo) -> String {
    if reading.character_name.is_empty() {
        return UNKNOWN.to_owned();
    }
    reading.character_name.clone()
}

/// Renders permille as a Vietnamese one-decimal percentage, matching the client's own `bA/10,bA%10`.
///
/// Only non-negative values reach here: experience permille is unsigned and an unmeasurable rate is
/// `None` rather than the client's `-1`.
fn permille(value: i64) -> String {
    let whole = value / 10;
    let tenth = (value % 10).abs();
    format!("{},{tenth}%", grouped(whole))
}

fn experience_rate(permille_per_hour: Option<i64>) -> String {
    match permille_per_hour {
        // A rate needs two samples over a measurable window; before that there is nothing to report,
        // and printing 0 would claim the character is not gaining when it simply has not been watched.
        None => "chưa đo được".to_owned(),
        Some(rate) => format!("{}/giờ", permille(rate)),
    }
}

/// Current over maximum, with the percentage and visual bar when the maximum is known.
fn bar(value: i64, maximum: i64, percentage: Option<u32>) -> String {
    if maximum <= 0 {
        return format!("{}/{UNKNOWN}", grouped(value));
    }
    match percentage {
        Some(percentage) => {
            let pct = percentage.min(100) as usize;
            let filled = (pct / 10).min(10);
            let empty = 10 - filled;
            let visual = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));
            format!("{visual} {}/{} ({percentage}%)", grouped(value), grouped(maximum))
        }
        None => format!("{}/{}", grouped(value), grouped(maximum)),
    }
}

/// An undelivered wallet is a dash: zero gold and unknown gold are different facts.
fn wallet(known: bool, amount: i64) -> String {
    if known {
        grouped(amount)
    } else {
        UNKNOWN.to_owned()
    }
}

fn location(map_id: Option<u16>, zone: i16) -> String {
    let map = match map_id {
        // The client resolves map names from a server-supplied table this tool never receives, so the
        // id is reported as an id rather than guessed at.
        Some(map_id) => format!("số {map_id}"),
        None => UNKNOWN.to_owned(),
    };
    if zone < 0 {
        return map;
    }
    format!("{map} — khu {zone}")
}

fn bag(used: Option<i64>, maximum: i64) -> String {
    let used = used.map(grouped).unwrap_or_else(|| UNKNOWN.to_owned());
    if maximum <= 0 {
        return format!("{used}/{UNKNOWN}");
    }
    format!("{used}/{}", grouped(maximum))
}

/// The quota gates auto-attack: at zero the client drops out of auto with nothing on screen, so an
/// exhausted quota is called out rather than left as a bare number.
fn attack_quota(reading: &UiPlayerInfo) -> String {
    if reading.quota_exhausted() {
        return format!("{} (đã hết)", grouped(reading.quota.max(0)));
    }
    grouped(reading.quota)
}

fn condition(reading: &UiPlayerInfo) -> &'static str {
    if reading.dead {
        "Đã chết"
    } else if reading.fighting {
        "Đang đánh"
    } else {
        "Bình thường"
    }
}

/// What the attack module is doing, and why it is not fighting when it is not.
///
/// The two stuck diagnoses are kept apart because they call for different fixes: an empty radius
/// means move the spot or change zone, while monsters that cannot be reached means the character is
/// wedged in terrain. Collapsing them into one message is what makes an operator try the wrong one.
fn automation(reading: &UiPlayerInfo) -> String {
    // Failing closed is silent, so a settings file the mod could not read must be named before
    // anything else: every other explanation would send the operator looking in the wrong place.
    if reading.settings_agreed == Some(false) {
        return "Không đọc được cài đặt".to_owned();
    }
    let Some(state) = reading.auto_state else {
        return "Tắt".to_owned();
    };
    let phase = match state {
        0 if reading.has_target => "đang đánh",
        0 => "đang chờ quái",
        1 => "đang về bãi",
        _ => "đang chờ ổn định",
    };
    match reading.stuck {
        1 => format!("{phase} — hết quái trong tầm"),
        2 => format!("{phase} — không tới được quái"),
        _ => phase.to_owned(),
    }
}

/// The collector settings actually in force inside the client, in the client's own words.
///
/// This is a read-back, not an echo of what the tool asked for. The client refuses settings of its
/// own accord — an unlearned buff, a record it would not accept — so rendering the request would
/// claim things are on that are not. `None` is the client's own "collector off" (`bq.q == null`).
fn collector(reading: &UiPlayerInfo) -> String {
    match reading.pickup {
        None => "Tắt".to_owned(),
        Some(pickup) => format!(
            "{} · MP,HP: {} · Vàng: {}",
            pickup.item_rank, pickup.potions, pickup.gold
        ),
    }
}

/// Which of the three buff slots the client will really cast.
///
/// Slots the character has not learned are dropped by the client itself, so a slot the operator
/// turned on can legitimately read as off here. That disagreement is the useful part.
fn buffs(slots: [bool; 3]) -> String {
    let on: Vec<String> = slots
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(index, _)| (index + 1).to_string())
        .collect();
    if on.is_empty() {
        return "Tắt".to_owned();
    }
    on.join(", ")
}

/// How many material drops the server has confirmed closed, and how many it never answered about.
///
/// Counted rather than named: the names live in the settings dialog, and repeating six of them here
/// would not fit the panel. The unknown count is the point — the close-drop is a toggle with no
/// readable state, so "never confirmed" is a real and common answer that must not read as "open".
fn closed_drops(states: [Option<bool>; crate::model::MATERIAL_SLOTS]) -> String {
    let total = states.len();
    let unknown = states.iter().filter(|state| state.is_none()).count();
    let closed = states.iter().filter(|state| **state == Some(true)).count();
    if unknown == total {
        return "chưa rõ".to_owned();
    }
    let mut line = format!("{closed}/{total} loại");
    if unknown > 0 {
        line.push_str(&format!(" ({unknown} chưa rõ)"));
    }
    line
}

/// Where TRAVEL is, and when it stopped, why.
///
/// The reason is not decoration: "walking" that never arrives and "gave up two maps ago" look
/// identical from outside, and the operator's next move differs completely. The hop count rides
/// along because a route that loops is only visible as a number that keeps climbing.
fn travel(reading: &UiPlayerInfo) -> String {
    let goal = match reading.travel_goal {
        None => return "Tắt".to_owned(),
        Some(goal) => goal,
    };
    let hops = if reading.travel_hops > 0 {
        format!(", {} lần qua map", reading.travel_hops)
    } else {
        String::new()
    };
    // Each phase carries its own preposition: "đã tới" already ends the phrase, so a shared
    // " tới " would render "đã tới tới map 43".
    let phase = match reading.travel_state {
        0 => return "Tắt".to_owned(),
        1 => "đang chờ để đi tới",
        2 => "đang dùng đá dịch chuyển tới",
        3 => "đang đi tới",
        4 => "đã tới",
        _ => {
            let why = match reading.travel_why {
                1 => "không có đường",
                2 => "map không có cửa ra, hoặc đi bị kẹt",
                3 => "đá dịch chuyển không nhận",
                4 => "đi quá nhiều map",
                5 => "lỗi trong lúc đi",
                // Stopped with no reason recorded is itself the fact worth showing, not a blank.
                _ => "không rõ lý do",
            };
            return format!("dừng — {why} (map {goal}{hops})");
        }
    };
    format!("{phase} map {goal}{hops}")
}

// ---- ENHANCE --------------------------------------------------------
/// Which step of an enhancement run the mod is on, and why it is not progressing when it is not.
///
/// The reason outranks the phase, deliberately: a run that stops returns to idle, so rendering the
/// phase first would answer every stop with "Tắt" and hide the one fact the operator needs. That is
/// the same failure [`super::model::UiPlayerInfo::travel_why`] exists to prevent.
///
/// The tally rides along on a working run because "on its third item" and "stuck on its first" are
/// the same phase repeated, and only the number tells them apart.
fn enhance(reading: &UiPlayerInfo) -> String {
    if reading.enhance_why != 0 {
        let why = match reading.enhance_why {
            1 => "không tới được thợ rèn",
            2 => "hết bùa",
            3 => "món đang làm không còn",
            // Reaching the target is a stop worth counting rather than diagnosing.
            4 => return format!("xong {} món", grouped(i64::from(reading.enhance_done))),
            // A reason this panel does not model is a mod newer than the tool. Saying so beats
            // naming the wrong one.
            _ => "không rõ lý do",
        };
        return format!("dừng — {why}");
    }
    let phase = match reading.enhance_phase {
        0 => return "Tắt".to_owned(),
        1 => "đang đi tới thợ rèn",
        2 => "đang mở thoại với thợ rèn",
        3 => "đang chọn trong menu",
        4 => "đang chọn món",
        5 => "đang chọn bùa",
        6 => "đang xác nhận",
        7 => "đang chờ kết quả",
        _ => "không rõ bước",
    };
    let done = if reading.enhance_done > 0 {
        format!(", đã xong {} món", grouped(i64::from(reading.enhance_done)))
    } else {
        String::new()
    };
    format!("{phase}{done}")
}
// ---- end ENHANCE ----------------------------------------------------
// ---- DUNGEON --------------------------------------------------------
/// Where the dungeon loop is, and why when it is not moving.
///
/// The reason outranks the state, deliberately, for the same reason it does in [`enhance`]: reaching
/// the run limit stops the loop, and a stopped loop publishes the idle state, so reading the state
/// first would answer "I finished the six runs you asked for" with "off" — indistinguishable from a
/// switch nobody turned on. That is the one fact the operator cannot recover from anywhere else.
///
/// The goal gate is the same call [`travel`] makes: an un-armed loop publishes -1 rather than a map
/// id, so "off" is read from the goal and not from the state byte, which a half-written snapshot
/// could leave stale.
fn dungeon(reading: &UiPlayerInfo) -> String {
    let goal = match reading.dungeon_goal {
        None => return "tắt".to_owned(),
        Some(goal) => goal,
    };
    if reading.dungeon_why != 0 {
        let why = match reading.dungeon_why {
            // Naming the map it wants is what makes this actionable: "no guide found" on its own
            // does not tell the operator whether to wait for a walk or to arm one.
            1 => format!("không thấy người dẫn, cần ở map {goal}"),
            2 => "menu không mở".to_owned(),
            3 => "đi bị kẹt".to_owned(),
            // Reaching the limit is a finish worth reporting rather than a fault to diagnose.
            4 => return "đủ lượt — dừng".to_owned(),
            // A reason this panel does not model is a mod newer than the tool. Saying so beats
            // naming the wrong one.
            _ => "không rõ lý do".to_owned(),
        };
        return format!("dừng — {why}");
    }
    match reading.dungeon_state {
        0 => "tắt".to_owned(),
        1 => "chờ".to_owned(),
        // Walking to the guide and talking to it are one journey from the operator's side: neither
        // is something to act on, and splitting them would make the line flicker every few seconds.
        2 | 3 => "đang vào PB".to_owned(),
        4 => format!(
            "trong PB — lượt {}",
            grouped(i64::from(reading.dungeon_runs))
        ),
        5 => format!("xong lượt {}", grouped(i64::from(reading.dungeon_runs))),
        _ => "không rõ bước".to_owned(),
    }
}
// ---- end DUNGEON ----------------------------------------------------

/// How much the reading can be trusted.
///
/// The client keeps publishing while a map loads, marking the reading stale; the age catches the other
/// failure, a client that stopped writing at all and would otherwise look permanently fresh.
fn currency(stale: bool, age_seconds: Option<i64>) -> String {
    let age = match age_seconds {
        None => return "không rõ".to_owned(),
        Some(age) if age <= 3 => "mới nhất".to_owned(),
        Some(age) if age < 60 => format!("{age} giây trước"),
        Some(age) => format!("{} phút trước", age / 60),
    };
    if stale {
        return format!("{age}, đang tải bản đồ");
    }
    age
}

/// Groups thousands with a dot, the Vietnamese separator.
fn grouped(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        grouped.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UiPickup;

    /// The values the game's own HUD showed during the live capture this module was written against.
    fn live_reading() -> UiPlayerInfo {
        UiPlayerInfo {
            character_name: "TestChar".to_owned(),
            level: 80,
            xp_permille: 105,
            xp_permille_per_hour: Some(0),
            hp: 45_991,
            hp_max: 45_991,
            mp: 9_406,
            mp_max: 9_406,
            wallet_known: true,
            gold: 44_916,
            gem: 0,
            map_id: Some(1),
            zone: 13,
            pixel_x: 504,
            pixel_y: 264,
            quota: 30_000,
            bag_used: Some(37),
            bag_max: 42,
            dead: false,
            fighting: false,
            mounted: false,
            guild_name: None,
            stale: false,
            age_seconds: Some(0),
            auto_state: Some(0),
            has_target: true,
            stuck: 0,
            pickup: Some(UiPickup {
                item_rank: "nhặt từ đồ xanh",
                potions: "nhặt tất cả",
                gold: "nhặt",
            }),
            buffs: [true, false, true],
            materials: [
                Some(true),
                None,
                Some(false),
                Some(true),
                Some(true),
                Some(false),
            ],
            mounts: Vec::new(),
            travel_state: 0,
            travel_why: 0,
            travel_goal: None,
            travel_hops: 0,
            potions: 3,
            revives: 1,
            settings_agreed: Some(true),
            // ---- ENHANCE ----
            enhance_phase: 0,
            enhance_why: 0,
            enhance_done: 0,
            // ---- end ENHANCE ----
            // ---- DUNGEON ----
            dungeon_state: 0,
            dungeon_why: 0,
            dungeon_runs: 0,
            dungeon_goal: None,
            // ---- end DUNGEON ----
        }
    }

    #[test]
    fn player_panel_renders_the_live_capture_the_way_the_game_showed_it() {
        let lines = reading_lines(&live_reading());
        assert_eq!(lines[0], "Tên: TestChar");
        // The client draws 105 permille as "10,5%"; showing "105" or "10.5" would be a different number.
        assert_eq!(lines[1], "Cấp: 80 — 10,5%");
        assert_eq!(lines[2], "Tăng cấp: 0,0%/giờ");
        assert_eq!(lines[3], "HP: [██████████] 45.991/45.991 (100%)");
        assert_eq!(lines[4], "MP: [██████████] 9.406/9.406 (100%)");
        assert_eq!(lines[5], "Vàng: 44.916");
        assert_eq!(lines[6], "Ngọc: 0");
        assert_eq!(lines[7], "Bản đồ: số 1 — khu 13");
        assert_eq!(lines[8], "Toạ độ: 504, 264");
        assert_eq!(lines[9], "Túi: 37/42");
        assert_eq!(lines[10], "Lượt đánh: 30.000");
        assert_eq!(lines[11], "Bang hội: —");
        assert_eq!(lines[12], "Thú cưỡi: Không");
        assert_eq!(lines[13], "Trạng thái: Bình thường");
        assert_eq!(lines[14], "Tự đánh: đang đánh");
        assert_eq!(
            lines[15],
            "Nhặt: nhặt từ đồ xanh · MP,HP: nhặt tất cả · Vàng: nhặt"
        );
        assert_eq!(lines[16], "Buff: 1, 3");
        assert_eq!(lines[17], "Đóng rớt: 3/6 loại (1 chưa rõ)");
        assert_eq!(lines[18], "Đi map: Tắt");
        // ---- ENHANCE ----
        assert_eq!(lines[19], "Cường hóa: Tắt");
        // ---- end ENHANCE ----
        assert_eq!(lines[20], "Phó bản: tắt");
        assert_eq!(lines[21], "Đã uống: 3 bình");
        assert_eq!(lines[22], "Hồi sinh: 1 lần");
        assert_eq!(lines[23], "Số liệu: mới nhất");
        assert_eq!(lines.len(), 24);
    }

    #[test]
    fn player_panel_says_why_auto_is_not_fighting() {
        // "Auto is on but nothing is happening" is the complaint this line exists to answer, and the
        // two stuck diagnoses need different fixes: move the spot, or unwedge the character.
        let mut reading = live_reading();
        reading.auto_state = None;
        assert_eq!(reading_lines(&reading)[14], "Tự đánh: Tắt");

        reading.auto_state = Some(0);
        reading.has_target = false;
        assert_eq!(reading_lines(&reading)[14], "Tự đánh: đang chờ quái");
        reading.stuck = 1;
        assert_eq!(
            reading_lines(&reading)[14],
            "Tự đánh: đang chờ quái — hết quái trong tầm"
        );

        reading.stuck = 2;
        reading.has_target = true;
        assert_eq!(
            reading_lines(&reading)[14],
            "Tự đánh: đang đánh — không tới được quái"
        );

        reading.stuck = 0;
        reading.auto_state = Some(1);
        assert_eq!(reading_lines(&reading)[14], "Tự đánh: đang về bãi");
        reading.auto_state = Some(2);
        assert_eq!(reading_lines(&reading)[14], "Tự đánh: đang chờ ổn định");

        // A settings file the mod could not read outranks every other explanation: any other
        // message would send the operator looking in the wrong place.
        reading.settings_agreed = Some(false);
        assert_eq!(
            reading_lines(&reading)[14],
            "Tự đánh: Không đọc được cài đặt"
        );
        // Never configured is not a failure, so it reads as off like any other idle module.
        reading.settings_agreed = None;
        reading.auto_state = None;
        assert_eq!(reading_lines(&reading)[14], "Tự đánh: Tắt");
    }

    #[test]
    fn player_panel_shows_an_undelivered_wallet_as_unknown_not_as_zero() {
        // Gold and gem only arrive with the inventory packet. Rendering 0 before it lands would tell
        // the operator the account is broke when the truth is that nothing has been delivered.
        let mut reading = live_reading();
        reading.wallet_known = false;
        reading.gold = 0;
        reading.gem = 0;
        let lines = reading_lines(&reading);
        assert_eq!(lines[5], "Vàng: —");
        assert_eq!(lines[6], "Ngọc: —");

        // A genuinely empty but delivered wallet still renders as zero.
        reading.wallet_known = true;
        let lines = reading_lines(&reading);
        assert_eq!(lines[5], "Vàng: 0");
    }

    #[test]
    fn player_panel_never_divides_by_an_unknown_maximum() {
        let mut reading = live_reading();
        reading.hp = 0;
        reading.hp_max = 0;
        reading.mp = 0;
        reading.mp_max = 0;
        reading.bag_used = None;
        reading.bag_max = 0;
        let lines = reading_lines(&reading);
        assert_eq!(lines[3], "HP: 0/—");
        assert_eq!(lines[4], "MP: 0/—");
        assert_eq!(lines[9], "Túi: —/—");
    }

    #[test]
    fn player_panel_separates_an_unmeasured_rate_from_a_measured_zero() {
        let mut reading = live_reading();
        reading.xp_permille_per_hour = None;
        assert_eq!(reading_lines(&reading)[2], "Tăng cấp: chưa đo được");

        reading.xp_permille_per_hour = Some(2_537);
        assert_eq!(reading_lines(&reading)[2], "Tăng cấp: 253,7%/giờ");
    }

    #[test]
    fn player_panel_says_where_travel_is_and_why_it_stopped() {
        /// Position of the travel line, so a line added above it fails here rather than silently
        /// asserting against whichever line moved into its place.
        const TRAVEL: usize = 18;
        let mut reading = live_reading();
        // No goal is off, whatever the state byte says: a goal-less route cannot be walked, and
        // "đang đi" with nowhere to go would be a lie the operator could not act on.
        reading.travel_state = 3;
        reading.travel_goal = None;
        assert_eq!(reading_lines(&reading)[TRAVEL], "Đi map: Tắt");

        reading.travel_goal = Some(43);
        reading.travel_hops = 2;
        assert_eq!(
            reading_lines(&reading)[TRAVEL],
            "Đi map: đang đi tới map 43, 2 lần qua map"
        );

        reading.travel_state = 2;
        reading.travel_hops = 0;
        assert_eq!(
            reading_lines(&reading)[TRAVEL],
            "Đi map: đang dùng đá dịch chuyển tới map 43"
        );

        reading.travel_state = 4;
        assert_eq!(reading_lines(&reading)[TRAVEL], "Đi map: đã tới map 43");

        // Stopped must name the reason: "walking that never arrives" and "gave up two maps ago"
        // are the same line otherwise, and they need opposite responses.
        reading.travel_state = 5;
        reading.travel_why = 1;
        assert_eq!(
            reading_lines(&reading)[TRAVEL],
            "Đi map: dừng — không có đường (map 43)"
        );
        reading.travel_why = 3;
        reading.travel_hops = 5;
        assert_eq!(
            reading_lines(&reading)[TRAVEL],
            "Đi map: dừng — đá dịch chuyển không nhận (map 43, 5 lần qua map)"
        );
        // A stop with no reason recorded is itself worth showing rather than rendering blank.
        reading.travel_why = 0;
        reading.travel_hops = 0;
        assert_eq!(
            reading_lines(&reading)[TRAVEL],
            "Đi map: dừng — không rõ lý do (map 43)"
        );
    }

    #[test]
    fn player_panel_says_which_step_enhance_is_on_and_why_it_stopped() {
        /// Position of the enhancement line, so a line added above it fails here rather than silently
        /// asserting against whichever line moved into its place.
        const ENHANCE: usize = 19;
        let mut reading = live_reading();
        // Idle is the default reading.
        assert_eq!(reading_lines(&reading)[ENHANCE], "Cường hóa: Tắt");

        // A working step reports itself, and the tally rides along once there is one to show.
        reading.enhance_phase = 3;
        assert_eq!(
            reading_lines(&reading)[ENHANCE],
            "Cường hóa: đang chọn trong menu"
        );
        reading.enhance_done = 2;
        assert_eq!(
            reading_lines(&reading)[ENHANCE],
            "Cường hóa: đang chọn trong menu, đã xong 2 món"
        );

        // The reason outranks the phase: a run that stops returns to idle, so without this a stopped
        // run would read "Tắt" and hide the one fact the operator needs.
        reading.enhance_phase = 0;
        reading.enhance_why = 2;
        assert_eq!(
            reading_lines(&reading)[ENHANCE],
            "Cường hóa: dừng — hết bùa"
        );
        reading.enhance_why = 4;
        assert_eq!(reading_lines(&reading)[ENHANCE], "Cường hóa: xong 2 món");
    }

    #[test]
    fn player_panel_reports_a_finished_dungeon_limit_as_a_finish_and_not_as_off() {
        /// Position of the dungeon line, so a line added above it fails here rather than silently
        /// asserting against whichever line moved into its place.
        const DUNGEON: usize = 20;
        let mut reading = live_reading();
        // Off is the default reading.
        assert_eq!(reading_lines(&reading)[DUNGEON], "Phó bản: tắt");

        // Each state names itself, and the tally rides along once there is one to show.
        reading.dungeon_goal = Some(48);
        reading.dungeon_state = 1;
        assert_eq!(reading_lines(&reading)[DUNGEON], "Phó bản: chờ");
        reading.dungeon_state = 3;
        assert_eq!(reading_lines(&reading)[DUNGEON], "Phó bản: đang vào PB");
        reading.dungeon_state = 4;
        reading.dungeon_runs = 2;
        assert_eq!(
            reading_lines(&reading)[DUNGEON],
            "Phó bản: trong PB — lượt 2"
        );

        // The reason outranks the state. Reaching the limit stops the loop and the mod publishes the
        // idle state along with the reason, so without this ordering the panel would say "tắt" for a
        // run that did exactly what the operator asked of it.
        reading.dungeon_state = 0;
        reading.dungeon_why = 4;
        assert_eq!(reading_lines(&reading)[DUNGEON], "Phó bản: đủ lượt — dừng");
    }

    #[test]
    fn player_panel_calls_out_the_states_that_silently_stop_progress() {
        // An exhausted quota is the top cause of "auto stopped for no reason": the client drops out of
        // auto-attack with nothing on screen, so the panel must name it.
        let mut reading = live_reading();
        reading.quota = 0;
        assert_eq!(reading_lines(&reading)[10], "Lượt đánh: 0 (đã hết)");

        reading.dead = true;
        assert_eq!(reading_lines(&reading)[13], "Trạng thái: Đã chết");
        reading.dead = false;
        reading.fighting = true;
        assert_eq!(reading_lines(&reading)[13], "Trạng thái: Đang đánh");
    }

    #[test]
    fn player_panel_reports_a_stale_or_frozen_reading_instead_of_looking_fresh() {
        /// Position of the currency line, so adding a line above it fails here rather than silently
        /// asserting against whichever line moved into its place.
        const CURRENCY: usize = 23;
        let mut reading = live_reading();
        reading.stale = true;
        assert_eq!(
            reading_lines(&reading)[CURRENCY],
            "Số liệu: mới nhất, đang tải bản đồ"
        );

        // A client that stopped writing keeps stale=0 forever, so the age is the only signal left.
        reading.stale = false;
        reading.age_seconds = Some(42);
        assert_eq!(reading_lines(&reading)[CURRENCY], "Số liệu: 42 giây trước");
        reading.age_seconds = Some(605);
        assert_eq!(reading_lines(&reading)[CURRENCY], "Số liệu: 10 phút trước");
        reading.age_seconds = None;
        assert_eq!(reading_lines(&reading)[CURRENCY], "Số liệu: không rõ");
    }

    #[test]
    fn player_panel_shows_the_collector_the_client_really_has_not_the_one_we_asked_for() {
        // These two lines exist to expose disagreement. The client drops a buff the character never
        // learned and turns its whole collector off on its own, so rendering the request would claim
        // things are running that are not.
        let mut reading = live_reading();
        reading.pickup = None;
        reading.buffs = [false, false, false];
        let lines = reading_lines(&reading);
        assert_eq!(lines[15], "Nhặt: Tắt");
        assert_eq!(lines[16], "Buff: Tắt");

        // A byte outside the client's own tables is named as unknown, never guessed at.
        reading.pickup = Some(UiPickup {
            item_rank: "không rõ",
            potions: "chỉ nhặt HP",
            gold: "không nhặt",
        });
        reading.buffs = [false, true, false];
        let lines = reading_lines(&reading);
        assert_eq!(
            lines[15],
            "Nhặt: không rõ · MP,HP: chỉ nhặt HP · Vàng: không nhặt"
        );
        assert_eq!(lines[16], "Buff: 2");
    }

    #[test]
    fn the_panel_keeps_an_unconfirmed_close_drop_apart_from_an_open_one() {
        // The close-drop is a toggle with no readable state, so "never confirmed" is a common and
        // real answer. Rendering it as open would tell the operator a setting is in force when the
        // tool has no idea, which is the failure this line exists to prevent.
        let mut reading = live_reading();
        reading.materials = [None; crate::model::MATERIAL_SLOTS];
        assert_eq!(reading_lines(&reading)[17], "Đóng rớt: chưa rõ");

        reading.materials = [Some(false); crate::model::MATERIAL_SLOTS];
        assert_eq!(reading_lines(&reading)[17], "Đóng rớt: 0/6 loại");

        reading.materials = [Some(true); crate::model::MATERIAL_SLOTS];
        assert_eq!(reading_lines(&reading)[17], "Đóng rớt: 6/6 loại");
    }

    #[test]
    fn player_panel_distinguishes_no_selection_from_no_reading() {
        // One is an instruction to the operator, the other is a status of the selected account.
        let empty = panel_text(false, None, None);
        assert!(empty.starts_with(PANEL_TITLE));
        assert!(empty.contains(NO_SELECTION));

        let waiting = panel_text(true, None, None);
        assert!(waiting.contains("Chưa có dữ liệu."));
        assert!(!waiting.contains(NO_SELECTION));

        let shown = panel_text(true, Some(&live_reading()), None);
        assert!(shown.contains("Tên: TestChar"));
        assert!(!shown.contains(NO_SELECTION));
        // Static controls break on CRLF, so the body must not rely on a bare newline.
        assert!(shown.contains("\r\n"));
        assert!(!shown.contains("\n\n"));
    }

    #[test]
    fn player_panel_never_renders_an_account_identity_or_a_path() {
        let reading = UiPlayerInfo {
            guild_name: Some("Zeus Clan".to_owned()),
            mounted: true,
            map_id: None,
            zone: -1,
            ..live_reading()
        };
        let body = panel_text(true, Some(&reading), None);
        assert!(body.contains("Bang hội: Zeus Clan"));
        assert!(body.contains("Thú cưỡi: Có"));
        // An unresolved map is unknown rather than reported as map 0 or as a negative zone.
        assert!(body.contains("Bản đồ: —"));
        assert!(!body.contains("khu -1"));
        // Nothing the panel prints may be a path, a drive, or a backend identifier. Slashes are
        // deliberately not banned: HP, MP, the bag, and the rate all render as "current/maximum".
        for forbidden in ["\\", "profile", "session", "runtime", "uuid", "_"] {
            assert!(
                !body.to_lowercase().contains(forbidden),
                "panel body renders {forbidden}"
            );
        }
    }

    #[test]
    fn player_panel_groups_thousands_with_the_vietnamese_separator() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1.000");
        assert_eq!(grouped(44_916), "44.916");
        assert_eq!(grouped(1_234_567), "1.234.567");
        // A negative quota is possible in the client and must not render as "-.1".
        assert_eq!(grouped(-1_500), "-1.500");
    }

    #[test]
    fn player_panel_permille_keeps_one_decimal_at_every_magnitude() {
        assert_eq!(permille(0), "0,0%");
        assert_eq!(permille(5), "0,5%");
        assert_eq!(permille(105), "10,5%");
        assert_eq!(permille(1_000), "100,0%");
        assert_eq!(permille(12_345), "1.234,5%");
    }
}

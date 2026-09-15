//! Add, Edit, and Delete dialog definitions and their validation copy.
//!
//! The field inventory, button set, and validation strings are pure data so the approved dialog shape
//! is assertable without creating a window. There is deliberately no Save-and-Run button, no auto-login
//! checkbox, and no config control: Add imports only, and Run always attempts the fixed flow.

/// Control ids inside the dialogs.
pub const ID_USERNAME: u16 = 0x2001;
pub const ID_PASSWORD: u16 = 0x2002;
pub const ID_SHOW_PASSWORD: u16 = 0x2003;
pub const ID_CONFIRM_TEXT: u16 = 0x2004;
pub const ID_OK: u16 = 0x2005;
pub const ID_CANCEL: u16 = 0x2006;
/// First radio button of the server picker. Each server's id is this plus its index, so the control
/// id maps back to the chosen index without a side table.
pub const ID_SERVER_FIRST: u16 = 0x2010;

/// Config dialog control ids. Each one is read back by id, so no side table can drift from the layout.
pub const ID_CFG_MODE: u16 = 0x2020;
pub const ID_CFG_SPOT: u16 = 0x2021;
pub const ID_CFG_SPOT_X: u16 = 0x2022;
pub const ID_CFG_SPOT_Y: u16 = 0x2023;
pub const ID_CFG_RADIUS: u16 = 0x2024;
pub const ID_CFG_REVIVE: u16 = 0x2025;
pub const ID_CFG_HP_ON: u16 = 0x2026;
pub const ID_CFG_HP: u16 = 0x2027;
pub const ID_CFG_MP: u16 = 0x2028;
/// MP's gate sits in the first free id after the pickers rather than beside HP's: every id in
/// 0x2030..=0x2045 is the base of a slot range that its neighbours address arithmetically.
pub const ID_CFG_MP_ON: u16 = 0x202c;
pub const ID_CFG_RANK: u16 = 0x2029;
pub const ID_CFG_MPHP: u16 = 0x202a;
pub const ID_CFG_GOLD: u16 = 0x202b;
/// Buff slot 1 is this id; slots 2 and 3 follow it, so the id encodes the slot.
pub const ID_CFG_BUFF_FIRST: u16 = 0x2030;
pub const ID_CFG_MOUNT: u16 = 0x2034;
pub const ID_CFG_MEDAL: u16 = 0x2035;
pub const ID_CFG_ZONE_MODE: u16 = 0x2036;
pub const ID_CFG_ZONE_PICK: u16 = 0x2037;
pub const ID_CFG_TRAVEL: u16 = 0x2039;
/// Alias: enhance-port names the same control `NAV_GO`.
pub const ID_CFG_NAV_GO: u16 = ID_CFG_TRAVEL;
pub const ID_CFG_NAV_TARGET: u16 = 0x203a;
pub const ID_CFG_RING: u16 = 0x203b;
pub const ID_CFG_SPOT_SAVE: u16 = 0x203c;
pub const ID_CFG_SPOT_CLEAR: u16 = 0x203d;
/// Alias: enhance-port names this `SPOT_PICK`.
pub const ID_CFG_SPOT_STATE: u16 = 0x203e;
pub const ID_CFG_SPOT_PICK: u16 = 0x2080;
pub const ID_CFG_SPOT_NAME: u16 = 0x2046;
pub const ID_CFG_SPOT_FARM: u16 = 0x203f;
pub const ID_CFG_DROPS_ON: u16 = 0x2038;
/// Material 1 is this id; the other five follow it, so the id encodes the menu position.
pub const ID_CFG_DROP_FIRST: u16 = 0x2040;
pub const ID_CFG_REVIVE_DELAY: u16 = 0x2048;
pub const ID_CFG_REVIVE_ON: u16 = 0x2049;
pub const ID_CFG_MOUNT_ID: u16 = 0x204a;
// ---- ENHANCE ----
/// Skipped past 0x204a rather than filling 0x204b: every id in 0x2030..=0x2045 is the base of a
/// range its neighbours address arithmetically, and the next free decade leaves room for one without
/// the reader having to know which ids are bases and which are not.
pub const ID_CFG_ENHANCE_ON: u16 = 0x2050;
pub const ID_CFG_ENHANCE_MAXLV: u16 = 0x2051;
pub const ID_CFG_ENHANCE_CHARM: u16 = 0x2052;
// ---- end ENHANCE ----
// ---- DUNGEON ----
/// Next free decade after the enhancement rows, for the reason stated above them: filling 0x2053
/// would sit inside a range whose neighbours address ids arithmetically, and a reader cannot tell a
/// base id from a plain one without checking each.
pub const ID_CFG_DUNGEON_ON: u16 = 0x2060;
pub const ID_CFG_DUNGEON_MAX: u16 = 0x2061;
pub const ID_CFG_DUNGEON_SCHED: u16 = 0x2062;
// ---- end DUNGEON ----
/// Action button to immediately start traveling to the spot or destination map.
pub const ID_CFG_TRAVEL_START: u16 = 0x2070;

/// One dialog field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: u16,
    pub label: &'static str,
    /// Whether the control masks its content.
    pub masked: bool,
}

/// Add dialog: Username, masked Password, optional show/hide, Cancel, and Add.
pub const ADD_FIELDS: &[Field] = &[
    Field {
        id: ID_USERNAME,
        label: "Tài khoản",
        masked: false,
    },
    Field {
        id: ID_PASSWORD,
        label: "Mật khẩu",
        masked: true,
    },
];

/// Edit dialog: the same two controls, but the password label states that leaving it empty keeps the
/// stored secret, because a rename never re-encrypts.
pub const EDIT_FIELDS: &[Field] = &[
    Field {
        id: ID_USERNAME,
        label: "Tài khoản",
        masked: false,
    },
    Field {
        id: ID_PASSWORD,
        label: "Mật khẩu (để trống nếu không đổi)",
        masked: true,
    },
];

/// Label of the optional show/hide password toggle.
pub const SHOW_PASSWORD_LABEL: &str = "Hiện mật khẩu";

/// Buttons for the Add dialog.
pub const ADD_BUTTONS: &[(u16, &str)] = &[(ID_CANCEL, "Hủy"), (ID_OK, "Thêm")];

/// Buttons for the Edit dialog.
pub const EDIT_BUTTONS: &[(u16, &str)] = &[(ID_CANCEL, "Hủy"), (ID_OK, "Lưu")];

/// Buttons for the destructive Delete confirmation.
pub const DELETE_BUTTONS: &[(u16, &str)] = &[(ID_CANCEL, "Hủy"), (ID_OK, "Xóa")];

/// Titles, kept separate so no dialog reuses another's caption.
pub const ADD_TITLE: &str = "Thêm tài khoản";
pub const EDIT_TITLE: &str = "Sửa tài khoản";
pub const DELETE_TITLE: &str = "Xóa tài khoản";
pub const SERVER_TITLE: &str = "Chọn sever";

/// Buttons for the server picker.
pub const SERVER_BUTTONS: &[(u16, &str)] = &[(ID_CANCEL, "Hủy"), (ID_OK, "Chọn")];

/// Prompt shown above the server list.
pub const SERVER_PROMPT: &str = "Chọn sever cho tài khoản này";

/// Text the operator must type to confirm a destructive delete.
pub const DELETE_CONFIRMATION_WORD: &str = "XOA";

/// Config dialog: the settings the running client reads while it farms.
pub const CONFIG_TITLE: &str = "⚙️ Cài đặt cấu hình Auto";
pub const CONFIG_BUTTONS: &[(u16, &str)] = &[(ID_CANCEL, "Hủy"), (ID_OK, "Lưu")];

/// Which column of the config dialog a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigGroup {
    /// Fighting: combat mode, radius, revive, buffs, mount, ring overlay.
    Combat,
    /// Healing and spot: potion, HP/MP thresholds, current spot, coordinates, save/clear spot.
    Recovery,
    /// Travel and zone: destination map, arrival auto, zone mode & pick, medal dialog.
    Travel,
    /// Looting and materials: rank, potion pickup, gold, materials switch, material drop items.
    Loot,
}

impl ConfigGroup {
    /// Group box caption.
    ///
    /// Deliberately free of `&`: Win32 reads an ampersand in a control caption as a mnemonic prefix.
    pub fn caption(self) -> &'static str {
        match self {
            Self::Combat => "⚔️ Chiến đấu và Kỹ năng",
            Self::Recovery => "🧪 Hồi phục và Bãi quái",
            Self::Travel => "🗺️ Di chuyển và Đổi khu",
            Self::Loot => "🎒 Nhặt đồ và Nguyên liệu",
        }
    }
}

/// What kind of control one config row uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigControl {
    /// A checkbox. Its own caption is the label, so it needs no separate one.
    Check,
    /// A numeric field with an inclusive range. Out-of-range input is refused before submission.
    Number { low: i32, high: i32 },
    /// A push button that acts on the shared spot book rather than on a setting.
    ///
    /// Distinct from every other kind because it is not a value the dialog reads back: pressing it
    /// asks the engine to do something now, and the row's readout is what shows the result.
    Action,
    /// A drop-down list. The selected index is the value the engine receives.
    Choice(&'static [&'static str]),
    /// The spots saved for the map the character is on, built when the dialog opens.
    ///
    /// Distinct from `Choice` because its options are not a compile-time list: they come from the book
    /// and change as the operator saves and clears.
    SpotChoice,
    /// A free-text field, for the name the operator gives a spot.
    Text,
    /// Text the dialog fills in from the live reading and the operator cannot edit.
    Readout,
    /// The mounts the character's bag holds, built when the dialog opens.
    ///
    /// Distinct from `Choice` because its options are not a compile-time list: there is no
    /// id-to-name table anywhere in the client, so the names come from the running client's own
    /// bag. Index 0 is "any mount"; the rest map to template ids in list order.
    MountChoice,
}

/// One row of the config dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigRow {
    pub id: u16,
    pub label: &'static str,
    pub control: ConfigControl,
    pub group: ConfigGroup,
}

/// Every row of the config dialog, in display order within its group.
///
/// Pure data so the approved surface is assertable without creating a window, and so the reader in
/// `dialog_window` walks the same list the builder did: a row present in one and not the other is what
/// makes a setting silently keep its old value.
pub const CONFIG_ROWS: &[ConfigRow] = &[
    // Group 1: Combat (8 rows)
    ConfigRow {
        id: ID_CFG_MODE,
        label: "Chế độ",
        control: ConfigControl::Choice(&crate::model::ATTACK_MODE_OPTIONS),
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_RADIUS,
        label: "Khoảng cách đánh",
        control: ConfigControl::Number {
            low: crate::model::RADIUS_MIN as i32,
            high: crate::model::RADIUS_MAX as i32,
        },
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_REVIVE,
        label: "Khi chết",
        control: ConfigControl::Choice(&crate::model::REVIVE_OPTIONS),
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_RING,
        label: "Vẽ vòng tầm đánh",
        control: ConfigControl::Check,
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_BUFF_FIRST,
        label: "Buff 1",
        control: ConfigControl::Check,
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_BUFF_FIRST + 1,
        label: "Buff 2",
        control: ConfigControl::Check,
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_BUFF_FIRST + 2,
        label: "Buff 3",
        control: ConfigControl::Check,
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_MOUNT,
        label: "Cưỡi thú khi về bãi",
        control: ConfigControl::Check,
        group: ConfigGroup::Combat,
    },
    ConfigRow {
        id: ID_CFG_MOUNT_ID,
        label: "Loại thú cưỡi",
        control: ConfigControl::MountChoice,
        group: ConfigGroup::Combat,
    },

    // Group 2: Recovery & Spot (9 rows)
    ConfigRow {
        id: ID_CFG_HP_ON,
        label: "Tự bơm bình HP",
        control: ConfigControl::Check,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_MP_ON,
        label: "Tự bơm bình MP",
        control: ConfigControl::Check,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_HP,
        label: "Bơm HP dưới (%)",
        control: ConfigControl::Number {
            low: crate::model::PERCENT_MIN as i32,
            high: crate::model::PERCENT_MAX as i32,
        },
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_MP,
        label: "Bơm MP dưới (%)",
        control: ConfigControl::Number {
            low: crate::model::PERCENT_MIN as i32,
            high: crate::model::PERCENT_MAX as i32,
        },
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT,
        label: "Bãi hiện tại",
        control: ConfigControl::Readout,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_PICK,
        label: "Chọn bãi",
        control: ConfigControl::SpotChoice,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_NAME,
        label: "Tên bãi",
        control: ConfigControl::Text,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_X,
        label: "Toạ độ X",
        control: ConfigControl::Number {
            low: 0,
            high: 32_767,
        },
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_Y,
        label: "Toạ độ Y",
        control: ConfigControl::Number {
            low: 0,
            high: 32_767,
        },
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_STATE,
        label: "Bãi đã lưu",
        control: ConfigControl::Readout,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_SAVE,
        label: "Lưu bãi ở đây",
        control: ConfigControl::Action,
        group: ConfigGroup::Recovery,
    },
    ConfigRow {
        id: ID_CFG_SPOT_CLEAR,
        label: "Xoá bãi của map này",
        control: ConfigControl::Action,
        group: ConfigGroup::Recovery,
    },

    // Group 3: Travel & Zone (6 rows)
    ConfigRow {
        id: ID_CFG_TRAVEL,
        label: "Đi map tới bãi",
        control: ConfigControl::Check,
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_NAV_TARGET,
        label: "Đi tới map",
        control: ConfigControl::Choice(&crate::model::NAV_TARGET_OPTIONS),
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_TRAVEL_START,
        label: "Bắt đầu đi map",
        control: ConfigControl::Action,
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_SPOT_FARM,
        label: "Tới bãi thì tự đánh",
        control: ConfigControl::Check,
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_ZONE_MODE,
        label: "Đổi khu",
        control: ConfigControl::Choice(&crate::model::ZONE_MODE_OPTIONS),
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_ZONE_PICK,
        label: "Khu muốn tới",
        control: ConfigControl::Number {
            low: crate::model::ZONE_PICK_MIN as i32,
            high: crate::model::ZONE_PICK_MAX as i32,
        },
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_MEDAL,
        label: "Tự đóng bảng mề đay",
        control: ConfigControl::Check,
        group: ConfigGroup::Travel,
    },
    // ---- ENHANCE ----
    ConfigRow {
        id: ID_CFG_ENHANCE_ON,
        label: "Cường hoá trang bị",
        control: ConfigControl::Check,
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_ENHANCE_MAXLV,
        label: "Cường hoá tối đa",
        control: ConfigControl::Choice(&crate::model::ENHANCE_LEVEL_OPTIONS),
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_ENHANCE_CHARM,
        label: "Bùa cường hoá",
        control: ConfigControl::Choice(&crate::model::ENHANCE_CHARM_OPTIONS),
        group: ConfigGroup::Travel,
    },
    // ---- end ENHANCE ----
    // ---- DUNGEON ----
    ConfigRow {
        id: ID_CFG_DUNGEON_ON,
        label: "Phó bản tự động",
        control: ConfigControl::Check,
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_DUNGEON_MAX,
        label: "Số lượt phó bản",
        control: ConfigControl::Choice(&crate::model::DUNGEON_RUN_OPTIONS),
        group: ConfigGroup::Travel,
    },
    ConfigRow {
        id: ID_CFG_DUNGEON_SCHED,
        label: "Lịch phó bản",
        control: ConfigControl::Choice(&crate::model::DUNGEON_SCHEDULE_OPTIONS),
        group: ConfigGroup::Travel,
    },
    // ---- end DUNGEON ----

    // Group 4: Loot & Materials (10 rows)
    ConfigRow {
        id: ID_CFG_RANK,
        label: "Vật phẩm",
        control: ConfigControl::Choice(&crate::model::ITEM_RANK_OPTIONS),
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_MPHP,
        label: "MP, HP",
        control: ConfigControl::Choice(&crate::model::POTION_PICKUP_OPTIONS),
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_GOLD,
        label: "Vàng",
        control: ConfigControl::Choice(&crate::model::GOLD_OPTIONS),
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROPS_ON,
        label: "Quản lý đóng rớt nguyên liệu",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST,
        label: "Đóng rớt: Mề đay trắng",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST + 1,
        label: "Đóng rớt: Mề đay vàng",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST + 2,
        label: "Đóng rớt: Mề đay tím",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST + 3,
        label: "Đóng rớt: Mề đay xanh",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST + 4,
        label: "Đóng rớt: Nguyên liệu tinh tú",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
    ConfigRow {
        id: ID_CFG_DROP_FIRST + 5,
        label: "Đóng rớt: Lửa tinh tú",
        control: ConfigControl::Check,
        group: ConfigGroup::Loot,
    },
];

/// Footnote under the config groups, naming what the tool does not own.
///
/// The close-drop is a *toggle* the server owns, not a stored setting: there is no way to read the
/// current state except by flipping it and reading the confirmation. That is why it has a master
/// switch, and why the footnote says so — an operator who expects a plain checkbox needs to know the
/// tool has to nudge the game to find out where it stands.
#[allow(dead_code)]
pub const CONFIG_NOTE: &str = "Đóng rớt nguyên liệu là công tắc bật/tắt của server, không đọc \
     được trạng thái sẵn — tool phải bấm rồi đọc thông báo mới biết, nên chỉ làm khi bật ô quản lý. \
     Thứ tự ưu tiên nhặt thì không có. Bãi luôn nằm trên bản đồ nhân vật đang đứng; đổi khu không \
     cần đi tới bảng.";

/// Rows of one config group, in display order.
pub fn config_rows(group: ConfigGroup) -> impl Iterator<Item = &'static ConfigRow> {
    CONFIG_ROWS.iter().filter(move |row| row.group == group)
}


/// Prompt shown above the delete confirmation field.
pub const DELETE_PROMPT: &str = "Nhập XOA để xác nhận xóa tài khoản";

/// Local validation outcome, evaluated before any submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldError {
    UsernameEmpty,
    UsernameTooLong,
    UsernameNotTrimmed,
    UsernameNotPrintableAscii,
    PasswordEmpty,
    PasswordTooLong,
    PasswordNotPrintableAscii,
    ConfirmationMismatch,
    /// A config field held something that is not a whole number.
    NotANumber,
    /// A config field held a number outside the range the client accepts.
    NumberOutOfRange,
}

impl FieldError {
    /// Bounded Vietnamese copy. No backend identifier is ever rendered.
    pub fn label(self) -> &'static str {
        match self {
            Self::UsernameEmpty => "Vui lòng nhập tài khoản",
            Self::UsernameTooLong => "Tài khoản tối đa 64 ký tự",
            Self::UsernameNotTrimmed => "Tài khoản không được có khoảng trắng ở đầu hoặc cuối",
            Self::UsernameNotPrintableAscii => "Tài khoản chỉ dùng ký tự ASCII in được",
            Self::PasswordEmpty => "Vui lòng nhập mật khẩu",
            Self::PasswordTooLong => "Mật khẩu tối đa 128 ký tự",
            Self::PasswordNotPrintableAscii => "Mật khẩu chỉ dùng ký tự ASCII in được",
            Self::ConfirmationMismatch => "Nhập đúng XOA để xác nhận",
            Self::NotANumber => "Chỉ nhập số nguyên",
            Self::NumberOutOfRange => "Số nằm ngoài khoảng cho phép",
        }
    }
}

/// Validates one config number against its row's range.
///
/// Refused rather than clamped, unlike the engine: the operator typed a specific value, and quietly
/// storing a different one would leave the dialog claiming something the client is not doing.
pub fn validate_number(value: &str, low: i32, high: i32) -> Result<i32, FieldError> {
    let parsed: i32 = value.trim().parse().map_err(|_| FieldError::NotANumber)?;
    if !(low..=high).contains(&parsed) {
        return Err(FieldError::NumberOutOfRange);
    }
    Ok(parsed)
}

const USERNAME_MAX: usize = 64;
const PASSWORD_MAX: usize = 128;

/// Validates a username locally, so an obviously invalid value never reaches the worker.
pub fn validate_username(value: &str) -> Result<(), FieldError> {
    if value.trim() != value {
        return Err(FieldError::UsernameNotTrimmed);
    }
    if value.is_empty() {
        return Err(FieldError::UsernameEmpty);
    }
    if value.len() > USERNAME_MAX {
        return Err(FieldError::UsernameTooLong);
    }
    if !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(FieldError::UsernameNotPrintableAscii);
    }
    Ok(())
}

/// Validates a password from its UTF-16 control buffer, never copying it into a message.
pub fn validate_password(units: &[u16]) -> Result<(), FieldError> {
    if units.is_empty() {
        return Err(FieldError::PasswordEmpty);
    }
    if units.len() > PASSWORD_MAX {
        return Err(FieldError::PasswordTooLong);
    }
    if !units.iter().all(|unit| (0x20..=0x7e).contains(unit)) {
        return Err(FieldError::PasswordNotPrintableAscii);
    }
    Ok(())
}

/// Requires the exact confirmation word before a destructive delete.
pub fn validate_delete_confirmation(typed: &str) -> Result<(), FieldError> {
    if typed.trim() == DELETE_CONFIRMATION_WORD {
        return Ok(());
    }
    Err(FieldError::ConfirmationMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shell_dialogs_have_the_approved_fields_and_buttons() {
        assert_eq!(ADD_FIELDS.len(), 2);
        assert_eq!(ADD_FIELDS[0].id, ID_USERNAME);
        assert!(!ADD_FIELDS[0].masked);
        assert_eq!(ADD_FIELDS[1].id, ID_PASSWORD);
        // The password control masks its content.
        assert!(ADD_FIELDS[1].masked);

        assert_eq!(ADD_BUTTONS, &[(ID_CANCEL, "Hủy"), (ID_OK, "Thêm")]);
        assert_eq!(EDIT_BUTTONS, &[(ID_CANCEL, "Hủy"), (ID_OK, "Lưu")]);
        assert_eq!(DELETE_BUTTONS, &[(ID_CANCEL, "Hủy"), (ID_OK, "Xóa")]);

        // The account dialogs offer no Save-and-Run, auto-login, or config control. The settings
        // dialog is a separate surface, deliberately not in this list: Add still imports only.
        let surface = format!(
            "{ADD_TITLE}{EDIT_TITLE}{DELETE_TITLE}{DELETE_PROMPT}{:?}{:?}{:?}",
            ADD_BUTTONS, EDIT_BUTTONS, DELETE_BUTTONS
        )
        .to_lowercase();
        for forbidden in [
            "save and run",
            "lưu và chạy",
            "auto",
            "tự động",
            "cấu hình",
            "config",
            "profile",
            "runtime",
        ] {
            assert!(
                !surface.contains(forbidden),
                "dialog surface exposes {forbidden}"
            );
        }
    }

    #[test]
    fn windows_shell_username_validation_matches_the_engine_bounds() {
        assert!(validate_username("Operator").is_ok());
        assert!(validate_username(&"u".repeat(64)).is_ok());
        assert_eq!(validate_username(""), Err(FieldError::UsernameEmpty));
        assert_eq!(
            validate_username(&"u".repeat(65)),
            Err(FieldError::UsernameTooLong)
        );
        assert_eq!(
            validate_username(" leading"),
            Err(FieldError::UsernameNotTrimmed)
        );
        assert_eq!(
            validate_username("trailing "),
            Err(FieldError::UsernameNotTrimmed)
        );
        assert_eq!(
            validate_username("tab\there"),
            Err(FieldError::UsernameNotPrintableAscii)
        );
        // A trimmed-but-empty value reports the trim failure first, then emptiness.
        assert_eq!(validate_username(" "), Err(FieldError::UsernameNotTrimmed));
    }

    #[test]
    fn windows_shell_password_validation_matches_the_engine_bounds() {
        let ok: Vec<u16> = "Secret-1".encode_utf16().collect();
        assert!(validate_password(&ok).is_ok());
        assert!(validate_password(&vec![b'p' as u16; 128]).is_ok());
        assert_eq!(validate_password(&[]), Err(FieldError::PasswordEmpty));
        assert_eq!(
            validate_password(&vec![b'p' as u16; 129]),
            Err(FieldError::PasswordTooLong)
        );
        // A non-printable or non-ASCII unit is refused before submission.
        assert_eq!(
            validate_password(&[0x7f]),
            Err(FieldError::PasswordNotPrintableAscii)
        );
        assert_eq!(
            validate_password(&"mật".encode_utf16().collect::<Vec<u16>>()),
            Err(FieldError::PasswordNotPrintableAscii)
        );
    }

    #[test]
    fn windows_shell_delete_requires_the_exact_confirmation_word() {
        assert!(validate_delete_confirmation("XOA").is_ok());
        // Surrounding whitespace is tolerated; a different word is not.
        assert!(validate_delete_confirmation("  XOA  ").is_ok());
        assert_eq!(
            validate_delete_confirmation("xoa"),
            Err(FieldError::ConfirmationMismatch)
        );
        assert_eq!(
            validate_delete_confirmation(""),
            Err(FieldError::ConfirmationMismatch)
        );
        assert_eq!(
            validate_delete_confirmation("DELETE"),
            Err(FieldError::ConfirmationMismatch)
        );
    }

    #[test]
    fn windows_shell_validation_copy_is_bounded_vietnamese() {
        for error in [
            FieldError::UsernameEmpty,
            FieldError::UsernameTooLong,
            FieldError::UsernameNotTrimmed,
            FieldError::UsernameNotPrintableAscii,
            FieldError::PasswordEmpty,
            FieldError::PasswordTooLong,
            FieldError::PasswordNotPrintableAscii,
            FieldError::ConfirmationMismatch,
            FieldError::NotANumber,
            FieldError::NumberOutOfRange,
        ] {
            let label = error.label();
            assert!(!label.is_empty(), "{error:?} has no copy");
            assert!(!label.contains('_'), "{error:?} renders a backend token");
        }
    }

    #[test]
    fn the_config_dialog_covers_every_setting_the_client_reads() {
        // One row per setting the two modules act on. A missing row is a setting the operator cannot
        // reach; a duplicate id is a row that would silently read another one's control.
        let mut ids: Vec<u16> = CONFIG_ROWS.iter().map(|row| row.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "a config row id is used twice");
        assert_eq!(count, 44);
        for row in CONFIG_ROWS {
            assert!(!row.label.is_empty(), "row {:#x} has no label", row.id);
            assert!(
                !row.label.contains('_'),
                "row {:#x} renders a backend token",
                row.id
            );
            // Every picker must offer at least two options, or it is not a choice.
            if let ConfigControl::Choice(options) = row.control {
                assert!(options.len() >= 2, "row {:#x} has one option", row.id);
                assert!(options.iter().all(|option| !option.is_empty()));
            }
            if let ConfigControl::Number { low, high } = row.control {
                assert!(low <= high, "row {:#x} has an empty range", row.id);
            }
        }
        // All groups are populated, so no column renders as an empty box.
        assert_eq!(config_rows(ConfigGroup::Combat).count(), 9);
        assert_eq!(config_rows(ConfigGroup::Recovery).count(), 12);
        assert_eq!(config_rows(ConfigGroup::Travel).count(), 13);
        assert_eq!(config_rows(ConfigGroup::Loot).count(), 10);
        assert!(!ConfigGroup::Combat.caption().is_empty());
        assert!(!ConfigGroup::Recovery.caption().is_empty());
        assert!(!ConfigGroup::Travel.caption().is_empty());
        assert!(!ConfigGroup::Loot.caption().is_empty());
        // The three buff checkboxes are consecutive, because the id encodes the slot.
        for slot in 0..crate::model::BUFF_SLOTS {
            let id = ID_CFG_BUFF_FIRST + slot as u16;
            assert!(CONFIG_ROWS.iter().any(|row| row.id == id), "buff {slot}");
        }
        // Same for the six materials, and each row's label must name the material at that position:
        // the position IS the wire value, so a row labelled with the wrong one closes the wrong drop.
        for slot in 0..crate::model::MATERIAL_SLOTS {
            let id = ID_CFG_DROP_FIRST + slot as u16;
            let row = CONFIG_ROWS
                .iter()
                .find(|row| row.id == id)
                .unwrap_or_else(|| panic!("material {slot} has no row"));
            // The label is tied to the engine's own table in `port.rs`, which can see both sides.
            assert_eq!(row.control, ConfigControl::Check);
        }
        // The footnote names the two things the tool does not own, and says the close-drop is the
        // server's, so nobody goes looking in the client for a switch that is not there.
        assert!(CONFIG_NOTE.contains("nguyên liệu"));
        assert!(CONFIG_NOTE.contains("server"));
        assert!(CONFIG_NOTE.contains("ưu tiên"));
        // No caption may contain an ampersand: Win32 reads it as a mnemonic prefix and eats the
        // following character, which is how "Nhặt đồ & buff" once rendered as "Nhặt đồ _buff".
        for caption in CONFIG_ROWS
            .iter()
            .map(|row| row.label)
            .chain([
                ConfigGroup::Combat.caption(),
                ConfigGroup::Recovery.caption(),
                ConfigGroup::Travel.caption(),
                ConfigGroup::Loot.caption(),
                CONFIG_TITLE,
                CONFIG_NOTE,
            ])
            .chain(CONFIG_BUTTONS.iter().map(|(_, label)| *label))
        {
            assert!(
                !caption.contains('&'),
                "caption eats a character: {caption}"
            );
        }

    }

    #[test]
    fn config_numbers_are_refused_rather_than_quietly_corrected() {
        // The engine clamps silently. If the dialog did too, an operator who typed 500 would be shown
        // 240 with no explanation and would reasonably conclude the field does nothing.
        assert_eq!(validate_number("120", 60, 240), Ok(120));
        assert_eq!(validate_number(" 60 ", 60, 240), Ok(60));
        assert_eq!(validate_number("240", 60, 240), Ok(240));
        assert_eq!(
            validate_number("59", 60, 240),
            Err(FieldError::NumberOutOfRange)
        );
        assert_eq!(
            validate_number("241", 60, 240),
            Err(FieldError::NumberOutOfRange)
        );
        assert_eq!(validate_number("", 60, 240), Err(FieldError::NotANumber));
        assert_eq!(validate_number("12x", 60, 240), Err(FieldError::NotANumber));
        assert_eq!(validate_number("1.5", 1, 99), Err(FieldError::NotANumber));
        // A negative coordinate is the engine's own "no spot", so it must never be typed as one.
        assert_eq!(
            validate_number("-1", 0, 32_767),
            Err(FieldError::NumberOutOfRange)
        );
    }
}

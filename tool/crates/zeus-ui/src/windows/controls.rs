//! Account table layout and row projection.
//!
//! The column inventory and per-row cell text are pure data, so the approved table shape is assertable
//! without creating an HWND. The `RowKey` travels in the list view's hidden item data and is never
//! rendered as a cell.

use crate::model::{RowKey, UiAccountRow, UiRowAction};

/// One table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub index: usize,
    /// Header text. The checkbox and action columns are intentionally unlabeled.
    pub header: &'static str,
    pub width: i32,
}

/// The exact approved columns: selection checkbox, Username, Server, Status, Last run.
pub const COLUMNS: &[Column] = &[
    Column {
        index: 0,
        header: "",
        width: 28,
    },
    Column {
        index: 1,
        header: "Tài khoản",
        width: 200,
    },
    Column {
        index: 2,
        header: "Sever",
        width: 140,
    },
    Column {
        index: 3,
        header: "Trạng thái",
        width: 150,
    },
    Column {
        index: 4,
        header: "Lần chạy cuối",
        width: 160,
    },
];

/// Rendered cells of one row, in column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowCells {
    /// Hidden routing key, carried as list view item data rather than as text.
    pub key: RowKey,
    pub username: String,
    pub server: &'static str,
    pub status: &'static str,
    pub last_run: String,
    pub action: UiRowAction,
}

/// Projects one model row into its rendered cells.
pub fn project_row(row: &UiAccountRow) -> RowCells {
    RowCells {
        key: row.key,
        username: row.username.clone(),
        server: server_name(row.server_index),
        status: row.status.label(),
        last_run: format_last_run(row.last_run_at_unix_ms),
        action: row.status.context_action(),
    }
}

/// Names the world an index selects, or reports the index as unknown.
///
/// An out-of-range index cannot come from Core, which validates on both read and write, so this
/// renders a placeholder rather than panicking on a value a future build might introduce.
pub fn server_name(server_index: u8) -> &'static str {
    zeus_core::SERVER_NAMES
        .get(usize::from(server_index))
        .copied()
        .unwrap_or("Không rõ")
}

/// Formats a run stamp as a local calendar value, or the never-run placeholder.
///
/// The stamp is rendered without a backend identifier, so no epoch integer reaches the operator.
pub fn format_last_run(unix_ms: Option<i64>) -> String {
    let Some(unix_ms) = unix_ms else {
        return "Chưa có".to_owned();
    };
    if unix_ms <= 0 {
        return "Chưa có".to_owned();
    }
    let seconds = unix_ms / 1_000;
    let days = seconds / 86_400;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = time_of_day / 3_600;
    let minute = (time_of_day % 3_600) / 60;
    format!("{day:02}/{month:02}/{year} {hour:02}:{minute:02}")
}

/// Converts days since the Unix epoch into a civil date.
///
/// Implemented locally because the crate deliberately has no date dependency; the algorithm is the
/// standard shift-to-March civil calendar conversion.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use crate::model::UiAccountStatus;

    #[test]
    fn windows_shell_table_has_exactly_the_approved_columns() {
        assert_eq!(COLUMNS.len(), 5);
        let headers: Vec<&str> = COLUMNS.iter().map(|column| column.header).collect();
        assert_eq!(
            headers,
            vec![
                "",
                "Tài khoản",
                "Sever",
                "Trạng thái",
                "Lần chạy cuối",
            ]
        );
        // Indices are dense and ordered, so a column cannot be silently reordered.
        for (position, column) in COLUMNS.iter().enumerate() {
            assert_eq!(column.index, position);
            assert!(column.width > 0);
        }
        for column in COLUMNS {
            let header = column.header.to_lowercase();
            for forbidden in ["id", "uuid", "profile", "session", "runtime", "pid"] {
                assert!(
                    !header.contains(forbidden),
                    "column header exposes {forbidden}"
                );
            }
        }
    }

    #[test]
    fn windows_shell_projects_rows_without_leaking_identity() {
        let row = UiAccountRow {
            key: RowKey::new(NonZeroU64::new(7).expect("nonzero")),
            revision: 3,
            username: "Operator".to_owned(),
            status: UiAccountStatus::Running,
            last_run_at_unix_ms: Some(1_800_000_000_000),
            server_index: 6,
        };

        let cells = project_row(&row);
        assert_eq!(cells.username, "Operator");
        assert_eq!(cells.status, "Đang chạy");
        assert_eq!(cells.action, UiRowAction::Stop);
        // The world is rendered by name, so no raw index reaches the operator.
        assert_eq!(cells.server, "Thiên Hà (New)");
        // The key routes the row but never appears in rendered text.
        assert_eq!(cells.key, row.key);
        assert!(!cells.username.contains('7'));
        assert!(!cells.last_run.contains("1800000000000"));
    }

    #[test]
    fn windows_shell_names_every_world_and_refuses_an_unknown_index() {
        // Every valid index renders a non-empty name, so no row can show a blank world.
        for index in 0..zeus_core::SERVER_NAMES.len() {
            let name = server_name(index as u8);
            assert!(!name.is_empty(), "index {index} rendered empty");
            assert_eq!(name, zeus_core::SERVER_NAMES[index]);
        }
        // One past the table is a placeholder rather than a panic or a wrong world.
        assert_eq!(server_name(zeus_core::SERVER_NAMES.len() as u8), "Không rõ");
        assert_eq!(server_name(u8::MAX), "Không rõ");
    }

    #[test]
    fn windows_shell_formats_last_run_or_the_never_placeholder() {
        assert_eq!(format_last_run(None), "Chưa có");
        assert_eq!(format_last_run(Some(0)), "Chưa có");
        assert_eq!(format_last_run(Some(-1)), "Chưa có");
        // 1_800_000_000_000 ms is 2027-01-15 08:00 UTC, cross-checked independently.
        assert_eq!(format_last_run(Some(1_800_000_000_000)), "15/01/2027 08:00");
        // The Unix epoch itself renders as a real date, not as a raw integer.
        assert_eq!(format_last_run(Some(1_000)), "01/01/1970 00:00");
    }

    #[test]
    fn windows_shell_row_action_matches_row_status() {
        for (status, expected) in [
            (UiAccountStatus::Idle, UiRowAction::Run),
            (UiAccountStatus::LoginFailed, UiRowAction::Run),
            (UiAccountStatus::Running, UiRowAction::Stop),
            (UiAccountStatus::Starting, UiRowAction::Stop),
            (UiAccountStatus::Authenticating, UiRowAction::Stop),
            (UiAccountStatus::Stopping, UiRowAction::None),
            (UiAccountStatus::CleanupPending, UiRowAction::RetryCleanup),
        ] {
            let row = UiAccountRow {
                key: RowKey::new(NonZeroU64::new(1).expect("nonzero")),
                revision: 1,
                username: "Row".to_owned(),
                status,
                last_run_at_unix_ms: None,
                server_index: 0,
            };
            assert_eq!(project_row(&row).action, expected, "status {status:?}");
        }
    }
}

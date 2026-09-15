//! DPI-scaled shell geometry.
//!
//! Every rectangle the shell places is derived here from the window's current DPI and the measured
//! caption width, so the layout is assertable without creating an HWND and cannot drift from what
//! `WM_SIZE` and `WM_DPICHANGED` apply.

/// Reference DPI of the constants below. Windows reports 96 at 100% scaling.
pub const REFERENCE_DPI: u32 = 96;

/// Design constants, in reference pixels at [`REFERENCE_DPI`].
const PADDING: i32 = 8;
const GAP: i32 = 6;
const BUTTON_HEIGHT: i32 = 28;
const STATUS_HEIGHT: i32 = 20;
/// Floor for a caption cell, so a short caption still yields a clickable target.
const MIN_BUTTON_WIDTH: i32 = 72;
/// Width of the character panel beside the table. Wide enough for the longest labelled line.
const PLAYER_PANEL_WIDTH: i32 = 270;
/// Commands in the longest command row, which is what the minimum width has to hold.
///
/// Named here rather than read from `icons`, because this module stays free of the command
/// inventory; the test below asserts the two agree, so adding a button fails there.
const WIDEST_COMMAND_ROW: usize = 9;
/// Smallest usable client area: the widest command row plus a table tall enough for a few rows.
///
/// The row is uniform-width and spans the whole client area, so this floor is what keeps the last
/// button on screen. Adding a command widens it; a narrower window would simply hide the button
/// with nothing to reveal it.
const MIN_CLIENT_WIDTH: i32 = 760;
const MIN_CLIENT_HEIGHT: i32 = 380;

/// A control rectangle in client coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Scales a reference-DPI length to the supplied DPI, rounding to nearest.
///
/// A zero DPI cannot come from `GetDpiForWindow` on a live window, but it is treated as the reference
/// so a hostile or stubbed value can never divide by zero or collapse the layout.
pub fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { REFERENCE_DPI } else { dpi };
    let scaled = i64::from(value) * i64::from(dpi) + i64::from(REFERENCE_DPI / 2);
    (scaled / i64::from(REFERENCE_DPI)) as i32
}

/// All shell geometry for one DPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellMetrics {
    pub dpi: u32,
    pub padding: i32,
    pub gap: i32,
    pub button_height: i32,
    pub status_height: i32,
}

impl ShellMetrics {
    pub fn new(dpi: u32) -> Self {
        Self {
            dpi: if dpi == 0 { REFERENCE_DPI } else { dpi },
            padding: scale(PADDING, dpi),
            gap: scale(GAP, dpi),
            button_height: scale(BUTTON_HEIGHT, dpi),
            status_height: scale(STATUS_HEIGHT, dpi),
        }
    }

    /// Widens a measured caption to a full button width: text plus symmetric horizontal padding.
    ///
    /// Capped so a row of `columns` buttons always fits `client_width`. Past that the captions clip,
    /// which is visible and recoverable by widening the window; a button placed beyond the client
    /// edge is neither. The minimum width is set so the cap does not normally bite.
    pub fn button_width(&self, measured_text_width: i32, columns: usize, client_width: i32) -> i32 {
        let padded = measured_text_width + self.padding * 3;
        let wanted = padded.max(scale(MIN_BUTTON_WIDTH, self.dpi));
        let columns = columns.max(1) as i32;
        let available = client_width - self.padding * 2 - self.gap * (columns - 1);
        // A client too narrow to hold even one pixel per button yields 1 rather than 0 or negative,
        // so `SetWindowPos` is never handed an inverted rectangle.
        let ceiling = (available / columns).max(1);
        wanted.min(ceiling)
    }

    /// Places the `index`-th button of a command row of uniform width.
    pub fn button_rect(&self, row: usize, index: usize, button_width: i32) -> Rect {
        let row_pitch = self.button_height + self.gap;
        Rect {
            x: self.padding + (button_width + self.gap) * index as i32,
            y: self.padding + row_pitch * row as i32,
            width: button_width,
            height: self.button_height,
        }
    }

    /// Total height reserved by `rows` command rows above the table.
    pub fn header_height(&self, rows: usize) -> i32 {
        self.padding + (self.button_height + self.gap) * rows as i32
    }

    /// The table fills everything between the command rows and the status line, left of the panel.
    pub fn table_rect(&self, client_width: i32, client_height: i32, rows: usize) -> Rect {
        let top = self.header_height(rows);
        let bottom = client_height - self.status_height - self.padding;
        let panel = self.player_panel_rect(client_width, client_height, rows);
        Rect {
            x: self.padding,
            y: top,
            // The panel is placed first and the table takes what is left, so a narrow window shrinks
            // the table rather than letting the two overlap.
            width: (panel.x - self.gap - self.padding).max(0),
            height: (bottom - top).max(0),
        }
    }

    /// The character panel occupies a fixed-width column on the right of the table's vertical band.
    ///
    /// Fixed rather than proportional: the lines it renders are a known length, so a share of the
    /// window would either clip them on a small screen or waste space on a large one.
    pub fn player_panel_rect(&self, client_width: i32, client_height: i32, rows: usize) -> Rect {
        let top = self.header_height(rows);
        let bottom = client_height - self.status_height - self.padding;
        let width = scale(PLAYER_PANEL_WIDTH, self.dpi);
        // Never wider than the client can hold, so a forced-narrow window cannot push it off screen.
        let width = width.min((client_width - self.padding * 2).max(0));
        Rect {
            x: (client_width - self.padding - width).max(self.padding),
            y: top,
            width,
            height: (bottom - top).max(0),
        }
    }

    /// Top block: Live Character Information card.
    pub fn player_info_rect(&self, client_width: i32, client_height: i32, rows: usize) -> Rect {
        let whole = self.player_panel_rect(client_width, client_height, rows);
        let total_h = whole.height;
        let gap = self.gap;
        let info_h = ((total_h - gap) * 55) / 100;
        Rect {
            height: info_h.max(0),
            ..whole
        }
    }

    /// Bottom block: Applied Auto Configuration card.
    pub fn player_config_rect(&self, client_width: i32, client_height: i32, rows: usize) -> Rect {
        let whole = self.player_panel_rect(client_width, client_height, rows);
        let total_h = whole.height;
        let gap = self.gap;
        let info_h = ((total_h - gap) * 55) / 100;
        let config_y = whole.y + info_h + gap;
        let config_h = (total_h - info_h - gap).max(0);
        Rect {
            y: config_y,
            height: config_h,
            ..whole
        }
    }

    /// The status line sits on the bottom padding band, inset like the table.
    pub fn status_rect(&self, client_width: i32, client_height: i32) -> Rect {
        Rect {
            x: self.padding,
            y: (client_height - self.status_height - self.padding / 2).max(0),
            width: (client_width - self.padding * 2).max(0),
            height: self.status_height,
        }
    }

    /// Minimum client size, so no resize can collapse the table to nothing.
    ///
    /// Never narrower than the widest command row at its own floor width, so even a build whose
    /// captions all measure zero keeps every button inside the client area.
    pub fn min_client_size(&self) -> (i32, i32) {
        let columns = WIDEST_COMMAND_ROW as i32;
        let row = self.padding * 2
            + scale(MIN_BUTTON_WIDTH, self.dpi) * columns
            + self.gap * (columns - 1);
        (
            scale(MIN_CLIENT_WIDTH, self.dpi).max(row),
            scale(MIN_CLIENT_HEIGHT, self.dpi),
        )
    }
}

/// Config dialog design constants, in reference pixels.
const CONFIG_MARGIN: i32 = 8;
const CONFIG_ROW_HEIGHT: i32 = 20;
const CONFIG_ROW_GAP: i32 = 4;
const CONFIG_LABEL_WIDTH: i32 = 112;
const CONFIG_CONTROL_WIDTH: i32 = 124;
const CONFIG_INNER_GAP: i32 = 6;
/// Height reserved for a group box caption above its first row.
const CONFIG_CAPTION_HEIGHT: i32 = 22;
/// Footnote removed, height is 0.
const CONFIG_NOTE_HEIGHT: i32 = 0;
const CONFIG_ERROR_HEIGHT: i32 = 20;
const CONFIG_BUTTON_HEIGHT: i32 = 28;
const CONFIG_BUTTON_WIDTH: i32 = 92;


/// Geometry of the settings dialog for one DPI.
///
/// Separate from [`ShellMetrics`] because the dialog is a fixed grid rather than a resizable layout:
/// its size is derived from how many rows the tallest group has, so adding a setting widens the window
/// instead of clipping a control. Every rectangle is assertable without creating a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigMetrics {
    pub dpi: u32,
    /// Rows in the tallest group, which sets the height of both group boxes.
    pub rows: i32,
}

impl ConfigMetrics {
    pub fn new(dpi: u32, rows: usize) -> Self {
        Self {
            dpi: if dpi == 0 { REFERENCE_DPI } else { dpi },
            // At least one row, so an empty group cannot produce a negative box.
            rows: (rows as i32).max(1),
        }
    }

    fn row_pitch(&self) -> i32 {
        scale(CONFIG_ROW_HEIGHT + CONFIG_ROW_GAP, self.dpi)
    }

    fn group_width(&self) -> i32 {
        scale(
            CONFIG_MARGIN
                + CONFIG_LABEL_WIDTH
                + CONFIG_INNER_GAP
                + CONFIG_CONTROL_WIDTH
                + CONFIG_MARGIN,
            self.dpi,
        )
    }

    fn group_height(&self) -> i32 {
        scale(CONFIG_CAPTION_HEIGHT + CONFIG_MARGIN, self.dpi) + self.row_pitch() * self.rows
    }

    /// The `column`-th group box, in client coordinates.
    pub fn group_rect(&self, column: usize) -> Rect {
        let margin = scale(CONFIG_MARGIN, self.dpi);
        Rect {
            x: margin + (self.group_width() + margin) * column as i32,
            y: margin,
            width: self.group_width(),
            height: self.group_height(),
        }
    }

    /// The label of the `index`-th row of the `column`-th group.
    pub fn label_rect(&self, column: usize, index: usize) -> Rect {
        let group = self.group_rect(column);
        Rect {
            x: group.x + scale(CONFIG_MARGIN, self.dpi),
            y: group.y
                + scale(CONFIG_CAPTION_HEIGHT, self.dpi)
                + self.row_pitch() * index as i32
                + scale(CONFIG_ROW_GAP / 2, self.dpi),
            width: scale(CONFIG_LABEL_WIDTH, self.dpi),
            height: scale(CONFIG_ROW_HEIGHT, self.dpi),
        }
    }

    /// The control of the `index`-th row of the `column`-th group.
    pub fn control_rect(&self, column: usize, index: usize) -> Rect {
        let label = self.label_rect(column, index);
        Rect {
            x: label.x + label.width + scale(CONFIG_INNER_GAP, self.dpi),
            y: label.y,
            width: scale(CONFIG_CONTROL_WIDTH, self.dpi),
            height: label.height,
        }
    }

    /// A checkbox spans the whole row: its own caption is the label.
    pub fn check_rect(&self, column: usize, index: usize) -> Rect {
        let label = self.label_rect(column, index);
        let control = self.control_rect(column, index);
        Rect {
            width: control.x + control.width - label.x,
            ..label
        }
    }

    /// The footnote band under both groups.
    pub fn note_rect(&self, columns: usize) -> Rect {
        let margin = scale(CONFIG_MARGIN, self.dpi);
        Rect {
            x: margin,
            y: margin + self.group_height() + margin,
            width: self.content_width(columns),
            height: scale(CONFIG_NOTE_HEIGHT, self.dpi),
        }
    }

    /// The validation line under the footnote.
    pub fn error_rect(&self, columns: usize) -> Rect {
        let note = self.note_rect(columns);
        Rect {
            y: note.y + note.height,
            height: scale(CONFIG_ERROR_HEIGHT, self.dpi),
            ..note
        }
    }

    /// The `index`-th button, laid out from the right edge so Save is rightmost.
    pub fn button_rect(&self, columns: usize, index: usize, count: usize) -> Rect {
        let margin = scale(CONFIG_MARGIN, self.dpi);
        let width = scale(CONFIG_BUTTON_WIDTH, self.dpi);
        let error = self.error_rect(columns);
        let right = margin + self.content_width(columns);
        let slots = count.max(1) as i32;
        let offset = slots - 1 - index as i32;
        Rect {
            x: right - width - (width + margin) * offset,
            y: error.y + error.height + scale(CONFIG_ROW_GAP, self.dpi),
            width,
            height: scale(CONFIG_BUTTON_HEIGHT, self.dpi),
        }
    }

    fn content_width(&self, columns: usize) -> i32 {
        let margin = scale(CONFIG_MARGIN, self.dpi);
        let columns = columns.max(1) as i32;
        self.group_width() * columns + margin * (columns - 1)
    }

    /// Client size the dialog needs for every control it places.
    pub fn client_size(&self, columns: usize) -> (i32, i32) {
        let margin = scale(CONFIG_MARGIN, self.dpi);
        let buttons = self.button_rect(columns, 0, 1);
        (
            self.content_width(columns) + margin * 2,
            buttons.y + buttons.height + margin,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigMetrics, PLAYER_PANEL_WIDTH, REFERENCE_DPI, ShellMetrics, WIDEST_COMMAND_ROW, scale,
    };

    #[test]
    fn the_widest_command_row_matches_the_shipped_inventory() {
        // The layout constant and the command list live in different modules on purpose, so this is
        // what makes adding a button widen the minimum instead of pushing it off the client edge.
        let widest = super::super::icons::TOOLBAR_COMMANDS
            .len()
            .max(super::super::icons::ROW_COMMANDS.len());
        assert_eq!(widest, WIDEST_COMMAND_ROW);
    }

    #[test]
    fn scaling_is_identity_at_the_reference_dpi_and_proportional_above_it() {
        assert_eq!(scale(100, REFERENCE_DPI), 100);
        // 125% and 150%, the two scalings Windows offers by default on laptop panels.
        assert_eq!(scale(100, 120), 125);
        assert_eq!(scale(100, 144), 150);
        // Rounding is to nearest, never truncating toward zero.
        assert_eq!(scale(7, 120), 9);
    }

    #[test]
    fn a_zero_dpi_falls_back_to_the_reference_instead_of_collapsing() {
        // GetDpiForWindow returns 0 only for an invalid window; the layout must stay usable.
        assert_eq!(scale(100, 0), 100);
        assert_eq!(ShellMetrics::new(0).dpi, REFERENCE_DPI);
        assert_eq!(ShellMetrics::new(0), ShellMetrics::new(REFERENCE_DPI));
    }

    #[test]
    fn command_rows_never_overlap_each_other_or_the_table() {
        for dpi in [96, 120, 144, 192] {
            let metrics = ShellMetrics::new(dpi);
            let width = metrics.button_width(80, 6, 1200);
            let first = metrics.button_rect(0, 0, width);
            let second = metrics.button_rect(1, 0, width);
            assert!(
                first.y + first.height <= second.y,
                "rows overlap at {dpi} dpi"
            );
            let neighbour = metrics.button_rect(0, 1, width);
            assert!(
                first.x + first.width <= neighbour.x,
                "columns overlap at {dpi} dpi"
            );
            let table = metrics.table_rect(1200, 800, 2);
            assert!(
                second.y + second.height <= table.y,
                "table overlaps the second row at {dpi} dpi"
            );
            let panel = metrics.player_panel_rect(1200, 800, 2);
            assert!(
                table.x + table.width <= panel.x,
                "panel overlaps the table at {dpi} dpi"
            );
            assert!(
                panel.x + panel.width <= 1200,
                "panel leaves the client area at {dpi} dpi"
            );
            // The panel shares the table's vertical band exactly, so neither can crowd the status line.
            assert_eq!(panel.y, table.y);
            assert_eq!(panel.height, table.height);
            let status = metrics.status_rect(1200, 800);
            assert!(
                table.y + table.height <= status.y,
                "status overlaps the table at {dpi} dpi"
            );
            assert!(
                status.y + status.height <= 800,
                "status leaves the client area at {dpi} dpi"
            );
        }
    }

    #[test]
    fn a_client_smaller_than_the_chrome_yields_no_negative_extent() {
        let metrics = ShellMetrics::new(144);
        let table = metrics.table_rect(10, 10, 2);
        assert_eq!(table.width, 0);
        assert_eq!(table.height, 0);
        let status = metrics.status_rect(0, 0);
        assert_eq!(status.width, 0);
        assert_eq!(status.y, 0);
        // A client narrower than the panel keeps it on screen instead of pushing it off the left edge.
        let panel = metrics.player_panel_rect(10, 10, 2);
        assert!(panel.x >= metrics.padding);
        assert_eq!(panel.width, 0);
        assert_eq!(metrics.player_panel_rect(0, 0, 2).width, 0);
    }

    #[test]
    fn the_minimum_client_size_leaves_room_for_the_table_and_the_panel() {
        for dpi in [96, 120, 144, 192] {
            let metrics = ShellMetrics::new(dpi);
            let (width, height) = metrics.min_client_size();
            let table = metrics.table_rect(width, height, 2);
            let panel = metrics.player_panel_rect(width, height, 2);
            // The panel must never be squeezed, and the table must keep a usable width beside it.
            assert_eq!(panel.width, scale(PLAYER_PANEL_WIDTH, dpi), "panel squeezed at {dpi} dpi");
            assert!(
                table.width >= scale(400, dpi),
                "table collapsed to {} at {dpi} dpi",
                table.width
            );
            assert!(table.height > 0);
        }
    }

    #[test]
    fn a_button_is_never_narrower_than_the_scaled_minimum() {
        let metrics = ShellMetrics::new(120);
        let floor = metrics.button_width(0, 1, 4_000);
        assert_eq!(floor, scale(72, 120));
        // A wide caption widens the button instead of being clipped.
        assert!(metrics.button_width(400, 1, 4_000) > floor);
    }

    #[test]
    fn a_command_row_never_reaches_past_the_client_edge() {
        // The row spans the whole client area, so a button placed beyond it is simply invisible with
        // nothing the operator can drag to reveal it. Adding a command must widen the minimum, and
        // this is what fails when it does not.
        for dpi in [96, 120, 144, 192] {
            let metrics = ShellMetrics::new(dpi);
            let (min_width, min_height) = metrics.min_client_size();
            // 180 reference pixels is wider than the longest Vietnamese caption the shell ships,
            // measured at 172 device pixels at 96 dpi, so this is the pessimistic case.
            let measured = scale(180, dpi);
            let width = metrics.button_width(measured, WIDEST_COMMAND_ROW, min_width);
            let last = metrics.button_rect(1, WIDEST_COMMAND_ROW - 1, width);
            assert!(
                last.x + last.width <= min_width,
                "the last row button leaves the client at {dpi} dpi: {} > {min_width}",
                last.x + last.width
            );
            assert!(width > 0);
            // Shrinking below the minimum clips captions rather than producing an inverted rect.
            let squeezed = metrics.button_width(measured, WIDEST_COMMAND_ROW, 40);
            assert!(squeezed >= 1);
            let last = metrics.button_rect(1, WIDEST_COMMAND_ROW - 1, squeezed);
            assert!(last.width >= 1);
            // The table still keeps a usable width at the minimum, beside the panel.
            let table = metrics.table_rect(min_width, min_height, 2);
            assert!(table.width >= scale(400, dpi));
        }
    }

    #[test]
    fn the_minimum_client_size_scales_with_dpi() {
        let (low_width, low_height) = ShellMetrics::new(96).min_client_size();
        let (high_width, high_height) = ShellMetrics::new(144).min_client_size();
        assert!(high_width > low_width && high_height > low_height);
    }

    #[test]
    fn the_config_grid_holds_every_row_inside_its_group_at_every_scaling() {
        // The dialog is the one fixed-size window in the shell, so a control that fell outside its
        // group box or the client area would simply be invisible with nothing to resize. The row
        // count is the real one, so adding a setting fails here rather than on the operator's screen.
        let rows = [
            super::super::dialogs::ConfigGroup::Combat,
            super::super::dialogs::ConfigGroup::Recovery,
            super::super::dialogs::ConfigGroup::Travel,
            super::super::dialogs::ConfigGroup::Loot,
        ]
        .into_iter()
        .map(|group| super::super::dialogs::config_rows(group).count())
        .max()
        .expect("all groups are populated");
        for dpi in [96, 120, 144, 192] {
            let metrics = ConfigMetrics::new(dpi, rows);
            let (client_width, client_height) = metrics.client_size(4);
            for column in 0..4 {
                let group = metrics.group_rect(column);
                assert!(group.x >= 0 && group.y >= 0);
                assert!(
                    group.x + group.width <= client_width,
                    "group {column} leaves the client at {dpi} dpi"
                );
                for index in 0..rows {
                    let label = metrics.label_rect(column, index);
                    let control = metrics.control_rect(column, index);
                    let check = metrics.check_rect(column, index);
                    assert!(
                        label.x + label.width <= control.x,
                        "label overlaps its control at {dpi} dpi"
                    );
                    assert!(
                        control.x + control.width <= group.x + group.width,
                        "row {index} of group {column} leaves its box at {dpi} dpi"
                    );
                    assert!(
                        label.y + label.height <= group.y + group.height,
                        "row {index} overflows its box at {dpi} dpi"
                    );
                    assert_eq!(check.x, label.x);
                    assert_eq!(check.width, control.x + control.width - label.x);
                    if index > 0 {
                        let previous = metrics.label_rect(column, index - 1);
                        assert!(
                            previous.y + previous.height <= label.y,
                            "rows overlap at {dpi} dpi"
                        );
                    }
                }
            }
            // Columns never overlap each other
            for col in 0..3 {
                let first = metrics.group_rect(col);
                let second = metrics.group_rect(col + 1);
                assert!(first.x + first.width <= second.x);
            }
            // The note, the error line, and the buttons stack under the groups without overlapping.
            let note = metrics.note_rect(4);
            let error = metrics.error_rect(4);
            let cancel = metrics.button_rect(4, 0, 2);
            let save = metrics.button_rect(4, 1, 2);
            assert!(metrics.group_rect(0).y + metrics.group_rect(0).height <= note.y);
            assert!(note.y + note.height <= error.y);
            assert!(error.y + error.height <= cancel.y);
            assert!(cancel.x + cancel.width <= save.x);
            assert!(save.x + save.width <= client_width);
            assert!(save.y + save.height <= client_height);
            // The dialog cannot be resized, so it has to fit a 1080p laptop at 125% scaling
            // with room for the frame and the taskbar.
            if dpi <= 120 {
                assert!(
                    client_width <= 1_500 && client_height <= 820,
                    "the settings dialog is {client_width}x{client_height} at {dpi} dpi"
                );
            }
        }
    }

    #[test]
    fn the_config_dialog_grows_with_its_row_count_instead_of_clipping() {
        let short = ConfigMetrics::new(96, 4);
        let tall = ConfigMetrics::new(96, 9);
        assert!(tall.client_size(2).1 > short.client_size(2).1);
        // Width follows the column count, not the row count.
        assert_eq!(short.client_size(2).0, tall.client_size(2).0);
        assert!(tall.client_size(2).0 > tall.client_size(1).0);
        // A group with no rows still yields a positive box rather than an inverted one.
        let empty = ConfigMetrics::new(96, 0);
        assert!(empty.group_rect(0).height > 0);
        assert!(empty.client_size(1).1 > 0);
    }
}

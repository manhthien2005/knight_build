//! Named monster spots, several per map, shared by every account.
//!
//! A monster spot belongs to the world, not to a login, so spots are one shared table rather than a copy
//! inside each account's profile: two accounts farming the same map want the same coordinates, and
//! copies would be free to disagree.
//!
//! Several per map, each named. One per map was the first shape and it was wrong: a map holds more than
//! one worthwhile place to stand, and replacing the only entry every time made the second one
//! unreachable. The name is how the operator tells them apart, so it is theirs to write — a generated
//! label is a suggestion, never the identity.
//!
//! The `spots` table is the storage, with `(map_id, name)` as the key: "saving a name again moves it" is
//! then the key's own rule rather than a second implementation of it, and a spot has the same durability
//! as the account that farms with it. A text file came first; it is still parsed, once, so an upgrade
//! carries the operator's spots into the table instead of losing them.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::control::AttackSpot;
use crate::error::{CoreError, CoreResult};

/// Name of the legacy spot file inside the data root, read once at migration.
pub(crate) const SPOT_FILE_NAME: &str = "zeus-spots.txt";

/// One past the last map id the mod can route to, matching its `int[136][]` adjacency table.
const MAP_COUNT: u16 = 136;

/// Longest spot name accepted, in characters.
///
/// Bounded so one pathological name cannot push the file past its own ceiling, and so the picker has a
/// width it can rely on.
pub const MAX_SPOT_NAME: usize = 48;

/// Most spots one map may hold.
///
/// A limit rather than none: the picker lists them, and a map with hundreds would be a list nobody can
/// use. Generous enough that no honest map reaches it.
pub const MAX_SPOTS_PER_MAP: usize = 32;

/// Most spots the whole book may hold, so the file cannot grow without bound.
pub const MAX_SPOTS_TOTAL: usize = 256;

/// Hard ceiling on the file, so a corrupt or hostile file cannot be read into memory unbounded.
const MAX_SPOT_BYTES: u64 = 128 * 1024;

/// Field separator. Absent from names by validation, so a name can never split its own line.
const FIELD: char = '|';

/// One saved place to stand, with the name the operator gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSpot {
    /// Where it is. The map inside is the map it is filed under.
    pub spot: AttackSpot,
    /// What the operator calls it. Unique within its map, non-empty, trimmed.
    pub name: String,
}

/// Every saved spot, in file order.
///
/// A flat list rather than a map keyed by map id: the picker shows them in one list, insertion order is
/// what the operator saw last, and grouping is a presentation choice the UI makes rather than a shape
/// the storage imposes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpotBook {
    spots: Vec<SavedSpot>,
}

impl SpotBook {
    /// Every saved spot, in the order the file holds them.
    pub fn spots(&self) -> &[SavedSpot] {
        &self.spots
    }

    /// Every spot saved for one map, in file order.
    pub fn for_map(&self, map_id: u16) -> Vec<&SavedSpot> {
        self.spots
            .iter()
            .filter(|saved| saved.spot.map_id == map_id)
            .collect()
    }

    /// The spot with this name on this map, or `None`.
    ///
    /// Name and map together are the identity: two maps may both hold a "Bãi trên", and they are
    /// different places.
    pub fn find(&self, map_id: u16, name: &str) -> Option<&SavedSpot> {
        self.spots
            .iter()
            .find(|saved| saved.spot.map_id == map_id && saved.name == name)
    }

    /// How many spots the book holds.
    pub fn len(&self) -> usize {
        self.spots.len()
    }

    /// Whether nothing has been saved yet.
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }

    /// Saves one spot, replacing any spot of the same name on the same map.
    ///
    /// Replacing by name rather than appending is what makes a name mean one place: saving "Bãi trên"
    /// again moves it, which is what the operator means when they stand somewhere better and save under
    /// the name they already use.
    pub fn set(&mut self, spot: AttackSpot, name: &str) -> CoreResult<()> {
        let name = validate_name(name)?;
        validate_spot(&spot)?;
        if let Some(existing) = self
            .spots
            .iter_mut()
            .find(|saved| saved.spot.map_id == spot.map_id && saved.name == name)
        {
            existing.spot = spot;
            return Ok(());
        }
        if self.for_map(spot.map_id).len() >= MAX_SPOTS_PER_MAP {
            return Err(spot_error("spot_map_is_full"));
        }
        if self.spots.len() >= MAX_SPOTS_TOTAL {
            return Err(spot_error("spot_book_is_full"));
        }
        self.spots.push(SavedSpot { spot, name });
        Ok(())
    }

    /// A name not yet used on this map, derived from `wanted`.
    ///
    /// Suffixes rather than refuses: the caller is usually a generated suggestion, and failing a save
    /// because two clusters produced the same label would make the operator rename by hand for no
    /// reason. Returns `wanted` untouched when it is free.
    pub fn free_name(&self, map_id: u16, wanted: &str) -> String {
        let wanted = wanted.trim();
        let wanted = if wanted.is_empty() { "Bãi" } else { wanted };
        if self.find(map_id, wanted).is_none() {
            return wanted.to_owned();
        }
        for suffix in 2..=MAX_SPOTS_PER_MAP + 1 {
            let candidate = format!("{wanted} {suffix}");
            if candidate.chars().count() <= MAX_SPOT_NAME && self.find(map_id, &candidate).is_none()
            {
                return candidate;
            }
        }
        wanted.to_owned()
    }
}

/// Path of the legacy spot file, read once so an upgrade keeps what the operator saved.
pub(crate) fn spot_path(data_root: &Path) -> PathBuf {
    data_root.join(SPOT_FILE_NAME)
}

/// Reads every spot the database holds, in insertion order.
pub(crate) fn read_spots(connection: &Connection) -> CoreResult<SpotBook> {
    let mut statement = connection
        .prepare("SELECT map_id, name, zone, pixel_x, pixel_y FROM spots ORDER BY rowid")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut book = SpotBook::default();
    for row in rows {
        let (map_id, name, zone, pixel_x, pixel_y) = row?;
        // The table's own CHECKs already bound these, so a value outside them means the file was
        // edited underneath: refused rather than clamped, for the same reason the parser refuses.
        book.set(
            AttackSpot {
                map_id: u16::try_from(map_id).map_err(|_| spot_error("spot_map_out_of_range"))?,
                zone: i16::try_from(zone).map_err(|_| spot_error("spot_zone_out_of_range"))?,
                pixel_x: i32::try_from(pixel_x).map_err(|_| spot_error("spot_position_unknown"))?,
                pixel_y: i32::try_from(pixel_y).map_err(|_| spot_error("spot_position_unknown"))?,
            },
            &name,
        )?;
    }
    Ok(book)
}

/// Saves one named spot, replacing any spot of the same name on the same map.
///
/// One statement, not read-modify-write: the primary key is `(map_id, name)`, so the upsert IS the
/// "saving the same name moves it" rule rather than a second implementation of it.
pub(crate) fn save_spot(
    connection: &Connection,
    spot: AttackSpot,
    name: &str,
    now_unix_ms: i64,
) -> CoreResult<()> {
    let name = validate_name(name)?;
    validate_spot(&spot)?;
    let existing: i64 = connection.query_row(
        "SELECT COUNT(*) FROM spots WHERE map_id = ?1 AND name <> ?2",
        params![i64::from(spot.map_id), &name],
        |row| row.get(0),
    )?;
    if existing >= MAX_SPOTS_PER_MAP as i64 {
        return Err(spot_error("spot_map_is_full"));
    }
    let total: i64 = connection.query_row(
        "SELECT COUNT(*) FROM spots WHERE NOT (map_id = ?1 AND name = ?2)",
        params![i64::from(spot.map_id), &name],
        |row| row.get(0),
    )?;
    if total >= MAX_SPOTS_TOTAL as i64 {
        return Err(spot_error("spot_book_is_full"));
    }
    connection.execute(
        "INSERT INTO spots (map_id, name, zone, pixel_x, pixel_y, created_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (map_id, name) DO UPDATE SET \
             zone = excluded.zone, pixel_x = excluded.pixel_x, pixel_y = excluded.pixel_y",
        params![
            i64::from(spot.map_id),
            &name,
            i64::from(spot.zone),
            i64::from(spot.pixel_x),
            i64::from(spot.pixel_y),
            now_unix_ms,
        ],
    )?;
    Ok(())
}

/// Forgets one named spot. `true` when there was one to forget.
pub(crate) fn clear_spot(connection: &Connection, map_id: u16, name: &str) -> CoreResult<bool> {
    let removed = connection.execute(
        "DELETE FROM spots WHERE map_id = ?1 AND name = ?2",
        params![i64::from(map_id), name],
    )?;
    Ok(removed > 0)
}

/// Adopts the legacy text file into the table, once, if it is there and the table is empty.
///
/// Guarded on emptiness rather than on the file's absence: a second adoption would resurrect spots the
/// operator deleted after the upgrade, which is worse than not adopting at all. The file is left where
/// it is — deleting the operator's own data is not this function's business.
pub(crate) fn adopt_legacy_file(
    connection: &Connection,
    data_root: &Path,
    now_unix_ms: i64,
) -> CoreResult<usize> {
    let already: i64 = connection.query_row("SELECT COUNT(*) FROM spots", [], |row| row.get(0))?;
    if already > 0 {
        return Ok(0);
    }
    let path = spot_path(data_root);
    // Size-checked before reading: the file is the operator's to edit, and a hand-edited or corrupt one
    // should be refused rather than read wholesale into memory.
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_SPOT_BYTES => {
            return Err(spot_error("spot_file_too_large"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(CoreError::io("inspect legacy saved spots", error)),
    }
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(CoreError::io("read legacy saved spots", error)),
    };
    let book = parse_spots(&body)?;
    for saved in book.spots() {
        save_spot(connection, saved.spot, &saved.name, now_unix_ms)?;
    }
    Ok(book.len())
}

/// Parses the file, refusing the whole thing the moment one line is wrong.
///
/// All-or-nothing for the same reason the control parser is: a half-read book would offer the operator
/// a spot list missing entries they saved, with nothing to say which.
fn parse_spots(body: &str) -> CoreResult<SpotBook> {
    let mut book = SpotBook::default();
    for line in body.lines() {
        let line = line.trim_end_matches(['\r']);
        if line.trim().is_empty() {
            continue;
        }
        // A line from the one-spot-per-map format this replaced: `<map>=<zone>,<x>,<y>`, with no name.
        // Read rather than refused, because refusing it means the operator's saved spots vanish on
        // upgrade and the file they came from reports a parse error instead of saying so.
        if !line.contains(FIELD) {
            let (map, position) = line
                .split_once('=')
                .ok_or_else(|| spot_error("spot_line_has_no_separator"))?;
            let map_id: u16 = map
                .trim()
                .parse()
                .map_err(|_| spot_error("spot_map_not_a_number"))?;
            let mut numbers = position.split(',');
            let zone = next_number::<i16>(&mut numbers)?;
            let pixel_x = next_number::<i32>(&mut numbers)?;
            let pixel_y = next_number::<i32>(&mut numbers)?;
            if numbers.next().is_some() {
                return Err(spot_error("spot_position_has_extra_fields"));
            }
            // Named after where it is: the old format carried no name, and the coordinates are the one
            // thing that certainly distinguishes it from another spot on the same map.
            let name = format!("Bãi {pixel_x},{pixel_y}");
            book.set(
                AttackSpot {
                    map_id,
                    zone,
                    pixel_x,
                    pixel_y,
                },
                &name,
            )?;
            continue;
        }
        let mut fields = line.split(FIELD);
        let map_id: u16 = next_field(&mut fields)?
            .trim()
            .parse()
            .map_err(|_| spot_error("spot_map_not_a_number"))?;
        let name = next_field(&mut fields)?;
        let position = next_field(&mut fields)?;
        if fields.next().is_some() {
            return Err(spot_error("spot_line_has_extra_fields"));
        }
        let mut numbers = position.split(',');
        let zone = next_number::<i16>(&mut numbers)?;
        let pixel_x = next_number::<i32>(&mut numbers)?;
        let pixel_y = next_number::<i32>(&mut numbers)?;
        if numbers.next().is_some() {
            return Err(spot_error("spot_position_has_extra_fields"));
        }
        if book.find(map_id, name.trim()).is_some() {
            return Err(spot_error("spot_name_listed_twice"));
        }
        book.set(
            AttackSpot {
                map_id,
                zone,
                pixel_x,
                pixel_y,
            },
            name,
        )?;
    }
    Ok(book)
}

/// Accepts a name the operator can tell apart later, and that cannot break its own line.
///
/// `|` and newlines are refused rather than escaped: escaping would need a reader that unescapes, and
/// no name worth having contains either.
fn validate_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(spot_error("spot_name_is_empty"));
    }
    if name.chars().count() > MAX_SPOT_NAME {
        return Err(spot_error("spot_name_too_long"));
    }
    if name.contains(FIELD) || name.contains('\n') || name.contains('\r') {
        return Err(spot_error("spot_name_has_a_separator"));
    }
    if name.chars().any(char::is_control) {
        return Err(spot_error("spot_name_has_a_control_character"));
    }
    Ok(name.to_owned())
}

fn validate_spot(spot: &AttackSpot) -> CoreResult<()> {
    if spot.map_id >= MAP_COUNT {
        return Err(spot_error("spot_map_out_of_range"));
    }
    if spot.pixel_x < 0 || spot.pixel_y < 0 {
        return Err(spot_error("spot_position_unknown"));
    }
    if spot.zone < -1 || spot.zone > 127 {
        return Err(spot_error("spot_zone_out_of_range"));
    }
    Ok(())
}

fn next_field<'a>(fields: &mut std::str::Split<'a, char>) -> CoreResult<&'a str> {
    fields
        .next()
        .ok_or_else(|| spot_error("spot_line_is_short"))
}

fn next_number<T: std::str::FromStr>(fields: &mut std::str::Split<'_, char>) -> CoreResult<T> {
    fields
        .next()
        .ok_or_else(|| spot_error("spot_position_is_short"))?
        .trim()
        .parse()
        .map_err(|_| spot_error("spot_field_not_a_number"))
}

fn spot_error(code: &'static str) -> CoreError {
    CoreError::SavedSpots { code }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        MAX_SPOT_NAME, SpotBook, adopt_legacy_file, clear_spot, parse_spots, read_spots, save_spot,
    };
    use crate::control::AttackSpot;
    use crate::store::schema::SPOTS_V3_SQL;

    /// A spot table with no data root behind it, so a store test needs no temporary directory.
    fn table() -> Connection {
        let connection = Connection::open_in_memory().expect("an in-memory database");
        connection
            .execute_batch(SPOTS_V3_SQL)
            .expect("the spots table");
        connection
    }

    fn spot(map_id: u16, zone: i16, pixel_x: i32, pixel_y: i32) -> AttackSpot {
        AttackSpot {
            map_id,
            zone,
            pixel_x,
            pixel_y,
        }
    }

    #[test]
    fn one_map_holds_several_named_spots() {
        // The shape this replaced allowed one spot per map, which made a map's second worthwhile place
        // to stand unreachable.
        let mut book = SpotBook::default();
        book.set(spot(7, 0, 100, 200), "Bãi trên")
            .expect("a valid spot");
        book.set(spot(7, 0, 900, 800), "Bãi dưới")
            .expect("a valid spot");
        book.set(spot(8, 1, 50, 60), "Bãi trên")
            .expect("a name repeats across maps");
        assert_eq!(book.len(), 3);
        assert_eq!(book.for_map(7).len(), 2);
        // Name and map together are the identity: the same name on another map is another place.
        assert_eq!(
            book.find(8, "Bãi trên").map(|saved| saved.spot.pixel_x),
            Some(50)
        );
    }

    #[test]
    fn saving_a_name_again_moves_it_rather_than_adding_a_second() {
        let mut book = SpotBook::default();
        book.set(spot(7, 0, 100, 200), "Bãi trên")
            .expect("a valid spot");
        book.set(spot(7, 1, 300, 400), "Bãi trên")
            .expect("a valid spot");
        assert_eq!(book.for_map(7).len(), 1);
        let saved = book.find(7, "Bãi trên").expect("the moved spot");
        assert_eq!((saved.spot.pixel_x, saved.spot.pixel_y), (300, 400));
        assert_eq!(saved.spot.zone, 1);
    }

    #[test]
    fn a_suggested_name_is_suffixed_rather_than_refused() {
        // The caller is usually a generated label, and two clusters can honestly produce the same one.
        let mut book = SpotBook::default();
        assert_eq!(book.free_name(7, "Sói Xám"), "Sói Xám");
        book.set(spot(7, 0, 1, 2), "Sói Xám").expect("a valid spot");
        assert_eq!(book.free_name(7, "Sói Xám"), "Sói Xám 2");
        book.set(spot(7, 0, 3, 4), "Sói Xám 2")
            .expect("a valid spot");
        assert_eq!(book.free_name(7, "Sói Xám"), "Sói Xám 3");
        // Another map is untouched by either.
        assert_eq!(book.free_name(8, "Sói Xám"), "Sói Xám");
        // An empty suggestion still yields something usable rather than an empty name.
        assert_eq!(book.free_name(9, "   "), "Bãi");
    }

    #[test]
    fn a_book_survives_a_round_trip_through_the_table() {
        let connection = table();
        // Nothing saved yet is an empty book, not an error: it is the state every install starts in.
        assert!(
            read_spots(&connection)
                .expect("an empty table reads as empty")
                .is_empty()
        );

        save_spot(&connection, spot(1, 0, 480, 720), "Sói Xám lv12", 1).expect("a valid spot");
        save_spot(&connection, spot(1, 0, 96, 144), "Bãi cát", 2).expect("a valid spot");
        save_spot(&connection, spot(7, -1, 12, 24), "Chưa rõ khu", 3)
            .expect("a zone of -1 is unknown, not invalid");

        let read = read_spots(&connection).expect("the book reads back");
        assert_eq!(read.len(), 3);
        assert_eq!(read.for_map(1).len(), 2);
        let saved = read.find(7, "Chưa rõ khu").expect("the spot with no zone");
        assert_eq!(saved.spot.zone, -1);
        assert_eq!((saved.spot.pixel_x, saved.spot.pixel_y), (12, 24));
    }

    #[test]
    fn saving_a_name_again_in_the_table_moves_it_rather_than_adding_a_second() {
        // The operator stands somewhere better and saves under the name they already use. Two spots of
        // one name would leave the list with a name that means two places. Enforced by the primary key
        // here, separately from SpotBook's own rule above.
        let connection = table();
        save_spot(&connection, spot(1, 0, 480, 720), "Bãi sói", 1).expect("a valid spot");
        save_spot(&connection, spot(1, 2, 96, 144), "Bãi sói", 2).expect("the same name again");

        let read = read_spots(&connection).expect("the book reads back");
        assert_eq!(read.len(), 1);
        let saved = read.find(1, "Bãi sói").expect("the moved spot");
        assert_eq!(saved.spot.zone, 2);
        assert_eq!((saved.spot.pixel_x, saved.spot.pixel_y), (96, 144));
        // The same name on another map is a different spot, not a move.
        save_spot(&connection, spot(2, 0, 1, 2), "Bãi sói", 3).expect("another map");
        assert_eq!(read_spots(&connection).expect("reads back").len(), 2);
    }

    #[test]
    fn forgetting_a_spot_reports_whether_there_was_one() {
        let connection = table();
        save_spot(&connection, spot(1, 0, 480, 720), "Bãi sói", 1).expect("a valid spot");
        assert!(clear_spot(&connection, 1, "Bãi sói").expect("the delete runs"));
        assert!(!clear_spot(&connection, 1, "Bãi sói").expect("the delete runs"));
        assert!(read_spots(&connection).expect("reads back").is_empty());
    }

    #[test]
    fn the_legacy_file_is_adopted_once_and_not_resurrected() {
        // Spots saved before the table existed must survive the upgrade. Adopting twice would bring back
        // spots the operator deleted afterwards, which is worse than never adopting.
        let root = std::env::temp_dir().join(format!("zeus-spots-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("a temporary directory");
        std::fs::write(
            super::spot_path(&root),
            "1|Sói Xám|0,480,720\n9=5,1248,864\n",
        )
        .expect("a legacy file");

        let connection = table();
        assert_eq!(
            adopt_legacy_file(&connection, &root, 1).expect("the file is adopted"),
            2
        );
        assert!(
            read_spots(&connection)
                .expect("reads back")
                .find(1, "Sói Xám")
                .is_some()
        );

        // Deleted after the upgrade, then adopted again: it stays deleted.
        assert!(clear_spot(&connection, 1, "Sói Xám").expect("the delete runs"));
        assert_eq!(
            adopt_legacy_file(&connection, &root, 2).expect("a second adoption is a no-op"),
            0
        );
        assert!(
            read_spots(&connection)
                .expect("reads back")
                .find(1, "Sói Xám")
                .is_none()
        );

        // A data root with no legacy file at all is not an error.
        let empty = std::env::temp_dir().join(format!("zeus-spots-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&empty).expect("a temporary directory");
        let fresh = table();
        assert_eq!(
            adopt_legacy_file(&fresh, &empty, 1).expect("a missing file adopts nothing"),
            0
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn spots_saved_by_the_one_per_map_format_are_still_read() {
        // The format this replaced carried no name. Refusing it would make the operator's saved spots
        // vanish on upgrade, reported as a parse error rather than as the loss it is.
        let book = parse_spots("9=5,1248,864\n25=3,672,60\n").expect("the old format reads");
        assert_eq!(book.len(), 2);
        let saved = book
            .find(9, "Bãi 1248,864")
            .expect("named after where it is");
        assert_eq!(saved.spot.zone, 5);
        assert_eq!((saved.spot.pixel_x, saved.spot.pixel_y), (1248, 864));
        // And a file holding both shapes at once, which is what one save after an upgrade produces.
        let mixed = parse_spots("9=5,1248,864\n9|Bãi trên|5,10,20\n").expect("both shapes read");
        assert_eq!(mixed.for_map(9).len(), 2);
        assert!(mixed.find(9, "Bãi trên").is_some());
    }

    #[test]
    fn a_malformed_file_is_refused_whole_rather_than_half_read() {
        // Half a book would offer a spot list missing entries the operator saved, with nothing to say
        // which — worse than refusing and reporting it.
        let long = "x".repeat(MAX_SPOT_NAME + 1);
        for (case, body) in [
            ("no separator", "1 Bãi 0,480,720\n".to_owned()),
            ("map is not a number", "x|Bãi|0,480,720\n".to_owned()),
            ("no name field", "1|0,480,720\n".to_owned()),
            ("an empty name", "1||0,480,720\n".to_owned()),
            ("a name too long", format!("1|{long}|0,480,720\n")),
            ("a field missing", "1|Bãi|0,480\n".to_owned()),
            ("an extra field", "1|Bãi|0,480,720|9\n".to_owned()),
            ("an extra position", "1|Bãi|0,480,720,9\n".to_owned()),
            (
                "the same name twice",
                "1|Bãi|0,480,720\n1|Bãi|1,10,20\n".to_owned(),
            ),
            (
                "a map past the routable ids",
                "136|Bãi|0,480,720\n".to_owned(),
            ),
            ("an unknown position", "1|Bãi|0,-1,720\n".to_owned()),
        ] {
            assert!(parse_spots(&body).is_err(), "{case} was accepted");
        }
        // Blank lines are not malformed: a trailing newline is ordinary.
        assert_eq!(
            parse_spots("1|Bãi|0,480,720\n\n")
                .expect("blank lines are skipped")
                .len(),
            1
        );
    }
}

//! Seeds the MicroEmulator record stores the KO402 MIDlet reads at startup.
//!
//! The client already contains an auto-login path: `bs.c()` reads the `user_pass` store while it
//! builds the login screen and, when the record exists, fills both text boxes and submits opcode 1
//! itself. Writing the two stores before the JVM starts therefore logs the account in through the
//! client's own code, with no synthesized packet, no click, and no dependence on window geometry.
//!
//! Two file formats are reproduced here, both verified against stores the real runtime wrote:
//!
//! - The container is `RecordStoreImpl.write(DataOutputStream)`: `writeUTF(name)`, `writeInt`
//!   version, `writeLong` last-modified, `writeInt` next-record ID, then `writeInt` ID, `writeInt`
//!   length and the bytes for each record.
//! - Every payload is bitwise complemented, because `com.silverknight.TemMidlet` complements on
//!   write and again on read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::data_root;
use crate::error::{CoreError, CoreResult};

/// Profile child directory passed to the JVM as `user.home`.
///
/// Named here because the seeder and the launch spec must agree: the stores are written into this
/// directory before the process starts, and the JVM then resolves them from the same path.
pub(crate) const MICROEMU_HOME_DIRECTORY: &str = "microemu-home";

/// Directory MicroEmulator keeps its per-suite stores in, relative to `user.home`.
const CONFIG_DIRECTORY_NAME: &str = ".microemulator";

/// Suite folder name for a class-name launch.
///
/// `FileRecordStoreManager.getSuiteFolder` builds `getConfigPath()/suite-<suiteName>`, and
/// `Launcher.midletSuiteName` is only assigned when a JAD supplies it. The pinned launch spec starts
/// the MIDlet by class name, so the name stays null and Java renders it as the literal `null`. This
/// matches the folder the real runtime created.
const SUITE_DIRECTORY_NAME: &str = "suite-null";

/// Store the login screen reads the saved credentials from, written by `bs.g()`.
const USER_PASS_STORE: &str = "user_pass";

/// Store the connection code reads the server index from, written by `bs.h()`.
pub(crate) const INDEX_SERVER_STORE: &str = "isIndexServer";

/// Store the official v4.0.3 LogoScreen reads for server re-identification by host, written by `LogoScreen.saveSelectedServer()`.
pub(crate) const SELECTED_SERVER_HOST_STORE: &str = "selectedServerHost";

/// Store the official v4.0.3 LogoScreen reads for offline server table cache, written by `LogoScreen.saveListServer()`.
pub(crate) const LIST_SERVER_STORE: &str = "listServer";

/// Number of stable logical server entries, bounding valid logical IDs (0..=8).
pub const SERVER_COUNT: u8 = 9;

/// Specification of one stable logical server world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSpec {
    pub logical_id: u8,
    pub name: &'static str,
    pub host: &'static str,
    pub port: u16,
    pub lang: u8,
}

/// Dense stable logical server catalog (0..=8).
///
/// Persistent logical IDs 0..7 preserve their historical meanings and world assignments.
/// Bạch Hổ New is assigned stable logical ID 8 (current publisher live index 0).
pub const SERVER_CATALOG: [ServerSpec; SERVER_COUNT as usize] = [
    ServerSpec {
        logical_id: 0,
        name: "Chiến Thần",
        host: "hs1.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 1,
        name: "Rồng Lửa",
        host: "hs2.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 2,
        name: "Global Server",
        host: "hsglobal.teamobi.com",
        port: 19129,
        lang: 1,
    },
    ServerSpec {
        logical_id: 3,
        name: "Phượng Hoàng",
        host: "hs3.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 4,
        name: "Nhân Mã",
        host: "hs5.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 5,
        name: "Kì Lân",
        host: "hs6.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 6,
        name: "Thiên Hà (New)",
        host: "hs7.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 7,
        name: "Thách Đấu",
        host: "hs4.teamobi.com",
        port: 19129,
        lang: 0,
    },
    ServerSpec {
        logical_id: 8,
        name: "Bạch Hổ New",
        host: "hs8.teamobi.com",
        port: 19129,
        lang: 0,
    },
];

/// Display names of the client's stable logical catalog, in logical ID order.
pub const SERVER_NAMES: [&str; SERVER_COUNT as usize] = [
    SERVER_CATALOG[0].name,
    SERVER_CATALOG[1].name,
    SERVER_CATALOG[2].name,
    SERVER_CATALOG[3].name,
    SERVER_CATALOG[4].name,
    SERVER_CATALOG[5].name,
    SERVER_CATALOG[6].name,
    SERVER_CATALOG[7].name,
    SERVER_CATALOG[8].name,
];

/// Returns the stable `ServerSpec` for a logical server ID, or `None` if out of bounds.
pub fn server_spec(logical_id: u8) -> Option<&'static ServerSpec> {
    SERVER_CATALOG.get(logical_id as usize)
}

/// Official captured 9-server list payload used as RMS bootstrap cache.
pub const BOOTSTRAP_LIST_SERVER_RAW: &str =
    "Bạch Hổ New:hs8.teamobi.com:19129:0,Chiến Thần:hs1.teamobi.com:19129:0,Rồng Lửa:hs2.teamobi.com:19129:0,Global Server:hsglobal.teamobi.com:19129:1,Phượng Hoàng:hs3.teamobi.com:19129:0,Nhân Mã:hs5.teamobi.com:19129:0,Kì Lân:hs6.teamobi.com:19129:0,Thiên Hà:hs7.teamobi.com:19129:0,Thách Đấu:hs4.teamobi.com:19129:0,";

/// Extension `FileRecordStoreManager.recordStoreName2FileName` appends.
const STORE_EXTENSION: &str = "rs";

/// Record-store format version. The reader accepts any value; the runtime writes small integers.
const STORE_VERSION: i32 = 1;

fn seed_error(code: &'static str) -> CoreError {
    CoreError::RmsSeed { code }
}

/// Complements every byte, matching `TemMidlet`'s obfuscation on both read and write.
///
/// The transform is its own inverse, which is what lets the same helper serve both directions.
fn complement(data: &[u8]) -> Vec<u8> {
    data.iter().map(|byte| !byte).collect()
}

/// Encodes one string exactly as `DataOutput.writeUTF`: a `u16` byte count, then modified UTF-8.
///
/// Modified UTF-8 is not standard UTF-8. A NUL is written as two bytes so it cannot terminate the
/// run, and a supplementary character is written as its two surrogates in three bytes each rather
/// than as one four-byte sequence. ASCII credentials encode identically under both, but the MIDlet
/// reads with `DataInput.readUTF`, so the exact encoding is reproduced.
fn write_java_utf8(text: &str, out: &mut Vec<u8>) -> CoreResult<()> {
    let mut encoded = Vec::new();
    for character in text.chars() {
        let code = character as u32;
        match code {
            0 => encoded.extend_from_slice(&[0xc0, 0x80]),
            0x1..=0x7f => encoded.push(code as u8),
            0x80..=0x7ff => {
                encoded.push(0xc0 | (code >> 6) as u8);
                encoded.push(0x80 | (code & 0x3f) as u8);
            }
            0x800..=0xffff => {
                encoded.push(0xe0 | (code >> 12) as u8);
                encoded.push(0x80 | ((code >> 6) & 0x3f) as u8);
                encoded.push(0x80 | (code & 0x3f) as u8);
            }
            _ => {
                // Supplementary planes travel as a surrogate pair, three bytes each.
                let offset = code - 0x1_0000;
                let high = 0xd800 + (offset >> 10);
                let low = 0xdc00 + (offset & 0x3ff);
                for surrogate in [high, low] {
                    encoded.push(0xe0 | (surrogate >> 12) as u8);
                    encoded.push(0x80 | ((surrogate >> 6) & 0x3f) as u8);
                    encoded.push(0x80 | (surrogate & 0x3f) as u8);
                }
            }
        }
    }
    let length = u16::try_from(encoded.len()).map_err(|_| seed_error("rms_utf8_too_long"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&encoded);
    Ok(())
}

/// Serializes one record store, big-endian, in `RecordStoreImpl.write` order.
fn record_store_bytes(name: &str, records: &[Vec<u8>], modified_ms: i64) -> CoreResult<Vec<u8>> {
    let mut out = Vec::new();
    write_java_utf8(name, &mut out)?;
    out.extend_from_slice(&STORE_VERSION.to_be_bytes());
    out.extend_from_slice(&modified_ms.to_be_bytes());
    // IDs are handed out from 1, so the next free ID is one past the last record.
    let next_record_id =
        i32::try_from(records.len() + 1).map_err(|_| seed_error("rms_record_count"))?;
    out.extend_from_slice(&next_record_id.to_be_bytes());
    for (index, payload) in records.iter().enumerate() {
        let record_id = i32::try_from(index + 1).map_err(|_| seed_error("rms_record_count"))?;
        let length = i32::try_from(payload.len()).map_err(|_| seed_error("rms_record_too_long"))?;
        out.extend_from_slice(&record_id.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(payload);
    }
    Ok(out)
}

/// Milliseconds since the Unix epoch, clamped at the epoch so a backdated clock cannot go negative.
fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The `user_pass` record: `writeUTF(username)`, `writeUTF(password)`, complemented.
///
/// Mirrors `bs.g()`, which is what the client itself stores when the operator saves credentials.
fn user_pass_record(username: &str, password: &str) -> CoreResult<Vec<u8>> {
    let mut plain = Vec::new();
    write_java_utf8(username, &mut plain)?;
    write_java_utf8(password, &mut plain)?;
    Ok(complement(&plain))
}

/// Returns the safe client bootstrap index for a logical server ID.
///
/// IDs 0..7 retain their historical static-fallback index (0..7).
/// Logical ID 8 (Bạch Hổ) returns bootstrap index 0 to stay within the official
/// static 8-server array bounds (0..7) before dynamic host resolution.
pub fn bootstrap_index_for_server(logical_id: u8) -> CoreResult<u8> {
    if logical_id >= SERVER_COUNT {
        return Err(seed_error("rms_server_index_out_of_range"));
    }
    match logical_id {
        0..=7 => Ok(logical_id),
        8 => Ok(0),
        _ => Err(seed_error("rms_server_index_out_of_range")),
    }
}

/// The `isIndexServer` record: one bootstrap index byte, complemented.
/// For legacy IDs 0..7, this is the historical index.
/// For Bạch Hổ ID 8, this is bootstrap index 0 to keep the client within valid array bounds before dynamic host resolution.
fn index_server_record(logical_id: u8) -> CoreResult<Vec<u8>> {
    let bootstrap_index = bootstrap_index_for_server(logical_id)?;
    Ok(complement(&[bootstrap_index]))
}

/// The `selectedServerHost` record: raw UTF-8 bytes of canonical host name, complemented.
/// Mirrors `LogoScreen.saveSelectedServer()` / `CRes.saveRMS("selectedServerHost", host.getBytes("UTF-8"))`.
pub fn selected_server_host_record(host: &str) -> CoreResult<Vec<u8>> {
    if host.is_empty() {
        return Err(seed_error("rms_host_empty"));
    }
    Ok(complement(host.as_bytes()))
}

/// Decodes a `selectedServerHost` record back to canonical host string.
pub fn parse_selected_server_host_record(record: &[u8]) -> CoreResult<String> {
    let plain = complement(record);
    String::from_utf8(plain).map_err(|_| seed_error("rms_host_invalid_utf8"))
}

/// The `listServer` record: `writeUTF(raw_list)`, complemented.
/// Mirrors `LogoScreen.saveListServer(raw_list)` / `CRes.saveRMS("listServer", byteArrayOutputStream.toByteArray())`.
pub fn list_server_record(raw_list: &str) -> CoreResult<Vec<u8>> {
    let mut plain = Vec::new();
    write_java_utf8(raw_list, &mut plain)?;
    Ok(complement(&plain))
}

/// Decodes a `listServer` record back to raw list string.
pub fn parse_list_server_record(record: &[u8]) -> CoreResult<String> {
    let plain = complement(record);
    if plain.len() < 2 {
        return Err(seed_error("rms_list_server_too_short"));
    }
    let length = u16::from_be_bytes([plain[0], plain[1]]) as usize;
    if plain.len() < 2 + length {
        return Err(seed_error("rms_list_server_length_mismatch"));
    }
    String::from_utf8(plain[2..2 + length].to_vec())
        .map_err(|_| seed_error("rms_list_server_invalid_utf8"))
}

/// Directory the stores for one profile live in.
///
/// Canonical MicroEmulator RMS store directory: `<microemu_home>/.microemulator/suite-null`.
pub fn suite_directory(microemu_home: &Path) -> PathBuf {
    microemu_home
        .join(CONFIG_DIRECTORY_NAME)
        .join(SUITE_DIRECTORY_NAME)
}

/// Writes one store atomically: a uniquely named temporary beside the target, then a replace.
///
/// A torn store is worse than a missing one. A missing `user_pass` only means the client shows its
/// login screen, but a half-written one makes `readUTF` throw inside the MIDlet's startup path.
fn write_record_store(directory: &Path, name: &str, records: &[Vec<u8>]) -> CoreResult<()> {
    let bytes = record_store_bytes(name, records, now_unix_ms())?;
    fs::create_dir_all(directory)
        .map_err(|error| CoreError::io("create record store directory", error))?;
    let destination = directory.join(format!("{name}.{STORE_EXTENSION}"));
    let temporary = directory.join(format!(".{name}.tmp-{}", Uuid::new_v4()));
    if let Err(error) = fs::write(&temporary, &bytes) {
        // Leaving a stray temporary behind would fail the next managed-shape check.
        let _ = fs::remove_file(&temporary);
        return Err(CoreError::io("write record store", error));
    }
    if let Err(error) = data_root::atomic_replace(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// Writes all stores the official v4.0.3 client's server-selection and auto-login paths read.
///
/// Enforces strict fail-closed sequencing:
/// 1. Remove any pre-existing `user_pass` before beginning re-seed.
/// 2. Validate inputs and construct all records in memory first.
/// 3. Seed `listServer` (cached 9-server table for offline resilience).
/// 4. Seed `selectedServerHost` (canonical host for dynamic list matching).
/// 5. Seed `isIndexServer` (legacy ID or sentinel 8).
/// 6. Seed `user_pass` LAST.
///
/// If any server identity store fails to write or inputs are invalid, `user_pass` is guaranteed absent.
pub fn seed_credentials(
    microemu_home: &Path,
    username: &str,
    password: &str,
    server_index: u8,
) -> CoreResult<()> {
    let directory = suite_directory(microemu_home);
    let user_pass_path = directory.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}"));

    // Hardening: Always ensure pre-existing user_pass cannot survive if validation
    // or any subsequent server-identity write fails.
    if user_pass_path.exists() {
        let _ = fs::remove_file(&user_pass_path);
    }

    if username.is_empty() || password.is_empty() {
        return Err(seed_error("rms_credentials_empty"));
    }
    let spec = server_spec(server_index).ok_or_else(|| seed_error("rms_server_index_out_of_range"))?;

    let list_record = list_server_record(BOOTSTRAP_LIST_SERVER_RAW)?;
    let host_record = selected_server_host_record(spec.host)?;
    let index_record = index_server_record(server_index)?;
    let credential_record = user_pass_record(username, password)?;

    // Write server identity stores first. If any write fails, ensure user_pass remains absent.
    if let Err(err) = write_record_store(&directory, LIST_SERVER_STORE, &[list_record]) {
        let _ = fs::remove_file(&user_pass_path);
        return Err(err);
    }
    if let Err(err) = write_record_store(&directory, SELECTED_SERVER_HOST_STORE, &[host_record]) {
        let _ = fs::remove_file(&user_pass_path);
        return Err(err);
    }
    if let Err(err) = write_record_store(&directory, INDEX_SERVER_STORE, &[index_record]) {
        let _ = fs::remove_file(&user_pass_path);
        return Err(err);
    }

    // Write user_pass LAST.
    write_record_store(&directory, USER_PASS_STORE, &[credential_record])
}

/// Removes all seeded stores, so a stopped account leaves no credential or server identity material on disk.
pub fn clear_credentials(microemu_home: &Path) -> CoreResult<()> {
    let directory = suite_directory(microemu_home);
    for name in [
        USER_PASS_STORE,
        INDEX_SERVER_STORE,
        SELECTED_SERVER_HOST_STORE,
        LIST_SERVER_STORE,
    ] {
        let path = directory.join(format!("{name}.{STORE_EXTENSION}"));
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CoreError::io("remove record store", error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BOOTSTRAP_LIST_SERVER_RAW, CONFIG_DIRECTORY_NAME, INDEX_SERVER_STORE, LIST_SERVER_STORE,
        SELECTED_SERVER_HOST_STORE, SERVER_CATALOG, SERVER_COUNT, SERVER_NAMES, STORE_EXTENSION,
        SUITE_DIRECTORY_NAME, USER_PASS_STORE, bootstrap_index_for_server, clear_credentials,
        complement, index_server_record, list_server_record, parse_list_server_record,
        parse_selected_server_host_record, record_store_bytes, seed_credentials,
        selected_server_host_record, server_spec, suite_directory, user_pass_record, write_java_utf8,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    /// A temporary `user.home` that removes itself, so a failed test leaves no seeded credential.
    ///
    /// The workspace pins its dependency set, so this mirrors the guard the other modules' tests use
    /// rather than adding a crate for it.
    struct TestHome(PathBuf);

    impl TestHome {
        fn new(label: &str) -> Self {
            Self(std::env::temp_dir().join(format!("zeus-rms-{label}-{}", Uuid::new_v4())))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            // Guarded so a surprising path can never delete outside the system temp directory.
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    /// Reads a `writeUTF` run, returning the text and the offset just past it.
    fn read_java_utf8(bytes: &[u8], at: usize) -> (String, usize) {
        let length = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        let start = at + 2;
        let text = String::from_utf8(bytes[start..start + length].to_vec())
            .expect("test inputs stay inside the shared UTF-8 subset");
        (text, start + length)
    }

    #[test]
    fn complementing_twice_returns_the_original_bytes() {
        // The MIDlet complements on write and again on read, so the transform must be an involution.
        let original: Vec<u8> = (0..=255u8).collect();
        assert_eq!(complement(&complement(&original)), original);
    }

    #[test]
    fn java_utf8_encodes_nul_and_supplementary_characters_the_java_way() {
        let mut encoded = Vec::new();
        write_java_utf8("a\u{0}b", &mut encoded).expect("a short string encodes");
        // Four bytes, not three: the NUL takes two so it cannot terminate the run.
        assert_eq!(encoded, vec![0, 4, b'a', 0xc0, 0x80, b'b']);

        let mut vietnamese = Vec::new();
        write_java_utf8("Chiến", &mut vietnamese).expect("a Vietnamese name encodes");
        // Standard UTF-8 for the BMP, so the length is the byte count rather than the char count.
        assert_eq!(&vietnamese[0..2], &[0, 7]);
        assert_eq!(&vietnamese[2..], "Chiến".as_bytes());

        let mut supplementary = Vec::new();
        write_java_utf8("\u{1f600}", &mut supplementary).expect("an astral character encodes");
        // Six bytes as a surrogate pair, where standard UTF-8 would emit four.
        assert_eq!(
            supplementary,
            vec![0, 6, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]
        );
    }

    #[test]
    fn a_store_has_the_header_and_record_layout_the_reader_expects() {
        let records = vec![vec![0xaa, 0xbb], vec![0xcc]];
        let bytes = record_store_bytes("user_pass", &records, 0x0123_4567_89ab_cdef)
            .expect("two small records serialize");
        let (name, mut at) = read_java_utf8(&bytes, 0);
        assert_eq!(name, "user_pass");
        assert_eq!(&bytes[at..at + 4], &1i32.to_be_bytes());
        at += 4;
        assert_eq!(&bytes[at..at + 8], &0x0123_4567_89ab_cdefi64.to_be_bytes());
        at += 8;
        // Next free ID is one past the last record, because IDs start at 1.
        assert_eq!(&bytes[at..at + 4], &3i32.to_be_bytes());
        at += 4;
        for (index, payload) in records.iter().enumerate() {
            let expected_id = i32::try_from(index + 1).expect("two records fit in an i32");
            assert_eq!(&bytes[at..at + 4], &expected_id.to_be_bytes());
            at += 4;
            let expected_length =
                i32::try_from(payload.len()).expect("a short payload fits in an i32");
            assert_eq!(&bytes[at..at + 4], &expected_length.to_be_bytes());
            at += 4;
            assert_eq!(&bytes[at..at + payload.len()], payload.as_slice());
            at += payload.len();
        }
        // Nothing trails the last record: the reader loops until it hits the end of the file.
        assert_eq!(at, bytes.len());
    }

    #[test]
    fn the_credential_record_round_trips_through_the_midlet_transform() {
        let record = user_pass_record(SAMPLE_ACCOUNT, "secret").expect("credentials encode");
        let plain = complement(&record);
        let (username, at) = read_java_utf8(&plain, 0);
        let (password, end) = read_java_utf8(&plain, at);
        assert_eq!(username, SAMPLE_ACCOUNT);
        assert_eq!(password, "secret");
        assert_eq!(end, plain.len());
    }

    /// A ten-digit account, matching the shape a real login has without being one.
    ///
    /// Deliberately synthetic: the record below is the credential blob, and the transform protecting
    /// it is a bitwise complement, so pinning a real capture here would publish that account.
    const SAMPLE_ACCOUNT: &str = "0000000000";

    /// The record `bs.g()` produces for [`SAMPLE_ACCOUNT`] as both username and password.
    ///
    /// Byte-for-byte what the game's own writer emits: `writeUTF` twice, then complemented. The header
    /// and framing around it were verified against a store the pinned runtime actually wrote, which is
    /// what makes this layout evidence rather than a guess.
    const SAMPLE_USER_PASS_RECORD: [u8; 24] = [
        0xff, 0xf5, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xff, 0xf5, 0xcf,
        0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf, 0xcf,
    ];

    /// Header of that same real `user_pass.rs`, up to but excluding the first record.
    ///
    /// Its version field reads 4 and its next-record ID reads 2: the game had read a seeded store and
    /// rewritten it, which is what makes these bytes evidence that the client accepts this layout.
    const SAMPLE_USER_PASS_HEADER: [u8; 27] = [
        0x00, 0x09, b'u', b's', b'e', b'r', b'_', b'p', b'a', b's', b's', 0x00, 0x00, 0x00, 0x04,
        0x00, 0x00, 0x01, 0xa0, 0x4c, 0xa0, 0xfa, 0xab, 0x00, 0x00, 0x00, 0x02,
    ];

    #[test]
    fn the_encoder_reproduces_the_bytes_the_real_runtime_wrote() {
        // The strongest available check: not that this encoder agrees with the decoder beside it, but
        // that it agrees with the game. A drift here means the MIDlet would throw inside startup.
        let record = user_pass_record(SAMPLE_ACCOUNT, SAMPLE_ACCOUNT)
            .expect("the sample credentials encode");
        assert_eq!(record, SAMPLE_USER_PASS_RECORD);
        assert_eq!(
            index_server_record(6).expect("the captured server encodes"),
            [0xf9]
        );

        // The container is compared against the same real header. Version and last-modified are the
        // two fields a writer legitimately chooses, so they are spliced to the captured values and
        // everything else must match byte for byte.
        let mut produced =
            record_store_bytes("user_pass", &[record.to_vec()], 0x0000_01a0_4ca0_faab)
                .expect("the store serializes");
        produced[11..15].copy_from_slice(&4i32.to_be_bytes());
        assert_eq!(
            &produced[..SAMPLE_USER_PASS_HEADER.len()],
            &SAMPLE_USER_PASS_HEADER
        );
        // Record framing: ID 1, then the 24-byte length the real file declared.
        let at = SAMPLE_USER_PASS_HEADER.len();
        assert_eq!(&produced[at..at + 4], &1i32.to_be_bytes());
        assert_eq!(&produced[at + 4..at + 8], &24i32.to_be_bytes());
        assert_eq!(&produced[at + 8..], &SAMPLE_USER_PASS_RECORD);
    }

    #[test]
    fn the_server_record_is_one_complemented_index_byte_inside_the_table() {
        // Index 6 is the byte the real runtime stored, and 0xf9 is its complement.
        assert_eq!(
            index_server_record(6).expect("a valid index encodes"),
            [0xf9]
        );
        assert_eq!(
            complement(&index_server_record(0).expect("the first server encodes")),
            [0]
        );
        let last = SERVER_COUNT - 1;
        assert!(index_server_record(last).is_ok());
        // One past the table would index out of bounds inside the client.
        let rejected =
            index_server_record(SERVER_COUNT).expect_err("an index past the table fails");
        assert_eq!(rejected.code(), "rms_server_index_out_of_range");
    }

    #[test]
    fn server_catalog_is_dense_nine_entries_with_unique_hosts_and_historical_integrity() {
        assert_eq!(SERVER_CATALOG.len(), 9);
        assert_eq!(SERVER_COUNT, 9);
        assert_eq!(SERVER_NAMES.len(), 9);

        // Every logical_id equals its stable catalog position.
        for (index, spec) in SERVER_CATALOG.iter().enumerate() {
            assert_eq!(spec.logical_id, index as u8);
            assert_eq!(spec.name, SERVER_NAMES[index]);
            assert!(!spec.name.is_empty());
            assert!(!spec.host.is_empty());
            assert_eq!(spec.port, 19129);
        }

        // IDs 0..7 preserve historical world/host meanings.
        assert_eq!(SERVER_CATALOG[0].name, "Chiến Thần");
        assert_eq!(SERVER_CATALOG[0].host, "hs1.teamobi.com");
        assert_eq!(SERVER_CATALOG[1].name, "Rồng Lửa");
        assert_eq!(SERVER_CATALOG[1].host, "hs2.teamobi.com");
        assert_eq!(SERVER_CATALOG[2].name, "Global Server");
        assert_eq!(SERVER_CATALOG[2].host, "hsglobal.teamobi.com");
        assert_eq!(SERVER_CATALOG[3].name, "Phượng Hoàng");
        assert_eq!(SERVER_CATALOG[3].host, "hs3.teamobi.com");
        assert_eq!(SERVER_CATALOG[4].name, "Nhân Mã");
        assert_eq!(SERVER_CATALOG[4].host, "hs5.teamobi.com");
        assert_eq!(SERVER_CATALOG[5].name, "Kì Lân");
        assert_eq!(SERVER_CATALOG[5].host, "hs6.teamobi.com");
        assert_eq!(SERVER_CATALOG[6].name, "Thiên Hà (New)");
        assert_eq!(SERVER_CATALOG[6].host, "hs7.teamobi.com");
        assert_eq!(SERVER_CATALOG[7].name, "Thách Đấu");
        assert_eq!(SERVER_CATALOG[7].host, "hs4.teamobi.com");

        // ID 8 is Bạch Hổ New / hs8.teamobi.com.
        assert_eq!(SERVER_CATALOG[8].name, "Bạch Hổ New");
        assert_eq!(SERVER_CATALOG[8].host, "hs8.teamobi.com");
        assert_eq!(SERVER_CATALOG[8].lang, 0);

        // All canonical hosts are unique.
        let mut hosts: Vec<&str> = SERVER_CATALOG.iter().map(|s| s.host).collect();
        hosts.sort();
        let before_dedup = hosts.len();
        hosts.dedup();
        assert_eq!(hosts.len(), before_dedup);

        // Logical ID 9 is rejected.
        assert!(server_spec(9).is_none());
        assert!(server_spec(u8::MAX).is_none());
    }

    #[test]
    fn test_bootstrap_index_mapping_and_server_distinguishability() {
        assert_eq!(bootstrap_index_for_server(0).unwrap(), 0);
        assert_eq!(bootstrap_index_for_server(1).unwrap(), 1);
        assert_eq!(bootstrap_index_for_server(2).unwrap(), 2);
        assert_eq!(bootstrap_index_for_server(3).unwrap(), 3);
        assert_eq!(bootstrap_index_for_server(4).unwrap(), 4);
        assert_eq!(bootstrap_index_for_server(5).unwrap(), 5);
        assert_eq!(bootstrap_index_for_server(6).unwrap(), 6);
        assert_eq!(bootstrap_index_for_server(7).unwrap(), 7);
        assert_eq!(bootstrap_index_for_server(8).unwrap(), 0);
        assert!(bootstrap_index_for_server(9).is_err());
        assert!(bootstrap_index_for_server(255).is_err());

        // Bạch Hổ stable account identity remains 8 even though RMS isIndexServer contains 0.
        let rec8 = index_server_record(8).expect("ID 8 encodes");
        assert_eq!(rec8, [!0]);
        assert_eq!(complement(&rec8), [0]);

        // Existing ID 0 and ID 8 remain distinguishable through selectedServerHost.
        let host0 = server_spec(0).unwrap().host;
        let host8 = server_spec(8).unwrap().host;
        assert_ne!(host0, host8);
        assert_eq!(host0, "hs1.teamobi.com");
        assert_eq!(host8, "hs8.teamobi.com");

        let host_rec0 = selected_server_host_record(host0).unwrap();
        let host_rec8 = selected_server_host_record(host8).unwrap();
        assert_ne!(host_rec0, host_rec8);
        assert_eq!(parse_selected_server_host_record(&host_rec0).unwrap(), host0);
        assert_eq!(parse_selected_server_host_record(&host_rec8).unwrap(), host8);
    }

    #[test]
    fn selected_server_host_and_list_server_records_round_trip() {
        assert_eq!(server_spec(0).unwrap().host, "hs1.teamobi.com");
        assert_eq!(server_spec(6).unwrap().host, "hs7.teamobi.com");
        assert_eq!(server_spec(7).unwrap().host, "hs4.teamobi.com");
        assert_eq!(server_spec(8).unwrap().host, "hs8.teamobi.com");

        // selectedServerHost round trip
        for spec in SERVER_CATALOG.iter() {
            let host_rec = selected_server_host_record(spec.host).expect("host encodes");
            let decoded = parse_selected_server_host_record(&host_rec).expect("host decodes");
            assert_eq!(decoded, spec.host);
        }

        // listServer round trip to exact captured 9-server payload
        let list_rec = list_server_record(BOOTSTRAP_LIST_SERVER_RAW).expect("listServer encodes");
        let decoded_list = parse_list_server_record(&list_rec).expect("listServer decodes");
        assert_eq!(decoded_list, BOOTSTRAP_LIST_SERVER_RAW);
    }

    #[test]
    fn seeding_writes_all_four_stores_where_the_emulator_looks_and_leaves_no_temporary() {
        let home = TestHome::new("seed");
        seed_credentials(home.path(), SAMPLE_ACCOUNT, "secret", 6).expect("seeding succeeds");
        let suite = home
            .path()
            .join(CONFIG_DIRECTORY_NAME)
            .join(SUITE_DIRECTORY_NAME);
        let mut names: Vec<String> = fs::read_dir(&suite)
            .expect("the suite directory exists")
            .map(|entry| {
                entry
                    .expect("each entry is readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        // Exactly the four stores: isIndexServer, listServer, selectedServerHost, user_pass.
        assert_eq!(
            names,
            vec![
                "isIndexServer.rs",
                "listServer.rs",
                "selectedServerHost.rs",
                "user_pass.rs"
            ]
        );

        let stored =
            fs::read(suite.join("user_pass.rs")).expect("the credential store is readable");
        let (name, _) = read_java_utf8(&stored, 0);
        assert_eq!(name, "user_pass");
        // The password must never appear in cleartext on disk.
        assert!(!stored.windows(6).any(|window| window == b"secret"));

        // Verify selectedServerHost store content for ID 6
        let host_store = fs::read(suite.join("selectedServerHost.rs")).expect("host store readable");
        assert_eq!(read_java_utf8(&host_store, 0).0, "selectedServerHost");

        // Verify listServer store content
        let list_store = fs::read(suite.join("listServer.rs")).expect("listServer store readable");
        assert_eq!(read_java_utf8(&list_store, 0).0, "listServer");

        // Seeding again replaces rather than duplicating, so a re-run stays at 4 stores.
        seed_credentials(home.path(), SAMPLE_ACCOUNT, "other", 1).expect("re-seeding succeeds");
        assert_eq!(
            fs::read_dir(&suite)
                .expect("the suite directory still exists")
                .count(),
            4
        );

        // Clear removes all four stores.
        clear_credentials(home.path()).expect("clearing succeeds");
        assert_eq!(
            fs::read_dir(&suite)
                .expect("the suite directory survives clearing")
                .count(),
            0
        );
        // Clearing an already-clear profile is not an error, so a second stop cannot fail.
        clear_credentials(home.path()).expect("clearing is idempotent");
    }

    #[test]
    fn empty_credentials_are_refused_before_anything_is_written() {
        let home = TestHome::new("empty-credentials");
        let rejected =
            seed_credentials(home.path(), "", "secret", 0).expect_err("an empty username fails");
        assert_eq!(rejected.code(), "rms_credentials_empty");
        assert!(!home.path().join(CONFIG_DIRECTORY_NAME).exists());
    }

    #[test]
    fn an_out_of_range_server_writes_no_store_at_all() {
        let home = TestHome::new("server-index");
        seed_credentials(home.path(), "user", "secret", SERVER_COUNT)
            .expect_err("an index past the table fails");
        // The index is validated before the first write, so no partial seed survives.
        assert!(!home.path().join(CONFIG_DIRECTORY_NAME).exists());
    }

    #[test]
    fn pre_existing_user_pass_with_forced_list_server_failure_leaves_no_user_pass() {
        let home = TestHome::new("forced-list-fail");
        let suite = suite_directory(home.path());
        fs::create_dir_all(&suite).unwrap();
        // Write pre-existing user_pass
        let user_pass_file = suite.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}"));
        fs::write(&user_pass_file, b"stale_credentials").unwrap();
        assert!(user_pass_file.exists());

        // Force listServer write failure by placing a directory where listServer.rs should be
        let list_server_path = suite.join(format!("{LIST_SERVER_STORE}.{STORE_EXTENSION}"));
        fs::create_dir_all(&list_server_path).unwrap();

        let err = seed_credentials(home.path(), "user", "pass", 8);
        assert!(err.is_err(), "seed_credentials must fail when listServer write fails");
        assert!(!user_pass_file.exists(), "user_pass must be absent after listServer write failure");
    }

    #[test]
    fn pre_existing_user_pass_with_forced_selected_server_host_failure_leaves_no_user_pass() {
        let home = TestHome::new("forced-host-fail");
        let suite = suite_directory(home.path());
        fs::create_dir_all(&suite).unwrap();
        // Write pre-existing user_pass
        let user_pass_file = suite.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}"));
        fs::write(&user_pass_file, b"stale_credentials").unwrap();
        assert!(user_pass_file.exists());

        // Force selectedServerHost write failure by placing a directory where selectedServerHost.rs should be
        let host_path = suite.join(format!("{SELECTED_SERVER_HOST_STORE}.{STORE_EXTENSION}"));
        fs::create_dir_all(&host_path).unwrap();

        let err = seed_credentials(home.path(), "user", "pass", 8);
        assert!(err.is_err(), "seed_credentials must fail when selectedServerHost write fails");
        assert!(!user_pass_file.exists(), "user_pass must be absent after selectedServerHost write failure");
    }

    #[test]
    fn pre_existing_user_pass_with_forced_is_index_server_failure_leaves_no_user_pass() {
        let home = TestHome::new("forced-index-fail");
        let suite = suite_directory(home.path());
        fs::create_dir_all(&suite).unwrap();
        // Write pre-existing user_pass
        let user_pass_file = suite.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}"));
        fs::write(&user_pass_file, b"stale_credentials").unwrap();
        assert!(user_pass_file.exists());

        // Force isIndexServer write failure by placing a directory where isIndexServer.rs should be
        let index_path = suite.join(format!("{INDEX_SERVER_STORE}.{STORE_EXTENSION}"));
        fs::create_dir_all(&index_path).unwrap();

        let err = seed_credentials(home.path(), "user", "pass", 8);
        assert!(err.is_err(), "seed_credentials must fail when isIndexServer write fails");
        assert!(!user_pass_file.exists(), "user_pass must be absent after isIndexServer write failure");
    }

    #[test]
    fn successful_seed_writes_all_four_stores_and_user_pass_last() {
        let home = TestHome::new("seed-success");
        let suite = suite_directory(home.path());
        seed_credentials(home.path(), "newuser", "newpass", 8).expect("successful seed");
        assert!(suite.join(format!("{LIST_SERVER_STORE}.{STORE_EXTENSION}")).exists());
        assert!(suite.join(format!("{SELECTED_SERVER_HOST_STORE}.{STORE_EXTENSION}")).exists());
        assert!(suite.join(format!("{INDEX_SERVER_STORE}.{STORE_EXTENSION}")).exists());
        assert!(suite.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}")).exists());
    }

    #[test]
    fn invalid_logical_server_id_leaves_no_user_pass() {
        let home = TestHome::new("invalid-id-no-pass");
        let suite = suite_directory(home.path());
        fs::create_dir_all(&suite).unwrap();
        // Pre-existing user_pass
        let user_pass_file = suite.join(format!("{USER_PASS_STORE}.{STORE_EXTENSION}"));
        fs::write(&user_pass_file, b"stale_credentials").unwrap();
        assert!(user_pass_file.exists());

        // Seed with invalid logical ID 9
        let err = seed_credentials(home.path(), "user", "pass", 9);
        assert!(err.is_err());
        assert!(!user_pass_file.exists(), "invalid logical server ID must leave no user_pass");

        // Pre-existing user_pass again
        fs::write(&user_pass_file, b"stale_credentials").unwrap();
        assert!(user_pass_file.exists());

        // Seed with invalid logical ID 255
        let err2 = seed_credentials(home.path(), "user", "pass", 255);
        assert!(err2.is_err());
        assert!(!user_pass_file.exists(), "invalid logical server ID 255 must leave no user_pass");
    }
}

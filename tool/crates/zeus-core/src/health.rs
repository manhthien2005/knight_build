//! Health transport foundation for observing Zeus / game-loop progress.

use std::fs;
use std::path::{Path, PathBuf};

/// Exact file name of the health transport sidecar.
pub const HEALTH_FILE_NAME: &str = "zeus-health.txt";

/// Current health wire contract version.
pub const HEALTH_VERSION: u32 = 1;

/// Maximum allowed bytes for the health transport file (1 KiB).
pub const MAX_HEALTH_BYTES: u64 = 1024;

/// Observable screen states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthScreen {
    None,
    Login,
    Server,
    Character,
    World,
    Other,
}

impl HealthScreen {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Login => "login",
            Self::Server => "server",
            Self::Character => "character",
            Self::World => "world",
            Self::Other => "other",
        }
    }
}

/// One health reading published by Zeus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub written_at_unix_ms: i64,
    pub sequence: u64,
    pub screen: HealthScreen,
    pub dialog_open: bool,
    pub native_disconnect: bool,
}

/// Failure modes during health transport reading and parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthError {
    MissingKey(&'static str),
    UnknownKey(String),
    DuplicateKey(String),
    UnsupportedVersion(u32),
    MalformedInteger(&'static str),
    InvalidBoolean(&'static str),
    InvalidScreen(String),
    OversizedFile(u64),
    NotAFile,
    InvalidEncoding,
    Io(String),
}

impl std::fmt::Display for HealthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingKey(key) => write!(f, "missing health key: {key}"),
            Self::UnknownKey(key) => write!(f, "unknown health key: {key}"),
            Self::DuplicateKey(key) => write!(f, "duplicate health key: {key}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported health version: {v}"),
            Self::MalformedInteger(field) => write!(f, "malformed integer for health field: {field}"),
            Self::InvalidBoolean(field) => write!(f, "invalid boolean for health field: {field}"),
            Self::InvalidScreen(screen) => write!(f, "invalid health screen: {screen}"),
            Self::OversizedFile(bytes) => write!(f, "health file exceeds maximum size: {bytes}"),
            Self::NotAFile => write!(f, "health path is not a regular file"),
            Self::InvalidEncoding => write!(f, "health file is not valid UTF-8"),
            Self::Io(msg) => write!(f, "health I/O error: {msg}"),
        }
    }
}

impl std::error::Error for HealthError {}

pub fn health_path(home: &Path) -> PathBuf {
    home.join(HEALTH_FILE_NAME)
}

pub fn parse_health(text: &str) -> Result<HealthSnapshot, HealthError> {
    let mut v: Option<u32> = None;
    let mut t: Option<i64> = None;
    let mut seq: Option<u64> = None;
    let mut screen: Option<HealthScreen> = None;
    let mut dialog: Option<bool> = None;
    let mut disconnect: Option<bool> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| HealthError::UnknownKey(line.to_string()))?;

        match key {
            "v" => {
                if v.is_some() {
                    return Err(HealthError::DuplicateKey("v".to_string()));
                }
                let parsed_v = value
                    .parse::<u32>()
                    .map_err(|_| HealthError::MalformedInteger("v"))?;
                if parsed_v != HEALTH_VERSION {
                    return Err(HealthError::UnsupportedVersion(parsed_v));
                }
                v = Some(parsed_v);
            }
            "t" => {
                if t.is_some() {
                    return Err(HealthError::DuplicateKey("t".to_string()));
                }
                let parsed_t = value
                    .parse::<i64>()
                    .map_err(|_| HealthError::MalformedInteger("t"))?;
                t = Some(parsed_t);
            }
            "seq" => {
                if seq.is_some() {
                    return Err(HealthError::DuplicateKey("seq".to_string()));
                }
                let parsed_seq = value
                    .parse::<u64>()
                    .map_err(|_| HealthError::MalformedInteger("seq"))?;
                seq = Some(parsed_seq);
            }
            "screen" => {
                if screen.is_some() {
                    return Err(HealthError::DuplicateKey("screen".to_string()));
                }
                let parsed_screen = match value {
                    "none" => HealthScreen::None,
                    "login" => HealthScreen::Login,
                    "server" => HealthScreen::Server,
                    "character" => HealthScreen::Character,
                    "world" => HealthScreen::World,
                    "other" => HealthScreen::Other,
                    _ => return Err(HealthError::InvalidScreen(value.to_string())),
                };
                screen = Some(parsed_screen);
            }
            "dialog" => {
                if dialog.is_some() {
                    return Err(HealthError::DuplicateKey("dialog".to_string()));
                }
                let parsed_dialog = match value {
                    "0" => false,
                    "1" => true,
                    _ => return Err(HealthError::InvalidBoolean("dialog")),
                };
                dialog = Some(parsed_dialog);
            }
            "disconnect" => {
                if disconnect.is_some() {
                    return Err(HealthError::DuplicateKey("disconnect".to_string()));
                }
                let parsed_disconnect = match value {
                    "0" => false,
                    "1" => true,
                    _ => return Err(HealthError::InvalidBoolean("disconnect")),
                };
                disconnect = Some(parsed_disconnect);
            }
            unknown => return Err(HealthError::UnknownKey(unknown.to_string())),
        }
    }

    let _v = v.ok_or(HealthError::MissingKey("v"))?;
    let t = t.ok_or(HealthError::MissingKey("t"))?;
    let seq = seq.ok_or(HealthError::MissingKey("seq"))?;
    let screen = screen.ok_or(HealthError::MissingKey("screen"))?;
    let dialog = dialog.ok_or(HealthError::MissingKey("dialog"))?;
    let disconnect = disconnect.ok_or(HealthError::MissingKey("disconnect"))?;

    Ok(HealthSnapshot {
        written_at_unix_ms: t,
        sequence: seq,
        screen,
        dialog_open: dialog,
        native_disconnect: disconnect,
    })
}

pub fn read_health(home: &Path) -> Result<Option<HealthSnapshot>, HealthError> {
    let path = health_path(home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(HealthError::Io(format!("inspect health file: {error}"))),
    };
    if !metadata.is_file() || crate::data_root::metadata_is_link_or_reparse(&metadata) {
        return Err(HealthError::NotAFile);
    }
    if metadata.len() > MAX_HEALTH_BYTES {
        return Err(HealthError::OversizedFile(metadata.len()));
    }
    let bytes = fs::read(&path).map_err(|error| HealthError::Io(format!("read health file: {error}")))?;
    let text = String::from_utf8(bytes).map_err(|_| HealthError::InvalidEncoding)?;
    parse_health(&text).map(Some)
}

pub fn clear_health(home: &Path) -> Result<(), HealthError> {
    let path = health_path(home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(HealthError::Io(format!("remove health file: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    struct TestHome(PathBuf);

    impl TestHome {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!("zeus-health-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("temporary home is creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    const VALID_HEALTH_TEXT: &str = "\
v=1
t=1700000000000
seq=42
screen=world
dialog=0
disconnect=0
";

    #[test]
    fn health_path_uses_zeus_health_txt_under_supplied_account_home() {
        let home = Path::new("/opt/knight/accounts/0/home");
        let path = health_path(home);
        assert_eq!(path, PathBuf::from("/opt/knight/accounts/0/home/zeus-health.txt"));
        assert_eq!(path.file_name().unwrap(), HEALTH_FILE_NAME);
    }

    #[test]
    fn valid_v1_health_text_parses_exactly() {
        let snapshot = parse_health(VALID_HEALTH_TEXT).expect("valid health text must parse");
        assert_eq!(
            snapshot,
            HealthSnapshot {
                written_at_unix_ms: 1700000000000,
                sequence: 42,
                screen: HealthScreen::World,
                dialog_open: false,
                native_disconnect: false,
            }
        );
    }

    #[test]
    fn all_six_screen_enum_values_parse() {
        let screens = [
            ("none", HealthScreen::None),
            ("login", HealthScreen::Login),
            ("server", HealthScreen::Server),
            ("character", HealthScreen::Character),
            ("world", HealthScreen::World),
            ("other", HealthScreen::Other),
        ];

        for (screen_str, expected_enum) in screens {
            let text = format!("v=1\nt=1000\nseq=1\nscreen={screen_str}\ndialog=1\ndisconnect=1\n");
            let snapshot = parse_health(&text).unwrap_or_else(|e| panic!("failed to parse screen {screen_str}: {e}"));
            assert_eq!(snapshot.screen, expected_enum);
            assert_eq!(snapshot.screen.as_str(), screen_str);
            assert!(snapshot.dialog_open);
            assert!(snapshot.native_disconnect);
        }
    }

    #[test]
    fn missing_key_is_rejected() {
        let required_keys = ["v", "t", "seq", "screen", "dialog", "disconnect"];
        for omit_key in required_keys {
            let mut lines = Vec::new();
            for key in required_keys {
                if key == omit_key {
                    continue;
                }
                let val = match key {
                    "v" => "1",
                    "t" => "1000",
                    "seq" => "1",
                    "screen" => "login",
                    "dialog" => "0",
                    "disconnect" => "0",
                    _ => unreachable!(),
                };
                lines.push(format!("{key}={val}"));
            }
            let text = lines.join("\n");
            let result = parse_health(&text);
            assert_eq!(
                result,
                Err(HealthError::MissingKey(omit_key)),
                "omitting {omit_key} must be rejected"
            );
        }
    }

    #[test]
    fn unknown_key_is_rejected() {
        let text = "v=1\nt=1000\nseq=1\nscreen=login\ndialog=0\ndisconnect=0\nextra=1\n";
        assert_eq!(
            parse_health(text),
            Err(HealthError::UnknownKey("extra".to_string()))
        );
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let text = "v=1\nv=1\nt=1000\nseq=1\nscreen=login\ndialog=0\ndisconnect=0\n";
        assert_eq!(
            parse_health(text),
            Err(HealthError::DuplicateKey("v".to_string()))
        );
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let text = "v=2\nt=1000\nseq=1\nscreen=login\ndialog=0\ndisconnect=0\n";
        assert_eq!(
            parse_health(text),
            Err(HealthError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn invalid_screen_is_rejected() {
        let text = "v=1\nt=1000\nseq=1\nscreen=unknown_screen\ndialog=0\ndisconnect=0\n";
        assert_eq!(
            parse_health(text),
            Err(HealthError::InvalidScreen("unknown_screen".to_string()))
        );
    }

    #[test]
    fn invalid_dialog_boolean_is_rejected() {
        let bad_values = ["true", "2", "-1", "yes"];
        for bad in bad_values {
            let text = format!("v=1\nt=1000\nseq=1\nscreen=world\ndialog={bad}\ndisconnect=0\n");
            assert_eq!(
                parse_health(&text),
                Err(HealthError::InvalidBoolean("dialog")),
                "dialog={bad} must be rejected"
            );
        }
    }

    #[test]
    fn invalid_disconnect_boolean_is_rejected() {
        let bad_values = ["true", "2", "-1", "yes"];
        for bad in bad_values {
            let text = format!("v=1\nt=1000\nseq=1\nscreen=world\ndialog=0\ndisconnect={bad}\n");
            assert_eq!(
                parse_health(&text),
                Err(HealthError::InvalidBoolean("disconnect")),
                "disconnect={bad} must be rejected"
            );
        }
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = TestHome::new("oversized");
        let path = health_path(dir.path());
        let oversized_content = vec![b'a'; (MAX_HEALTH_BYTES + 1) as usize];
        fs::write(&path, oversized_content).unwrap();

        let result = read_health(dir.path());
        assert_eq!(
            result,
            Err(HealthError::OversizedFile(MAX_HEALTH_BYTES + 1))
        );
    }

    #[test]
    fn missing_health_file_returns_ok_none() {
        let dir = TestHome::new("missing");
        let result = read_health(dir.path());
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn clear_health_removes_an_existing_file_and_tolerates_a_missing_file() {
        let dir = TestHome::new("clear");
        let path = health_path(dir.path());

        // Tolerates missing file
        assert_eq!(clear_health(dir.path()), Ok(()));

        // Removes existing file
        fs::write(&path, VALID_HEALTH_TEXT).unwrap();
        assert!(path.exists());
        assert_eq!(clear_health(dir.path()), Ok(()));
        assert!(!path.exists());

        // Tolerates missing file again
        assert_eq!(clear_health(dir.path()), Ok(()));
    }
}

//! Reconnect status transport foundation for observing Zeus reconnect lifecycle state.
//!
//! Exposes a strict, versioned sidecar (`zeus-reconnect.txt`, contract v1)
//! published by Zeus alongside the game loop.

use std::fs;
use std::path::{Path, PathBuf};

/// Exact file name of the reconnect status transport sidecar.
pub const RECONNECT_STATUS_FILE_NAME: &str = "zeus-reconnect.txt";

/// Current reconnect status wire contract version.
pub const RECONNECT_STATUS_VERSION: u32 = 1;

/// Maximum allowed bytes for the reconnect status transport file (1 KiB).
pub const MAX_RECONNECT_STATUS_BYTES: u64 = 1024;

/// Observable reconnect lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    Idle,
    NativeWait,
    Login,
    Server,
    Character,
    Loading,
    WorldSettle,
    Other,
}

impl ReconnectState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::NativeWait => "native_wait",
            Self::Login => "login",
            Self::Server => "server",
            Self::Character => "character",
            Self::Loading => "loading",
            Self::WorldSettle => "world_settle",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(Self::Idle),
            "native_wait" => Some(Self::NativeWait),
            "login" => Some(Self::Login),
            "server" => Some(Self::Server),
            "character" => Some(Self::Character),
            "loading" => Some(Self::Loading),
            "world_settle" => Some(Self::WorldSettle),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// One reconnect status snapshot published by Zeus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectStatusSnapshot {
    pub written_at_unix_ms: i64,
    pub sequence: u64,
    pub episode_id: u32,
    pub active: bool,
    pub state: ReconnectState,
    pub transitions: u32,
    pub world_seen_before_episode: bool,
}

/// Failure modes during reconnect status transport reading and parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectStatusError {
    MissingKey(&'static str),
    UnknownKey(String),
    DuplicateKey(String),
    UnsupportedVersion(u32),
    MalformedInteger(&'static str),
    InvalidBoolean(&'static str),
    InvalidState(String),
    InvariantViolation(String),
    OversizedFile(u64),
    NotAFile,
    InvalidEncoding,
    Io(String),
}

impl std::fmt::Display for ReconnectStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingKey(key) => write!(f, "missing reconnect status key: {key}"),
            Self::UnknownKey(key) => write!(f, "unknown reconnect status key: {key}"),
            Self::DuplicateKey(key) => write!(f, "duplicate reconnect status key: {key}"),
            Self::UnsupportedVersion(v) => {
                write!(f, "unsupported reconnect status version: {v}")
            }
            Self::MalformedInteger(field) => {
                write!(f, "malformed integer for reconnect status field: {field}")
            }
            Self::InvalidBoolean(field) => {
                write!(f, "invalid boolean for reconnect status field: {field}")
            }
            Self::InvalidState(state) => write!(f, "invalid reconnect state: {state}"),
            Self::InvariantViolation(msg) => write!(f, "reconnect status invariant violation: {msg}"),
            Self::OversizedFile(bytes) => {
                write!(f, "reconnect status file exceeds maximum size: {bytes}")
            }
            Self::NotAFile => write!(f, "reconnect status path is not a regular file"),
            Self::InvalidEncoding => write!(f, "reconnect status file is not valid UTF-8"),
            Self::Io(msg) => write!(f, "reconnect status I/O error: {msg}"),
        }
    }
}

impl std::error::Error for ReconnectStatusError {}

pub fn reconnect_status_path(home: &Path) -> PathBuf {
    home.join(RECONNECT_STATUS_FILE_NAME)
}

pub fn parse_reconnect_status(text: &str) -> Result<ReconnectStatusSnapshot, ReconnectStatusError> {
    let mut v: Option<u32> = None;
    let mut t: Option<i64> = None;
    let mut seq: Option<u64> = None;
    let mut episode: Option<u32> = None;
    let mut active: Option<bool> = None;
    let mut state: Option<ReconnectState> = None;
    let mut transitions: Option<u32> = None;
    let mut world_before: Option<bool> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| ReconnectStatusError::UnknownKey(line.to_string()))?;

        match key {
            "v" => {
                if v.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("v".to_string()));
                }
                let parsed_v = value
                    .parse::<u32>()
                    .map_err(|_| ReconnectStatusError::MalformedInteger("v"))?;
                if parsed_v != RECONNECT_STATUS_VERSION {
                    return Err(ReconnectStatusError::UnsupportedVersion(parsed_v));
                }
                v = Some(parsed_v);
            }
            "t" => {
                if t.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("t".to_string()));
                }
                let parsed_t = value
                    .parse::<i64>()
                    .map_err(|_| ReconnectStatusError::MalformedInteger("t"))?;
                t = Some(parsed_t);
            }
            "seq" => {
                if seq.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("seq".to_string()));
                }
                let parsed_seq = value
                    .parse::<u64>()
                    .map_err(|_| ReconnectStatusError::MalformedInteger("seq"))?;
                seq = Some(parsed_seq);
            }
            "episode" => {
                if episode.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("episode".to_string()));
                }
                let parsed_episode = value
                    .parse::<u32>()
                    .map_err(|_| ReconnectStatusError::MalformedInteger("episode"))?;
                episode = Some(parsed_episode);
            }
            "active" => {
                if active.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("active".to_string()));
                }
                let parsed_active = match value {
                    "0" => false,
                    "1" => true,
                    _ => return Err(ReconnectStatusError::InvalidBoolean("active")),
                };
                active = Some(parsed_active);
            }
            "state" => {
                if state.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("state".to_string()));
                }
                let parsed_state = ReconnectState::parse(value)
                    .ok_or_else(|| ReconnectStatusError::InvalidState(value.to_string()))?;
                state = Some(parsed_state);
            }
            "transitions" => {
                if transitions.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("transitions".to_string()));
                }
                let parsed_transitions = value
                    .parse::<u32>()
                    .map_err(|_| ReconnectStatusError::MalformedInteger("transitions"))?;
                transitions = Some(parsed_transitions);
            }
            "world_before" => {
                if world_before.is_some() {
                    return Err(ReconnectStatusError::DuplicateKey("world_before".to_string()));
                }
                let parsed_world_before = match value {
                    "0" => false,
                    "1" => true,
                    _ => return Err(ReconnectStatusError::InvalidBoolean("world_before")),
                };
                world_before = Some(parsed_world_before);
            }
            unknown => return Err(ReconnectStatusError::UnknownKey(unknown.to_string())),
        }
    }

    let _v = v.ok_or(ReconnectStatusError::MissingKey("v"))?;
    let t = t.ok_or(ReconnectStatusError::MissingKey("t"))?;
    let seq = seq.ok_or(ReconnectStatusError::MissingKey("seq"))?;
    let episode = episode.ok_or(ReconnectStatusError::MissingKey("episode"))?;
    let active = active.ok_or(ReconnectStatusError::MissingKey("active"))?;
    let state = state.ok_or(ReconnectStatusError::MissingKey("state"))?;
    let transitions = transitions.ok_or(ReconnectStatusError::MissingKey("transitions"))?;
    let world_before = world_before.ok_or(ReconnectStatusError::MissingKey("world_before"))?;

    // Invariants
    if !active && state != ReconnectState::Idle {
        return Err(ReconnectStatusError::InvariantViolation(
            "active=0 requires state=idle".to_string(),
        ));
    }
    if active && state == ReconnectState::Idle {
        return Err(ReconnectStatusError::InvariantViolation(
            "active=1 must not use state=idle".to_string(),
        ));
    }

    Ok(ReconnectStatusSnapshot {
        written_at_unix_ms: t,
        sequence: seq,
        episode_id: episode,
        active,
        state,
        transitions,
        world_seen_before_episode: world_before,
    })
}

pub fn read_reconnect_status(home: &Path) -> Result<Option<ReconnectStatusSnapshot>, ReconnectStatusError> {
    let path = reconnect_status_path(home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReconnectStatusError::Io(format!("inspect reconnect status file: {error}"))),
    };
    if !metadata.is_file() || crate::data_root::metadata_is_link_or_reparse(&metadata) {
        return Err(ReconnectStatusError::NotAFile);
    }
    if metadata.len() > MAX_RECONNECT_STATUS_BYTES {
        return Err(ReconnectStatusError::OversizedFile(metadata.len()));
    }
    let bytes = fs::read(&path)
        .map_err(|error| ReconnectStatusError::Io(format!("read reconnect status file: {error}")))?;
    let text = String::from_utf8(bytes).map_err(|_| ReconnectStatusError::InvalidEncoding)?;
    parse_reconnect_status(&text).map(Some)
}

pub fn clear_reconnect_status(home: &Path) -> Result<(), ReconnectStatusError> {
    let path = reconnect_status_path(home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ReconnectStatusError::Io(format!("remove reconnect status file: {error}"))),
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
            let path = std::env::temp_dir().join(format!("zeus-reconnect-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("temporary home is creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn valid_v1_reconnect_status_parses_exactly() {
        let text = "v=1\nt=1700000000123\nseq=42\nepisode=7\nactive=1\nstate=login\ntransitions=3\nworld_before=1\n";
        let snap = parse_reconnect_status(text).expect("valid text must parse");
        assert_eq!(snap.written_at_unix_ms, 1700000000123);
        assert_eq!(snap.sequence, 42);
        assert_eq!(snap.episode_id, 7);
        assert!(snap.active);
        assert_eq!(snap.state, ReconnectState::Login);
        assert_eq!(snap.transitions, 3);
        assert!(snap.world_seen_before_episode);
    }

    #[test]
    fn valid_inactive_idle_reconnect_status_parses() {
        let text = "v=1\nt=1700000000000\nseq=1\nepisode=0\nactive=0\nstate=idle\ntransitions=0\nworld_before=0\n";
        let snap = parse_reconnect_status(text).expect("inactive idle must parse");
        assert_eq!(snap.written_at_unix_ms, 1700000000000);
        assert_eq!(snap.sequence, 1);
        assert_eq!(snap.episode_id, 0);
        assert!(!snap.active);
        assert_eq!(snap.state, ReconnectState::Idle);
        assert_eq!(snap.transitions, 0);
        assert!(!snap.world_seen_before_episode);
    }

    #[test]
    fn all_eight_reconnect_state_values_parse() {
        let states = [
            ("idle", ReconnectState::Idle, "0", "0"),
            ("native_wait", ReconnectState::NativeWait, "1", "1"),
            ("login", ReconnectState::Login, "1", "0"),
            ("server", ReconnectState::Server, "1", "1"),
            ("character", ReconnectState::Character, "1", "0"),
            ("loading", ReconnectState::Loading, "1", "1"),
            ("world_settle", ReconnectState::WorldSettle, "1", "0"),
            ("other", ReconnectState::Other, "1", "1"),
        ];

        for (state_str, expected_state, active_str, wb_str) in states {
            let text = format!(
                "v=1\nt=100\nseq=1\nepisode=1\nactive={active_str}\nstate={state_str}\ntransitions=1\nworld_before={wb_str}\n"
            );
            let snap = parse_reconnect_status(&text).expect("state must parse");
            assert_eq!(snap.state, expected_state);
            assert_eq!(snap.state.as_str(), state_str);
        }
    }

    #[test]
    fn missing_key_is_rejected() {
        let keys = ["v", "t", "seq", "episode", "active", "state", "transitions", "world_before"];
        for missing in keys {
            let mut lines = Vec::new();
            if missing != "v" { lines.push("v=1"); }
            if missing != "t" { lines.push("t=100"); }
            if missing != "seq" { lines.push("seq=1"); }
            if missing != "episode" { lines.push("episode=1"); }
            if missing != "active" { lines.push("active=1"); }
            if missing != "state" { lines.push("state=login"); }
            if missing != "transitions" { lines.push("transitions=0"); }
            if missing != "world_before" { lines.push("world_before=0"); }

            let text = lines.join("\n") + "\n";
            let err = parse_reconnect_status(&text).unwrap_err();
            assert_eq!(err, ReconnectStatusError::MissingKey(missing));
        }
    }

    #[test]
    fn unknown_key_is_rejected() {
        let text = "v=1\nt=100\nseq=1\nepisode=1\nactive=0\nstate=idle\ntransitions=0\nworld_before=0\nextra=value\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::UnknownKey("extra".to_string()));
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let text = "v=1\nv=1\nt=100\nseq=1\nepisode=1\nactive=0\nstate=idle\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::DuplicateKey("v".to_string()));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let text = "v=2\nt=100\nseq=1\nepisode=1\nactive=0\nstate=idle\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::UnsupportedVersion(2));
    }

    #[test]
    fn malformed_integer_is_rejected() {
        let text = "v=1\nt=not_a_num\nseq=1\nepisode=1\nactive=0\nstate=idle\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::MalformedInteger("t"));
    }

    #[test]
    fn invalid_boolean_is_rejected() {
        let text = "v=1\nt=100\nseq=1\nepisode=1\nactive=yes\nstate=idle\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::InvalidBoolean("active"));

        let text2 = "v=1\nt=100\nseq=1\nepisode=1\nactive=0\nstate=idle\ntransitions=0\nworld_before=true\n";
        let err2 = parse_reconnect_status(text2).unwrap_err();
        assert_eq!(err2, ReconnectStatusError::InvalidBoolean("world_before"));
    }

    #[test]
    fn invalid_state_is_rejected() {
        let text = "v=1\nt=100\nseq=1\nepisode=1\nactive=1\nstate=unknown_screen\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(err, ReconnectStatusError::InvalidState("unknown_screen".to_string()));
    }

    #[test]
    fn active_zero_with_non_idle_is_rejected() {
        let text = "v=1\nt=100\nseq=1\nepisode=1\nactive=0\nstate=login\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(
            err,
            ReconnectStatusError::InvariantViolation("active=0 requires state=idle".to_string())
        );
    }

    #[test]
    fn active_one_with_idle_is_rejected() {
        let text = "v=1\nt=100\nseq=1\nepisode=1\nactive=1\nstate=idle\ntransitions=0\nworld_before=0\n";
        let err = parse_reconnect_status(text).unwrap_err();
        assert_eq!(
            err,
            ReconnectStatusError::InvariantViolation("active=1 must not use state=idle".to_string())
        );
    }

    #[test]
    fn oversized_file_is_rejected() {
        let home = TestHome::new("oversized");
        let path = reconnect_status_path(home.path());
        let payload = vec![b'a'; (MAX_RECONNECT_STATUS_BYTES + 1) as usize];
        fs::write(&path, payload).expect("oversized payload is writable");

        let err = read_reconnect_status(home.path()).unwrap_err();
        assert_eq!(err, ReconnectStatusError::OversizedFile(MAX_RECONNECT_STATUS_BYTES + 1));
    }

    #[test]
    fn missing_file_returns_ok_none() {
        let home = TestHome::new("missing");
        let res = read_reconnect_status(home.path()).expect("read must succeed with None");
        assert!(res.is_none());
    }

    #[test]
    fn clear_reconnect_status_removes_existing_and_tolerates_missing() {
        let home = TestHome::new("clear");
        let path = reconnect_status_path(home.path());

        // Tolerates missing
        clear_reconnect_status(home.path()).expect("clear missing file must succeed");

        // Removes existing
        fs::write(&path, "v=1\n").expect("write test file");
        assert!(path.exists());
        clear_reconnect_status(home.path()).expect("clear existing file must succeed");
        assert!(!path.exists());
    }

    #[test]
    fn reconnect_status_path_uses_exact_filename() {
        let home = Path::new("/var/data/account");
        assert_eq!(
            reconnect_status_path(home),
            PathBuf::from("/var/data/account/zeus-reconnect.txt")
        );
    }
}

use serde::Deserialize;

use crate::rms::SERVER_COUNT;
use crate::{CoreError, CoreResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountConfigV1 {
    pub task_timeout_ms: u64,
    pub ready_timeout_ms: u64,
    pub menu_settle_ms: u64,
    pub screen_transition_ms: u64,
    pub editor_settle_ms: u64,
    pub foreground_retry_timeout_ms: u64,
    /// Index into the client's server table `dx.b`, seeded into the `isIndexServer` record store.
    pub server_index: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredConfigV1 {
    schema: u64,
    login: StoredLoginV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLoginV1 {
    task_timeout_ms: u64,
    ready_timeout_ms: u64,
    menu_settle_ms: u64,
    screen_transition_ms: u64,
    editor_settle_ms: u64,
    foreground_retry_timeout_ms: u64,
    /// Absent in every config written before per-account server selection existed.
    ///
    /// Defaulted rather than required so an account imported by an earlier build still loads; it
    /// resolves to the first server, and the next write persists the value explicitly. A `null`
    /// is still rejected, because only a missing key means "written before this field existed".
    #[serde(default)]
    server_index: u8,
}

impl AccountConfigV1 {
    pub(crate) fn defaults() -> Self {
        Self {
            task_timeout_ms: 60_000,
            ready_timeout_ms: 30_000,
            menu_settle_ms: 6_000,
            screen_transition_ms: 2_000,
            editor_settle_ms: 750,
            foreground_retry_timeout_ms: 5_000,
            // First entry of the client's server table, so an imported account is always seedable.
            server_index: 0,
        }
    }

    pub(crate) fn parse_strict(json: &str) -> CoreResult<Self> {
        let stored: StoredConfigV1 =
            serde_json::from_str(json).map_err(|_| invalid_account_config())?;
        if stored.schema != 1 {
            return Err(invalid_account_config());
        }
        let config = Self {
            task_timeout_ms: stored.login.task_timeout_ms,
            ready_timeout_ms: stored.login.ready_timeout_ms,
            menu_settle_ms: stored.login.menu_settle_ms,
            screen_transition_ms: stored.login.screen_transition_ms,
            editor_settle_ms: stored.login.editor_settle_ms,
            foreground_retry_timeout_ms: stored.login.foreground_retry_timeout_ms,
            server_index: stored.login.server_index,
        };
        if !(10_000..=120_000).contains(&config.task_timeout_ms)
            || !(5_000..=60_000).contains(&config.ready_timeout_ms)
            || !(0..=60_000).contains(&config.menu_settle_ms)
            || !(100..=10_000).contains(&config.screen_transition_ms)
            || !(100..=5_000).contains(&config.editor_settle_ms)
            || !(100..=30_000).contains(&config.foreground_retry_timeout_ms)
            // An index past the client's table would make the MIDlet read out of bounds, so it is
            // refused here rather than at seed time.
            || config.server_index >= SERVER_COUNT
        {
            return Err(invalid_account_config());
        }
        Ok(config)
    }

    pub(crate) fn to_canonical_json(&self) -> String {
        format!(
            concat!(
                r#"{{"schema":1,"login":{{"task_timeout_ms":{},"ready_timeout_ms":{},"#,
                r#""menu_settle_ms":{},"screen_transition_ms":{},"editor_settle_ms":{},"#,
                r#""foreground_retry_timeout_ms":{},"server_index":{}}}}}"#,
            ),
            self.task_timeout_ms,
            self.ready_timeout_ms,
            self.menu_settle_ms,
            self.screen_transition_ms,
            self.editor_settle_ms,
            self.foreground_retry_timeout_ms,
            self.server_index,
        )
    }
}

fn invalid_account_config() -> CoreError {
    CoreError::UnmanagedDatabase
}

#[cfg(test)]
mod tests {
    use super::{AccountConfigV1, SERVER_COUNT};

    const DEFAULT_JSON: &str = concat!(
        r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"#,
        r#""menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"#,
        r#""foreground_retry_timeout_ms":5000,"server_index":0}}"#,
    );

    /// The exact config an account imported before per-account server selection carries on disk.
    const PRE_SERVER_INDEX_JSON: &str = concat!(
        r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"#,
        r#""menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"#,
        r#""foreground_retry_timeout_ms":5000}}"#,
    );

    #[test]
    fn account_config_defaults_have_exact_canonical_json() {
        let defaults = AccountConfigV1::defaults();
        assert_eq!(defaults.to_canonical_json(), DEFAULT_JSON);
        assert_eq!(
            AccountConfigV1::parse_strict(DEFAULT_JSON).unwrap(),
            defaults
        );
    }

    #[test]
    fn a_config_written_before_server_selection_still_loads_on_the_first_server() {
        // An account imported by an earlier build must keep working, so the absent key defaults.
        let migrated = AccountConfigV1::parse_strict(PRE_SERVER_INDEX_JSON)
            .expect("a pre-server-index config still parses");
        assert_eq!(migrated.server_index, 0);
        assert_eq!(migrated, AccountConfigV1::defaults());
        // The next write persists the field explicitly rather than leaving it implicit.
        assert_eq!(migrated.to_canonical_json(), DEFAULT_JSON);
    }

    #[test]
    fn every_server_in_the_client_table_round_trips_and_anything_past_it_is_refused() {
        for index in 0..SERVER_COUNT {
            let mut config = AccountConfigV1::defaults();
            config.server_index = index;
            let json = config.to_canonical_json();
            assert_eq!(
                AccountConfigV1::parse_strict(&json).expect("an in-table server parses"),
                config,
                "server {index} did not round-trip"
            );
        }
        // One past the last entry would index out of bounds inside the client.
        let mut past_end = AccountConfigV1::defaults();
        past_end.server_index = SERVER_COUNT;
        assert!(
            AccountConfigV1::parse_strict(&past_end.to_canonical_json()).is_err(),
            "accepted a server index past the client table"
        );
        // Only an absent key means "written before this field existed"; an explicit null is corrupt.
        for json in [
            DEFAULT_JSON.replace(r#""server_index":0"#, r#""server_index":null"#),
            DEFAULT_JSON.replace(r#""server_index":0"#, r#""server_index":256"#),
            DEFAULT_JSON.replace(r#""server_index":0"#, r#""server_index":-1"#),
            DEFAULT_JSON.replace(r#""server_index":0"#, r#""server_index":"0""#),
            DEFAULT_JSON.replace(r#""server_index":0"#, r#""server_index":0.0"#),
        ] {
            assert!(
                AccountConfigV1::parse_strict(&json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn account_config_rejects_every_missing_key() {
        for json in [
            r#"{"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1}"#,
            r#"{"schema":1,"login":{"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750}}"#,
        ] {
            assert!(
                AccountConfigV1::parse_strict(json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn account_config_rejects_unknown_keys_at_every_object_level() {
        for json in [
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000},"unknown":0}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000,"unknown":0}}"#,
        ] {
            assert!(
                AccountConfigV1::parse_strict(json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn account_config_rejects_every_duplicate_key() {
        for json in [
            r#"{"schema":1,"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000},"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000}}"#,
            r#"{"schema":1,"login":{"task_timeout_ms":60000,"ready_timeout_ms":30000,"menu_settle_ms":6000,"screen_transition_ms":2000,"editor_settle_ms":750,"foreground_retry_timeout_ms":5000,"foreground_retry_timeout_ms":5000}}"#,
        ] {
            assert!(
                AccountConfigV1::parse_strict(json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn account_config_rejects_non_integers_and_unsupported_schema() {
        for json in [
            DEFAULT_JSON.replace(r#""schema":1"#, r#""schema":0"#),
            DEFAULT_JSON.replace(r#""schema":1"#, r#""schema":2"#),
            DEFAULT_JSON.replace(r#""schema":1"#, r#""schema":1.0"#),
            DEFAULT_JSON.replace("60000", "60000.0"),
            DEFAULT_JSON.replace("30000", r#""30000""#),
            DEFAULT_JSON.replace("6000", "null"),
            DEFAULT_JSON.replace("2000", "true"),
            DEFAULT_JSON.replace("750", "[]"),
            DEFAULT_JSON.replace("5000", "{}"),
        ] {
            assert!(
                AccountConfigV1::parse_strict(&json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn account_config_accepts_each_numeric_boundary_and_rejects_neighbors() {
        let cases = [
            ("task_timeout_ms", 10_000, 120_000),
            ("ready_timeout_ms", 5_000, 60_000),
            ("menu_settle_ms", 0, 60_000),
            ("screen_transition_ms", 100, 10_000),
            ("editor_settle_ms", 100, 5_000),
            ("foreground_retry_timeout_ms", 100, 30_000),
        ];
        for (field, minimum, maximum) in cases {
            let mut config = AccountConfigV1::defaults();
            set_field(&mut config, field, minimum);
            let minimum_json = config.to_canonical_json();
            assert_eq!(
                AccountConfigV1::parse_strict(&minimum_json).unwrap(),
                config
            );
            set_field(&mut config, field, maximum);
            let maximum_json = config.to_canonical_json();
            assert_eq!(
                AccountConfigV1::parse_strict(&maximum_json).unwrap(),
                config
            );

            if minimum > 0 {
                set_field(&mut config, field, minimum - 1);
                assert!(AccountConfigV1::parse_strict(&config.to_canonical_json()).is_err());
            }
            set_field(&mut config, field, maximum + 1);
            assert!(AccountConfigV1::parse_strict(&config.to_canonical_json()).is_err());
        }
    }

    fn set_field(config: &mut AccountConfigV1, field: &str, value: u64) {
        match field {
            "task_timeout_ms" => config.task_timeout_ms = value,
            "ready_timeout_ms" => config.ready_timeout_ms = value,
            "menu_settle_ms" => config.menu_settle_ms = value,
            "screen_transition_ms" => config.screen_transition_ms = value,
            "editor_settle_ms" => config.editor_settle_ms = value,
            "foreground_retry_timeout_ms" => config.foreground_retry_timeout_ms = value,
            _ => unreachable!(),
        }
    }
}

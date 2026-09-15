use serde::Serialize;

pub(crate) const DEFAULT_LAUNCH_POLICY_JSON: &str = r#"{"priority":0,"stagger_class":"default"}"#;
pub(crate) const DEFAULT_PRESENTATION_JSON: &str = "{}";
pub(crate) const MAX_PROFILE_PAGE_LIMIT: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileRecord {
    pub profile_id: String,
    pub revision: i64,
    pub display_name: String,
    pub runtime_id: String,
    pub launch_policy_json: String,
    pub presentation_json: String,
    pub archived_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfilePage {
    pub items: Vec<ProfileRecord>,
    pub next_cursor: Option<String>,
}

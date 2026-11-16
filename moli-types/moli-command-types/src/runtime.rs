#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRealmInfo {
    pub context_id: i64,
    pub realm_id: Option<String>,
    pub frame_id: Option<String>,
    pub origin: String,
    pub name: String,
    pub is_default: bool,
    pub context_type: String,
    pub grant_universal_access: Option<bool>,
}

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

/// Pause transition metadata shared by the preserved Inspector output path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectorPauseEffect {
    None,
    Resume,
    Step,
}

/// Scheduling policy shared by runtime owner messages and environment wakes.
/// Protocol method classification belongs to the optional DevTools adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeTaskMode {
    Interrupt,
    DontInterrupt,
}

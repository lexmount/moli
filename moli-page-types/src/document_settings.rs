use crate::{
    DocumentActivity, EmulatedIdleOverride, EmulatedMediaOverrides, NavigatorOverrides,
    ViewportSurface,
};

/// Target settings installed before a Document starts executing scripts.
///
/// HTML creation, streaming creation and prepared-document commit carry the
/// same snapshot. Live Page commands update these values independently of
/// immutable browser startup configuration and document response metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentSettings {
    pub script_execution_disabled: bool,
    /// Target emulation contribution; startup scrollbar hiding remains separate.
    pub scrollbars_hidden: bool,
    pub bypass_content_security_policy: bool,
    pub emulated_media: EmulatedMediaOverrides,
    pub idle_override: Option<EmulatedIdleOverride>,
    pub navigator_overrides: NavigatorOverrides,
    pub viewport_surface: Option<ViewportSurface>,
    pub document_activity: DocumentActivity,
}

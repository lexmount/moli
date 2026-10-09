use crate::NodeId;

/// Final computed facts needed by AX. The renderer owns style observation;
/// accessibility never reads CSS declarations or published layout boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityStyle {
    pub display_none: bool,
    pub visibility_visible: bool,
    pub hides_contents: bool,
    pub block_level: bool,
}

/// Reads from one synchronous style observation. A missing required style
/// makes the AX request unavailable; it must not become a visible default.
pub trait AccessibilityStyleSource {
    fn element_style(&mut self, node_id: NodeId) -> Option<AccessibilityStyle>;
}

pub struct AccessibilityInput<'a> {
    pub(super) styles: &'a mut dyn AccessibilityStyleSource,
    pub(super) frame_state: AccessibilityFrameState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityFrameState {
    Active,
    Inert,
}

impl<'a> AccessibilityInput<'a> {
    pub fn new(
        styles: &'a mut dyn AccessibilityStyleSource,
        frame_state: AccessibilityFrameState,
    ) -> Self {
        Self {
            styles,
            frame_state,
        }
    }
}

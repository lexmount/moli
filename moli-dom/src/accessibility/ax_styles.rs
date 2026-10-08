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

pub struct AccessibilityInput {
    elements: Vec<Option<AccessibilityStyle>>,
    pub(super) frame_state: AccessibilityFrameState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityFrameState {
    Active,
    Inert,
}

impl AccessibilityInput {
    pub fn new(node_count: usize, frame_state: AccessibilityFrameState) -> Self {
        Self {
            elements: vec![None; node_count],
            frame_state,
        }
    }

    pub fn insert(&mut self, node_id: NodeId, style: AccessibilityStyle) {
        self.elements[node_id.index()] = Some(style);
    }

    pub(super) fn element(&self, node_id: NodeId) -> Option<AccessibilityStyle> {
        self.elements.get(node_id.index()).copied().flatten()
    }

    #[cfg(test)]
    pub(super) fn visible_fixture(document: &crate::native::DomHost) -> Self {
        let mut styles = Self::new(document.len(), AccessibilityFrameState::Active);
        for node in document.nodes() {
            if node.is_element() {
                styles.insert(
                    node.id(),
                    AccessibilityStyle {
                        display_none: false,
                        visibility_visible: true,
                        hides_contents: false,
                        block_level: false,
                    },
                );
            }
        }
        styles
    }
}

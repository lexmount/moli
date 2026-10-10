mod ax_dom;
mod ax_node;
mod ax_projection;
mod ax_properties;
mod ax_roles;
mod ax_styles;
mod ax_tree;

pub use ax_node::{AccessibilityNode, AccessibilityNodeId, AccessibilityValue};
pub use ax_styles::{
    AccessibilityFrameState, AccessibilityInput, AccessibilityStyle, AccessibilityStyleSource,
};

pub use ax_tree::{
    AccessibilityRequest, accessibility_nodes_for_document, accessibility_payloads_for_document,
};

#[cfg(test)]
mod tests;

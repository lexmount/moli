mod ax_dom;
mod ax_projection;
mod ax_properties;
mod ax_roles;
mod ax_styles;
mod ax_tree;

pub use ax_styles::{AccessibilityFrameState, AccessibilityInput, AccessibilityStyle};

pub use ax_tree::{
    accessibility_child_node_payloads_for_document,
    accessibility_child_node_payloads_for_document_with_backend_node_ids,
    accessibility_node_and_ancestor_payloads_for_document,
    accessibility_node_and_ancestor_payloads_for_document_with_backend_node_ids,
    accessibility_node_payload_for_document,
    accessibility_node_payload_for_document_with_backend_node_ids,
    accessibility_partial_tree_payloads_for_document,
    accessibility_partial_tree_payloads_for_document_with_backend_node_ids,
    accessibility_tree_payloads_for_document,
    accessibility_tree_payloads_for_document_with_backend_node_ids,
};

#[cfg(test)]
mod tests;

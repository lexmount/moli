mod engine;

#[cfg(test)]
mod tests;

use super::{NativeDom, NativeNodeId};
pub(super) use engine::{
    HtmlSerializationTarget, HtmlSerializedShadowRoot, escape_html_attribute_into_string,
    serialize_html_with_shadow_root_provider,
};
use engine::{serialize_html, serialize_html_with_limit};

/// A bounded serialization stopped before appending bytes past its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlSerializationLimitExceeded {
    pub max_bytes: usize,
}

impl std::fmt::Display for HtmlSerializationLimitExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "serialized HTML exceeds the {}-byte output limit",
            self.max_bytes
        )
    }
}

impl std::error::Error for HtmlSerializationLimitExceeded {}

impl NativeDom {
    pub fn serialize_document(&self) -> String {
        self.serialize_document_with_scripting_enabled(true)
    }

    pub fn serialize_document_with_scripting_enabled(&self, scripting_enabled: bool) -> String {
        serialize_html(
            self,
            self.document_node_id,
            HtmlSerializationTarget::ChildrenOnly,
            scripting_enabled,
        )
        .unwrap_or_default()
    }

    pub fn outer_html(&self, node_id: NativeNodeId) -> Option<String> {
        self.outer_html_with_scripting_enabled(node_id, true)
    }

    pub fn outer_html_with_scripting_enabled(
        &self,
        node_id: NativeNodeId,
        scripting_enabled: bool,
    ) -> Option<String> {
        serialize_html(
            self,
            node_id,
            HtmlSerializationTarget::IncludeNode,
            scripting_enabled,
        )
    }

    /// Serializes one subtree without ever growing the output beyond `max_bytes`.
    ///
    /// This is intended for bounded derived consumers such as a fresh inline
    /// SVG image parse. Web-exposed `outerHTML` continues to use the unbounded
    /// serializer because truncation would violate its string contract.
    pub fn outer_html_with_limit(
        &self,
        node_id: NativeNodeId,
        max_bytes: usize,
    ) -> Result<Option<String>, HtmlSerializationLimitExceeded> {
        serialize_html_with_limit(self, node_id, max_bytes)
    }

    pub fn inner_html(&self, node_id: NativeNodeId) -> Option<String> {
        self.inner_html_with_scripting_enabled(node_id, true)
    }

    pub fn inner_html_with_scripting_enabled(
        &self,
        node_id: NativeNodeId,
        scripting_enabled: bool,
    ) -> Option<String> {
        serialize_html(
            self,
            node_id,
            HtmlSerializationTarget::ChildrenOnly,
            scripting_enabled,
        )
    }
}

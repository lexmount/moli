mod attributes;
mod engine;

#[cfg(test)]
mod embedded_tests;
#[cfg(test)]
mod tests;

use super::{Attribute, NativeDom, NativeNodeId, Node};
pub use engine::escape_html_attribute_into_string;
pub(super) use engine::{
    HtmlSerializationOptions, HtmlSerializationTarget, HtmlSerializedShadowRoot,
    HtmlShadowRootProvider, serialize_html_with_options,
};
use engine::{serialize_html, serialize_html_with_limit};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlInsertionPoint {
    BeforeNode,
    FirstChild,
    LastChild,
}

/// Trusted, already escaped export markup inserted at one traversal boundary.
#[derive(Clone, Copy, Debug)]
pub struct HtmlSerializationInsertion<'a> {
    pub node: NativeNodeId,
    pub point: HtmlInsertionPoint,
    pub markup: &'a str,
}

/// Read-only output rules. No rule changes the source nodes or their attributes.
/// Ordinary Web serialization does not install a policy.
pub trait HtmlSerializationPolicy {
    /// Whether `include_node` or `include_attribute` can exclude content.
    /// Insertion, attribute overrides and shadow metadata still apply when
    /// false. The conservative default preserves existing filter callbacks.
    fn has_filters(&self) -> bool {
        true
    }

    /// Excluding a node excludes its entire subtree.
    fn include_node(&self, _node: &Node) -> bool {
        true
    }

    fn include_attribute(&self, _node: NativeNodeId, _attribute: &Attribute) -> bool {
        true
    }

    /// Replace attributes with matching namespace/local name, then append new
    /// attributes in slice order. Values still use the normal HTML escaper.
    fn attribute_overrides(&self, _node: NativeNodeId) -> &[Attribute] {
        &[]
    }

    fn insertion(&self) -> Option<HtmlSerializationInsertion<'_>> {
        None
    }

    fn include_shadow_adopted_style_sheets(&self) -> bool {
        true
    }
}

/// A live embedded Document already present in the native DOM.
pub struct HtmlEmbeddedDocument<'a> {
    pub document: NativeNodeId,
    pub url: Cow<'a, str>,
}

/// Lookup only: implementations must not load documents or execute scripts.
pub trait HtmlEmbeddedDocumentProvider {
    /// If known, this is the only element that can own an embedded Document.
    /// None leaves ownership unrestricted; the hint is read once per export.
    fn sole_owner(&self) -> Option<NativeNodeId> {
        None
    }

    fn embedded_document(&self, owner: NativeNodeId) -> Option<HtmlEmbeddedDocument<'_>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlSerializationError {
    InvalidNode { node: NativeNodeId },
    CyclicEmbeddedDocument { document: NativeNodeId },
    OutputLimitExceeded { max_bytes: usize },
}

impl std::fmt::Display for HtmlSerializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNode { node } => write!(f, "invalid HTML serialization node {node:?}"),
            Self::CyclicEmbeddedDocument { document } => {
                write!(f, "cyclic embedded Document {document:?}")
            }
            Self::OutputLimitExceeded { max_bytes } => write!(
                f,
                "serialized HTML exceeds the {max_bytes}-byte output limit"
            ),
        }
    }
}
impl std::error::Error for HtmlSerializationError {}

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

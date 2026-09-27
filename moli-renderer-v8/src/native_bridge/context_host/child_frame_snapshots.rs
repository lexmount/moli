use serde::Serialize;
use url::Url;

use crate::document_runtime::DocumentPolicyContainer;

mod detached;
mod document_lookup;
mod frame_tree;
mod ids;
mod markup;

const MAX_CHILD_FRAME_TREE_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChildBrowsingContextFrameSnapshot {
    pub(crate) frame_id: String,
    pub(crate) loader_id: String,
    pub(crate) name: Option<String>,
    pub(crate) owner_element_id: Option<String>,
    pub(crate) url: String,
    pub(crate) storage_key: String,
    #[serde(default)]
    pub(crate) security_origin_inherited: bool,
    #[serde(default)]
    pub(crate) security_origin_opaque: bool,
    #[serde(default)]
    pub(crate) child_frames: Vec<ChildBrowsingContextFrameSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildBrowsingContextDocumentSnapshot {
    pub(crate) url: String,
    pub(crate) markup: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetachedChildBrowsingContextDocumentSnapshot {
    pub(crate) parent_frame_id: String,
    pub(crate) frame_id: String,
    pub(crate) owner_node_id: crate::document_runtime::DomHandle,
    pub(crate) url: Url,
    pub(crate) markup: String,
    /// Parser-only scripting state captured from the detached Document owner.
    /// DOMSnapshot reparses `markup`, so it must preserve `<noscript>` semantics
    /// without implying that snapshot parsing executes JavaScript.
    pub(crate) scripting_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildBrowsingContextSnapshot {
    pub(crate) url: Url,
    pub(crate) fallback_base_url: Option<Url>,
    pub(crate) markup: String,
    pub(crate) resource_was_cached: bool,
    pub(crate) content_type: Option<String>,
    pub(crate) character_set: String,
    pub(crate) policy_container: DocumentPolicyContainer,
}

impl ChildBrowsingContextSnapshot {
    pub(crate) fn new(url: Url, markup: String, content_type: Option<String>) -> Self {
        Self::with_character_set(url, markup, content_type, "UTF-8")
    }

    pub(crate) fn with_character_set(
        url: Url,
        markup: String,
        content_type: Option<String>,
        character_set: impl Into<String>,
    ) -> Self {
        Self {
            url,
            fallback_base_url: None,
            markup,
            resource_was_cached: false,
            content_type,
            character_set: character_set.into(),
            policy_container: DocumentPolicyContainer::default(),
        }
    }

    pub(crate) fn html(url: Url, markup: String) -> Self {
        Self::new(url, markup, Some("text/html".to_owned()))
    }

    pub(crate) fn about_blank(fallback_base_url: Url) -> Self {
        Self::html(
            Url::parse("about:blank").expect("static about:blank should parse"),
            "<!DOCTYPE html><html><head></head><body></body></html>".to_owned(),
        )
        .with_fallback_base_url(fallback_base_url)
    }

    pub(crate) fn srcdoc(fallback_base_url: Url, markup: String) -> Self {
        // srcdoc supplies Unicode markup, serialized as UTF-8 for navigation.
        Self::with_character_set(
            Url::parse("about:srcdoc").expect("static about:srcdoc should parse"),
            markup,
            Some("text/html".to_owned()),
            "UTF-8",
        )
        .with_fallback_base_url(fallback_base_url)
    }

    pub(crate) fn with_fallback_base_url(mut self, fallback_base_url: Url) -> Self {
        self.fallback_base_url = Some(fallback_base_url);
        self
    }

    pub(crate) fn with_resource_was_cached(mut self, resource_was_cached: bool) -> Self {
        self.resource_was_cached = resource_was_cached;
        self
    }

    pub(crate) fn with_policy_container(
        mut self,
        policy_container: DocumentPolicyContainer,
    ) -> Self {
        self.policy_container = policy_container;
        self
    }
}

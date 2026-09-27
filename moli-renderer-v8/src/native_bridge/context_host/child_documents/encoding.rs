use super::super::JsContextHost;
use crate::document_runtime::DomHandle;
use moli_url::WebOrigin;
use url::Url;

/// The container Document's encoding and origin, captured for this navigation.
/// An opener is not a container, and a response can introduce another origin
/// through a redirect or CSP sandboxing.
#[derive(Default)]
pub(super) struct ChildDocumentEncodingContext {
    parent: Option<(WebOrigin, String)>,
    sandbox_forces_opaque_origin: bool,
}

impl ChildDocumentEncodingContext {
    pub(super) fn fallback_encoding(
        &self,
        response_url: &Url,
        content_type: Option<&str>,
        response_forces_opaque_origin: bool,
    ) -> Option<&str> {
        if content_type.is_some_and(moli_web_mime::is_dom_parser_xml_mime) {
            return Some("UTF-8");
        }
        if self.sandbox_forces_opaque_origin || response_forces_opaque_origin {
            return None;
        }
        let (origin, charset) = self.parent.as_ref()?;
        if !origin.same_origin_url(response_url) {
            return None;
        }
        let encoding = moli_encoding::encoding_for_label(charset)?;
        // HTML permits inheritance from the same-origin container except for
        // UTF-16, which would reinterpret the child's markup byte structure.
        (!matches!(encoding.name(), "UTF-16LE" | "UTF-16BE")).then_some(charset.as_str())
    }
}

impl JsContextHost {
    pub(super) fn child_document_container_character_set(&self, handle: DomHandle) -> Option<&str> {
        let parent = self.dom_host().node(handle)?.owner_document()?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_character_set_for_handle(parent)
    }

    pub(super) fn child_document_encoding_context(
        &self,
        handle: DomHandle,
    ) -> ChildDocumentEncodingContext {
        let parent = self
            .owner_dispatch_scope_for_node(handle)
            .and_then(|owner| self.window_access_origin_for_dispatch_scope(owner))
            .zip(self.child_document_container_character_set(handle))
            .map(|(origin, charset)| {
                (
                    WebOrigin::from_serialized(&origin.serialized_origin()),
                    charset.to_owned(),
                )
            });
        ChildDocumentEncodingContext {
            parent,
            sandbox_forces_opaque_origin: self
                .child_browsing_context_sandbox_policy_from_owner(handle)
                .forces_opaque_origin,
        }
    }
}

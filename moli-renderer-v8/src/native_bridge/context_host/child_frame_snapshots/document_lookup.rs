use super::super::PageScriptHost;
use crate::dom::native::{
    HtmlEmbeddedDocument, HtmlEmbeddedDocumentProvider, HtmlSerializationError,
    HtmlSerializationPolicy,
};
use std::borrow::Cow;

impl HtmlEmbeddedDocumentProvider for PageScriptHost {
    fn embedded_document(
        &self,
        owner: crate::document_runtime::DomHandle,
    ) -> Option<HtmlEmbeddedDocument<'_>> {
        let document = self.child_browsing_context_document_handle(owner)?;
        if !self.child_browsing_context_host_is_active(owner) {
            return None;
        }
        let current = self.frame_owner_current_child_snapshot(owner)?;
        if document != current.document_handle
            || !self
                .dom_host()
                .node(current.document_handle)
                .is_some_and(crate::dom::native::Node::is_document)
        {
            return None;
        }
        Some(HtmlEmbeddedDocument {
            document: current.document_handle,
            url: Cow::Owned(current.document_url.into()),
        })
    }
}

impl PageScriptHost {
    pub(crate) fn document_markup_for_export(
        &self,
        document: crate::document_runtime::DomHandle,
        policy: Option<&dyn HtmlSerializationPolicy>,
        with_frames: bool,
        max_bytes: Option<usize>,
    ) -> Result<String, HtmlSerializationError> {
        let scripting = |node| self.node_document_scripting_enabled(node);
        self.dom_host().get_html_for_export(
            document,
            &scripting,
            policy,
            (with_frames && !self.child_browsing_context_document_handles.is_empty())
                .then_some(self as &dyn HtmlEmbeddedDocumentProvider),
            max_bytes,
        )
    }
}

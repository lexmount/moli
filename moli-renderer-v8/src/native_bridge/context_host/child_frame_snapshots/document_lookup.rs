use super::super::PageScriptHost;
use super::ChildBrowsingContextDocumentSnapshot;
use crate::dom::native::{NativeDom, Node};

impl PageScriptHost {
    /// Serializes a loaded child frame's Document from `dom`, an export copy
    /// of the live DOM, with the child Document's shadow roots and scripting
    /// flag, as the live child snapshot does.
    pub(crate) fn child_browsing_context_document_snapshot_from_copy(
        &self,
        frame_id: &str,
        dom: &NativeDom,
        include_shadow_style_sheets: bool,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        let handle = self.child_browsing_context_owner_node_id_by_frame_id(frame_id)?;
        let document_handle = self.child_browsing_context_document_handle(handle)?;
        let url = self.child_browsing_context_current_url(handle)?;
        let markup =
            self.document_markup_from_copy(document_handle, dom, include_shadow_style_sheets)?;
        Some(ChildBrowsingContextDocumentSnapshot {
            url: url.as_str().to_owned(),
            markup,
        })
    }

    /// Serializes `document_handle` (the page or a child frame Document) from
    /// `dom`, an export copy of the live DOM, including that Document's
    /// shadow roots and with its scripting flag.
    pub(crate) fn document_markup_from_copy(
        &self,
        document_handle: crate::document_runtime::DomHandle,
        dom: &NativeDom,
        include_shadow_style_sheets: bool,
    ) -> Option<String> {
        let explicit_shadow_roots = self
            .dom_host()
            .snapshot_shadow_root_bindings()
            .into_iter()
            .filter_map(|binding| {
                (self
                    .dom_host()
                    .node(binding.host)
                    .and_then(Node::owner_document)
                    == Some(document_handle))
                .then_some(binding.root)
            })
            .collect::<Vec<_>>();
        let scripting_enabled_for_node = |node| self.node_document_scripting_enabled(node);
        self.dom_host().get_html_from_copy(
            dom,
            document_handle,
            &scripting_enabled_for_node,
            &explicit_shadow_roots,
            include_shadow_style_sheets,
        )
    }
}

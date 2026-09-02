use super::super::JsContextHost;
use super::{ChildBrowsingContextDocumentSnapshot, MAX_CHILD_FRAME_TREE_DEPTH};
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use url::Url;

impl JsContextHost {
    pub(crate) fn child_browsing_context_document_snapshot_by_frame_id(
        &mut self,
        frame_id: &str,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        if let Some(owner_node_id) = self.child_browsing_context_owner_node_id_by_frame_id(frame_id)
        {
            if let Some(snapshot) =
                self.live_child_browsing_context_document_snapshot(owner_node_id)
            {
                return Some(snapshot);
            }
            let snapshot = self.child_browsing_context_snapshot_markup(owner_node_id)?;
            return Some(ChildBrowsingContextDocumentSnapshot {
                url: snapshot.url.as_str().to_owned(),
                markup: snapshot.markup,
            });
        }

        for handle in self.child_browsing_context_handles_in_document_order() {
            let Some(entry) = self.child_browsing_contexts.get(&handle) else {
                continue;
            };
            let parent_frame_id = entry.frame_id().to_owned();
            let Some(snapshot) = self.child_browsing_context_snapshot_markup(handle) else {
                continue;
            };
            let scripting_enabled = self.child_browsing_context_scripting_enabled(handle);
            if let Some(snapshot) = self.find_detached_child_document_snapshot_by_frame_id(
                &parent_frame_id,
                frame_id,
                &snapshot.url,
                &snapshot.markup,
                scripting_enabled,
                0,
            ) {
                return Some(snapshot);
            }
        }

        None
    }

    fn live_child_browsing_context_document_snapshot(
        &self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        let document_handle = self.child_browsing_context_document_handle(handle)?;
        let url = self.child_browsing_context_current_url(handle)?;
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
        let markup = self.dom_host().get_html(
            document_handle,
            self.child_browsing_context_scripting_enabled(handle),
            false,
            &explicit_shadow_roots,
        )?;
        Some(ChildBrowsingContextDocumentSnapshot {
            url: url.as_str().to_owned(),
            markup,
        })
    }

    fn find_detached_child_document_snapshot_by_frame_id(
        &mut self,
        parent_frame_id: &str,
        target_frame_id: &str,
        document_url: &Url,
        markup: &str,
        scripting_enabled: bool,
        depth: usize,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        if depth >= MAX_CHILD_FRAME_TREE_DEPTH {
            return None;
        }

        let document = crate::parser::HtmlParser::with_scripting_enabled(scripting_enabled)
            .parse(document_url.clone(), markup.to_owned());
        let document_base_url = document
            .document()
            .map(|doc| doc.base_url().clone())
            .unwrap_or_else(|| document_url.clone());
        self.find_detached_child_document_snapshot_by_frame_id_in_document(
            &document,
            document.document_node_id(),
            &document_base_url,
            parent_frame_id,
            target_frame_id,
            scripting_enabled,
            depth,
        )
    }

    fn find_detached_child_document_snapshot_by_frame_id_in_document(
        &mut self,
        document: &crate::dom::native::NativeDom,
        root: DomHandle,
        document_base_url: &Url,
        parent_frame_id: &str,
        target_frame_id: &str,
        document_scripting_enabled: bool,
        depth: usize,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        let mut ordinal = 0usize;
        self.find_detached_child_document_snapshot_by_frame_id_in_subtree(
            document,
            root,
            document_base_url,
            parent_frame_id,
            target_frame_id,
            document_scripting_enabled,
            depth,
            &mut ordinal,
        )
    }

    fn find_detached_child_document_snapshot_by_frame_id_in_subtree(
        &mut self,
        document: &crate::dom::native::NativeDom,
        root: DomHandle,
        document_base_url: &Url,
        parent_frame_id: &str,
        target_frame_id: &str,
        document_scripting_enabled: bool,
        depth: usize,
        ordinal: &mut usize,
    ) -> Option<ChildBrowsingContextDocumentSnapshot> {
        for child in document.child_ids(root) {
            if self.is_detached_child_browsing_context_host_handle(document, child) {
                *ordinal += 1;
                let frame_id = format!("{parent_frame_id}/child-{}", *ordinal);
                let bootstrap = Self::detached_child_browsing_context_bootstrap(
                    document,
                    child,
                    document_base_url,
                );
                let snapshot = self.detached_child_browsing_context_snapshot_markup(
                    &bootstrap,
                    document_base_url,
                )?;
                let scripting_enabled = Self::detached_child_document_scripting_enabled(
                    document,
                    child,
                    document_scripting_enabled,
                    &snapshot,
                );
                if frame_id == target_frame_id {
                    return Some(ChildBrowsingContextDocumentSnapshot {
                        url: snapshot.url.as_str().to_owned(),
                        markup: snapshot.markup,
                    });
                }
                if let Some(found) = self.find_detached_child_document_snapshot_by_frame_id(
                    &frame_id,
                    target_frame_id,
                    &snapshot.url,
                    &snapshot.markup,
                    scripting_enabled,
                    depth + 1,
                ) {
                    return Some(found);
                }
                continue;
            }
            if let Some(found) = self.find_detached_child_document_snapshot_by_frame_id_in_subtree(
                document,
                child,
                document_base_url,
                parent_frame_id,
                target_frame_id,
                document_scripting_enabled,
                depth,
                ordinal,
            ) {
                return Some(found);
            }
        }
        None
    }
}

use super::super::{ChildBrowsingContextBootstrap, JsContextHost, child_frames::WebStorageScope};
use super::ChildBrowsingContextFrameSnapshot;
use url::Url;

pub(in crate::native_bridge::context_host::child_frame_snapshots) fn child_browsing_context_bootstrap_url_inherits_security_origin(
    bootstrap: &ChildBrowsingContextBootstrap,
    url: &Url,
) -> bool {
    bootstrap.security_origin_inherited() || moli_url::is_about_blank(url)
}

impl JsContextHost {
    pub(crate) fn child_browsing_context_child_frame_count(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) -> usize {
        if self
            .child_browsing_context_document_handle(handle)
            .is_some()
        {
            return self
                .child_browsing_context_direct_host_handles(handle)
                .into_iter()
                .filter(|child| {
                    self.dom_host()
                        .node(*child)
                        .is_some_and(|node| node.flags().in_document_tree())
                })
                .count();
        }
        self.child_browsing_context_snapshot_markup(handle)
            .map(|snapshot| {
                let parent_scope = self
                    .child_browsing_context_web_storage_scope(
                        handle,
                        &moli_url::origin_ascii_serialization(self.document_url()),
                    )
                    .unwrap_or_else(|| self.top_web_storage_scope());
                self.detached_child_browsing_context_frame_tree_snapshot(
                    "",
                    &snapshot.url,
                    &snapshot.markup,
                    self.child_browsing_context_scripting_enabled(handle),
                    0,
                    parent_scope,
                )
                .len()
            })
            .unwrap_or(0)
    }

    pub(crate) fn child_browsing_context_child_frame_named_indices(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) -> Vec<(usize, String)> {
        if self
            .child_browsing_context_document_handle(handle)
            .is_some()
        {
            return self
                .child_browsing_context_direct_host_handles(handle)
                .into_iter()
                .filter(|child| {
                    self.dom_host()
                        .node(*child)
                        .is_some_and(|node| node.flags().in_document_tree())
                })
                .enumerate()
                .filter_map(|(index, child_handle)| {
                    self.dom_host()
                        .get_attribute(child_handle, "name")
                        .filter(|name| !name.is_empty())
                        .map(|name| (index, name))
                })
                .collect();
        }
        self.child_browsing_context_snapshot_markup(handle)
            .map(|snapshot| {
                let parent_scope = self
                    .child_browsing_context_web_storage_scope(
                        handle,
                        &moli_url::origin_ascii_serialization(self.document_url()),
                    )
                    .unwrap_or_else(|| self.top_web_storage_scope());
                self.detached_child_browsing_context_frame_tree_snapshot(
                    "",
                    &snapshot.url,
                    &snapshot.markup,
                    self.child_browsing_context_scripting_enabled(handle),
                    0,
                    parent_scope,
                )
                .into_iter()
                .enumerate()
                .filter_map(|(index, snapshot)| snapshot.name.map(|name| (index, name)))
                .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn child_browsing_context_frame_tree_snapshot(
        &mut self,
    ) -> Vec<ChildBrowsingContextFrameSnapshot> {
        let parent_scope = self.top_web_storage_scope();
        self.top_level_child_browsing_context_handles_in_frame_tree_order()
            .into_iter()
            .filter_map(|handle| {
                self.live_child_browsing_context_frame_tree_snapshot(
                    handle,
                    0,
                    parent_scope.clone(),
                )
            })
            .collect()
    }

    fn live_child_browsing_context_frame_tree_snapshot(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> Option<ChildBrowsingContextFrameSnapshot> {
        if depth >= super::MAX_CHILD_FRAME_TREE_DEPTH {
            return None;
        }
        let identity = {
            let entry = self.child_browsing_contexts.get(&handle)?;
            (
                entry.frame_identity_snapshot(),
                entry.current_document_loader_id()?.to_owned(),
            )
        };
        let (identity, loader_id) = identity;
        let url = self.child_browsing_context_current_url(handle)?;
        let security_origin_opaque = self.child_browsing_context_has_opaque_origin(handle);
        let storage_scope =
            self.child_browsing_context_web_storage_scope_with_parent(handle, parent_scope)?;
        let storage_key = storage_scope.storage_key().serialized_storage_key();
        let child_handles = self.child_browsing_context_child_frame_handles(handle);
        let child_frames = if child_handles.is_empty()
            && self
                .child_browsing_context_document_handle(handle)
                .is_none()
        {
            self.child_browsing_context_snapshot_markup(handle)
                .map(|snapshot| {
                    self.detached_child_browsing_context_frame_tree_snapshot(
                        &identity.frame_id,
                        &snapshot.url,
                        &snapshot.markup,
                        self.child_browsing_context_scripting_enabled(handle),
                        depth,
                        storage_scope.clone(),
                    )
                })
                .unwrap_or_default()
        } else {
            child_handles
                .into_iter()
                .filter_map(|child_handle| {
                    self.live_child_browsing_context_frame_tree_snapshot(
                        child_handle,
                        depth + 1,
                        storage_scope.clone(),
                    )
                })
                .collect()
        };
        Some(ChildBrowsingContextFrameSnapshot {
            frame_id: identity.frame_id,
            loader_id,
            name: identity.name,
            owner_element_id: identity.owner_element_id,
            url: url.as_str().to_owned(),
            storage_key,
            security_origin_inherited: identity.security_origin_inherited,
            security_origin_opaque,
            child_frames,
        })
    }
}

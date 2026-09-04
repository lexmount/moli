use super::*;
use crate::dom::native::Node;
use std::collections::HashSet;

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn child_browsing_context_has_cached_snapshot_for_test(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(ChildBrowsingContextEntry::has_cached_snapshot)
    }

    pub(crate) fn child_browsing_context_count(&self) -> usize {
        self.top_level_child_browsing_context_handles_in_frame_tree_order()
            .len()
    }

    pub(crate) fn child_browsing_context_is_on_initial_about_blank_entry(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_current_document_is_initial_empty(handle)
            && self
                .child_browsing_context_current_url(handle)
                .is_some_and(|url| moli_url::is_about_blank(&url))
    }

    pub(crate) fn child_browsing_context_count_for_document(&self, document: DomHandle) -> usize {
        self.window_child_browsing_context_handles_for_document(document)
            .len()
    }

    pub(crate) fn child_browsing_context_handles_in_document_order(&self) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        self.collect_child_browsing_context_handles_in_document_order_from_document(
            self.document_handle(),
            &mut handles,
        );
        handles
    }

    pub(crate) fn live_child_browsing_context_owner_snapshots(
        &self,
    ) -> Vec<(DomHandle, ChildFrameOwnerSnapshot)> {
        let mut handles = self
            .child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| self.child_browsing_context_host_is_active(*handle))
            .collect::<Vec<_>>();
        handles.sort_unstable_by_key(|handle| handle.index());
        handles
            .into_iter()
            .filter_map(|handle| {
                self.frame_owner_current_child_snapshot(handle)
                    .map(|owner| (handle, owner))
            })
            .collect()
    }

    pub(crate) fn child_browsing_context_popup_owner_id(&self, handle: DomHandle) -> Option<u64> {
        let owner_document = self
            .dom_host()
            .node(handle)
            .and_then(|node| node.owner_document())?;
        self.lightweight_popup_id_for_document_handle(owner_document)
    }

    fn collect_child_browsing_context_handles_in_document_order_from_document(
        &self,
        document: DomHandle,
        out: &mut Vec<DomHandle>,
    ) {
        let mut visited_documents = HashSet::new();
        let mut visited_handles = HashSet::new();
        self.collect_child_browsing_context_handles_in_document_order_from_document_inner(
            document,
            out,
            &mut visited_documents,
            &mut visited_handles,
        );
    }

    fn collect_child_browsing_context_handles_in_document_order_from_document_inner(
        &self,
        document: DomHandle,
        out: &mut Vec<DomHandle>,
        visited_documents: &mut HashSet<DomHandle>,
        visited_handles: &mut HashSet<DomHandle>,
    ) {
        if !visited_documents.insert(document) {
            return;
        }
        let mut local_handles = Vec::new();
        self.collect_child_browsing_context_host_handles(document, &mut local_handles);
        local_handles.retain(|handle| self.child_browsing_contexts.contains_key(handle));
        for handle in local_handles {
            if !visited_handles.insert(handle) {
                continue;
            }
            out.push(handle);
            if let Some(child_document) = self.child_browsing_context_document_handle(handle) {
                self.collect_child_browsing_context_handles_in_document_order_from_document_inner(
                    child_document,
                    out,
                    visited_documents,
                    visited_handles,
                );
            }
        }
    }

    pub(crate) fn top_level_child_browsing_context_handles_in_document_order(
        &self,
    ) -> Vec<DomHandle> {
        self.child_browsing_context_handles_in_document_order()
            .into_iter()
            .filter(|handle| self.child_browsing_context_parent_handle(*handle).is_none())
            .collect()
    }

    pub(crate) fn top_level_child_browsing_context_handles_in_frame_tree_order(
        &self,
    ) -> Vec<DomHandle> {
        self.child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| self.child_browsing_context_parent_handle(*handle).is_none())
            .collect()
    }

    fn child_browsing_context_direct_frame_handles_for_document(
        &self,
        document: DomHandle,
    ) -> Vec<DomHandle> {
        self.child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| {
                self.dom_host().node(*handle).and_then(Node::owner_document) == Some(document)
            })
            .collect()
    }

    fn window_child_browsing_context_handles_for_document(
        &self,
        document: DomHandle,
    ) -> Vec<DomHandle> {
        // A frame inside a shadow tree still owns a browsing context, but it is
        // not exposed through the containing Window's indexed properties.
        self.child_browsing_context_direct_frame_handles_for_document(document)
            .into_iter()
            .filter(|handle| {
                self.dom_host()
                    .node(*handle)
                    .is_some_and(|node| node.flags().in_document_tree())
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn child_browsing_context_handle_by_index(&self, index: usize) -> Option<DomHandle> {
        // Tests use this to inspect the global frame-tree projection rather
        // than one Window's scoped indexed properties.
        self.top_level_child_browsing_context_handles_in_frame_tree_order()
            .into_iter()
            .nth(index)
    }

    pub(crate) fn child_browsing_context_handle_by_index_for_document(
        &self,
        document: DomHandle,
        index: usize,
    ) -> Option<DomHandle> {
        // Window indexed/named interceptors hit this on every miss.
        if self.child_browsing_contexts.is_empty() {
            return None;
        }
        self.window_child_browsing_context_handles_for_document(document)
            .into_iter()
            .nth(index)
    }

    pub(crate) fn child_browsing_context_child_frame_handles(
        &self,
        parent: DomHandle,
    ) -> Vec<DomHandle> {
        self.child_browsing_context_document_handle(parent)
            .map(|document| self.child_browsing_context_direct_frame_handles_for_document(document))
            .unwrap_or_default()
    }

    pub(crate) fn child_browsing_context_child_frame_handle_by_index(
        &self,
        parent: DomHandle,
        index: usize,
    ) -> Option<DomHandle> {
        let document = self.child_browsing_context_document_handle(parent)?;
        self.child_browsing_context_handle_by_index_for_document(document, index)
    }

    pub(crate) fn child_browsing_context_direct_host_handles(
        &self,
        parent: DomHandle,
    ) -> Vec<DomHandle> {
        let Some(document) = self.child_browsing_context_document_handle(parent) else {
            return Vec::new();
        };
        let mut handles = Vec::new();
        self.collect_child_browsing_context_host_handles(document, &mut handles);
        handles.retain(|handle| {
            self.dom_host().node(*handle).and_then(Node::owner_document) == Some(document)
        });
        handles
    }

    pub(crate) fn child_browsing_context_parent_handle(
        &self,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        let owner_document = self
            .dom_host()
            .node(handle)
            .and_then(Node::owner_document)?;
        if owner_document == self.document_handle() {
            return None;
        }
        self.child_browsing_context_document_handles
            .iter()
            .find_map(|(child_handle, document_handle)| {
                (*document_handle == owner_document).then_some(*child_handle)
            })
    }

    pub(crate) fn child_browsing_context_handle_by_name(&self, key: &str) -> Option<DomHandle> {
        // Keep the common no-frame named-property miss cheap.
        if self.child_browsing_contexts.is_empty() {
            return None;
        }
        if !self.child_browsing_context_name_exists(key) {
            return None;
        }
        self.child_browsing_context_handles_in_document_order()
            .into_iter()
            .find(|handle| {
                self.child_browsing_contexts
                    .get(handle)
                    .is_some_and(|entry| entry.matches_browsing_context_name(key))
            })
    }

    pub(crate) fn child_browsing_context_named_child_handle(
        &self,
        parent: Option<DomHandle>,
        key: &str,
    ) -> Option<DomHandle> {
        // Window named access only searches direct scoped children. A full
        // frame-tree search can rediscover the receiver (for example when a
        // nested frame is named `document`) and recursively materialize it.
        let handles = match parent {
            Some(parent) => self.child_browsing_context_child_frame_handles(parent),
            None => self.top_level_child_browsing_context_handles_in_frame_tree_order(),
        };
        handles.into_iter().find(|handle| {
            self.dom_host()
                .node(*handle)
                .is_some_and(|node| node.flags().in_document_tree())
                && self
                    .child_browsing_contexts
                    .get(handle)
                    .is_some_and(|entry| entry.matches_browsing_context_name(key))
        })
    }

    fn child_browsing_context_handle_by_name_in_document_order_from_document(
        &self,
        key: &str,
        document: DomHandle,
    ) -> Option<DomHandle> {
        if !self.child_browsing_context_name_exists(key) {
            return None;
        }
        let mut handles = Vec::new();
        self.collect_child_browsing_context_handles_in_document_order_from_document(
            document,
            &mut handles,
        );
        handles.into_iter().find(|handle| {
            self.child_browsing_contexts
                .get(handle)
                .is_some_and(|entry| entry.matches_browsing_context_name(key))
        })
    }

    pub(crate) fn child_browsing_context_handle_by_name_for_navigation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        key: &str,
    ) -> Option<DomHandle> {
        self.sync_child_browsing_context_subtree(scope, self.document_handle());
        self.child_browsing_context_handle_by_name(key)
    }

    pub(crate) fn child_browsing_context_handle_by_name_for_navigation_from_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        key: &str,
        document: DomHandle,
    ) -> Option<DomHandle> {
        self.sync_child_browsing_context_subtree(scope, document);
        self.child_browsing_context_handle_by_name_in_document_order_from_document(key, document)
    }

    pub(crate) fn child_browsing_context_handle_by_document_handle(
        &self,
        _scope: &mut v8::PinScope<'_, '_>,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.child_browsing_context_document_handles
            .iter()
            .find_map(|(child_handle, child_document)| {
                (*child_document == document_handle).then_some(*child_handle)
            })
    }

    fn child_browsing_context_name_exists(&self, key: &str) -> bool {
        self.child_browsing_contexts
            .values()
            .any(|entry| entry.matches_browsing_context_name(key))
    }

    pub(crate) fn child_browsing_context_document_handle(
        &self,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        self.child_browsing_context_document_handles
            .get(&handle)
            .copied()
    }

    pub(crate) fn child_browsing_context_host_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.child_browsing_context_document_handles
            .iter()
            .find_map(|(child_handle, child_document)| {
                (*child_document == document_handle
                    && self.child_browsing_contexts.contains_key(child_handle))
                .then_some(*child_handle)
            })
    }

    pub(crate) fn child_browsing_context_host_is_ancestor_of_document(
        &self,
        ancestor: DomHandle,
        document_handle: DomHandle,
    ) -> bool {
        let mut current_document = Some(document_handle);
        let mut visited_documents = HashSet::new();
        while let Some(document) = current_document {
            if !visited_documents.insert(document) {
                return false;
            }
            let Some(host) = self.child_browsing_context_host_for_document_handle(document) else {
                return false;
            };
            if host == ancestor {
                return true;
            }
            current_document = self.dom_host().node(host).and_then(Node::owner_document);
        }
        false
    }

    pub(crate) fn child_browsing_context_character_set_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<&str> {
        let child_handle = self.child_browsing_context_host_for_document_handle(document_handle)?;
        if let Some(character_set) = self
            .child_browsing_contexts
            .get(&child_handle)
            .and_then(|entry| entry.cached_snapshot_ref())
            .map(|snapshot| snapshot.character_set.as_str())
        {
            return Some(character_set);
        }
        self.child_current_document_is_initial_empty(child_handle)
            .then_some("UTF-8")
    }

    pub(crate) fn child_browsing_context_referrer_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<&str> {
        self.child_browsing_context_host_for_document_handle(document_handle)
            .and_then(|child_handle| self.child_browsing_contexts.get(&child_handle))
            .map(|entry| entry.document_referrer())
    }

    pub(crate) fn child_browsing_context_referrer_policy_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<&str> {
        self.child_browsing_context_host_for_document_handle(document_handle)
            .and_then(|child_handle| self.child_browsing_contexts.get(&child_handle))
            .and_then(|entry| entry.document_referrer_policy())
    }

    pub(crate) fn child_browsing_context_response_referrer_policy(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        self.child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.document_referrer_policy().map(ToOwned::to_owned))
    }

    pub(crate) fn child_browsing_context_policy_container_snapshot(
        &self,
        handle: DomHandle,
    ) -> Option<crate::document_runtime::DocumentPolicyContainer> {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.document_policy_container_snapshot())
    }

    pub(crate) fn child_browsing_context_parent_frame_id(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        let owner_document = self
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::owner_document)?;
        if owner_document == self.document_handle() {
            return self
                .frame_owner_store
                .current_main_owner_snapshot()
                .map(|snapshot| snapshot.frame_id.0);
        }
        self.child_browsing_context_document_handles
            .iter()
            .find_map(|(child_handle, document_handle)| {
                (*document_handle == owner_document)
                    .then(|| self.child_browsing_contexts.get(child_handle))
                    .flatten()
                    .map(|entry| entry.frame_id().to_owned())
            })
    }

    pub(in crate::native_bridge::context_host) fn clear_child_browsing_context_pending_document_load_if_matches(
        &mut self,
        handle: DomHandle,
        load_id: u64,
    ) {
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle)
            && entry.pending_document_load_matches(load_id)
        {
            entry.clear_pending_document_load();
        }
    }

    pub(in crate::native_bridge::context_host) fn set_child_browsing_context_pending_navigation(
        &mut self,
        handle: DomHandle,
        bootstrap: ChildBrowsingContextBootstrap,
        initiator_url: Option<Url>,
        reflects_window_state: bool,
    ) -> Option<crate::frame_owner_model::FrameDocumentNavigationLoadBinding> {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        };
        let ancestor_origins_referrer_policy =
            self.child_ancestor_origins_referrer_policy_from_owner_attribute(handle);
        let permissions_policy =
            self.child_browsing_context_permissions_policy_for_navigation(handle, &bootstrap);
        let Some(navigation) = self.replace_child_navigation_load(handle) else {
            tracing::warn!(
                ?handle,
                "refusing child navigation without a current document lifecycle owner"
            );
            return None;
        };
        let entry = self.child_browsing_contexts.get_mut(&handle)?;
        entry.set_document_permissions_policy(permissions_policy);
        entry.set_ancestor_origins_referrer_policy_snapshot(ancestor_origins_referrer_policy);
        entry.set_pending_navigation(bootstrap, initiator_url, reflects_window_state);
        self.note_child_frame_load_started_for_parent(handle);
        Some(navigation)
    }

    pub(in crate::native_bridge::context_host) fn clear_child_browsing_context_pending_navigation(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return false;
        };
        entry.clear_pending_navigation();
        true
    }

    pub(crate) fn child_browsing_context_is_live(&self, handle: DomHandle) -> bool {
        self.child_browsing_contexts.contains_key(&handle)
            && self.child_browsing_context_host_is_active(handle)
    }

    pub(crate) fn bind_child_default_execution_context_id(
        &mut self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        execution_context_id: i64,
    ) -> Option<FrameRealmId> {
        let realm_id = self.frame_owner_store.bind_child_realm_inspector_context(
            handle,
            owner,
            execution_context_id,
        )?;
        self.child_window_proxy_records
            .set_default_execution_context_id(handle, execution_context_id);
        Some(realm_id)
    }

    pub(crate) fn complete_child_default_realm_materialization(
        &mut self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        self.frame_owner_store
            .complete_child_realm_materialization(handle, owner, realm_id)
    }

    #[cfg(test)]
    pub(crate) fn clear_child_default_execution_context_id(&mut self, handle: DomHandle) {
        self.child_window_proxy_records
            .clear_default_execution_context_id(handle);
        self.frame_owner_store.clear_child_realm(handle);
    }

    pub(crate) fn clear_child_default_execution_context_if_matches(
        &mut self,
        handle: DomHandle,
        expected_realm_id: FrameRealmId,
        expected_execution_context_id: i64,
    ) {
        self.child_window_proxy_records
            .clear_default_execution_context_id_if_matches(handle, expected_execution_context_id);
        self.frame_owner_store
            .clear_child_realm_if_matches(handle, expected_realm_id);
    }

    pub(crate) fn child_default_execution_context_id(&self, handle: DomHandle) -> Option<i64> {
        self.child_window_proxy_records
            .default_execution_context_id(handle)
    }

    pub(crate) fn frame_owner_frame_id_for_child_handle(
        &self,
        handle: DomHandle,
    ) -> Option<FrameId> {
        self.frame_owner_store
            .frame_id_for_child_handle(handle)
            .cloned()
    }

    pub(crate) fn frame_owner_current_child_snapshot(
        &self,
        handle: DomHandle,
    ) -> Option<ChildFrameOwnerSnapshot> {
        self.frame_owner_store.current_child_owner_snapshot(handle)
    }

    pub(crate) fn frame_owner_current_child_snapshot_for_realm(
        &self,
        realm_id: FrameRealmId,
    ) -> Option<ChildFrameOwnerSnapshot> {
        self.frame_owner_store
            .current_child_owner_snapshot_for_realm(realm_id)
    }

    pub(crate) fn frame_owner_current_realm_id_for_script_job(
        &self,
        job: &FrameScriptJob,
    ) -> Option<FrameRealmId> {
        self.frame_owner_store
            .current_realm_id_for_frame_script_job(job)
    }

    pub(crate) fn frame_owner_child_handle_for_script_job(
        &self,
        job: &FrameScriptJob,
    ) -> Option<DomHandle> {
        self.frame_owner_store
            .child_handle_for_frame_script_job(job)
    }

    pub(crate) fn frame_owner_child_source_script_job(
        &self,
        handle: DomHandle,
        kind: FrameScriptJobKind,
        source: String,
    ) -> Option<FrameScriptJob> {
        self.frame_owner_store
            .child_source_script_job(handle, kind, source)
    }

    pub(crate) fn frame_owner_child_parser_classic_script_job(
        &self,
        handle: DomHandle,
        current_script: Option<DomHandle>,
        source: String,
    ) -> Option<FrameScriptJob> {
        self.frame_owner_store
            .child_parser_classic_script_job(handle, current_script, source)
    }

    #[cfg(test)]
    pub(crate) fn frame_owner_child_function_constructor_script_job(
        &self,
        handle: DomHandle,
        parameters: Vec<String>,
        body: String,
    ) -> Option<FrameScriptJob> {
        self.frame_owner_store
            .child_function_constructor_script_job(handle, parameters, body)
    }
}

use super::super::{ChildBrowsingContextSnapshot, JsContextHost, NavigationHistoryEntrySeed};
use crate::document_runtime::DomHandle;
use crate::frame_owner_model::DocumentCreationKind;

impl JsContextHost {
    pub(crate) fn cache_child_browsing_context_snapshot(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        snapshot: Option<ChildBrowsingContextSnapshot>,
    ) {
        if self
            .frame_owner_store
            .current_child_document_task_owner(handle)
            .is_some()
        {
            tracing::warn!(
                ?handle,
                "refusing direct child snapshot replacement outside NavigationCommit"
            );
            return;
        }
        self.clear_pending_child_document_loads_for_handle(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.set_cached_snapshot(snapshot.clone());
            if snapshot.is_some() {
                entry.reset_performance_time_origin();
            }
            entry.clear_document_runtime_state();
        }
        let window_commit_preflight = self.capture_child_document_window_commit_preflight(handle);
        self.disconnect_shared_worker_clients_for_child_context(handle);
        if let Some(snapshot) = snapshot.as_ref() {
            let loader_id = self.allocate_child_document_loader_id();
            let window_commit = self.plan_child_document_window_commit(
                handle,
                snapshot,
                window_commit_preflight,
                DocumentCreationKind::Navigation,
                Some(loader_id),
            );
            let install = self.install_child_browsing_context_current_document_from_snapshot(
                scope,
                handle,
                snapshot,
                window_commit,
                false,
                None,
            );
            debug_assert!(
                install
                    .as_ref()
                    .is_none_or(|install| { install.owner_transition.retired_owner().is_none() })
            );
        } else {
            self.clear_child_browsing_context_current_document(handle);
            self.finish_child_frame_without_current_document_load_dispatch(handle);
        }
    }

    pub(crate) fn clear_child_browsing_context_cached_snapshot_for_navigation(
        &mut self,
        handle: DomHandle,
    ) {
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.clear_cached_snapshot();
        }
    }

    pub(crate) fn set_child_browsing_context_navigation_entry_seed(
        &mut self,
        handle: DomHandle,
        entry_seed: NavigationHistoryEntrySeed,
    ) -> bool {
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return false;
        };
        let committed = entry.set_navigation_entry_seed(entry_seed);
        if committed {
            self.remember_child_history(handle);
        }
        committed
    }

    pub(crate) fn pending_child_browsing_context_navigation_position(
        &self,
        handle: DomHandle,
    ) -> Option<(u32, u32)> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        entry.pending_navigation_position()
    }
}

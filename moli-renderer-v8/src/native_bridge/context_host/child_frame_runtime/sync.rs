use super::super::JsContextHost;
use super::document_slots::sync_child_document_window_slots;
use crate::{
    context_bootstrap::{
        bind_window_performance_seed, install_navigation_bootstrap_entry_for_holder,
        reset_window_location_history_navigation_runtime_state, set_window_origin_runtime_state,
        sync_window_location_history_navigation_runtime_surface,
    },
    document_runtime::DomHandle,
    native_bridge::{
        child_window_surface::bind_materialized_child_window_indexed_db_factory,
        helpers::set_object_slot,
    },
    util::v8str,
};

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn sync_child_window_origin_slot<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        wrapper: v8::Local<'s, v8::Object>,
    ) {
        let Some(origin) = self.child_browsing_context_window_origin(handle) else {
            return;
        };
        let _ = set_window_origin_runtime_state(scope, wrapper, &origin);
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_window_origin(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        // The browsing-context entry may already expose a pending navigation's
        // target URL. The live Window remains owned by the committed Document
        // until NavigationCommit replaces or reuses its LocalWindow.
        self.frame_owner_store
            .current_child_owner_snapshot(handle)
            .map(|owner| owner.settings.origin)
    }

    pub(crate) fn sync_existing_child_browsing_context_window_state(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(wrapper) = self.child_window_proxy_records.live_window(scope, handle) else {
            return;
        };
        if self.child_window_proxy_records.window_proxy_exposed(handle)
            && !self.child_browsing_context_is_same_origin_with_top(handle)
        {
            let _ = self.ensure_top_exposed_cross_origin_window_proxy(scope, handle);
            let Some(wrapper) = self.child_browsing_context_window_wrapper(scope, handle) else {
                return;
            };
            self.sync_child_browsing_context_window_state_for_wrapper(scope, handle, wrapper);
            return;
        }
        self.sync_child_browsing_context_window_state_for_wrapper(scope, handle, wrapper);
    }

    fn sync_child_browsing_context_window_state_for_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        wrapper: v8::Local<'s, v8::Object>,
    ) {
        self.sync_child_browsing_context_window_parent_top_slots(scope, handle, wrapper);
        self.sync_child_window_origin_slot(scope, handle, wrapper);
        let Some(visible_state) =
            self.child_browsing_context_visible_window_navigation_state(handle)
        else {
            return;
        };
        let should_reset_runtime_state = wrapper
            .get(scope, v8str(scope, "location").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|location| {
                location
                    .get(scope, v8str(scope, "href").into())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
            })
            .is_none_or(|href| href != visible_state.href);
        if should_reset_runtime_state {
            let _ = reset_window_location_history_navigation_runtime_state(
                scope,
                wrapper,
                &visible_state.href,
            );
        } else {
            sync_window_location_history_navigation_runtime_surface(scope, wrapper);
        }
        install_navigation_bootstrap_entry_for_holder(scope, wrapper, &visible_state.entry_seed);
        let performance_navigation_type = self.child_performance_navigation_type(handle);
        let _ = bind_window_performance_seed(
            scope,
            wrapper,
            &performance_navigation_type,
            self.child_performance_time_origin(handle),
        );
        bind_materialized_child_window_indexed_db_factory(scope, wrapper, handle);
        let current_realm_owns_proxy = self
            .current_registered_window_execution_context_identity(
                crate::native_bridge::OwnerDispatchScope::Child(handle),
            )
            .is_some();
        let document = self
            .child_browsing_context_document_wrapper(scope, handle)
            .map(|document| {
                sync_child_document_window_slots(
                    scope,
                    document,
                    wrapper,
                    visible_state.seed_is_committed,
                );
                if current_realm_owns_proxy {
                    crate::context_bootstrap::sync_window_document_cached_accessor(
                        scope, wrapper, document,
                    );
                }
                document.into()
            })
            .unwrap_or_else(|| v8::null(scope).into());
        set_object_slot(scope, wrapper, "document", document);
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_visible_window_navigation_state(
        &self,
        handle: DomHandle,
    ) -> Option<super::super::child_frames::ChildBrowsingContextVisibleNavigationState> {
        self.child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.visible_window_navigation_state())
    }

    pub(crate) fn child_browsing_context_visible_url(&self, handle: DomHandle) -> Option<String> {
        self.child_browsing_context_visible_window_navigation_state(handle)
            .map(|state| state.href)
    }

    pub(crate) fn sync_existing_child_browsing_context_runtime_surface_from_seed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(wrapper) = self.child_window_proxy_records.live_window(scope, handle) else {
            return;
        };
        let Some(entry_seed) = self
            .child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.navigation_entry_seed())
        else {
            return;
        };
        let Some(current_entry) = entry_seed
            .entries
            .iter()
            .find(|entry| entry.history_index == entry_seed.current_index)
        else {
            return;
        };
        let _ = reset_window_location_history_navigation_runtime_state(
            scope,
            wrapper,
            &current_entry.url,
        );
        install_navigation_bootstrap_entry_for_holder(scope, wrapper, &entry_seed);
        let performance_navigation_type = self.child_performance_navigation_type(handle);
        let _ = bind_window_performance_seed(
            scope,
            wrapper,
            &performance_navigation_type,
            self.child_performance_time_origin(handle),
        );
        sync_window_location_history_navigation_runtime_surface(scope, wrapper);
    }
}

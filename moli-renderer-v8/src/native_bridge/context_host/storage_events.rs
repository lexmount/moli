use super::*;
use crate::{
    context_bootstrap::{construct_original_storage_event_utf16, mark_event_trusted},
    document_runtime::EventTargetHandle,
    page_task_queue::RendererPageStorageEventData,
    util::v8str,
};

impl JsContextHost {
    pub(super) fn refresh_storage_event_recipients(&mut self) {
        let windows = self
            .window_execution_contexts
            .iter()
            .map(|(owner, binding)| (*owner, binding.dispatch_scope()))
            .collect::<Vec<_>>();
        for (owner, dispatch_scope) in windows {
            self.register_storage_event_recipient(owner, dispatch_scope);
        }
    }

    pub(super) fn register_storage_event_recipient(
        &mut self,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) {
        // Bootstrap registers the first realm before Page task capabilities
        // are installed. Installation refreshes that realm before script runs.
        if self.page_task_capabilities.get().is_none()
            || !self.window_execution_context_owner_is_current(owner, dispatch_scope)
        {
            return;
        }
        let storage = match dispatch_scope {
            OwnerDispatchScope::Top => Some(self.top_document_storage_context()),
            OwnerDispatchScope::Child(handle) => {
                self.storage_context_for_child_browsing_context(handle)
            }
            OwnerDispatchScope::LightweightPopup(id) => {
                self.storage_context_for_lightweight_popup(id)
            }
        };
        let Some(storage) = storage else {
            self.storage_event_recipients.remove(&owner);
            return;
        };
        let (origin, area_key) = storage.into_origin_and_area_key();
        let target = WindowTaskTarget::new(dispatch_scope, owner);
        if self
            .storage_event_recipients
            .get(&owner)
            .is_some_and(|entry| entry.matches(&origin, &area_key, target))
        {
            return;
        }
        let recipient =
            std::sync::Arc::new(crate::context_bootstrap::WebStorageEventRecipient::new(
                origin,
                area_key,
                target,
                self.page_storage_event_delivery_sender(),
            ));
        self.web_storage_store
            .lock()
            .register_event_recipient(&recipient);
        self.storage_event_recipients.insert(owner, recipient);
    }

    /// Capture one DOM-manipulation task per exact recipient LocalDOMWindow.
    ///
    /// The Web Storage mutation, storage-area match and source exclusion are
    /// synchronous. Delivery is asynchronous, but it must not rediscover a
    /// replacement Window or dispatch several recipients in one Page turn.
    pub(crate) fn queue_storage_event_deliveries(
        &mut self,
        source: WindowTaskTarget,
        origin: &str,
        area_key: &str,
        data: RendererPageStorageEventData,
    ) -> usize {
        let targets =
            self.storage_event_delivery_targets(source, data.is_session(), origin, area_key);
        let sender = self.page_storage_event_delivery_sender();
        let mut queued = 0;
        for target in targets {
            match sender.send(target, data.clone()) {
                Ok(()) => queued += 1,
                Err(_) => {
                    tracing::debug!(
                        ?source,
                        ?target,
                        "retired Page DOM-manipulation route rejected StorageEvent delivery"
                    );
                    break;
                }
            }
        }
        if !data.is_session() {
            queued += self
                .web_storage_store
                .lock()
                .queue_remote_storage_events(&sender, origin, area_key, &data);
        }
        queued
    }

    fn storage_event_delivery_targets(
        &mut self,
        source: WindowTaskTarget,
        is_session: bool,
        origin: &str,
        area_key: &str,
    ) -> Vec<WindowTaskTarget> {
        let mut targets = Vec::new();
        let top_origin = moli_url::origin_ascii_serialization(self.document_url());

        let source_scope = source.dispatch_scope();
        let top_is_eligible = !matches!(source_scope, OwnerDispatchScope::Top)
            && (!is_session || !matches!(source_scope, OwnerDispatchScope::LightweightPopup(_)));
        if top_is_eligible {
            let target_scope = self.top_web_storage_scope();
            if target_scope.origin() == origin && target_scope.area_key() == area_key {
                self.push_current_storage_event_target(OwnerDispatchScope::Top, &mut targets);
            }
        }

        let child_contexts_are_eligible =
            !is_session || !matches!(source_scope, OwnerDispatchScope::LightweightPopup(_));
        if child_contexts_are_eligible {
            for handle in self.child_browsing_context_handles_in_document_order() {
                let dispatch_scope = OwnerDispatchScope::Child(handle);
                if dispatch_scope == source_scope {
                    continue;
                }
                let Some(target_scope) =
                    self.child_browsing_context_web_storage_scope(handle, &top_origin)
                else {
                    continue;
                };
                if target_scope.origin() != origin || target_scope.area_key() != area_key {
                    continue;
                }
                self.push_current_storage_event_target(dispatch_scope, &mut targets);
            }
        }

        if !is_session {
            for popup_id in self.open_lightweight_popup_ids() {
                let dispatch_scope = OwnerDispatchScope::LightweightPopup(popup_id);
                if dispatch_scope == source_scope {
                    continue;
                }
                let Some(target_context) = self.storage_context_for_lightweight_popup(popup_id)
                else {
                    continue;
                };
                if target_context.origin() != origin
                    || target_context.web_storage_area_key() != area_key
                {
                    continue;
                }
                self.push_current_storage_event_target(dispatch_scope, &mut targets);
            }
        }

        targets
    }

    fn push_current_storage_event_target(
        &self,
        dispatch_scope: OwnerDispatchScope,
        targets: &mut Vec<WindowTaskTarget>,
    ) {
        if let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) {
            targets.push(WindowTaskTarget::new(dispatch_scope, owner));
        }
    }

    /// Dispatch one StorageEvent after the Page arbiter has authorized its
    /// exact LocalDOMWindow. This method resolves that Window's default realm;
    /// it does not perform a second current/stale decision.
    pub(crate) fn dispatch_authorized_storage_event_delivery(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowTaskTarget,
        data: &RendererPageStorageEventData,
    ) -> bool {
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let global = scope.get_current_context().global(scope);
                let Some(event) = self.storage_event_for_target(scope, global, data) else {
                    return false;
                };
                self.dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)
                    .is_ok()
            }
            OwnerDispatchScope::Child(handle) => {
                if self
                    .ensure_prebootstrapped_child_default_context(scope, handle)
                    .is_err()
                {
                    return false;
                }
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let Some(window) =
                    self.existing_child_browsing_context_window_wrapper(scope, handle)
                else {
                    return false;
                };
                let Some(event) = self.storage_event_for_target(scope, window, data) else {
                    return false;
                };
                self.dispatch_child_window_event(scope, handle, "storage", event);
                true
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                if !self.ensure_lightweight_popup_execution_context(scope, popup_id) {
                    return false;
                }
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
                    return false;
                };
                let Some(event) = self.storage_event_for_target(scope, window, data) else {
                    return false;
                };
                self.dispatch_lightweight_popup_window_event(scope, popup_id, "storage", event);
                true
            }
        }
    }

    fn storage_event_for_target<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target: v8::Local<'s, v8::Object>,
        data: &RendererPageStorageEventData,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let storage_name = if data.is_session() {
            "sessionStorage"
        } else {
            "localStorage"
        };
        let storage_area = target.get(scope, v8str(scope, storage_name).into());
        let event = construct_original_storage_event_utf16(
            scope,
            "storage",
            data.key(),
            data.old_value(),
            data.new_value(),
            data.url(),
            storage_area,
        )?;
        mark_event_trusted(scope, event);
        Some(event)
    }
}

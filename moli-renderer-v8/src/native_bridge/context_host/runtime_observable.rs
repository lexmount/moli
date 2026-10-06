use super::{
    JsContextHost, OwnerDispatchScope, WindowExecutionContextAccessPolicy,
    WindowExecutionContextBinding, WindowExecutionContextIdentity, WindowExecutionContextOwner,
    WindowExecutionContextRealmRegistration, active_lightweight_popup_id,
};
use crate::runtime::RuntimeConsoleMessageSnapshot;
use serde_json::Value;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct RuntimeObservableContextToken(u64);

impl RuntimeObservableContextToken {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) const fn as_u64(self) -> u64 {
        self.0
    }

    fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

pub(crate) fn install_runtime_observable_context_token_for_context(
    context: v8::Local<'_, v8::Context>,
    context_token: RuntimeObservableContextToken,
) {
    let _previous = context.set_slot(Rc::new(context_token));
}

pub(crate) fn current_runtime_observable_context_token(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<RuntimeObservableContextToken> {
    scope
        .get_current_context()
        .get_slot::<RuntimeObservableContextToken>()
        .as_deref()
        .copied()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingRuntimeObservableConsoleSourceEvent {
    context_token: RuntimeObservableContextToken,
    message: String,
    args: Vec<Value>,
    stack: Option<String>,
}

impl PendingRuntimeObservableConsoleSourceEvent {
    pub(crate) fn new(
        context_token: RuntimeObservableContextToken,
        message: String,
        args: Vec<Value>,
        stack: Option<String>,
    ) -> Self {
        Self {
            context_token,
            message,
            args,
            stack,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_testing(context_token: u64, message: impl Into<String>) -> Self {
        Self {
            context_token: RuntimeObservableContextToken::from_raw(context_token),
            message: message.into(),
            args: Vec::new(),
            stack: None,
        }
    }

    pub(crate) fn context_token(&self) -> RuntimeObservableContextToken {
        self.context_token
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn into_runtime_console_message_snapshot(
        self,
        execution_context_id: i64,
    ) -> RuntimeConsoleMessageSnapshot {
        RuntimeConsoleMessageSnapshot {
            execution_context_id,
            message: self.message,
            args: self.args,
            stack: self.stack,
        }
    }
}

impl JsContextHost {
    /// Close only this concrete realm's running work. Default and isolated
    /// worlds, including unpromoted children, use the same service list.
    /// Registry retirement remains separate so Inspector can identify the
    /// Context until its destruction notification has been delivered.
    pub(crate) fn retire_window_realm_resources(
        &mut self,
        realm_token: RuntimeObservableContextToken,
    ) {
        let timers = self.cancel_timers_for_context_token(realm_token);
        let bindings = self.retire_runtime_binding_context_token(realm_token);
        let images = self.retire_image_decode_requests_for_context_token(realm_token);
        let bitmaps = self.retire_bitmap_context_token(realm_token);
        let crypto = self.retire_webcrypto_context_token(realm_token);
        self.retire_opfs_context_token(realm_token);
        let workers = self.retire_workers_for_context_token(realm_token);
        let shared_workers = self.disconnect_shared_worker_clients_for_context_token(realm_token);
        let xhrs = self.retire_window_xhrs_for_context_token(realm_token);
        let fetches = self.retire_window_fetches_for_context_token(realm_token);
        self.retire_window_event_sources_for_context_token(realm_token);
        let ports = self.retire_message_ports_for_context_token(realm_token);
        let messages = self.retire_window_messages_for_context_token(realm_token);
        let broadcasts = self.close_broadcast_channels_for_context_token(realm_token);
        let sockets = self.retire_websockets_for_context_token(realm_token);
        tracing::debug!(
            ?realm_token,
            timers,
            bindings = bindings.retired_execution_context_count(),
            images,
            bitmaps,
            crypto,
            workers,
            shared_workers,
            xhrs,
            aborted_fetches = fetches.0,
            detached_keepalive_fetches = fetches.1,
            ports,
            messages,
            broadcasts,
            sockets,
            "closed Window realm execution resources"
        );
    }

    pub(crate) fn retire_all_window_execution_context_resources_for_teardown(&mut self) {
        let owners = self
            .window_execution_contexts
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let mut realm_tokens = self
            .window_execution_contexts
            .values()
            .map(WindowExecutionContextBinding::realm_token)
            .chain(
                self.window_execution_context_realms
                    .concrete_by_token
                    .keys()
                    .copied(),
            )
            .collect::<Vec<_>>();
        realm_tokens.sort_unstable();
        realm_tokens.dedup();
        let resource_owner_id = self
            .window_default_context_bootstrap
            .as_ref()
            .map(|config| config.resource_owner_id);

        for realm_token in realm_tokens {
            self.retire_window_realm_resources(realm_token);
            self.retire_window_execution_contexts_for_context_token(realm_token, resource_owner_id);
        }

        for owner in owners {
            self.cancel_window_execution_context_timers(owner);
            self.retire_webcrypto_execution_context_owner(owner);
            self.retire_opfs_execution_context_owner(owner);
            self.retire_workers_for_execution_context_owner(owner);
            self.disconnect_shared_worker_clients_for_execution_context_owner(owner);
            self.retire_window_xhrs_for_execution_context_owner(owner);
            self.retire_window_fetches_for_execution_context_owner(owner);
            self.retire_window_event_sources_for_execution_context_owner(owner);
            self.retire_window_messages_for_execution_context_owner(owner);
            self.close_broadcast_channels_for_execution_context_owner(owner);
            self.retire_websockets_for_execution_context_owner(owner);
            self.retire_image_decode_requests_for_execution_context_owner(owner);
            self.retire_message_ports_for_execution_context_owner(owner);
            self.retire_window_execution_context(owner);
        }

        self.retire_v8_execution_state_for_context_teardown();

        // A detached Document realm may keep this host and its native DOM
        // alive through the V8 Context slot. None of the host's active
        // execution registries may in turn keep that Context alive with a
        // strong Global handle: that would form an untraceable
        // Context -> Rust host -> Global -> Context cycle. Chromium retires
        // these LocalDOMWindow/ExecutionContext-owned services when the frame
        // is detached while ordinary retained Document/Node values remain
        // usable.
        drop(std::mem::take(&mut self.custom_elements));
        self.child_custom_elements.clear();
        self.scoped_custom_elements.clear();
        drop(std::mem::take(&mut self.custom_element_reactions));
        drop(std::mem::take(&mut self.observers));
        drop(std::mem::take(&mut self.child_window_proxy_records));
        self.pending_service_worker_registers.clear();
        self.pending_service_worker_unregisters.clear();
        self.pending_service_worker_ready.clear();
        self.service_worker_registration_watchers.clear();
        self.pending_window_messages.clear();
        drop(std::mem::take(&mut self.directory_reader_callbacks));
        drop(std::mem::take(&mut self.misc_platform_api_tasks));
        drop(std::mem::take(&mut self.file_entry_file_callbacks));
        drop(std::mem::take(&mut self.user_interaction_tasks));
        self.pending_image_load_events.clear();
        self.pending_media_load_sequences.clear();
        self.pending_text_track_load_sequences.clear();
        self.pending_media_text_track_gates.clear();
        self.resource_timing_buffers =
            super::resource_timing::SharedResourceTimingBufferRegistry::new();
        drop(std::mem::take(&mut self.history_queue));
        drop(std::mem::take(&mut self.rendering_updates));
        drop(std::mem::take(&mut self.view_transition_updates));
        drop(std::mem::take(&mut self.media_element_events));
        drop(std::mem::take(&mut self.element_toggle_events));
        drop(std::mem::take(&mut self.text_track_default_modes));
        self.child_window_event_listeners.clear();
        drop(std::mem::take(&mut self.event_callbacks));
        self.bridge.abort.clear_for_context_teardown();
    }

    pub(crate) fn ensure_lightweight_popup_execution_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) -> bool {
        self.ensure_popup_default_context(scope, popup_id).is_ok()
    }

    pub(crate) fn current_runtime_window_execution_context_binding(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<WindowExecutionContextBinding> {
        let identity = self.current_runtime_window_execution_context_identity(scope)?;
        Some(WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, scope.get_current_context()),
        ))
    }

    pub(crate) fn current_runtime_window_execution_context_identity(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<WindowExecutionContextIdentity> {
        if let Some(owner) =
            crate::script_continuation::running_window(scope, scope.get_current_context())
        {
            return Some(owner.identity);
        }
        let dispatch_scope = if let Some(identity) =
            self.window_execution_context_identity_for_access_check(scope.get_current_context())
        {
            identity.dispatch_scope()
        } else if let Some(popup_id) = active_lightweight_popup_id(scope) {
            OwnerDispatchScope::LightweightPopup(popup_id)
        } else if let Some(child_handle) =
            crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
        {
            OwnerDispatchScope::Child(child_handle)
        } else {
            OwnerDispatchScope::Top
        };
        self.current_runtime_window_execution_context_identity_for_dispatch_scope(
            scope,
            dispatch_scope,
        )
    }

    pub(crate) fn current_runtime_window_execution_context_identity_for_dispatch_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowExecutionContextIdentity> {
        let realm_token = current_runtime_observable_context_token(scope)?;
        let registration = self
            .window_execution_context_realms
            .registration(dispatch_scope, realm_token)?;
        let owner = registration.owner;
        if !self.window_execution_context_owner_is_current(owner, dispatch_scope) {
            return None;
        }
        Some(WindowExecutionContextIdentity::new(
            owner,
            dispatch_scope,
            realm_token,
            registration.access_policy,
        ))
    }

    pub(crate) fn current_registered_window_execution_context_identity(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowExecutionContextIdentity> {
        let owner = self.current_window_execution_context_owner(dispatch_scope)?;
        let binding = self.window_execution_contexts.get(&owner)?;
        if binding.dispatch_scope() != dispatch_scope {
            return None;
        }
        let realm_token = binding.realm_token();
        let registration = self
            .window_execution_context_realms
            .registration(dispatch_scope, realm_token)?;
        (registration.owner == owner).then(|| {
            WindowExecutionContextIdentity::new(
                owner,
                dispatch_scope,
                realm_token,
                registration.access_policy,
            )
        })
    }

    /// Resolves a concrete Window realm without invoking any V8 property API.
    ///
    /// V8 calls this while `MayAccess` is already active, so reading global or
    /// private properties here would recursively enter the access callback.
    pub(crate) fn window_execution_context_identity_for_access_check(
        &self,
        context: v8::Local<'_, v8::Context>,
    ) -> Option<WindowExecutionContextIdentity> {
        let realm_token = context
            .get_slot::<RuntimeObservableContextToken>()
            .as_deref()
            .copied()?;
        let registered = self
            .window_execution_context_realms
            .concrete_registration(realm_token)?;
        let registration = registered.registration;
        Some(WindowExecutionContextIdentity::new(
            registration.owner,
            registered.dispatch_scope,
            realm_token,
            registration.access_policy,
        ))
    }

    pub(crate) fn window_execution_context_identity_for_v8_context(
        &self,
        _scope: &mut v8::PinScope<'_, '_>,
        context: v8::Local<'_, v8::Context>,
    ) -> Option<WindowExecutionContextIdentity> {
        let realm_token = context
            .get_slot::<RuntimeObservableContextToken>()
            .as_deref()
            .copied()?;
        let registered = self
            .window_execution_context_realms
            .concrete_registration(realm_token)?;
        Some(WindowExecutionContextIdentity::new(
            registered.registration.owner,
            registered.dispatch_scope,
            realm_token,
            registered.registration.access_policy,
        ))
    }

    pub(crate) fn register_window_execution_context(
        &mut self,
        binding: WindowExecutionContextBinding,
    ) {
        let owner = binding.owner();
        let current_realm = binding.realm_token();
        let current_dispatch_scope = binding.dispatch_scope();
        if !self.register_window_execution_context_realm(
            owner,
            current_dispatch_scope,
            current_realm,
            WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        ) {
            return;
        }
        let previous = self.window_execution_contexts.insert(owner, binding);
        self.register_storage_event_recipient(owner, current_dispatch_scope);
        if let Some(previous) = previous.as_ref()
            && previous.realm_token() != current_realm
        {
            self.window_execution_context_realms
                .remove(previous.dispatch_scope(), previous.realm_token());
        }
        if let Some(previous) = previous
            && previous.realm_token() != current_realm
        {
            tracing::debug!(
                ?owner,
                previous_realm = ?previous.realm_token(),
                ?current_realm,
                "replaced LocalWindow execution context binding"
            );
        }
    }

    pub(crate) fn register_window_execution_context_realm(
        &mut self,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
        access_policy: WindowExecutionContextAccessPolicy,
    ) -> bool {
        if !self.window_execution_context_owner_is_current(owner, dispatch_scope) {
            tracing::debug!(
                ?owner,
                ?dispatch_scope,
                ?realm_token,
                "refused to register stale Window execution-context realm"
            );
            return false;
        }
        let registration = WindowExecutionContextRealmRegistration::new(owner, access_policy);
        match self.window_execution_context_realms.register(
            dispatch_scope,
            realm_token,
            registration,
        ) {
            Ok(()) => true,
            Err(registered) => {
                tracing::warn!(
                    ?owner,
                    ?dispatch_scope,
                    ?realm_token,
                    ?access_policy,
                    registered_realm = ?registered,
                    "refused to mutate Window realm owner or access policy"
                );
                false
            }
        }
    }

    pub(crate) fn window_execution_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<(RuntimeObservableContextToken, v8::Local<'s, v8::Context>)> {
        let binding = self.window_execution_contexts.get(&owner)?;
        (binding.dispatch_scope() == dispatch_scope)
            .then(|| (binding.realm_token(), binding.context(scope)))
    }

    pub(crate) fn clone_window_execution_context_binding(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowExecutionContextBinding> {
        let (realm_token, context) = self.window_execution_context(scope, owner, dispatch_scope)?;
        Some(WindowExecutionContextBinding::new(
            owner,
            dispatch_scope,
            realm_token,
            v8::Global::new(scope, context),
        ))
    }

    pub(crate) fn retire_window_execution_context(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> bool {
        self.animation_frames.retire_owner(owner);
        self.storage_event_recipients.remove(&owner);
        self.retire_event_callbacks_for_execution_context(owner);
        self.close_watcher_managers.remove(&owner);
        crate::observer_runtime::retire_execution_context_owner(self, owner);
        let retired = self.window_execution_contexts.remove(&owner);
        if let Some(binding) = retired.as_ref() {
            let retirement = self.retire_indexed_db_context(binding.realm_token());
            retirement.finish(self.indexed_db_manager.as_ref());
            self.bridge
                .retire_default_world_wrappers_for_realm(binding.realm_token());
        }
        self.window_execution_context_realms.retire_owner(owner);
        if retired.is_some() {
            tracing::debug!(?owner, "retired LocalWindow execution context binding");
        }
        retired.is_some()
    }

    pub(crate) fn retire_window_execution_contexts_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
        resource_owner_id: Option<crate::resource_owner::ResourceOwnerId>,
    ) -> usize {
        self.animation_frames.retire_realm(context_token);
        // A host whose bootstrap failed may not have a resource owner yet.
        // Still retire its registries without revoking another host's URLs.
        let revoked_blob_object_url_count = resource_owner_id
            .map(|owner| crate::blob::cleanup_object_urls_for_context(owner, context_token))
            .unwrap_or(0);
        crate::observer_runtime::retire_context_token(self, context_token);
        let indexed_db_retirement = self.retire_indexed_db_context(context_token);
        let retired_indexed_db_connections = indexed_db_retirement.retired_connections.len();
        indexed_db_retirement.finish(self.indexed_db_manager.as_ref());
        let owners = self
            .window_execution_contexts
            .iter()
            .filter_map(|(owner, binding)| {
                (binding.realm_token() == context_token).then_some(*owner)
            })
            .collect::<Vec<_>>();
        let retired_count = owners.len();
        for owner in owners {
            self.animation_frames.retire_owner(owner);
            self.storage_event_recipients.remove(&owner);
            self.close_watcher_managers.remove(&owner);
            self.window_execution_contexts.remove(&owner);
        }
        self.bridge
            .retire_default_world_wrappers_for_realm(context_token);
        let _ = self
            .window_execution_context_realms
            .remove_token(context_token);
        if retired_count > 0 {
            tracing::debug!(
                ?context_token,
                retired_count,
                retired_indexed_db_connections,
                revoked_blob_object_url_count,
                "retired LocalWindow bindings with destroyed V8 execution context"
            );
        } else if retired_indexed_db_connections > 0 || revoked_blob_object_url_count > 0 {
            tracing::debug!(
                ?context_token,
                retired_indexed_db_connections,
                revoked_blob_object_url_count,
                "retired context-owned state with destroyed V8 execution context"
            );
        }
        retired_count
    }

    /// Retires an isolated Window realm registration without touching the
    /// LocalWindow's default-world binding or wrapper cache.
    ///
    /// Isolated worlds share a LocalWindow owner with its default world but
    /// have their own realm token. The owner-indexed binding remains the
    /// default world, so using `retire_window_execution_contexts_for_context_token`
    /// would conflate a realm registration with the separately owned default
    /// binding and wrapper-cache lifecycle.
    pub(crate) fn retire_isolated_window_execution_context(
        &mut self,
        context_token: RuntimeObservableContextToken,
        resource_owner_id: crate::resource_owner::ResourceOwnerId,
    ) -> usize {
        self.animation_frames.retire_realm(context_token);
        self.retire_close_watcher_realm(context_token);
        let revoked_blob_object_url_count =
            crate::blob::cleanup_object_urls_for_context(resource_owner_id, context_token);
        crate::observer_runtime::retire_context_token(self, context_token);
        let indexed_db_retirement = self.retire_indexed_db_context(context_token);
        indexed_db_retirement.finish(self.indexed_db_manager.as_ref());
        let retired_realm_count = self
            .window_execution_context_realms
            .remove_token(context_token);
        tracing::debug!(
            ?context_token,
            retired_realm_count,
            revoked_blob_object_url_count,
            "retired isolated Window realm registration"
        );
        retired_realm_count
    }

    #[cfg(test)]
    pub(crate) fn window_execution_context_registry_counts_for_test(&self) -> (usize, usize) {
        (
            self.window_execution_contexts.len(),
            self.window_execution_context_realms.concrete_by_token.len(),
        )
    }

    pub(crate) fn current_window_execution_context_owner(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowExecutionContextOwner> {
        match dispatch_scope {
            OwnerDispatchScope::Top => Some(WindowExecutionContextOwner::Frame(
                self.current_main_document_task_owner()?.local_window_id,
            )),
            OwnerDispatchScope::Child(child_handle) => Some(WindowExecutionContextOwner::Frame(
                self.current_child_document_task_owner(child_handle)?
                    .local_window_id,
            )),
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                Some(WindowExecutionContextOwner::LightweightPopup {
                    popup_id,
                    local_window_id: self.current_lightweight_popup_local_window_id(popup_id)?,
                })
            }
        }
    }

    pub(crate) fn current_window_execution_context_binding(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowExecutionContextBinding> {
        let (owner, realm_token) =
            self.current_window_execution_context_identity(scope, dispatch_scope)?;
        Some(WindowExecutionContextBinding::new(
            owner,
            dispatch_scope,
            realm_token,
            v8::Global::new(scope, scope.get_current_context()),
        ))
    }

    pub(crate) fn current_window_execution_context_identity(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<(WindowExecutionContextOwner, RuntimeObservableContextToken)> {
        Some((
            self.current_window_execution_context_owner(dispatch_scope)?,
            current_runtime_observable_context_token(scope)?,
        ))
    }

    pub(crate) fn window_execution_context_owner_is_current(
        &self,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> bool {
        match (owner, dispatch_scope) {
            (WindowExecutionContextOwner::Frame(local_window_id), OwnerDispatchScope::Top) => self
                .current_main_document_task_owner()
                .is_some_and(|current| current.local_window_id == local_window_id),
            (
                WindowExecutionContextOwner::Frame(local_window_id),
                OwnerDispatchScope::Child(child_handle),
            ) => self
                .current_child_document_task_owner(child_handle)
                .is_some_and(|current| current.local_window_id == local_window_id),
            (
                WindowExecutionContextOwner::LightweightPopup {
                    popup_id,
                    local_window_id,
                },
                OwnerDispatchScope::LightweightPopup(dispatch_popup_id),
            ) => {
                popup_id == dispatch_popup_id
                    && self.current_lightweight_popup_local_window_id(popup_id)
                        == Some(local_window_id)
            }
            _ => false,
        }
    }

    pub(crate) fn window_execution_context_identity_is_current(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> bool {
        self.window_execution_context_owner_is_current(identity.owner(), identity.dispatch_scope())
            && self
                .window_execution_context_realms
                .registration(identity.dispatch_scope(), identity.realm_token())
                .is_some_and(|registration| {
                    registration.owner == identity.owner()
                        && registration.access_policy == identity.access_policy()
                })
    }

    pub(crate) fn allocate_runtime_observable_context_token(
        &mut self,
    ) -> RuntimeObservableContextToken {
        let token = self.next_runtime_observable_context_token;
        self.next_runtime_observable_context_token = self
            .next_runtime_observable_context_token
            .checked_next()
            .expect("runtime observable context token overflow");
        token
    }

    pub(crate) fn record_runtime_observable_console_source_event(
        &mut self,
        context_token: RuntimeObservableContextToken,
        execution_context_id: i64,
        message: String,
        args: Vec<Value>,
        stack: Option<String>,
    ) {
        let event =
            PendingRuntimeObservableConsoleSourceEvent::new(context_token, message, args, stack);
        let protocol_message = event
            .clone()
            .into_runtime_console_message_snapshot(execution_context_id);
        // Script/CLI reporting owns a separate authoritative history. Keeping
        // that history does not delay or rediscover the protocol fact: the
        // concrete record below already owns its exact V8 context identity.
        self.pending_runtime_observable_console_source_events
            .push(event);
        self.append_live_turn_observation(
            crate::runtime::RendererProtocolObservation::RuntimeConsole(protocol_message),
        );
    }

    pub(crate) fn take_pending_runtime_observable_console_source_events(
        &mut self,
    ) -> Vec<PendingRuntimeObservableConsoleSourceEvent> {
        std::mem::take(&mut self.pending_runtime_observable_console_source_events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document_runtime::DomHandle,
        frame_owner_model::LocalWindowId,
        native_bridge::context_host::{
            WindowExecutionContextRealmRecords,
            window_execution_context::WindowExecutionContextScopedRealmRegistration,
        },
        window_document_identity::LightweightPopupLocalWindowId,
    };

    #[test]
    fn popup_and_opener_have_distinct_concrete_realm_tokens() {
        let mut records = WindowExecutionContextRealmRecords::default();
        let token = RuntimeObservableContextToken::from_raw(17);
        let top_scope = OwnerDispatchScope::Top;
        let top_registration = WindowExecutionContextRealmRegistration::new(
            WindowExecutionContextOwner::Frame(LocalWindowId(1)),
            WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        );
        let popup_scope = OwnerDispatchScope::LightweightPopup(7);
        let popup_token = RuntimeObservableContextToken::from_raw(18);
        let popup_registration = WindowExecutionContextRealmRegistration::new(
            WindowExecutionContextOwner::LightweightPopup {
                popup_id: 7,
                local_window_id: LightweightPopupLocalWindowId::new(2),
            },
            WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        );

        assert!(records.register(top_scope, token, top_registration).is_ok());
        assert!(
            records
                .register(popup_scope, token, popup_registration)
                .is_err()
        );
        assert!(
            records
                .register(popup_scope, popup_token, popup_registration)
                .is_ok()
        );
        assert_eq!(
            records.concrete_registration(popup_token),
            Some(WindowExecutionContextScopedRealmRegistration::new(
                popup_scope,
                popup_registration
            ))
        );

        assert_eq!(
            records.concrete_registration(token),
            Some(WindowExecutionContextScopedRealmRegistration::new(
                top_scope,
                top_registration,
            ))
        );
        assert_eq!(
            records.registration(popup_scope, popup_token),
            Some(popup_registration)
        );

        let conflicting_child = OwnerDispatchScope::Child(DomHandle::new(9));
        assert!(
            records
                .register(conflicting_child, token, top_registration)
                .is_err(),
            "one V8 context token must have exactly one concrete Window realm"
        );

        assert_eq!(records.remove_token(token), 1);
        assert!(records.registration(popup_scope, popup_token).is_some());
        assert_eq!(records.remove_token(popup_token), 1);
        assert!(records.concrete_registration(token).is_none());
        assert!(records.registration(popup_scope, token).is_none());
    }
}

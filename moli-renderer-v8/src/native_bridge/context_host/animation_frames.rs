//! Document-owned animation callbacks. A timer only wakes the rendering
//! source; one rendering task snapshots and invokes a Document's callback map,
//! followed by layout/observer processing and focus fixup at the ScriptVm
//! rendering boundary. Geometry readers never perform that layout work.

use super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken, WindowDocumentTaskTarget,
    WindowExecutionContextOwner, WindowOperationReceiver,
};
use crate::{
    context_bootstrap::{current_performance_time_origin, dom_time_since_origin_millis},
    page_task_queue::RendererPageRenderingUpdateTaskKind,
    v8_execution_watchdog::{
        V8ExecutionWatchdog, V8ExecutionWatchdogKind, V8ExecutionWatchdogOutcome,
    },
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};

use super::rendering_updates::PendingRenderingUpdatePayload;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct AnimationFrameState {
    providers: Vec<AnimationFrameProvider>,
}

struct AnimationFrameProvider {
    dispatch_scope: OwnerDispatchScope,
    owner: WindowExecutionContextOwner,
    next_handle: u32,
    wake_pending: bool,
    focus_fixup_target: Option<WindowDocumentTaskTarget>,
    observer_target: Option<WindowDocumentTaskTarget>,
    handles: Vec<u32>,
    callbacks: HashMap<u32, WindowWebIdlCallbackFunction>,
}

impl AnimationFrameState {
    pub(super) fn retire_owner(&mut self, owner: WindowExecutionContextOwner) {
        self.providers.retain(|provider| provider.owner != owner);
        for provider in &mut self.providers {
            provider.callbacks.retain(|_, callback| {
                callback
                    .relevant_identity()
                    .is_some_and(|identity| identity.owner() != owner)
            });
        }
    }

    pub(super) fn retire_realm(&mut self, token: RuntimeObservableContextToken) {
        for provider in &mut self.providers {
            provider.callbacks.retain(|_, callback| {
                callback
                    .relevant_identity()
                    .is_some_and(|identity| identity.realm_token() != token)
            });
        }
    }

    fn provider_mut(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> Option<&mut AnimationFrameProvider> {
        self.providers
            .iter_mut()
            .find(|provider| provider.owner == owner)
    }

    fn ensure_provider(
        &mut self,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> &mut AnimationFrameProvider {
        if !self
            .providers
            .iter()
            .any(|provider| provider.owner == owner)
        {
            self.providers.push(AnimationFrameProvider {
                dispatch_scope,
                owner,
                next_handle: 0,
                wake_pending: false,
                focus_fixup_target: None,
                observer_target: None,
                handles: Vec::new(),
                callbacks: HashMap::new(),
            });
        }
        self.provider_mut(owner).expect("provider was inserted")
    }

    fn discard(&mut self, owner: WindowExecutionContextOwner) {
        self.providers.retain(|provider| provider.owner != owner);
    }
}

impl JsContextHost {
    /// A rendering body can yield to author code several times. Revalidate
    /// its immutable Document target before resolving each later phase; the
    /// initial Page authorization alone cannot survive document.open().
    pub(crate) fn resolve_current_rendering_update_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowDocumentTaskTarget,
    ) -> Option<(
        crate::document_runtime::DomHandle,
        v8::Local<'s, v8::Context>,
    )> {
        if scope.is_execution_terminating()
            || !self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            )
        {
            return None;
        }
        let resolved = self.resolve_authorized_window_document_task_context(scope, target)?;
        Some((resolved.document_handle, resolved.context))
    }

    /// DOM changes and focus styling can make the focused element unavailable.
    /// Revalidate it at the end of a rendering update, using the same native
    /// wake and exact Document owner as animation callbacks. No author timer
    /// or animation-frame function participates in this internal scheduling.
    pub(crate) fn queue_focused_document_fixup(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let Some(active) = self.active_element_handle() else {
            return;
        };
        let Some(document) = self.dom_host().owner_document_handle(active) else {
            return;
        };
        let Some(endpoint) = self.window_endpoint_for_document(document) else {
            return;
        };
        let dispatch_scope = endpoint.dispatch_scope();
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return;
        };
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
        else {
            return;
        };
        let provider = self.animation_frames.ensure_provider(owner, dispatch_scope);
        provider.focus_fixup_target = Some(target);
        self.ensure_document_rendering_wake(scope, owner, dispatch_scope);
    }

    pub(crate) fn queue_document_observer_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        dispatch_scope: OwnerDispatchScope,
    ) {
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return;
        };
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
        else {
            return;
        };
        self.animation_frames
            .ensure_provider(owner, dispatch_scope)
            .observer_target = Some(target);
        self.ensure_document_rendering_wake(scope, owner, dispatch_scope);
    }

    fn ensure_document_rendering_wake(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
    ) {
        if self
            .animation_frames
            .ensure_provider(owner, dispatch_scope)
            .wake_pending
        {
            return;
        }
        let Some(binding) =
            self.clone_window_execution_context_binding(scope, owner, dispatch_scope)
        else {
            self.animation_frames.discard(owner);
            return;
        };
        self.animation_frames
            .provider_mut(owner)
            .expect("provider was inserted")
            .wake_pending = true;
        self.queue_window_animation_frame_wake(scope, binding);
    }

    pub(crate) fn request_window_animation_frame<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: WindowOperationReceiver,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
    ) -> u32 {
        let Some(binding) = receiver.resolve_live_binding(self) else {
            return 0;
        };
        let owner = binding.owner();
        let callback = WindowWebIdlCallbackFunction::new(scope, self, callback);
        if callback.relevant_identity().is_none() {
            return 0;
        }
        // document.open() preserves this callback map and its handle counter.
        // Actual Window retirement removes both through the owner registry.
        let provider = self
            .animation_frames
            .ensure_provider(owner, binding.dispatch_scope());
        let handle = loop {
            provider.next_handle = provider.next_handle.wrapping_add(1);
            if provider.next_handle != 0 && !provider.callbacks.contains_key(&provider.next_handle)
            {
                break provider.next_handle;
            }
        };
        provider.handles.push(handle);
        provider.callbacks.insert(handle, callback);
        if !provider.wake_pending {
            provider.wake_pending = true;
            // The wake belongs to the Document's default Window realm; each
            // callback independently retains its own relevant/incumbent realms.
            let binding = self
                .clone_window_execution_context_binding(
                    scope,
                    binding.owner(),
                    binding.dispatch_scope(),
                )
                .unwrap_or(binding);
            self.queue_window_animation_frame_wake(scope, binding);
        }
        handle
    }

    pub(crate) fn cancel_window_animation_frame(
        &mut self,
        receiver: WindowOperationReceiver,
        handle: u32,
    ) {
        let Some(binding) = receiver.resolve_live_binding(self) else {
            return;
        };
        if let Some(provider) = self.animation_frames.provider_mut(binding.owner()) {
            provider.callbacks.remove(&handle);
        }
    }

    /// Consuming the deadline never invokes page callbacks. The immutable
    /// Document target is validated again by the rendering source's arbiter.
    pub(crate) fn publish_animation_frame_rendering_update(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        let Some(provider) = self.animation_frames.provider_mut(owner) else {
            return;
        };
        let dispatch_scope = provider.dispatch_scope;
        if !self.window_execution_context_owner_is_current(owner, dispatch_scope) {
            self.animation_frames.discard(owner);
            return;
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
        else {
            self.animation_frames.discard(owner);
            return;
        };
        let provider = self
            .animation_frames
            .provider_mut(owner)
            .expect("provider is current");
        if provider.focus_fixup_target != Some(target) {
            // Unlike the Window's callback map, an old Document's pending
            // focus check must not survive document.open().
            provider.focus_fixup_target = None;
        }
        if provider.observer_target != Some(target) {
            provider.observer_target = None;
        }
        if provider.callbacks.is_empty()
            && provider.focus_fixup_target.is_none()
            && provider.observer_target.is_none()
        {
            provider.wake_pending = false;
            provider.handles.clear();
            return;
        }
        if !self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::AnimationFrameCallbacks,
            PendingRenderingUpdatePayload::AnimationFrameCallbacks(owner),
        ) {
            self.animation_frames.discard(owner);
        }
    }

    pub(super) fn dispatch_authorized_animation_frame_callbacks(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        owner: WindowExecutionContextOwner,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            self.animation_frames.discard(owner);
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let timestamp = dom_time_since_origin_millis(current_performance_time_origin(scope));
        let mut invoked = false;

        // HTML flushes autofocus before taking the animation callback snapshot.
        // Use its top-level Document's realm, including an independent popup.
        if let Some(document) = self.top_level_document_for_document(resolved.document_handle)
            && let Some(endpoint) = self.window_endpoint_for_document(document)
            && let Some(top) = self
                .current_window_document_task_target_for_dispatch_scope(endpoint.dispatch_scope())
        {
            invoked |= self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, top);
        }

        let handles = self
            .animation_frames
            .provider_mut(owner)
            .map(|provider| {
                provider.wake_pending = false;
                provider.observer_target = None;
                std::mem::take(&mut provider.handles)
            })
            .unwrap_or_default();
        for handle in handles {
            if !self.window_execution_context_owner_is_current(owner, dispatch_scope) {
                self.animation_frames.discard(owner);
                break;
            }
            let callback = self
                .animation_frames
                .provider_mut(owner)
                .and_then(|provider| provider.callbacks.remove(&handle));
            let Some(callback) = callback else {
                continue;
            };
            let prepared = callback.prepare(scope);
            let watchdog = V8ExecutionWatchdog::arm(
                V8ExecutionWatchdogKind::AnimationFrameCallback,
                scope.thread_safe_handle(),
                std::time::Duration::from_secs(8),
            );
            let execution_scope = crate::script_cleanup::ScriptExecutionScope::enter(scope);
            let receiver = v8::undefined(scope).into();
            let argument = v8::Number::new(scope, timestamp).into();
            let result = invoke_window_webidl_callback_function(
                scope,
                host_ptr,
                "requestAnimationFrame callback",
                "animation frame callback threw",
                "requestAnimationFrame callback",
                &prepared,
                receiver,
                &[argument],
            );
            if !scope.is_execution_terminating()
                && let WindowWebIdlCallbackFunctionOutcome::Threw(report) = result
            {
                crate::host::report_window_timer_exception(
                    scope,
                    callback.relevant_identity(),
                    None,
                    &report,
                );
            }
            drop(execution_scope);
            crate::script_cleanup::perform_callback_cleanup_checkpoint(scope);
            if watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut {
                tracing::warn!("animation frame callback exceeded its execution deadline");
            }
            invoked = true;
        }
        dispatch_scope.restore(scope, previous_scope);
        invoked
    }

    pub(crate) fn finish_document_animation_frame(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        owner: WindowExecutionContextOwner,
    ) -> bool {
        let Some((document, context)) =
            self.resolve_current_rendering_update_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let mut invoked = false;
        // A callback can replace its Document or move focus elsewhere. The
        // rendering entry must never clear focus in a replacement Document.
        if self.window_document_owner_is_current_for_dispatch_scope(target.owner(), dispatch_scope)
            && self
                .animation_frames
                .provider_mut(owner)
                .is_some_and(|provider| {
                    if provider.focus_fixup_target == Some(target) {
                        provider.focus_fixup_target = None;
                        true
                    } else {
                        false
                    }
                })
        {
            invoked |= super::super::element::apply_document_focus_fixup(scope, host_ptr, document);
        }
        dispatch_scope.restore(scope, previous_scope);
        invoked
    }
}

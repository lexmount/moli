//! Document-owned animation callbacks. A timer only wakes the rendering
//! source; one rendering task snapshots and invokes a Document's callback map.

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

    fn discard(&mut self, owner: WindowExecutionContextOwner) {
        self.providers.retain(|provider| provider.owner != owner);
    }
}

impl JsContextHost {
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
        if self.animation_frames.provider_mut(owner).is_none() {
            self.animation_frames
                .providers
                .push(AnimationFrameProvider {
                    dispatch_scope: binding.dispatch_scope(),
                    owner,
                    next_handle: 0,
                    wake_pending: false,
                    handles: Vec::new(),
                    callbacks: HashMap::new(),
                });
        }
        let provider = self
            .animation_frames
            .provider_mut(owner)
            .expect("provider was inserted");
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
        if provider.callbacks.is_empty() {
            provider.wake_pending = false;
            provider.handles.clear();
            return;
        }
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
        // Use the top Document's own realm even when a child requested the frame.
        if !matches!(dispatch_scope, OwnerDispatchScope::LightweightPopup(_))
            && let Some(top) =
                self.current_window_document_task_target_for_dispatch_scope(OwnerDispatchScope::Top)
        {
            invoked |= self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, top);
        }

        let handles = self
            .animation_frames
            .provider_mut(owner)
            .map(|provider| {
                provider.wake_pending = false;
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
}

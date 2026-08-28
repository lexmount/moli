use super::*;
use crate::webidl;

impl JsContextHost {
    pub(in crate::native_bridge) fn detach_abort_signals_for_context_teardown(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        context_token: super::RuntimeObservableContextToken,
    ) {
        let retirement = self
            .bridge
            .abort
            .detach_owned_by_context_token(scope, context_token);
        for callback_id in retirement.signal_listener_callback_ids {
            self.release_event_callback(callback_id);
        }
    }

    pub(crate) fn is_abort_signal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.bridge.abort.is_signal_object(scope, signal)
    }

    pub(crate) fn abort_signal_aborted<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.bridge.abort.signal_aborted(scope, signal)
    }

    pub(crate) fn abort_signal_reason<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.bridge.abort.signal_reason(scope, signal)
    }

    pub(crate) fn register_abort_target_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        callback_id: EventCallbackId,
        capture: bool,
    ) {
        self.bridge.abort.register_target_listener(
            scope,
            signal,
            target,
            event_type,
            callback_id,
            capture,
        );
    }

    pub(crate) fn register_abort_signal_event_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        event_type: &str,
        callback: webidl::WebIdlCallbackInterface,
        options: webidl::EventListenerOptions,
    ) -> bool {
        let Some(signal_id) =
            crate::native_bridge::abort::AbortStore::signal_id_from_object(scope, signal)
        else {
            return false;
        };
        if !self.bridge.abort.is_signal_object(scope, signal) {
            return false;
        }
        let callback_value = callback.value(scope);
        let Ok(callback_object) = v8::Local::<v8::Object>::try_from(callback_value) else {
            return false;
        };
        if self
            .bridge
            .abort
            .listener_callback_ids(signal_id, event_type, options.capture)
            .into_iter()
            .any(|callback_id| self.event_callback_matches(scope, callback_id, callback_object))
        {
            return false;
        }

        let callback_id = self.register_webidl_event_callback(scope, callback);
        if self.bridge.abort.register_listener(
            signal_id,
            event_type,
            callback_id,
            options.capture,
            options.once,
            options.passive.unwrap_or(false),
        ) {
            true
        } else {
            self.release_event_callback(callback_id);
            false
        }
    }

    pub(crate) fn unregister_abort_signal_event_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        event_type: &str,
        callback: &webidl::WebIdlCallbackInterface,
        capture: bool,
    ) -> bool {
        let Some(signal_id) =
            crate::native_bridge::abort::AbortStore::signal_id_from_object(scope, signal)
        else {
            return false;
        };
        let callback_value = callback.value(scope);
        let Ok(callback_object) = v8::Local::<v8::Object>::try_from(callback_value) else {
            return false;
        };
        let Some(callback_id) = self
            .bridge
            .abort
            .listener_callback_ids(signal_id, event_type, capture)
            .into_iter()
            .find(|callback_id| self.event_callback_matches(scope, *callback_id, callback_object))
        else {
            return false;
        };
        if !self
            .bridge
            .abort
            .unregister_listener_by_id(signal_id, event_type, callback_id, capture)
        {
            return false;
        }
        self.release_event_callback(callback_id);
        true
    }

    pub(in crate::native_bridge) fn claim_abort_signal_event_listener_for_dispatch(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal_id: u32,
        event_type: &str,
        callback_id: EventCallbackId,
    ) -> Option<crate::native_bridge::abort::PreparedAbortListener> {
        let listener =
            self.bridge
                .abort
                .claim_listener_for_dispatch(signal_id, event_type, callback_id)?;
        let callback = self.prepare_event_callback(scope, callback_id);
        if listener.once {
            self.release_event_callback(callback_id);
        }
        let Some(callback) = callback else {
            if !listener.once {
                let _ = self.bridge.abort.unregister_listener_by_id(
                    signal_id,
                    event_type,
                    callback_id,
                    listener.capture,
                );
            }
            return None;
        };
        Some(crate::native_bridge::abort::PreparedAbortListener {
            callback,
            passive: listener.passive,
        })
    }

    pub(crate) fn register_abort_signal_algorithm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        self.bridge
            .abort
            .register_abort_algorithm(scope, signal, algorithm)
    }

    pub(crate) fn unregister_abort_signal_algorithm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        self.bridge
            .abort
            .unregister_abort_algorithm(scope, signal, algorithm)
    }

    pub(crate) fn unregister_abort_target_listener(&mut self, callback_id: EventCallbackId) {
        self.bridge.abort.unregister_target_listener(callback_id);
    }

    pub(crate) fn register_abort_message_port_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        port_id: crate::types::MessagePortId,
        listener_id: crate::context_bootstrap::MessagePortEventListenerId,
    ) -> bool {
        self.bridge
            .abort
            .register_message_port_listener(scope, signal, port_id, listener_id)
    }

    pub(crate) fn unregister_abort_message_port_listener(
        &mut self,
        port_id: crate::types::MessagePortId,
        listener_id: crate::context_bootstrap::MessagePortEventListenerId,
    ) {
        self.bridge
            .abort
            .unregister_message_port_listener(port_id, listener_id);
    }

    pub(crate) fn remove_registered_event_listener_by_id(
        &mut self,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        callback_id: EventCallbackId,
        capture: bool,
    ) {
        let removed = match target {
            crate::document_runtime::EventTargetHandle::ChildWindow(target) => self
                .remove_child_window_event_listener_by_id(
                    target.child_handle(),
                    event_type,
                    callback_id,
                    capture,
                ),
            crate::document_runtime::EventTargetHandle::Window
            | crate::document_runtime::EventTargetHandle::Node(_) => {
                self.remove_event_listener_by_id(target, event_type, callback_id, capture)
            }
        };
        if removed {
            self.release_event_callback(callback_id);
        }
    }

    pub(crate) fn abort_signal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        reason: v8::Local<'s, v8::Value>,
    ) {
        let host_ptr = self as *mut Self;
        unsafe {
            (*host_ptr)
                .bridge
                .abort
                .abort_signal(scope, &mut *host_ptr, signal, reason);
        }
    }
}

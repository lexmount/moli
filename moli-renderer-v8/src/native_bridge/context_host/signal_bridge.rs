use super::*;

impl JsContextHost {
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
}

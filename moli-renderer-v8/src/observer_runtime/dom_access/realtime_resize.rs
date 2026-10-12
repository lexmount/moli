use super::*;
use crate::native_bridge::{WindowDocumentOwner, WindowDocumentTaskTarget};

fn registry(host: &JsContextHost) -> &super::super::callback::ObserverCallbackRegistry {
    &host.observers(&OBSERVER_STORE_ACCESS).callback_registry
}

pub(crate) fn register_realtime_resize_observer(host: &JsContextHost, id: ObserverCallbackId) {
    if host.layout_policy().is_realtime() {
        registry(host).register_realtime_resize_observer(host, id);
    }
}

pub(crate) fn activate_realtime_resize_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    id: ObserverCallbackId,
    observer: v8::Local<'s, v8::Object>,
) {
    if registry(host).activate_realtime_resize_observer(scope, host, id, observer) {
        host.refresh_realtime_rendering_opportunity();
    }
}

pub(crate) fn deactivate_realtime_resize_observer(host: &JsContextHost, id: ObserverCallbackId) {
    registry(host).deactivate_realtime_resize_observer(id);
    host.refresh_realtime_rendering_opportunity();
}

pub(crate) fn request_realtime_resize_observer_checks(host: &JsContextHost) {
    registry(host).request_realtime_resize_observer_checks();
}

pub(crate) fn request_realtime_resize_observer_document(
    host: &JsContextHost,
    target: WindowDocumentTaskTarget,
) {
    registry(host).request_realtime_resize_observer_document(target);
    host.refresh_realtime_rendering_opportunity();
}

pub(crate) fn has_pending_realtime_resize_observers(
    host: &JsContextHost,
    target: Option<WindowDocumentTaskTarget>,
) -> bool {
    registry(host).has_pending_realtime_resize_observers(host, target)
}

pub(crate) fn take_realtime_resize_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    target: WindowDocumentTaskTarget,
) -> Vec<(ObserverCallbackId, v8::Local<'s, v8::Object>)> {
    registry(host).take_realtime_resize_observers(scope, host, target)
}

pub(crate) fn rebind_realtime_resize_observer_document(
    host: &JsContextHost,
    retired: WindowDocumentOwner,
    current: WindowDocumentOwner,
) {
    registry(host).rebind_realtime_resize_observer_document(retired, current);
}

pub(crate) fn retire_realtime_resize_observer_document(
    host: &JsContextHost,
    retired: WindowDocumentOwner,
) {
    registry(host).retire_realtime_resize_observer_document(retired);
}

pub(crate) fn prune_realtime_resize_observers(host: &JsContextHost) {
    registry(host).prune_realtime_resize_observers(host);
}

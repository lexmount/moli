use super::*;
use crate::common_runtime::{CallbackRegistrationId, RealtimeResizeObserverSampling};
use crate::native_bridge::{
    WindowDocumentOwner, WindowDocumentTaskTarget, WindowExecutionContextOwner,
};

fn registry(host: &JsContextHost) -> &crate::common_runtime::CallbackRegistry {
    &host.common.callbacks
}

pub(crate) fn register_realtime_resize_observer(host: &JsContextHost, id: CallbackRegistrationId) {
    if host.layout_policy().is_realtime() {
        registry(host).register_realtime_resize_observer(host, id);
    }
}

pub(crate) fn ensure_realtime_resize_observer_controller(
    host: &JsContextHost,
    target: WindowDocumentTaskTarget,
) {
    registry(host).ensure_realtime_resize_observer_controller(host, target);
}

pub(crate) fn retire_realtime_resize_observer_controller(
    host: &JsContextHost,
    owner: WindowExecutionContextOwner,
) {
    registry(host).retire_realtime_resize_observer_controller(owner);
}

pub(crate) fn activate_realtime_resize_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    id: CallbackRegistrationId,
    observer: v8::Local<'s, v8::Object>,
) {
    if registry(host).activate_realtime_resize_observer(scope, host, id, observer) {
        host.refresh_realtime_rendering_opportunity();
    }
}

pub(crate) fn deactivate_realtime_resize_observer(
    host: &JsContextHost,
    id: CallbackRegistrationId,
) {
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
) -> (
    bool,
    Vec<(CallbackRegistrationId, v8::Local<'s, v8::Object>)>,
) {
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

pub(crate) fn begin_realtime_resize_observer_sampling(
    host: &JsContextHost,
    sources: Vec<crate::document_runtime::DomHandle>,
) -> Result<RealtimeResizeObserverSampling, moli_layout::LayoutError> {
    registry(host).begin_realtime_resize_observer_sampling(sources)
}

pub(crate) fn realtime_resize_observer_box_was_removed(
    host: &JsContextHost,
    source: crate::document_runtime::DomHandle,
) -> bool {
    registry(host).realtime_resize_observer_box_was_removed(source)
}

pub(crate) fn discard_realtime_resize_observer_tree_boxes(
    host: &JsContextHost,
    dom: &DomHost,
    effects: &DomMutationEffects,
) {
    if host.layout_policy().is_realtime() {
        registry(host).discard_realtime_resize_observer_tree_boxes(dom, effects);
    }
}

pub(crate) fn discard_realtime_resize_observer_document_boxes(
    host: &JsContextHost,
    document: crate::document_runtime::DomHandle,
) {
    if host.layout_policy().is_realtime() {
        registry(host).discard_realtime_resize_observer_document_boxes(host.dom_host(), document);
    }
}

pub(crate) fn did_publish_realtime_resize_observer_layout(host: &JsContextHost) {
    registry(host).did_publish_realtime_resize_observer_layout();
}

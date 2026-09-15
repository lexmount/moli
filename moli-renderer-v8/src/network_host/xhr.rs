mod bindings;
mod delivery;
mod encoding;
mod events;
mod header_surface;
mod instance_state;
mod response_type;
mod send;
mod upload;

use super::*;
use crate::context_bootstrap::{
    install_simple_event_target_ordered_handlers, mark_simple_event_target_slot,
};
use crate::native_bridge::throw_dom_exception;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::worker::WORKER_STATE_SLOT;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ProgressEvent)]
struct ProgressEventPrototypeDeclaration {
    #[webapi(
        accessor_property = "lengthComputable",
        getter = progress_event_length_computable_function_getter,
        enumerable
    )]
    _length_computable: (),
    #[webapi(accessor_property, getter = progress_event_loaded_function_getter, enumerable)]
    loaded: (),
    #[webapi(accessor_property, getter = progress_event_total_function_getter, enumerable)]
    total: (),
}

pub(crate) const XHR_ABORTED_SLOT: &str = "__lmXhrAborted";
pub(crate) const XHR_ASYNC_SLOT: &str = "__lmXhrAsync";
pub(crate) const XHR_SEND_FLAG_SLOT: &str = "__lmXhrSendFlag";
pub(crate) const XHR_UPLOAD_IN_PROGRESS_SLOT: &str = "__lmXhrUploadInProgress";
const XHR_UPLOAD_LOADED_SLOT: &str = "__lmXhrUploadLoaded";
const XHR_UPLOAD_LISTENER_SLOT: &str = "__lmXhrUploadListener";
const XHR_PENDING_KIND_SLOT: &str = "__lmXhrPendingKind";
const XHR_PENDING_STATUS_SLOT: &str = "__lmXhrPendingStatus";
const XHR_PENDING_STATUS_TEXT_SLOT: &str = "__lmXhrPendingStatusText";
const XHR_PENDING_URL_SLOT: &str = "__lmXhrPendingUrl";
const XHR_PENDING_BODY_BYTES_SLOT: &str = "__lmXhrPendingBodyBytes";
const XHR_PENDING_HEADERS_SLOT: &str = "__lmXhrPendingHeadersJson";
pub(crate) const XHR_ACTIVE_INTERNAL_ID_SLOT: &str = "__lmXhrActiveInternalId";
pub(crate) const XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT: &str = "__moliXhrEventTargetListeners";
pub(crate) const XHR_SIMPLE_EVENT_TARGET_MARKER_SLOT: &str = "__moliXhrUsesSimpleEventTarget";

pub(crate) use self::bindings::{
    install_window_xml_http_request_template_bindings, install_xml_http_request_bindings,
    install_xml_http_request_event_target_bindings, progress_event_constructor_callback,
    progress_event_length_computable_function_getter, progress_event_loaded_function_getter,
    progress_event_total_function_getter, xhr_constructor_callback,
};
pub(crate) use self::delivery::{
    apply_xhr_failure, apply_xhr_response, apply_xhr_response_body_source,
    apply_xhr_response_body_source_with_status_text, apply_xhr_streaming_failure,
    apply_xhr_streaming_response_body_source, apply_xhr_streaming_response_chunk,
    apply_xhr_streaming_response_head, apply_xhr_timeout, finish_xhr_abort,
    throw_synchronous_xhr_failure,
};
pub(crate) use self::encoding::xhr_response_text_decoder;
pub(crate) use self::instance_state::{
    XHR_METHOD_SLOT, XHR_OPEN_GENERATION_SLOT, XHR_READY_STATE_SLOT, XHR_REQUEST_HEADERS_SLOT,
    XHR_RESPONSE_URL_SLOT, XHR_RESPONSE_XML_SLOT, XHR_STATUS_SLOT, XHR_STATUS_TEXT_SLOT,
    XHR_TIMEOUT_SLOT, XHR_TIMEOUT_START_MS_SLOT, XHR_TIMEOUT_TIMER_SLOT, XHR_URL_SLOT,
    XHR_WITH_CREDENTIALS_SLOT, set_xhr_state_bool, set_xhr_state_number, set_xhr_state_string,
    set_xhr_state_value, xhr_execution_context_binding, xhr_state_bool_property,
    xhr_state_number_property, xhr_state_string_property,
};
use self::instance_state::{
    XHR_OVERRIDE_MIME_TYPE_SLOT, XHR_RESPONSE_HEADERS_SLOT, XHR_RESPONSE_SLOT,
    XHR_RESPONSE_TEXT_SLOT, XHR_RESPONSE_TYPE_SLOT, configure_xml_http_request_instance_template,
    initialize_xml_http_request_instance, xhr_state_value, xhr_upload_object,
};
use self::response_type::{XmlHttpRequestResponseType, xhr_response_type};
#[cfg(test)]
pub(crate) use self::send::prepare_xhr_send_body;
pub(crate) use self::send::{
    PreparedXhrSendBody, capture_xhr_upload_listener_flag, dispatch_xhr_loadstart,
    prepare_xhr_send_body_from_args, xhr_author_request_headers,
};
pub(crate) use self::upload::apply_xhr_upload_event;

pub(crate) fn install_progress_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "ProgressEvent" {
        ProgressEventPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(crate) fn finalize_xml_http_request_event_target_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_target_prototype: v8::Local<'s, v8::Object>,
) {
    mark_simple_event_target_slot(
        scope,
        event_target_prototype,
        XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
    );
    install_simple_event_target_ordered_handlers(scope, event_target_prototype);
    let marker = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        event_target_prototype,
        XHR_SIMPLE_EVENT_TARGET_MARKER_SLOT,
        marker.into(),
    );
}

pub(crate) fn xhr_throw_invalid_state(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidStateError", 11, message);
}

pub(crate) fn xhr_throw_invalid_access(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidAccessError", 15, message);
}

pub(crate) fn xhr_current_context_is_worker_global(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WORKER_STATE_SLOT)
        .and_then(|value| v8::Local::<v8::External>::try_from(value).ok())
        .is_some()
}

pub(crate) fn xhr_is_synchronous_document_request(
    scope: &mut v8::PinScope<'_, '_>,
    async_request: bool,
) -> bool {
    !async_request && !xhr_current_context_is_worker_global(scope)
}

pub(crate) fn xhr_ensure_send_allowed(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> bool {
    let ready_state = xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0);
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    if ready_state as u32 == 1 && !send_flag {
        return true;
    }
    xhr_throw_invalid_state(
        scope,
        "XMLHttpRequest.send() is not allowed in the current state.",
    );
    false
}

mod bindings;
mod delivery;
mod events;
mod header_surface;
mod instance_state;
mod request_state;
mod response_type;
mod send;

use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::get_private_value;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

// Generated guards validate the interface. Resolve native aliases before XHR
// slots, owner realms, Blob capture and async jobs use the backing object.
fn xhr_receiver_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let receiver = v8::Local::new(scope, receiver);
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("generated XMLHttpRequest receiver validation")
}

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
const XHR_UPLOAD_LISTENER_SLOT: &str = "__lmXhrUploadListener";
pub(crate) const XHR_ACTIVE_INTERNAL_ID_SLOT: &str = "__lmXhrActiveInternalId";

pub(crate) use self::bindings::{
    install_window_xml_http_request_template_bindings, install_xml_http_request_bindings,
    install_xml_http_request_event_target_bindings, progress_event_constructor_callback,
    progress_event_length_computable_function_getter, progress_event_loaded_function_getter,
    progress_event_total_function_getter, xhr_constructor_callback,
};
pub(crate) use self::delivery::{
    apply_xhr_failure, apply_xhr_response, apply_xhr_response_body_source,
    apply_xhr_response_body_source_with_status_text, apply_xhr_streaming_response_body_source,
    apply_xhr_streaming_response_chunk, apply_xhr_streaming_response_head, apply_xhr_timeout,
    finish_xhr_abort, throw_synchronous_xhr_failure,
};
pub(crate) use self::instance_state::xhr_has_upload_listeners;
pub(crate) use self::instance_state::{
    XHR_METHOD_SLOT, XHR_OPEN_GENERATION_SLOT, XHR_REQUEST_HEADERS_SLOT, XHR_RESPONSE_URL_SLOT,
    XHR_RESPONSE_XML_SLOT, XHR_STATUS_SLOT, XHR_STATUS_TEXT_SLOT, XHR_TIMEOUT_SLOT,
    XHR_TIMEOUT_START_MS_SLOT, XHR_TIMEOUT_TIMER_SLOT, XHR_URL_SLOT, XHR_WITH_CREDENTIALS_SLOT,
    set_xhr_state_bool, set_xhr_state_number, set_xhr_state_string, set_xhr_state_value,
    xhr_execution_context_binding, xhr_state_bool_property, xhr_state_number_property,
    xhr_state_string_property,
};
use self::instance_state::{
    XHR_OVERRIDE_MIME_TYPE_SLOT, XHR_READY_STATE_SLOT, XHR_RESPONSE_HEADERS_SLOT,
    XHR_RESPONSE_SLOT, XHR_RESPONSE_TEXT_SLOT, XHR_RESPONSE_TYPE_SLOT,
    configure_xml_http_request_instance_template, initialize_xml_http_request_instance,
    xhr_state_value, xhr_upload_object,
};
pub(crate) use self::request_state::begin_xhr_send;
use self::request_state::{
    XhrReadyState, clear_xhr_active_request, set_xhr_ready_state, xhr_ready_state,
};
use self::response_type::XmlHttpRequestResponseType;
#[cfg(test)]
pub(crate) use self::send::prepare_xhr_send_body;
pub(crate) use self::send::{
    PreparedXhrSendBody, convert_xhr_send_body_from_args, dispatch_xhr_loadstart,
    dispatch_xhr_upload_complete, xhr_author_request_headers,
};

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

pub(crate) fn xhr_throw_invalid_state(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidStateError", 11, message);
}

pub(crate) fn xhr_throw_invalid_access(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidAccessError", 15, message);
}

pub(crate) fn xhr_current_context_is_worker_global(scope: &mut v8::PinScope<'_, '_>) -> bool {
    crate::worker::get_worker_state(scope).is_some()
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
    let ready_state = xhr_ready_state(scope, xhr);
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    if ready_state == XhrReadyState::Opened && !send_flag {
        return true;
    }
    xhr_throw_invalid_state(
        scope,
        "XMLHttpRequest.send() is not allowed in the current state.",
    );
    false
}

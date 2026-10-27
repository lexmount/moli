mod abort;
mod constants;
mod constructor;
mod open;
mod progress_event;
mod prototype;

use super::events::xhr_fire_readystatechange;
use super::header_surface::{
    xhr_get_all_response_headers_callback, xhr_get_response_header_callback,
    xhr_override_mime_type_callback, xhr_set_request_header_callback,
};
use super::send::xhr_send_callback;
use super::*;

/// Retire the native request without dispatching the public abort algorithm.
/// open() and abort() each retain their own state and event transitions.
fn terminate_window_xhr_fetch(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    let internal_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u64;
    if internal_id == 0 {
        return;
    }
    let Some(context) = xhr.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: this context's bridge owns the host in the current isolate.
        let _ = unsafe { &mut *host }.abort_subresource_fetch(internal_id);
    }
}

pub(crate) use self::constants::{
    install_window_xml_http_request_template_bindings, install_xml_http_request_template_surface,
};
pub(crate) use self::constructor::xhr_constructor_callback;
pub(super) use self::progress_event::construct_native_progress_event;
pub(crate) use self::progress_event::{
    progress_event_constructor_callback, progress_event_length_computable_function_getter,
    progress_event_loaded_function_getter, progress_event_total_function_getter,
};
pub(crate) use self::prototype::{
    install_xml_http_request_bindings, install_xml_http_request_event_target_bindings,
};

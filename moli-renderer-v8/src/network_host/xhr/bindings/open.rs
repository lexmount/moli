use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.open")]
struct XhrOpenArgs {
    #[webidl(required, converter = "byte_string")]
    method: String,
    #[webidl(required, converter = "usv_string")]
    url: String,
    #[webidl(default = true)]
    async_request: bool,
    #[webidl(converter = "usv_string", nullable)]
    username: Option<String>,
    #[webidl(converter = "usv_string", nullable)]
    password: Option<String>,
}

pub(super) fn xhr_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = args.this();
    let Some(parsed) = webidl::parse_args::<XhrOpenArgs>(scope, &args) else {
        return;
    };
    let method = match normalize_request_method(&parsed.method) {
        Ok(method) => method,
        Err(message) => {
            throw_type_error(scope, message);
            return;
        }
    };
    let Some(request_url) = xhr_open_request_url(
        scope,
        xhr,
        &parsed.url,
        parsed.username.as_deref(),
        parsed.password.as_deref(),
    ) else {
        return;
    };
    let timeout = xhr_state_number_property(scope, xhr, XHR_TIMEOUT_SLOT).unwrap_or(0.0);
    if timeout != 0.0 && xhr_is_synchronous_document_request(scope, parsed.async_request) {
        xhr_throw_invalid_access(
            scope,
            "Failed to execute 'open' on 'XMLHttpRequest': Synchronous requests must not set a timeout.",
        );
        return;
    }
    let response_type = xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default);
    if response_type != XmlHttpRequestResponseType::Default
        && xhr_is_synchronous_document_request(scope, parsed.async_request)
    {
        xhr_throw_invalid_access(
            scope,
            "Failed to execute 'open' on 'XMLHttpRequest': Synchronous requests from a document must not set a response type.",
        );
        return;
    }
    let previous_ready_state =
        xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0) as u32;
    super::super::delivery::cancel_xhr_timeout(scope, xhr);
    super::super::delivery::clear_xhr_progress_throttle(scope, xhr);
    super::super::delivery::clear_xhr_timeout_start(scope, xhr);
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    set_xhr_state_number(scope, xhr, XHR_OPEN_GENERATION_SLOT, open_generation + 1.0);
    set_xhr_state_string(scope, xhr, XHR_METHOD_SLOT, &method);
    set_xhr_state_string(scope, xhr, XHR_URL_SLOT, request_url.as_str());
    set_blob_url_entry(scope, xhr, CapturedBlobUrl::capture(&request_url));
    set_xhr_state_string(scope, xhr, XHR_REQUEST_HEADERS_SLOT, "[]");
    set_xhr_state_bool(scope, xhr, XHR_ASYNC_SLOT, parsed.async_request);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 1.0);
    set_xhr_state_number(scope, xhr, XHR_STATUS_SLOT, 0.0);
    set_xhr_state_string(scope, xhr, XHR_STATUS_TEXT_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_TEXT_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_URL_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_HEADERS_SLOT, "[]");
    set_xhr_state_string(scope, xhr, XHR_PENDING_KIND_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_PENDING_URL_SLOT, "");
    set_xhr_state_value(
        scope,
        xhr,
        XHR_PENDING_BODY_BYTES_SLOT,
        v8::undefined(scope).into(),
    );
    set_xhr_state_string(scope, xhr, XHR_PENDING_HEADERS_SLOT, "[]");
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_number(scope, xhr, XHR_PENDING_STATUS_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_LISTENER_SLOT, false);
    let empty_response: v8::Local<'_, v8::Value> = v8_string(scope, "")
        .map(|s| s.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_SLOT, empty_response);
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, v8::null(scope).into());
    if previous_ready_state != 1 {
        xhr_fire_readystatechange(scope, xhr, 1);
    }
}

fn xhr_open_request_url(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    input: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Option<url::Url> {
    let base_url = if xhr_current_context_is_worker_global(scope) {
        crate::worker::worker_current_script_url(scope)
    } else {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            xhr_open_throw_invalid_state(
                scope,
                "Failed to execute 'open' on 'XMLHttpRequest': The object's document is not fully active.",
            );
            return None;
        };
        let host = unsafe { &*host_ptr };
        let Some(execution_context) = xhr_execution_context_binding(scope, host, xhr) else {
            xhr_open_throw_invalid_state(
                scope,
                "Failed to execute 'open' on 'XMLHttpRequest': The object's document is not fully active.",
            );
            return None;
        };
        match execution_context.dispatch_scope() {
            crate::native_bridge::OwnerDispatchScope::Top => {
                Some(host.document_base_url_for_handle(host.document_handle()))
            }
            crate::native_bridge::OwnerDispatchScope::Child(handle) => {
                host.child_browsing_context_base_url(handle)
            }
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
                host.lightweight_popup_request_base_url(scope, popup_id)
            }
        }
    };
    let Some(base_url) = base_url else {
        xhr_open_throw_invalid_state(
            scope,
            "Failed to execute 'open' on 'XMLHttpRequest': The object's execution context is unavailable.",
        );
        return None;
    };
    let mut request_url = match resolve_context_url(&base_url, input, None) {
        Ok(url) => url,
        Err(_) => {
            throw_dom_exception(
                scope,
                "SyntaxError",
                12,
                "Failed to execute 'open' on 'XMLHttpRequest': The URL is invalid.",
            );
            return None;
        }
    };
    if request_url.host().is_some() {
        if let Some(username) = username {
            let _ = request_url.set_username(username);
        }
        if let Some(password) = password {
            let _ = request_url.set_password(Some(password));
        }
    }
    Some(request_url)
}

fn xhr_open_throw_invalid_state(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    let current_context = scope.get_current_context();
    let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
    let incumbent_scope = &mut v8::ContextScope::new(scope, incumbent_context);
    xhr_throw_invalid_state(incumbent_scope, message);
}

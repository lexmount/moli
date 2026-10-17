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
    let xhr = xhr_receiver_target(scope, args.this());
    let Some(parsed) = webidl::parse_args::<XhrOpenArgs>(scope, &args) else {
        return;
    };
    let Some(url_context) = xhr_open_url_context(scope, xhr) else {
        return;
    };
    let method = match normalize_request_method(&parsed.method) {
        Ok(method) => method,
        Err(RequestMethodError::InvalidToken) => {
            throw_dom_exception(
                scope,
                "SyntaxError",
                12,
                "Failed to execute 'open' on 'XMLHttpRequest': The method is not a valid HTTP token.",
            );
            return;
        }
        Err(RequestMethodError::Forbidden) => {
            throw_dom_exception(
                scope,
                "SecurityError",
                18,
                "Failed to execute 'open' on 'XMLHttpRequest': The method is forbidden.",
            );
            return;
        }
    };
    let Some(request_url) = xhr_open_request_url(
        scope,
        &url_context,
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
    let previous_ready_state = xhr_ready_state(scope, xhr);
    terminate_window_xhr_fetch(scope, xhr);
    crate::worker::try_worker_xhr_terminate_fetch(scope, xhr);
    super::super::delivery::clear_xhr_request_timers(scope, xhr);
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    set_xhr_state_number(scope, xhr, XHR_OPEN_GENERATION_SLOT, open_generation + 1.0);
    set_xhr_state_string(scope, xhr, XHR_METHOD_SLOT, &method);
    set_xhr_state_string(scope, xhr, XHR_URL_SLOT, request_url.as_str());
    set_blob_url_entry(scope, xhr, CapturedBlobUrl::capture(&request_url));
    set_xhr_state_string(scope, xhr, XHR_REQUEST_HEADERS_SLOT, "[]");
    set_xhr_state_bool(scope, xhr, XHR_ASYNC_SLOT, parsed.async_request);
    set_xhr_ready_state(scope, xhr, XhrReadyState::Opened);
    super::super::delivery::reset_xhr_response(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_LISTENER_SLOT, false);
    if previous_ready_state != XhrReadyState::Opened {
        xhr_fire_readystatechange(scope, xhr, XhrReadyState::Opened);
    }
}

struct XhrOpenUrlContext {
    base_url: url::Url,
    query_encoding: Option<&'static encoding_rs::Encoding>,
}

impl XhrOpenUrlContext {
    fn parse(&self, input: &str) -> Result<url::Url, url::ParseError> {
        let options = url::Url::options().base_url(Some(&self.base_url));
        if let Some(encoding) = self.query_encoding {
            options
                .encoding_override(Some(&|query| {
                    moli_encoding::encode_url_query_for_url_parser(query, encoding)
                }))
                .parse(input)
        } else {
            options.parse(input)
        }
    }
}

fn xhr_open_url_context(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<XhrOpenUrlContext> {
    let context = if xhr_current_context_is_worker_global(scope) {
        crate::worker::worker_current_script_url(scope).map(|base_url| XhrOpenUrlContext {
            base_url,
            query_encoding: None,
        })
    } else {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            xhr_throw_invalid_state(
                scope,
                "Failed to execute 'open' on 'XMLHttpRequest': The object's document is not fully active.",
            );
            return None;
        };
        let host = unsafe { &*host_ptr };
        let Some(execution_context) = xhr_execution_context_binding(scope, host, xhr) else {
            xhr_throw_invalid_state(
                scope,
                "Failed to execute 'open' on 'XMLHttpRequest': The object's document is not fully active.",
            );
            return None;
        };
        // Both settings belong to the XHR's live owner, including when open()
        // is borrowed from a different realm. Read them after WebIDL argument
        // conversion, which can change the base or retire the owner.
        let owner = execution_context.dispatch_scope();
        let document = match owner {
            crate::native_bridge::OwnerDispatchScope::Top => Some(host.document_handle()),
            crate::native_bridge::OwnerDispatchScope::Child(handle) => {
                host.child_browsing_context_document_handle(handle)
            }
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
                host.lightweight_popup_document_handle(popup_id)
            }
        };
        let query_encoding = document
            .and_then(|document| host.document_character_set_for_handle(document))
            .and_then(moli_encoding::encoding_for_label)
            .filter(|encoding| *encoding != encoding_rs::UTF_8);
        subresource_api_base_url(scope, host, owner).map(|base_url| XhrOpenUrlContext {
            base_url,
            query_encoding,
        })
    };
    if context.is_none() {
        xhr_throw_invalid_state(
            scope,
            "Failed to execute 'open' on 'XMLHttpRequest': The object's execution context is unavailable.",
        );
    }
    context
}

fn xhr_open_request_url(
    scope: &mut v8::PinScope<'_, '_>,
    url_context: &XhrOpenUrlContext,
    input: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Option<url::Url> {
    let mut request_url = match url_context.parse(input) {
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

#[cfg(test)]
mod tests {
    use super::XhrOpenUrlContext;

    #[test]
    fn xhr_query_encoding_respects_url_parser_component_and_scheme_rules() {
        let context = XhrOpenUrlContext {
            base_url: url::Url::parse("https://example.test/base/page").unwrap(),
            query_encoding: Some(encoding_rs::WINDOWS_1252),
        };
        for (input, expected) in [
            ("?ß", "https://example.test/base/page?%DF"),
            ("?\u{fffd}", "https://example.test/base/page?%26%2365533%3B"),
            ("/ß?ß#ß", "https://example.test/%C3%9F?%DF#%C3%9F"),
            (
                "?a=ß&b=%DF+ß",
                "https://example.test/base/page?a=%DF&b=%DF+%DF",
            ),
            ("?ß\t\r\nß ", "https://example.test/base/page?%DF%DF"),
            ("#?ß", "https://example.test/base/page#?%C3%9F"),
            ("data:text/plain,?ß", "data:text/plain,?%C3%9F"),
            ("ws://example.test/?ß", "ws://example.test/?%C3%9F"),
            ("wss://example.test/?ß", "wss://example.test/?%C3%9F"),
            ("custom://example.test/?ß", "custom://example.test/?%C3%9F"),
            ("https://example.test/?ß", "https://example.test/?%DF"),
        ] {
            assert_eq!(
                context.parse(input).unwrap().as_str(),
                expected,
                "{input:?}"
            );
        }
        assert!(context.parse("https://[").is_err());
    }
}

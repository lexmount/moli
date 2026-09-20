use super::*;
use crate::webidl;
use http::HeaderName;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest header")]
struct XhrHeaderNameArgs {
    #[webidl(required, converter = "byte_string")]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.setRequestHeader")]
struct XhrSetRequestHeaderArgs {
    #[webidl(required, converter = "byte_string")]
    name: String,
    #[webidl(required, converter = "byte_string")]
    value: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.overrideMimeType")]
struct XhrOverrideMimeTypeArgs {
    #[webidl(required)]
    mime: String,
}

pub(super) fn xhr_set_request_header_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = args.this();
    let Some(parsed) = webidl::parse_args::<XhrSetRequestHeaderArgs>(scope, &args) else {
        return;
    };
    let ready_state = xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0);
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    if ready_state as u32 != 1 || send_flag {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.setRequestHeader cannot be called in the current state.",
        );
        return;
    }

    let existing_json = xhr_state_string_property(scope, xhr, XHR_REQUEST_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());
    let mut pairs: Vec<[String; 2]> = serde_json::from_str(&existing_json).unwrap_or_default();
    let lower_name = parsed.name.to_ascii_lowercase();
    if let Some(pair) = pairs
        .iter_mut()
        .find(|pair| pair[0].to_ascii_lowercase() == lower_name)
    {
        pair[1] = parsed.value;
    } else {
        pairs.push([parsed.name, parsed.value]);
    }
    let new_json = serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_owned());
    set_xhr_state_string(scope, xhr, XHR_REQUEST_HEADERS_SLOT, &new_json);
}

pub(super) fn xhr_response_headers_for_script(
    headers: &[(String, String)],
) -> Vec<(String, String)> {
    use crate::network_host::headers::{
        HeadersGuard, filter_headers_for_guard, normalized_headers_entries,
    };

    let mut headers =
        normalized_headers_entries(&filter_headers_for_guard(headers, HeadersGuard::Response));
    // XHR uses legacy uppercase-byte order, unlike Headers iteration. Keep
    // the serialized names lowercase and preserve duplicate values in order.
    headers.sort_by(|(left, _), (right, _)| {
        left.bytes()
            .map(|byte| byte.to_ascii_uppercase())
            .cmp(right.bytes().map(|byte| byte.to_ascii_uppercase()))
    });
    headers
}

pub(super) fn xhr_get_all_response_headers_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = args.this();
    let headers_json = xhr_state_string_property(scope, xhr, XHR_RESPONSE_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());

    let formatted =
        if let Ok(headers) = serde_json::from_str::<Vec<(String, String)>>(&headers_json) {
            headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>()
        } else {
            String::new()
        };

    if let Some(result) = v8_string(scope, &formatted) {
        rv.set(result.into());
    } else {
        rv.set(
            v8_string(scope, "")
                .map(|s| s.into())
                .unwrap_or_else(|| v8::undefined(scope).into()),
        );
    }
}

pub(super) fn xhr_get_response_header_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = args.this();
    let Some(parsed) = webidl::parse_args::<XhrHeaderNameArgs>(scope, &args) else {
        rv.set_null();
        return;
    };
    let headers_json = xhr_state_string_property(scope, xhr, XHR_RESPONSE_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());

    if let (Some(name), Ok(headers)) = (
        HeaderName::from_bytes(parsed.name.as_bytes()).ok(),
        serde_json::from_str::<Vec<(String, String)>>(&headers_json),
    ) {
        let values: Vec<&str> = headers
            .iter()
            .filter(|(header_name, _)| response_header_name_is(header_name, &name))
            .map(|(_, value)| value.as_str())
            .collect();

        if !values.is_empty() {
            let combined = values.join(", ");
            if let Some(result) = v8_string(scope, &combined) {
                rv.set(result.into());
                return;
            }
        }
    }

    rv.set_null();
}

pub(super) fn xhr_override_mime_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = args.this();
    let Some(parsed) = webidl::parse_args::<XhrOverrideMimeTypeArgs>(scope, &args) else {
        return;
    };
    let ready_state = xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0);
    if matches!(ready_state as u32, 3 | 4) {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.overrideMimeType() cannot be called once loading has started.",
        );
        return;
    }
    let mime = moli_web_mime::parse_mime_type(&parsed.mime)
        .map(|mime| mime.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    set_xhr_state_string(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT, &mime);
    rv.set_undefined();
}

fn response_header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}

use super::*;
use crate::webidl;
use std::str::FromStr;

pub(super) struct ParsedWindowFetchInput {
    pub(super) url: String,
    pub(super) method: String,
    pub(super) body: Option<Vec<u8>>,
    pub(super) body_stream: Option<v8::Global<v8::Object>>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) suppress_default_content_type: bool,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(super) redirect_mode: moli_fetch::RequestRedirectMode,
    pub(super) priority: Option<moli_fetch::FetchPriorityHint>,
    pub(super) cache: String,
    pub(super) referrer: String,
    pub(super) referrer_policy: String,
    pub(super) integrity: String,
    pub(super) keepalive: bool,
    pub(super) request_body_owner: Option<v8::Global<v8::Object>>,
    pub(super) init_validation: super::super::request::RequestInitValidation,
}

pub(super) fn parse_window_fetch_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<ParsedWindowFetchInput, String> {
    if args.length() < 1 {
        return Err(
            webidl::WebIdlError::missing_required(webidl::Context::argument("fetch", 1))
                .to_string(),
        );
    }
    let arg0 = args.get(0);
    let inherited = request_input_snapshot(scope, arg0).map_err(|error| error.to_string())?;
    if let Some(inherited) = inherited {
        let req_obj = v8::Local::<v8::Object>::try_from(arg0).expect("request-like object");
        let url = inherited.url.clone();
        let init = parse_fetch_init(scope, args, 1)?;
        let body_stream = if init.body_present {
            init.body_stream.clone()
        } else {
            body_stream_object(scope, req_obj).map(|stream| v8::Global::new(scope, stream))
        };
        let request_body_owner = (!init.body_present
            && (inherited.body.is_some() || body_stream.is_some()))
        .then(|| v8::Global::new(scope, req_obj));
        let method = if init.method_present {
            init.method.clone()
        } else {
            inherited.method.clone()
        };
        let body = if init.body_present {
            init.body.clone()
        } else {
            inherited.body.clone()
        };
        let mut headers = if init.headers_present {
            init.headers.clone()
        } else {
            inherited.headers.clone()
        };
        let suppress_default_content_type = if init.body_present {
            if !init.headers_present {
                append_default_body_content_type(&mut headers, init.body_content_type.as_deref());
            }
            init.suppress_default_content_type
                || (body.is_some()
                    && init.body_content_type.is_none()
                    && !has_header(&headers, "content-type"))
        } else {
            body.is_some() && !has_header(&headers, "content-type")
        };
        let inherited_credentials = request_object_credentials_mode(scope, req_obj)?;
        let request_mode = init
            .validation
            .mode
            .or_else(|| moli_fetch::RequestMode::from_str(&inherited.mode).ok())
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_no_cors_method(request_mode, &method)?;
        let has_stream_body = init.body_stream.is_some()
            || !init.body_present && inherited.body.is_none() && body_stream.is_some();
        validate_fetch_body(
            body.is_some() || body_stream.is_some(),
            has_stream_body,
            &method,
            request_mode,
        )?;
        if request_mode == moli_fetch::RequestMode::NoCors {
            headers = filter_headers_for_guard(&headers, HeadersGuard::RequestNoCors);
        }
        let credentials_mode = init
            .credentials_mode
            .or(inherited_credentials)
            .unwrap_or(moli_fetch::RequestCredentialsMode::SameOrigin);
        let redirect_mode = init
            .redirect_mode
            .or_else(|| parse_request_redirect_mode_label(&inherited.redirect))
            .unwrap_or(moli_fetch::RequestRedirectMode::Follow);
        let priority = init.priority.or_else(|| {
            (inherited.priority != moli_fetch::FetchPriorityHint::Auto)
                .then_some(inherited.priority)
        });
        Ok(ParsedWindowFetchInput {
            url,
            method,
            body,
            body_stream,
            headers,
            suppress_default_content_type,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            cache: init.cache.unwrap_or(inherited.cache),
            referrer: inherited.referrer,
            referrer_policy: init.referrer_policy.unwrap_or(inherited.referrer_policy),
            integrity: init.integrity.unwrap_or(inherited.integrity),
            keepalive: init.keepalive.unwrap_or(inherited.keepalive),
            request_body_owner,
            init_validation: init.validation,
        })
    } else {
        let url = fetch_request_info_url(scope, arg0)?;
        let init = parse_fetch_init(scope, args, 1)?;
        let request_mode = init
            .validation
            .mode
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_no_cors_method(request_mode, &init.method)?;
        validate_fetch_body(
            init.body.is_some() || init.body_stream.is_some(),
            init.body_stream.is_some(),
            &init.method,
            request_mode,
        )?;
        let headers = if request_mode == moli_fetch::RequestMode::NoCors {
            filter_headers_for_guard(&init.headers, HeadersGuard::RequestNoCors)
        } else {
            init.headers
        };
        let credentials_mode = init
            .credentials_mode
            .unwrap_or(moli_fetch::RequestCredentialsMode::SameOrigin);
        let redirect_mode = init
            .redirect_mode
            .unwrap_or(moli_fetch::RequestRedirectMode::Follow);
        Ok(ParsedWindowFetchInput {
            url,
            method: init.method,
            body: init.body,
            body_stream: init.body_stream,
            headers,
            suppress_default_content_type: init.suppress_default_content_type,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority: init.priority,
            cache: init.cache.unwrap_or_else(|| "default".to_owned()),
            referrer: "about:client".to_owned(),
            referrer_policy: init.referrer_policy.unwrap_or_default(),
            integrity: init.integrity.unwrap_or_default(),
            keepalive: init.keepalive.unwrap_or(false),
            request_body_owner: None,
            init_validation: init.validation,
        })
    }
}

fn validate_no_cors_method(
    request_mode: moli_fetch::RequestMode,
    method: &str,
) -> Result<(), String> {
    if request_mode == moli_fetch::RequestMode::NoCors
        && !moli_fetch::is_cors_safelisted_method(method)
    {
        return Err(format!(
            "Failed to execute 'fetch': method `{method}` is unsupported in no-cors mode."
        ));
    }
    Ok(())
}

fn fetch_request_info_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<String, String> {
    webidl::convert::<webidl::UsvString>(scope, value, webidl::Context::argument("fetch", 1))
        .map(Into::into)
        .map_err(|error| error.to_string())
}

use super::*;
use crate::webidl;
use std::str::FromStr;

pub(super) struct ParsedWindowFetchInput {
    pub(super) url: String,
    pub(super) blob_url_entry: Option<CapturedBlobUrl>,
    pub(super) method: String,
    pub(super) body: Option<bytes::Bytes>,
    pub(super) body_stream: Option<v8::Global<v8::Object>>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(super) redirect_mode: moli_fetch::RequestRedirectMode,
    pub(super) priority: Option<moli_fetch::FetchPriorityHint>,
    pub(super) cache: String,
    pub(super) referrer: String,
    pub(super) referrer_policy: String,
    pub(super) integrity: String,
    pub(super) keepalive: bool,
    pub(super) signal: Option<v8::Global<v8::Object>>,
    pub(super) request_body_owner: Option<v8::Global<v8::Object>>,
    pub(super) init_validation: super::super::request::RequestInitValidation,
}

pub(super) fn parse_window_fetch_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<ParsedWindowFetchInput, FetchArgumentError> {
    if args.length() < 1 {
        return Err(
            webidl::WebIdlError::missing_required(webidl::Context::argument("fetch", 1)).into(),
        );
    }
    let args = webidl::try_parse_args::<crate::network_host::FetchArgs>(scope, args)?;
    let (url, inherited, source) = args.input.materialize(scope)?;
    if let Some(inherited) = inherited {
        let req_obj = source.expect("request input source");
        let init = parse_fetch_init(scope, args.init)?;
        if inherited.body_unusable && !init.body_present {
            return Err(crate::network_host::request_body_already_used_error().into());
        }
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
        if init.body_present && !init.headers_present {
            append_default_body_content_type(&mut headers, init.body_content_type.as_deref());
        }
        let inherited_credentials = request_object_credentials_mode(scope, req_obj)?;
        let request_mode = init
            .validation
            .mode
            .or_else(|| moli_fetch::RequestMode::from_str(&inherited.mode).ok())
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_no_cors_method(request_mode, &method)?;
        let has_stream_body =
            init.body_stream.is_some() || !init.body_present && inherited.has_stream_body;
        validate_fetch_body(
            body.is_some() || body_stream.is_some(),
            has_stream_body,
            &method,
            request_mode,
        )?;
        if init.headers_present || request_mode == moli_fetch::RequestMode::NoCors {
            headers = filter_headers_for_guard(
                &headers,
                request_headers_guard_for_mode(request_mode.as_ref()),
            );
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
        // Legacy request-like signals have not passed RequestInit conversion.
        // Validate the selected value so an explicit init.signal can override it.
        let signal = init.signal.unwrap_or(inherited.signal);
        if let Some(signal) = signal.as_ref() {
            let signal = v8::Local::new(scope, signal);
            webidl::convert::<crate::abort_signal_route::ResolvedAbortSignal>(
                scope,
                signal.into(),
                webidl::Context::member("Request", "signal"),
            )?;
        }
        Ok(ParsedWindowFetchInput {
            url,
            blob_url_entry: inherited.blob_url_entry,
            method,
            body,
            body_stream,
            headers,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            cache: init.cache.unwrap_or(inherited.cache),
            referrer: inherited.referrer,
            referrer_policy: init.referrer_policy.unwrap_or(inherited.referrer_policy),
            integrity: init.integrity.unwrap_or(inherited.integrity),
            keepalive: init.keepalive.unwrap_or(inherited.keepalive),
            signal,
            request_body_owner,
            init_validation: init.validation,
        })
    } else {
        let init = parse_fetch_init(scope, args.init)?;
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
        let headers = filter_headers_for_guard(
            &init.headers,
            request_headers_guard_for_mode(request_mode.as_ref()),
        );
        let credentials_mode = init
            .credentials_mode
            .unwrap_or(moli_fetch::RequestCredentialsMode::SameOrigin);
        let redirect_mode = init
            .redirect_mode
            .unwrap_or(moli_fetch::RequestRedirectMode::Follow);
        Ok(ParsedWindowFetchInput {
            url,
            blob_url_entry: None,
            method: init.method,
            body: init.body,
            body_stream: init.body_stream,
            headers,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority: init.priority,
            cache: init.cache.unwrap_or_else(|| "default".to_owned()),
            referrer: "about:client".to_owned(),
            referrer_policy: init.referrer_policy.unwrap_or_default(),
            integrity: init.integrity.unwrap_or_default(),
            keepalive: init.keepalive.unwrap_or(false),
            signal: init.signal.flatten(),
            request_body_owner: None,
            init_validation: init.validation,
        })
    }
}

fn validate_no_cors_method(
    request_mode: moli_fetch::RequestMode,
    method: &str,
) -> Result<(), FetchArgumentError> {
    if request_mode == moli_fetch::RequestMode::NoCors
        && !moli_fetch::is_cors_safelisted_method(method)
    {
        return Err(FetchArgumentError::UnsupportedNoCorsMethod {
            method: method.to_owned(),
            interface: None,
        });
    }
    Ok(())
}

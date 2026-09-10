use super::super::*;

const CONTENT_TYPE_HEADER: &str = "Content-Type";
const URL_SEARCH_PARAMS_CONTENT_TYPE: &str = "application/x-www-form-urlencoded;charset=UTF-8";
const TEXT_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.send")]
struct XhrSendArgs<'s> {
    #[webidl(converter = "raw")]
    body: Option<v8::Local<'s, v8::Value>>,
}

pub(super) struct PreparedXhrSendRequest {
    pub(super) frame_id: Option<String>,
    pub(super) owner: crate::native_bridge::OwnerDispatchScope,
    pub(super) execution_context: crate::native_bridge::WindowExecutionContextBinding,
    pub(super) resource_loader: crate::network::context::DocumentResourceLoader,
    pub(super) document_url: url::Url,
    pub(super) network_partition_key: Option<String>,
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    pub(super) resolved_url: url::Url,
    pub(super) method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) cors_preflight_request_headers: Vec<(String, String)>,
    pub(super) send_body: Option<Vec<u8>>,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
}

pub(super) enum XhrSendPrepareError {
    ExecutionContext,
    Url(String),
}

pub(super) fn xhr_dom_debugger_request_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
) -> String {
    let raw_url = xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();
    if raw_url.is_empty() {
        return raw_url;
    }
    let resolved = xhr_execution_context_binding(scope, host, xhr)
        .and_then(|execution_context| {
            subresource_request_scope_for_owner(scope, host, execution_context.dispatch_scope())
        })
        .and_then(|(_, document_url)| resolve_context_url(&document_url, &raw_url, None).ok());
    resolved.map(|url| url.to_string()).unwrap_or(raw_url)
}

pub(super) fn prepare_xhr_send_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    method: String,
    prepared_body: PreparedXhrSendBody,
) -> Result<PreparedXhrSendRequest, XhrSendPrepareError> {
    let url_str = xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();

    let execution_context = xhr_execution_context_binding(scope, host, xhr)
        .ok_or(XhrSendPrepareError::ExecutionContext)?;
    let owner = execution_context.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)
        .ok_or(XhrSendPrepareError::ExecutionContext)?;
    let (frame_id, document_url) = subresource_request_scope_for_owner(scope, host, owner)
        .ok_or(XhrSendPrepareError::ExecutionContext)?;
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let resolved_url =
        resolve_context_url(&document_url, &url_str, None).map_err(XhrSendPrepareError::Url)?;
    let (request_headers, cors_preflight_request_headers) =
        xhr_request_headers(scope, host, xhr, prepared_body.default_content_type);
    let credentials_mode =
        if xhr_state_bool_property(scope, xhr, XHR_WITH_CREDENTIALS_SLOT).unwrap_or(false) {
            moli_fetch::RequestCredentialsMode::Include
        } else {
            moli_fetch::RequestCredentialsMode::SameOrigin
        };

    Ok(PreparedXhrSendRequest {
        frame_id,
        owner,
        execution_context,
        resource_loader,
        document_url,
        network_partition_key,
        policy_context,
        resolved_url,
        method,
        request_headers,
        cors_preflight_request_headers,
        send_body: prepared_body.body,
        credentials_mode,
    })
}

pub(crate) struct PreparedXhrSendBody {
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) default_content_type: Option<String>,
}

impl PreparedXhrSendBody {
    pub(crate) fn empty() -> Self {
        Self {
            body: None,
            default_content_type: None,
        }
    }

    fn new(body: Vec<u8>, default_content_type: Option<String>) -> Self {
        Self {
            body: Some(body),
            default_content_type,
        }
    }
}

pub(crate) fn prepare_xhr_send_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<PreparedXhrSendBody, crate::webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(PreparedXhrSendBody::empty());
    }

    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        if let Some((body, content_type)) =
            crate::context_bootstrap::form_data_request_body(scope, object)
        {
            return Ok(PreparedXhrSendBody::new(body, Some(content_type)));
        }
        if let Some(body) = crate::context_bootstrap::url_search_params_request_body(scope, object)
        {
            return Ok(PreparedXhrSendBody::new(
                body.into_bytes(),
                Some(URL_SEARCH_PARAMS_CONTENT_TYPE.to_owned()),
            ));
        }
        if let Some(bytes) = blob::blob_bytes_from_object(scope, object) {
            let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
            let default_content_type = (!mime_type.is_empty()).then_some(mime_type);
            return Ok(PreparedXhrSendBody::new(bytes, default_content_type));
        }
    }

    if let Some(bytes) = blob::buffer_source_bytes_from_value(scope, value) {
        return Ok(PreparedXhrSendBody::new(bytes, None));
    }

    let body = crate::webidl::convert::<crate::webidl::UsvString>(
        scope,
        value,
        crate::webidl::Context::argument("XMLHttpRequest.send", 1),
    )?;
    Ok(PreparedXhrSendBody::new(
        body.0.into_bytes(),
        Some(TEXT_CONTENT_TYPE.to_owned()),
    ))
}

pub(crate) fn prepare_xhr_send_body_from_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &str,
) -> Result<PreparedXhrSendBody, crate::webidl::WebIdlError> {
    let parsed = crate::webidl::try_parse_args::<XhrSendArgs<'s>>(scope, args)?;
    let Some(body) = parsed.body else {
        return Ok(PreparedXhrSendBody::empty());
    };
    if matches!(method, "GET" | "HEAD") {
        return Ok(PreparedXhrSendBody::empty());
    }
    prepare_xhr_send_body(scope, body)
}

fn xhr_request_headers(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    default_content_type: Option<String>,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let author_headers = xhr_author_request_headers(scope, xhr, default_content_type);
    let merged = merge_subresource_request_headers(host.extra_http_headers(), &author_headers);
    (merged, author_headers)
}

pub(crate) fn xhr_author_request_headers(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    default_content_type: Option<String>,
) -> Vec<(String, String)> {
    let headers_json = xhr_state_string_property(scope, xhr, XHR_REQUEST_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());
    let request_headers: Vec<[String; 2]> = serde_json::from_str(&headers_json).unwrap_or_default();
    let mut author_headers: Vec<(String, String)> = request_headers
        .into_iter()
        .map(|[name, value]| (name, value))
        .collect();
    if let Some(default_content_type) = default_content_type
        && !has_header(&author_headers, CONTENT_TYPE_HEADER)
    {
        author_headers.push((CONTENT_TYPE_HEADER.to_owned(), default_content_type));
    }
    author_headers
}

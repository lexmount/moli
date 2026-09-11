use super::super::*;
use crate::network_host::url_helpers::ResolveContextUrlError;

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
    pub(super) request_origin: moli_url::WebOrigin,
    pub(super) network_partition_key: Option<String>,
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    pub(super) resolved_url: url::Url,
    pub(super) blob_url_entry: Option<CapturedBlobUrl>,
    pub(super) method: String,
    pub(super) request_headers: moli_fetch::RequestHeaders,
    pub(super) cors_preflight_request_headers: Vec<(String, String)>,
    pub(super) send_body: Option<Vec<u8>>,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
}

pub(super) enum XhrSendPrepareError {
    ExecutionContext,
    Url(ResolveContextUrlError),
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
            let owner = execution_context.dispatch_scope();
            let loader = host.document_resource_loader_for_dispatch_scope(owner)?;
            host.subresource_request_environment(&loader, owner)
        })
        .and_then(|environment| resolve_context_url(&environment.base_url, &raw_url, None).ok());
    resolved.map(|url| url.to_string()).unwrap_or(raw_url)
}

pub(super) fn prepare_xhr_send_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    xhr: v8::Local<'s, v8::Object>,
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
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or(XhrSendPrepareError::ExecutionContext)?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        base_url,
        request_origin,
        frame_id,
    } = environment;
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let resolved_url =
        resolve_context_url(&base_url, &url_str, None).map_err(XhrSendPrepareError::Url)?;
    let (request_headers, cors_preflight_request_headers) =
        xhr_request_headers(scope, host, xhr, &prepared_body);
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
        request_origin,
        network_partition_key,
        policy_context,
        resolved_url,
        blob_url_entry: blob_url_entry(scope, xhr),
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
    rewrite_content_type_charset: bool,
}

pub(crate) enum ConvertedXhrSendBody<'s> {
    Native(v8::Local<'s, v8::Value>),
    Text(String),
}

impl<'s> ConvertedXhrSendBody<'s> {
    pub(crate) fn prepare(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        method: &str,
    ) -> Result<PreparedXhrSendBody, crate::webidl::WebIdlError> {
        // Conversion precedes state checks, but GET/HEAD never extract body bytes.
        if matches!(method, "GET" | "HEAD") {
            return Ok(PreparedXhrSendBody::empty());
        }
        match self {
            Self::Native(value) => prepare_xhr_send_body(scope, value),
            Self::Text(text) => Ok(PreparedXhrSendBody::utf8_text(
                text.into_bytes(),
                TEXT_CONTENT_TYPE,
            )),
        }
    }
}

impl PreparedXhrSendBody {
    pub(crate) fn empty() -> Self {
        Self {
            body: None,
            default_content_type: None,
            rewrite_content_type_charset: false,
        }
    }

    fn new(body: Vec<u8>, default_content_type: Option<String>) -> Self {
        Self {
            body: Some(body),
            default_content_type,
            rewrite_content_type_charset: false,
        }
    }

    fn utf8_text(body: Vec<u8>, content_type: &str) -> Self {
        Self {
            body: Some(body),
            default_content_type: Some(content_type.to_owned()),
            rewrite_content_type_charset: true,
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
        if let Some(body) = prepare_xhr_document_body(scope, object) {
            return Ok(body);
        }
        if let Some((body, content_type)) =
            crate::context_bootstrap::form_data_request_body(scope, object)
        {
            return Ok(PreparedXhrSendBody::new(body, Some(content_type)));
        }
        if let Some(body) = crate::context_bootstrap::url_search_params_request_body(scope, object)
        {
            return Ok(PreparedXhrSendBody::utf8_text(
                body.into_bytes(),
                URL_SEARCH_PARAMS_CONTENT_TYPE,
            ));
        }
        if let Some(bytes) = blob::blob_bytes_from_object(scope, object) {
            let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
            let default_content_type = (!mime_type.is_empty()).then_some(mime_type);
            return Ok(PreparedXhrSendBody::new(bytes, default_content_type));
        }
    }

    validate_xhr_buffer_source(value)?;

    if let Some(bytes) = blob::buffer_source_bytes_from_value(scope, value) {
        return Ok(PreparedXhrSendBody::new(bytes, None));
    }

    let body = crate::webidl::convert::<crate::webidl::UsvString>(
        scope,
        value,
        crate::webidl::Context::argument("XMLHttpRequest.send", 1),
    )?;
    Ok(PreparedXhrSendBody::utf8_text(
        body.0.into_bytes(),
        TEXT_CONTENT_TYPE,
    ))
}

fn prepare_xhr_document_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<PreparedXhrSendBody> {
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)
            .ok()?;
    // SAFETY: the node bridge supplies the live callback's context host. Neither
    // serialization nor document classification calls author JavaScript.
    let runtime = unsafe { &*runtime_ptr };
    let dom_host = runtime.dom_host();
    let document = dom_host.node(handle)?.as_document()?;
    let (body, content_type) = if document.is_html_document() {
        let scripting_enabled = |node| runtime.node_document_scripting_enabled(node);
        (
            dom_host.get_html(handle, &scripting_enabled, false, &[])?,
            "text/html;charset=UTF-8",
        )
    } else {
        (
            crate::xml_serializer::serialize_native_handle(dom_host, handle),
            "application/xml;charset=UTF-8",
        )
    };
    Some(PreparedXhrSendBody::utf8_text(
        body.into_bytes(),
        content_type,
    ))
}

pub(crate) fn convert_xhr_send_body_from_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<ConvertedXhrSendBody<'s>, crate::webidl::WebIdlError> {
    let parsed = crate::webidl::try_parse_args::<XhrSendArgs<'s>>(scope, args)?;
    let body = parsed.body.unwrap_or_else(|| v8::null(scope).into());
    validate_xhr_buffer_source(body)?;
    if body.is_null_or_undefined() || body.is_array_buffer() || body.is_array_buffer_view() {
        return Ok(ConvertedXhrSendBody::Native(body));
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(body)
        && (web_api_interfaces::Document::is_instance(scope, object)
            || web_api_interfaces::Blob::is_instance(scope, object)
            || web_api_interfaces::FormData::is_instance(scope, object)
            || web_api_interfaces::URLSearchParams::is_instance(scope, object))
    {
        return Ok(ConvertedXhrSendBody::Native(body));
    }
    let text = crate::webidl::convert::<crate::webidl::UsvString>(
        scope,
        body,
        crate::webidl::Context::argument("XMLHttpRequest.send", 1),
    )?;
    Ok(ConvertedXhrSendBody::Text(text.0))
}

fn validate_xhr_buffer_source(
    value: v8::Local<'_, v8::Value>,
) -> Result<(), crate::webidl::WebIdlError> {
    if value.is_shared_array_buffer()
        || blob::buffer_source_has_shared_or_resizable_backing_store(value)
    {
        return Err(crate::webidl::WebIdlError::custom_message(
            "XMLHttpRequest.send does not accept shared or resizable BufferSource backing stores",
        ));
    }
    Ok(())
}

fn xhr_request_headers(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared_body: &PreparedXhrSendBody,
) -> (moli_fetch::RequestHeaders, Vec<(String, String)>) {
    let author_headers = xhr_author_request_headers(scope, xhr, prepared_body);
    let merged = merge_byte_string_request_headers(host.extra_http_headers(), &author_headers);
    (merged, author_headers)
}

pub(crate) fn xhr_author_request_headers(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    prepared_body: &PreparedXhrSendBody,
) -> Vec<(String, String)> {
    let headers_json = xhr_state_string_property(scope, xhr, XHR_REQUEST_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());
    let request_headers: Vec<[String; 2]> = serde_json::from_str(&headers_json).unwrap_or_default();
    let mut author_headers: Vec<(String, String)> = request_headers
        .into_iter()
        .map(|[name, value]| (name, value))
        .collect();
    if let Some((_, value)) = author_headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case(CONTENT_TYPE_HEADER))
    {
        if prepared_body.rewrite_content_type_charset
            && let Some(rewritten) = xhr_content_type_with_utf8_charset(value)
        {
            *value = rewritten;
        }
    } else if let Some(default_content_type) = &prepared_body.default_content_type {
        author_headers.push((CONTENT_TYPE_HEADER.to_owned(), default_content_type.clone()));
    }
    author_headers
}

fn xhr_content_type_with_utf8_charset(original: &str) -> Option<String> {
    let mut mime = moli_content_type::parse_mime_type(original)?;
    let charset = mime.parameter_mut("charset")?;
    if charset.eq_ignore_ascii_case("UTF-8") {
        return None;
    }
    *charset = "UTF-8".to_owned();
    Some(mime.to_string())
}

#[cfg(test)]
mod tests {
    use super::xhr_content_type_with_utf8_charset;

    #[test]
    fn xhr_charset_rewriting_uses_mime_parameter_parsing_and_serialization() {
        for (input, expected) in [
            ("", None),
            ("text; charset=ascii", None),
            ("text/plain", None),
            ("text/plain;  hi=bye", None),
            ("text/plain; charset =ascii", None),
            ("text/plain;charset=utf-8;charset=ascii", None),
            (r#"Text/Plain; CHARSET="uTf-8"; KEEP=Value"#, None),
            (r#"text/plain;charset="u\t\f-8""#, None),
            (r#"text/plain;boundary="; charset=ascii""#, None),
            ("text/plain;charset=utf-8 ;x=x", None),
            (
                "text/plain;charset= utf-8",
                Some("text/plain;charset=UTF-8"),
            ),
            (
                "text/plain;charset=;charset=ascii",
                Some("text/plain;charset=UTF-8"),
            ),
            (
                r#"text/plain;charset="";charset=utf-8"#,
                Some("text/plain;charset=UTF-8"),
            ),
            (
                "text/plain;charset='utf-8'",
                Some("text/plain;charset=UTF-8"),
            ),
            (
                r#"text/plain;charset="ASCII"#,
                Some("text/plain;charset=UTF-8"),
            ),
            (
                "text/x-pink-unicorn; charset=windows-1252; charset=bogus; notrelated; charset=ascii",
                Some("text/x-pink-unicorn;charset=UTF-8"),
            ),
            (
                "YO/yo;charset=x;yo=YO; X=y",
                Some("yo/yo;charset=UTF-8;yo=YO;x=y"),
            ),
        ] {
            assert_eq!(
                xhr_content_type_with_utf8_charset(input).as_deref(),
                expected,
                "{input}"
            );
        }
    }
}

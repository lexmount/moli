use super::*;
use crate::web_api_interfaces;
use crate::webidl;

pub(in crate::network_host) const URL_SEARCH_PARAMS_CONTENT_TYPE: &str =
    "application/x-www-form-urlencoded;charset=UTF-8";
pub(in crate::network_host) const TEXT_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";

#[derive(Debug, Clone)]
pub(in crate::network_host) struct PreparedBodyInit {
    pub(in crate::network_host) bytes: Vec<u8>,
    pub(in crate::network_host) content_type: Option<String>,
}

impl PreparedBodyInit {
    fn new(bytes: Vec<u8>, content_type: Option<String>) -> Self {
        Self {
            bytes,
            content_type,
        }
    }
}

pub(in crate::network_host) fn body_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<Option<PreparedBodyInit>, webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        if let Some((body, content_type)) =
            crate::context_bootstrap::form_data_request_body(scope, object)
        {
            return Ok(Some(PreparedBodyInit::new(body, Some(content_type))));
        }
        if let Some(body) = crate::context_bootstrap::url_search_params_request_body(scope, object)
        {
            return Ok(Some(PreparedBodyInit::new(
                body.into_bytes(),
                Some(URL_SEARCH_PARAMS_CONTENT_TYPE.to_owned()),
            )));
        }
        if let Some(bytes) = blob::blob_bytes_from_object(scope, object) {
            let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
            let content_type = (!mime_type.is_empty()).then_some(mime_type);
            return Ok(Some(PreparedBodyInit::new(bytes, content_type)));
        }
        if web_api_interfaces::ReadableStream::is_instance(scope, object) {
            return Ok(Some(PreparedBodyInit::new(Vec::new(), None)));
        }
    }
    if blob::buffer_source_has_shared_or_resizable_backing_store(value) {
        return Err(webidl::WebIdlError::custom_message(
            "BodyInit does not accept shared or resizable BufferSource backing stores",
        ));
    }
    if let Some(bytes) = blob::buffer_source_bytes_from_value(scope, value) {
        return Ok(Some(PreparedBodyInit::new(bytes, None)));
    }
    let value = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
    Ok(Some(PreparedBodyInit::new(
        value.into_bytes(),
        Some(TEXT_CONTENT_TYPE.to_owned()),
    )))
}

pub(crate) fn append_default_body_content_type(
    headers: &mut Vec<(String, String)>,
    content_type: Option<&str>,
) {
    let Some(content_type) = content_type.filter(|value| !value.is_empty()) else {
        return;
    };
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        return;
    }
    headers.push(("Content-Type".to_owned(), content_type.to_owned()));
}

pub(crate) fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
}

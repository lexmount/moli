use super::*;
use crate::web_api_interfaces;
use crate::webidl;

pub(in crate::network_host) const URL_SEARCH_PARAMS_CONTENT_TYPE: &str =
    "application/x-www-form-urlencoded;charset=UTF-8";
pub(in crate::network_host) const TEXT_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";

pub(crate) fn body_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_branded_request_object(scope, object) {
        request_slot_object(scope, object, REQUEST_BODY_SLOT)
    } else if is_branded_response_object(scope, object) {
        response_slot_object(scope, object, RESPONSE_BODY_SLOT)
    } else {
        None
    }
}

pub(in crate::network_host) fn readable_body_stream_unusable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    crate::context_bootstrap::readable_stream_locked(scope, stream)
        || crate::context_bootstrap::readable_stream_disturbed(scope, stream)
}

pub(in crate::network_host) fn body_is_used<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    body_stream_object(scope, object)
        .is_some_and(|stream| crate::context_bootstrap::readable_stream_disturbed(scope, stream))
}

pub(in crate::network_host) fn body_is_unusable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    body_stream_object(scope, object)
        .is_some_and(|stream| readable_body_stream_unusable(scope, stream))
}

#[derive(Debug, Clone)]
pub(in crate::network_host) struct PreparedBodyInit {
    pub(in crate::network_host) source: PreparedBodySource,
    pub(in crate::network_host) content_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::network_host) enum PreparedBodySource {
    Bytes(bytes::Bytes),
    Blob(v8::Global<v8::Object>),
    Stream(v8::Global<v8::Object>),
}

impl PreparedBodySource {
    pub(in crate::network_host) fn into_body_stream<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        match self {
            Self::Bytes(bytes) => {
                let buffer = set_network_body_owned_bytes(scope, owner, bytes)?;
                new_readable_stream_from_array_buffer(scope, buffer, buffer.byte_length())
            }
            Self::Blob(blob) => {
                let blob = v8::Local::new(scope, blob);
                let stream = blob::stream_blob(scope, blob)?;
                body_source::set_network_body_blob_source(scope, owner, blob);
                Some(stream)
            }
            Self::Stream(stream) => Some(v8::Local::new(scope, stream)),
        }
    }
}

impl PreparedBodyInit {
    fn new(bytes: Vec<u8>, content_type: Option<String>) -> Self {
        Self {
            source: PreparedBodySource::Bytes(bytes.into()),
            content_type,
        }
    }
}

/// Materialized native body variants for consumers that need immediate bytes.
/// Request/Response Blob bodies keep their native source until consumption.
#[derive(Debug, Clone)]
pub(in crate::network_host) struct PreparedBodyBytes {
    pub(in crate::network_host) bytes: bytes::Bytes,
    pub(in crate::network_host) content_type: Option<String>,
}

impl PreparedBodyBytes {
    fn new(bytes: Vec<u8>, content_type: Option<String>) -> Self {
        Self {
            bytes: bytes.into(),
            content_type,
        }
    }
}

pub(in crate::network_host) fn object_body_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<PreparedBodyBytes>, webidl::WebIdlError> {
    if let Some((body, content_type)) =
        crate::context_bootstrap::form_data_request_body(scope, object)?
    {
        return Ok(Some(PreparedBodyBytes::new(body, Some(content_type))));
    }
    if let Some(body) = crate::context_bootstrap::url_search_params_request_body(scope, object) {
        return Ok(Some(PreparedBodyBytes::new(
            body.into_bytes(),
            Some(URL_SEARCH_PARAMS_CONTENT_TYPE.to_owned()),
        )));
    }
    if let Some(data) = blob::blob_data_from_object(scope, object) {
        // A recognized Blob must never fall through to USVString on I/O failure.
        let bytes = data
            .read_blocking()
            .map_err(|_| webidl::WebIdlError::custom_message("Failed to read Blob body"))?;
        let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
        let content_type = (!mime_type.is_empty()).then_some(mime_type);
        return Ok(Some(PreparedBodyBytes::new(bytes, content_type)));
    }
    Ok(None)
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
        if web_api_interfaces::Blob::is_instance(scope, object) {
            let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
            let content_type = (!mime_type.is_empty()).then_some(mime_type);
            return Ok(Some(PreparedBodyInit {
                source: PreparedBodySource::Blob(v8::Global::new(scope, object)),
                content_type,
            }));
        }
        if let Some(body) = object_body_init(scope, object)? {
            return Ok(Some(PreparedBodyInit {
                source: PreparedBodySource::Bytes(body.bytes),
                content_type: body.content_type,
            }));
        }
        if web_api_interfaces::ReadableStream::is_instance(scope, object) {
            if readable_body_stream_unusable(scope, object) {
                return Err(webidl::WebIdlError::custom_message(
                    "BodyInit ReadableStream is locked or disturbed",
                ));
            }
            return Ok(Some(PreparedBodyInit {
                source: PreparedBodySource::Stream(v8::Global::new(scope, object)),
                content_type: None,
            }));
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

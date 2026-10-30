use super::storage::{form_data_entries, form_data_is_object};
use super::*;
use crate::blob;
use crate::webidl::WebIdlError;
use moli_multipart::{MultipartFormDataPart, MultipartFormDataPartValue};

const FORM_DATA_MULTIPART_BOUNDARY_PREFIX: &str = "----MoliFormDataBoundary";

pub(crate) fn form_data_request_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<(Vec<u8>, String)>, WebIdlError> {
    if !form_data_is_object(scope, object) {
        return Ok(None);
    }
    let entries = form_data_entries(scope, object);
    Ok(Some(form_data_entries_multipart_body(
        scope,
        &entries,
        FORM_DATA_MULTIPART_BOUNDARY_PREFIX,
        encoding_rs::UTF_8,
    )?))
}

pub(crate) fn form_data_entries_multipart_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
    boundary_prefix: &str,
    encoding: &'static encoding_rs::Encoding,
) -> Result<(Vec<u8>, String), WebIdlError> {
    let parts = entries
        .iter()
        .map(|(name, value)| {
            let value = v8::Local::new(scope, value);
            Ok(MultipartFormDataPart {
                name: name.clone(),
                value: form_data_request_value(scope, value)?,
            })
        })
        .collect::<Result<Vec<_>, WebIdlError>>()?;
    Ok(moli_multipart::serialize_multipart_form_data_with_encoding(
        &parts,
        boundary_prefix,
        encoding,
    ))
}

fn form_data_request_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<MultipartFormDataPartValue, WebIdlError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(data) = blob::blob_data_from_object(scope, object)
    {
        let bytes = data
            .read_blocking()
            .map_err(|_| WebIdlError::custom_message("Failed to read Blob body"))?;
        return Ok(MultipartFormDataPartValue::Blob {
            filename: file_api::file_name_from_object(scope, object)
                .unwrap_or_else(|| "blob".to_owned()),
            content_type: blob::blob_mime_type_from_object(scope, object).unwrap_or_default(),
            body: bytes,
        });
    }
    let text = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    Ok(MultipartFormDataPartValue::Text(text))
}

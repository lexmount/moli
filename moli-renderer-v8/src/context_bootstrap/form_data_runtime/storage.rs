use super::*;
use crate::util::{get_private_value, serialize_v8_array, set_private_value};
use crate::web_api_interfaces;
use crate::{dom::native::SelectedFile, webidl};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FormData)]
struct FormDataObjectDeclaration<'scope> {
    #[webapi(slot = FORM_DATA_ENTRIES_SLOT)]
    entries: v8::Local<'scope, v8::Array>,
}

pub(in crate::context_bootstrap) fn form_data_is_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, object, "FormData")
}

pub(in crate::context_bootstrap) fn form_data_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Vec<(String, v8::Global<v8::Value>)> {
    let Some(array) = form_data_entries_private_array(scope, object) else {
        return Vec::new();
    };
    let mut entries = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let Some(pair) = array
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            continue;
        };
        let Some(key) = pair
            .get_index(scope, 0)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        let value = pair
            .get_index(scope, 1)
            .unwrap_or_else(|| v8::undefined(scope).into());
        entries.push((key, v8::Global::new(scope, value)));
    }
    entries
}

pub(super) fn mutate_form_data_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mutate: impl FnOnce(&mut Vec<(String, v8::Global<v8::Value>)>),
) {
    let mut entries = form_data_entries(scope, object);
    mutate(&mut entries);
    set_form_data_entries(scope, object, &entries);
}

pub(super) fn push_form_data_entry(
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    name: &str,
    value: v8::Global<v8::Value>,
) {
    entries.push((name.to_owned(), value));
}

pub(super) fn set_form_data_entries(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    entries: &[(String, v8::Global<v8::Value>)],
) {
    let array = form_data_entries_array(scope, entries);
    set_private_value(scope, object, FORM_DATA_ENTRIES_SLOT, array.into());
}

pub(super) fn initialize_form_data_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    entries: &[(String, v8::Global<v8::Value>)],
) {
    let entries = form_data_entries_array(scope, entries);
    FormDataObjectDeclaration::new(entries)
        .initialize(scope, object)
        .expect("FormData declaration should initialize entries");
}

fn form_data_entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
) -> v8::Local<'s, v8::Array> {
    let pairs: Vec<(&str, v8::Local<'s, v8::Value>)> = entries
        .iter()
        .map(|(key, value)| (key.as_str(), v8::Local::new(scope, value)))
        .collect();
    serialize_v8_array(scope, pairs.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn form_data_entries_private_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, FORM_DATA_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn normalize_form_data_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    filename: Option<v8::Local<'s, v8::Value>>,
    value_context: webidl::Context,
    filename_context: webidl::Context,
) -> std::result::Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(bytes) = blob::blob_bytes_from_object(scope, object)
    {
        let filename = filename
            .map(|value| webidl::convert::<webidl::UsvString>(scope, value, filename_context))
            .transpose()?
            .map(|value| value.0);
        if filename.is_none() && file_api::file_name_from_object(scope, object).is_some() {
            return Ok(value);
        }
        let selected = SelectedFile {
            bytes,
            mime_type: blob::blob_mime_type_from_object(scope, object).unwrap_or_default(),
            name: filename.unwrap_or_else(|| "blob".to_owned()),
            last_modified: file_api::selected_file_from_object(scope, object)
                .map(|file| file.last_modified)
                .unwrap_or_else(unix_epoch_millis),
        };
        if let Some(file) = file_api::build_file_object(scope, &selected) {
            return Ok(file.into());
        }
        return Ok(value);
    }
    if filename.is_some() {
        return Err(webidl::WebIdlError::custom_message(
            "FormData: Argument 2 can not be converted to Blob",
        ));
    }
    let value = webidl::convert::<webidl::UsvString>(scope, value, value_context)?.0;
    Ok(v8_string(scope, &value)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into()))
}

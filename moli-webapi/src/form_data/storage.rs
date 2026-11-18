use super::*;
use crate::{blob, file, interfaces as web_api_interfaces};
use moli_time::unix_epoch_millis;
use moli_v8_util::{get_private_value, set_private_value};
use moli_webapi_declare::{WebApiObject, serialize_v8_array};
use moli_webidl as webidl;

const FORM_DATA_ENTRIES_SLOT: &str = "__moliFormDataEntries";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FormData)]
struct FormDataObjectDeclaration<'scope> {
    #[webapi(slot = FORM_DATA_ENTRIES_SLOT)]
    entries: v8::Local<'scope, v8::Array>,
}

pub fn form_data_is_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::FormData::is_instance(scope, object)
}

pub fn form_data_entries<'s>(
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
        let context = object
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let value = runtime(scope)
            .and_then(|runtime| (runtime.value_in_realm)(scope, value, context))
            .unwrap_or(value);
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

pub fn push_form_data_entry(
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    name: &str,
    value: v8::Global<v8::Value>,
) {
    entries.push((name.to_owned(), value));
}

pub fn set_form_data_entries(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    entries: &[(String, v8::Global<v8::Value>)],
) {
    let object = object_owner(scope, v8::Local::new(scope, object));
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
        .map(|(key, value)| {
            let value = v8::Local::new(scope, value);
            let value = v8::Local::<v8::Object>::try_from(value)
                .map(|object| object_owner(scope, object).into())
                .unwrap_or(value);
            (key.as_str(), value)
        })
        .collect();
    serialize_v8_array(scope, pairs.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn form_data_entries_private_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let object = object_owner(scope, object);
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
        && let Some(data) = blob::blob_data_from_object(scope, object)
    {
        let filename = filename
            .map(|value| webidl::convert::<webidl::UsvString>(scope, value, filename_context))
            .transpose()?
            .map(|value| value.0);
        if filename.is_none() && file::file_name_from_object(scope, object).is_some() {
            return Ok(value);
        }
        let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
        let name = filename.unwrap_or_else(|| "blob".to_owned());
        let last_modified = file::file_last_modified_from_object(scope, object)
            .filter(|modified| modified.is_finite())
            .unwrap_or_else(unix_epoch_millis);
        if let Some((file, _)) =
            file::build_file_object_unpublished(scope, data, mime_type, &name, last_modified)
        {
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

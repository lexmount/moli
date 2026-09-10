use super::url_form::callback_value_string;
use super::*;

mod callbacks;
mod iterators;
mod multipart_parse;
mod request_body;
mod serialize;
mod storage;
mod template;

pub(crate) use callbacks::construct_form_data_entries_for_form;
pub(crate) use multipart_parse::form_data_object_from_multipart_bytes;
pub(crate) use request_body::{
    form_data_entries_multipart_body_with_prefix, form_data_request_body,
};
pub(crate) use serialize::form_data_entries_to_string_pairs;
pub(in crate::context_bootstrap) use storage::{form_data_entries, form_data_is_object};
pub(super) use template::build_form_data_constructor_template;

pub(crate) fn form_data_object_from_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "FormData").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let form_data = constructor.new_instance(scope, &[])?;
    storage::set_form_data_entries(scope, form_data, entries);
    Some(form_data.into())
}

pub(crate) fn snapshot_form_data_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return value;
    };
    if !storage::form_data_is_object(scope, object) {
        return value;
    }
    let entries = storage::form_data_entries(scope, object);
    form_data_object_from_entries(scope, &entries).unwrap_or(value)
}

pub(crate) fn form_data_object_from_urlencoded_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "FormData").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let form_data = constructor.new_instance(scope, &[])?;
    let mut entries: Vec<(String, v8::Global<v8::Value>)> = Vec::new();
    for (name, value) in url::form_urlencoded::parse(bytes) {
        let value = v8_string(scope, value.as_ref())?;
        let value: v8::Local<'s, v8::Value> = value.into();
        entries.push((name.into_owned(), v8::Global::new(scope, value)));
    }
    storage::set_form_data_entries(scope, form_data, &entries);
    Some(form_data.into())
}

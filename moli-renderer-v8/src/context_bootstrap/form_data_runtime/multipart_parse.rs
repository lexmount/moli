use super::*;
use crate::dom::native::SelectedFile;

pub(crate) fn form_data_object_from_multipart_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
    boundary: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let parsed = moli_multipart::parse_multipart_form_data(bytes, boundary)?;
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "FormData").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let form_data = constructor.new_instance(scope, &[])?;
    let mut entries: Vec<(String, v8::Global<v8::Value>)> = Vec::with_capacity(parsed.len());
    for entry in parsed {
        let value: v8::Local<'s, v8::Value> = if let Some(filename) = entry.filename {
            let file = SelectedFile {
                relative_path: String::new(),
                bytes: entry.body,
                mime_type: entry.content_type,
                name: filename,
                last_modified: unix_epoch_millis(),
            };
            file_api::build_file_object(scope, &file)?.into()
        } else {
            let text = String::from_utf8_lossy(&entry.body);
            v8_string(scope, text.as_ref())?.into()
        };
        entries.push((entry.name, v8::Global::new(scope, value)));
    }
    storage::set_form_data_entries(scope, form_data, &entries);
    Some(form_data.into())
}

use crate::blob;
use crate::dom::native::SelectedFile;
use crate::web_api_interfaces;
use moli_time::unix_epoch_millis;
pub(crate) use moli_webapi::file::{
    bind_file_world_wrapper, build_file_object_unpublished, file_clone_metadata,
    file_constructor_callback, file_last_modified_from_object, file_name_from_object,
    file_relative_path_from_object, install_file_template_bindings, set_file_relative_path,
};

pub(crate) fn build_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: &SelectedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    let (object, _) = build_file_object_unpublished(
        scope,
        file.data.clone(),
        file.mime_type.clone(),
        &file.name,
        file.last_modified,
    )?;
    set_file_relative_path(scope, object, &file.relative_path);
    Some(object)
}

pub(crate) fn selected_file_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<SelectedFile> {
    if !web_api_interfaces::File::is_instance(scope, object) {
        return None;
    }
    let data = blob::blob_data_from_object(scope, object)?;
    let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    let name = file_name_from_object(scope, object)?;
    let last_modified = file_last_modified_from_object(scope, object)
        .filter(|value| value.is_finite())
        .unwrap_or_else(unix_epoch_millis);
    Some(SelectedFile {
        relative_path: file_relative_path_from_object(scope, object).unwrap_or_default(),
        data,
        mime_type,
        name,
        last_modified,
    })
}

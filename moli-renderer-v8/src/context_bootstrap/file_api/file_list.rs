use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_LIST_LENGTH_SLOT: &str = "__lmFileListLength";
const FILE_LIST_BRAND_SLOT: &str = "__lmFileListBrand";
const FILE_LIST_FILES_SLOT: &str = "__lmFileListFiles";

#[derive(WebApiObject)]
#[webapi(interface = "FileList", require_prototype)]
struct FileListObjectDeclaration {
    #[webapi(slot = FILE_LIST_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = FILE_LIST_LENGTH_SLOT)]
    length: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileList")]
struct FileListPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = file_list_length_getter_callback, enumerable)]
    length: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileList.item")]
struct FileListItemArgs {
    #[webidl(required)]
    index: u32,
}

pub(super) fn install_file_list_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "FileList" {
        return;
    }
    FileListPrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(crate) fn build_file_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    files: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Object>> {
    let length = u32::try_from(files.len()).ok()?;
    let contents = build_file_list_contents_array(scope, files)?;
    let object = FileListObjectDeclaration::new(f64::from(length))
        .bind(scope)
        .ok()?;
    for (index, file) in files.iter().enumerate() {
        let index = u32::try_from(index).ok()?;
        if object.set_index(scope, index, (*file).into()) != Some(true) {
            return None;
        }
    }
    set_private_value(scope, object, FILE_LIST_FILES_SLOT, contents.into());
    Some(object)
}

pub(crate) fn sync_file_list_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    files: &[v8::Local<'s, v8::Object>],
) {
    let Ok(length) = u32::try_from(files.len()) else {
        return;
    };
    let Some(contents) = build_file_list_contents_array(scope, files) else {
        return;
    };
    let previous_length = file_list_length_from_object(scope, object)
        .filter(|value| {
            value.is_finite()
                && *value >= 0.0
                && value.fract() == 0.0
                && *value <= f64::from(u32::MAX)
        })
        .map(|value| value as u32)
        .unwrap_or(0);
    for index in 0..previous_length {
        let _ = object.delete_index(scope, index);
    }
    for (index, file) in files.iter().enumerate() {
        let _ = object.set_index(scope, index as u32, (*file).into());
    }
    set_private_value(scope, object, FILE_LIST_FILES_SLOT, contents.into());
    let length = v8::Number::new(scope, f64::from(length));
    set_private_value(scope, object, FILE_LIST_LENGTH_SLOT, length.into());
}

fn build_file_list_contents_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    files: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Array>> {
    let contents = v8::Array::new(scope, 0);
    for (index, file) in files.iter().enumerate() {
        let index = u32::try_from(index).ok()?;
        if contents.set_index(scope, index, (*file).into()) != Some(true) {
            return None;
        }
    }
    Some(contents)
}

pub(crate) fn is_file_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, object, FILE_LIST_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn file_list_files_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    if !is_file_list_object(scope, object) {
        return None;
    }
    let length = file_list_length_from_object(scope, object)?;
    if !length.is_finite() || length < 0.0 || length.fract() != 0.0 || length > f64::from(u32::MAX)
    {
        return None;
    }
    let contents = get_private_value(scope, object, FILE_LIST_FILES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    let length = length as u32;
    let mut files = Vec::new();
    files.try_reserve_exact(length as usize).ok()?;
    for index in 0..length {
        let file = contents
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
        files.push(file);
    }
    Some(files)
}

pub(in crate::context_bootstrap) fn file_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_file_list_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileListItemArgs>(scope, &args) else {
        return;
    };
    let Some(value) = get_private_value(scope, args.this(), FILE_LIST_FILES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .and_then(|files| files.get_index(scope, parsed.index))
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    if value.is_undefined() {
        rv.set(v8::null(scope).into());
        return;
    }
    rv.set(value);
}

fn file_list_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_file_list_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let length = file_list_length_from_object(scope, args.this())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, length).into());
}

fn file_list_length_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, object, FILE_LIST_LENGTH_SLOT)
        .and_then(|value| value.number_value(scope))
}

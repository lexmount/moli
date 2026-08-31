use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_LIST_LENGTH_SLOT: &str = "__lmFileListLength";
const FILE_LIST_BRAND_SLOT: &str = "__lmFileListBrand";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FileList, require_prototype)]
struct FileListObjectDeclaration {
    #[webapi(slot = FILE_LIST_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = FILE_LIST_LENGTH_SLOT)]
    length: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FileList)]
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
    let object = FileListObjectDeclaration::new(files.len() as f64)
        .bind(scope)
        .ok()?;
    for (index, file) in files.iter().enumerate() {
        let _ = object.set_index(scope, index as u32, (*file).into());
    }
    Some(object)
}

pub(crate) fn sync_file_list_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    files: &[v8::Local<'s, v8::Object>],
) {
    let previous_length = file_list_length_from_object(scope, object)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);
    for index in 0..previous_length {
        let _ = object.delete_index(scope, index);
    }
    for (index, file) in files.iter().enumerate() {
        let _ = object.set_index(scope, index as u32, (*file).into());
    }
    let length = v8::Number::new(scope, files.len() as f64);
    set_private_value(scope, object, FILE_LIST_LENGTH_SLOT, length.into());
}

pub(in crate::context_bootstrap) fn file_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_list_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileListItemArgs>(scope, &args) else {
        return;
    };
    let Some(value) = args.this().get_index(scope, parsed.index) else {
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
    if !file_list_receiver_branded(scope, args.this()) {
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

fn file_list_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, FILE_LIST_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

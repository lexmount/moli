//! File metadata and constructors over the shared Blob binding.

use crate::{blob, interfaces as web_api_interfaces};
use moli_file_api::file::normalize_file_last_modified;
use moli_time::unix_epoch_millis;
use moli_v8_util::{
    get_private_value, global_constructor_prototype, set_private_value, throw_type_error, v8_string,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;

const FILE_NAME_SLOT: &str = "__lmFileName";
const FILE_RELATIVE_PATH_SLOT: &str = "__lmFileRelativePath";
const FILE_LAST_MODIFIED_SLOT: &str = "__lmFileLastModified";

pub fn bind_file_world_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let name = file_name_from_object(scope, source)?;
    let modified =
        get_private_value(scope, source, FILE_LAST_MODIFIED_SLOT)?.number_value(scope)?;
    initialize_file_metadata(scope, wrapper, &name, modified);
    let relative_path = file_relative_path_from_object(scope, source).unwrap_or_default();
    set_file_relative_path(scope, wrapper, &relative_path);
    let prototype = global_constructor_prototype(scope, "File")?;
    wrapper
        .set_prototype(scope, prototype.into())?
        .then_some(())?;
    Some(())
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::File)]
struct FileMetadataDeclaration {
    #[webapi(slot = FILE_NAME_SLOT)]
    name: String,
    #[webapi(slot = FILE_LAST_MODIFIED_SLOT)]
    last_modified: f64,
    #[webapi(slot = FILE_RELATIVE_PATH_SLOT)]
    relative_path: String,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::File)]
struct FilePrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = file_name_attribute_getter_callback, enumerable)]
    name: (),
    #[webapi(
        accessor_property,
        getter = file_last_modified_attribute_getter_callback,
        enumerable
    )]
    last_modified: (),
    #[webapi(accessor_property, getter = file_relative_path_attribute_getter_callback, enumerable)]
    webkit_relative_path: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "File")]
struct FileConstructorArgs<'s> {
    #[webidl(
        required,
        name = "fileBits",
        with = blob::parts::required_blob_parts_argument,
        missing_message = "Failed to construct 'File': fileBits argument is required."
    )]
    file_bits: blob::parts::BlobParts<'s>,
    #[webidl(
        required,
        name = "fileName",
        converter = "usv_string",
        missing_message = "Failed to construct 'File': fileName argument is required."
    )]
    name: String,
    #[webidl(dictionary)]
    options: FilePropertyBag,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FilePropertyBag")]
struct FilePropertyBag {
    #[webidl(inherit)]
    base: blob::parts::BlobPropertyBag,
    last_modified: Option<i64>,
}

pub fn install_file_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "File" {
        return;
    }
    FilePrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn file_name_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let name = file_name_from_object(scope, args.this()).unwrap_or_default();
    if let Some(value) = v8_string(scope, &name) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub fn file_name_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_value(scope, object, FILE_NAME_SLOT)
}

fn file_last_modified_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let last_modified =
        file_last_modified_from_object(scope, args.this()).unwrap_or_else(unix_epoch_millis);
    rv.set(v8::Number::new(scope, last_modified).into());
}

pub fn file_last_modified_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    get_private_value(scope, object, FILE_LAST_MODIFIED_SLOT)?.number_value(scope)
}

fn private_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn initialize_file_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    last_modified: f64,
) {
    let last_modified = normalize_file_last_modified(last_modified, unix_epoch_millis());
    FileMetadataDeclaration::new(name.to_owned(), last_modified, String::new())
        .initialize(scope, object)
        .expect("File metadata declaration should initialize object");
}

pub fn file_object_with_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    last_modified: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = global_constructor_prototype(scope, "File")?;
    let last_modified = normalize_file_last_modified(last_modified, unix_epoch_millis());
    FileMetadataDeclaration::new(name.to_owned(), last_modified, String::new())
        .bind(scope)
        .ok()
}

pub fn file_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'File': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let bytes = match parsed
        .file_bits
        .into_data(scope, parsed.options.base.line_endings())
    {
        Ok(bytes) => bytes,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mime_type = moli_file_api::normalize_blob_mime_type(&parsed.options.base.r#type);
    blob::init_blob_object_from_data(scope, args.this(), bytes, mime_type);

    let last_modified = parsed
        .options
        .last_modified
        .map(|value| value as f64)
        .unwrap_or_else(unix_epoch_millis);
    initialize_file_metadata(scope, args.this(), &parsed.name, last_modified);
    rv.set(args.this().into());
}

fn file_relative_path_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let relative_path = file_relative_path_from_object(scope, args.this()).unwrap_or_default();
    rv.set(
        v8::String::new(scope, &relative_path)
            .expect("File path string allocation")
            .into(),
    );
}

pub fn file_relative_path_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_value(scope, object, FILE_RELATIVE_PATH_SLOT)
}

pub fn set_file_relative_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    path: &str,
) {
    let value = v8::String::new(scope, path).expect("File path string allocation");
    set_private_value(scope, object, FILE_RELATIVE_PATH_SLOT, value.into());
}

pub fn file_clone_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(String, f64, String)> {
    if !web_api_interfaces::File::is_instance(scope, object) {
        return None;
    }
    let name = file_name_from_object(scope, object)?;
    let modified =
        get_private_value(scope, object, FILE_LAST_MODIFIED_SLOT)?.number_value(scope)?;
    Some((
        name,
        modified,
        file_relative_path_from_object(scope, object).unwrap_or_default(),
    ))
}

pub fn build_file_object_unpublished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: moli_file_api::BlobData,
    mime_type: String,
    name: &str,
    last_modified: f64,
) -> Option<(v8::Local<'s, v8::Object>, moli_file_api::BlobId)> {
    let object = file_object_with_metadata(scope, name, last_modified)?;
    let id = blob::try_init_blob_object_from_data(scope, object, bytes, mime_type)?;
    Some((object, id))
}

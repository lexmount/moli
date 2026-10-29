use super::*;
use crate::blob;
use crate::dom::native::SelectedFile;
use crate::util::get_private_value;
use crate::web_api_interfaces;
use crate::webidl;
use moli_file_api::file::normalize_file_last_modified;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_NAME_SLOT: &str = "__lmFileName";
const FILE_LAST_MODIFIED_SLOT: &str = "__lmFileLastModified";

pub(in crate::context_bootstrap) fn bind_file_world_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let name = file_name_from_object(scope, source)?;
    let modified =
        get_private_value(scope, source, FILE_LAST_MODIFIED_SLOT)?.number_value(scope)?;
    initialize_file_metadata(scope, wrapper, &name, modified);
    let prototype = super::super::ensure_intrinsic_interface_prototype(scope, "File").ok()?;
    wrapper.set_prototype(scope, prototype.into())?;
    Some(())
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::File)]
struct FileMetadataDeclaration {
    #[webapi(slot = FILE_NAME_SLOT)]
    name: String,
    #[webapi(slot = FILE_LAST_MODIFIED_SLOT)]
    last_modified: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::File, receiver)]
struct FilePrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = file_name_attribute_getter_callback, enumerable)]
    name: (),
    #[webapi(
        accessor_property,
        getter = file_last_modified_attribute_getter_callback,
        enumerable
    )]
    last_modified: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "File")]
struct FileConstructorArgs<'s> {
    #[webidl(
        required,
        name = "fileBits",
        with = blob::required_blob_parts_argument,
        missing_message = "Failed to construct 'File': fileBits argument is required."
    )]
    file_bits: blob::BlobParts<'s>,
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
    base: blob::BlobPropertyBag,
    last_modified: Option<i64>,
}

pub(super) fn install_file_template_bindings<'s>(
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

pub(in crate::context_bootstrap) fn file_name_from_object<'s>(
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

pub(in crate::context_bootstrap) fn file_last_modified_from_object<'s>(
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

pub(super) fn initialize_file_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    last_modified: f64,
) {
    let last_modified = normalize_file_last_modified(last_modified, unix_epoch_millis());
    FileMetadataDeclaration::new(name.to_owned(), last_modified)
        .initialize(scope, object)
        .expect("File metadata declaration should initialize object");
}

pub(super) fn file_object_with_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    last_modified: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = global_constructor_prototype(scope, "File")?;
    let last_modified = normalize_file_last_modified(last_modified, unix_epoch_millis());
    FileMetadataDeclaration::new(name.to_owned(), last_modified)
        .bind(scope)
        .ok()
}

pub(in crate::context_bootstrap) fn file_constructor_callback<'s>(
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
        .into_bytes(scope, parsed.options.base.line_endings())
    {
        Ok(bytes) => bytes,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mime_type = moli_file_api::normalize_blob_mime_type(&parsed.options.base.r#type);
    blob::init_blob_object(scope, args.this(), bytes, mime_type);

    let last_modified = parsed
        .options
        .last_modified
        .map(|value| value as f64)
        .unwrap_or_else(unix_epoch_millis);
    initialize_file_metadata(scope, args.this(), &parsed.name, last_modified);
    rv.set(args.this().into());
}

pub(crate) fn build_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: &SelectedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    build_file_object_from_shared_parts(
        scope,
        std::sync::Arc::from(file.bytes.as_slice()),
        file.mime_type.clone(),
        &file.name,
        file.last_modified,
    )
}

pub(crate) fn build_file_object_from_shared_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: std::sync::Arc<[u8]>,
    mime_type: String,
    name: &str,
    last_modified: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    build_file_object_unpublished(scope, bytes, mime_type, name, last_modified)
        .map(|(object, _)| object)
}

pub(crate) fn build_file_object_unpublished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: std::sync::Arc<[u8]>,
    mime_type: String,
    name: &str,
    last_modified: f64,
) -> Option<(v8::Local<'s, v8::Object>, moli_file_api::BlobId)> {
    let object = file_object_with_metadata(scope, name, last_modified)?;
    let id = blob::try_init_blob_object_from_shared(scope, object, bytes, mime_type)?;
    Some((object, id))
}

pub(crate) fn selected_file_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<SelectedFile> {
    if !web_api_interfaces::File::is_instance(scope, object) {
        return None;
    }
    let bytes = blob::blob_bytes_from_object(scope, object)?;
    let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    let name = file_name_from_object(scope, object)?;
    let last_modified = get_private_value(scope, object, FILE_LAST_MODIFIED_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
        .unwrap_or_else(unix_epoch_millis);
    Some(SelectedFile {
        bytes,
        mime_type,
        name,
        last_modified,
    })
}

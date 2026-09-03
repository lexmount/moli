use super::*;
use crate::blob;
use crate::dom::native::SelectedFile;
use crate::util::get_private_value;
use crate::webidl;
use moli_file_api::file::normalize_file_last_modified;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_NAME_SLOT: &str = "__lmFileName";
const FILE_LAST_MODIFIED_SLOT: &str = "__lmFileLastModified";

#[derive(WebApiObject)]
#[webapi(interface = "File")]
struct FileMetadataDeclaration {
    #[webapi(slot = FILE_NAME_SLOT)]
    name: String,
    #[webapi(slot = FILE_LAST_MODIFIED_SLOT)]
    last_modified: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "File")]
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
        converter = "raw",
        missing_message = "Failed to construct 'File': fileBits argument is required."
    )]
    file_bits: v8::Local<'s, v8::Value>,
    #[webidl(
        required,
        name = "fileName",
        converter = "usv_string",
        missing_message = "Failed to construct 'File': fileName argument is required."
    )]
    name: String,
    #[webidl(index = 2, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
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
    if !file_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
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
    if !file_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let last_modified = get_private_value(scope, args.this(), FILE_LAST_MODIFIED_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or_else(unix_epoch_millis);
    rv.set(v8::Number::new(scope, last_modified).into());
}

fn file_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, FILE_NAME_SLOT).is_some()
}

fn private_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
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
    let options = parsed
        .options
        .unwrap_or_else(|| v8::undefined(scope).into());
    let Some((bytes, mime_type)) = blob::collect_required_blob_bytes_and_type_with_options_context(
        scope,
        parsed.file_bits,
        options,
        "FilePropertyBag",
        3,
    ) else {
        return;
    };
    blob::init_blob_object(scope, args.this(), bytes, mime_type);

    let Some(last_modified) = file_last_modified_from_options(scope, options) else {
        return;
    };
    let last_modified = last_modified.unwrap_or_else(unix_epoch_millis);
    initialize_file_metadata(scope, args.this(), &parsed.name, last_modified);
    rv.set(args.this().into());
}

fn file_last_modified_from_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options_value: v8::Local<'s, v8::Value>,
) -> Option<Option<f64>> {
    if options_value.is_null_or_undefined() {
        return Some(None);
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(options_value) else {
        return Some(None);
    };
    let value = match webidl::property_result(
        scope,
        options,
        "lastModified",
        webidl::Context::member("FilePropertyBag", "lastModified"),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Some(value) = value else {
        return Some(None);
    };
    if value.is_undefined() {
        return Some(None);
    }
    match webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        value,
        webidl::Context::member("FilePropertyBag", "lastModified"),
    ) {
        Ok(value) => Some(Some(value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(crate) fn build_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: &SelectedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = file_object_with_metadata(scope, &file.name, file.last_modified)?;
    blob::init_blob_object(scope, object, file.bytes.clone(), file.mime_type.clone());
    Some(object)
}

pub(crate) fn selected_file_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<SelectedFile> {
    let bytes = blob::blob_bytes_from_object(scope, object)?;
    let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    let name_value = object.get(scope, v8str(scope, "name").into())?;
    if name_value.is_null_or_undefined() {
        return None;
    }
    let name = name_value.to_string(scope)?.to_rust_string_lossy(scope);
    let last_modified = object
        .get(scope, v8str(scope, "lastModified").into())
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

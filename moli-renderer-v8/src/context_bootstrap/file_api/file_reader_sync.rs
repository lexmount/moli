use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_file_api::{file_reader_binary_string, file_reader_data_url, file_reader_text};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FileReaderSync, enumerable)]
struct FileReaderSyncPrototypeDeclaration {
    #[webapi(method, length = 1, callback = file_reader_sync_read_as_text_callback)]
    read_as_text: (),
    #[webapi(
        method = "readAsDataURL",
        length = 1,
        callback = file_reader_sync_read_as_data_url_callback
    )]
    read_as_data_url: (),
    #[webapi(method, length = 1, callback = file_reader_sync_read_as_array_buffer_callback)]
    read_as_array_buffer: (),
    #[webapi(
        method = "readAsBinaryString",
        length = 1,
        callback = file_reader_sync_read_as_binary_string_callback
    )]
    read_as_binary_string: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReaderSync.read")]
struct FileReaderSyncReadBlobArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        converter = "raw",
        missing_message = "Failed to execute 'read' on 'FileReaderSync': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReaderSync.readAsText")]
struct FileReaderSyncReadAsTextArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        converter = "raw",
        missing_message = "Failed to execute 'readAsText' on 'FileReaderSync': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Value>,
    encoding: Option<String>,
}

pub(in crate::context_bootstrap) fn file_reader_sync_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'FileReaderSync': Please use the 'new' operator.",
        );
        return;
    }
    rv.set(args.this().into());
}

pub(super) fn install_file_reader_sync_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "FileReaderSync" {
        return;
    }
    FileReaderSyncPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn file_reader_sync_read_as_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderSyncReadAsTextArgs<'s>>(scope, &args) else {
        return;
    };
    let Some((bytes, mime_type)) = file_reader_sync_blob_bytes(scope, parsed.blob, "readAsText")
    else {
        return;
    };
    let value = v8_string(
        scope,
        &file_reader_text(&bytes, parsed.encoding.as_deref(), &mime_type),
    )
    .map(v8::Local::<v8::String>::into)
    .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn file_reader_sync_read_as_data_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderSyncReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    let Some((bytes, mime_type)) = file_reader_sync_blob_bytes(scope, parsed.blob, "readAsDataURL")
    else {
        return;
    };
    let value = v8_string(scope, &file_reader_data_url(&bytes, &mime_type))
        .map(v8::Local::<v8::String>::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn file_reader_sync_read_as_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderSyncReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    let Some((bytes, _)) = file_reader_sync_blob_bytes(scope, parsed.blob, "readAsArrayBuffer")
    else {
        return;
    };
    let value = blob::array_buffer_from_bytes(scope, bytes)
        .map(v8::Local::<v8::ArrayBuffer>::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn file_reader_sync_read_as_binary_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderSyncReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    let Some((bytes, _)) = file_reader_sync_blob_bytes(scope, parsed.blob, "readAsBinaryString")
    else {
        return;
    };
    let value = v8_string(scope, &file_reader_binary_string(&bytes))
        .map(v8::Local::<v8::String>::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn file_reader_sync_blob_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Option<(Vec<u8>, String)> {
    let object = value.to_object(scope)?;
    let bytes = match blob::blob_bytes_from_object(scope, object) {
        Some(bytes) => bytes,
        None => {
            throw_type_error(
                scope,
                &format!("FileReaderSync.{operation} requires a Blob or File"),
            );
            return None;
        }
    };
    let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    Some((bytes, mime_type))
}

use crate::{blob, interfaces as web_api_interfaces};
use moli_file_api::{file_reader_binary_string, file_reader_data_url, file_reader_text};
use moli_v8_util::{array_buffer_from_bytes, throw_type_error, v8_string};
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;

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
        interface = web_api_interfaces::Blob,
        missing_message = "Failed to execute 'read' on 'FileReaderSync': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReaderSync.readAsText")]
struct FileReaderSyncReadAsTextArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        interface = web_api_interfaces::Blob,
        missing_message = "Failed to execute 'readAsText' on 'FileReaderSync': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Object>,
    encoding: Option<String>,
}

pub fn install_file_reader_sync_template_bindings<'s>(
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
    let value = array_buffer_from_bytes(scope, bytes)
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
    value: v8::Local<'s, v8::Object>,
    operation: &'static str,
) -> Option<(Vec<u8>, String)> {
    let object =
        moli_webapi_declare::web_api_object_target(scope, value).expect("converted Blob argument");
    let data = match blob::blob_data_from_object(scope, object) {
        Some(data) => data,
        None => {
            throw_type_error(
                scope,
                &format!("FileReaderSync.{operation} requires a Blob or File"),
            );
            return None;
        }
    };
    let bytes = match data.read_blocking() {
        Ok(bytes) => bytes,
        Err(error) => {
            let exception = crate::dom_exception::new_dom_exception_value(
                scope,
                &error.to_string(),
                moli_file_api::file::read_error_name(&error),
            );
            scope.throw_exception(exception);
            return None;
        }
    };
    let mime_type = blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    Some((bytes, mime_type))
}

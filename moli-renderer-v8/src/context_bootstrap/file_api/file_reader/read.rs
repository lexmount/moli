use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_file_api::{file_reader_binary_string, file_reader_data_url, file_reader_text};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReader.read")]
struct FileReaderReadBlobArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        interface = web_api_interfaces::Blob,
        missing_message = "Failed to execute 'read' on 'FileReader': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReader.readAsText")]
struct FileReaderReadAsTextArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        interface = web_api_interfaces::Blob,
        missing_message = "Failed to execute 'readAsText' on 'FileReader': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Object>,
    encoding: Option<String>,
}

pub(in crate::context_bootstrap) fn file_reader_read_as_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderReadAsTextArgs<'s>>(scope, &args) else {
        return;
    };
    if !ensure_file_reader_can_start_read(scope, args.this()) {
        return;
    }
    if !file_reader_start_read(
        scope,
        args.this(),
        parsed.blob,
        FileReaderReadMode::Text {
            encoding_label: parsed.encoding,
        },
    ) {
        throw_type_error(scope, "FileReader.readAsText requires a Blob or File");
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_read_as_data_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if !ensure_file_reader_can_start_read(scope, args.this()) {
        return;
    }
    if !file_reader_start_read(scope, args.this(), parsed.blob, FileReaderReadMode::DataUrl) {
        throw_type_error(scope, "FileReader.readAsDataURL requires a Blob or File");
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_read_as_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if !ensure_file_reader_can_start_read(scope, args.this()) {
        return;
    }
    if !file_reader_start_read(
        scope,
        args.this(),
        parsed.blob,
        FileReaderReadMode::ArrayBuffer,
    ) {
        throw_type_error(
            scope,
            "FileReader.readAsArrayBuffer requires a Blob or File",
        );
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_read_as_binary_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if !ensure_file_reader_can_start_read(scope, args.this()) {
        return;
    }
    if !file_reader_start_read(
        scope,
        args.this(),
        parsed.blob,
        FileReaderReadMode::BinaryString,
    ) {
        throw_type_error(
            scope,
            "FileReader.readAsBinaryString requires a Blob or File",
        );
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let reader = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("FileReader receiver validated by WebIDL");
    if file_reader_ready_state(scope, reader) != 1.0 {
        set_file_reader_result(scope, reader, v8::null(scope).into());
        rv.set_undefined();
        return;
    }
    let next_read_id = file_reader_read_id(scope, reader) + 1.0;
    set_file_reader_read_id(scope, reader, next_read_id);
    set_file_reader_ready_state(scope, reader, 2.0);
    set_file_reader_result(scope, reader, v8::null(scope).into());
    set_file_reader_pending_result(scope, reader, v8::null(scope).into());
    dispatch_file_reader_event(scope, reader, "abort", 0.0, 0.0);
    if file_reader_ready_state(scope, reader) != 1.0 {
        dispatch_file_reader_event(scope, reader, "loadend", 0.0, 0.0);
    }
    rv.set_undefined();
}

#[derive(Clone)]
pub(crate) enum FileReaderReadMode {
    Text { encoding_label: Option<String> },
    DataUrl,
    ArrayBuffer,
    BinaryString,
}

fn file_reader_start_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    blob: v8::Local<'s, v8::Object>,
    mode: FileReaderReadMode,
) -> bool {
    let blob = moli_webapi_declare::web_api_object_target(scope, blob)
        .expect("converted Blob argument");
    let Some(data) = blob::blob_data_from_object(scope, blob) else {
        return false;
    };
    let mime_type = blob::blob_mime_type_from_object(scope, blob).unwrap_or_default();
    let total = data.len() as f64;
    let reader = moli_webapi_declare::web_api_object_target(scope, reader)
        .expect("FileReader receiver validated by WebIDL");
    let Some(context) = reader.get_creation_context(scope) else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(bytes) = data.as_bytes() {
        let Some(result) = file_reader_prepare_result(scope, bytes.to_vec(), mode, &mime_type)
        else {
            return false;
        };
        file_reader_begin_read(scope, reader, (result, total));
    } else {
        file_reader_begin_read(scope, reader, (v8::undefined(scope).into(), total));
        let read_id = file_reader_read_id(scope, reader);
        blob::read::file_reader_read(scope, reader, read_id, data, mode, mime_type);
    }
    true
}

fn file_reader_prepare_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    mode: FileReaderReadMode,
    mime_type: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let value: v8::Local<'s, v8::Value> = match mode {
        FileReaderReadMode::Text { encoding_label } => v8_string(
            scope,
            &file_reader_text(&bytes, encoding_label.as_deref(), mime_type),
        )?
        .into(),
        FileReaderReadMode::DataUrl => {
            v8_string(scope, &file_reader_data_url(&bytes, mime_type))?.into()
        }
        FileReaderReadMode::ArrayBuffer => blob::array_buffer_from_bytes(scope, bytes)?.into(),
        FileReaderReadMode::BinaryString => {
            v8_string(scope, &file_reader_binary_string(&bytes))?.into()
        }
    };
    Some(value)
}

pub(crate) fn finish_file_reader_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    read_id: f64,
    mode: FileReaderReadMode,
    mime_type: &str,
    result: std::io::Result<Vec<u8>>,
) {
    if file_reader_ready_state(scope, reader) != 1.0
        || file_reader_read_id(scope, reader) != read_id
    {
        return;
    }
    let result = match result {
        Ok(bytes) => {
            let Some(value) = file_reader_prepare_result(scope, bytes, mode, mime_type) else {
                return;
            };
            value
        }
        Err(error) => crate::context_bootstrap::new_dom_exception_value(
            scope,
            &error.to_string(),
            "NotReadableError",
        ),
    };
    set_file_reader_pending_result(scope, reader, result);
    if file_reader_load_start_dispatched(scope, reader) {
        queue_file_reader_phase(scope, reader, read_id, crate::page_task_queue::RendererPageFileReaderPhase::Progress);
    }
}

fn ensure_file_reader_can_start_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> bool {
    if file_reader_ready_state(scope, reader) != 1.0 {
        return true;
    }
    native_bridge::throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The FileReader is already loading.",
    );
    false
}

fn file_reader_begin_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    (result, total): (v8::Local<'s, v8::Value>, f64),
) {
    set_file_reader_ready_state(scope, reader, 1.0);
    set_file_reader_result(scope, reader, v8::null(scope).into());
    set_file_reader_error(scope, reader, v8::null(scope).into());
    let next_read_id = file_reader_read_id(scope, reader) + 1.0;
    set_file_reader_read_id(scope, reader, next_read_id);
    set_file_reader_load_start_dispatched(scope, reader, false);
    set_file_reader_pending_result(scope, reader, result);
    set_file_reader_pending_total(scope, reader, total);
    queue_file_reader_phase(
        scope,
        reader,
        next_read_id,
        crate::page_task_queue::RendererPageFileReaderPhase::LoadStart,
    );
}

use super::*;
use crate::webidl;
use moli_file_api::{file_reader_binary_string, file_reader_data_url, file_reader_text};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReader.read")]
struct FileReaderReadBlobArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        converter = "raw",
        missing_message = "Failed to execute 'read' on 'FileReader': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileReader.readAsText")]
struct FileReaderReadAsTextArgs<'s> {
    #[webidl(
        required,
        name = "blob",
        converter = "raw",
        missing_message = "Failed to execute 'readAsText' on 'FileReader': blob argument is required."
    )]
    blob: v8::Local<'s, v8::Value>,
    encoding: Option<String>,
}

pub(in crate::context_bootstrap) fn file_reader_read_as_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !ensure_file_reader_can_start_read(scope, args.this(), &mut rv) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileReaderReadAsTextArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(result) = file_reader_prepare_result(
        scope,
        parsed.blob,
        FileReaderReadMode::Text {
            encoding_label: parsed.encoding,
        },
    ) {
        file_reader_begin_read(scope, args.this(), result);
    } else {
        throw_type_error(scope, "FileReader.readAsText requires a Blob or File");
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_read_as_data_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !ensure_file_reader_can_start_read(scope, args.this(), &mut rv) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(result) =
        file_reader_prepare_result(scope, parsed.blob, FileReaderReadMode::DataUrl)
    {
        file_reader_begin_read(scope, args.this(), result);
    } else {
        throw_type_error(scope, "FileReader.readAsDataURL requires a Blob or File");
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_read_as_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !ensure_file_reader_can_start_read(scope, args.this(), &mut rv) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(result) =
        file_reader_prepare_result(scope, parsed.blob, FileReaderReadMode::ArrayBuffer)
    {
        file_reader_begin_read(scope, args.this(), result);
    } else {
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
    if !ensure_file_reader_can_start_read(scope, args.this(), &mut rv) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<FileReaderReadBlobArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(result) =
        file_reader_prepare_result(scope, parsed.blob, FileReaderReadMode::BinaryString)
    {
        file_reader_begin_read(scope, args.this(), result);
    } else {
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
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if file_reader_ready_state(scope, args.this()) != 1.0 {
        rv.set_undefined();
        return;
    }
    let next_read_id = file_reader_read_id(scope, args.this()) + 1.0;
    set_file_reader_read_id(scope, args.this(), next_read_id);
    set_file_reader_ready_state(scope, args.this(), 2.0);
    set_file_reader_result(scope, args.this(), v8::null(scope).into());
    dispatch_file_reader_event(scope, args.this(), "abort", 0.0, 0.0);
    dispatch_file_reader_event(scope, args.this(), "loadend", 0.0, 0.0);
    rv.set_undefined();
}

enum FileReaderReadMode {
    Text { encoding_label: Option<String> },
    DataUrl,
    ArrayBuffer,
    BinaryString,
}

fn file_reader_prepare_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob_value: v8::Local<'s, v8::Value>,
    mode: FileReaderReadMode,
) -> Option<(v8::Local<'s, v8::Value>, f64)> {
    let blob = blob_value.to_object(scope)?;
    let bytes = blob::blob_bytes_from_object(scope, blob)?;
    let mime_type = blob::blob_mime_type_from_object(scope, blob).unwrap_or_default();
    let total = bytes.len() as f64;
    let value: v8::Local<'s, v8::Value> = match mode {
        FileReaderReadMode::Text { encoding_label } => v8_string(
            scope,
            &file_reader_text(&bytes, encoding_label.as_deref(), &mime_type),
        )?
        .into(),
        FileReaderReadMode::DataUrl => {
            v8_string(scope, &file_reader_data_url(&bytes, &mime_type))?.into()
        }
        FileReaderReadMode::ArrayBuffer => blob::array_buffer_from_bytes(scope, bytes)?.into(),
        FileReaderReadMode::BinaryString => {
            v8_string(scope, &file_reader_binary_string(&bytes))?.into()
        }
    };
    Some((value, total))
}

fn ensure_file_reader_can_start_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> bool {
    if !file_reader_receiver_branded(scope, reader) {
        throw_type_error(scope, "Illegal invocation");
        return false;
    }
    if file_reader_ready_state(scope, reader) != 1.0 {
        return true;
    }
    native_bridge::throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The FileReader is already loading.",
    );
    rv.set_undefined();
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
    set_file_reader_task_phase(scope, reader, 0.0);
    set_file_reader_pending_result(scope, reader, result);
    set_file_reader_pending_total(scope, reader, total);
    if file_reader_scheduled(scope, reader) {
        return;
    }
    set_file_reader_scheduled(scope, reader, true);
    push_object_to_global_queue(scope, FILE_READER_QUEUE_SLOT, reader);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        schedule_host_callback(scope, host, file_reader_flush_callback);
    }
}

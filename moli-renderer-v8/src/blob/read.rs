use std::{io, sync::Arc};

use moli_file_api::BlobReadCompletion;
use parking_lot::Mutex;

use super::*;

#[derive(Clone, Copy)]
pub(crate) enum BlobReadFormat {
    Text,
    ArrayBuffer,
    Bytes,
}

pub(crate) struct BlobReadTask {
    context: v8::Global<v8::Context>,
    target: BlobReadTarget,
    pub(crate) result: Arc<Mutex<Option<io::Result<Vec<u8>>>>>,
}

enum BlobReadTarget {
    Promise {
        resolver: v8::Global<v8::PromiseResolver>,
        format: BlobReadFormat,
    },
    Stream {
        resolver: v8::Global<v8::PromiseResolver>,
        source: v8::Global<v8::Object>,
        stream: v8::Global<v8::Object>,
        last: bool,
    },
    FileReader {
        reader: v8::Global<v8::Object>,
        read_id: f64,
        mode: crate::context_bootstrap::FileReaderReadMode,
        mime_type: String,
    },
}

impl BlobReadTask {
    pub(crate) fn settle(self, scope: &mut v8::PinScope<'_, '_>) {
        let Some(result) = self.result.lock().take() else {
            return;
        };
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        match self.target {
            BlobReadTarget::Promise { resolver, format } => {
                let resolver = v8::Local::new(scope, resolver);
                settle(scope, resolver, format, result);
            }
            BlobReadTarget::Stream {
                resolver,
                source,
                stream,
                last,
            } => {
                let resolver = v8::Local::new(scope, resolver);
                let source = v8::Local::new(scope, source);
                let stream = v8::Local::new(scope, stream);
                super::stream::finish_read(scope, source, stream, resolver, last, result);
            }
            BlobReadTarget::FileReader {
                reader,
                read_id,
                mode,
                mime_type,
            } => {
                let reader = v8::Local::new(scope, reader);
                crate::context_bootstrap::finish_file_reader_read(
                    scope, reader, read_id, mode, &mime_type, result,
                );
            }
        }
    }
}

fn settle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    format: BlobReadFormat,
    result: io::Result<Vec<u8>>,
) {
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(error) => {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                &error.to_string(),
                "NotReadableError",
            );
            let _ = resolver.reject(scope, error);
            return;
        }
    };
    let value = match format {
        BlobReadFormat::Text => {
            v8_string(scope, &moli_encoding::decode_utf8(&bytes)).map(Into::into)
        }
        BlobReadFormat::ArrayBuffer => array_buffer_from_bytes(scope, bytes).map(Into::into),
        BlobReadFormat::Bytes => new_uint8_array_from_bytes(scope, bytes).map(Into::into),
    };
    if let Some(value) = value {
        let _ = resolver.resolve(scope, value);
    }
}

pub(super) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    format: BlobReadFormat,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = blob_data_from_object(scope, object) else {
        return;
    };
    let Some(context) = moli_webapi_declare::web_api_object_target(scope, object)
        .and_then(|target| target.get_creation_context(scope)) else { return; };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(promise) = read_promise(scope, data, format) {
        rv.set(promise.into());
    }
}

pub(crate) fn read_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: moli_file_api::BlobData,
    format: BlobReadFormat,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let target = BlobReadTarget::Promise {
        resolver: v8::Global::new(scope, resolver),
        format,
    };
    submit(scope, resolver, data, target);
    Some(promise)
}

pub(super) fn stream_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
    data: moli_file_api::BlobData,
    last: bool,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    let target = BlobReadTarget::Stream {
        resolver: v8::Global::new(scope, resolver),
        source: v8::Global::new(scope, source),
        stream: v8::Global::new(scope, stream),
        last,
    };
    submit(scope, resolver, data, target);
}

pub(crate) fn file_reader_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    read_id: f64,
    data: moli_file_api::BlobData,
    mode: crate::context_bootstrap::FileReaderReadMode,
    mime_type: String,
) {
    let Some(context) = reader.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let target = BlobReadTarget::FileReader {
        reader: v8::Global::new(scope, reader),
        read_id,
        mode: mode.clone(),
        mime_type: mime_type.clone(),
    };
    if !submit_task(scope, data, target) {
        crate::context_bootstrap::finish_file_reader_read(
            scope,
            reader,
            read_id,
            mode,
            &mime_type,
            Err(io::Error::other("Blob read has no live execution owner")),
        );
    }
}

fn submit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    data: moli_file_api::BlobData,
    target: BlobReadTarget,
) {
    if !submit_task(scope, data, target) {
        let error = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "Blob read has no live execution owner",
            "NotReadableError",
        );
        let _ = resolver.reject(scope, error);
    }
}

fn submit_task(
    scope: &mut v8::PinScope<'_, '_>,
    data: moli_file_api::BlobData,
    target: BlobReadTarget,
) -> bool {
    let result = Arc::new(Mutex::new(None));
    let task = BlobReadTask {
        context: v8::Global::new(scope, scope.get_current_context()),
        target,
        result: result.clone(),
    };
    if let Some(bytes) = data.as_bytes() {
        *result.lock() = Some(Ok(bytes.to_vec()));
        return crate::context_bootstrap::queue_file_reading_task(
            scope, crate::context_bootstrap::FileReadingTask::Blob(task),
        );
    }
    let complete = if let Some(host) = crate::util::context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.register_blob_read_task(scope, task)
    } else {
        crate::worker::register_worker_blob_read_task(scope, task)
    };
    match complete {
        Some(complete) => {
            data.read_all(complete);
            true
        }
        None => false,
    }
}

pub(crate) fn completion_slot(
    slot: &Arc<Mutex<Option<io::Result<Vec<u8>>>>>,
    wake: impl FnOnce() -> bool + Send + 'static,
) -> BlobReadCompletion {
    let slot = Arc::downgrade(slot);
    Box::new(move |result| {
        let Some(slot) = slot.upgrade() else {
            return;
        };
        let mut slot = slot.lock();
        *slot = Some(result);
        if !wake() {
            slot.take();
        }
    })
}

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

pub(crate) fn read_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: moli_file_api::BlobData,
    format: BlobReadFormat,
) -> Option<v8::Local<'s, v8::Promise>> {
    let owner = scope.get_current_context();
    read_promise_in_owner(scope, data, format, owner)
}

pub(super) fn read_promise_in_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: moli_file_api::BlobData,
    format: BlobReadFormat,
    owner: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    if let Some(bytes) = data.as_bytes() {
        settle(scope, resolver, format, Ok(bytes.to_vec()));
    } else {
        let target = BlobReadTarget::Promise {
            resolver: v8::Global::new(scope, resolver),
            format,
        };
        let scope = &mut v8::ContextScope::new(scope, owner);
        submit(scope, resolver, data, target);
    }
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
    if let Some(bytes) = data.as_bytes() {
        let task = super::reading::BlobReadTask::stream_chunk(
            scope,
            source,
            stream,
            resolver,
            bytes.to_vec(),
            last,
        );
        if !crate::context_bootstrap::queue_file_reading_task(
            scope,
            crate::context_bootstrap::FileReadingTask::Blob(task),
        ) {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "Blob read has no live execution owner",
                "NotReadableError",
            );
            let _ = resolver.reject(scope, error);
        }
        return;
    }
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
    // FileReading ownership follows the Blob; borrowed promise methods still
    // allocate their binary result in the method's realm.
    let context = match &target {
        BlobReadTarget::Promise { resolver, .. } => {
            v8::Local::new(scope, resolver).get_creation_context(scope)
        }
        _ => None,
    }
    .unwrap_or_else(|| scope.get_current_context());
    let task = BlobReadTask {
        context: v8::Global::new(scope, context),
        target,
        result: result.clone(),
    };
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

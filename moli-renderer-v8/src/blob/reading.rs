use super::{array_buffer_from_bytes, blob_bytes_from_object, new_uint8_array_from_bytes};
use crate::context_bootstrap::{
    FileReadingTask, complete_blob_readable_stream, new_blob_readable_stream,
    queue_file_reading_task,
};
use crate::util::v8_string;

pub(super) enum BlobReadMode {
    Text,
    ArrayBuffer,
    Bytes,
}

enum BlobReadRecipient {
    Promise {
        resolver: v8::Global<v8::PromiseResolver>,
        mode: BlobReadMode,
    },
    Stream(v8::Global<v8::Object>),
}

/// Own the immutable bytes until a file-reading task for the Blob's global runs.
/// Neither the borrowing method's realm nor the lifetime of its wrapper owns
/// this completion. Keep the exact realm for isolated-world result allocation.
pub(crate) struct BlobReadTask {
    context: v8::Global<v8::Context>,
    bytes: Vec<u8>,
    recipient: BlobReadRecipient,
}

impl BlobReadTask {
    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) {
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let scope = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut scope.init();
        match self.recipient {
            BlobReadRecipient::Promise { resolver, mode } => {
                let resolver = v8::Local::new(scope, &resolver);
                let value = match mode {
                    BlobReadMode::Text => {
                        v8_string(scope, &moli_encoding::decode_utf8(&self.bytes)).map(Into::into)
                    }
                    BlobReadMode::ArrayBuffer => {
                        array_buffer_from_bytes(scope, self.bytes).map(Into::into)
                    }
                    BlobReadMode::Bytes => {
                        new_uint8_array_from_bytes(scope, self.bytes).map(Into::into)
                    }
                };
                if let Some(value) = value {
                    let _ = resolver.resolve(scope, value);
                } else {
                    let error = scope.exception().unwrap_or_else(|| {
                        let message = v8::String::new(scope, "Could not allocate Blob read result")
                            .unwrap_or_else(|| v8::String::empty(scope));
                        v8::Exception::error(scope, message)
                    });
                    scope.reset();
                    let _ = resolver.reject(scope, error);
                }
            }
            BlobReadRecipient::Stream(stream) => {
                let stream = v8::Local::new(scope, &stream);
                if complete_blob_readable_stream(scope, stream, self.bytes).is_none() {
                    let error = scope.exception().unwrap_or_else(|| {
                        let message =
                            v8::String::new(scope, "Could not allocate Blob stream chunk")
                                .unwrap_or_else(|| v8::String::empty(scope));
                        v8::Exception::error(scope, message)
                    });
                    scope.reset();
                    crate::context_bootstrap::error_stream(scope, stream, error);
                }
            }
        }
    }
}

fn blob_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Context>> {
    moli_webapi_declare::web_api_object_target(scope, blob)?.get_creation_context(scope)
}

pub(super) fn read_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
    mode: BlobReadMode,
) -> Option<v8::Local<'s, v8::Promise>> {
    let bytes = blob_bytes_from_object(scope, blob)?;
    let context = blob_relevant_context(scope, blob)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let task = BlobReadTask {
        context: v8::Global::new(scope, context),
        bytes,
        recipient: BlobReadRecipient::Promise {
            resolver: v8::Global::new(scope, resolver),
            mode,
        },
    };
    let _ = queue_file_reading_task(scope, FileReadingTask::Blob(task));
    Some(promise)
}

pub(crate) fn stream_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let bytes = blob_bytes_from_object(scope, blob)?;
    let context = blob_relevant_context(scope, blob)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let stream = new_blob_readable_stream(scope);
    let task = BlobReadTask {
        context: v8::Global::new(scope, context),
        bytes,
        recipient: BlobReadRecipient::Stream(v8::Global::new(scope, stream)),
    };
    let _ = queue_file_reading_task(scope, FileReadingTask::Blob(task));
    Some(stream)
}

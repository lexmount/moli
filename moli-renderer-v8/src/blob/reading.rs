use super::{array_buffer_from_bytes, blob_data_from_object, new_uint8_array_from_bytes};
use crate::context_bootstrap::{FileReadingTask, queue_file_reading_task};
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
    Stream {
        resolver: v8::Global<v8::PromiseResolver>,
        source: v8::Global<v8::Object>,
        stream: v8::Global<v8::Object>,
        last: bool,
    },
}

/// Promise results own fully-read bytes; streams admit only the bounded range
/// requested by pull. Both use the shared FileReading FIFO.
pub(crate) struct BlobReadTask {
    context: v8::Global<v8::Context>,
    bytes: Vec<u8>,
    recipient: BlobReadRecipient,
}

impl BlobReadTask {
    pub(super) fn stream_chunk<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Object>,
        stream: v8::Local<'s, v8::Object>,
        resolver: v8::Local<'s, v8::PromiseResolver>,
        bytes: Vec<u8>,
        last: bool,
    ) -> Self {
        Self {
            context: v8::Global::new(scope, scope.get_current_context()),
            bytes,
            recipient: BlobReadRecipient::Stream {
                resolver: v8::Global::new(scope, resolver),
                source: v8::Global::new(scope, source),
                stream: v8::Global::new(scope, stream),
                last,
            },
        }
    }

    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) {
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let scope = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut scope.init();
        match self.recipient {
            BlobReadRecipient::Promise { resolver, mode } => {
                let resolver = v8::Local::new(scope, resolver);
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
            BlobReadRecipient::Stream {
                resolver,
                source,
                stream,
                last,
            } => {
                let resolver = v8::Local::new(scope, resolver);
                let source = v8::Local::new(scope, source);
                let stream = v8::Local::new(scope, stream);
                super::stream::finish_read(scope, source, stream, resolver, last, Ok(self.bytes));
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
    let context = blob_relevant_context(scope, blob)?;
    let data = blob_data_from_object(scope, blob)?;
    let Some(bytes) = data.as_bytes() else {
        let format = match mode {
            BlobReadMode::Text => super::read::BlobReadFormat::Text,
            BlobReadMode::ArrayBuffer => super::read::BlobReadFormat::ArrayBuffer,
            BlobReadMode::Bytes => super::read::BlobReadFormat::Bytes,
        };
        return super::read::read_promise_in_owner(scope, data, format, context);
    };
    let bytes = bytes.to_vec();
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let task = BlobReadTask {
        context: v8::Global::new(scope, scope.get_current_context()),
        bytes,
        recipient: BlobReadRecipient::Promise {
            resolver: v8::Global::new(scope, resolver),
            mode,
        },
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let _ = queue_file_reading_task(scope, FileReadingTask::Blob(task));
    Some(promise)
}

pub(crate) fn stream_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = blob_relevant_context(scope, blob)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    super::stream::create(scope, blob)
}

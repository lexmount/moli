use std::io;

use super::*;
use crate::util::set_private_value;

const BLOB: &str = "__moliBlobStreamBlob";
const STREAM: &str = "__moliBlobStream";
const OFFSET: &str = "__moliBlobStreamOffset";
const CANCELLED: &str = "__moliBlobStreamCancelled";

#[derive(WebApiObject)]
#[webapi(plain, data_properties)]
struct BlobStreamSource<'s> {
    #[webapi(slot = BLOB)]
    blob: v8::Local<'s, v8::Object>,
    #[webapi(slot = STREAM, init = "null")]
    stream: (),
    #[webapi(slot = OFFSET, init = 0)]
    offset: (),
    #[webapi(slot = CANCELLED, init = false)]
    cancelled: (),
    #[webapi(method, callback = pull)]
    pull: (),
    start: v8::Local<'s, v8::Value>,
    #[webapi(method, callback = cancel)]
    cancel: (),
}

pub(crate) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let blob = moli_webapi_declare::web_api_object_target(scope, blob)?;
    let context = blob.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = BlobStreamSource {
        blob,
        stream: (),
        offset: (),
        cancelled: (),
        pull: (),
        start: v8::undefined(scope).into(),
        cancel: (),
    }
    .bind(scope)
    .ok()?;
    let stream = crate::context_bootstrap::new_readable_stream_from_source(scope, source);
    set_private_value(scope, source, STREAM, stream.into());
    Some(stream)
}

fn pull<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some(stream) =
        get_private_value(scope, source, STREAM).and_then(|value| value.to_object(scope))
    else {
        return;
    };
    let Some(blob) =
        get_private_value(scope, source, BLOB).and_then(|value| value.to_object(scope))
    else {
        return;
    };
    let Some(data) = blob_data_from_object(scope, blob) else {
        return;
    };
    let offset = get_private_value(scope, source, OFFSET)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0) as usize;
    let end = offset.saturating_add(64 * 1024).min(data.len());
    if offset >= data.len() {
        crate::context_bootstrap::close_stream(scope, stream);
        set_private_value(scope, source, BLOB, v8::null(scope).into());
        return;
    }
    set_private_value(
        scope,
        source,
        OFFSET,
        v8::Number::new(scope, end as f64).into(),
    );
    read::stream_read(
        scope,
        source,
        stream,
        data.slice(offset..end),
        end == data.len(),
        &mut rv,
    );
}

fn cancel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_private_value(
        scope,
        args.this(),
        CANCELLED,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(scope, args.this(), BLOB, v8::null(scope).into());
}

pub(super) fn finish_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    last: bool,
    result: io::Result<Vec<u8>>,
) {
    let cancelled =
        get_private_value(scope, source, CANCELLED).is_some_and(|value| value.boolean_value(scope));
    if cancelled {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    }
    match result {
        Ok(bytes) => {
            let Some(chunk) = new_uint8_array_from_bytes(scope, bytes) else {
                return;
            };
            let _ = crate::context_bootstrap::enqueue_byte_chunk(scope, stream, chunk.into());
            if last {
                crate::context_bootstrap::close_stream(scope, stream);
                set_private_value(scope, source, BLOB, v8::null(scope).into());
            }
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                &error.to_string(),
                "NotReadableError",
            );
            crate::context_bootstrap::error_stream(scope, stream, error);
            set_private_value(scope, source, BLOB, v8::null(scope).into());
            let _ = resolver.reject(scope, error);
        }
    }
}

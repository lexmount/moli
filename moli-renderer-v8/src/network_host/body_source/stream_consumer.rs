//! Fetch's read-all-bytes loop uses internal Streams read requests. Public
//! reader promises would expose intermediate `{ value, done }` objects to
//! thenable assimilation before Fetch has checked or copied the chunk.

use super::*;
use crate::context_bootstrap::{
    begin_readable_stream_body_consumption, maybe_pull_stream,
    prepare_readable_stream_read_with_steps,
};
use crate::util::set_null_prototype;

const STREAM: &str = "__moliBodyConsumerStream";
const RESOLVER: &str = "__moliBodyConsumerResolver";
const CHUNKS: &str = "__moliBodyConsumerChunks";
const KIND: &str = "__moliBodyConsumerKind";
const MIME: &str = "__moliBodyConsumerMime";
const ON_CHUNK: &str = "__moliBodyConsumerOnChunk";
const PENDING_CHUNK: &str = "__moliBodyConsumerPendingChunk";
const STEPS: &str = "__moliBodyConsumerSteps";
const DRAINING: &str = "__moliBodyConsumerDraining";
const READ_AGAIN: &str = "__moliBodyConsumerReadAgain";
const FINISHED: &str = "__moliBodyConsumerFinished";
const SUCCEEDED: &str = "__moliBodyConsumerSucceeded";
const REASON: &str = "__moliBodyConsumerReason";

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct BodyConsumerDeclaration<'scope> {
    #[webapi(slot = STREAM)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CHUNKS)]
    chunks: v8::Local<'scope, v8::Array>,
    #[webapi(slot = KIND)]
    kind: &'static str,
    #[webapi(slot = MIME)]
    mime: String,
    #[webapi(slot = ON_CHUNK)]
    on_chunk: Option<v8::Local<'scope, v8::Function>>,
    #[webapi(slot = PENDING_CHUNK, init = "null")]
    pending_chunk: (),
    #[webapi(slot = STEPS, init = "null")]
    steps: (),
    #[webapi(slot = DRAINING, init = false)]
    draining: (),
    #[webapi(slot = READ_AGAIN, init = false)]
    read_again: (),
    #[webapi(slot = FINISHED, init = false)]
    finished: (),
    #[webapi(slot = SUCCEEDED, init = false)]
    succeeded: (),
    #[webapi(slot = REASON, init = "null")]
    reason: (),
}

#[derive(Clone, Copy)]
struct Consumer<'s>(v8::Local<'s, v8::Object>);

impl<'s> Consumer<'s> {
    fn value(self, scope: &mut v8::PinScope<'s, '_>, slot: &str) -> v8::Local<'s, v8::Value> {
        get_private_value(scope, self.0, slot).unwrap_or_else(|| v8::undefined(scope).into())
    }

    fn flag(self, scope: &mut v8::PinScope<'s, '_>, slot: &str) -> bool {
        self.value(scope, slot).is_true()
    }

    fn set_flag(self, scope: &mut v8::PinScope<'s, '_>, slot: &str, value: bool) {
        set_private_value(scope, self.0, slot, v8::Boolean::new(scope, value).into());
    }

    fn array(self, scope: &mut v8::PinScope<'s, '_>, slot: &str) -> v8::Local<'s, v8::Array> {
        v8::Local::try_from(self.value(scope, slot)).expect("body consumer array must exist")
    }

    fn read(self, scope: &mut v8::PinScope<'s, '_>) {
        if self.flag(scope, FINISHED) {
            return;
        }
        if self.flag(scope, DRAINING) {
            self.set_flag(scope, READ_AGAIN, true);
            return;
        }
        self.set_flag(scope, DRAINING, true);
        let stream = v8::Local::try_from(self.value(scope, STREAM))
            .expect("body consumer must retain its stream");
        let steps = self.array(scope, STEPS);
        let callbacks = [0, 1, 2].map(|index| {
            v8::Local::try_from(steps.get_index(scope, index).expect("read step must exist"))
                .expect("read step must be a function")
        });
        let mut pull = false;
        loop {
            self.set_flag(scope, READ_AGAIN, false);
            pull |= prepare_readable_stream_read_with_steps(
                scope,
                stream,
                callbacks[0],
                callbacks[1],
                callbacks[2],
            );
            if self.flag(scope, FINISHED) || !self.flag(scope, READ_AGAIN) {
                break;
            }
        }
        self.set_flag(scope, DRAINING, false);
        // Drain already queued chunks before calling author pull code. This
        // implements recursive chunk steps without growing the native stack.
        if pull {
            maybe_pull_stream(scope, stream);
        }
    }

    fn finish(self, scope: &mut v8::PinScope<'s, '_>, error: Option<v8::Local<'s, v8::Value>>) {
        if self.flag(scope, FINISHED) {
            return;
        }
        self.set_flag(scope, FINISHED, true);
        self.set_flag(scope, SUCCEEDED, error.is_none());
        if let Some(error) = error {
            set_private_value(scope, self.0, REASON, error);
            set_private_value(scope, self.0, CHUNKS, v8::null(scope).into());
        }
        let callback = v8::Function::builder(materialize_callback)
            .data(self.0.into())
            .build(scope)
            .expect("body materialization callback must allocate");
        // Retain the existing asynchronous completion boundary while removing
        // the observable intermediate reader promises.
        scope.enqueue_microtask(callback);
    }
}

pub(super) fn consume_readable_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: Option<v8::Local<'s, v8::Function>>,
) -> (NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return (NetworkBodyConsumption::Failed, None);
    };
    if !begin_readable_stream_body_consumption(scope, stream) {
        let error = v8::Exception::type_error(scope, v8str(scope, "Body stream is locked"));
        return (NetworkBodyConsumption::Rejected(error), None);
    }
    let promise = resolver.get_promise(scope);
    let (kind, mime) = match kind {
        NetworkBodyConsumptionKind::Text => ("text", String::new()),
        NetworkBodyConsumptionKind::Json => ("json", String::new()),
        NetworkBodyConsumptionKind::ArrayBuffer => ("arrayBuffer", String::new()),
        NetworkBodyConsumptionKind::Bytes => ("bytes", String::new()),
        NetworkBodyConsumptionKind::Blob { mime_type } => ("blob", mime_type),
        NetworkBodyConsumptionKind::FormData { content_type } => ("formData", content_type),
    };
    let chunks = v8::Array::new(scope, 0);
    set_null_prototype(scope, chunks.into());
    let consumer = Consumer(
        BodyConsumerDeclaration::new(stream, resolver.into(), chunks, kind, mime, chunk_callback)
            .bind(scope)
            .expect("body consumer declaration must bind"),
    );
    let callbacks = [
        v8::Function::builder(chunk_callback_step)
            .data(consumer.0.into())
            .build(scope),
        v8::Function::builder(close_callback_step)
            .data(consumer.0.into())
            .build(scope),
        v8::Function::builder(error_callback_step)
            .data(consumer.0.into())
            .build(scope),
    ]
    .map(|step| step.expect("body read step must allocate").into());
    let steps = v8::Array::new_with_elements(scope, &callbacks);
    set_null_prototype(scope, steps.into());
    set_private_value(scope, consumer.0, STEPS, steps.into());
    let cancel_handle = chunk_callback.map(|_| v8::Global::new(scope, stream));
    consumer.read(scope);
    (NetworkBodyConsumption::Pending(promise), cancel_handle)
}

fn chunk_callback_step<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let consumer = Consumer(v8::Local::try_from(args.data()).expect("body read step owner"));
    let Ok(chunk) = v8::Local::<v8::Uint8Array>::try_from(args.get(0)) else {
        let error = v8::Exception::type_error(
            scope,
            v8str(scope, "ReadableStream body chunks must be Uint8Array"),
        );
        consumer.finish(scope, Some(error));
        return;
    };
    // Snapshot the represented bytes now, including view offset and length.
    // A subsequent pull, enqueue, or callback may mutate or detach this chunk.
    let mut bytes = vec![0; chunk.byte_length()];
    let written = chunk.copy_contents(&mut bytes);
    bytes.truncate(written);
    let buffer = blob::array_buffer_from_bytes(scope, bytes).expect("chunk copy must allocate");
    let chunks = consumer.array(scope, CHUNKS);
    chunks
        .set_index(scope, chunks.length(), buffer.into())
        .expect("private chunk list must append");
    if consumer.value(scope, ON_CHUNK).is_function() {
        // Streaming response delivery first publishes its head and cancel
        // handle. Host network enqueue paths can also hold a mutable worker
        // borrow: invoke their chunk consumer after that delivery returns.
        set_private_value(scope, consumer.0, PENDING_CHUNK, buffer.into());
        let callback = v8::Function::builder(deliver_chunk_callback)
            .data(consumer.0.into())
            .build(scope)
            .expect("body chunk delivery callback must allocate");
        scope.enqueue_microtask(callback);
    } else {
        consumer.read(scope);
    }
}

fn deliver_chunk_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let consumer = Consumer(v8::Local::try_from(args.data()).expect("body chunk delivery owner"));
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(consumer.value(scope, PENDING_CHUNK))
        .expect("body chunk delivery must retain its copied bytes");
    let chunk = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())
        .expect("body chunk delivery view must allocate");
    set_private_value(scope, consumer.0, PENDING_CHUNK, v8::null(scope).into());
    let callback = v8::Local::<v8::Function>::try_from(consumer.value(scope, ON_CHUNK))
        .expect("body chunk delivery consumer must exist");
    {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut try_catch.init();
        if callback
            .call(scope, v8::undefined(scope).into(), &[chunk.into()])
            .is_none()
        {
            let error = scope
                .exception()
                .unwrap_or_else(|| v8::undefined(scope).into());
            consumer.finish(scope, Some(error));
            return;
        }
    }
    consumer.read(scope);
}

fn close_callback_step<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    Consumer(v8::Local::try_from(args.data()).expect("body read step owner")).finish(scope, None);
}

fn error_callback_step<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    Consumer(v8::Local::try_from(args.data()).expect("body read step owner"))
        .finish(scope, Some(args.get(0)));
}

fn materialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let consumer = Consumer(v8::Local::try_from(args.data()).expect("body materialization owner"));
    let resolver = v8::Local::<v8::Object>::try_from(consumer.value(scope, RESOLVER))
        .expect("body materialization resolver");
    // SAFETY: the private slot is initialized only from PromiseResolver::new
    // and is cleared after this single completion callback.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    if consumer.flag(scope, SUCCEEDED) {
        let chunks = consumer.array(scope, CHUNKS);
        let mut bytes = Vec::new();
        for index in 0..chunks.length() {
            let chunk = chunks
                .get_index(scope, index)
                .expect("body chunk copy must exist");
            bytes.extend(
                blob::buffer_source_bytes_from_value(scope, chunk).expect("body chunk bytes"),
            );
        }
        let mime = consumer.value(scope, MIME).to_rust_string_lossy(scope);
        let kind = match consumer
            .value(scope, KIND)
            .to_rust_string_lossy(scope)
            .as_str()
        {
            "text" => PendingBodyMaterializationKind::Text,
            "json" => PendingBodyMaterializationKind::Json,
            "arrayBuffer" => PendingBodyMaterializationKind::ArrayBuffer,
            "bytes" => PendingBodyMaterializationKind::Bytes,
            "blob" => PendingBodyMaterializationKind::Blob { mime_type: mime },
            "formData" => PendingBodyMaterializationKind::FormData { content_type: mime },
            _ => unreachable!("body materialization kind is private"),
        };
        resolve_body_materialization(scope, resolver, bytes, kind);
    } else {
        let error = consumer.value(scope, REASON);
        let _ = resolver.reject(scope, error);
    }
    for slot in [
        STREAM,
        RESOLVER,
        CHUNKS,
        ON_CHUNK,
        PENDING_CHUNK,
        STEPS,
        REASON,
    ] {
        set_private_value(scope, consumer.0, slot, v8::null(scope).into());
    }
}

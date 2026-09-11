//! Web IDL callback values embedded in page-supplied Streams dictionaries.
//!
//! Streams own the algorithms that call these callbacks: the stream/controller
//! still chooses when to call, supplies the underlying object as `this`,
//! normalizes Promise results, and applies queue/backpressure state. This
//! module owns only the one-time Web IDL dictionary conversion, a V8-traced
//! callback residence, and relevant/incumbent-context entry for one call.
//!
//! Browser-created stream algorithms are deliberately not represented by
//! these types. They remain ordinary V8 functions in the existing controller
//! state and are selected by `invoke_stored_stream_algorithm`.

use moli_webidl_callback::invoke_webidl_callback_function;

use super::{call_function_result, *};
use crate::{
    util::context_host_ptr_from_global_bridge,
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    webidl,
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunctionOutcome, V8TracedWindowWebIdlCallbackFunction,
    },
};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ReadableStreamType")]
enum ReadableStreamType {
    #[webidl(token = "bytes")]
    Bytes,
}

/// Members are declared in Web IDL lexicographic order. The non-callback
/// members are retained so their getters and conversions happen at the
/// standard-defined point even though byte-stream behavior remains owned by
/// the existing stream implementation.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UnderlyingSource")]
struct UnderlyingSourceMembers {
    #[webidl(
        name = "autoAllocateChunkSize",
        converter = "enforce_range_unsigned_long_long"
    )]
    auto_allocate_chunk_size: Option<u64>,
    #[webidl(converter = "callback_function")]
    cancel: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    pull: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    start: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "type", converter = "enum")]
    stream_type: Option<ReadableStreamType>,
}

/// Members are declared in Web IDL lexicographic order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UnderlyingSink")]
struct UnderlyingSinkMembers<'s> {
    #[webidl(converter = "callback_function")]
    abort: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    close: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    start: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "type", converter = "raw")]
    sink_type: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "callback_function")]
    write: Option<webidl::WebIdlCallbackFunction>,
}

/// Members are declared in Web IDL lexicographic order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "Transformer")]
struct TransformerMembers<'s> {
    #[webidl(converter = "callback_function")]
    cancel: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    flush: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "readableType", converter = "raw")]
    readable_type: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "callback_function")]
    start: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_function")]
    transform: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "writableType", converter = "raw")]
    writable_type: Option<v8::Local<'s, v8::Value>>,
}

/// The two queuing-strategy members are also read in lexicographic order:
/// `highWaterMark` before `size`.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "QueuingStrategy")]
struct QueuingStrategyMembers {
    #[webidl(name = "highWaterMark", converter = "unrestricted_double")]
    high_water_mark: Option<f64>,
    #[webidl(converter = "callback_function")]
    size: Option<webidl::WebIdlCallbackFunction>,
}

pub(in crate::context_bootstrap) struct WebIdlReadableStreamSource<'s> {
    pub(in crate::context_bootstrap) object: v8::Local<'s, v8::Object>,
    pub(in crate::context_bootstrap) start: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) pull: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) cancel: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) auto_allocate_chunk_size: Option<u64>,
    pub(in crate::context_bootstrap) byte_stream: bool,
}

pub(in crate::context_bootstrap) struct WebIdlWritableStreamSink<'s> {
    pub(in crate::context_bootstrap) object: v8::Local<'s, v8::Object>,
    pub(in crate::context_bootstrap) start: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) write: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) close: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) abort: Option<webidl::WebIdlCallbackFunction>,
}

pub(in crate::context_bootstrap) struct WebIdlTransformStreamTransformer<'s> {
    pub(in crate::context_bootstrap) object: v8::Local<'s, v8::Object>,
    pub(in crate::context_bootstrap) start: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) transform: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) flush: Option<webidl::WebIdlCallbackFunction>,
    pub(in crate::context_bootstrap) cancel: Option<webidl::WebIdlCallbackFunction>,
}

pub(in crate::context_bootstrap) struct StreamQueuingStrategy {
    pub(in crate::context_bootstrap) high_water_mark: f64,
    pub(in crate::context_bootstrap) high_water_mark_provided: bool,
    pub(in crate::context_bootstrap) size: Option<webidl::WebIdlCallbackFunction>,
}

impl StreamQueuingStrategy {
    pub(in crate::context_bootstrap) const fn without_size(high_water_mark: f64) -> Self {
        Self {
            high_water_mark,
            high_water_mark_provided: true,
            size: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) struct StreamWebIdlCallbackCarrier<'s> {
    object: v8::Local<'s, v8::Object>,
}

impl<'s> StreamWebIdlCallbackCarrier<'s> {
    pub(in crate::context_bootstrap) fn new(
        scope: &mut v8::PinScope<'s, '_>,
        callback: webidl::WebIdlCallbackFunction,
    ) -> Self {
        Self {
            object: V8TracedWebIdlCallbackFunction::new(scope, callback).into_object(),
        }
    }

    pub(in crate::context_bootstrap) const fn from_object(
        object: v8::Local<'s, v8::Object>,
    ) -> Self {
        Self { object }
    }

    pub(in crate::context_bootstrap) fn into_value(self) -> v8::Local<'s, v8::Value> {
        self.object.into()
    }
}

pub(in crate::context_bootstrap) fn callback_carrier_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: Option<webidl::WebIdlCallbackFunction>,
) -> v8::Local<'s, v8::Value> {
    callback
        .map(|callback| StreamWebIdlCallbackCarrier::new(scope, callback).into_value())
        .unwrap_or_else(|| v8::undefined(scope).into())
}

pub(in crate::context_bootstrap) fn parse_readable_stream_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<WebIdlReadableStreamSource<'s>, webidl::WebIdlError> {
    let members = webidl::parse_dictionary_object::<UnderlyingSourceMembers>(scope, object)?;
    Ok(WebIdlReadableStreamSource {
        object,
        start: members.start,
        pull: members.pull,
        cancel: members.cancel,
        auto_allocate_chunk_size: members.auto_allocate_chunk_size,
        byte_stream: matches!(members.stream_type, Some(ReadableStreamType::Bytes)),
    })
}

pub(in crate::context_bootstrap) fn parse_writable_stream_sink_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<WebIdlWritableStreamSink<'s>, webidl::WebIdlError> {
    let context = webidl::Context::argument("WritableStream", 1);
    let members = webidl::parse_dictionary_object::<UnderlyingSinkMembers<'s>>(scope, object)?;
    if members.sink_type.is_some() {
        crate::util::throw_range_error(
            scope,
            "WritableStream underlyingSink.type is reserved and must be undefined.",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    Ok(WebIdlWritableStreamSink {
        object,
        start: members.start,
        write: members.write,
        close: members.close,
        abort: members.abort,
    })
}

pub(in crate::context_bootstrap) fn parse_transform_stream_transformer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<WebIdlTransformStreamTransformer<'s>, webidl::WebIdlError> {
    let context = webidl::Context::argument("TransformStream", 1);
    let members = webidl::parse_dictionary_object::<TransformerMembers<'s>>(scope, object)?;
    if members.readable_type.is_some() {
        crate::util::throw_range_error(
            scope,
            "TransformStream transformer.readableType is reserved and must be undefined.",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    if members.writable_type.is_some() {
        crate::util::throw_range_error(
            scope,
            "TransformStream transformer.writableType is reserved and must be undefined.",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    Ok(WebIdlTransformStreamTransformer {
        object,
        start: members.start,
        transform: members.transform,
        flush: members.flush,
        cancel: members.cancel,
    })
}

pub(in crate::context_bootstrap) fn parse_stream_strategy_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
    default_high_water_mark: f64,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    let context = webidl::Context::argument(prefix, (index + 1) as usize);
    let Some(object) = webidl::dictionary_arg(args, index, context)? else {
        return Ok(StreamQueuingStrategy {
            high_water_mark: default_high_water_mark,
            high_water_mark_provided: false,
            size: None,
        });
    };
    let members = webidl::parse_dictionary_object::<QueuingStrategyMembers>(scope, object)?;
    let high_water_mark = members.high_water_mark.unwrap_or(default_high_water_mark);
    if moli_streams::numeric::validate_high_water_mark(high_water_mark).is_err() {
        crate::util::throw_range_error(
            scope,
            "A queuing strategy's highWaterMark must be a nonnegative, non-NaN number",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    Ok(StreamQueuingStrategy {
        high_water_mark,
        high_water_mark_provided: members.high_water_mark.is_some(),
        size: members.size,
    })
}

/// Invokes one page-supplied callback through Web IDL while leaving exception,
/// return-value, Promise, and stream-state policy to the caller.
pub(in crate::context_bootstrap) fn invoke_stream_webidl_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: StreamWebIdlCallbackCarrier<'s>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    invoke_stream_webidl_callback_with(scope, callback, receiver, arguments, call_function_result)
}

fn invoke_stream_webidl_callback_with<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: StreamWebIdlCallbackCarrier<'s>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    invoke_function: impl FnOnce(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Function>,
        v8::Local<'s, v8::Value>,
        &[v8::Local<'s, v8::Value>],
    ) -> Result<R, v8::Local<'s, v8::Value>>,
) -> Result<R, v8::Local<'s, v8::Value>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let callback =
            V8TracedWindowWebIdlCallbackFunction::from_object(callback.object).prepare(scope, host);
        return match callback.invoke(scope, host, receiver, arguments, invoke_function) {
            PreparedWindowWebIdlCallbackFunctionOutcome::Returned(result) => Ok(result),
            PreparedWindowWebIdlCallbackFunctionOutcome::Failed(error) => Err(error),
            PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
                Err(stream_callback_no_longer_runnable(scope))
            }
        };
    }

    if crate::worker::get_worker_state(scope).is_none() {
        return Err(stream_callback_no_longer_runnable(scope));
    }
    let callback = V8TracedWebIdlCallbackFunction::from_object(callback.object).prepare(scope);
    invoke_webidl_callback_function(scope, &callback, receiver, arguments, invoke_function)
}

/// Dispatches either a page-supplied typed callback carrier or a
/// browser-created raw algorithm. The array/slot that owns `algorithm`
/// determines which representation is valid; arbitrary objects are an
/// internal-state violation rather than a second dynamic callback protocol.
pub(in crate::context_bootstrap) fn invoke_stored_stream_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    if algorithm.is_undefined() {
        return Ok(None);
    }
    if let Ok(function) = v8::Local::<v8::Function>::try_from(algorithm) {
        return call_function_result(scope, function, receiver, arguments);
    }
    let carrier = v8::Local::<v8::Object>::try_from(algorithm)
        .map(StreamWebIdlCallbackCarrier::from_object)
        .expect("a stored stream algorithm must be a browser function or typed callback carrier");
    invoke_stream_webidl_callback(scope, carrier, receiver, arguments)
}

/// Invokes a stored Promise-returning Streams algorithm.
///
/// A page-supplied Web IDL callback is converted to a Promise while its
/// relevant Realm is still entered. That matters for non-Promise return
/// values and thenables: the promise created by Web IDL belongs to the
/// callback Realm, while settlement and stream-state effects remain owned by
/// the caller. Browser-created algorithms retain their existing raw return
/// contract.
pub(in crate::context_bootstrap) fn invoke_stored_stream_promise_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    if algorithm.is_undefined() {
        return Ok(None);
    }
    if let Ok(function) = v8::Local::<v8::Function>::try_from(algorithm) {
        return call_function_result(scope, function, receiver, arguments);
    }
    let carrier = v8::Local::<v8::Object>::try_from(algorithm)
        .map(StreamWebIdlCallbackCarrier::from_object)
        .expect("a stored stream algorithm must be a browser function or typed callback carrier");
    invoke_stream_webidl_callback_with(
        scope,
        carrier,
        receiver,
        arguments,
        |scope, callback, receiver, arguments| {
            let result = match call_function_result(scope, callback, receiver, arguments) {
                Ok(result) => result.unwrap_or_else(|| v8::undefined(scope).into()),
                Err(error) => {
                    let Some(resolver) = v8::PromiseResolver::new(scope) else {
                        return Err(error);
                    };
                    let promise = resolver.get_promise(scope);
                    if resolver.reject(scope, error) == Some(true) {
                        return Ok(Some(promise.into()));
                    }
                    return Err(error);
                }
            };
            // Web IDL Promise<T> conversion creates a new capability even
            // when the callback returned a Promise. Resolving it observes
            // thenables and preserves the adoption boundary in this Realm.
            let Some(resolver) = v8::PromiseResolver::new(scope) else {
                return Err(v8::undefined(scope).into());
            };
            let promise = resolver.get_promise(scope);
            if resolver.resolve(scope, result) == Some(true) {
                Ok(Some(promise.into()))
            } else {
                Err(v8::undefined(scope).into())
            }
        },
    )
}

pub(in crate::context_bootstrap) fn stored_stream_algorithm_is_webidl(
    algorithm: v8::Local<'_, v8::Value>,
) -> bool {
    !algorithm.is_undefined() && v8::Local::<v8::Function>::try_from(algorithm).is_err()
}

/// Invokes a queuing-strategy callback and performs its Web IDL
/// `unrestricted double` return conversion before leaving the callback's
/// relevant Realm. Browser-created size algorithms retain their raw stream
/// representation and are converted by their existing owner.
pub(in crate::context_bootstrap) fn invoke_stored_stream_size_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Value>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    if algorithm.is_undefined() {
        return Ok(None);
    }
    let undefined = v8::undefined(scope);
    if let Ok(function) = v8::Local::<v8::Function>::try_from(algorithm) {
        return call_function_result(scope, function, undefined.into(), &[chunk]);
    }
    let carrier = v8::Local::<v8::Object>::try_from(algorithm)
        .map(StreamWebIdlCallbackCarrier::from_object)
        .expect("a stored size algorithm must be a browser function or typed callback carrier");
    invoke_stream_webidl_callback_with(
        scope,
        carrier,
        undefined.into(),
        &[chunk],
        |scope, callback, receiver, arguments| {
            let result = call_function_result(scope, callback, receiver, arguments)?
                .unwrap_or_else(|| v8::undefined(scope).into());
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let scope = try_catch.init();
            match result.number_value(&scope) {
                Some(number) => Ok(Some(v8::Number::new(&scope, number).into())),
                None => Err(scope
                    .exception()
                    .unwrap_or_else(|| v8::undefined(&scope).into())),
            }
        },
    )
}

fn stream_callback_no_longer_runnable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(
        scope,
        v8str(
            scope,
            "The provided Streams callback is no longer runnable.",
        ),
    )
}

use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(interface = "ReadableStream")]
struct ReadableStreamShellDeclaration {
    #[webapi(slot = READABLE_STREAM_QUEUE_SLOT, init = "array")]
    queue: (),
    #[webapi(slot = READABLE_STREAM_QUEUE_HEAD_SLOT, init = 0)]
    queue_head: (),
    #[webapi(slot = READABLE_STREAM_PENDING_READS_SLOT, init = "array")]
    pending_reads: (),
    #[webapi(slot = READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT, init = "array")]
    pending_closed_promises: (),
    #[webapi(slot = READABLE_STREAM_CLOSED_SLOT, init = false)]
    closed: (),
    #[webapi(slot = READABLE_STREAM_LOCKED_SLOT, init = false)]
    locked: (),
    #[webapi(slot = READABLE_STREAM_DISTURBED_SLOT, init = false)]
    disturbed: (),
    #[webapi(slot = READABLE_STREAM_HWM_SLOT, init = 0)]
    high_water_mark: (),
    #[webapi(slot = READABLE_STREAM_CONTROLLER_SLOT, init = "null")]
    controller: (),
    #[webapi(slot = READABLE_STREAM_BYTE_STREAM_SLOT, init = false)]
    byte_stream: (),
    #[webapi(slot = READABLE_STREAM_PULL_STATE_SLOT, init = 0)]
    pull_state: (),
    #[webapi(slot = READABLE_STREAM_PIPE_OWNER_SLOT, init = "null")]
    pipe_owner: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "WritableStream")]
struct WritableStreamShellDeclaration {
    #[webapi(slot = WRITABLE_STREAM_LOCKED_SLOT, init = false)]
    locked: (),
    #[webapi(slot = WRITABLE_STREAM_CLOSED_SLOT, init = false)]
    closed: (),
    #[webapi(slot = WRITABLE_STREAM_SINK_SLOT, init = "null")]
    sink: (),
    #[webapi(slot = WRITABLE_STREAM_CONTROLLER_SLOT, init = "null")]
    controller: (),
    #[webapi(slot = WRITABLE_STREAM_ALGORITHMS_SLOT, init = "null")]
    algorithms: (),
    #[webapi(slot = WRITABLE_STREAM_CURRENT_WRITER_SLOT, init = "undefined")]
    current_writer: (),
    #[webapi(slot = WRITABLE_STREAM_TARGET_READABLE_SLOT, init = "null")]
    target_readable: (),
    #[webapi(slot = WRITABLE_STREAM_TRANSFORMER_SLOT, init = "null")]
    transformer: (),
    #[webapi(slot = WRITABLE_STREAM_MODE_SLOT, init = "null")]
    mode: (),
    #[webapi(slot = WRITABLE_STREAM_STRATEGY_SLOT, init = "array")]
    strategy: (),
    #[webapi(slot = WRITABLE_STREAM_PIPE_OWNER_SLOT, init = "null")]
    pipe_owner: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "TransformStream")]
struct TransformStreamShellDeclaration {
    #[webapi(slot = TRANSFORM_STREAM_READABLE_SLOT, init = "null")]
    readable: (),
    #[webapi(slot = TRANSFORM_STREAM_WRITABLE_SLOT, init = "null")]
    writable: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "ReadableStream")]
struct ReadableStreamObjectDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_QUEUE_SLOT, init = "array")]
    queue: (),
    #[webapi(slot = READABLE_STREAM_QUEUE_HEAD_SLOT, init = 0)]
    queue_head: (),
    #[webapi(slot = READABLE_STREAM_PENDING_READS_SLOT, init = "array")]
    pending_reads: (),
    #[webapi(slot = READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT, init = "array")]
    pending_closed_promises: (),
    #[webapi(slot = READABLE_STREAM_CLOSED_SLOT, init = false)]
    closed: (),
    #[webapi(slot = READABLE_STREAM_LOCKED_SLOT, init = false)]
    locked: (),
    #[webapi(slot = READABLE_STREAM_DISTURBED_SLOT, init = false)]
    disturbed: (),
    #[webapi(slot = READABLE_STREAM_HWM_SLOT)]
    high_water_mark: f64,
    #[webapi(slot = READABLE_STREAM_CONTROLLER_SLOT)]
    controller: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_BYTE_STREAM_SLOT)]
    byte_stream: bool,
    #[webapi(slot = READABLE_STREAM_PULL_STATE_SLOT)]
    pull_state: f64,
    #[webapi(slot = READABLE_STREAM_PIPE_OWNER_SLOT, init = "null")]
    pipe_owner: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ReadableStreamStartRejectedDataDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_START_REJECTED_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ReadableStreamStartFulfilledDataDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_START_REJECTED_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_START_PULL_AFTER_START_SLOT)]
    pull_after_start: bool,
}

const READABLE_STREAM_START_REJECTED_STREAM_SLOT: &str = "__moliReadableStreamStartRejectedStream";
const READABLE_STREAM_START_PULL_AFTER_START_SLOT: &str = "__moliReadableStreamStartPullAfterStart";

#[derive(WebApiObject)]
#[webapi(
    interface = "WritableStream",
    scope_lifetime = 'scope,
)]
struct WritableStreamObjectDeclaration<'scope, 'value> {
    #[webapi(slot = WRITABLE_STREAM_LOCKED_SLOT)]
    locked: bool,
    #[webapi(slot = WRITABLE_STREAM_CLOSED_SLOT, init = false)]
    closed: (),
    #[webapi(slot = WRITABLE_STREAM_SINK_SLOT)]
    sink: v8::Local<'scope, v8::Value>,
    #[webapi(slot = WRITABLE_STREAM_CONTROLLER_SLOT)]
    controller: v8::Local<'scope, v8::Object>,
    #[webapi(slot = WRITABLE_STREAM_ALGORITHMS_SLOT)]
    algorithms: Option<v8::Local<'scope, v8::Array>>,
    #[webapi(slot = WRITABLE_STREAM_CURRENT_WRITER_SLOT, init = "undefined")]
    current_writer: (),
    #[webapi(slot = WRITABLE_STREAM_TARGET_READABLE_SLOT)]
    target_readable: v8::Local<'scope, v8::Value>,
    #[webapi(slot = WRITABLE_STREAM_TRANSFORMER_SLOT)]
    transformer: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = WRITABLE_STREAM_MODE_SLOT)]
    mode: Option<&'value str>,
    #[webapi(slot = WRITABLE_STREAM_STRATEGY_SLOT)]
    strategy: v8::Local<'scope, v8::Array>,
    #[webapi(slot = WRITABLE_STREAM_PIPE_OWNER_SLOT, init = "null")]
    pipe_owner: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TransformStream", unbranded)]
struct TransformStreamObjectDeclaration<'scope> {
    #[webapi(slot = TRANSFORM_STREAM_READABLE_SLOT)]
    readable: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TRANSFORM_STREAM_WRITABLE_SLOT)]
    writable: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap) fn initialize_transform_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    transformer: Option<v8::Local<'s, v8::Object>>,
    mode: Option<&str>,
    writable_high_water_mark: f64,
    writable_size_algorithm: Option<v8::Local<'s, v8::Function>>,
    readable_high_water_mark: f64,
    readable_size_algorithm: Option<v8::Local<'s, v8::Function>>,
) {
    let readable = ReadableStreamShellDeclaration::default()
        .bind(scope)
        .expect("ReadableStream shell declaration should bind");
    initialize_readable_stream_object(
        scope,
        readable,
        None,
        readable_high_water_mark,
        readable_size_algorithm,
    );
    let writable = WritableStreamShellDeclaration::default()
        .bind(scope)
        .expect("WritableStream shell declaration should bind");
    let controller = super::super::stream_objects::new_transform_stream_controller_object(
        scope, readable, writable,
    );
    let writable_size_algorithm = writable_size_algorithm
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    initialize_writable_stream_object_fields(
        scope,
        writable,
        None,
        controller,
        None,
        readable.into(),
        transformer,
        mode,
        writable_high_water_mark,
        writable_size_algorithm,
    );
    install_transform_stream_readable_pull_algorithm(scope, readable, writable);
    begin_writable_stream_start(scope, writable);
    let start_result = if let Some(transformer) = transformer
        && !transformer.is_null_or_undefined()
    {
        match call_named_method_result(scope, transformer, "start", &[controller.into()]) {
            Ok(result) => result,
            Err(error) => {
                scope.throw_exception(error);
                return;
            }
        }
    } else {
        None
    };
    set_transform_stream_start_result(scope, writable, readable, start_result);
    TransformStreamObjectDeclaration::new(readable, writable)
        .initialize(scope, stream)
        .expect("TransformStream declaration should initialize object");
    // Encoding wrappers share endpoint storage, but are separate interfaces.
    let interface = match mode {
        Some("text-encoder") => "TextEncoderStream",
        Some("text-decoder") => "TextDecoderStream",
        _ => "TransformStream",
    };
    moli_webapi_declare::initialize_web_api_object(scope, stream, interface)
        .expect("stream identity should initialize");
}

pub(in crate::context_bootstrap) fn initialize_webidl_transform_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    transformer: Option<WebIdlTransformStreamTransformer<'s>>,
    mode: Option<&str>,
    writable_high_water_mark: f64,
    writable_size_algorithm: Option<crate::webidl::WebIdlCallbackFunction>,
    readable_high_water_mark: f64,
    readable_size_algorithm: Option<crate::webidl::WebIdlCallbackFunction>,
) {
    let readable = ReadableStreamShellDeclaration::default()
        .bind(scope)
        .expect("ReadableStream shell declaration should bind");
    initialize_webidl_readable_stream_object(
        scope,
        readable,
        None,
        false,
        readable_high_water_mark,
        readable_size_algorithm,
    );
    let writable = WritableStreamShellDeclaration::default()
        .bind(scope)
        .expect("WritableStream shell declaration should bind");
    let controller = super::super::stream_objects::new_transform_stream_controller_object(
        scope, readable, writable,
    );
    let writable_size_algorithm = callback_carrier_value(scope, writable_size_algorithm);

    let (transformer_object, start, algorithms) = match transformer {
        Some(transformer) => {
            let WebIdlTransformStreamTransformer {
                object,
                start,
                transform,
                flush,
                cancel,
                ..
            } = transformer;
            let algorithms = new_writable_stream_algorithms(scope);
            set_callback_algorithm(
                scope,
                algorithms,
                WRITABLE_STREAM_ALGORITHM_TRANSFORM_INDEX,
                transform,
            );
            set_callback_algorithm(
                scope,
                algorithms,
                WRITABLE_STREAM_ALGORITHM_TRANSFORM_FLUSH_INDEX,
                flush,
            );
            set_callback_algorithm(
                scope,
                algorithms,
                WRITABLE_STREAM_ALGORITHM_TRANSFORM_CANCEL_INDEX,
                cancel,
            );
            (
                Some(object),
                start.map(|callback| StreamWebIdlCallbackCarrier::new(scope, callback)),
                Some(algorithms),
            )
        }
        None => (None, None, None),
    };
    initialize_writable_stream_object_fields(
        scope,
        writable,
        None,
        controller,
        algorithms,
        readable.into(),
        transformer_object,
        mode,
        writable_high_water_mark,
        writable_size_algorithm,
    );
    install_transform_stream_readable_pull_algorithm(scope, readable, writable);
    begin_writable_stream_start(scope, writable);
    let start_result = match (transformer_object, start) {
        (Some(transformer), Some(start)) => match invoke_stream_webidl_callback(
            scope,
            start,
            transformer.into(),
            &[controller.into()],
        ) {
            Ok(result) => result,
            Err(error) => {
                scope.throw_exception(error);
                return;
            }
        },
        _ => None,
    };
    set_transform_stream_start_result(scope, writable, readable, start_result);
    TransformStreamObjectDeclaration::new(readable, writable)
        .initialize(scope, stream)
        .expect("TransformStream declaration should initialize object");
    moli_webapi_declare::initialize_web_api_object(scope, stream, "TransformStream")
        .expect("TransformStream identity should initialize");
}

fn install_transform_stream_readable_pull_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
) {
    let Some(controller) = stream_slot_object(scope, readable, READABLE_STREAM_CONTROLLER_SLOT)
    else {
        return;
    };
    let Some(algorithms) = stream_slot_array(scope, controller, STREAM_CONTROLLER_ALGORITHMS_SLOT)
    else {
        return;
    };
    let StreamOwnerPublication::Published(pull_algorithm) = build_required_stream_callback(
        scope,
        v8::Function::builder(transform_stream_readable_pull_callback),
        "transform readable pull algorithm",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(cancel_algorithm) = build_required_stream_callback(
        scope,
        v8::Function::builder(transform_stream_readable_cancel_callback),
        "transform readable cancel algorithm",
    ) else {
        return;
    };
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_SOURCE_INDEX,
        writable.into(),
    );
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_PULL_INDEX,
        pull_algorithm.into(),
    );
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_CANCEL_INDEX,
        cancel_algorithm.into(),
    );
}

pub(in crate::context_bootstrap) fn new_readable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: Option<v8::Local<'s, v8::Object>>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
) -> v8::Local<'s, v8::Object> {
    let stream = new_readable_stream_shell_object(scope);
    initialize_readable_stream_object(scope, stream, source, high_water_mark, size_algorithm);
    stream
}

/// Allocates only the Web-visible ReadableStream identity and inert slots.
///
/// Structured-clone host-object decoding runs inside V8's
/// `DisallowJavascriptExecutionScope`, so controller setup and its required
/// start Promise reaction must be deferred until `read_value()` returns. No
/// author-observable stream operation may receive this shell before
/// `initialize_readable_stream_object` commits it.
pub(in crate::context_bootstrap) fn new_readable_stream_shell_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    ReadableStreamShellDeclaration::default()
        .bind(scope)
        .expect("ReadableStream shell declaration should bind")
}

pub(in crate::context_bootstrap) fn new_readable_byte_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let stream = new_readable_stream_shell_object(scope);
    initialize_webidl_readable_stream_object(scope, stream, None, true, 0.0, None);
    stream
}

pub(in crate::context_bootstrap) fn new_lazy_readable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
) -> v8::Local<'s, v8::Object> {
    new_lazy_readable_stream_object_with_kind(scope, source, high_water_mark, size_algorithm, false)
}

pub(in crate::context_bootstrap) fn new_lazy_readable_byte_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    new_lazy_readable_stream_object_with_kind(scope, source, 0.0, None, true)
}

fn new_lazy_readable_stream_object_with_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
    byte_stream: bool,
) -> v8::Local<'s, v8::Object> {
    let stream = new_readable_stream_shell_object(scope);
    let pull_algorithm = resolve_callable_property_result(scope, source, "pull")
        .ok()
        .flatten()
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let cancel_algorithm = resolve_callable_property_result(scope, source, "cancel")
        .ok()
        .flatten()
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let size_algorithm = size_algorithm
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    initialize_readable_stream_object_fields(
        scope,
        stream,
        Some(source),
        pull_algorithm,
        cancel_algorithm,
        high_water_mark,
        size_algorithm,
        byte_stream,
        None,
    );
    finish_browser_readable_stream_start(scope, stream, Some(source), false);
    stream
}

pub(in crate::context_bootstrap) fn new_writable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: Option<v8::Local<'s, v8::Object>>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
) -> v8::Local<'s, v8::Object> {
    let stream = new_writable_stream_shell_object(scope);
    initialize_writable_stream_object(scope, stream, sink, high_water_mark, size_algorithm);
    stream
}

/// Create only the Web-visible WritableStream identity. Structured-clone host
/// object decoding runs under V8's DisallowJavascriptExecutionScope, so
/// controller setup and start reactions must be deferred until ReadValue has
/// returned.
pub(in crate::context_bootstrap) fn new_writable_stream_shell_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    WritableStreamShellDeclaration::default()
        .bind(scope)
        .expect("WritableStream shell declaration should bind")
}

/// Create only the Web-visible TransformStream identity for deferred
/// structured-clone materialization.
pub(in crate::context_bootstrap) fn new_transform_stream_shell_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    TransformStreamShellDeclaration::default()
        .bind(scope)
        .expect("TransformStream shell declaration should bind")
}

pub(in crate::context_bootstrap) fn initialize_transform_stream_endpoints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
) {
    TransformStreamObjectDeclaration::new(readable, writable)
        .initialize(scope, stream)
        .expect("TransformStream endpoints should initialize");
}

pub(in crate::context_bootstrap) fn initialize_readable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    source: Option<v8::Local<'s, v8::Object>>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
) {
    let (pull_algorithm, cancel_algorithm) = match source {
        Some(source) => {
            let pull_algorithm = match resolve_callable_property_result(scope, source, "pull") {
                Ok(function) => function
                    .map(|function| function.into())
                    .unwrap_or_else(|| v8::undefined(scope).into()),
                Err(error) => {
                    scope.throw_exception(error);
                    return;
                }
            };
            let cancel_algorithm = match resolve_callable_property_result(scope, source, "cancel") {
                Ok(function) => function
                    .map(|function| function.into())
                    .unwrap_or_else(|| v8::undefined(scope).into()),
                Err(error) => {
                    scope.throw_exception(error);
                    return;
                }
            };
            (pull_algorithm, cancel_algorithm)
        }
        None => (v8::undefined(scope).into(), v8::undefined(scope).into()),
    };
    let size_algorithm = size_algorithm
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    initialize_readable_stream_object_fields(
        scope,
        stream,
        source,
        pull_algorithm,
        cancel_algorithm,
        high_water_mark,
        size_algorithm,
        false,
        None,
    );
    finish_browser_readable_stream_start(scope, stream, source, true);
}

pub(in crate::context_bootstrap) fn initialize_webidl_readable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    source: Option<WebIdlReadableStreamSource<'s>>,
    byte_stream: bool,
    high_water_mark: f64,
    size_algorithm: Option<crate::webidl::WebIdlCallbackFunction>,
) {
    let size_algorithm = callback_carrier_value(scope, size_algorithm);
    let Some(source) = source else {
        initialize_readable_stream_object_fields(
            scope,
            stream,
            None,
            v8::undefined(scope).into(),
            v8::undefined(scope).into(),
            high_water_mark,
            size_algorithm,
            byte_stream,
            None,
        );
        attach_readable_stream_start_reaction_handlers(
            scope,
            stream,
            v8::undefined(scope).into(),
            true,
        );
        return;
    };
    let WebIdlReadableStreamSource {
        object,
        auto_allocate_chunk_size,
        start,
        pull,
        cancel,
        ..
    } = source;
    let start = start.map(|callback| StreamWebIdlCallbackCarrier::new(scope, callback));
    let pull = callback_carrier_value(scope, pull);
    let cancel = callback_carrier_value(scope, cancel);
    initialize_readable_stream_object_fields(
        scope,
        stream,
        Some(object),
        pull,
        cancel,
        high_water_mark,
        size_algorithm,
        byte_stream,
        auto_allocate_chunk_size,
    );
    finish_webidl_readable_stream_start(scope, stream, object, start, true);
}

fn initialize_readable_stream_object_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    source: Option<v8::Local<'s, v8::Object>>,
    pull_algorithm: v8::Local<'s, v8::Value>,
    cancel_algorithm: v8::Local<'s, v8::Value>,
    high_water_mark: f64,
    size_algorithm: v8::Local<'s, v8::Value>,
    byte_stream: bool,
    auto_allocate_chunk_size: Option<u64>,
) {
    let source_value = source
        .map(|value| value.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let algorithms = v8::Array::new(scope, 4);
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_SOURCE_INDEX, source_value);
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_PULL_INDEX, pull_algorithm);
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_CANCEL_INDEX,
        cancel_algorithm,
    );
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_SIZE_INDEX, size_algorithm);
    let controller = super::super::stream_objects::new_readable_stream_controller_object(
        scope,
        stream,
        algorithms.into(),
        byte_stream,
    );
    ReadableStreamObjectDeclaration::new(high_water_mark, controller, byte_stream, 0.0)
        .initialize(scope, stream)
        .expect("ReadableStream declaration should initialize object");
    // Replace the declaration's realm Array with the internal queue shape
    // owned by stream_adapter::queue_v8 before any author callback can observe or
    // re-enter this stream.
    reset_readable_stream_queue(scope, stream);
    initialize_readable_byte_stream_state(scope, stream, auto_allocate_chunk_size);
}

fn finish_browser_readable_stream_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    source: Option<v8::Local<'s, v8::Object>>,
    pull_after_start: bool,
) {
    if let Some(source) = source {
        let Some(controller) = stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)
        else {
            return;
        };
        match call_named_method_result(scope, source, "start", &[controller.into()]) {
            Ok(Some(start_result)) => {
                attach_readable_stream_start_reaction_handlers(
                    scope,
                    stream,
                    start_result,
                    pull_after_start,
                );
            }
            Ok(None) if pull_after_start => {
                mark_readable_stream_started(scope, stream);
                maybe_pull_stream(scope, stream);
            }
            Ok(None) => {
                mark_readable_stream_started(scope, stream);
            }
            Err(error) => {
                scope.throw_exception(error);
            }
        }
    } else {
        // Internal streams use the same SetupReadableStreamDefaultController
        // start boundary as author-created streams. Even a trivial start
        // algorithm transitions [[started]] and considers the first pull from
        // a Promise reaction, after the owner has installed its algorithms.
        attach_readable_stream_start_reaction_handlers(
            scope,
            stream,
            v8::undefined(scope).into(),
            pull_after_start,
        );
    }
}

fn finish_webidl_readable_stream_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    start: Option<StreamWebIdlCallbackCarrier<'s>>,
    pull_after_start: bool,
) {
    let start_result = match start {
        Some(start) => {
            let Some(controller) =
                stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)
            else {
                return;
            };
            match invoke_stream_webidl_callback(scope, start, source.into(), &[controller.into()]) {
                Ok(result) => result.unwrap_or_else(|| v8::undefined(scope).into()),
                Err(error) => {
                    scope.throw_exception(error);
                    return;
                }
            }
        }
        None => v8::undefined(scope).into(),
    };
    // SetUpReadableStreamDefaultController and its byte-stream counterpart
    // always Promise-resolve startResult, including the implicit undefined
    // result when no start callback exists. Consequently [[started]] and the
    // first pull transition occur in a promise reaction, never inline during
    // construction or the first read.
    attach_readable_stream_start_reaction_handlers(scope, stream, start_result, pull_after_start);
}

fn attach_readable_stream_start_reaction_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    start_result: v8::Local<'s, v8::Value>,
    pull_after_start: bool,
) {
    let Some(start_promise) = readable_stream_start_result_promise(scope, start_result) else {
        return;
    };
    let fulfilled_data = ReadableStreamStartFulfilledDataDeclaration::new(stream, pull_after_start)
        .bind(scope)
        .expect("ReadableStream start fulfilled data should bind");
    let rejected_data = ReadableStreamStartRejectedDataDeclaration::new(stream)
        .bind(scope)
        .expect("ReadableStream start rejection data should bind");
    publish_required_stream_promise_reactions(
        scope,
        start_promise,
        v8::Function::builder(readable_stream_start_fulfilled_callback).data(fulfilled_data.into()),
        "readable start fulfillment",
        v8::Function::builder(readable_stream_start_rejected_callback).data(rejected_data.into()),
        "readable start rejection",
        "readable start",
    )
    .finish_at_owner_boundary();
}

fn readable_stream_start_result_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        return Some(promise);
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    if resolver.resolve(scope, value) != Some(true) {
        return None;
    }
    Some(promise)
}

fn readable_stream_start_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(stream) = get_private_value(scope, data, READABLE_STREAM_START_REJECTED_STREAM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    mark_readable_stream_started(scope, stream);
    if stream_slot_bool(scope, data, READABLE_STREAM_START_PULL_AFTER_START_SLOT).unwrap_or(false) {
        // SetUpReadableStreamDefaultController always performs
        // CallPullIfNeeded after [[started]] becomes true. A pipe owns a read
        // request, but it does not replace the controller's pull scheduling.
        maybe_pull_stream(scope, stream);
    }
    rv.set(v8::undefined(scope).into());
}

fn readable_stream_start_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(stream) = get_private_value(scope, data, READABLE_STREAM_START_REJECTED_STREAM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    error_stream(scope, stream, args.get(0));
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn initialize_writable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    sink: Option<v8::Local<'s, v8::Object>>,
    high_water_mark: f64,
    size_algorithm: Option<v8::Local<'s, v8::Function>>,
) {
    let controller =
        super::super::stream_objects::new_writable_stream_controller_object(scope, stream);
    let sink = sink.unwrap_or_else(|| v8::Object::new(scope));
    let size_algorithm = size_algorithm
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    initialize_writable_stream_object_fields(
        scope,
        stream,
        Some(sink),
        controller,
        None,
        v8::null(scope).into(),
        None,
        None,
        high_water_mark,
        size_algorithm,
    );
    let _ = call_named_method(scope, sink, "start", &[controller.into()]);
}

pub(in crate::context_bootstrap) fn initialize_webidl_writable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    sink: Option<WebIdlWritableStreamSink<'s>>,
    high_water_mark: f64,
    size_algorithm: Option<crate::webidl::WebIdlCallbackFunction>,
) {
    let controller =
        super::super::stream_objects::new_writable_stream_controller_object(scope, stream);
    let size_algorithm = callback_carrier_value(scope, size_algorithm);
    let Some(sink) = sink else {
        let sink = v8::Object::new(scope);
        let algorithms = new_writable_stream_algorithms(scope);
        initialize_writable_stream_object_fields(
            scope,
            stream,
            Some(sink),
            controller,
            Some(algorithms),
            v8::null(scope).into(),
            None,
            None,
            high_water_mark,
            size_algorithm,
        );
        begin_writable_stream_start(scope, stream);
        set_writable_stream_start_result(scope, stream, None);
        return;
    };
    let WebIdlWritableStreamSink {
        object,
        start,
        write,
        close,
        abort,
        ..
    } = sink;
    let algorithms = new_writable_stream_algorithms(scope);
    set_callback_algorithm(
        scope,
        algorithms,
        WRITABLE_STREAM_ALGORITHM_SINK_WRITE_INDEX,
        write,
    );
    set_callback_algorithm(
        scope,
        algorithms,
        WRITABLE_STREAM_ALGORITHM_SINK_CLOSE_INDEX,
        close,
    );
    set_callback_algorithm(
        scope,
        algorithms,
        WRITABLE_STREAM_ALGORITHM_SINK_ABORT_INDEX,
        abort,
    );
    let start = start.map(|callback| StreamWebIdlCallbackCarrier::new(scope, callback));
    initialize_writable_stream_object_fields(
        scope,
        stream,
        Some(object),
        controller,
        Some(algorithms),
        v8::null(scope).into(),
        None,
        None,
        high_water_mark,
        size_algorithm,
    );
    begin_writable_stream_start(scope, stream);
    let start_result = match start {
        Some(start) => {
            match invoke_stream_webidl_callback(scope, start, object.into(), &[controller.into()]) {
                Ok(result) => result,
                Err(error) => {
                    scope.throw_exception(error);
                    return;
                }
            }
        }
        None => None,
    };
    set_writable_stream_start_result(scope, stream, start_result);
}

fn initialize_writable_stream_object_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    sink: Option<v8::Local<'s, v8::Object>>,
    controller: v8::Local<'s, v8::Object>,
    algorithms: Option<v8::Local<'s, v8::Array>>,
    target_readable: v8::Local<'s, v8::Value>,
    transformer: Option<v8::Local<'s, v8::Object>>,
    mode: Option<&str>,
    high_water_mark: f64,
    size_algorithm: v8::Local<'s, v8::Value>,
) {
    let sink_value = sink
        .map(|value| value.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let strategy = v8::Array::new(scope, 13);
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_HIGH_WATER_MARK_INDEX,
        v8::Number::new(scope, high_water_mark).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_SIZE_ALGORITHM_INDEX,
        size_algorithm,
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX,
        v8::Number::new(scope, 0.0).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_STORED_ERROR_INDEX,
        v8::undefined(scope).into(),
    );
    let pending_writes = v8::Array::new(scope, 0);
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_WRITES_INDEX,
        pending_writes.into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_START_PENDING_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_ERRORED_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_QUEUE_PUMP_STATE_INDEX,
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_WRITES_HEAD_INDEX,
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_CLOSE_REQUESTED_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_ERRORING_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_ABORT_INDEX,
        v8::undefined(scope).into(),
    );
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_CLOSE_INDEX,
        v8::undefined(scope).into(),
    );
    WritableStreamObjectDeclaration::new(
        false,
        sink_value,
        controller,
        algorithms,
        target_readable,
        transformer,
        mode,
        strategy,
    )
    .initialize(scope, stream)
    .expect("WritableStream declaration should initialize object");
}

fn new_writable_stream_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Array> {
    let algorithms = v8::Array::new(scope, 6);
    for index in 0..6 {
        let _ = algorithms.set_index(scope, index, v8::undefined(scope).into());
    }
    algorithms
}

fn set_callback_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithms: v8::Local<'s, v8::Array>,
    index: u32,
    callback: Option<crate::webidl::WebIdlCallbackFunction>,
) {
    let callback = callback_carrier_value(scope, callback);
    let _ = algorithms.set_index(scope, index, callback);
}

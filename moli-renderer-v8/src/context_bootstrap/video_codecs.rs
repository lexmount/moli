//! WebCodecs frontend state without a video codec backend. Validity and WebIDL
//! conversion are implemented independently of support; all codecs currently
//! report unsupported and configure closes asynchronously with NotSupportedError.

use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

use super::{
    events::initialize_event_object, media_queries, new_dom_exception_value,
    throw_dom_exception_value,
};
use crate::{
    callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker},
    exception_reporting::CallbackExceptionLogLevel,
    host::report_event_callback_exception,
    util::{
        context_host_ptr_from_global_bridge, get_private_object, get_private_value,
        set_private_value, throw_type_error, v8str,
    },
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    web_api_interfaces, webidl,
};

mod config;

const STATE: &str = "__moliVideoCodecState";
const QUEUE_SIZE: &str = "__moliVideoCodecQueueSize";
const ERROR: &str = "__moliVideoCodecErrorCallback";
const OUTPUT: &str = "__moliVideoCodecOutputCallback";
const FLUSHES: &str = "__moliVideoCodecPendingFlushes";
const GENERATION: &str = "__moliVideoCodecGeneration";
const KEY_REQUIRED: &str = "__moliVideoCodecKeyRequired";
const ONDEQUEUE: &str = "__moliVideoCodecOndequeue";
const DEQUEUE_SCHEDULED: &str = "__moliVideoCodecDequeueScheduled";
const LISTENERS: &str = "__moliVideoCodecListeners";

#[derive(WebApiObject)]
#[webapi(plain)]
struct CodecSlots<'s> {
    #[webapi(slot = STATE, value = "unconfigured")]
    state: (),
    #[webapi(slot = QUEUE_SIZE, init = 0)]
    queue_size: (),
    #[webapi(slot = ERROR)]
    error: v8::Local<'s, v8::Object>,
    #[webapi(slot = OUTPUT)]
    output: v8::Local<'s, v8::Object>,
    #[webapi(slot = FLUSHES)]
    flushes: v8::Local<'s, v8::Array>,
    #[webapi(slot = GENERATION)]
    generation: v8::Local<'s, v8::Object>,
    #[webapi(slot = KEY_REQUIRED, init = true)]
    key_required: (),
    #[webapi(slot = ONDEQUEUE, init = "null")]
    ondequeue: (),
    #[webapi(slot = DEQUEUE_SCHEDULED, init = false)]
    dequeue_scheduled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoDecoder, enumerable, receiver)]
struct DecoderPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, STATE))]
    state: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, QUEUE_SIZE))]
    decode_queue_size: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, ONDEQUEUE))]
    ondequeue: (),
    #[webapi(method, length = 1, callback = configure_decoder)]
    configure: (),
    #[webapi(method, length = 1, callback = decode)]
    decode: (),
    #[webapi(method, length = 0, returns_promise, callback = flush)]
    flush: (),
    #[webapi(method, length = 0, callback = reset)]
    reset: (),
    #[webapi(method, length = 0, callback = close)]
    close: (),
    #[webapi(static_method, length = 1, returns_promise, callback = decoder_support)]
    is_config_supported: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoEncoder, enumerable, receiver)]
struct EncoderPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, STATE))]
    state: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, QUEUE_SIZE))]
    encode_queue_size: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, ONDEQUEUE))]
    ondequeue: (),
    #[webapi(method, length = 1, callback = configure_encoder)]
    configure: (),
    #[webapi(method, length = 1, callback = encode)]
    encode: (),
    #[webapi(method, length = 0, returns_promise, callback = flush)]
    flush: (),
    #[webapi(method, length = 0, callback = reset)]
    reset: (),
    #[webapi(method, length = 0, callback = close)]
    close: (),
    #[webapi(static_method, length = 1, returns_promise, callback = encoder_support)]
    is_config_supported: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    match name {
        "VideoDecoder" => {
            DecoderPrototype::initialize_template(scope, template);
            DecoderPrototype::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "VideoEncoder" => {
            EncoderPrototype::initialize_template(scope, template);
            EncoderPrototype::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        _ => (),
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoCodecInit")]
struct CodecInit {
    #[webidl(required, converter = "callback_function")]
    error: webidl::WebIdlCallbackFunction,
    #[webidl(required, converter = "callback_function")]
    output: webidl::WebIdlCallbackFunction,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoCodec")]
struct ConstructorArgs {
    #[webidl(required, dictionary)]
    init: CodecInit,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoDecoder config")]
struct DecoderArgs<'s> {
    #[webidl(required, dictionary)]
    config: config::DecoderConfig<'s>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoEncoder config")]
struct EncoderArgs {
    #[webidl(required, dictionary)]
    config: config::EncoderConfig,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoDecoder.decode")]
struct DecodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::EncodedVideoChunk)]
    chunk: v8::Local<'s, v8::Object>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoEncoder.encode")]
struct EncodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::VideoFrame)]
    frame: v8::Local<'s, v8::Object>,
    #[webidl(dictionary, default = config::EncodeOptions::default())]
    _options: config::EncodeOptions,
}

pub(super) fn decoder_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    constructor(scope, args, rv, "VideoDecoder");
}
pub(super) fn encoder_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    constructor(scope, args, rv, "VideoEncoder");
}
fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    name: &'static str,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "A VideoCodec constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), name) {
        return;
    }
    let error = V8TracedWebIdlCallbackFunction::new(scope, parsed.init.error).into_object();
    let output = V8TracedWebIdlCallbackFunction::new(scope, parsed.init.output).into_object();
    CodecSlots::new(
        error,
        output,
        v8::Array::new(scope, 0),
        v8::Object::new(scope),
    )
    .initialize(scope, args.this())
    .expect("VideoCodec slots");
    media_queries::mark_simple_event_target_slot(scope, args.this(), LISTENERS);
    media_queries::install_simple_event_target_ordered_handlers(scope, args.this());
    rv.set(args.this().into());
}
fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("validated VideoCodec receiver")
}
fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, target, &slot).expect("VideoCodec slot"));
}
fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, ONDEQUEUE, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, target, LISTENERS, "dequeue", ONDEQUEUE, active,
    );
}
fn has_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    state: &'static str,
) -> bool {
    get_private_value(scope, target, STATE)
        .expect("VideoCodec state")
        .strict_equals(v8str(scope, state).into())
}
fn require_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    configured: bool,
) -> bool {
    if has_state(scope, target, "closed") || (configured && !has_state(scope, target, "configured"))
    {
        throw_dom_exception_value(
            scope,
            "VideoCodec is not in the required state.",
            "InvalidStateError",
        );
        return false;
    }
    true
}
fn configure_decoder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DecoderArgs>(scope, &args) else {
        return;
    };
    if !parsed.config.is_valid(scope) {
        throw_type_error(scope, "Invalid VideoDecoderConfig.");
        return;
    }
    let target = target(scope, args.this());
    configure(scope, target);
}
fn configure_encoder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EncoderArgs>(scope, &args) else {
        return;
    };
    if !parsed.config.is_valid() {
        throw_type_error(scope, "Invalid VideoEncoderConfig.");
        return;
    }
    let target = target(scope, args.this());
    configure(scope, target);
}
fn configure<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    if !require_state(scope, target, false) {
        return;
    }
    set_private_value(scope, target, STATE, v8str(scope, "configured").into());
    set_private_value(
        scope,
        target,
        KEY_REQUIRED,
        v8::Boolean::new(scope, true).into(),
    );
    let generation = get_private_value(scope, target, GENERATION).expect("VideoCodec generation");
    let data = v8::Array::new_with_elements(scope, &[target.into(), generation]);
    let context = target
        .get_creation_context(scope)
        .expect("VideoCodec owner realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Function::builder(configuration_task)
        .data(data.into())
        .build(scope)
        .expect("VideoCodec control task");
    queue_task(scope, task);
}
fn queue_task<'s>(scope: &mut v8::PinScope<'s, '_>, task: v8::Local<'s, v8::Function>) {
    if crate::worker::queue_worker_codec_task(scope, task) {
        return;
    }
    let host =
        context_host_ptr_from_global_bridge(scope).expect("VideoCodec Window or worker task owner");
    unsafe { &mut *host }.queue_internal_task(scope, task);
}
fn configuration_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("VideoCodec control task data");
    let target = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("VideoCodec control target");
    let generation = data
        .get_index(scope, 1)
        .expect("VideoCodec control generation");
    if !get_private_value(scope, target, GENERATION)
        .expect("VideoCodec generation")
        .strict_equals(generation)
        || !has_state(scope, target, "configured")
    {
        return;
    }
    let callback = get_private_object(scope, target, ERROR).expect("VideoCodec error callback");
    let error = new_dom_exception_value(
        scope,
        "Video codec backends are not implemented.",
        "NotSupportedError",
    );
    reset_codec(scope, target, error);
    set_private_value(scope, target, STATE, v8str(scope, "closed").into());
    release_callbacks(scope, target);
    invoke_error_callback(scope, callback, error);
}
fn invoke_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    carrier: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    let relevant_context = callback.relevant_context(scope);
    let incumbent_context = callback.incumbent_context(scope);
    let callback = callback.callback(scope);
    let host = context_host_ptr_from_global_bridge(scope);
    let identity = host.and_then(|host| {
        unsafe { &*host }.window_execution_context_identity_for_access_check(relevant_context)
    });
    let arguments = [error];
    let invocation = CallbackInvocation::new(
        callback,
        v8::undefined(scope).into(),
        relevant_context,
        incumbent_context,
        true,
        "",
        &arguments,
        None,
    );
    let invocation = match host {
        Some(host) => invocation.with_execution_context_currentness(host, identity),
        None => invocation,
    };
    if let CallbackInvocationOutcome::Threw(report) = CallbackInvoker::invoke(
        scope,
        "WebCodecsErrorCallback",
        "VideoCodec error callback threw",
        CallbackExceptionLogLevel::Debug,
        "VideoCodec error callback",
        invocation,
    ) {
        if let Some(host) = host {
            report_event_callback_exception(scope, host, "codec", identity, None, &report);
        } else {
            crate::worker::dispatch_current_worker_callback_exception(scope, *report);
        }
    }
}
fn decode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DecodeArgs>(scope, &args) else {
        return;
    };
    let target = target(scope, args.this());
    if !require_state(scope, target, true) {
        return;
    }
    if get_private_value(scope, target, KEY_REQUIRED)
        .expect("VideoCodec key required")
        .is_true()
    {
        if !super::encoded_video_chunk::is_key_chunk(scope, parsed.chunk) {
            throw_dom_exception_value(scope, "A key chunk is required.", "DataError");
            return;
        }
        set_private_value(
            scope,
            target,
            KEY_REQUIRED,
            v8::Boolean::new(scope, false).into(),
        );
    }
    increment_queue(scope, target);
}
fn encode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EncodeArgs>(scope, &args) else {
        return;
    };
    if super::video_frame::is_closed(scope, parsed.frame) {
        throw_type_error(scope, "The VideoFrame is closed.");
        return;
    }
    let target = target(scope, args.this());
    if require_state(scope, target, true) {
        increment_queue(scope, target);
    }
}
fn increment_queue<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    let size = get_private_value(scope, target, QUEUE_SIZE)
        .expect("VideoCodec queue size")
        .uint32_value(scope)
        .expect("VideoCodec queue size number");
    set_private_value(
        scope,
        target,
        QUEUE_SIZE,
        v8::Integer::new_from_unsigned(scope, size.saturating_add(1)).into(),
    );
}
fn flush<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    if !require_state(scope, target, true) {
        return;
    }
    let resolver = v8::PromiseResolver::new(scope).expect("VideoCodec flush resolver");
    let flushes = get_private_value(scope, target, FLUSHES)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("VideoCodec pending flushes");
    moli_webapi_declare::define_array_data_property(
        scope,
        flushes,
        flushes.length(),
        resolver.into(),
    )
    .expect("VideoCodec pending flush array");
    set_private_value(
        scope,
        target,
        KEY_REQUIRED,
        v8::Boolean::new(scope, true).into(),
    );
    rv.set(resolver.get_promise(scope).into());
}
fn reset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    if !require_state(scope, target, false) {
        return;
    }
    let reason = new_dom_exception_value(scope, "VideoCodec was reset.", "AbortError");
    reset_codec(scope, target, reason);
}
fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    if !require_state(scope, target, false) {
        return;
    }
    let reason = new_dom_exception_value(scope, "VideoCodec was closed.", "AbortError");
    reset_codec(scope, target, reason);
    set_private_value(scope, target, STATE, v8str(scope, "closed").into());
    release_callbacks(scope, target);
}
fn reset_codec<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, target, STATE, v8str(scope, "unconfigured").into());
    set_private_value(scope, target, GENERATION, v8::Object::new(scope).into());
    let flushes = get_private_value(scope, target, FLUSHES)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("VideoCodec pending flushes");
    set_private_value(scope, target, FLUSHES, v8::Array::new(scope, 0).into());
    for index in 0..flushes.length() {
        let resolver = flushes
            .get_index(scope, index)
            .map(private_resolver)
            .expect("VideoCodec flush resolver");
        let _ = resolver.reject(scope, reason);
    }
    let old_size = get_private_value(scope, target, QUEUE_SIZE)
        .expect("VideoCodec queue size")
        .uint32_value(scope)
        .expect("VideoCodec queue size number");
    set_private_value(scope, target, QUEUE_SIZE, v8::Integer::new(scope, 0).into());
    if old_size != 0 {
        schedule_dequeue(scope, target);
    }
}
fn release_callbacks<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    set_private_value(scope, target, ERROR, v8::null(scope).into());
    set_private_value(scope, target, OUTPUT, v8::null(scope).into());
}
fn schedule_dequeue<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    if get_private_value(scope, target, DEQUEUE_SCHEDULED)
        .expect("VideoCodec dequeue scheduled")
        .is_true()
    {
        return;
    }
    set_private_value(
        scope,
        target,
        DEQUEUE_SCHEDULED,
        v8::Boolean::new(scope, true).into(),
    );
    let context = target
        .get_creation_context(scope)
        .expect("VideoCodec owner realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Function::builder(dequeue_task)
        .data(target.into())
        .build(scope)
        .expect("VideoCodec dequeue task");
    queue_task(scope, task);
}
fn dequeue_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = v8::Local::<v8::Object>::try_from(args.data()).expect("VideoCodec dequeue target");

    let event = v8::Object::new(scope);
    let prototype =
        super::ensure_intrinsic_interface_prototype(scope, "Event").expect("Event prototype");
    let _ = event.set_prototype(scope, prototype.into());
    initialize_event_object(scope, event, "dequeue", false, false);
    super::mark_event_trusted(scope, event);
    media_queries::dispatch_simple_event_target_event(scope, target, LISTENERS, "dequeue", event);
    set_private_value(
        scope,
        target,
        DEQUEUE_SCHEDULED,
        v8::Boolean::new(scope, false).into(),
    );
}

#[derive(WebApiObject)]
#[webapi(plain, enumerable)]
struct CodecSupport<'s> {
    #[webapi(data_property)]
    config: v8::Local<'s, v8::Object>,
    #[webapi(data_property, init = false)]
    supported: (),
}
fn decoder_support<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DecoderArgs>(scope, &args) else {
        return;
    };
    if !parsed.config.is_valid(scope) {
        throw_type_error(scope, "Invalid VideoDecoderConfig.");
        return;
    }
    let config = parsed
        .config
        .bind(scope)
        .expect("VideoDecoder config snapshot");
    support(scope, config, rv);
}
fn encoder_support<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EncoderArgs>(scope, &args) else {
        return;
    };
    if !parsed.config.is_valid() {
        throw_type_error(scope, "Invalid VideoEncoderConfig.");
        return;
    }
    let config = parsed
        .config
        .bind(scope)
        .expect("VideoEncoder config snapshot");
    support(scope, config, rv);
}
fn support<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let support = CodecSupport::new(config)
        .bind(scope)
        .expect("VideoCodec support dictionary");
    let resolver = v8::PromiseResolver::new(scope).expect("VideoCodec support resolver");
    let data = v8::Array::new_with_elements(scope, &[resolver.into(), support.into()]);
    let task = v8::Function::builder(support_task)
        .data(data.into())
        .build(scope)
        .expect("VideoCodec support task");
    queue_task(scope, task);
    rv.set(resolver.get_promise(scope).into());
}
fn support_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("VideoCodec support task data");
    let resolver = data
        .get_index(scope, 0)
        .map(private_resolver)
        .expect("VideoCodec support resolver");
    let support = data.get_index(scope, 1).expect("VideoCodec support record");
    let _ = resolver.resolve(scope, support);
}

fn private_resolver<'s>(value: v8::Local<'s, v8::Value>) -> v8::Local<'s, v8::PromiseResolver> {
    let object = v8::Local::<v8::Object>::try_from(value).expect("private codec resolver object");
    // SAFETY: only PromiseResolver::new values enter these private task/codec
    // records. Neither author properties nor author Proxy targets are read.
    unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) }
}

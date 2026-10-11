//! Native encoded-transform frontend. An encoder/depacketizer backend must
//! supply frames; this module never manufactures a frame or keyframe success.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    context_bootstrap::{
        events, exposed_interfaces, media_queries, shared, stream_adapter, worker_host,
    },
    native_bridge::{abort::dom_exception_value, throw_dom_exception},
    util::{get_private_object, get_private_value, set_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
    worker::{
        WorkerMessage,
        rtp_tasks::{RtpTransformTask, TransformLease, TransformerEntry},
    },
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

pub(super) const TRANSFORM: &str = "__moliRtpTransform";
pub(super) const SFRAME: &str = "__moliRtpUseSFrame";
const WORKER: &str = "__moliRtpTransformWorker";
const ID: &str = "__moliRtpTransformId";
const OWNER: &str = "__moliRtpTransformOwner";
const READABLE: &str = "__moliRtpTransformReadable";
const WRITABLE: &str = "__moliRtpTransformWritable";
const OPTIONS: &str = "__moliRtpTransformOptions";
const ASSOCIATED: &str = "__moliRtpTransformAssociated";
const HANDLER: &str = "__moliRtpKeyFrameRequestHandler";
const EVENT_TRANSFORMER: &str = "__moliRtcEventTransformer";
const RID: &str = "__moliRtcKeyFrameRequestRid";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "RTCRtpScriptTransformType", rename_all = "lowercase")]
enum TransformType {
    Sframe,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WorkerAndParameters")]
struct WorkerAndParameters<'s> {
    #[webidl(name = "type", converter = "enum")]
    transform_type: Option<TransformType>,
    #[webidl(required, interface = web_api_interfaces::Worker)]
    worker: v8::Local<'s, v8::Object>,
}

struct WorkerArgument<'s> {
    worker: v8::Local<'s, v8::Object>,
    sframe: bool,
}

impl<'s> webidl::WebIdlConverter<'s> for WorkerArgument<'s> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::Worker::is_instance(scope, object)
        {
            return Ok(Self {
                worker: object,
                sframe: false,
            });
        }
        let parameters =
            <webidl::Dictionary<WorkerAndParameters>>::convert(scope, value, context, &())?.0;
        Ok(Self {
            worker: parameters.worker,
            sframe: parameters.transform_type.is_some(),
        })
    }
}

struct TransferObject<'s>(v8::Local<'s, v8::Object>);
impl<'s> webidl::WebIdlConverter<'s> for TransferObject<'s> {
    type Options = ();
    fn convert(
        _: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        v8::Local::<v8::Object>::try_from(value)
            .map(Self)
            .map_err(|_| webidl::WebIdlError::cannot_convert(context, "object"))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpScriptTransform")]
struct ConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    worker: WorkerArgument<'s>,
    #[webidl(converter = "raw", default = v8::undefined(scope).into())]
    options: v8::Local<'s, v8::Value>,
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    transfer: Vec<TransferObject<'s>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCRtpScriptTransform)]
struct TransformSlots<'s> {
    #[webapi(slot = WORKER)]
    worker: v8::Local<'s, v8::Object>,
    #[webapi(slot = ID)]
    id: v8::Local<'s, v8::BigInt>,
    #[webapi(slot = SFRAME)]
    sframe: bool,
    #[webapi(slot = OWNER, init = "null")]
    owner: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCRtpScriptTransformer)]
struct TransformerSlots<'s> {
    #[webapi(slot = READABLE)]
    readable: v8::Local<'s, v8::Object>,
    #[webapi(slot = WRITABLE)]
    writable: v8::Local<'s, v8::Object>,
    #[webapi(slot = OPTIONS)]
    options: v8::Local<'s, v8::Value>,
    #[webapi(slot = crate::context_bootstrap::shared::SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    listeners: (),
    #[webapi(slot = ASSOCIATED, init = false)]
    associated: (),
    #[webapi(slot = HANDLER, init = "null")]
    handler: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCTransformEvent)]
struct TransformEventSlots<'s> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = EVENT_TRANSFORMER)]
    transformer: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::KeyFrameRequestEvent)]
struct KeyFrameEventSlots<'s> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = RID)]
    rid: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpScriptTransformer, enumerable, receiver)]
struct TransformerPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, READABLE))]
    readable: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, WRITABLE))]
    writable: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, OPTIONS))]
    options: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler)]
    onkeyframerequest: (),
    #[webapi(method, length = 0, returns_promise, callback = generate_keyframe)]
    generate_key_frame: (),
    #[webapi(method, length = 0, returns_promise, callback = send_keyframe_request)]
    send_key_frame_request: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCTransformEvent, enumerable, receiver)]
struct TransformEventPrototype {
    #[webapi(accessor_property, getter = event_slot_getter, data = v8str(scope, EVENT_TRANSFORMER))]
    transformer: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::KeyFrameRequestEvent, enumerable, receiver)]
struct KeyFrameEventPrototype {
    #[webapi(accessor_property, getter = event_slot_getter, data = v8str(scope, RID))]
    rid: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct EncodedFrameSink {
    #[webapi(method, length = 1, callback = write_frame)]
    write: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    match name {
        "RTCRtpScriptTransformer" => {
            TransformerPrototype::initialize_prototype_template(scope, prototype)
        }
        "RTCTransformEvent" => {
            TransformEventPrototype::initialize_prototype_template(scope, prototype)
        }
        "KeyFrameRequestEvent" => {
            KeyFrameEventPrototype::initialize_prototype_template(scope, prototype)
        }
        _ => (),
    }
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCRtpScriptTransform requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "RTCRtpScriptTransform") {
        return;
    }
    let objects = parsed.transfer.into_iter().map(|o| o.0).collect();
    let Some(options) = shared::structured_serialize_with_transfer_objects(
        scope,
        parsed.options,
        objects,
        "RTCRtpScriptTransform",
    ) else {
        return;
    };
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    TransformSlots {
        worker: parsed.worker.worker,
        id: v8::BigInt::new_from_u64(scope, id),
        sframe: parsed.worker.sframe,
        owner: (),
    }
    .initialize(scope, args.this())
    .expect("RTP script transform slots");
    let lease = TransformLease::new(id);
    let finalizer = lease.clone();
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, args.this(), move || {
        finalizer.retire()
    });
    worker_host::queue_worker_task(
        scope,
        parsed.worker.worker,
        WorkerMessage::RtpTransform(RtpTransformTask::Create {
            id,
            options: Box::new(options),
            lease,
        }),
    );
    rv.set(args.this().into());
}

fn id<'s>(scope: &mut v8::PinScope<'s, '_>, transform: v8::Local<'s, v8::Object>) -> u64 {
    v8::Local::<v8::BigInt>::try_from(
        get_private_value(scope, transform, ID).expect("native transform ID"),
    )
    .expect("transform ID is a BigInt")
    .u64_value()
    .0
}

fn queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transform: v8::Local<'s, v8::Object>,
    task: RtpTransformTask,
) {
    let worker = get_private_object(scope, transform, WORKER).expect("native transform Worker");
    worker_host::queue_worker_task(scope, worker, WorkerMessage::RtpTransform(task));
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpSender.transform")]
struct SetterArgs<'s> {
    #[webidl(required, nullable, interface = web_api_interfaces::RTCRtpScriptTransform)]
    transform: Option<v8::Local<'s, v8::Object>>,
}

pub(super) fn get_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let target = super::rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, target, TRANSFORM).expect("RTP endpoint transform"));
}

pub(super) fn set_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<SetterArgs>(scope, &args) else {
        return;
    };
    let target = super::rtp_transceivers::target(scope, args.this());
    let old = get_private_value(scope, target, TRANSFORM).expect("RTP endpoint transform");
    let value = parsed
        .transform
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    if old.strict_equals(value) {
        return;
    }
    let transform = parsed.transform.map(|t| {
        moli_webapi_declare::web_api_object_target(scope, t).expect("native script transform")
    });
    let transceiver =
        get_private_object(scope, target, super::rtp_transceivers::ENDPOINT_TRANSCEIVER)
            .expect("RTP endpoint transceiver");
    let use_sframe = transform.is_some_and(|t| {
        get_private_value(scope, t, SFRAME)
            .expect("transform mode")
            .boolean_value(scope)
    });
    // The shared private-slot reader represents an undefined value as None.
    if get_private_value(scope, transceiver, SFRAME)
        .is_some_and(|mode| mode.boolean_value(scope) != use_sframe)
    {
        throw_dom_exception(
            scope,
            "InvalidModificationError",
            13,
            "The transform's SFrame mode differs from the transceiver.",
        );
        return;
    }
    set_private_value(
        scope,
        transceiver,
        SFRAME,
        v8::Boolean::new(scope, use_sframe).into(),
    );
    if let Some(transform) = transform {
        if !get_private_value(scope, transform, OWNER)
            .expect("transform owner")
            .is_null()
        {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The transform has already been used.",
            );
            return;
        }
        set_private_value(scope, transform, OWNER, target.into());
    }
    if let Ok(previous) = v8::Local::<v8::Object>::try_from(old) {
        let previous = moli_webapi_declare::web_api_object_target(scope, previous)
            .expect("native previous script transform");
        let id = id(scope, previous);
        queue(scope, previous, RtpTransformTask::Disassociate { id });
    }
    set_private_value(scope, target, TRANSFORM, value);
    if let Some(transform) = transform {
        let id = id(scope, transform);
        queue(scope, transform, RtpTransformTask::Associate { id });
    }
}

pub(super) fn initialize_sframe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transceiver: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, transceiver, SFRAME).is_none() {
        set_private_value(
            scope,
            transceiver,
            SFRAME,
            v8::Boolean::new(scope, false).into(),
        );
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let object =
        moli_webapi_declare::web_api_object_target(scope, args.this()).expect("native transformer");
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).unwrap_or_else(|| v8::undefined(scope).into()));
}
fn event_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = events::event_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}
fn handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let object =
        moli_webapi_declare::web_api_object_target(scope, args.this()).expect("native transformer");
    rv.set(get_private_value(scope, object, HANDLER).expect("transformer event handler"));
}
fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'s>,
) {
    let object =
        moli_webapi_declare::web_api_object_target(scope, args.this()).expect("native transformer");
    media_queries::simple_object_event_set_handler(scope, object, "keyframerequest", HANDLER, args.get(0));
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpScriptTransformer.generateKeyFrame")]
struct KeyFrameArgs {
    #[webidl(converter = "raw")]
    _rid: Option<webidl::DomString16>,
}
fn generate_keyframe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if webidl::parse_args::<KeyFrameArgs>(scope, &args).is_some() {
        rv.set(no_frame_source(scope).into());
    }
}
fn send_keyframe_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    rv.set(no_frame_source(scope).into());
}
fn no_frame_source<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Promise> {
    // No native encoder or depacketizer exists yet. In particular, association
    // alone does not turn a local sender record into an active frame source.
    let resolver = v8::PromiseResolver::new(scope).expect("keyframe promise");
    let error = dom_exception_value(
        scope,
        "No active encoded frame source is associated.",
        "InvalidStateError",
    );
    resolver.reject(scope, error);
    resolver.get_promise(scope)
}

fn write_frame<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'s>,
) {
    let valid = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .is_some_and(|object| {
            web_api_interfaces::RTCEncodedAudioFrame::is_instance(scope, object)
                || web_api_interfaces::RTCEncodedVideoFrame::is_instance(scope, object)
        });
    if !valid {
        throw_type_error(scope, "The chunk must be an encoded audio or video frame.");
    }
    // Without an encoder/depacketizer there are no frames owned by this
    // pipeline. writeEncodedData silently drops frames with another owner.
}

fn stop_streams<'s>(scope: &mut v8::PinScope<'s, '_>, transformer: v8::Local<'s, v8::Object>) {
    let readable = get_private_object(scope, transformer, READABLE).expect("transform readable");
    let writable = get_private_object(scope, transformer, WRITABLE).expect("transform writable");
    let reason = v8::undefined(scope).into();
    for result in [
        stream_adapter::cancel_readable_stream(scope, readable, reason),
        stream_adapter::writable_stream_abort_internal(scope, writable, reason),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
            promise.mark_as_handled();
        }
    }
    set_private_value(
        scope,
        transformer,
        ASSOCIATED,
        v8::Boolean::new(scope, false).into(),
    );
}

/// Selected on the owning Worker's loop, after classic/module bootstrap and
/// the common termination check. Only Create returns a trusted global event.
pub(crate) fn run_worker_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: RtpTransformTask,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = crate::worker::get_worker_state(scope)?;
    match task {
        RtpTransformTask::Create { id, options, lease } => {
            let options = shared::structured_deserialize_value(scope, &options)?;
            let sink = EncodedFrameSink::default().bind(scope).ok()?;
            if sink.set_prototype(scope, v8::null(scope).into()) != Some(true) {
                return None;
            }
            let readable = stream_adapter::new_readable_stream_object(scope, None, 1.0, None);
            let writable =
                stream_adapter::new_writable_stream_object(scope, Some(sink), f64::INFINITY, None);
            let transformer = TransformerSlots {
                readable,
                writable,
                options,
                listeners: (),
                associated: (),
                handler: (),
            }
            .bind(scope)
            .ok()?;
            let wake = state.borrow().worker_wake_tx.clone();
            lease.bind(wake);
            state.borrow_mut().rtp_transformers.insert(
                id,
                TransformerEntry {
                    transformer: v8::Global::new(scope, transformer),
                    lease,
                },
            );
            let event_state = events::new_event_state(scope);
            events::initialize_event_object(scope, event_state, "rtctransform", false, false);
            TransformEventSlots {
                composed: false,
                transformer,
            }
            .initialize(scope, event_state)
            .ok()?;
            events::mark_event_trusted(scope, event_state);
            let event =
                exposed_interfaces::build_intrinsic_interface_instance(scope, "RTCTransformEvent")
                    .ok()?;
            let prototype = exposed_interfaces::ensure_intrinsic_interface_prototype(
                scope,
                "RTCTransformEvent",
            )
            .ok()?;
            if event.set_prototype(scope, prototype.into()) != Some(true) {
                return None;
            }
            events::initialize_event_wrapper(scope, event, event_state)?;
            // A constructor's task still fires once if its Window wrapper was
            // collected before delivery; Retire runs behind this selected task.
            if !state.borrow().rtp_transformers.get(&id)?.lease.is_alive() {
                let wake = state.borrow().worker_wake_tx.clone();
                let _ = wake.send(WorkerMessage::RtpTransform(RtpTransformTask::Retire { id }));
            }
            Some(event)
        }
        RtpTransformTask::Associate { id } => {
            let transformer = state
                .borrow()
                .rtp_transformers
                .get(&id)
                .filter(|entry| entry.lease.is_alive())
                .map(|entry| v8::Local::new(scope, &entry.transformer));
            if let Some(transformer) = transformer {
                set_private_value(
                    scope,
                    transformer,
                    ASSOCIATED,
                    v8::Boolean::new(scope, true).into(),
                );
            }
            None
        }
        RtpTransformTask::Disassociate { id } => {
            let transformer = state
                .borrow()
                .rtp_transformers
                .get(&id)
                .map(|entry| v8::Local::new(scope, &entry.transformer));
            if let Some(transformer) = transformer {
                stop_streams(scope, transformer);
            }
            None
        }
        RtpTransformTask::Retire { id } => {
            let entry = state.borrow_mut().rtp_transformers.remove(&id);
            if let Some(entry) = entry {
                let transformer = v8::Local::new(scope, &entry.transformer);
                stop_streams(scope, transformer);
            }
            None
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "KeyFrameRequestEvent")]
struct KeyFrameEventArgs {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(converter = "raw")]
    rid: Option<webidl::DomString16>,
}

pub(in crate::context_bootstrap) fn keyframe_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "KeyFrameRequestEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<KeyFrameEventArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "KeyFrameRequestEvent") {
        return;
    }
    let event_type =
        v8::String::new_from_two_byte(scope, &parsed.event_type.0, v8::NewStringType::Normal)
            .expect("event type");
    let rid = parsed
        .rid
        .map(|rid| {
            v8::String::new_from_two_byte(scope, &rid.0, v8::NewStringType::Normal)
                .expect("keyframe RID")
                .into()
        })
        .unwrap_or_else(|| v8::null(scope).into());
    let state = events::new_event_state(scope);
    events::initialize_event_object_with_type(scope, state, event_type, false, false);
    if (KeyFrameEventSlots {
        composed: false,
        rid,
    })
    .initialize(scope, state)
    .is_ok()
        && events::initialize_event_wrapper(scope, args.this(), state).is_some()
    {
        rv.set(args.this().into());
    }
}

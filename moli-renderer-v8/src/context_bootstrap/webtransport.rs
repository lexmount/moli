//! Native WebTransport frontend. The initial streams, WebIDL conversion and
//! failure lifecycle are real; no HTTP/3 or HTTP/2 session backend is available.

use crate::{
    util::{
        context_host_ptr_from_global_bridge, get_private_object, get_private_value,
        set_private_value, throw_range_error, throw_type_error, v8str,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

mod datagrams;
mod schema;
mod tasks;

const STATE: &str = "__moliWebTransportState";
const READY: &str = "__moliWebTransportReady";
const CLOSED: &str = "__moliWebTransportClosed";
const DRAINING: &str = "__moliWebTransportDraining";
const READY_RESOLVER: &str = "__moliWebTransportReadyResolver";
const CLOSED_RESOLVER: &str = "__moliWebTransportClosedResolver";
const DATAGRAMS: &str = "__moliWebTransportDatagrams";
const INCOMING_BIDI: &str = "__moliWebTransportIncomingBidirectionalStreams";
const INCOMING_UNI: &str = "__moliWebTransportIncomingUnidirectionalStreams";
const LIMIT_BIDI: &str = "__moliWebTransportAnticipatedIncomingBidi";
const LIMIT_UNI: &str = "__moliWebTransportAnticipatedIncomingUni";
const PENDING: &str = "__moliWebTransportPendingOperations";
const OWNER: &str = "__moliWebTransportOwner";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransport)]
struct Session<'s> {
    #[webapi(slot = STATE, init = string("connecting"))]
    state: (),
    #[webapi(slot = READY)]
    ready: v8::Local<'s, v8::Value>,
    #[webapi(slot = CLOSED)]
    closed: v8::Local<'s, v8::Value>,
    #[webapi(slot = DRAINING)]
    draining: v8::Local<'s, v8::Value>,
    #[webapi(slot = READY_RESOLVER)]
    ready_resolver: v8::Local<'s, v8::Value>,
    #[webapi(slot = CLOSED_RESOLVER)]
    closed_resolver: v8::Local<'s, v8::Value>,
    #[webapi(slot = DATAGRAMS)]
    datagrams: v8::Local<'s, v8::Object>,
    #[webapi(slot = INCOMING_BIDI)]
    incoming_bidi: v8::Local<'s, v8::Object>,
    #[webapi(slot = INCOMING_UNI)]
    incoming_uni: v8::Local<'s, v8::Object>,
    #[webapi(slot = LIMIT_BIDI)]
    limit_bidi: v8::Local<'s, v8::Value>,
    #[webapi(slot = LIMIT_UNI)]
    limit_uni: v8::Local<'s, v8::Value>,
    #[webapi(slot = PENDING, init = "array")]
    pending: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransport, enumerable, receiver)]
struct Prototype {
    #[webapi(method, returns_promise, length = 0, callback = get_stats)]
    get_stats: (),
    #[webapi(method, returns_promise, length = 3, callback = export_keying_material)]
    export_keying_material: (),
    #[webapi(accessor_property, returns_promise, getter = slot_getter, data = v8str(scope, READY))]
    ready: (),
    #[webapi(accessor_property, getter = reliability)]
    reliability: (),
    #[webapi(accessor_property, getter = congestion_control)]
    congestion_control: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_limit, data = v8str(scope, LIMIT_UNI))]
    anticipated_concurrent_incoming_unidirectional_streams: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_limit, data = v8str(scope, LIMIT_BIDI))]
    anticipated_concurrent_incoming_bidirectional_streams: (),
    #[webapi(accessor_property, getter = response_headers)]
    response_headers: (),
    #[webapi(accessor_property, getter = protocol)]
    protocol: (),
    #[webapi(accessor_property, returns_promise, getter = slot_getter, data = v8str(scope, CLOSED))]
    closed: (),
    #[webapi(accessor_property, returns_promise, getter = slot_getter, data = v8str(scope, DRAINING))]
    draining: (),
    #[webapi(method, length = 0, callback = close)]
    close: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, DATAGRAMS))]
    datagrams: (),
    #[webapi(method, returns_promise, length = 0, callback = create_stream)]
    create_bidirectional_stream: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, INCOMING_BIDI))]
    incoming_bidirectional_streams: (),
    #[webapi(method, returns_promise, length = 0, callback = create_stream)]
    create_unidirectional_stream: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, INCOMING_UNI))]
    incoming_unidirectional_streams: (),
    #[webapi(method, length = 0, callback = create_send_group)]
    create_send_group: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransportSendGroup, require_prototype)]
struct SendGroup<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransportSendGroup, enumerable, receiver)]
struct SendGroupPrototype {
    #[webapi(method, returns_promise, length = 0, callback = group_stats)]
    get_stats: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransportBidirectionalStream, enumerable, receiver)]
struct BidirectionalPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, "__moliWebTransportBidiReadable"))]
    readable: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, "__moliWebTransportBidiWritable"))]
    writable: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "WebTransport" => {
            Prototype::initialize_prototype_template(scope, prototype);
            let getter = v8::FunctionTemplate::builder(supports_reliable_only)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope);
            getter.set_class_name(v8str(scope, "get supportsReliableOnly"));
            template.set_accessor_property(
                v8str(scope, "supportsReliableOnly").into(),
                Some(getter),
                None,
                v8::PropertyAttribute::NONE,
            );
        }
        "WebTransportSendGroup" => {
            SendGroupPrototype::initialize_prototype_template(scope, prototype)
        }
        "WebTransportBidirectionalStream" => {
            BidirectionalPrototype::initialize_prototype_template(scope, prototype)
        }
        _ => datagrams::install(scope, prototype, name),
    }
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebTransport constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<schema::ConstructorArgs>(scope, &args) else {
        return;
    };
    if !moli_webapi_declare::initialize_web_api_constructor_receiver(
        scope,
        args.this(),
        "WebTransport",
    ) {
        return;
    }
    let owner = args.this();
    let context = owner
        .get_creation_context(scope)
        .expect("WebTransport realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let base = if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        Some(super::window_runtime::entered_window_api_base_url(
            scope,
            unsafe { &*host },
        ))
    } else {
        crate::worker::worker_current_script_url(scope)
    };
    let url = match base {
        Some(base) => base.join(&parsed.url),
        None => url::Url::parse(&parsed.url),
    };
    if !url.is_ok_and(|url| url.scheme() == "https" && url.fragment().is_none()) {
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            "WebTransport requires an HTTPS URL without a fragment.",
        );
        return;
    }
    let options = parsed.options;
    if options.allow_pooling && !options.server_certificate_hashes.is_empty() {
        webidl::throw_dom_exception(
            scope,
            "NotSupportedError",
            "Certificate hashes require a dedicated connection.",
        );
        return;
    }
    let mut protocols = std::collections::HashSet::new();
    for protocol in &options.protocols {
        // HTTP WebTransport protocol names are structured field strings.
        if protocol.0.is_empty()
            || protocol.0.len() > 512
            || !protocol.0.iter().all(|unit| (0x20..=0x7e).contains(unit))
            || !protocols.insert(&protocol.0)
        {
            webidl::throw_dom_exception(
                scope,
                "SyntaxError",
                "Invalid or duplicate WebTransport protocol.",
            );
            return;
        }
    }
    if !options.headers.validate(scope) {
        return;
    }
    let ready = v8::PromiseResolver::new(scope).expect("WebTransport ready promise");
    let closed = v8::PromiseResolver::new(scope).expect("WebTransport closed promise");
    let draining = v8::PromiseResolver::new(scope).expect("WebTransport draining promise");
    let incoming_bidi = super::stream_adapter::new_readable_stream_object(scope, None, 0.0, None);
    let incoming_uni = super::stream_adapter::new_readable_stream_object(scope, None, 0.0, None);
    let datagrams = datagrams::build(scope, owner, options.datagrams_readable_type.is_some());
    let limit_bidi = nullable_limit(
        scope,
        options.anticipated_concurrent_incoming_bidirectional_streams,
    );
    let limit_uni = nullable_limit(
        scope,
        options.anticipated_concurrent_incoming_unidirectional_streams,
    );
    Session::new(
        ready.get_promise(scope).into(),
        closed.get_promise(scope).into(),
        draining.get_promise(scope).into(),
        ready.into(),
        closed.into(),
        datagrams,
        incoming_bidi,
        incoming_uni,
        limit_bidi,
        limit_uni,
    )
    .initialize(scope, owner)
    .expect("WebTransport native slots");
    tasks::queue_connection_failure(scope, owner);
    rv.set(owner.into());
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    web_api_object_target(scope, receiver).expect("validated WebTransport receiver")
}

fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, owner, slot).expect("WebTransport object slot")
}

fn connecting<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, owner, STATE)
        .expect("WebTransport state")
        .to_rust_string_lossy(scope)
        == "connecting"
}

fn invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    webidl::throw_dom_exception(
        scope,
        "InvalidStateError",
        "The WebTransport session is not available.",
    );
}

fn nullable_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<u16>,
) -> v8::Local<'s, v8::Value> {
    value
        .map(|value| v8::Integer::new_from_unsigned(scope, u32::from(value)).into())
        .unwrap_or_else(|| v8::null(scope).into())
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = target(scope, args.this());
    let key = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, receiver, &key) {
        rv.set(value);
    }
}

fn set_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::LimitArgs>(scope, &args) else {
        return;
    };
    let receiver = target(scope, args.this());
    let key = args.data().to_rust_string_lossy(scope);
    let value = nullable_limit(scope, parsed.value);
    set_private_value(scope, receiver, &key, value);
}

fn reliability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8str(scope, "pending").into());
}
fn congestion_control<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8str(scope, "default").into());
}
fn protocol<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8str(scope, "").into());
}
fn response_headers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::null(scope).into());
}
fn supports_reliable_only<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(false);
}

fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::CloseArgs>(scope, &args) else {
        return;
    };
    let receiver = target(scope, args.this());
    if connecting(scope, receiver) {
        tasks::fail(
            scope,
            receiver,
            "WebTransport was closed before connecting.",
        );
    }
}

fn get_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = target(scope, args.this());
    tasks::wait_for_connection(scope, receiver, rv);
}

fn export_keying_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::ExportArgs>(scope, &args) else {
        return;
    };
    if parsed.label.byte_length() > 255
        || parsed.context.byte_length() > 255
        || parsed.output_length == 0
        || parsed.output_length > 4096
    {
        throw_range_error(scope, "Invalid WebTransport key material length.");
        return;
    }
    let receiver = target(scope, args.this());
    tasks::wait_for_connection(scope, receiver, rv);
}

fn same_group<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    group: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    group.is_none_or(|group| {
        let group = target(scope, group);
        object(scope, group, OWNER) == owner
    })
}

fn create_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::CreateStreamArgs>(scope, &args) else {
        return;
    };
    let owner = target(scope, args.this());
    if !same_group(scope, owner, parsed.options.send_group) {
        throw_type_error(scope, "The send group belongs to another WebTransport.");
        return;
    }
    tasks::wait_for_connection(scope, owner, rv);
}

fn create_send_group<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = target(scope, args.this());
    if !connecting(scope, owner) {
        invalid_state(scope);
        return;
    }
    let context = owner
        .get_creation_context(scope)
        .expect("WebTransport realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    super::exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "WebTransportSendGroup")
        .expect("send group intrinsic");
    rv.set(
        SendGroup::new(owner)
            .bind(scope)
            .expect("send group")
            .into(),
    );
}

fn group_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let group = target(scope, args.this());
    let owner = object(scope, group, OWNER);
    tasks::queue_empty_group_stats(scope, owner, rv);
}

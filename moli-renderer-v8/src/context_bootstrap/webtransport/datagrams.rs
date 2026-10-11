//! Native datagram stream identity, configuration and pre-connection buffering.
//! Buffered writes do not imply network delivery. Session failure errors the
//! streams and releases every queued payload and pending sink operation.

use super::*;

const READABLE: &str = "__moliWebTransportDatagramsReadable";
const WRITABLES: &str = "__moliWebTransportDatagramWritables";
const MAX_SIZE: &str = "__moliWebTransportDatagramMaxSize";
const INCOMING_AGE: &str = "__moliWebTransportDatagramIncomingAge";
const OUTGOING_AGE: &str = "__moliWebTransportDatagramOutgoingAge";
const INCOMING_LIMIT: &str = "__moliWebTransportDatagramIncomingLimit";
const OUTGOING_LIMIT: &str = "__moliWebTransportDatagramOutgoingLimit";
const SEND_GROUP: &str = "__moliWebTransportDatagramSendGroup";
const SEND_ORDER: &str = "__moliWebTransportDatagramSendOrder";
const QUEUE: &str = "__moliWebTransportDatagramQueue";
const WRITE_RESOLVER: &str = "__moliWebTransportDatagramWriteResolver";
const WRITE_BYTES: &str = "__moliWebTransportDatagramWriteBytes";
const WRITE_TIMESTAMP: &str = "__moliWebTransportDatagramWriteTimestamp";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransportDatagramDuplexStream, require_prototype)]
struct Duplex<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = READABLE)]
    readable: v8::Local<'s, v8::Object>,
    #[webapi(slot = WRITABLES, init = "array")]
    writables: (),
    #[webapi(slot = MAX_SIZE, value = 65535u32)]
    max_size: (),
    #[webapi(slot = INCOMING_AGE, init = "null")]
    incoming_age: (),
    #[webapi(slot = OUTGOING_AGE, init = "null")]
    outgoing_age: (),
    #[webapi(slot = INCOMING_LIMIT, value = 16u32)]
    incoming_limit: (),
    #[webapi(slot = OUTGOING_LIMIT, value = 16u32)]
    outgoing_limit: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransportDatagramsWritable, require_prototype)]
struct Writable<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = SEND_GROUP)]
    send_group: v8::Local<'s, v8::Value>,
    #[webapi(slot = SEND_ORDER)]
    send_order: f64,
    #[webapi(slot = QUEUE, init = "array")]
    queue: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct Datagram<'s> {
    #[webapi(slot = WRITE_RESOLVER)]
    resolver: v8::Local<'s, v8::Value>,
    #[webapi(slot = WRITE_BYTES)]
    bytes: v8::Local<'s, v8::Value>,
    #[webapi(slot = WRITE_TIMESTAMP)]
    timestamp: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransportDatagramDuplexStream, enumerable, receiver)]
struct DuplexPrototype {
    #[webapi(method, length = 0, callback = create_writable)]
    create_writable: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, READABLE))]
    readable: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, MAX_SIZE))]
    max_datagram_size: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_age, data = v8str(scope, INCOMING_AGE))]
    incoming_max_age: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_age, data = v8str(scope, OUTGOING_AGE))]
    outgoing_max_age: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_buffered, data = v8str(scope, INCOMING_LIMIT))]
    incoming_max_buffered_datagrams: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_buffered, data = v8str(scope, OUTGOING_LIMIT))]
    outgoing_max_buffered_datagrams: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransportDatagramsWritable, enumerable, receiver)]
struct WritablePrototype {
    #[webapi(accessor_property, getter = slot_getter, setter = set_group, data = v8str(scope, SEND_GROUP))]
    send_group: (),
    #[webapi(accessor_property, getter = slot_getter, setter = set_order, data = v8str(scope, SEND_ORDER))]
    send_order: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    match name {
        "WebTransportDatagramDuplexStream" => {
            DuplexPrototype::initialize_prototype_template(scope, prototype)
        }
        "WebTransportDatagramsWritable" => {
            WritablePrototype::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    bytes: bool,
) -> v8::Local<'s, v8::Object> {
    super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WebTransportDatagramDuplexStream",
    )
    .expect("datagram intrinsic");
    let readable = if bytes {
        super::super::stream_adapter::new_readable_byte_stream_object(scope)
    } else {
        super::super::stream_adapter::new_readable_stream_object(scope, None, 0.0, None)
    };
    Duplex::new(owner, readable)
        .bind(scope)
        .expect("datagram duplex stream")
}

fn create_writable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::CreateWritableArgs>(scope, &args) else {
        return;
    };
    let duplex = target(scope, args.this());
    let owner = object(scope, duplex, OWNER);
    if !same_group(scope, owner, parsed.options.send_group) {
        throw_type_error(scope, "The send group belongs to another WebTransport.");
        return;
    }
    if !connecting(scope, owner) {
        invalid_state(scope);
        return;
    }
    let context = owner
        .get_creation_context(scope)
        .expect("WebTransport datagram realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WebTransportDatagramsWritable",
    )
    .expect("datagram writable intrinsic");
    let group = parsed
        .options
        .send_group
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let writable = Writable::new(owner, group, parsed.options.send_order as f64)
        .bind(scope)
        .expect("datagram writable");
    // This browser-owned sink is not reachable by author script. A null
    // prototype prevents Object.prototype pollution from installing algorithms.
    let sink = crate::util::new_null_prototype_object(scope);
    let write = v8::Function::builder(write_datagram)
        .data(writable.into())
        .build(scope)
        .expect("datagram write algorithm");
    let key = v8str(scope, "write");
    assert_eq!(
        sink.create_data_property(scope, key.into(), write.into()),
        Some(true)
    );
    super::super::stream_adapter::initialize_writable_stream_object(
        scope,
        writable,
        Some(sink),
        1.0,
        None,
    );
    let writables = tasks::array(scope, duplex, WRITABLES);
    tasks::push(scope, writables, writable.into());
    rv.set(writable.into());
}

fn set_age<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::AgeArgs>(scope, &args) else {
        return;
    };
    if parsed
        .value
        .is_some_and(|value| value.is_nan() || value < 0.0)
    {
        throw_range_error(scope, "Datagram maximum age must not be negative or NaN.");
        return;
    }
    let receiver = target(scope, args.this());
    let key = args.data().to_rust_string_lossy(scope);
    let value = parsed
        .value
        .filter(|value| *value != 0.0)
        .map(|value| v8::Number::new(scope, value).into())
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, receiver, &key, value);
}

fn set_buffered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::BufferedArgs>(scope, &args) else {
        return;
    };
    let receiver = target(scope, args.this());
    let key = args.data().to_rust_string_lossy(scope);
    let value = v8::Integer::new_from_unsigned(scope, parsed.value.max(1));
    set_private_value(scope, receiver, &key, value.into());
}

fn set_group<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::GroupArgs>(scope, &args) else {
        return;
    };
    let receiver = target(scope, args.this());
    let owner = object(scope, receiver, OWNER);
    if !same_group(scope, owner, parsed.value) {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            "The send group belongs to another WebTransport.",
        );
        return;
    }
    let value = parsed
        .value
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, receiver, SEND_GROUP, value);
}

fn set_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::OrderArgs>(scope, &args) else {
        return;
    };
    let receiver = target(scope, args.this());
    let value = v8::Number::new(scope, parsed.value as f64);
    set_private_value(scope, receiver, SEND_ORDER, value.into());
}

fn write_datagram<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let timestamp = moli_time::monotonic_timestamp_seconds() * 1000.0;
    let writable = v8::Local::<v8::Object>::try_from(args.data()).expect("datagram sink payload");
    let bytes = match webidl::convert::<webidl::NonSharedBufferSource>(
        scope,
        args.get(0),
        webidl::Context::argument("WebTransportDatagramsWritable.write", 1),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let owner = object(scope, writable, OWNER);
    let duplex = object(scope, owner, DATAGRAMS);
    let max_size = get_private_value(scope, duplex, MAX_SIZE)
        .expect("datagram maximum size")
        .uint32_value(scope)
        .unwrap();
    let resolver = v8::PromiseResolver::new(scope).expect("datagram write promise");
    rv.set(resolver.get_promise(scope).into());
    if bytes.byte_length() > max_size as usize {
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        return;
    }
    let bytes = bytes.to_vec(scope);
    let bytes = crate::blob::array_buffer_from_bytes(scope, bytes).expect("datagram byte copy");
    let datagram = Datagram::new(resolver.into(), bytes.into(), timestamp)
        .bind(scope)
        .expect("buffered datagram");
    let queue = tasks::array(scope, writable, QUEUE);
    tasks::push(scope, queue, datagram.into());
    let limit = get_private_value(scope, duplex, OUTGOING_LIMIT)
        .expect("datagram buffering limit")
        .uint32_value(scope)
        .unwrap();
    if queue.length() < limit {
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
    }
}

pub(super) struct Failure<'s> {
    readable: v8::Local<'s, v8::Object>,
    writables: Vec<v8::Local<'s, v8::Object>>,
    writes: Vec<v8::Local<'s, v8::PromiseResolver>>,
}

pub(super) fn prepare_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    duplex: v8::Local<'s, v8::Object>,
) -> Failure<'s> {
    let readable = object(scope, duplex, READABLE);
    let array = tasks::array(scope, duplex, WRITABLES);
    let mut writables = Vec::new();
    let mut writes = Vec::new();
    for index in 0..array.length() {
        let writable = v8::Local::<v8::Object>::try_from(array.get_index(scope, index).unwrap())
            .expect("datagram writable");
        let queue = tasks::array(scope, writable, QUEUE);
        for index in 0..queue.length() {
            let data = v8::Local::<v8::Object>::try_from(queue.get_index(scope, index).unwrap())
                .expect("buffered datagram");
            let value = get_private_value(scope, data, WRITE_RESOLVER).expect("datagram resolver");
            writes.push(tasks::resolver(value));
        }
        let empty = v8::Array::new(scope, 0);
        set_private_value(scope, writable, QUEUE, empty.into());
        writables.push(writable);
    }
    let empty = v8::Array::new(scope, 0);
    set_private_value(scope, duplex, WRITABLES, empty.into());
    Failure {
        readable,
        writables,
        writes,
    }
}

pub(super) fn finish_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    failure: Failure<'s>,
    error: v8::Local<'s, v8::Value>,
) {
    for write in failure.writes {
        write.reject(scope, error);
    }
    for writable in failure.writables {
        super::super::stream_adapter::error_writable_stream_with_value(scope, writable, error);
    }
    super::super::stream_adapter::error_stream(scope, failure.readable, error);
}

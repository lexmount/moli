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
const WRITE_WRITABLE: &str = "__moliWebTransportDatagramWritable";

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
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = WRITE_WRITABLE)]
    writable: v8::Local<'s, v8::Object>,
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
    ready: v8::Local<'s, v8::Promise>,
    bytes: bool,
) -> v8::Local<'s, v8::Object> {
    super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WebTransportDatagramDuplexStream",
    )
    .expect("datagram intrinsic");
    let readable = streams::datagrams(scope, owner, ready, bytes);
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
    if !connecting(scope, owner) && backend::connection(scope, owner).is_none() {
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
    if key == INCOMING_AGE {
        let owner = object(scope, receiver, OWNER);
        backend::configure_incoming(scope, owner);
    }
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
    if key == INCOMING_LIMIT {
        let owner = object(scope, receiver, OWNER);
        backend::configure_incoming(scope, owner);
    }
    if key == OUTGOING_LIMIT {
        let writables = tasks::array(scope, receiver, WRITABLES);
        let writables: Vec<_> = (0..writables.length())
            .map(|index| {
                v8::Local::<v8::Object>::try_from(
                    writables
                        .get_index(scope, index)
                        .expect("datagram writable"),
                )
                .expect("writable object")
            })
            .collect();
        for writable in writables {
            relieve_backpressure(scope, receiver, writable);
        }
    }
}

pub(super) fn incoming_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> (usize, Option<std::time::Duration>) {
    let duplex = object(scope, owner, DATAGRAMS);
    let maximum = get_private_value(scope, duplex, INCOMING_LIMIT)
        .expect("incoming buffer limit")
        .uint32_value(scope)
        .expect("numeric limit") as usize;
    let age = get_private_value(scope, duplex, INCOMING_AGE)
        .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())
        .and_then(|value| std::time::Duration::try_from_secs_f64(value.value() / 1000.0).ok());
    (maximum, age)
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
    let datagram = Datagram::new(owner, writable, resolver.into(), bytes.into(), timestamp)
        .bind(scope)
        .expect("buffered datagram");
    if !connecting(scope, owner) && backend::connection(scope, owner).is_none() {
        reject(scope, resolver, "The WebTransport session is closed.");
        return;
    }
    let queue = tasks::array(scope, writable, QUEUE);
    tasks::push(scope, queue, datagram.into());
    if backend::connection(scope, owner).is_some() {
        send(scope, owner, datagram);
    }
    relieve_backpressure(scope, duplex, writable);
}

fn relieve_backpressure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    duplex: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
) {
    let queue = tasks::array(scope, writable, QUEUE);
    let limit = get_private_value(scope, duplex, OUTGOING_LIMIT)
        .expect("outgoing buffer limit")
        .uint32_value(scope)
        .expect("numeric limit");
    if queue.length() >= limit {
        return;
    }
    let writes: Vec<_> = (0..queue.length())
        .map(|index| {
            let datagram = v8::Local::<v8::Object>::try_from(
                queue.get_index(scope, index).expect("buffered datagram"),
            )
            .expect("datagram object");
            tasks::resolver(
                get_private_value(scope, datagram, WRITE_RESOLVER).expect("write resolver"),
            )
        })
        .collect();
    for write in writes {
        let undefined = v8::undefined(scope);
        write.resolve(scope, undefined.into());
    }
}

fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = super::super::build_web_transport_error(
        scope,
        &super::super::WebTransportErrorClonePayload {
            message: message.encode_utf16().collect(),
            source: super::super::WebTransportErrorSource::Session,
            stream_error_code: None,
        },
    )
    .expect("datagram error");
    resolver.reject(scope, error.into());
}

fn send<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    datagram: v8::Local<'s, v8::Object>,
) {
    let resolver = tasks::resolver(
        get_private_value(scope, datagram, WRITE_RESOLVER).expect("write resolver"),
    );
    let Some((session, runner)) = backend::connection(scope, owner) else {
        reject(scope, resolver, "The WebTransport session is closed.");
        return;
    };
    let Some(producer) = networking::register(scope, owner, datagram) else {
        return;
    };
    let value = get_private_value(scope, datagram, WRITE_BYTES).expect("private datagram bytes");
    let bytes = webidl::convert::<webidl::NonSharedBufferSource>(
        scope,
        value,
        webidl::Context::member("WebTransport", "datagrams"),
    )
    .expect("private datagram buffer")
    .to_vec(scope);
    let duplex = object(scope, owner, DATAGRAMS);
    let timestamp = get_private_value(scope, datagram, WRITE_TIMESTAMP)
        .expect("datagram timestamp")
        .number_value(scope)
        .expect("numeric timestamp");
    let age = get_private_value(scope, duplex, OUTGOING_AGE)
        .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())
        .map(|value| value.value());
    let remaining_ms =
        age.map(|age| age - (moli_time::monotonic_timestamp_seconds() * 1000.0 - timestamp));
    let expired = remaining_ms.is_some_and(|age| age <= 0.0);
    let deadline = remaining_ms
        .filter(|age| age.is_finite() && *age > 0.0)
        .and_then(|age| std::time::Duration::try_from_secs_f64(age / 1000.0).ok());
    runner.spawn(async move {
        let result = if expired || bytes.len() > session.max_datagram_size() {
            Ok(())
        } else if let Some(deadline) = deadline {
            match tokio::time::timeout(deadline, session.send_datagram_wait(bytes.into())).await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Ok(()), // Expired unreliable payload, not a session failure.
            }
        } else {
            session
                .send_datagram_wait(bytes.into())
                .await
                .map_err(|error| error.to_string())
        };
        producer.send(networking::Completion::Datagram(result));
    });
}

pub(super) fn complete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    datagram: v8::Local<'s, v8::Object>,
    result: Result<(), String>,
) {
    let resolver = tasks::resolver(
        get_private_value(scope, datagram, WRITE_RESOLVER).expect("write resolver"),
    );
    let writable = object(scope, datagram, WRITE_WRITABLE);
    let previous = tasks::array(scope, writable, QUEUE);
    let remaining = v8::Array::new(scope, 0);
    for index in 0..previous.length() {
        let value = previous.get_index(scope, index).expect("queued datagram");
        if !value.strict_equals(datagram.into()) {
            tasks::push(scope, remaining, value);
        }
    }
    set_private_value(scope, writable, QUEUE, remaining.into());
    if let Err(message) = result {
        let error = tasks::fail(scope, owner, &message);
        resolver.reject(scope, error.into());
    } else {
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        let duplex = object(scope, owner, DATAGRAMS);
        relieve_backpressure(scope, duplex, writable);
    }
}

pub(super) fn connected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    maximum_size: usize,
) {
    let duplex = object(scope, owner, DATAGRAMS);
    let maximum_size = v8::Integer::new_from_unsigned(scope, maximum_size as u32);
    set_private_value(scope, duplex, MAX_SIZE, maximum_size.into());
    let writables = tasks::array(scope, duplex, WRITABLES);
    let mut queued = Vec::new();
    for index in 0..writables.length() {
        let writable = v8::Local::<v8::Object>::try_from(
            writables
                .get_index(scope, index)
                .expect("datagram writable"),
        )
        .expect("writable");
        let queue = tasks::array(scope, writable, QUEUE);
        for index in 0..queue.length() {
            let data = v8::Local::<v8::Object>::try_from(
                queue.get_index(scope, index).expect("datagram record"),
            )
            .expect("datagram record object");
            queued.push(data);
        }
    }
    for data in queued {
        send(scope, owner, data);
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

pub(super) fn finish_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    duplex: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let failure = prepare_failure(scope, duplex);
    for write in failure.writes {
        write.reject(scope, error);
    }
    for writable in failure.writables {
        super::super::stream_adapter::error_writable_stream_with_value(scope, writable, error);
    }
    super::super::stream_adapter::close_stream(scope, failure.readable);
}

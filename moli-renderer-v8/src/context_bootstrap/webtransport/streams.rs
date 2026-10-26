//! Pull-driven QUIC byte streams. Each source admits one native read at a
//! time; WritableStream's sink promise applies transport backpressure.

use std::sync::Arc;
use tokio::sync::{Mutex, watch};

use super::super::stream_adapter as streams;
use super::{
    networking::{self, Completion},
    *,
};

pub(super) const RESOLVER: &str = "__moliWebTransportOperationResolver";
const STREAM: &str = "__moliWebTransportOperationStream";
const OPEN_BIDI: &str = "__moliWebTransportOpenBidirectional";
const ENDPOINT: &str = "__moliWebTransportNativeEndpoint";

#[derive(Clone)]
enum ReadKind {
    IncomingBi,
    IncomingUni,
    Datagram { bytes: bool },
    Bytes(Arc<Mutex<moli_webtransport::RecvStream>>),
}

#[derive(Clone)]
enum Endpoint {
    Read {
        kind: ReadKind,
        stop: watch::Sender<Option<u32>>,
    },
    Send {
        stream: Arc<Mutex<moli_webtransport::SendStream>>,
        stop: watch::Sender<Option<u32>>,
    },
}

pub(crate) struct TransferError {
    message: String,
    code: Option<u32>,
    session: bool,
}

impl TransferError {
    fn session(error: impl std::fmt::Display) -> Self {
        Self {
            message: error.to_string(),
            code: None,
            session: true,
        }
    }
    fn read(error: moli_webtransport::ReadError) -> Self {
        let code = match error {
            moli_webtransport::ReadError::Reset(code) => Some(code),
            _ => None,
        };
        let session = matches!(error, moli_webtransport::ReadError::SessionError(_));
        Self {
            message: error.to_string(),
            code,
            session,
        }
    }
    fn write(error: moli_webtransport::WriteError) -> Self {
        let code = match error {
            moli_webtransport::WriteError::Stopped(code) => Some(code),
            _ => None,
        };
        let session = matches!(error, moli_webtransport::WriteError::SessionError(_));
        Self {
            message: error.to_string(),
            code,
            session,
        }
    }
}

pub(crate) enum StreamValue {
    Bidirectional(moli_webtransport::SendStream, moli_webtransport::RecvStream),
    Send(moli_webtransport::SendStream),
    Receive(moli_webtransport::RecvStream),
    Bytes(Option<Vec<u8>>),
    Datagram(Vec<u8>),
    Complete,
    Cancelled,
}

pub(crate) type StreamCompletion = Result<StreamValue, TransferError>;

#[derive(WebApiObject)]
#[webapi(plain)]
struct Operation<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'s, v8::PromiseResolver>,
    #[webapi(slot = STREAM)]
    stream: v8::Local<'s, v8::Object>,
    #[webapi(slot = OPEN_BIDI)]
    bidi: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransportBidirectionalStream, require_prototype)]
struct Bidirectional<'s> {
    #[webapi(slot = "__moliWebTransportBidiReadable")]
    readable: v8::Local<'s, v8::Object>,
    #[webapi(slot = "__moliWebTransportBidiWritable")]
    writable: v8::Local<'s, v8::Object>,
}

fn set_endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    endpoint: Endpoint,
) {
    let mut endpoint = Box::new(endpoint);
    let pointer = (&mut *endpoint as *mut Endpoint).cast();
    let external = v8::External::new(scope, pointer);
    set_private_value(scope, object, ENDPOINT, external.into());
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, object, move || drop(endpoint));
}

fn endpoint<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> Endpoint {
    let value = get_private_value(scope, object, ENDPOINT).expect("native stream endpoint");
    let external = v8::Local::<v8::External>::try_from(value).expect("endpoint private slot");
    // SAFETY: set_endpoint owns this typed allocation through the context's
    // guaranteed finalizer. Only native algorithm payloads call this helper.
    unsafe { &*external.value().cast::<Endpoint>() }.clone()
}

fn algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    name: &'static str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
) {
    let function = v8::Function::builder(callback)
        .data(data)
        .build(scope)
        .expect("native stream algorithm");
    let name = v8str(scope, name);
    assert_eq!(
        source.create_data_property(scope, name.into(), function.into()),
        Some(true)
    );
}

pub(super) fn incoming<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    ready: v8::Local<'s, v8::Promise>,
    bidi: bool,
) -> v8::Local<'s, v8::Object> {
    readable(
        scope,
        owner,
        ready,
        if bidi {
            ReadKind::IncomingBi
        } else {
            ReadKind::IncomingUni
        },
    )
}

pub(super) fn datagrams<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    ready: v8::Local<'s, v8::Promise>,
    bytes: bool,
) -> v8::Local<'s, v8::Object> {
    readable(scope, owner, ready, ReadKind::Datagram { bytes })
}

fn readable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    ready: v8::Local<'s, v8::Promise>,
    kind: ReadKind,
) -> v8::Local<'s, v8::Object> {
    let received_bytes = matches!(kind, ReadKind::Bytes(_));
    let byte_stream = matches!(
        kind,
        ReadKind::Bytes(_) | ReadKind::Datagram { bytes: true }
    );
    let stream = streams::new_readable_stream_shell_object(scope);
    set_private_value(scope, stream, OWNER, owner.into());
    let (stop, _) = watch::channel(None);
    set_endpoint(scope, stream, Endpoint::Read { kind, stop });
    let source = crate::util::new_null_prototype_object(scope);
    algorithm(scope, source, "start", start, ready.into());
    algorithm(scope, source, "pull", pull, stream.into());
    algorithm(scope, source, "cancel", cancel_read, stream.into());
    let source = streams::parse_readable_stream_source_object(scope, source)
        .expect("native source callbacks");
    streams::initialize_webidl_readable_stream_object(
        scope,
        stream,
        Some(source),
        byte_stream,
        0.0,
        None,
    );
    if received_bytes {
        backend::remember_stream(scope, owner, stream);
    }
    stream
}

fn start<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data());
}

fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
    bidi: bool,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let resolver = v8::PromiseResolver::new(scope).expect("WebTransport stream promise");
    rv.set(resolver.get_promise(scope).into());
    Operation::new(owner, resolver, stream, bidi)
        .bind(scope)
        .expect("native stream operation")
}

pub(super) fn open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    bidi: bool,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !connecting(scope, owner) && backend::connection(scope, owner).is_none() {
        invalid_state(scope);
        return;
    }
    let payload = operation(scope, owner, owner, bidi, rv);
    if connecting(scope, owner) {
        let pending = tasks::array(scope, owner, PENDING);
        tasks::push(scope, pending, payload.into());
    } else {
        open_pending(scope, owner, payload);
    }
}

pub(super) fn open_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    payload: v8::Local<'s, v8::Object>,
) {
    let bidi = get_private_value(scope, payload, OPEN_BIDI)
        .expect("stream kind")
        .is_true();
    let Some((session, runner)) = backend::connection(scope, owner) else {
        reject_open(scope, payload);
        return;
    };
    let Some(producer) = networking::register(scope, owner, payload) else {
        return;
    };
    runner.spawn(async move {
        let result = if bidi {
            session
                .open_bi()
                .await
                .map(|(send, recv)| StreamValue::Bidirectional(send, recv))
        } else {
            session.open_uni().await.map(StreamValue::Send)
        };
        producer.send(Completion::Stream(result.map_err(TransferError::session)));
    });
}

fn pull<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = v8::Local::<v8::Object>::try_from(args.data()).expect("native readable payload");
    let owner = object(scope, stream, OWNER);
    let payload = operation(scope, owner, stream, false, rv);
    let Endpoint::Read { kind, stop } = endpoint(scope, stream) else {
        unreachable!("read endpoint")
    };
    let Some((session, runner)) = backend::connection(scope, owner) else {
        reject(
            scope,
            payload,
            TransferError::session("The WebTransport session is closed."),
        );
        return;
    };
    let Some(producer) = networking::register(scope, owner, payload) else {
        return;
    };
    let mut cancelled = stop.subscribe();
    let incoming = backend::incoming(scope, owner);
    runner.spawn(async move {
        if cancelled.borrow().is_some() { producer.send(Completion::Stream(Ok(StreamValue::Cancelled))); return; }
        let result = match kind {
            ReadKind::Bytes(stream) => {
                let mut stream = stream.lock().await;
                let mut bytes = vec![0; 64 * 1024];
                tokio::select! { biased;
                    _ = cancelled.changed() => {
                        let _ = stream.stop(cancelled.borrow().unwrap_or(0));
                        Ok(StreamValue::Cancelled)
                    }
                    result = stream.read(&mut bytes) => result.map(|length| {
                        StreamValue::Bytes(length.map(|length| { bytes.truncate(length); bytes }))
                    }).map_err(TransferError::read),
                }
            }
            kind => tokio::select! { biased;
                _ = cancelled.changed() => Ok(StreamValue::Cancelled),
                result = async {
                    match kind {
                        ReadKind::IncomingBi => session.accept_bi().await.map(|(send, recv)| StreamValue::Bidirectional(send, recv)).map_err(TransferError::session),
                        ReadKind::IncomingUni => session.accept_uni().await.map(StreamValue::Receive).map_err(TransferError::session),
                        ReadKind::Datagram { bytes } => loop {
                            let incoming = incoming.as_ref().expect("connected datagram ingress");
                            match incoming.read().await {
                                Ok(value) if bytes && value.is_empty() => continue,
                                result => break result.map(|value| StreamValue::Datagram(value.to_vec())).map_err(TransferError::session),
                            }
                        },
                        ReadKind::Bytes(_) => unreachable!("byte stream handled above"),
                    }
                } => result,
            },
        };
        producer.send(Completion::Stream(result));
    });
}

fn error_code<'s>(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> u32 {
    v8::Local::<v8::Object>::try_from(value)
        .ok()
        .and_then(|value| super::super::web_transport_error_clone_payload_from_object(scope, value))
        .and_then(|payload| payload.stream_error_code)
        .unwrap_or(0)
}

fn cancel_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = v8::Local::<v8::Object>::try_from(args.data()).expect("native readable payload");
    let code = error_code(scope, args.get(0));
    let owner = object(scope, stream, OWNER);
    let Endpoint::Read { kind, stop } = endpoint(scope, stream) else {
        unreachable!("read endpoint")
    };
    stop.send_replace(Some(code));
    if matches!(kind, ReadKind::Datagram { .. })
        && let Some(incoming) = backend::incoming(scope, owner)
    {
        incoming.discard();
    }
    if let ReadKind::Bytes(stream) = kind
        && let Some((_, runner)) = backend::connection(scope, owner)
    {
        runner.spawn(async move {
            let _ = stream.lock().await.stop(code);
        });
    }
}

fn stream_owner<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    v8::Local::<v8::Object>::try_from(data).expect("stream payload")
}

fn writable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    native: moli_webtransport::SendStream,
) -> v8::Local<'s, v8::Object> {
    let stream = streams::new_writable_stream_shell_object(scope);
    set_private_value(scope, stream, OWNER, owner.into());
    let (stop, _) = watch::channel(None);
    set_endpoint(
        scope,
        stream,
        Endpoint::Send {
            stream: Arc::new(Mutex::new(native)),
            stop,
        },
    );
    let sink = crate::util::new_null_prototype_object(scope);
    algorithm(scope, sink, "write", write, stream.into());
    algorithm(scope, sink, "close", finish, stream.into());
    algorithm(scope, sink, "abort", abort, stream.into());
    let sink =
        streams::parse_writable_stream_sink_object(scope, sink).expect("native sink callbacks");
    streams::initialize_webidl_writable_stream_object(scope, stream, Some(sink), 1.0, None);
    backend::remember_stream(scope, owner, stream);
    stream
}

fn write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let bytes = match webidl::convert::<webidl::NonSharedBufferSource>(
        scope,
        args.get(0),
        webidl::Context::argument("WebTransportSendStream.write", 1),
    ) {
        Ok(bytes) => bytes.to_vec(scope),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    send(scope, args.data(), Some(bytes), rv);
}

fn finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    send(scope, args.data(), None, rv);
}

fn send<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    bytes: Option<Vec<u8>>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = stream_owner(scope, data);
    let owner = object(scope, stream, OWNER);
    let payload = operation(scope, owner, stream, false, rv);
    let Endpoint::Send { stream, stop } = endpoint(scope, stream) else {
        unreachable!("send endpoint")
    };
    let Some((_, runner)) = backend::connection(scope, owner) else {
        reject(
            scope,
            payload,
            TransferError::session("The WebTransport session is closed."),
        );
        return;
    };
    let Some(producer) = networking::register(scope, owner, payload) else {
        return;
    };
    let mut cancelled = stop.subscribe();
    runner.spawn(async move {
        let mut stream = stream.lock().await;
        let cancelled_code = *cancelled.borrow();
        let result = if let Some(code) = cancelled_code {
            let _ = stream.reset(code);
            Ok(StreamValue::Complete)
        } else {
            tokio::select! { biased;
                _ = cancelled.changed() => { let _ = stream.reset(cancelled.borrow().unwrap_or(0)); Ok(StreamValue::Complete) }
                result = async {
                    if let Some(bytes) = bytes {
                        stream.write_all(&bytes).await.map_err(TransferError::write)?;
                    } else {
                        stream.finish().map_err(TransferError::session)?;
                        if let Some(code) = stream.stopped().await.map_err(TransferError::session)? {
                            return Err(TransferError { message: "The peer stopped the stream.".into(), code: Some(code), session: false });
                        }
                    }
                    Ok(StreamValue::Complete)
                } => result,
            }
        };
        producer.send(Completion::Stream(result));
    });
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let code = error_code(scope, args.get(0));
    let target = stream_owner(scope, args.data());
    let Endpoint::Send { stream, stop } = endpoint(scope, target) else {
        unreachable!("send endpoint")
    };
    stop.send_replace(Some(code));
    let owner = object(scope, target, OWNER);
    let payload = operation(scope, owner, target, false, rv);
    let Some((_, runner)) = backend::connection(scope, owner) else {
        reject(
            scope,
            payload,
            TransferError::session("The WebTransport session is closed."),
        );
        return;
    };
    let Some(producer) = networking::register(scope, owner, payload) else {
        return;
    };
    runner.spawn(async move {
        let result = stream
            .lock()
            .await
            .reset(code)
            .map(|_| StreamValue::Complete)
            .map_err(TransferError::session);
        producer.send(Completion::Stream(result));
    });
}

fn bidi<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    send: moli_webtransport::SendStream,
    recv: moli_webtransport::RecvStream,
) -> v8::Local<'s, v8::Object> {
    let ready = object_promise(scope, owner, READY);
    let readable = readable(
        scope,
        owner,
        ready,
        ReadKind::Bytes(Arc::new(Mutex::new(recv))),
    );
    let writable = writable(scope, owner, send);
    super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WebTransportBidirectionalStream",
    )
    .expect("bidirectional intrinsic");
    Bidirectional::new(readable, writable)
        .bind(scope)
        .expect("bidirectional stream")
}

fn object_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Promise> {
    v8::Local::<v8::Promise>::try_from(
        get_private_value(scope, owner, slot).expect("session promise"),
    )
    .expect("promise slot")
}

fn reject_open<'s>(scope: &mut v8::PinScope<'s, '_>, payload: v8::Local<'s, v8::Object>) {
    let error = super::super::new_dom_exception_value(
        scope,
        "The WebTransport session closed before opening the stream.",
        "InvalidStateError",
    );
    let resolver =
        tasks::resolver(get_private_value(scope, payload, RESOLVER).expect("operation resolver"));
    resolver.reject(scope, error);
}

fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Object>,
    error: TransferError,
) {
    let error = super::super::build_web_transport_error(
        scope,
        &super::super::WebTransportErrorClonePayload {
            message: error.message.encode_utf16().collect(),
            source: if error.session {
                super::super::WebTransportErrorSource::Session
            } else {
                super::super::WebTransportErrorSource::Stream
            },
            stream_error_code: error.code,
        },
    )
    .expect("WebTransport stream error");
    let resolver =
        tasks::resolver(get_private_value(scope, payload, RESOLVER).expect("operation resolver"));
    resolver.reject(scope, error.into());
}

pub(super) fn apply<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    payload: v8::Local<'s, v8::Object>,
    result: StreamCompletion,
) {
    let resolver =
        tasks::resolver(get_private_value(scope, payload, RESOLVER).expect("operation resolver"));
    let stream = object(scope, payload, STREAM);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            // Opening has no stream to error. A session that closed while
            // allocating its id rejects the pending creation operation with
            // InvalidStateError; established endpoints preserve transport errors.
            if stream == owner && error.session {
                reject_open(scope, payload);
            } else {
                reject(scope, payload, error);
            }
            return;
        }
    };
    let pulling = stream != owner;
    if !pulling && backend::connection(scope, owner).is_none() {
        reject_open(scope, payload);
        return;
    }
    if pulling
        && let Endpoint::Read { stop, .. } = endpoint(scope, stream)
        && stop.borrow().is_some()
    {
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        return;
    }
    let value: v8::Local<v8::Value> = match result {
        StreamValue::Bidirectional(send, recv) => bidi(scope, owner, send, recv).into(),
        StreamValue::Send(send) => writable(scope, owner, send).into(),
        StreamValue::Receive(recv) => {
            let ready = object_promise(scope, owner, READY);
            readable(
                scope,
                owner,
                ready,
                ReadKind::Bytes(Arc::new(Mutex::new(recv))),
            )
            .into()
        }
        StreamValue::Bytes(Some(bytes)) | StreamValue::Datagram(bytes) => {
            let length = bytes.len();
            let buffer =
                crate::blob::array_buffer_from_bytes(scope, bytes).expect("transport read buffer");
            v8::Uint8Array::new(scope, buffer, 0, length)
                .expect("transport read view")
                .into()
        }
        StreamValue::Bytes(None) => {
            streams::close_stream(scope, stream);
            v8::undefined(scope).into()
        }
        StreamValue::Complete | StreamValue::Cancelled => v8::undefined(scope).into(),
    };
    if pulling {
        if !value.is_undefined() {
            let result = if streams::readable_stream_is_byte_stream(scope, stream) {
                streams::enqueue_byte_chunk(scope, stream, value)
            } else {
                streams::enqueue_chunk(scope, stream, value)
            };
            if let Err(streams::EnqueueChunkError::Strategy(error)) = result {
                resolver.reject(scope, error);
                return;
            }
        }
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
    } else {
        resolver.resolve(scope, value);
    }
}

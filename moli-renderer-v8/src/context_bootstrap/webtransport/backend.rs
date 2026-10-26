//! Native session state stays with its branded wrapper. Resource authority
//! cancels QUIC on Document/worker retirement, independently from V8 GC.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
};

use moli_webtransport::{CloseInfo, CloseReason, ConnectOptions, Session};

use super::{
    networking::{self, Completion},
    *,
};
use crate::{
    network::{RendererResourceTaskRunner, loads::ResourceLoadLease},
    util::{get_private_value, set_private_value, v8str},
};

const NATIVE: &str = "__moliWebTransportNativeState";
const PROTOCOL: &str = "__moliWebTransportProtocol";
const CONGESTION: &str = "__moliWebTransportCongestion";

pub(crate) struct ConnectionContext {
    pub(crate) runner: RendererResourceTaskRunner,
    pub(crate) connector: moli_fetch::WebTransportConnector,
    pub(crate) lease: ResourceLoadLease,
    pub(crate) origin: String,
    pub(crate) blocked: bool,
}

struct NativeState {
    runner: RendererResourceTaskRunner,
    lease: ResourceLoadLease,
    session: RefCell<Option<Session>>,
    streams: RefCell<BTreeMap<u64, v8::Weak<v8::Object>>>,
    next_stream: Cell<u64>,
    incoming: RefCell<Option<Arc<incoming_datagrams::Incoming>>>,
    ingress: RefCell<Option<tokio::task::AbortHandle>>,
    graceful_close: Cell<bool>,
}

impl NativeState {
    fn stop_ingress(&self) {
        if let Some(incoming) = self.incoming.borrow().as_ref() {
            incoming.finish("The WebTransport session is closed.".to_owned());
        }
        if let Some(ingress) = self.ingress.borrow_mut().take() {
            ingress.abort();
        }
    }
}

impl Drop for NativeState {
    fn drop(&mut self) {
        self.stop_ingress();
        // A graceful close capsule retains the native session until it is
        // sent. Context retirement remains authoritative through the lease.
        if !self.graceful_close.get() {
            self.lease.cancel();
        }
        self.lease.finish();
    }
}

fn state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<Rc<NativeState>> {
    let value = get_private_value(scope, owner, NATIVE)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    // SAFETY: only initialize installs this private slot on a branded native
    // receiver. The context-owned finalizer retains its boxed Rc through GC.
    Some(unsafe { &*external.value().cast::<Rc<NativeState>>() }.clone())
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    context: &ConnectionContext,
) {
    let mut state = Box::new(Rc::new(NativeState {
        runner: context.runner.clone(),
        lease: context.lease.clone(),
        session: RefCell::new(None),
        streams: RefCell::new(BTreeMap::new()),
        next_stream: Cell::new(1),
        incoming: RefCell::new(None),
        ingress: RefCell::new(None),
        graceful_close: Cell::new(false),
    }));
    let pointer = (&mut *state as *mut Rc<NativeState>).cast();
    let external = v8::External::new(scope, pointer);
    set_private_value(scope, owner, NATIVE, external.into());
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, owner, move || drop(state));
}

pub(super) fn connect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    url: url::Url,
    headers: Vec<(String, String)>,
    protocols: Vec<webidl::DomString16>,
    congestion: schema::CongestionControl,
    certificate_hashes: Vec<moli_webtransport::CertificateHash>,
) {
    if url.port() == Some(0) {
        tasks::queue_connection_failure(scope, owner, "WebTransport cannot connect to port 0.");
        return;
    }
    let context = if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: this realm's bridge owns the isolate-local host.
        unsafe { &*host }.webtransport_connection_context(scope, &url)
    } else {
        crate::worker::webtransport_tasks::connection_context(scope, &url)
    };
    let Some(context) = context else {
        tasks::queue_connection_failure(
            scope,
            owner,
            "No WebTransport session backend is available.",
        );
        return;
    };
    let Some(producer) = networking::register(scope, owner, owner) else {
        tasks::queue_connection_failure(
            scope,
            owner,
            "No WebTransport session task route is available.",
        );
        return;
    };
    initialize(scope, owner, &context);
    let congestion_name = match congestion {
        schema::CongestionControl::Default => "default",
        schema::CongestionControl::Throughput => "throughput",
        schema::CongestionControl::LowLatency => "low-latency",
    };
    let value = v8str(scope, congestion_name);
    set_private_value(scope, owner, CONGESTION, value.into());
    let mut options = ConnectOptions::new(url, context.origin);
    options.server_certificate_hashes = certificate_hashes;
    options.protocols = protocols
        .into_iter()
        .map(|p| String::from_utf16(&p.0).expect("validated ASCII protocol"))
        .collect();
    options.congestion_control = match congestion {
        schema::CongestionControl::Default => moli_webtransport::CongestionControl::Default,
        schema::CongestionControl::Throughput => moli_webtransport::CongestionControl::Throughput,
        schema::CongestionControl::LowLatency => moli_webtransport::CongestionControl::LowLatency,
    };
    for (name, value) in headers {
        let bytes: Vec<_> = value.chars().map(|value| value as u8).collect();
        let Ok(value) = http::HeaderValue::from_bytes(&bytes) else {
            producer.send(Completion::Connected(Err(
                "WebTransport header cannot be sent over HTTP/3.".to_owned(),
            )));
            return;
        };
        options.headers.append(
            http::HeaderName::from_bytes(name.as_bytes()).expect("validated header name"),
            value,
        );
    }
    let lease = context.lease;
    let task_lease = lease.clone();
    // Register cancellation before polling the handshake: immediate failure or
    // a fast local server must not race the handoff to session cancellation.
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let handle = context.runner.spawn_abortable(async move {
        if start_rx.await.is_err() {
            return;
        }
        let result = if context.blocked {
            Err("WebTransport request blocked by network policy.".to_owned())
        } else {
            context
                .connector
                .connect(options)
                .await
                .map_err(|error| format!("{error:#}"))
        };
        task_lease.release_consumer_cancel();
        if let Ok(session) = &result {
            let session = session.clone();
            task_lease.attach_consumer_cancel(move || session.cancel());
        }
        producer.send(Completion::Connected(result));
    });
    lease.attach_consumer_cancel(move || handle.abort());
    let _ = start_tx.send(());
}

pub(crate) fn apply<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Object>,
    result: Completion,
) {
    let context = payload
        .get_creation_context(scope)
        .expect("WebTransport completion realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = crate::util::get_private_object(scope, payload, OWNER).unwrap_or(payload);
    let Some(native) = state(scope, owner) else {
        return;
    };
    // Exact owner authorization already discarded retired Documents/workers.
    // A failed live session still has in-flight sink promises to settle.
    match result {
        Completion::Stream(result) => streams::apply(scope, owner, payload, result),
        Completion::StreamClosed { id, end } => streams::closed(scope, owner, id, end),
        Completion::Datagram(result) => datagrams::complete(scope, owner, payload, result),
        Completion::Connected(Err(message)) => {
            if connecting(scope, owner) {
                native.lease.finish();
                tasks::fail(scope, owner, &message);
            }
        }
        Completion::Connected(Ok(session)) => {
            if !connecting(scope, owner) {
                return;
            }
            let protocol = v8::String::new(scope, session.protocol().unwrap_or(""))
                .expect("negotiated protocol");
            set_private_value(scope, owner, PROTOCOL, protocol.into());
            *native.session.borrow_mut() = Some(session.clone());
            let (maximum, age) = datagrams::incoming_policy(scope, owner);
            let incoming = Arc::new(incoming_datagrams::Incoming::new(maximum, age));
            *native.incoming.borrow_mut() = Some(incoming.clone());
            let receiver = session.clone();
            let ingress = native.runner.spawn_abortable(async move {
                loop {
                    match receiver.read_datagram().await {
                        Ok(datagram) => incoming.push(datagram),
                        Err(error) => {
                            incoming.finish(error.to_string());
                            break;
                        }
                    }
                }
            });
            *native.ingress.borrow_mut() = Some(ingress.abort_handle());
            let connected = v8str(scope, "connected");
            set_private_value(scope, owner, STATE, connected.into());
            if let Some(producer) = networking::register(scope, owner, owner) {
                native.runner.spawn(async move {
                    producer.send(Completion::Closed(session.closed().await));
                });
            }
            let maximum_size = session_size(scope, owner);
            datagrams::connected(scope, owner, maximum_size);
            if connection(scope, owner).is_none() {
                return;
            }
            let pending = tasks::array(scope, owner, PENDING);
            let values: Vec<_> = (0..pending.length())
                .map(|index| pending.get_index(scope, index).expect("pending operation"))
                .collect();
            let empty = v8::Array::new(scope, 0);
            set_private_value(scope, owner, PENDING, empty.into());
            for value in values {
                let payload =
                    v8::Local::<v8::Object>::try_from(value).expect("pending operation object");
                if get_private_value(scope, payload, streams::RESOLVER).is_some() {
                    streams::open_pending(scope, owner, payload);
                } else {
                    tasks::queue_unsupported(scope, owner, value);
                }
            }
            let ready = tasks::resolver(
                get_private_value(scope, owner, READY_RESOLVER).expect("ready resolver"),
            );
            let undefined = v8::undefined(scope);
            ready.resolve(scope, undefined.into());
        }
        Completion::Closed(CloseReason::Local(info) | CloseReason::Remote(info)) => {
            native.graceful_close.set(true);
            native.stop_ingress();
            native.lease.finish();
            let value = v8str(scope, "closed");
            set_private_value(scope, owner, STATE, value.into());
            let error = tasks::session_error(scope, owner, "The WebTransport session is closed.");
            error_streams(scope, owner, error.into());
            let incoming_bidi = object(scope, owner, INCOMING_BIDI);
            let incoming_uni = object(scope, owner, INCOMING_UNI);
            super::super::stream_adapter::close_stream(scope, incoming_bidi);
            super::super::stream_adapter::close_stream(scope, incoming_uni);
            let duplex = object(scope, owner, DATAGRAMS);
            datagrams::finish_close(scope, duplex, error.into());
            let result = v8::Object::new(scope);
            let key = v8str(scope, "closeCode");
            let code = v8::Integer::new_from_unsigned(scope, info.code);
            result.create_data_property(scope, key.into(), code.into());
            let key = v8str(scope, "reason");
            let reason = v8::String::new(scope, &info.reason).expect("close reason");
            result.create_data_property(scope, key.into(), reason.into());
            let closed = tasks::resolver(
                get_private_value(scope, owner, CLOSED_RESOLVER).expect("closed resolver"),
            );
            closed.resolve(scope, result.into());
        }
        Completion::Closed(CloseReason::Failed(error)) => {
            native.lease.finish();
            tasks::fail(scope, owner, &error.to_string());
        }
        Completion::Closed(CloseReason::OwnerRetired) => {}
    }
}

pub(super) fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    info: schema::CloseInfo,
) {
    let native = state(scope, owner);
    if connecting(scope, owner) {
        if let Some(native) = native {
            native.lease.cancel();
        }
        tasks::fail(scope, owner, "WebTransport was closed before connecting.");
    } else if let Some(native) = native {
        let session = native.session.borrow().clone();
        if let Some(session) = session {
            native.graceful_close.set(true);
            let closed = v8str(scope, "closed");
            set_private_value(scope, owner, STATE, closed.into());
            session.close(CloseInfo {
                code: info.close_code,
                reason: info.reason,
            });
        }
    }
}

pub(super) fn reliability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> &'static str {
    if state(scope, owner).is_some_and(|state| state.session.borrow().is_some()) {
        "supports-unreliable"
    } else {
        "pending"
    }
}

pub(super) fn protocol<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, owner, PROTOCOL).unwrap_or_else(|| v8str(scope, "").into())
}

pub(super) fn congestion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, owner, CONGESTION).unwrap_or_else(|| v8str(scope, "default").into())
}

pub(super) fn connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<(Session, RendererResourceTaskRunner)> {
    if get_private_value(scope, owner, STATE)?.to_rust_string_lossy(scope) != "connected" {
        return None;
    }
    let native = state(scope, owner)?;
    if native.lease.is_cancelled() {
        return None;
    }
    let session = native.session.borrow().clone()?;
    Some((session, native.runner.clone()))
}

pub(super) fn session_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> usize {
    connection(scope, owner).map_or(65535, |(session, _)| session.max_datagram_size())
}

pub(super) fn incoming<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<Arc<incoming_datagrams::Incoming>> {
    state(scope, owner)?.incoming.borrow().clone()
}

pub(super) fn configure_incoming<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    if let Some(incoming) = incoming(scope, owner) {
        let (maximum, age) = datagrams::incoming_policy(scope, owner);
        incoming.configure(maximum, age);
    }
}

pub(super) fn remember_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
) -> u64 {
    let native = state(scope, owner).expect("native endpoint owner");
    let id = native.next_stream.get();
    native
        .next_stream
        .set(id.checked_add(1).expect("stream identity overflow"));
    let weak = v8::Weak::new(scope, stream);
    native.streams.borrow_mut().insert(id, weak);
    id
}

pub(super) fn forget_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    id: u64,
) -> Option<v8::Local<'s, v8::Object>> {
    let native = state(scope, owner)?;
    let weak = native.streams.borrow_mut().remove(&id)?;
    weak.to_local(scope)
}

pub(super) fn error_streams<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let Some(native) = state(scope, owner) else {
        return;
    };
    let endpoints = std::mem::take(&mut *native.streams.borrow_mut());
    let endpoints: Vec<_> = endpoints
        .into_values()
        .filter_map(|weak| weak.to_local(scope))
        .collect();
    for endpoint in endpoints {
        streams::error_endpoint(scope, endpoint, error);
    }
}

pub(super) fn cancel<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) {
    if let Some(native) = state(scope, owner) {
        native.stop_ingress();
        native.lease.cancel();
        native.lease.finish();
    }
}

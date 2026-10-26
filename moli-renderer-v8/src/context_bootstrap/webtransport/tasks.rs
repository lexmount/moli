//! Session completion belongs to the transport's networking task source.
//! Promise results belong to the invoking binding's realm. Retired Window
//! Documents and terminated workers drop their browser tasks before dispatch.

use super::*;

const FAILURE_MESSAGE: &str = "__moliWebTransportConnectionFailureMessage";
const SESSION_ERROR: &str = "__moliWebTransportSessionError";

pub(super) fn session_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    message: &str,
) -> v8::Local<'s, v8::Object> {
    if let Some(error) = crate::util::get_private_object(scope, owner, SESSION_ERROR) {
        return error;
    }
    let context = owner
        .get_creation_context(scope)
        .expect("session error realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let error = super::super::build_web_transport_error(
        scope,
        &super::super::WebTransportErrorClonePayload {
            message: message.encode_utf16().collect(),
            source: super::super::WebTransportErrorSource::Session,
            stream_error_code: None,
        },
    )
    .expect("WebTransport session error");
    set_private_value(scope, owner, SESSION_ERROR, error.into());
    error
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ConnectionFailure<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = FAILURE_MESSAGE)]
    message: v8::Local<'s, v8::String>,
}

pub(super) fn queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
) {
    let context = owner
        .get_creation_context(scope)
        .expect("WebTransport task realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Function::builder(callback)
        .data(data)
        .build(scope)
        .expect("WebTransport task");
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: this context's bridge owns the host in the current isolate.
        unsafe { &mut *host }.queue_networking_task(scope, callback);
    } else {
        crate::worker::queue_worker_networking_task(scope, callback);
    }
}

pub(super) fn queue_connection_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    message: &str,
) {
    let message = v8::String::new(scope, message).expect("connection failure message");
    let payload = ConnectionFailure::new(owner, message)
        .bind(scope)
        .expect("failure payload");
    queue(scope, owner, connection_failure, payload.into());
}

fn connection_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let payload =
        v8::Local::<v8::Object>::try_from(args.data()).expect("WebTransport task payload");
    let owner = object(scope, payload, OWNER);
    if connecting(scope, owner) {
        let message = get_private_value(scope, payload, FAILURE_MESSAGE)
            .expect("failure message")
            .to_rust_string_lossy(scope);
        fail(scope, owner, &message);
    }
}

pub(super) fn resolver<'s>(value: v8::Local<'s, v8::Value>) -> v8::Local<'s, v8::PromiseResolver> {
    let object = v8::Local::<v8::Object>::try_from(value).expect("WebTransport private resolver");
    // SAFETY: callers use only private slots/task payloads initialized with a
    // PromiseResolver. Author values and public property lookup never enter.
    unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) }
}

pub(super) fn push<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    value: v8::Local<'s, v8::Value>,
) {
    let index = v8::String::new(scope, &array.length().to_string()).expect("private array index");
    assert_eq!(
        array.create_data_property(scope, index.into(), value),
        Some(true)
    );
}

pub(super) fn array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Array> {
    v8::Local::<v8::Array>::try_from(super::object(scope, object, slot))
        .expect("WebTransport array slot")
}

pub(super) fn wait_for_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !connecting(scope, owner) && backend::connection(scope, owner).is_none() {
        invalid_state(scope);
        return;
    }
    let resolver = v8::PromiseResolver::new(scope).expect("WebTransport operation promise");
    rv.set(resolver.get_promise(scope).into());
    if !connecting(scope, owner) {
        queue_unsupported(scope, owner, resolver.into());
        return;
    }
    let pending = array(scope, owner, PENDING);
    push(scope, pending, resolver.into());
}

pub(super) fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    message: &str,
) -> v8::Local<'s, v8::Object> {
    let context = owner
        .get_creation_context(scope)
        .expect("WebTransport cleanup realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    if get_private_value(scope, owner, STATE)
        .is_some_and(|value| value.to_rust_string_lossy(scope) == "failed")
    {
        let value = get_private_value(scope, owner, CLOSED_RESOLVER).expect("closed resolver");
        let closed = resolver(value).get_promise(scope);
        if closed.state() == v8::PromiseState::Rejected {
            return v8::Local::<v8::Object>::try_from(closed.result(scope)).expect("session error");
        }
    }
    let error = session_error(scope, owner, message);
    let ready_value = get_private_value(scope, owner, READY_RESOLVER).expect("ready resolver");
    let ready = resolver(ready_value);
    let closed_value = get_private_value(scope, owner, CLOSED_RESOLVER).expect("closed resolver");
    let closed = resolver(closed_value);
    let incoming_bidi = object(scope, owner, INCOMING_BIDI);
    let incoming_uni = object(scope, owner, INCOMING_UNI);
    let datagrams = object(scope, owner, DATAGRAMS);
    let pending = array(scope, owner, PENDING);
    let pending_values: Vec<_> = (0..pending.length())
        .map(|index| pending.get_index(scope, index).expect("pending operation"))
        .collect();
    let empty = v8::Array::new(scope, 0);
    set_private_value(scope, owner, PENDING, empty.into());
    let failed = v8str(scope, "failed");
    set_private_value(scope, owner, STATE, failed.into());
    // Commit all session state and retain cleanup snapshots before settling
    // promises or erroring streams, which can expose user Promise hooks.
    let datagrams = datagrams::prepare_failure(scope, datagrams);
    backend::cancel(scope, owner);
    backend::error_streams(scope, owner, error.into());
    for pending in pending_values {
        let object = v8::Local::<v8::Object>::try_from(pending).expect("pending operation");
        let resolver = get_private_value(scope, object, streams::RESOLVER).unwrap_or(pending);
        queue(scope, owner, reject_operation, resolver);
    }
    closed.reject(scope, error.into());
    closed.get_promise(scope).mark_as_handled();
    ready.reject(scope, error.into());
    ready.get_promise(scope).mark_as_handled();
    super::super::stream_adapter::error_stream(scope, incoming_bidi, error.into());
    super::super::stream_adapter::error_stream(scope, incoming_uni, error.into());
    datagrams::finish_failure(scope, datagrams, error.into());
    error
}

fn reject_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let resolver = resolver(args.data());
    let context = resolver
        .get_creation_context(scope)
        .expect("WebTransport operation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let error = super::super::new_dom_exception_value(
        scope,
        "The WebTransport session failed.",
        "InvalidStateError",
    );
    resolver.reject(scope, error);
}

pub(super) fn queue_empty_group_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let resolver = v8::PromiseResolver::new(scope).expect("send group stats promise");
    rv.set(resolver.get_promise(scope).into());
    queue(scope, owner, empty_group_stats, resolver.into());
}

fn empty_group_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let resolver = resolver(args.data());
    let context = resolver
        .get_creation_context(scope)
        .expect("send group stats realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    // No send stream can exist before a session is established. These are
    // exact aggregates of the empty group, not fabricated connection stats.
    let stats = v8::Object::new(scope);
    for name in ["bytesWritten", "bytesSent", "bytesAcknowledged"] {
        let key = v8str(scope, name);
        let zero = v8::Integer::new(scope, 0);
        assert_eq!(
            stats.create_data_property(scope, key.into(), zero.into()),
            Some(true)
        );
    }
    resolver.resolve(scope, stats.into());
}

pub(super) fn queue_unsupported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::Value>,
) {
    queue(scope, owner, unsupported_operation, resolver);
}
fn unsupported_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let resolver = resolver(args.data());
    let context = resolver
        .get_creation_context(scope)
        .expect("operation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let error = super::super::new_dom_exception_value(
        scope,
        "This native WebTransport operation is not available.",
        "NotSupportedError",
    );
    resolver.reject(scope, error);
}

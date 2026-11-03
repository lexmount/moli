//! Fetch body pipes carry snapshotted bytes and network errors. Their source
//! remains independent of the response promise, including synthetic responses
//! which never consume their FetchEvent request body.

use super::*;
use crate::context_bootstrap::stream_adapter::{
    begin_readable_stream_body_consumption, cancel_readable_stream_for_fetch, maybe_pull_stream,
    prepare_readable_stream_read_with_steps, stream_slot_value,
};
use moli_streams::transfer::{SenderReadResultSnapshot, SenderStartReadPlan};

const BYTE_BODY: &str = "__moliFetchBodyByteSender";
const SOURCE_STATE: &str = "__moliFetchBodyTransferState";
const ABORT_CALLBACK: &str = "__moliFetchBodyAbortCallback";
const ABORT_SIGNAL: &str = "__moliFetchBodyAbortSignal";
const READ_STEPS: &str = "__moliFetchBodyReadSteps";

pub(crate) type RequestBodyOwner = std::sync::Arc<
    dyn Fn(ReadableStreamClonePayload, moli_fetch::UploadBodySender, moli_fetch::FetchCancelHandle)
        + Send
        + Sync,
>;

pub(crate) struct RequestBodyProducer {
    stream: Option<ReadableStreamClonePayload>,
    sender: Option<moli_fetch::UploadBodySender>,
    pub(crate) cancel: moli_fetch::FetchCancelHandle,
    registry: SharedMessagePortRegistry,
}

impl std::fmt::Debug for RequestBodyProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestBodyProducer")
            .field("stream", &self.stream)
            .finish_non_exhaustive()
    }
}

impl RequestBodyProducer {
    pub(crate) fn take_stream(&mut self) -> ReadableStreamClonePayload {
        self.stream.take().expect("single body producer delivery")
    }
    pub(crate) fn take_sender(&mut self) -> moli_fetch::UploadBodySender {
        self.sender.take().expect("single body producer sender")
    }
}

impl Drop for RequestBodyProducer {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            self.registry.discard_message_port_channel(stream.port_id());
        }
    }
}

pub(crate) fn request_body_owner(
    scope: &mut v8::PinScope<'_, '_>,
    send: impl Fn(RequestBodyProducer) + Send + Sync + 'static,
) -> RequestBodyOwner {
    let realm = MessagePortRealmBinding::current(scope).expect("request body owner realm");
    let registry = realm.registry().clone();
    std::sync::Arc::new(move |stream, sender, cancel| {
        send(RequestBodyProducer {
            stream: Some(stream),
            sender: Some(sender),
            cancel,
            registry: registry.clone(),
        });
    })
}

pub(crate) fn initialize_request_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &ReadableStreamClonePayload,
) -> Option<()> {
    receiver::initialize_mode(scope, stream, payload, true)
}

pub(crate) fn transfer_request_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    already_locked: bool,
) -> Option<ReadableStreamClonePayload> {
    if !is_readable_stream_object(scope, stream)
        || (!already_locked && readable_stream_access_snapshot(scope, stream).locked())
    {
        let error = state::type_error(scope, "Body stream is locked");
        scope.throw_exception(error);
        return None;
    }
    let realm = MessagePortRealmBinding::current(scope)?;
    let registry = realm.registry().clone();
    let (sender_id, receiver_id) = registry.create_entangled_message_port_pair(realm.owner());
    let Some(port) = ensure_message_port_wrapper_for_id_in_realm(scope, sender_id, &realm) else {
        discard_prepared_port_pair(scope, &registry, None, sender_id);
        return None;
    };
    let state = state::create(scope, stream, port);
    set_stream_slot_bool(scope, state, BYTE_BODY, true);
    let Some((onmessage, onmessageerror)) = build_transfer_port_handlers(
        scope,
        state,
        sender::message_callback,
        sender::messageerror_callback,
    ) else {
        discard_prepared_port_pair(scope, &registry, Some(port), sender_id);
        return None;
    };
    let callbacks = (|| {
        let steps = [
            v8::Function::builder(chunk)
                .data(state.into())
                .build(scope)?,
            v8::Function::builder(close)
                .data(state.into())
                .build(scope)?,
            v8::Function::builder(error)
                .data(state.into())
                .build(scope)?,
        ]
        .map(v8::Local::<v8::Value>::from);
        let steps = v8::Array::new_with_elements(scope, &steps);
        set_stream_slot_value(scope, state, READ_STEPS, steps.into());
        v8::Function::builder(start_callback)
            .data(state.into())
            .build(scope)
    })();
    let Some(start) = callbacks else {
        discard_prepared_port_pair(scope, &registry, Some(port), sender_id);
        return None;
    };
    if !already_locked && !begin_readable_stream_body_consumption(scope, stream) {
        discard_prepared_port_pair(scope, &registry, Some(port), sender_id);
        return None;
    }
    registry.detach_message_port_owner_for_transfer(receiver_id);
    install_transfer_port_handlers(scope, port, onmessage, onmessageerror);
    set_stream_slot_value(scope, stream, SOURCE_STATE, state.into());
    // Author pull algorithms run only after the fetch callback releases its
    // host borrow and installs cancellation, as on the direct network path.
    scope.enqueue_microtask(start);
    Some(ReadableStreamClonePayload {
        port_id: receiver_id,
    })
}

pub(super) fn is_request_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_bool(scope, state, BYTE_BODY).unwrap_or(false)
}

pub(super) fn start_read<'s>(scope: &mut v8::PinScope<'s, '_>, state: v8::Local<'s, v8::Object>) {
    if matches!(
        state::snapshot(scope, state).plan_sender_start_read(),
        SenderStartReadPlan::Ignore
    ) {
        return;
    }
    let Some(stream) = state::stream(scope, state) else {
        return;
    };
    let Some(steps) = stream_slot_array(scope, state, READ_STEPS) else {
        return;
    };
    let callbacks = [0, 1, 2].map(|index| {
        v8::Local::<v8::Function>::try_from(
            steps.get_index(scope, index).expect("body pipe read step"),
        )
        .expect("body pipe read function")
    });
    state::set_read_in_flight(scope, state, true);
    let pull = prepare_readable_stream_read_with_steps(
        scope,
        stream,
        callbacks[0],
        callbacks[1],
        callbacks[2],
    );
    if pull && state::snapshot(scope, state).active() {
        maybe_pull_stream(scope, stream);
    }
}

fn active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (binding, _) = crate::common_runtime::CommonRuntimeBinding::from_scope(scope)?;
    binding.callback_execution_context(scope)?;
    let state = v8::Local::<v8::Object>::try_from(value).ok()?;
    state::snapshot(scope, state).active().then_some(state)
}

fn start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = active(scope, args.data()) {
        start_read(scope, state);
    }
}

fn chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = active(scope, args.data()) else {
        return;
    };
    state::set_read_in_flight(scope, state, false);
    let Ok(chunk) = v8::Local::<v8::Uint8Array>::try_from(args.get(0)) else {
        fail(scope, state, None);
        return;
    };
    // The next source pull can mutate or detach the original backing store.
    let mut bytes = vec![0; chunk.byte_length()];
    let count = chunk.copy_contents(&mut bytes);
    bytes.truncate(count);
    let length = bytes.len();
    let backing = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing);
    let copy = v8::Uint8Array::new(scope, buffer, 0, length).expect("body pipe byte snapshot");
    let plan = state::snapshot(scope, state)
        .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, false));
    sender::apply_read_fulfillment(scope, state, plan, Some(copy.into()));
}

fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = active(scope, args.data()) else {
        return;
    };
    state::set_read_in_flight(scope, state, false);
    let plan = state::snapshot(scope, state)
        .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, true));
    sender::apply_read_fulfillment(scope, state, plan, None);
}

fn error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = active(scope, args.data()) {
        fail(scope, state, None);
    }
}

fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    cancel_reason: Option<v8::Local<'s, v8::Value>>,
) {
    state::set_active(scope, state, false);
    let error = state::type_error(scope, "Failed to fetch");
    let _ = protocol::post(scope, state, protocol::MessageKind::Error, error);
    state::close_port(scope, state);
    remove_abort(scope, state);
    if let Some(stream) = state::stream(scope, state) {
        cancel_readable_stream_for_fetch(scope, stream, cancel_reason.unwrap_or(error));
    }
}

pub(crate) fn install_request_body_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    signal: v8::Local<'s, v8::Object>,
) {
    let Some(signal) = crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal)
    else {
        return;
    };
    let callback = v8::Function::builder(abort)
        .data(stream.into())
        .build(scope)
        .expect("request body abort algorithm");
    set_stream_slot_value(scope, stream, ABORT_CALLBACK, callback.into());
    if signal.is_aborted(scope) {
        let reason = signal.reason(scope);
        cancel_readable_stream_for_fetch(scope, stream, reason);
        return;
    }
    assert!(signal.register_weak_algorithm(scope, callback));
    if let Some(state) = stream_slot_object(scope, stream, SOURCE_STATE) {
        set_stream_slot_value(scope, state, ABORT_SIGNAL, signal.value().into());
    }
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if let Some(state) = stream_slot_object(scope, stream, SOURCE_STATE) {
        fail(scope, state, Some(args.get(0)));
    } else {
        cancel_readable_stream_for_fetch(scope, stream, args.get(0));
    }
}

pub(super) fn remove_abort<'s>(scope: &mut v8::PinScope<'s, '_>, state: v8::Local<'s, v8::Object>) {
    if !is_request_body(scope, state) {
        return;
    }
    if let Some(stream) = state::stream(scope, state) {
        if let Some(signal) = stream_slot_object(scope, state, ABORT_SIGNAL)
            && let Some(callback) = stream_slot_value(scope, stream, ABORT_CALLBACK)
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            && let Some(signal) =
                crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal)
        {
            signal.unregister_algorithm(scope, callback);
        }
        set_stream_slot_value(scope, state, ABORT_SIGNAL, v8::undefined(scope).into());
        set_stream_slot_value(scope, stream, ABORT_CALLBACK, v8::undefined(scope).into());
        set_stream_slot_value(scope, stream, SOURCE_STATE, v8::undefined(scope).into());
    }
}

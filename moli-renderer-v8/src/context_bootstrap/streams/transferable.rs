//! Transferable `ReadableStream` support.
//!
//! Serialization only records the stream's transfer-list index. Transfer
//! setup starts after the complete value has serialized, so a serialization
//! error cannot lock or consume the source stream. Individual transfer steps
//! then commit in transfer-list order, matching the platform's irreversible,
//! non-transactional transfer semantics.

use super::*;
use crate::{
    context_bootstrap::{
        message_ports::{
            MessagePortRealmBinding, close_message_port_object, current_message_port_registry,
            discard_message_port_channel, ensure_message_port_wrapper_for_id_in_realm,
            schedule_message_port_delivery, set_internal_message_port_handlers,
        },
        stream_adapter::{
            EnqueueChunkError, READABLE_STREAM_ALGORITHM_CANCEL_INDEX,
            READABLE_STREAM_ALGORITHM_PULL_INDEX, READABLE_STREAM_ALGORITHM_SOURCE_INDEX,
            StreamOwnerPublication, apply_readable_stream_access_transition,
            build_required_stream_callback, error_writable_stream_with_value,
            initialize_readable_stream_object, initialize_transform_stream_endpoints,
            initialize_writable_stream_object, new_pending_read_promise,
            new_readable_stream_shell_object, new_transform_stream_shell_object,
            new_writable_stream_shell_object, publish_required_stream_promise_reactions,
            read_from_stream_as_promise, readable_stream_access_snapshot, reject_pending_read,
            rejected_promise_value, resolve_pending_promise, set_stream_slot_bool,
            set_stream_slot_value, stream_slot_array, stream_slot_bool, stream_slot_number,
            stream_slot_object, suppress_promise_unhandled_rejection, writable_stream_snapshot,
        },
    },
    message_port_runtime::SharedMessagePortRegistry,
    structured_clone::V8StructuredClonePayload,
    types::MessagePortId,
};
use moli_streams::{
    readable::{ReadableAccessTransition, ReadableLockPlan},
    transfer::{TransferEntryPlan, TransferEntrySnapshot},
};

mod protocol;
mod receiver;
mod sender;
mod state;
mod writable;
mod writable_state;

#[derive(Clone, Debug)]
pub(crate) struct ReadableStreamClonePayload {
    pub(crate) port_id: MessagePortId,
}

#[derive(Clone, Debug)]
pub(crate) struct WritableStreamClonePayload {
    pub(crate) port_id: MessagePortId,
}

#[derive(Clone, Debug)]
pub(crate) struct TransformStreamClonePayload {
    pub(crate) readable: ReadableStreamClonePayload,
    pub(crate) writable: WritableStreamClonePayload,
}

impl ReadableStreamClonePayload {
    pub(crate) const fn port_id(&self) -> MessagePortId {
        self.port_id
    }

    pub(crate) fn discard_port(&self, scope: &mut v8::PinScope<'_, '_>) {
        discard_clone_port(scope, self.port_id);
    }
}

impl WritableStreamClonePayload {
    pub(crate) const fn port_id(&self) -> MessagePortId {
        self.port_id
    }

    pub(crate) fn discard_port(&self, scope: &mut v8::PinScope<'_, '_>) {
        discard_clone_port(scope, self.port_id);
    }
}

impl TransformStreamClonePayload {
    pub(crate) const fn port_ids(&self) -> [MessagePortId; 2] {
        [self.readable.port_id(), self.writable.port_id()]
    }

    pub(crate) fn discard_ports(&self, scope: &mut v8::PinScope<'_, '_>) {
        self.readable.discard_port(scope);
        self.writable.discard_port(scope);
    }
}

/// A transfer whose fallible setup is complete but which has not changed the
/// source stream yet.
pub(crate) struct PreparedReadableStreamTransfer<'s> {
    registry: SharedMessagePortRegistry,
    stream: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
    sender_port: v8::Local<'s, v8::Object>,
    sender_port_id: MessagePortId,
    receiver_port_id: MessagePortId,
    onmessage: v8::Local<'s, v8::Function>,
    onmessageerror: v8::Local<'s, v8::Function>,
    access: ReadableAccessTransition,
}

impl<'s> PreparedReadableStreamTransfer<'s> {
    /// Make the transfer author-observable. All operations here are infallible;
    /// fallible V8 and channel setup belongs in `prepare_readable_stream_transfer`.
    /// Structured-clone preparation and commit run synchronously without
    /// author JavaScript, so the captured access transition remains immediate.
    pub(crate) fn commit(self, scope: &mut v8::PinScope<'s, '_>) -> ReadableStreamClonePayload {
        self.registry
            .detach_message_port_owner_for_transfer(self.receiver_port_id);
        install_transfer_port_handlers(
            scope,
            self.sender_port,
            self.onmessage,
            self.onmessageerror,
        );
        apply_readable_stream_access_transition(scope, self.stream, self.access);
        sender::start_read(scope, self.state);
        ReadableStreamClonePayload {
            port_id: self.receiver_port_id,
        }
    }

    fn discard(self, scope: &mut v8::PinScope<'s, '_>) {
        discard_prepared_port_pair(
            scope,
            &self.registry,
            Some(self.sender_port),
            self.sender_port_id,
        );
    }
}

pub(crate) struct PreparedWritableStreamTransfer<'s> {
    registry: SharedMessagePortRegistry,
    stream: v8::Local<'s, v8::Object>,
    transfer_readable: v8::Local<'s, v8::Object>,
    receiver_port_id: MessagePortId,
}

impl<'s> PreparedWritableStreamTransfer<'s> {
    pub(crate) fn commit(self, scope: &mut v8::PinScope<'s, '_>) -> WritableStreamClonePayload {
        self.registry
            .detach_message_port_owner_for_transfer(self.receiver_port_id);
        let pipe = super::readable::start_internal_readable_stream_pipe_to(
            scope,
            self.transfer_readable,
            self.stream,
        )
        .expect("a prepared WritableStream transfer must create its internal pipe");
        suppress_promise_unhandled_rejection(scope, pipe);
        WritableStreamClonePayload {
            port_id: self.receiver_port_id,
        }
    }
}

pub(crate) struct PreparedTransformStreamTransfer<'s> {
    readable: PreparedReadableStreamTransfer<'s>,
    writable: PreparedWritableStreamTransfer<'s>,
}

impl<'s> PreparedTransformStreamTransfer<'s> {
    pub(crate) fn commit(self, scope: &mut v8::PinScope<'s, '_>) -> TransformStreamClonePayload {
        TransformStreamClonePayload {
            readable: self.readable.commit(scope),
            writable: self.writable.commit(scope),
        }
    }
}

pub(crate) fn prepare_readable_stream_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PreparedReadableStreamTransfer<'s>> {
    if !is_readable_stream_object(scope, stream) {
        throw_transfer_data_clone_error(scope, "Value is not a ReadableStream.");
        return None;
    }
    let access_snapshot = readable_stream_access_snapshot(scope, stream);
    match TransferEntrySnapshot::new(access_snapshot.locked()).plan() {
        TransferEntryPlan::RejectLocked => {
            throw_transfer_data_clone_error(scope, "ReadableStream is already locked.");
            return None;
        }
        TransferEntryPlan::Prepare => {}
    }
    let ReadableLockPlan::Lock(access) = access_snapshot.plan_lock() else {
        unreachable!("transfer admission already rejected a locked stream")
    };

    let Some(realm) = MessagePortRealmBinding::current(scope) else {
        throw_transfer_data_clone_error(
            scope,
            "ReadableStream transfer has no execution-context port binding.",
        );
        return None;
    };
    let registry = realm.registry().clone();
    let (sender_port_id, receiver_port_id) =
        registry.create_entangled_message_port_pair(realm.owner());
    let Some(sender_port) =
        ensure_message_port_wrapper_for_id_in_realm(scope, sender_port_id, &realm)
    else {
        discard_prepared_port_pair(scope, &registry, None, sender_port_id);
        return None;
    };
    let state = state::create(scope, stream, sender_port);
    let Some((onmessage, onmessageerror)) = build_transfer_port_handlers(
        scope,
        state,
        sender::message_callback,
        sender::messageerror_callback,
    ) else {
        discard_prepared_port_pair(scope, &registry, Some(sender_port), sender_port_id);
        return None;
    };

    Some(PreparedReadableStreamTransfer {
        registry,
        stream,
        state,
        sender_port,
        sender_port_id,
        receiver_port_id,
        onmessage,
        onmessageerror,
        access,
    })
}

pub(crate) fn prepare_writable_stream_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PreparedWritableStreamTransfer<'s>> {
    if !is_writable_stream_object(scope, stream) {
        throw_transfer_data_clone_error(scope, "Value is not a WritableStream.");
        return None;
    }
    if matches!(
        TransferEntrySnapshot::new(writable_stream_snapshot(scope, stream).locked()).plan(),
        TransferEntryPlan::RejectLocked
    ) {
        throw_transfer_data_clone_error(scope, "WritableStream is already locked.");
        return None;
    }

    let Some(realm) = MessagePortRealmBinding::current(scope) else {
        throw_transfer_data_clone_error(
            scope,
            "WritableStream transfer has no execution-context port binding.",
        );
        return None;
    };
    let registry = realm.registry().clone();
    let (sender_port_id, receiver_port_id) =
        registry.create_entangled_message_port_pair(realm.owner());
    let Some(sender_port) =
        ensure_message_port_wrapper_for_id_in_realm(scope, sender_port_id, &realm)
    else {
        discard_prepared_port_pair(scope, &registry, None, sender_port_id);
        return None;
    };
    let Some(transfer_readable) = receiver::new_from_port(scope, sender_port) else {
        discard_prepared_port_pair(scope, &registry, Some(sender_port), sender_port_id);
        return None;
    };

    Some(PreparedWritableStreamTransfer {
        registry,
        stream,
        transfer_readable,
        receiver_port_id,
    })
}

pub(crate) fn prepare_transform_stream_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PreparedTransformStreamTransfer<'s>> {
    let Some(readable) = stream_slot_object(scope, stream, TRANSFORM_STREAM_READABLE_SLOT)
        .filter(|readable| is_readable_stream_object(scope, *readable))
    else {
        throw_transfer_data_clone_error(scope, "Value is not a TransformStream.");
        return None;
    };
    let Some(writable) = stream_slot_object(scope, stream, TRANSFORM_STREAM_WRITABLE_SLOT)
        .filter(|writable| is_writable_stream_object(scope, *writable))
    else {
        throw_transfer_data_clone_error(scope, "Value is not a TransformStream.");
        return None;
    };
    if readable_stream_access_snapshot(scope, readable).locked()
        || writable_stream_snapshot(scope, writable).locked()
    {
        throw_transfer_data_clone_error(scope, "TransformStream is already locked.");
        return None;
    }

    let readable_transfer = prepare_readable_stream_transfer(scope, readable)?;
    let Some(writable_transfer) = prepare_writable_stream_transfer(scope, writable) else {
        readable_transfer.discard(scope);
        return None;
    };
    Some(PreparedTransformStreamTransfer {
        readable: readable_transfer,
        writable: writable_transfer,
    })
}

pub(crate) fn build_readable_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    new_readable_stream_shell_object(scope)
}

pub(crate) fn initialize_readable_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &ReadableStreamClonePayload,
) -> Option<()> {
    receiver::initialize(scope, stream, payload)
}

pub(crate) fn build_writable_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    new_writable_stream_shell_object(scope)
}

pub(crate) fn initialize_writable_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &WritableStreamClonePayload,
) -> Option<()> {
    writable::initialize(scope, stream, payload)
}

pub(crate) fn build_transform_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    new_transform_stream_shell_object(scope)
}

pub(crate) fn initialize_transform_stream_clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &TransformStreamClonePayload,
) -> Option<()> {
    let readable = build_readable_stream_clone_shell(scope);
    let writable = build_writable_stream_clone_shell(scope);
    initialize_readable_stream_clone_shell(scope, readable, &payload.readable)?;
    initialize_writable_stream_clone_shell(scope, writable, &payload.writable)?;
    initialize_transform_stream_endpoints(scope, stream, readable, writable);
    Some(())
}

fn discard_clone_port(scope: &mut v8::PinScope<'_, '_>, port_id: MessagePortId) {
    discard_message_port_channel(scope, port_id);
}

fn build_transfer_port_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    onmessage: impl v8::MapFnTo<v8::FunctionCallback>,
    onmessageerror: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
    let StreamOwnerPublication::Published(onmessage) = build_required_stream_callback(
        scope,
        v8::Function::builder(onmessage).data(state.into()),
        "transferred stream port message handler",
    ) else {
        return None;
    };
    let StreamOwnerPublication::Published(onmessageerror) = build_required_stream_callback(
        scope,
        v8::Function::builder(onmessageerror).data(state.into()),
        "transferred stream port message-error handler",
    ) else {
        return None;
    };
    Some((onmessage, onmessageerror))
}

fn install_transfer_port_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    onmessage: v8::Local<'s, v8::Function>,
    onmessageerror: v8::Local<'s, v8::Function>,
) {
    set_internal_message_port_handlers(scope, port, onmessage, onmessageerror);
}

fn discard_prepared_port_pair<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &SharedMessagePortRegistry,
    sender_port: Option<v8::Local<'s, v8::Object>>,
    sender_port_id: MessagePortId,
) {
    registry.discard_message_port_channel(sender_port_id);
    if let Some(sender_port) = sender_port {
        close_message_port_object(scope, sender_port);
    }
}

fn throw_transfer_data_clone_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let exception = new_dom_exception_value(scope, message, "DataCloneError");
    scope.throw_exception(exception);
}

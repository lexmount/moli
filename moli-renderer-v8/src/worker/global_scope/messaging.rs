//! Worker postMessage, structured cloning, MessagePorts and BroadcastChannels.

use super::*;

#[derive(Default, WebApiObject)]
#[webapi(plain)]
pub(super) struct DedicatedWorkerGlobalPostMessageDeclaration {
    #[webapi(method = "postMessage", callback = worker_post_message_callback, length = 1)]
    post_message: (),
}

/// Mutable state accessible from V8 callbacks inside the worker isolate.
pub(in crate::worker) struct WorkerMessagePortWrapperEntry {
    wrapper: v8::Global<v8::Object>,
}

pub(crate) fn worker_message_port_wake_sender(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<mpsc::UnboundedSender<crate::worker::handle::WorkerMessage>> {
    Some(get_worker_state(scope)?.borrow().worker_wake_tx.clone())
}

pub(crate) fn worker_message_port_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedMessagePortRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .message_port_registry
            .clone(),
    )
}

pub(crate) fn worker_broadcast_channel_wake_sender(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<mpsc::UnboundedSender<crate::worker::handle::WorkerMessage>> {
    Some(get_worker_state(scope)?.borrow().worker_wake_tx.clone())
}

pub(crate) fn worker_broadcast_channel_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedBroadcastChannelRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .broadcast_channel_registry
            .clone(),
    )
}

pub(crate) fn worker_broadcast_channel_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<MoliStorageKey> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .broadcast_channel_storage_key
            .clone(),
    )
}

pub(crate) fn register_worker_message_port_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    port: v8::Local<'_, v8::Object>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state.borrow_mut().message_port_wrappers.insert(
        port_id,
        WorkerMessagePortWrapperEntry {
            wrapper: v8::Global::new(scope, port),
        },
    );
}

pub(crate) fn register_shared_worker_connection_port(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .shared_worker_connection_ports
        .insert(port_id);
}

pub(crate) fn forget_worker_message_port_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    if let Some(state) = get_worker_state(scope) {
        state.borrow_mut().message_port_wrappers.remove(&port_id);
    }
}

pub(crate) fn worker_message_port_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    state
        .message_port_wrappers
        .get(&port_id)
        .map(|entry| v8::Local::new(scope, &entry.wrapper))
}

pub(crate) fn register_worker_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
    channel: v8::Local<'_, v8::Object>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .broadcast_channel_wrappers
        .insert(channel_id, v8::Global::new(scope, channel));
}

pub(crate) fn forget_worker_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .broadcast_channel_wrappers
        .remove(&channel_id);
}

pub(crate) fn worker_broadcast_channel_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel_id: BroadcastChannelId,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    state
        .broadcast_channel_wrappers
        .get(&channel_id)
        .map(|handle| v8::Local::new(scope, handle))
}

pub(in crate::worker) fn close_worker_owned_message_ports(state: &Rc<RefCell<WorkerGlobalState>>) {
    let (registry, port_ids): (SharedMessagePortRegistry, Vec<MessagePortId>) = {
        let mut state = state.borrow_mut();
        let shared_worker_connection_ports =
            std::mem::take(&mut state.shared_worker_connection_ports);
        (
            state.message_port_registry.clone(),
            state
                .message_port_wrappers
                .drain()
                .filter_map(|(port_id, _)| {
                    (!shared_worker_connection_ports.contains(&port_id)).then_some(port_id)
                })
                .collect(),
        )
    };
    for port_id in port_ids {
        registry.close_message_port(port_id);
    }
}

pub(in crate::worker) fn close_worker_owned_broadcast_channels(
    state: &Rc<RefCell<WorkerGlobalState>>,
) {
    let (registry, channel_ids): (SharedBroadcastChannelRegistry, Vec<BroadcastChannelId>) = {
        let mut state = state.borrow_mut();
        (
            state.broadcast_channel_registry.clone(),
            state
                .broadcast_channel_wrappers
                .drain()
                .map(|(channel_id, _)| channel_id)
                .collect(),
        )
    };
    for channel_id in channel_ids {
        registry.close_broadcast_channel(channel_id);
    }
}

fn worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'DedicatedWorkerGlobalScope': 1 argument required, but only 0 present.",
        );
        return;
    }
    let val = args.get(0);
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(data) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        val,
        transfer_arg,
        "DedicatedWorkerGlobalScope",
    ) else {
        return;
    };
    let _ = state
        .borrow()
        .parent_tx
        .send(WorkerToParentMessage::Post(data));
}

pub(super) fn worker_structured_clone_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = crate::context_bootstrap::structured_clone_value_with_options(
        scope,
        args.get(0),
        args.get(1),
    ) {
        rv.set(value);
    } else {
        rv.set_undefined();
    }
}

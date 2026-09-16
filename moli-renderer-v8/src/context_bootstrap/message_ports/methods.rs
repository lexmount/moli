use super::*;

pub(in crate::context_bootstrap) fn message_port_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let port = args.this();
    if message_port_is_closed(scope, port) {
        rv.set_undefined();
        return;
    }
    let Some(port_id) = message_port_id_from_object(scope, port) else {
        rv.set_undefined();
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(transfers) =
        parse_post_message_transfer_list(scope, transfer_arg, "MessagePort", Some(port_id))
    else {
        return;
    };
    // Web IDL conversion precedes destination selection. Serialization follows
    // it, so closing or retiring the source in a data getter does not redirect
    // or cancel the message already being sent to a live destination.
    retire_message_port_if_owner_is_stale(scope, port_id);
    let registry = current_message_port_registry(scope);
    let peer_id = registry
        .as_ref()
        .and_then(|registry| registry.message_port_peer_id(port_id));
    let data = transfers.serialize(scope, args.get(0));
    // Serialization can run getters that destroy the source document.
    retire_message_port_if_owner_is_stale(scope, port_id);
    let Some(data) = data else {
        return;
    };
    if let Some(peer_id) = peer_id
        && !data.transferred_message_ports().contains(&peer_id)
        && let Some(registry) = registry
        && registry.enqueue_message_to_endpoint(peer_id, data)
    {
        // Publish directly to the peer's stable owner route. The Page scheduler
        // cannot execute the task until the current PageVm turn is restored;
        // Worker peers are separate agents and may progress concurrently.
        schedule_message_port_delivery(scope, peer_id);
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn message_port_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_message_port_started(scope, args.this(), true);
    if let Some(port_id) = message_port_id_from_object(scope, args.this())
        && let Some(registry) = current_message_port_registry(scope)
    {
        registry.wake_message_port_if_pending(port_id);
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn message_port_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(port_id) = message_port_id_from_object(scope, args.this())
        && let Some(registry) = current_message_port_registry(scope)
    {
        registry.close_message_port(port_id);
    }
    close_message_port_object(scope, args.this());
    rv.set_undefined();
}

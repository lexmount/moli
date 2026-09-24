use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MessageChannel)]
struct MessageChannelObjectDeclaration<'scope> {
    #[webapi(slot = MESSAGE_CHANNEL_PORT1_SLOT)]
    port1: v8::Local<'scope, v8::Object>,
    #[webapi(slot = MESSAGE_CHANNEL_PORT2_SLOT)]
    port2: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap) fn message_channel_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'MessageChannel': Please use the 'new' operator.",
        );
        return;
    }

    let Some(realm) = MessagePortRealmBinding::current(scope) else {
        if message_channel_context_is_destroyed(scope) {
            // A retained constructor still creates branded objects after its
            // document is destroyed, but those ports have no live endpoint.
            let Some(port1) = new_detached_message_port_object(scope) else {
                return;
            };
            let Some(port2) = new_detached_message_port_object(scope) else {
                return;
            };
            MessageChannelObjectDeclaration::new(port1, port2)
                .initialize(scope, args.this())
                .expect("MessageChannel declaration should initialize detached ports");
            rv.set(args.this().into());
            return;
        }
        throw_type_error(
            scope,
            "Failed to construct 'MessageChannel': Execution context is unavailable.",
        );
        return;
    };
    let (port1_id, port2_id) = realm
        .registry()
        .create_entangled_message_port_pair(realm.owner());

    let Some(port1) = new_message_port_object(scope, port1_id, &realm) else {
        realm.discard_channel(scope, port1_id);
        throw_type_error(
            scope,
            "Failed to construct 'MessageChannel': MessagePort initialization failed.",
        );
        return;
    };
    let Some(port2) = new_message_port_object(scope, port2_id, &realm) else {
        realm.discard_channel(scope, port1_id);
        throw_type_error(
            scope,
            "Failed to construct 'MessageChannel': MessagePort initialization failed.",
        );
        return;
    };

    set_message_port_peer(scope, port1, port2);
    set_message_port_peer(scope, port2, port1);
    MessageChannelObjectDeclaration::new(port1, port2)
        .initialize(scope, args.this())
        .expect("MessageChannel declaration should initialize ports");
    rv.set(args.this().into());
}

fn message_channel_context_is_destroyed(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    if context
        .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
        .is_none()
    {
        return false;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    host.window_execution_context_identity_for_v8_context(scope, context)
        .is_none_or(|identity| !host.window_execution_context_identity_is_current(identity))
}

pub(in crate::context_bootstrap) fn message_port_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(
        scope,
        "Failed to construct 'MessagePort': Illegal constructor.",
    );
}

use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "MessageChannel")]
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

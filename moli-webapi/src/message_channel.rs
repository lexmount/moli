//! MessageChannel binds the two ports supplied by the embedding runtime.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{callback_data_index_value, callback_data_item, get_private_value};
use moli_webapi_declare::{BindError, WebApiFunctionTemplate, WebApiObject, web_api_object_target};

const MESSAGE_CHANNEL_PORT1_SLOT: &str = "__moliMessageChannelPort1";
const MESSAGE_CHANNEL_PORT2_SLOT: &str = "__moliMessageChannelPort2";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MessageChannel)]
struct MessageChannelObjectDeclaration<'scope> {
    #[webapi(slot = MESSAGE_CHANNEL_PORT1_SLOT)]
    port1: v8::Local<'scope, v8::Object>,
    #[webapi(slot = MESSAGE_CHANNEL_PORT2_SLOT)]
    port2: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MessageChannel)]
struct MessageChannelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = message_channel_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    port1: (),
    #[webapi(
        accessor_property,
        getter = message_channel_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    port2: (),
}

pub fn initialize_message_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
    port1: v8::Local<'s, v8::Object>,
    port2: v8::Local<'s, v8::Object>,
) -> Result<(), BindError> {
    MessageChannelObjectDeclaration::new(port1, port2).initialize(scope, channel)
}

pub fn install_prototype_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    MessageChannelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn message_channel_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        MESSAGE_CHANNEL_ATTRIBUTE_SLOTS,
        "MessageChannel attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        web_api_object_target(scope, args.this())
            .and_then(|channel| get_private_value(scope, channel, slot))
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const MESSAGE_CHANNEL_ATTRIBUTE_SLOTS: &[&str] =
    &[MESSAGE_CHANNEL_PORT1_SLOT, MESSAGE_CHANNEL_PORT2_SLOT];

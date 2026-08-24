use super::*;
use crate::{
    message_port_runtime::{MessagePortOwner, SharedMessagePortRegistry},
    types::MessagePortId,
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    worker::{
        forget_worker_message_port_wrapper, register_worker_message_port_wrapper,
        worker_message_port_registry, worker_message_port_wake_sender, worker_message_port_wrapper,
    },
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const MESSAGE_PORT_ID_SLOT: &str = "__lmMessagePortId";

/// Operation-entry binding for one Window or Worker realm.
///
/// Port-pair creation and wrapper publication must use one captured binding.
/// Re-resolving ambient child or lightweight-popup markers midway through the
/// operation can select a different owner and expose a partially initialized
/// `MessageChannel` receiver.
pub(crate) enum MessagePortRealmBinding {
    Page {
        registry: SharedMessagePortRegistry,
        owner: MessagePortOwner,
        identity: crate::native_bridge::WindowExecutionContextIdentity,
    },
    Worker {
        registry: SharedMessagePortRegistry,
        owner: MessagePortOwner,
    },
}

impl MessagePortRealmBinding {
    pub(crate) fn current(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        if let Some(host) = crate::util::context_host_from_global_bridge(scope) {
            let identity = host.current_runtime_window_execution_context_identity(scope)?;
            let owner = MessagePortOwner::Page(
                host.page_message_port_delivery_sender()
                    .bind_execution_context(identity),
            );
            return Some(Self::Page {
                registry: host.message_port_registry(),
                owner,
                identity,
            });
        }
        Some(Self::Worker {
            registry: worker_message_port_registry(scope)?,
            owner: MessagePortOwner::Worker(worker_message_port_wake_sender(scope)?),
        })
    }

    pub(crate) fn registry(&self) -> &SharedMessagePortRegistry {
        match self {
            Self::Page { registry, .. } | Self::Worker { registry, .. } => registry,
        }
    }

    pub(crate) fn owner(&self) -> MessagePortOwner {
        match self {
            Self::Page { owner, .. } | Self::Worker { owner, .. } => owner.clone(),
        }
    }

    fn register_wrapper(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        port_id: MessagePortId,
        port: v8::Local<'_, v8::Object>,
    ) -> bool {
        match self {
            Self::Page { identity, .. } => {
                if crate::native_bridge::current_runtime_observable_context_token(scope)
                    != Some(identity.realm_token())
                {
                    return false;
                }
                let Some(host) = crate::util::context_host_from_global_bridge(scope) else {
                    return false;
                };
                if !host.window_execution_context_owner_is_current(
                    identity.owner(),
                    identity.dispatch_scope(),
                ) {
                    return false;
                }
                host.register_message_port_wrapper(scope, port_id, port, *identity);
            }
            Self::Worker { .. } => {
                if context_host_ptr_from_global_bridge(scope).is_some() {
                    return false;
                }
                register_worker_message_port_wrapper(scope, port_id, port);
            }
        }
        true
    }

    fn forget_wrapper(&self, scope: &mut v8::PinScope<'_, '_>, port_id: MessagePortId) {
        match self {
            Self::Page { .. } => {
                if let Some(host) = crate::util::context_host_from_global_bridge(scope) {
                    host.forget_message_port_wrapper(port_id);
                }
            }
            Self::Worker { .. } => forget_worker_message_port_wrapper(scope, port_id),
        }
    }

    pub(crate) fn discard_channel(&self, scope: &mut v8::PinScope<'_, '_>, port_id: MessagePortId) {
        for discarded_port_id in self.registry().discard_message_port_channel(port_id) {
            self.forget_wrapper(scope, discarded_port_id);
        }
    }
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct MessagePortObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,

    #[webapi(slot = MESSAGE_PORT_ID_SLOT)]
    port_id: v8::Local<'scope, v8::BigInt>,

    #[webapi(slot = MESSAGE_PORT_PEER_SLOT, init = "undefined")]
    peer: (),

    #[webapi(slot = MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT, init = "null")]
    onmessage_handler: (),
    #[webapi(slot = MESSAGE_PORT_ONMESSAGE_ORDER_SLOT, init = "undefined")]
    onmessage_order: (),

    #[webapi(slot = MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT, init = "null")]
    onmessageerror_handler: (),
    #[webapi(slot = MESSAGE_PORT_ONMESSAGEERROR_ORDER_SLOT, init = "undefined")]
    onmessageerror_order: (),

    #[webapi(slot = MESSAGE_PORT_ONCLOSE_HANDLER_SLOT, init = "null")]
    onclose_handler: (),
    #[webapi(slot = MESSAGE_PORT_ONCLOSE_ORDER_SLOT, init = "undefined")]
    onclose_order: (),

    #[webapi(slot = MESSAGE_PORT_NEXT_LISTENER_ORDER_SLOT, init = 0)]
    next_listener_order: (),
    #[webapi(slot = MESSAGE_PORT_STARTED_SLOT, init = false)]
    started: (),
    #[webapi(slot = MESSAGE_PORT_CLOSED_SLOT, init = false)]
    closed: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MessagePort", enumerable)]
struct MessagePortPrototypeDeclaration {
    #[webapi(method, length = 1, callback = message_port_post_message_callback)]
    post_message: (),
    #[webapi(method, length = 0, callback = message_port_start_callback)]
    start: (),
    #[webapi(method, length = 0, callback = message_port_close_callback)]
    close: (),
    #[webapi(method, length = 2, callback = message_port_add_event_listener_callback)]
    add_event_listener: (),
    #[webapi(
        method,
        length = 2,
        callback = message_port_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(
        accessor_property,
        getter = message_port_onmessage_getter_callback,
        setter = message_port_onmessage_setter_callback,
        enumerable
    )]
    onmessage: (),
    #[webapi(
        accessor_property,
        getter = message_port_onmessageerror_getter_callback,
        setter = message_port_onmessageerror_setter_callback,
        enumerable
    )]
    onmessageerror: (),
    #[webapi(
        accessor_property,
        getter = message_port_onclose_getter_callback,
        setter = message_port_onclose_setter_callback,
        enumerable
    )]
    onclose: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MessageChannel")]
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

pub(in crate::context_bootstrap) fn install_message_port_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "MessageChannel" => {
            MessageChannelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "MessagePort" => {
            MessagePortPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
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
        message_port_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const MESSAGE_CHANNEL_ATTRIBUTE_SLOTS: &[&str] =
    &[MESSAGE_CHANNEL_PORT1_SLOT, MESSAGE_CHANNEL_PORT2_SLOT];

pub(in crate::context_bootstrap::message_ports) fn new_message_port_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    realm: &MessagePortRealmBinding,
) -> Option<v8::Local<'s, v8::Object>> {
    let port = message_port_object_declaration(scope, port_id)?
        .bind(scope)
        .ok()?;
    if !realm.register_wrapper(scope, port_id, port) {
        return None;
    }
    Some(port)
}

fn message_port_object_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<MessagePortObjectDeclaration<'s>> {
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "MessagePort",
    )
    .ok()?;
    let port_id = v8::BigInt::new_from_u64(scope, port_id);
    Some(MessagePortObjectDeclaration::new(prototype, port_id))
}

pub(crate) fn message_port_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<MessagePortId> {
    let value = get_private_value(scope, port, MESSAGE_PORT_ID_SLOT)?;
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (n, lossless) = big.u64_value();
        return lossless.then_some(n);
    }
    None
}

pub(crate) fn ensure_message_port_wrapper_for_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(port) = message_port_wrapper_for_id(scope, port_id) {
        return Some(port);
    }
    let realm = MessagePortRealmBinding::current(scope)?;
    ensure_message_port_wrapper_for_id_in_realm(scope, port_id, &realm)
}

pub(crate) fn ensure_message_port_wrapper_for_id_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    realm: &MessagePortRealmBinding,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(port) = message_port_wrapper_for_id(scope, port_id) {
        return Some(port);
    }
    let port = new_message_port_object(scope, port_id, realm)?;
    realm
        .registry()
        .attach_message_port_owner(port_id, realm.owner());
    Some(port)
}

pub(crate) fn detach_transferred_message_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) {
    let Some(port_id) = message_port_id_from_object(scope, port) else {
        return;
    };
    forget_message_port_wrapper(scope, port_id);
    set_private_value(
        scope,
        port,
        MESSAGE_PORT_ID_SLOT,
        v8::undefined(scope).into(),
    );
    set_message_port_bool_slot(scope, port, MESSAGE_PORT_CLOSED_SLOT, true);
}

pub(in crate::context_bootstrap) fn close_message_port_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) {
    if let Some(port_id) = message_port_id_from_object(scope, port) {
        let retained_for_queued_delivery = current_message_port_registry(scope)
            .is_some_and(|registry| registry.contains_message_port(port_id));
        if !retained_for_queued_delivery {
            forget_message_port_wrapper(scope, port_id);
        }
    }
    set_private_value(
        scope,
        port,
        MESSAGE_PORT_ID_SLOT,
        v8::undefined(scope).into(),
    );
    set_message_port_bool_slot(scope, port, MESSAGE_PORT_CLOSED_SLOT, true);
}

pub(in crate::context_bootstrap) fn current_message_port_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedMessagePortRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return Some(unsafe { &*host_ptr }.message_port_registry());
    }
    worker_message_port_registry(scope)
}

pub(crate) fn detach_message_port_owner_for_transfer(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    if let Some(registry) = current_message_port_registry(scope) {
        registry.detach_message_port_owner_for_transfer(port_id);
    }
}

pub(in crate::context_bootstrap::message_ports) fn message_port_wrapper_for_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return unsafe { &mut *host_ptr }.message_port_wrapper(scope, port_id);
    }
    worker_message_port_wrapper(scope, port_id)
}

pub(in crate::context_bootstrap::message_ports) fn forget_message_port_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.forget_message_port_wrapper(port_id);
        return;
    }
    forget_worker_message_port_wrapper(scope, port_id);
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onmessage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Function>> {
    message_port_event_handler(scope, port, MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onmessage_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    message_port_event_handler_order(scope, port, MESSAGE_PORT_ONMESSAGE_ORDER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onmessageerror<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Function>> {
    message_port_event_handler(scope, port, MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onmessageerror_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    message_port_event_handler_order(scope, port, MESSAGE_PORT_ONMESSAGEERROR_ORDER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onclose<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Function>> {
    message_port_event_handler(scope, port, MESSAGE_PORT_ONCLOSE_HANDLER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_onclose_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    message_port_event_handler_order(scope, port, MESSAGE_PORT_ONCLOSE_ORDER_SLOT)
}

pub(in crate::context_bootstrap::message_ports) fn next_message_port_listener_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> f64 {
    let order =
        message_port_number_slot(scope, port, MESSAGE_PORT_NEXT_LISTENER_ORDER_SLOT).unwrap_or(0.0);
    set_message_port_number_slot(
        scope,
        port,
        MESSAGE_PORT_NEXT_LISTENER_ORDER_SLOT,
        order + 1.0,
    );
    order
}

pub(in crate::context_bootstrap::message_ports) fn message_port_is_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> bool {
    message_port_bool_slot(scope, port, MESSAGE_PORT_STARTED_SLOT).unwrap_or(false)
}

pub(in crate::context_bootstrap::message_ports) fn message_port_is_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> bool {
    message_port_bool_slot(scope, port, MESSAGE_PORT_CLOSED_SLOT).unwrap_or(false)
}

fn message_port_onmessage_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        message_port_event_handler_value(scope, args.this(), MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT);
    rv.set(value);
}

fn message_port_onmessage_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_message_port_event_handler(
        scope,
        args.this(),
        args.get(0),
        MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT,
        MESSAGE_PORT_ONMESSAGE_ORDER_SLOT,
    );
    rv.set_undefined();
}

fn message_port_onclose_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        message_port_event_handler_value(scope, args.this(), MESSAGE_PORT_ONCLOSE_HANDLER_SLOT);
    rv.set(value);
}

fn message_port_onmessageerror_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = message_port_event_handler_value(
        scope,
        args.this(),
        MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT,
    );
    rv.set(value);
}

fn message_port_onmessageerror_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_message_port_event_handler(
        scope,
        args.this(),
        args.get(0),
        MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT,
        MESSAGE_PORT_ONMESSAGEERROR_ORDER_SLOT,
    );
    rv.set_undefined();
}

fn message_port_onclose_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_message_port_event_handler(
        scope,
        args.this(),
        args.get(0),
        MESSAGE_PORT_ONCLOSE_HANDLER_SLOT,
        MESSAGE_PORT_ONCLOSE_ORDER_SLOT,
    );
    rv.set_undefined();
}

fn message_port_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Value> {
    let value =
        message_port_slot_value(scope, port, slot).unwrap_or_else(|| v8::null(scope).into());
    if value.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        value
    }
}

fn message_port_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    handler_slot: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    message_port_event_handler_value(scope, port, handler_slot)
        .try_into()
        .ok()
}

fn message_port_event_handler_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    order_slot: &'static str,
) -> Option<f64> {
    message_port_number_slot(scope, port, order_slot)
        .filter(|order| order.is_finite() && *order >= 0.0)
}

fn set_message_port_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    handler_slot: &'static str,
    order_slot: &'static str,
) {
    if value.is_function() || value.is_object() {
        if message_port_event_handler_order(scope, port, order_slot).is_none() {
            let order = next_message_port_listener_order(scope, port);
            set_message_port_number_slot(scope, port, order_slot, order);
        }
        set_message_port_slot_value(scope, port, handler_slot, value);
        if let Some(port_id) = message_port_id_from_object(scope, port)
            && let Some(registry) = current_message_port_registry(scope)
        {
            registry.wake_message_port_if_pending(port_id);
        }
    } else {
        set_message_port_slot_value(scope, port, handler_slot, v8::null(scope).into());
        set_message_port_slot_value(scope, port, order_slot, v8::undefined(scope).into());
    }
}

pub(in crate::context_bootstrap) fn set_internal_message_port_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    onmessage: v8::Local<'s, v8::Function>,
    onmessageerror: v8::Local<'s, v8::Function>,
) {
    set_message_port_event_handler(
        scope,
        port,
        onmessage.into(),
        MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT,
        MESSAGE_PORT_ONMESSAGE_ORDER_SLOT,
    );
    set_message_port_event_handler(
        scope,
        port,
        onmessageerror.into(),
        MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT,
        MESSAGE_PORT_ONMESSAGEERROR_ORDER_SLOT,
    );
}

/// Roll back a private MessagePort channel that was prepared for browser
/// machinery but never successfully published. Unlike author-visible close,
/// this removes both endpoint records in one registry operation, then releases
/// any corresponding wrappers retained by the current realm before returning.
pub(in crate::context_bootstrap) fn discard_message_port_channel(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    let Some(registry) = current_message_port_registry(scope) else {
        return;
    };
    for discarded_port_id in registry.discard_message_port_channel(port_id) {
        forget_message_port_wrapper(scope, discarded_port_id);
    }
}

pub(in crate::context_bootstrap::message_ports) fn set_message_port_peer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    peer: v8::Local<'s, v8::Object>,
) {
    set_message_port_slot_value(scope, port, MESSAGE_PORT_PEER_SLOT, peer.into());
}

pub(in crate::context_bootstrap::message_ports) fn set_message_port_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    value: bool,
) {
    set_message_port_bool_slot(scope, port, MESSAGE_PORT_STARTED_SLOT, value);
}

fn message_port_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, port, slot)
}

fn set_message_port_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, port, slot, value);
}

fn message_port_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    message_port_slot_value(scope, port, slot)?.number_value(scope)
}

fn set_message_port_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
    value: f64,
) {
    set_message_port_slot_value(scope, port, slot, v8::Number::new(scope, value).into());
}

fn message_port_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<bool> {
    message_port_slot_value(scope, port, slot).map(|value| value.boolean_value(scope))
}

fn set_message_port_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    slot: &str,
    value: bool,
) {
    set_message_port_slot_value(scope, port, slot, v8::Boolean::new(scope, value).into());
}

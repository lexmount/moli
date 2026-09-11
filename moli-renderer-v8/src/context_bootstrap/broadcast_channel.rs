use super::*;
use crate::web_api_interfaces;
use crate::{
    broadcast_channel_runtime::{
        BroadcastChannelEvent, BroadcastChannelOwner, BroadcastChannelStorageKey,
        SharedBroadcastChannelRegistry,
    },
    context_bootstrap::events::mark_event_trusted,
    structured_clone::V8StructuredClonePayload,
    types::BroadcastChannelId,
    util::{callable_relevant_context, callback_data_item, get_private_value, set_private_value},
    webidl,
    worker::{
        forget_worker_broadcast_channel_wrapper, register_worker_broadcast_channel_wrapper,
        worker_broadcast_channel_registry, worker_broadcast_channel_storage_key,
        worker_broadcast_channel_wake_sender, worker_broadcast_channel_wrapper,
        worker_global_is_closed,
    },
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const BROADCAST_CHANNEL_ID_SLOT: &str = "__lmBroadcastChannelId";
const BROADCAST_CHANNEL_NAME_SLOT: &str = "__lmBroadcastChannelName";
const BROADCAST_CHANNEL_CLOSED_SLOT: &str = "__lmBroadcastChannelClosed";
const BROADCAST_CHANNEL_LISTENERS_SLOT: &str = "__lmBroadcastChannelListeners";
const BROADCAST_CHANNEL_ONMESSAGE_SLOT: &str = "__lmBroadcastChannelOnmessage";
const BROADCAST_CHANNEL_ONMESSAGEERROR_SLOT: &str = "__lmBroadcastChannelOnmessageerror";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::BroadcastChannel)]
struct BroadcastChannelObjectDeclaration<'scope> {
    #[webapi(slot = BROADCAST_CHANNEL_ID_SLOT)]
    channel_id: v8::Local<'scope, v8::Value>,

    #[webapi(slot = BROADCAST_CHANNEL_NAME_SLOT)]
    name: v8::Local<'scope, v8::String>,

    #[webapi(slot = BROADCAST_CHANNEL_CLOSED_SLOT, init = false)]
    closed: (),

    #[webapi(slot = BROADCAST_CHANNEL_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),

    #[webapi(slot = BROADCAST_CHANNEL_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = BROADCAST_CHANNEL_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(
        method,
        enumerable,
        callback = broadcast_channel_event_target_add_event_listener_callback
    )]
    add_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = broadcast_channel_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = broadcast_channel_event_target_dispatch_event_callback
    )]
    dispatch_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BroadcastChannel, enumerable)]
struct BroadcastChannelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = broadcast_channel_name_getter_callback,
        enumerable
    )]
    name: (),

    #[webapi(
        method = "postMessage",
        length = 1,
        callback = broadcast_channel_post_message_callback
    )]
    post_message: (),

    #[webapi(method, length = 0, callback = broadcast_channel_close_callback)]
    close: (),

    #[webapi(
        accessor_property,
        getter = broadcast_channel_event_handler_getter_callback,
        setter = broadcast_channel_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        enumerable
    )]
    onmessage: (),

    #[webapi(
        accessor_property,
        getter = broadcast_channel_event_handler_getter_callback,
        setter = broadcast_channel_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        enumerable
    )]
    onmessageerror: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct BroadcastChannelMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BroadcastChannel")]
struct BroadcastChannelConstructorArgs {
    #[webidl(required, name = "name")]
    name: String,
}

#[derive(Clone, Copy)]
struct BroadcastChannelEventHandler {
    event_type: &'static str,
    slot_name: &'static str,
}

const BROADCAST_CHANNEL_EVENT_HANDLERS: &[BroadcastChannelEventHandler] = &[
    BroadcastChannelEventHandler {
        event_type: "message",
        slot_name: BROADCAST_CHANNEL_ONMESSAGE_SLOT,
    },
    BroadcastChannelEventHandler {
        event_type: "messageerror",
        slot_name: BROADCAST_CHANNEL_ONMESSAGEERROR_SLOT,
    },
];

pub(in crate::context_bootstrap) fn broadcast_channel_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'BroadcastChannel': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<BroadcastChannelConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(relevant_context) = callable_relevant_context(scope, args.new_target()) else {
        // V8 no longer exposes a creation context after the constructor's
        // realm has been detached. Keep the constructor usable as a detached
        // BroadcastChannel surface, but never register it against the
        // caller's ambient Window.
        initialize_detached_broadcast_channel_object(scope, args.this(), &parsed.name);
        rv.set(args.this().into());
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    if current_child_window_is_detached(scope) {
        initialize_detached_broadcast_channel_object(scope, args.this(), &parsed.name);
        rv.set(args.this().into());
        return;
    }
    if worker_global_is_closed(scope) {
        initialize_detached_broadcast_channel_object(scope, args.this(), &parsed.name);
        rv.set(args.this().into());
        return;
    }
    let Some(registration) = current_broadcast_channel_registration(scope) else {
        initialize_detached_broadcast_channel_object(scope, args.this(), &parsed.name);
        rv.set(args.this().into());
        return;
    };
    let Some(registry) = current_broadcast_channel_registry(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(storage_key) = current_broadcast_channel_storage_key(scope, &registration) else {
        rv.set_undefined();
        return;
    };
    let channel_id =
        registry.create_broadcast_channel(storage_key, parsed.name.clone(), registration.owner());
    initialize_broadcast_channel_object(scope, args.this(), channel_id, &parsed.name);
    if !register_broadcast_channel_wrapper(scope, channel_id, args.this(), registration) {
        registry.close_broadcast_channel(channel_id);
        rv.set_undefined();
        return;
    }
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn install_broadcast_channel_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    BroadcastChannelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn initialize_broadcast_channel_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
    channel_id: BroadcastChannelId,
    name: &str,
) {
    let channel_id_value = v8::BigInt::new_from_u64(scope, channel_id);
    BroadcastChannelObjectDeclaration::new(
        channel_id_value.into(),
        v8_string(scope, name).unwrap_or_else(|| v8::String::empty(scope)),
    )
    .initialize(scope, channel)
    .expect("BroadcastChannel declaration should initialize");
}

fn initialize_detached_broadcast_channel_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let channel_id_value = v8::undefined(scope);
    BroadcastChannelObjectDeclaration::new(
        channel_id_value.into(),
        v8_string(scope, name).unwrap_or_else(|| v8::String::empty(scope)),
    )
    .initialize(scope, channel)
    .expect("BroadcastChannel declaration should initialize");
}

enum CurrentBroadcastChannelRegistration {
    Page {
        owner: BroadcastChannelOwner,
        identity: crate::native_bridge::WindowExecutionContextIdentity,
    },
    Worker {
        owner: BroadcastChannelOwner,
    },
}

impl CurrentBroadcastChannelRegistration {
    fn owner(&self) -> BroadcastChannelOwner {
        match self {
            Self::Page { owner, .. } | Self::Worker { owner } => owner.clone(),
        }
    }
}

fn current_broadcast_channel_registration(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<CurrentBroadcastChannelRegistration> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let identity = host.current_runtime_window_execution_context_identity(scope)?;
        let owner = BroadcastChannelOwner::Page(
            host.page_broadcast_channel_delivery_sender()
                .bind_execution_context(identity),
        );
        return Some(CurrentBroadcastChannelRegistration::Page { owner, identity });
    }
    Some(CurrentBroadcastChannelRegistration::Worker {
        owner: BroadcastChannelOwner::Worker(worker_broadcast_channel_wake_sender(scope)?),
    })
}

fn current_child_window_is_detached(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let Some(handle) =
        crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
            .or_else(|| crate::native_bridge::active_child_window_handle(scope))
    else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    !unsafe { &*host_ptr }.child_browsing_context_is_live(handle)
}

fn current_broadcast_channel_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedBroadcastChannelRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return Some(unsafe { &*host_ptr }.broadcast_channel_registry());
    }
    worker_broadcast_channel_registry(scope)
}

fn current_broadcast_channel_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
    registration: &CurrentBroadcastChannelRegistration,
) -> Option<BroadcastChannelStorageKey> {
    if let (Some(host_ptr), CurrentBroadcastChannelRegistration::Page { identity, .. }) =
        (context_host_ptr_from_global_bridge(scope), registration)
    {
        let host = unsafe { &mut *host_ptr };
        return host
            .storage_context_for_window_execution_context_identity(*identity)
            .map(|context| context.storage_key().clone());
    }
    if matches!(
        registration,
        CurrentBroadcastChannelRegistration::Worker { .. }
    ) {
        return worker_broadcast_channel_storage_key(scope);
    }
    None
}

fn broadcast_channel_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
) -> Option<BroadcastChannelId> {
    let value = get_private_value(scope, channel, BROADCAST_CHANNEL_ID_SLOT)?;
    let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) else {
        return None;
    };
    let (id, lossless) = big.u64_value();
    assert!(lossless, "BroadcastChannel id BigInt must fit in u64");
    Some(id)
}

fn broadcast_channel_is_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, channel, BROADCAST_CHANNEL_CLOSED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn broadcast_channel_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::BroadcastChannel::is_instance(scope, receiver)
}

fn broadcast_channel_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn broadcast_channel_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_remove_event_listener_callback(scope, args, rv);
}

fn broadcast_channel_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_dispatch_event_callback(scope, args, rv);
}

fn close_broadcast_channel_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel: v8::Local<'s, v8::Object>,
) {
    if broadcast_channel_is_closed(scope, channel) {
        return;
    }
    if let Some(channel_id) = broadcast_channel_id_from_object(scope, channel) {
        forget_broadcast_channel_wrapper(scope, channel_id);
        if let Some(registry) = current_broadcast_channel_registry(scope) {
            registry.close_broadcast_channel(channel_id);
        }
    }
    set_private_value(
        scope,
        channel,
        BROADCAST_CHANNEL_ID_SLOT,
        v8::undefined(scope).into(),
    );
    set_private_value(
        scope,
        channel,
        BROADCAST_CHANNEL_CLOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

fn broadcast_channel_wrapper_for_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel_id: BroadcastChannelId,
) -> Option<BroadcastChannelDispatchTarget<'s>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return unsafe { &mut *host_ptr }
            .broadcast_channel_wrapper(scope, channel_id)
            .map(|(dispatch_scope, realm_token, context, wrapper)| {
                BroadcastChannelDispatchTarget::Window {
                    dispatch_scope,
                    realm_token,
                    context,
                    wrapper,
                }
            });
    }
    worker_broadcast_channel_wrapper(scope, channel_id).map(BroadcastChannelDispatchTarget::Worker)
}

enum BroadcastChannelDispatchTarget<'s> {
    Window {
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
        context: v8::Local<'s, v8::Context>,
        wrapper: v8::Local<'s, v8::Object>,
    },
    Worker(v8::Local<'s, v8::Object>),
}

fn register_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
    channel: v8::Local<'_, v8::Object>,
    registration: CurrentBroadcastChannelRegistration,
) -> bool {
    match registration {
        CurrentBroadcastChannelRegistration::Page { identity, .. } => {
            let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                return false;
            };
            unsafe { &mut *host_ptr }
                .register_broadcast_channel_wrapper(scope, channel_id, channel, identity);
        }
        CurrentBroadcastChannelRegistration::Worker { .. } => {
            register_worker_broadcast_channel_wrapper(scope, channel_id, channel);
        }
    }
    true
}

fn forget_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.forget_broadcast_channel_wrapper(channel_id);
        return;
    }
    forget_worker_broadcast_channel_wrapper(scope, channel_id);
}

fn broadcast_channel_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), BROADCAST_CHANNEL_NAME_SLOT)
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn broadcast_channel_event_handler_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        BROADCAST_CHANNEL_EVENT_HANDLERS,
        "BroadcastChannel event handlers",
    ) else {
        rv.set_null();
        return;
    };
    let value = get_private_value(scope, args.this(), handler.slot_name)
        .unwrap_or_else(|| v8::null(scope).into());
    if value.is_null_or_undefined() {
        rv.set_null();
    } else {
        rv.set(value);
    }
}

fn broadcast_channel_event_handler_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        BROADCAST_CHANNEL_EVENT_HANDLERS,
        "BroadcastChannel event handlers",
    ) else {
        return;
    };
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), handler.slot_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        BROADCAST_CHANNEL_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        stored.is_function(),
    );
}

fn broadcast_channel_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if broadcast_channel_is_closed(scope, args.this()) {
        throw_broadcast_channel_invalid_state(scope);
        return;
    }
    if current_child_window_is_detached(scope) {
        rv.set_undefined();
        return;
    }
    if worker_global_is_closed(scope) {
        rv.set_undefined();
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'BroadcastChannel': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(channel_id) = broadcast_channel_id_from_object(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let Some(registry) = current_broadcast_channel_registry(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(payload) =
        structured_serialize_value_for_post_message(scope, args.get(0), None, "BroadcastChannel")
    else {
        return;
    };
    for recipient_id in registry.post_broadcast_channel_message(channel_id, payload) {
        schedule_broadcast_channel_delivery(scope, recipient_id);
    }
    rv.set_undefined();
}

fn throw_broadcast_channel_invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    native_bridge::throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "Failed to execute 'postMessage' on 'BroadcastChannel': This BroadcastChannel is closed.",
    );
}

fn broadcast_channel_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !broadcast_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    close_broadcast_channel_object(scope, args.this());
    rv.set_undefined();
}

pub(crate) fn dispatch_broadcast_channel_events_for_channel(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
) -> bool {
    let Some(target) = broadcast_channel_wrapper_for_id(scope, channel_id) else {
        return false;
    };
    dispatch_broadcast_channel_event_to_target(scope, channel_id, target)
}

/// Execute one Page delivery after the stable Page arbiter has authorized the
/// exact root namespace and Window realm. Wrapper lookup only matches the
/// captured channel/binding pair; it does not perform a second currentness
/// decision.
pub(crate) fn dispatch_authorized_page_broadcast_channel_event(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
    expected: crate::native_bridge::WindowExecutionContextIdentity,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let target = unsafe { &*host_ptr }
        .authorized_broadcast_channel_wrapper(scope, channel_id, expected)
        .map(|(dispatch_scope, realm_token, context, wrapper)| {
            BroadcastChannelDispatchTarget::Window {
                dispatch_scope,
                realm_token,
                context,
                wrapper,
            }
        });
    let Some(target) = target else {
        return false;
    };
    dispatch_broadcast_channel_event_to_target(scope, channel_id, target)
}

fn dispatch_broadcast_channel_event_to_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel_id: BroadcastChannelId,
    target: BroadcastChannelDispatchTarget<'s>,
) -> bool {
    match target {
        BroadcastChannelDispatchTarget::Window {
            dispatch_scope,
            realm_token,
            context,
            wrapper,
        } => {
            let scope = &mut v8::ContextScope::new(scope, context);
            if crate::native_bridge::current_runtime_observable_context_token(scope)
                != Some(realm_token)
            {
                close_broadcast_channel_object(scope, wrapper);
                return false;
            }
            let previous_owner_context = dispatch_scope.enter(scope);
            let dispatched =
                dispatch_broadcast_channel_events_in_current_context(scope, channel_id, wrapper);
            if dispatched {
                dispatch_scope.defer_restore(scope, previous_owner_context);
            } else {
                dispatch_scope.restore(scope, previous_owner_context);
            }
            dispatched
        }
        BroadcastChannelDispatchTarget::Worker(wrapper) => {
            dispatch_broadcast_channel_events_in_current_context(scope, channel_id, wrapper)
        }
    }
}

fn dispatch_broadcast_channel_events_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel_id: BroadcastChannelId,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(registry) = current_broadcast_channel_registry(scope) else {
        return false;
    };
    let origin = registry
        .broadcast_channel_origin(channel_id)
        .unwrap_or_else(|| "null".to_owned());
    let Some(event) = registry.take_pending_broadcast_channel_event(channel_id) else {
        return false;
    };
    dispatch_broadcast_channel_event(scope, target, event, &origin)
}

fn dispatch_broadcast_channel_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: BroadcastChannelEvent<V8StructuredClonePayload>,
    origin: &str,
) -> bool {
    match event {
        BroadcastChannelEvent::Message(payload) => {
            let target_origin = crate::context_bootstrap::current_runtime_message_origin(scope);
            let target_agent_cluster =
                crate::context_bootstrap::current_runtime_message_agent_cluster(scope);
            if !crate::context_bootstrap::wasm_module_message_allowed_for_target(
                &payload,
                target_origin.as_deref(),
                target_agent_cluster,
            ) {
                if let Some(event) = new_broadcast_channel_message_event(
                    scope,
                    "messageerror",
                    v8::null(scope).into(),
                    origin,
                    v8::Array::new(scope, 0),
                ) {
                    dispatch_simple_event_target_event(
                        scope,
                        target,
                        BROADCAST_CHANNEL_LISTENERS_SLOT,
                        "messageerror",
                        event,
                    );
                    return true;
                }
                return false;
            }
            if let Some((data, ports)) =
                structured_deserialize_value_for_message_event(scope, &payload)
                && let Some(event) =
                    new_broadcast_channel_message_event(scope, "message", data, origin, ports)
            {
                dispatch_simple_event_target_event(
                    scope,
                    target,
                    BROADCAST_CHANNEL_LISTENERS_SLOT,
                    "message",
                    event,
                );
                return true;
            }
            if let Some(event) = new_broadcast_channel_message_event(
                scope,
                "messageerror",
                v8::null(scope).into(),
                origin,
                v8::Array::new(scope, 0),
            ) {
                dispatch_simple_event_target_event(
                    scope,
                    target,
                    BROADCAST_CHANNEL_LISTENERS_SLOT,
                    "messageerror",
                    event,
                );
                return true;
            }
        }
    }
    false
}

fn schedule_broadcast_channel_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
) {
    if let Some(registry) = current_broadcast_channel_registry(scope) {
        registry.wake_broadcast_channel_if_pending(channel_id);
    }
}

fn new_broadcast_channel_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
    ports: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let message_ctor = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init =
        BroadcastChannelMessageEventInitDeclaration::new(data, v8_string(scope, origin)?, ports)
            .bind(scope)
            .expect("BroadcastChannel MessageEvent init declaration should bind");
    let event_type = v8_string(scope, event_type)?;
    let event = message_ctor.new_instance(scope, &[event_type.into(), init.into()])?;
    mark_event_trusted(scope, event);
    Some(event)
}

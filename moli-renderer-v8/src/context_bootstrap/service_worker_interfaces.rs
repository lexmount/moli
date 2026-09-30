//! Shared native identity, state and attributes for Window and Worker bindings.

use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(crate) const REGISTRATION_SCOPE: &str = "__moliServiceWorkerRegistrationScope";
pub(crate) const REGISTRATION_UPDATE_VIA_CACHE: &str =
    "__moliServiceWorkerRegistrationUpdateViaCache";
pub(crate) const WORKER_STATE: &str = "__moliServiceWorkerState";
pub(crate) const CONTAINER_READY: &str = "__moliServiceWorkerContainerReady";
const EVENT_HANDLERS: &[(&str, &str)] = &[
    (
        "updatefound",
        "__moliServiceWorkerRegistrationOnUpdateFound",
    ),
    ("statechange", "__moliServiceWorkerOnStateChange"),
    ("error", "__moliServiceWorkerOnError"),
];
const SCRIPT_URL: &str = "__moliServiceWorkerScriptUrl";
const REGISTRATION_SYNC: &str = "__moliServiceWorkerRegistrationSync";
const REGISTRATION_PERIODIC_SYNC: &str = "__moliServiceWorkerRegistrationPeriodicSync";
const REGISTRATION_PUSH_MANAGER: &str = "__moliServiceWorkerRegistrationPushManager";
const REGISTRATION_NAVIGATION_PRELOAD: &str = "__moliServiceWorkerRegistrationNavigationPreload";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ServiceWorkerRegistration)]
pub(crate) struct ServiceWorkerRegistrationObjectDeclaration<'scope> {
    #[webapi(prototype)]
    pub(crate) prototype: v8::Local<'scope, v8::Object>,
    #[webapi(slot = REGISTRATION_SCOPE)]
    pub(crate) scope: String,
    #[webapi(slot = REGISTRATION_UPDATE_VIA_CACHE)]
    pub(crate) update_via_cache: &'static str,
    #[webapi(slot = REGISTRATION_SYNC)]
    pub(crate) sync: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = REGISTRATION_PERIODIC_SYNC)]
    pub(crate) periodic_sync: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = REGISTRATION_PUSH_MANAGER)]
    pub(crate) push_manager: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = REGISTRATION_NAVIGATION_PRELOAD)]
    pub(crate) navigation_preload: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ServiceWorker)]
pub(crate) struct ServiceWorkerObjectDeclaration<'scope> {
    #[webapi(prototype)]
    pub(crate) prototype: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SCRIPT_URL)]
    pub(crate) script_url: String,
    #[webapi(slot = WORKER_STATE)]
    pub(crate) state: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationPreloadManager)]
pub(crate) struct NavigationPreloadManagerObjectDeclaration<'scope> {
    #[webapi(prototype)]
    pub(crate) prototype: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ServiceWorkerRegistration, enumerable, receiver)]
struct RegistrationAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_SCOPE))]
    scope: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_UPDATE_VIA_CACHE))]
    update_via_cache: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_SYNC))]
    sync: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_PERIODIC_SYNC))]
    periodic_sync: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_PUSH_MANAGER))]
    push_manager: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, REGISTRATION_NAVIGATION_PRELOAD))]
    navigation_preload: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = callback_data_index_value(scope, 0))]
    onupdatefound: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ServiceWorker, enumerable, receiver)]
struct WorkerAttributes {
    #[webapi(accessor_property = "scriptURL", getter = slot_getter, data = v8str(scope, SCRIPT_URL))]
    script_url: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, WORKER_STATE))]
    state: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = callback_data_index_value(scope, 1))]
    onstatechange: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = callback_data_index_value(scope, 2))]
    onerror: (),
}

pub(super) fn install_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "ServiceWorkerRegistration" => {
            RegistrationAttributes::initialize_prototype_template(scope, prototype)
        }
        "ServiceWorker" => WorkerAttributes::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((_, slot)) =
        callback_data_item(scope, &args, EVENT_HANDLERS, "ServiceWorker event handler")
    else {
        return;
    };
    rv.set(get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((event_type, slot)) =
        callback_data_item(scope, &args, EVENT_HANDLERS, "ServiceWorker event handler")
    else {
        return;
    };
    let value = args.get(0);
    let active = value.is_object();
    let value = if active {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), slot, value);
    if let Some(listeners) = simple_event_target_slot_name(scope, args.this()) {
        simple_object_event_set_ordered_handler(
            scope,
            args.this(),
            &listeners,
            event_type,
            slot,
            active,
        );
    }
}

/// The generated binding rejects invalid receivers in the callee realm. Native
/// operation results belong to the relevant realm of the valid receiver.
pub(super) fn receiver_promise_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let context = receiver.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    v8::PromiseResolver::new(scope)
}

// The main backend has no registration update job yet. Keep the shared operation
// and its generated Promise/receiver semantics without pretending an update ran.
pub(crate) fn registration_update_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    crate::context_bootstrap::throw_dom_exception_value(
        scope,
        "Service worker registration update is not supported by this backend.",
        "NotSupportedError",
    );
}

use super::delivery::{enqueue_buffered_performance_entries, queue_performance_observer_delivery};
use super::*;
use crate::observer_runtime::ObserverCallbackId;
use crate::util::{
    define_v8_array_data_property, get_private_value, serialize_v8_iter_array, set_private_value,
};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceObserver")]
struct PerformanceObserverObjectDeclaration<'s> {
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_ID_SLOT)]
    callback_id: u32,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT)]
    callback_value: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT)]
    callback_relevant_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT)]
    callback_incumbent_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_PENDING_SLOT, init = "array")]
    pending: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_TYPE_SLOT, init = "null")]
    observed_type: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT, init = "array")]
    entry_types: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_ACTIVE_SLOT, init = false)]
    active: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_SCHEDULED_SLOT, init = false)]
    scheduled: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserver")]
struct PerformanceObserverConstructorArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to construct 'PerformanceObserver': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserver.observe")]
struct PerformanceObserverObserveArgs<'s> {
    #[webidl(required)]
    options: v8::Local<'s, v8::Object>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceObserverInit")]
struct PerformanceObserverInit {
    #[webidl(name = "type")]
    observed_type: Option<String>,
    #[webidl(with = performance_entry_types_member)]
    entry_types: Option<Vec<String>>,
    #[webidl(default = false)]
    buffered: bool,
}

pub(in crate::context_bootstrap) fn performance_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'PerformanceObserver': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<PerformanceObserverConstructorArgs>(scope, &args)
    else {
        return;
    };
    let (callback_id, callback, relevant_global, incumbent_global) =
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let registered_callback = crate::observer_runtime::register_callback(
                scope,
                host_ptr,
                args.this(),
                parsed.callback,
            );
            let (callback_id, callback, relevant_global, incumbent_global) =
                registered_callback.into_parts();
            (
                callback_id.as_u32(),
                callback,
                relevant_global,
                incumbent_global,
            )
        } else {
            if crate::worker::get_worker_state(scope).is_none() {
                throw_type_error(
                    scope,
                    "PerformanceObserver is unavailable outside a Window or Worker realm.",
                );
                return;
            }
            let callback = v8::Local::<v8::Object>::try_from(parsed.callback.value(scope))
                .expect("a Web IDL callback function must be an object");
            let relevant_global = parsed.callback.relevant_context(scope).global(scope);
            let incumbent_global = parsed.callback.incumbent_context(scope).global(scope);
            // Worker observers do not need a Window execution-context binding.
            // Zero is deliberately outside ObserverCallbackId's valid range.
            (0, callback, relevant_global, incumbent_global)
        };
    PerformanceObserverObjectDeclaration::new(
        callback_id,
        callback,
        relevant_global,
        incumbent_global,
    )
    .initialize(scope, args.this())
    .expect("PerformanceObserver declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn performance_observer_observe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceObserverObserveArgs>(scope, &args) else {
        return;
    };
    let init =
        match webidl::parse_dictionary_object::<PerformanceObserverInit>(scope, parsed.options) {
            Ok(init) => init,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
    let has_entry_types_member = init.entry_types.is_some();
    let has_type_member = init.observed_type.is_some();
    if has_entry_types_member && has_type_member {
        throw_type_error(
            scope,
            "PerformanceObserver.observe options must not include both type and entryTypes.",
        );
        return;
    }
    if !has_entry_types_member && !has_type_member {
        throw_type_error(
            scope,
            "PerformanceObserver.observe options must include either type or entryTypes.",
        );
        return;
    }

    let current_type_mode = performance_observer_type_slot_is_set(scope, args.this());
    let current_entry_types_mode = !current_type_mode
        && performance_observer_entry_types(scope, args.this())
            .is_some_and(|entry_types| entry_types.length() > 0);
    if has_entry_types_member && current_type_mode {
        webidl::throw_dom_exception(
            scope,
            "InvalidModificationError",
            "This PerformanceObserver has performed observe({type: ...}) and cannot observe entryTypes.",
        );
        return;
    }
    if has_type_member && current_entry_types_mode {
        webidl::throw_dom_exception(
            scope,
            "InvalidModificationError",
            "This PerformanceObserver has performed observe({entryTypes: ...}) and cannot observe type.",
        );
        return;
    }

    let observed_entry_types = if let Some(observed_type) = init.observed_type.as_deref() {
        if !performance_observer_entry_type_supported(scope, observed_type) {
            rv.set_undefined();
            return;
        }
        let entry_types = performance_observer_entry_types(scope, args.this())
            .unwrap_or_else(|| v8::Array::new(scope, 0));
        if !array_contains_string(scope, entry_types, observed_type) {
            let value = v8_string(scope, observed_type)
                .unwrap_or_else(|| v8::String::empty(scope))
                .into();
            let _ = define_v8_array_data_property(scope, entry_types, entry_types.length(), value);
        }
        entry_types
    } else {
        let supported_entry_types = init
            .entry_types
            .unwrap_or_default()
            .into_iter()
            .filter(|entry_type| performance_observer_entry_type_supported(scope, entry_type))
            .collect::<Vec<_>>();
        performance_entry_types_array_from_strings(scope, supported_entry_types)
    };
    if observed_entry_types.length() == 0 {
        rv.set_undefined();
        return;
    }
    let performance = super::super::performance_runtime::ensure_current_performance_for_api(scope);
    let observes_navigation = (0..observed_entry_types.length()).any(|index| {
        observed_entry_types
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == "navigation")
    });
    if observes_navigation && let Some(performance) = performance {
        super::super::performance_runtime::ensure_navigation_performance_entry_for_api(
            scope,
            performance,
        );
    }
    set_performance_observer_type(
        scope,
        args.this(),
        init.observed_type
            .as_deref()
            .and_then(|value| v8_string(scope, value))
            .map(|value| value.into())
            .unwrap_or_else(|| v8::null(scope).into()),
    );
    set_performance_observer_entry_types(scope, args.this(), observed_entry_types);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let Some(callback_id) = performance_observer_callback_id(scope, args.this()) else {
            rv.set_undefined();
            return;
        };
        // Blink keeps a PerformanceObserver alive only while it is registered
        // (`HasPendingActivity() == is_registered_`). The exact callback binding
        // owns the same active root so disconnect and Realm retirement can release
        // it without a parallel global JS registry.
        if !crate::observer_runtime::activate_performance_observer_callback(
            scope,
            host_ptr,
            callback_id,
            args.this(),
        ) {
            set_performance_observer_active(scope, args.this(), false);
            let pending = v8::Array::new(scope, 0);
            set_performance_observer_pending(scope, args.this(), pending);
            rv.set_undefined();
            return;
        }
    } else if crate::worker::get_worker_state(scope).is_some() {
        // A worker has one Realm for the lifetime of its isolate run, so its
        // active observer set can live on that Realm without Window generation
        // authorization.
        push_object_to_global_registry(scope, PERFORMANCE_OBSERVER_REGISTRY_SLOT, args.this());
    } else {
        rv.set_undefined();
        return;
    }
    set_performance_observer_active(scope, args.this(), true);
    if init.buffered && has_type_member {
        enqueue_buffered_performance_entries(scope, args.this());
        queue_performance_observer_delivery(scope, args.this());
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn performance_observer_disconnect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_performance_observer_active(scope, args.this(), false);
    if let Some(callback_id) = performance_observer_callback_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        crate::observer_runtime::deactivate_performance_observer_callback(host_ptr, callback_id);
    }
    let pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, args.this(), pending);
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn performance_observer_take_records_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let records = performance_observer_pending(scope, args.this())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, args.this(), pending);
    rv.set(records.into());
}

fn performance_entry_types_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<Vec<String>>, webidl::WebIdlError> {
    let entry_types = webidl::optional_member::<webidl::Sequence<webidl::DomString>>(
        scope,
        object,
        name,
        webidl::Context::member("PerformanceObserverInit", name),
    )?
    .map(|entry_types| entry_types.0.into_iter().map(Into::into).collect());
    Ok(entry_types)
}

fn performance_entry_types_array_from_strings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: impl IntoIterator<Item = String>,
) -> v8::Local<'s, v8::Array> {
    let mut entry_types = Vec::new();
    for entry_type in values {
        if entry_types.iter().any(|existing| existing == &entry_type) {
            continue;
        }
        entry_types.push(entry_type);
    }
    serialize_v8_iter_array(scope, entry_types).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn performance_observer_type_slot_is_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_TYPE_SLOT)
        .is_some_and(|value| !value.is_null_or_undefined())
}

fn performance_observer_entry_type_supported(
    scope: &mut v8::PinScope<'_, '_>,
    entry_type: &str,
) -> bool {
    let global = scope.get_current_context().global(scope);
    get_private_value(
        scope,
        global,
        PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    .is_some_and(|entry_types| array_contains_string(scope, entry_types, entry_type))
}

pub(super) fn array_contains_string(
    scope: &mut v8::PinScope<'_, '_>,
    array: v8::Local<'_, v8::Array>,
    expected: &str,
) -> bool {
    (0..array.length()).any(|index| {
        array
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == expected)
    })
}

pub(super) fn performance_observer_callback_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<ObserverCallbackId> {
    let value =
        performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_CALLBACK_ID_SLOT)?;
    ObserverCallbackId::from_number(value.number_value(scope)?)
}

pub(super) fn performance_observer_callback_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<crate::observer_runtime::ObserverCallbackResidence<'s>> {
    let callback_id = performance_observer_callback_id(scope, observer)?;
    let callback =
        performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let relevant_global = performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let incumbent_global = performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    Some(
        crate::observer_runtime::ObserverCallbackResidence::from_parts(
            callback_id,
            callback,
            relevant_global,
            incumbent_global,
        ),
    )
}

pub(super) fn prepare_worker_performance_observer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<moli_webidl_callback::PreparedWebIdlCallbackFunction> {
    let callback =
        performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let relevant_global = performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let incumbent_global = performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let relevant_context = relevant_global.get_creation_context(scope)?;
    let incumbent_context = incumbent_global.get_creation_context(scope)?;
    moli_webidl_callback::PreparedWebIdlCallbackFunction::try_new(
        scope,
        callback,
        relevant_context,
        incumbent_context,
    )
}

pub(super) fn performance_observer_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_PENDING_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn set_performance_observer_pending(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    pending: v8::Local<'_, v8::Array>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_PENDING_SLOT,
        pending.into(),
    );
}

pub(super) fn performance_observer_entry_types<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_performance_observer_entry_types(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    entry_types: v8::Local<'_, v8::Array>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT,
        entry_types.into(),
    );
}

pub(super) fn performance_observer_observed_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<String> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_TYPE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_performance_observer_type(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    observed_type: v8::Local<'_, v8::Value>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_TYPE_SLOT,
        observed_type,
    );
}

pub(super) fn performance_observer_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    performance_observer_slot_bool(scope, observer, PERFORMANCE_OBSERVER_ACTIVE_SLOT)
        .unwrap_or(false)
}

fn set_performance_observer_active(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    active: bool,
) {
    set_performance_observer_bool_slot(scope, observer, PERFORMANCE_OBSERVER_ACTIVE_SLOT, active);
}

pub(super) fn performance_observer_scheduled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    performance_observer_slot_bool(scope, observer, PERFORMANCE_OBSERVER_SCHEDULED_SLOT)
        .unwrap_or(false)
}

pub(super) fn set_performance_observer_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    scheduled: bool,
) {
    set_performance_observer_bool_slot(
        scope,
        observer,
        PERFORMANCE_OBSERVER_SCHEDULED_SLOT,
        scheduled,
    );
}

fn performance_observer_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, observer, slot)
}

fn performance_observer_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    performance_observer_slot_value(scope, observer, slot).map(|value| value.boolean_value(scope))
}

fn set_performance_observer_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, observer, slot, value);
}

fn set_performance_observer_bool_slot(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_performance_observer_slot_value(scope, observer, slot, value.into());
}

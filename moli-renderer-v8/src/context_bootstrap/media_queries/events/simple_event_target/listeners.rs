use super::*;
use crate::abort_signal_route::ResolvedAbortSignal;
use crate::callback_invocation::CallbackInvocation;
use crate::event_listener_args::{
    AddEventListenerArgs, RemoveEventListenerArgs, parse_listener_args,
};
use crate::native_bridge::{WindowExecutionContextIdentity, lightweight_popup_id_from_window};
use crate::util::{
    context_host_ptr_from_global_bridge, get_private_object, get_private_value,
    new_null_prototype_object, set_private_value, v8_string,
};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

const SIMPLE_EVENT_TARGET_LISTENER_ORIGINAL_SLOT: &str = "__moliSimpleEventTargetListenerOriginal";
const SIMPLE_EVENT_TARGET_LISTENER_CALLBACK_SLOT: &str = "__moliSimpleEventTargetListenerCallback";
const SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_CONTEXT_ANCHOR_SLOT: &str =
    "__moliSimpleEventTargetListenerRelevantContextAnchor";
const SIMPLE_EVENT_TARGET_LISTENER_INCUMBENT_CONTEXT_ANCHOR_SLOT: &str =
    "__moliSimpleEventTargetListenerIncumbentContextAnchor";
const SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_IDENTITY_SLOT: &str =
    "__moliSimpleEventTargetListenerRelevantIdentity";
const SIMPLE_EVENT_TARGET_LISTENER_CALLABLE_SLOT: &str = "__moliSimpleEventTargetListenerCallable";
const SIMPLE_EVENT_TARGET_LISTENER_CAPTURE_SLOT: &str = "__moliSimpleEventTargetListenerCapture";
const SIMPLE_EVENT_TARGET_LISTENER_ONCE_SLOT: &str = "__moliSimpleEventTargetListenerOnce";
const SIMPLE_EVENT_TARGET_LISTENER_PASSIVE_SLOT: &str = "__moliSimpleEventTargetListenerPassive";
const SIMPLE_EVENT_TARGET_LISTENER_ABORT_SIGNAL_SLOT: &str =
    "__moliSimpleEventTargetListenerAbortSignal";
const SIMPLE_EVENT_TARGET_LISTENER_ABORT_ALGORITHM_SLOT: &str =
    "__moliSimpleEventTargetListenerAbortAlgorithm";
const SIMPLE_EVENT_TARGET_LISTENER_TYPE_ORDER_SLOT: &str =
    "__moliSimpleEventTargetListenerTypeOrder";

#[derive(WebApiObject)]
#[webapi(plain)]
struct SimpleObjectAbortListenerDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    entry: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    slot: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    listener: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    capture: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SimpleObjectEventListenerEntryDeclaration<'scope> {
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_ORIGINAL_SLOT)]
    original: v8::Local<'scope, v8::Value>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_CALLBACK_SLOT)]
    callback: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_CONTEXT_ANCHOR_SLOT)]
    relevant_context_anchor: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_INCUMBENT_CONTEXT_ANCHOR_SLOT)]
    incumbent_context_anchor: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_CALLABLE_SLOT)]
    callable: bool,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_CAPTURE_SLOT)]
    capture: bool,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_ONCE_SLOT)]
    once: bool,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_PASSIVE_SLOT)]
    passive: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SimpleObjectEventHandlerEntryDeclaration<'scope> {
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_ORIGINAL_SLOT)]
    original: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_HANDLER_SLOT_FIELD)]
    handler_slot: &'static str,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_CALLBACK_SLOT)]
    callback: v8::Local<'scope, v8::Value>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_CONTEXT_ANCHOR_SLOT)]
    relevant_context_anchor: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_INCUMBENT_CONTEXT_ANCHOR_SLOT)]
    incumbent_context_anchor: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENER_CALLABLE_SLOT)]
    callable: bool,
}

#[derive(Clone)]
pub(crate) struct SimpleObjectEventListenerSnapshot<'s> {
    entry: v8::Local<'s, v8::Object>,
    pub(crate) original: v8::Local<'s, v8::Value>,
    callback: Option<v8::Local<'s, v8::Object>>,
    relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    is_callable: bool,
    pub(crate) capture: bool,
    pub(crate) once: bool,
    pub(crate) passive: bool,
    pub(crate) handler_slot: Option<String>,
}

pub(crate) struct SimpleObjectEventListenerInspectorSnapshot<'s> {
    pub(crate) event_type: String,
    pub(crate) original: v8::Local<'s, v8::Value>,
    pub(crate) callback: v8::Local<'s, v8::Object>,
    pub(crate) relevant_context: v8::Local<'s, v8::Context>,
    pub(crate) is_callable: bool,
    pub(crate) capture: bool,
    pub(crate) once: bool,
    pub(crate) passive: bool,
}

struct SimpleObjectResolvedEventListener<'s> {
    original: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Object>,
    relevant_context_anchor: v8::Local<'s, v8::Object>,
    incumbent_context_anchor: v8::Local<'s, v8::Object>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    is_callable: bool,
}

impl<'s> SimpleObjectEventListenerSnapshot<'s> {
    pub(crate) fn prepare_for_invocation(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target: v8::Local<'s, v8::Object>,
        slot_name: &str,
        event_type: &str,
    ) -> Option<Self> {
        let listener = self.resolve_callback(scope, target, slot_name, event_type)?;
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                target,
                slot_name,
                event_type,
                listener.original,
                listener.capture,
            );
        }
        Some(listener)
    }

    fn resolve_callback(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target: v8::Local<'s, v8::Object>,
        slot_name: &str,
        event_type: &str,
    ) -> Option<Self> {
        // Removing and re-adding the same callback creates a different entry.
        // The removed entry must stay inactive in an existing dispatch snapshot.
        if !simple_object_event_listener_entry_registered(
            scope, target, slot_name, event_type, self.entry,
        ) {
            return None;
        }
        let listener = if self.handler_slot.is_some() {
            if let Some(popup_id) = lightweight_popup_id_from_window(scope, target)
                && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
            {
                let value = unsafe { &mut *host_ptr }
                    .lightweight_popup_event_handler_property_value(
                        scope,
                        popup_id,
                        self.handler_slot.as_deref()?,
                    )?;
                // A failed lazy compilation returns null for this read even
                // if its error handler has installed a replacement callback.
                if !value.is_object()
                    || !simple_object_event_listener_entry_registered(
                        scope, target, slot_name, event_type, self.entry,
                    )
                {
                    return None;
                }
            }
            // An active handler keeps its registration position, but its value
            // and callback contexts can change before dispatch reaches it.
            simple_object_event_listener_snapshot_entry(scope, self.entry.into())?
        } else {
            self.clone()
        };
        listener.callback?;
        Some(listener)
    }

    pub(crate) fn invocation<'a>(
        &self,
        callback_this: v8::Local<'s, v8::Value>,
        arguments: &'a [v8::Local<'s, v8::Value>],
        current_event: Option<v8::Local<'s, v8::Object>>,
    ) -> Option<CallbackInvocation<'s, 'a>> {
        let invocation = CallbackInvocation::new(
            self.callback?,
            callback_this,
            self.relevant_context,
            self.incumbent_context,
            self.is_callable,
            "handleEvent",
            arguments,
            current_event,
        );
        Some(if self.handler_slot.is_some() {
            invocation.with_legacy_event_handler()
        } else {
            invocation
        })
    }

    pub(crate) fn relevant_context(&self) -> v8::Local<'s, v8::Context> {
        self.relevant_context
    }

    pub(crate) fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        self.relevant_identity
    }

    pub(crate) fn callable_function(&self) -> Option<v8::Local<'s, v8::Function>> {
        let callback = self.callback?;
        self.is_callable
            .then(|| unsafe { v8::Local::<v8::Function>::cast_unchecked(callback) })
    }
}

pub(crate) fn simple_object_event_target_add_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot_name: &str,
) {
    let Some(call) = parse_listener_args::<AddEventListenerArgs>(scope, args, "addEventListener")
    else {
        return;
    };
    let Some(listener) = call.listener else {
        return;
    };
    let target = args.this();
    simple_object_event_target_register_webidl_listener(
        scope,
        target,
        slot_name,
        call.event_type,
        listener,
        call.options.options,
        call.options.signal,
    );
}

/// Registers an already converted EventListener on a JS-object-local target.
///
/// This is the shared compatibility boundary for EventTarget-like surfaces
/// that intentionally keep their registration list on the wrapper rather than
/// in the native DOM EventTarget registry. The strong callback value supplies
/// the callback object's relevant Realm, conversion-time incumbent context,
/// and callable-versus-`handleEvent` branch; this target layer continues to own
/// duplicate suppression, listener options, abort cleanup, and dispatch order.
pub(crate) fn simple_object_event_target_register_webidl_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: String,
    listener: webidl::WebIdlCallbackInterface,
    options: webidl::EventListenerOptions,
    signal: Option<ResolvedAbortSignal<'s>>,
) {
    let listener = simple_object_event_listener_parts(scope, listener);
    simple_object_event_target_register_resolved_listener(
        scope, target, slot_name, event_type, listener, options, signal,
    );
}

#[allow(clippy::too_many_arguments)]
fn simple_object_event_target_register_resolved_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: String,
    listener: SimpleObjectResolvedEventListener<'s>,
    options: webidl::EventListenerOptions,
    signal: Option<ResolvedAbortSignal<'s>>,
) {
    if signal.is_some_and(|signal| signal.is_aborted(scope)) {
        return;
    }
    let Some(listeners) =
        simple_object_event_listener_array(scope, target, slot_name, &event_type, true)
    else {
        return;
    };
    let added = !simple_object_event_listener_array_contains_original(
        scope,
        listeners,
        listener.original,
        options.capture,
    );
    let added_entry = if added {
        let entry = simple_object_event_listener_entry_object(
            scope,
            listener.original,
            listener.callback,
            listener.relevant_context_anchor,
            listener.incumbent_context_anchor,
            listener.relevant_identity,
            listener.is_callable,
            options.capture,
            options.once,
            options.passive.unwrap_or(false),
        );
        let _ = listeners.set_index(scope, listeners.length(), entry.into());
        Some(entry)
    } else {
        None
    };
    if added_entry.is_some() {
        ensure_simple_object_event_type_order(scope, target, slot_name, &event_type);
    }
    if let (Some(entry), Some(signal)) = (added_entry, signal)
        && !register_simple_object_abort_listener(
            scope,
            signal,
            target,
            entry,
            slot_name,
            &event_type,
            listener.original,
            options.capture,
        )
    {
        simple_object_event_remove_listener_value_for_type(
            scope,
            target,
            slot_name,
            &event_type,
            listener.original,
            options.capture,
        );
    }
    if event_type == "message" {
        crate::context_bootstrap::flush_pending_worker_messages_for_listener(scope, target);
    }
}

pub(crate) fn simple_object_event_target_remove_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot_name: &str,
) {
    let Some(call) =
        parse_listener_args::<RemoveEventListenerArgs>(scope, args, "removeEventListener")
    else {
        return;
    };
    let Some(listener) = call.listener else {
        return;
    };
    let listener = listener.value(scope);
    let target = args.this();
    simple_object_event_remove_listener_value_for_type(
        scope,
        target,
        slot_name,
        &call.event_type,
        listener,
        call.options.capture,
    );
}

fn simple_object_abort_remove_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(target) = data
        .get(scope, v8str(scope, "target").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(slot_name) = data
        .get(scope, v8str(scope, "slot").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Some(event_type) = data
        .get(scope, v8str(scope, "type").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Some(listener) = data.get(scope, v8str(scope, "listener").into()) else {
        return;
    };
    let Some(entry) = data
        .get(scope, v8str(scope, "entry").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let capture = data
        .get(scope, v8str(scope, "capture").into())
        .is_some_and(|value| value.boolean_value(scope));
    if !simple_object_event_listener_entry_registered(scope, target, &slot_name, &event_type, entry)
    {
        return;
    }
    simple_object_event_remove_listener_value_for_type(
        scope,
        target,
        &slot_name,
        &event_type,
        listener,
        capture,
    );
}

fn register_simple_object_abort_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: ResolvedAbortSignal<'s>,
    target: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    listener: v8::Local<'s, v8::Value>,
    capture: bool,
) -> bool {
    let Some(slot_value) = v8_string(scope, slot_name) else {
        return false;
    };
    let Some(event_type_value) = v8_string(scope, event_type) else {
        return false;
    };
    let data = SimpleObjectAbortListenerDataDeclaration::new(
        target,
        entry,
        slot_value,
        event_type_value,
        listener,
        capture,
    )
    .bind(scope)
    .expect("SimpleObject abort listener data declaration should bind");
    let Some(abort_listener) = v8::Function::builder(simple_object_abort_remove_listener_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    if !signal.register_algorithm(scope, abort_listener) {
        return false;
    }
    set_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_ABORT_SIGNAL_SLOT,
        signal.value().into(),
    );
    set_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_ABORT_ALGORITHM_SLOT,
        abort_listener.into(),
    );
    true
}

fn simple_object_event_listener_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    create: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(registry) = get_private_object(scope, target, slot_name) {
        return Some(registry);
    }
    if !create {
        return None;
    }
    // Internal listener registry keyed by page-provided event type strings.
    // It is not a Web API instance, so null-prototype is intentional.
    let registry = new_null_prototype_object(scope);
    set_private_value(scope, target, slot_name, registry.into());
    Some(registry)
}

pub(in crate::context_bootstrap::media_queries::events::simple_event_target) fn simple_object_event_listener_array<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    create: bool,
) -> Option<v8::Local<'s, v8::Array>> {
    let registry = simple_object_event_listener_registry(scope, target, slot_name, create)?;
    if let Some(listeners) = object_property_as_array(scope, registry, event_type) {
        return Some(listeners);
    }
    if !create {
        return None;
    }
    let listeners = v8::Array::new(scope, 0);
    let _ = registry.set(
        scope,
        v8_string(scope, event_type)?.into(),
        listeners.into(),
    );
    Some(listeners)
}

pub(crate) fn simple_object_event_listeners_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
) -> Vec<SimpleObjectEventListenerSnapshot<'s>> {
    let Some(listeners) =
        simple_object_event_listener_array(scope, target, slot_name, event_type, false)
    else {
        return Vec::new();
    };
    let mut snapshot = Vec::with_capacity(listeners.length() as usize);
    for index in 0..listeners.length() {
        let Some(candidate) = listeners.get_index(scope, index) else {
            continue;
        };
        let Some(listener) = simple_object_event_listener_snapshot_entry(scope, candidate) else {
            continue;
        };
        snapshot.push(listener);
    }
    snapshot
}

pub(crate) fn simple_object_has_event_listeners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
) -> bool {
    let Some(registry) = simple_object_event_listener_registry(scope, target, slot_name, false)
    else {
        return false;
    };
    let Some(event_types) = simple_object_event_type_order(scope, registry, false) else {
        return false;
    };
    for index in 0..event_types.length() {
        let Some(event_type) = event_types
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        if !simple_object_event_listeners_snapshot(scope, target, slot_name, &event_type).is_empty()
        {
            return true;
        }
    }
    false
}

pub(crate) fn simple_event_target_inspector_listener_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Vec<SimpleObjectEventListenerInspectorSnapshot<'s>> {
    let Some(slot_name) = super::state::simple_event_target_slot_name(scope, target) else {
        return Vec::new();
    };
    let Some(registry) = simple_object_event_listener_registry(scope, target, &slot_name, false)
    else {
        return Vec::new();
    };
    let Some(event_types) = simple_object_event_type_order(scope, registry, false) else {
        return Vec::new();
    };
    let mut snapshots = Vec::new();
    for index in 0..event_types.length() {
        let Some(event_type) = event_types
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        for listener in
            simple_object_event_listeners_snapshot(scope, target, &slot_name, &event_type)
        {
            let Some(listener) = listener.resolve_callback(scope, target, &slot_name, &event_type)
            else {
                continue;
            };
            let Some(callback) = listener.callback else {
                continue;
            };
            let original = if listener.handler_slot.is_some() {
                callback.into()
            } else {
                listener.original
            };
            snapshots.push(SimpleObjectEventListenerInspectorSnapshot {
                event_type: event_type.clone(),
                original,
                callback,
                relevant_context: listener.relevant_context,
                is_callable: listener.is_callable,
                capture: listener.capture,
                once: listener.once,
                passive: listener.passive,
            });
        }
    }
    snapshots
}

pub(crate) fn simple_object_event_set_ordered_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    handler_slot_name: &'static str,
    active: bool,
) {
    if active {
        let Some(callback) = get_private_value(scope, target, handler_slot_name)
            .or_else(|| {
                let key = v8_string(scope, handler_slot_name)?;
                target.get(scope, key.into())
            })
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            return;
        };
        set_simple_object_event_ordered_handler_callback(
            scope,
            target,
            slot_name,
            event_type,
            handler_slot_name,
            Some(callback),
        );
        return;
    }
    let Some(listeners) =
        simple_object_event_listener_array(scope, target, slot_name, event_type, false)
    else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..listeners.length() {
        let Some(candidate) = listeners.get_index(scope, index) else {
            continue;
        };
        if simple_object_event_listener_handler_slot(scope, candidate).as_deref()
            == Some(handler_slot_name)
        {
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    if let Some(registry) = simple_object_event_listener_registry(scope, target, slot_name, false)
        && let Some(key) = v8_string(scope, event_type)
    {
        let _ = registry.set(scope, key.into(), next.into());
        if next.length() == 0 {
            remove_simple_object_event_type_order(scope, registry, event_type);
        }
    }
}

/// An uncompiled or failed content handler retains its active listener's
/// registration position without inventing a callable value for it.
pub(crate) fn simple_object_event_activate_uncompiled_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    handler_slot_name: &'static str,
) {
    set_simple_object_event_ordered_handler_callback(
        scope,
        target,
        slot_name,
        event_type,
        handler_slot_name,
        None,
    );
}

fn set_simple_object_event_ordered_handler_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    handler_slot_name: &'static str,
    callback: Option<v8::Local<'s, v8::Object>>,
) {
    let Some(listeners) =
        simple_object_event_listener_array(scope, target, slot_name, event_type, true)
    else {
        return;
    };
    let (relevant_context_anchor, incumbent_context_anchor, relevant_identity) =
        if let Some(callback) = callback {
            simple_callback_context_anchors(scope, callback)
        } else {
            let context = scope.get_current_context();
            simple_callback_context_anchors_for_contexts(scope, context, context)
        };
    let value = callback
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    for index in 0..listeners.length() {
        let Some(candidate) = listeners.get_index(scope, index) else {
            continue;
        };
        if simple_object_event_listener_handler_slot(scope, candidate).as_deref()
            == Some(handler_slot_name)
        {
            if let Ok(entry) = v8::Local::<v8::Object>::try_from(candidate) {
                set_private_value(
                    scope,
                    entry,
                    SIMPLE_EVENT_TARGET_LISTENER_CALLBACK_SLOT,
                    value,
                );
                let callable =
                    v8::Boolean::new(scope, callback.is_some_and(|value| value.is_callable()));
                set_private_value(
                    scope,
                    entry,
                    SIMPLE_EVENT_TARGET_LISTENER_CALLABLE_SLOT,
                    callable.into(),
                );
                set_private_value(
                    scope,
                    entry,
                    SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_CONTEXT_ANCHOR_SLOT,
                    relevant_context_anchor.into(),
                );
                set_private_value(
                    scope,
                    entry,
                    SIMPLE_EVENT_TARGET_LISTENER_INCUMBENT_CONTEXT_ANCHOR_SLOT,
                    incumbent_context_anchor.into(),
                );
                set_simple_object_event_listener_relevant_identity(scope, entry, relevant_identity);
            }
            ensure_simple_object_event_type_order(scope, target, slot_name, event_type);
            return;
        }
    }
    let entry = simple_object_event_handler_entry_object(
        scope,
        handler_slot_name,
        callback,
        relevant_context_anchor,
        incumbent_context_anchor,
        relevant_identity,
    );
    let _ = listeners.set_index(scope, listeners.length(), entry.into());
    ensure_simple_object_event_type_order(scope, target, slot_name, event_type);
}

pub(in crate::context_bootstrap::media_queries::events::simple_event_target) fn simple_event_target_uses_ordered_handlers<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    simple_event_target_private_value(scope, target, SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT)
        .is_some_and(|value| value.is_true())
}

pub(crate) fn simple_object_event_remove_listener_value_for_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    listener: v8::Local<'s, v8::Value>,
    capture: bool,
) {
    let Some(registry) = simple_object_event_listener_registry(scope, target, slot_name, false)
    else {
        return;
    };
    let Some(current) = object_property_as_array(scope, registry, event_type) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..current.length() {
        let Some(candidate) = current.get_index(scope, index) else {
            continue;
        };
        let should_remove = simple_object_event_listener_original(scope, candidate)
            .is_some_and(|original| original.strict_equals(listener))
            && simple_object_event_listener_capture(scope, candidate).unwrap_or(false) == capture;
        if should_remove {
            unregister_simple_object_event_listener_abort_algorithm(scope, candidate);
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    if let Some(key) = v8_string(scope, event_type) {
        let _ = registry.set(scope, key.into(), next.into());
        if next.length() == 0 {
            remove_simple_object_event_type_order(scope, registry, event_type);
        }
    }
}

fn unregister_simple_object_event_listener_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
) {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(candidate) else {
        return;
    };
    let signal = get_private_value(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_ABORT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let algorithm = get_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_ABORT_ALGORITHM_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    if let (Some(signal), Some(algorithm)) = (signal, algorithm)
        && let Some(signal) = ResolvedAbortSignal::resolve(scope, signal)
    {
        let _ = signal.unregister_algorithm(scope, algorithm);
    }
    let undefined = v8::undefined(scope);
    set_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_ABORT_SIGNAL_SLOT,
        undefined.into(),
    );
    set_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_ABORT_ALGORITHM_SLOT,
        undefined.into(),
    );
}

fn simple_object_event_type_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    create: bool,
) -> Option<v8::Local<'s, v8::Array>> {
    if let Some(order) = get_private_value(
        scope,
        registry,
        SIMPLE_EVENT_TARGET_LISTENER_TYPE_ORDER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return Some(order);
    }
    if !create {
        return None;
    }
    let order = v8::Array::new(scope, 0);
    set_private_value(
        scope,
        registry,
        SIMPLE_EVENT_TARGET_LISTENER_TYPE_ORDER_SLOT,
        order.into(),
    );
    Some(order)
}

fn ensure_simple_object_event_type_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
) {
    let Some(registry) = simple_object_event_listener_registry(scope, target, slot_name, false)
    else {
        return;
    };
    let Some(order) = simple_object_event_type_order(scope, registry, true) else {
        return;
    };
    for index in 0..order.length() {
        if order
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == event_type)
        {
            return;
        }
    }
    if let Some(event_type) = v8_string(scope, event_type) {
        let _ = order.set_index(scope, order.length(), event_type.into());
    }
}

fn remove_simple_object_event_type_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    event_type: &str,
) {
    let Some(order) = simple_object_event_type_order(scope, registry, false) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..order.length() {
        let Some(candidate) = order.get_index(scope, index) else {
            continue;
        };
        if candidate
            .to_string(scope)
            .is_some_and(|value| value.to_rust_string_lossy(scope) == event_type)
        {
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    set_private_value(
        scope,
        registry,
        SIMPLE_EVENT_TARGET_LISTENER_TYPE_ORDER_SLOT,
        next.into(),
    );
}

fn simple_object_event_listener_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: webidl::WebIdlCallbackInterface,
) -> SimpleObjectResolvedEventListener<'s> {
    let callback_value = listener.value(scope);
    let callback = v8::Local::<v8::Object>::try_from(callback_value)
        .expect("converted EventListener callback must remain an object");
    let relevant_context = listener.relevant_context(scope);
    let incumbent_context = listener.incumbent_context(scope);
    let (relevant_context_anchor, incumbent_context_anchor, relevant_identity) =
        simple_callback_context_anchors_for_contexts(scope, relevant_context, incumbent_context);
    SimpleObjectResolvedEventListener {
        original: callback.into(),
        callback,
        relevant_context_anchor,
        incumbent_context_anchor,
        relevant_identity,
        is_callable: listener.callable_at_conversion(),
    }
}

fn simple_callback_context_anchors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Object>,
) -> (
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    Option<WindowExecutionContextIdentity>,
) {
    let current_context = scope.get_current_context();
    let relevant_context = callback
        .get_creation_context(scope)
        .unwrap_or(current_context);
    let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
    simple_callback_context_anchors_for_contexts(scope, relevant_context, incumbent_context)
}

fn simple_callback_context_anchors_for_contexts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
) -> (
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    Option<WindowExecutionContextIdentity>,
) {
    let relevant_identity = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        unsafe { &*host_ptr }
            .window_execution_context_identity_for_v8_context(scope, relevant_context)
    });
    let relevant_context_anchor = {
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        v8::Object::new(scope)
    };
    let incumbent_context_anchor = {
        let scope = &mut v8::ContextScope::new(scope, incumbent_context);
        v8::Object::new(scope)
    };
    (
        relevant_context_anchor,
        incumbent_context_anchor,
        relevant_identity,
    )
}

fn simple_object_event_listener_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Object>,
    relevant_context_anchor: v8::Local<'s, v8::Object>,
    incumbent_context_anchor: v8::Local<'s, v8::Object>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    is_callable: bool,
    capture: bool,
    once: bool,
    passive: bool,
) -> v8::Local<'s, v8::Object> {
    let entry = SimpleObjectEventListenerEntryDeclaration::new(
        original,
        callback,
        relevant_context_anchor,
        incumbent_context_anchor,
        is_callable,
        capture,
        once,
        passive,
    )
    .bind(scope)
    .expect("SimpleObject event listener entry declaration should bind");
    set_simple_object_event_listener_relevant_identity(scope, entry, relevant_identity);
    entry
}

fn simple_object_event_handler_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handler_slot_name: &'static str,
    callback: Option<v8::Local<'s, v8::Object>>,
    relevant_context_anchor: v8::Local<'s, v8::Object>,
    incumbent_context_anchor: v8::Local<'s, v8::Object>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
) -> v8::Local<'s, v8::Object> {
    let marker = format!("event-handler:{handler_slot_name}");
    let value = callback
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let entry = SimpleObjectEventHandlerEntryDeclaration::new(
        v8_string(scope, &marker),
        handler_slot_name,
        value,
        relevant_context_anchor,
        incumbent_context_anchor,
        callback.is_some_and(|value| value.is_callable()),
    )
    .bind(scope)
    .expect("SimpleObject event handler entry declaration should bind");
    set_simple_object_event_listener_relevant_identity(scope, entry, relevant_identity);
    entry
}

fn set_simple_object_event_listener_relevant_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    identity: Option<WindowExecutionContextIdentity>,
) {
    let value = identity
        .and_then(|identity| v8_string(scope, &identity.serialize_for_internal_slot()))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_IDENTITY_SLOT,
        value,
    );
}

fn simple_object_event_listener_array_contains_original(
    scope: &mut v8::PinScope<'_, '_>,
    listeners: v8::Local<'_, v8::Array>,
    original: v8::Local<'_, v8::Value>,
    capture: bool,
) -> bool {
    for index in 0..listeners.length() {
        let Some(candidate) = listeners.get_index(scope, index) else {
            continue;
        };
        let candidate_original = simple_object_event_listener_original(scope, candidate);
        let candidate_capture =
            simple_object_event_listener_capture(scope, candidate).unwrap_or(false);
        if candidate_original.is_some_and(|value| value.strict_equals(original))
            && candidate_capture == capture
        {
            return true;
        }
    }
    false
}

fn simple_object_event_listener_array_contains_entry(
    scope: &mut v8::PinScope<'_, '_>,
    listeners: v8::Local<'_, v8::Array>,
    entry: v8::Local<'_, v8::Object>,
) -> bool {
    for index in 0..listeners.length() {
        if listeners
            .get_index(scope, index)
            .is_some_and(|candidate| candidate.strict_equals(entry.into()))
        {
            return true;
        }
    }
    false
}

fn simple_object_event_listener_entry_registered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    entry: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(listeners) =
        simple_object_event_listener_array(scope, target, slot_name, event_type, false)
    else {
        return false;
    };
    simple_object_event_listener_array_contains_entry(scope, listeners, entry)
}

fn simple_object_event_listener_original<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(candidate) else {
        return None;
    };
    get_private_value(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_ORIGINAL_SLOT)
}

fn simple_object_event_listener_snapshot_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
) -> Option<SimpleObjectEventListenerSnapshot<'s>> {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(candidate) else {
        return None;
    };
    let original = get_private_value(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_ORIGINAL_SLOT)?;
    let callback_value =
        get_private_value(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_CALLBACK_SLOT)?;
    let callback = v8::Local::<v8::Object>::try_from(callback_value).ok();
    let handler_slot = simple_object_event_listener_handler_slot(scope, candidate);
    if callback.is_none() && handler_slot.is_none() {
        return None;
    }
    let relevant_context = simple_object_callback_context(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_CONTEXT_ANCHOR_SLOT,
    )?;
    let incumbent_context = simple_object_callback_context(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_INCUMBENT_CONTEXT_ANCHOR_SLOT,
    )?;
    let relevant_identity = get_private_value(
        scope,
        entry,
        SIMPLE_EVENT_TARGET_LISTENER_RELEVANT_IDENTITY_SLOT,
    )
    .and_then(|value| value.to_string(scope))
    .map(|value| value.to_rust_string_lossy(scope))
    .and_then(|serialized| {
        WindowExecutionContextIdentity::deserialize_from_internal_slot(&serialized)
    });
    let is_callable =
        simple_object_private_bool_slot(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_CALLABLE_SLOT)
            .unwrap_or(false);
    let once =
        simple_object_private_bool_slot(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_ONCE_SLOT)
            .unwrap_or(false);
    let capture =
        simple_object_private_bool_slot(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_CAPTURE_SLOT)
            .unwrap_or(false);
    let passive =
        simple_object_private_bool_slot(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_PASSIVE_SLOT)
            .unwrap_or(false);
    Some(SimpleObjectEventListenerSnapshot {
        entry,
        original,
        callback,
        relevant_context,
        incumbent_context,
        relevant_identity,
        is_callable,
        capture,
        once,
        passive,
        handler_slot,
    })
}

fn simple_object_callback_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Context>> {
    get_private_value(scope, entry, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|global| global.get_creation_context(scope))
}

fn simple_object_event_listener_capture<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
) -> Option<bool> {
    let entry = v8::Local::<v8::Object>::try_from(candidate).ok()?;
    Some(
        simple_object_private_bool_slot(scope, entry, SIMPLE_EVENT_TARGET_LISTENER_CAPTURE_SLOT)
            .unwrap_or(false),
    )
}

fn simple_object_event_listener_handler_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(candidate) else {
        return None;
    };
    get_private_value(scope, entry, SIMPLE_EVENT_TARGET_HANDLER_SLOT_FIELD)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn simple_object_private_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    get_private_value(scope, object, slot).map(|value| value.boolean_value(scope))
}

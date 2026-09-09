use super::*;
use crate::{
    context_bootstrap::{
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT, clear_event_composed_path, event_is_error_event,
        mark_event_trusted, set_event_composed_path,
    },
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    util::{get_private_object, get_private_value, serialize_v8_iter_array, set_private_value},
};
use moli_webapi_declare::WebApiObject;
use std::collections::HashSet;

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct HostErrorEventInitDeclaration<'scope> {
    cancelable: bool,
    bubbles: bool,
    message: v8::Local<'scope, v8::String>,
    filename: v8::Local<'scope, v8::String>,
    lineno: f64,
    colno: f64,
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct HostEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
}

struct EventListenerEntry {
    id: u64,
    callback_id: crate::native_bridge::EventCallbackId,
    function_name: String,
    script_id: i32,
    script_url: Option<String>,
    line_number: Option<u32>,
    column_number: Option<u32>,
    capture: bool,
    once: bool,
    passive: bool,
    removed: bool,
}

pub(crate) struct EventListenerRegistration {
    callback_id: crate::native_bridge::EventCallbackId,
    function_name: String,
    script_id: i32,
    script_url: Option<String>,
    line_number: Option<u32>,
    column_number: Option<u32>,
    capture: bool,
    once: bool,
    passive: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct EventListenerInspectorSnapshot {
    pub(crate) registration_id: u64,
    pub(crate) event_type: String,
    pub(crate) callback_id: crate::native_bridge::EventCallbackId,
    pub(crate) capture: bool,
    pub(crate) once: bool,
    pub(crate) passive: bool,
}

impl EventListenerRegistration {
    pub(crate) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        callback_id: crate::native_bridge::EventCallbackId,
        callback: v8::Local<'s, v8::Object>,
        capture: bool,
        once: bool,
        passive: bool,
    ) -> Self {
        let (function_name, script_id, script_url, line_number, column_number) =
            v8::Local::<v8::Function>::try_from(callback)
                .map(|listener| HostEventTargetRegistry::describe_listener(scope, listener))
                .unwrap_or_else(|_| ("handleEvent".to_owned(), -1, None, None, None));
        Self {
            callback_id,
            function_name,
            script_id,
            script_url,
            line_number,
            column_number,
            capture,
            once,
            passive,
        }
    }
}

#[derive(Clone, Copy)]
enum EventHandlerPropertyEntry {
    Callback {
        callback_id: crate::native_bridge::EventCallbackId,
        registration_id: u64,
    },
    Null,
}

enum EventHandlerPropertyValue {
    Callback(crate::native_bridge::EventCallbackId),
    Null,
}

pub(crate) struct HostEventTargetRegistry {
    listeners: HashMap<EventTargetHandle, HashMap<String, Vec<EventListenerEntry>>>,
    handler_properties: HashMap<EventTargetHandle, HashMap<String, EventHandlerPropertyEntry>>,
    event_type_registration_ids: HashMap<EventTargetHandle, HashMap<String, u64>>,
    next_listener_id: u64,
}

impl Default for HostEventTargetRegistry {
    fn default() -> Self {
        Self {
            listeners: HashMap::new(),
            handler_properties: HashMap::new(),
            event_type_registration_ids: HashMap::new(),
            next_listener_id: 1,
        }
    }
}

impl fmt::Debug for HostEventTargetRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostEventTargetRegistry")
            .field("target_count", &self.listeners.len())
            .field(
                "handler_property_target_count",
                &self.handler_properties.len(),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchStatus {
    Continue,
    StopPropagation,
    StopImmediate,
}

pub(crate) fn invoke_prepared_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    invocation_target_in_shadow_tree: bool,
    event_type: &str,
    callback_name: &str,
    callback: crate::native_bridge::PreparedEventCallback,
    target: EventTargetHandle,
    event: v8::Local<'s, v8::Object>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Global<v8::Value>> {
    let receiver = event_target_receiver(scope, host_ptr, target, event);
    let _dom_debugger_pause = unsafe { &*host_ptr }
        .schedule_dom_debugger_event_listener_pause_for_target(event_type, target);
    invoke_prepared_event_callback_with_receiver(
        scope,
        host_ptr,
        event_type,
        callback_name,
        callback,
        receiver,
        (!invocation_target_in_shadow_tree).then_some(event),
        arguments,
    )
}

/// Invokes an EventListener whose EventTarget has an API-specific object
/// residence rather than a DOM `EventTargetHandle`.
///
/// AbortSignal uses this path so its listener ordering remains with AbortStore
/// while callback Realm/currentness, `window.event`, dynamic `handleEvent`
/// lookup, and exception reporting stay identical to other EventListeners.
pub(crate) fn invoke_prepared_event_callback_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event_type: &str,
    callback_name: &str,
    callback: crate::native_bridge::PreparedEventCallback,
    receiver: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Global<v8::Value>> {
    let _dom_debugger_pause = unsafe { &*host_ptr }
        .schedule_dom_debugger_event_listener_pause_for_interface(event_type, "AbortSignal");
    invoke_prepared_event_callback_with_receiver(
        scope,
        host_ptr,
        event_type,
        callback_name,
        callback,
        receiver.into(),
        Some(event),
        &[event.into()],
    )
}

#[allow(clippy::too_many_arguments)]
fn invoke_prepared_event_callback_with_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event_type: &str,
    callback_name: &str,
    callback: crate::native_bridge::PreparedEventCallback,
    receiver: v8::Local<'s, v8::Value>,
    current_event: Option<v8::Local<'s, v8::Object>>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Global<v8::Value>> {
    let relevant_identity = callback.relevant_identity();
    let invocation = CallbackInvocation::new(
        callback.callback(scope),
        receiver,
        callback.relevant_context(scope),
        callback.incumbent_context(scope),
        callback.is_callable(),
        "handleEvent",
        arguments,
        current_event,
    )
    .with_execution_context_currentness(host_ptr, relevant_identity);
    match CallbackInvoker::invoke(
        scope,
        "event listener",
        "host event listener threw",
        crate::exception_reporting::CallbackExceptionLogLevel::Debug,
        callback_name,
        invocation,
    ) {
        CallbackInvocationOutcome::Returned(value) => Some(value),
        CallbackInvocationOutcome::Threw(report) => {
            report_event_callback_exception(
                scope,
                host_ptr,
                event_type,
                relevant_identity,
                None,
                &report,
            );
            None
        }
        CallbackInvocationOutcome::Retired => None,
    }
}

fn invoke_event_handler_property<'s>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
    invocation_target_in_shadow_tree: bool,
    target_object: v8::Local<'s, v8::Object>,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) {
    let handler_name = format!("on{event_type}");
    let (callback_id, temporary) = match registry.event_handler_property_value(target, event_type) {
        Some(EventHandlerPropertyValue::Callback(callback_id)) => (callback_id, false),
        Some(EventHandlerPropertyValue::Null) => {
            return;
        }
        None => {
            let Some(key) = v8_string(scope, &handler_name) else {
                return;
            };
            let handler = target_object
                .get(scope, key.into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
            let (handler, persistent) = match handler {
                Some(handler) => (handler, false),
                None if target == EventTargetHandle::Window && event_type == "messageerror" => {
                    let Some(handler) =
                        crate::native_bridge::element::compile_window_body_onmessageerror_attribute(
                            scope, host_ptr,
                        )
                    else {
                        return;
                    };
                    (handler, true)
                }
                None => return,
            };
            let callback = v8::Local::<v8::Object>::from(handler);
            let relevant_context = callback
                .get_creation_context(scope)
                .unwrap_or_else(|| scope.get_current_context());
            let incumbent_context = scope
                .get_incumbent_context()
                .unwrap_or_else(|| scope.get_current_context());
            let callback_id = unsafe { &mut *host_ptr }.register_event_callback(
                scope,
                callback,
                relevant_context,
                incumbent_context,
            );
            if persistent {
                if let Some(previous) = registry.set_event_handler_property(
                    EventTargetHandle::Window,
                    event_type,
                    Some(callback_id),
                ) {
                    unsafe { &mut *host_ptr }.release_event_callback(previous);
                }
                (callback_id, false)
            } else {
                (callback_id, true)
            }
        }
    };
    let Some(prepared) = unsafe { &*host_ptr }.prepare_event_callback(scope, callback_id) else {
        if temporary {
            unsafe { &mut *host_ptr }.release_event_callback(callback_id);
        }
        return;
    };

    let timing_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
    if target == EventTargetHandle::Window
        && event_type == "error"
        && event_is_error_event(scope, event)
    {
        let message = event
            .get(scope, v8str(scope, "message").into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        let source = event
            .get(scope, v8str(scope, "filename").into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        let lineno = event
            .get(scope, v8str(scope, "lineno").into())
            .unwrap_or_else(|| v8::Number::new(scope, 0.0).into());
        let colno = event
            .get(scope, v8str(scope, "colno").into())
            .unwrap_or_else(|| v8::Number::new(scope, 0.0).into());
        let error = event
            .get(scope, v8str(scope, "error").into())
            .unwrap_or_else(|| v8::null(scope).into());

        let returned = invoke_prepared_event_callback(
            scope,
            host_ptr,
            invocation_target_in_shadow_tree,
            event_type,
            &handler_name,
            prepared,
            target,
            event,
            &[message, source, lineno, colno, error],
        );
        if temporary {
            unsafe { &mut *host_ptr }.release_event_callback(callback_id);
        }
        if let Some(returned) = returned {
            let returned = v8::Local::new(scope, returned);
            if returned.boolean_value(scope) {
                let _ = event.set(
                    scope,
                    v8str(scope, "defaultPrevented").into(),
                    v8::Boolean::new(scope, true).into(),
                );
            }
        }
        if let Some(timing_started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "event_handler_property_invoked",
                event_type,
                handler_name,
                ?target,
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
        return;
    }

    let returned = invoke_prepared_event_callback(
        scope,
        host_ptr,
        invocation_target_in_shadow_tree,
        event_type,
        &handler_name,
        prepared,
        target,
        event,
        &[event.into()],
    );
    if temporary {
        unsafe { &mut *host_ptr }.release_event_callback(callback_id);
    }
    if let Some(returned) = returned {
        let returned = v8::Local::new(scope, returned);
        if returned.is_boolean() && !returned.boolean_value(scope) {
            let _ = event.set(
                scope,
                v8str(scope, "defaultPrevented").into(),
                v8::Boolean::new(scope, true).into(),
            );
        }
    }
    if let Some(timing_started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            stage = "event_handler_property_invoked",
            event_type,
            handler_name,
            ?target,
            elapsed_ms = timing_started.elapsed().as_millis(),
        );
    }
}

fn invoke_event_target_handler_property<'s>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
    invocation_target_in_shadow_tree: bool,
    target_object: v8::Local<'s, v8::Object>,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) {
    if matches!(target, EventTargetHandle::ChildWindow(_)) {
        // Child Window event-handler properties share the native listener
        // registry and are invoked in registration order by the listener phase.
        return;
    }
    invoke_event_handler_property(
        registry,
        scope,
        host_ptr,
        target,
        invocation_target_in_shadow_tree,
        target_object,
        event_type,
        event,
    );
}

fn call_event_target_listeners_filtered<'s>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
    invocation_target_in_shadow_tree: bool,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
    capture_only: bool,
    at_target: bool,
) -> std::result::Result<DispatchStatus, String> {
    if let EventTargetHandle::ChildWindow(target) = target {
        return Ok(
            unsafe { &mut *host_ptr }.call_child_window_event_path_listeners(
                scope,
                target,
                event_type,
                event,
                capture_only,
                at_target,
            ),
        );
    }
    registry.call_listeners_filtered(
        scope,
        host_ptr,
        target,
        invocation_target_in_shadow_tree,
        event_type,
        event,
        capture_only,
        at_target,
    )
}

fn event_target_is_current(host_ptr: *mut JsContextHost, target: EventTargetHandle) -> bool {
    match target {
        EventTargetHandle::ChildWindow(target) => {
            unsafe { &*host_ptr }.child_window_event_target_is_current(target)
        }
        EventTargetHandle::Window | EventTargetHandle::Node(_) => true,
    }
}

pub(crate) fn report_event_listener_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event_type: &str,
    handler: v8::Local<'s, v8::Function>,
    report: &V8ExceptionReport,
) {
    let relevant_identity = handler.get_creation_context(scope).and_then(|context| {
        unsafe { &*host_ptr }.window_execution_context_identity_for_v8_context(scope, context)
    });
    let child_handle = callback_error_window_handle(scope, handler);
    report_event_callback_exception(
        scope,
        host_ptr,
        event_type,
        relevant_identity,
        child_handle,
        report,
    );
}

pub(crate) fn report_event_callback_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event_type: &str,
    relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    child_handle: Option<DomHandle>,
    report: &V8ExceptionReport,
) {
    if event_type == "error" {
        return;
    }

    let child_handle = child_handle.or_else(|| {
        relevant_identity.and_then(|identity| match identity.dispatch_scope() {
            crate::native_bridge::OwnerDispatchScope::Child(handle) => Some(handle),
            crate::native_bridge::OwnerDispatchScope::Top
            | crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => None,
        })
    });
    if let Some(handle) = child_handle {
        let event = {
            let host = unsafe { &mut *host_ptr };
            host.child_browsing_context_window_wrapper(scope, handle)
                .and_then(|window| event_listener_error_event_from_report(scope, window, report))
        };
        if let Some(event) = event {
            unsafe { &mut *host_ptr }.dispatch_child_window_event(scope, handle, "error", event);
            return;
        }
    }

    let error_value = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(scope, exception));
    let _ = dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        &report.summary,
        report.source.as_deref().unwrap_or(""),
        report.line.unwrap_or(0) as u32,
        report.column.unwrap_or(0) as u32,
        error_value,
    );
}

fn callback_error_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) -> Option<DomHandle> {
    if let Some(handle) =
        get_private_value(scope, callback.into(), CALLBACK_ERROR_WINDOW_HANDLE_SLOT)
            .and_then(|value| dom_handle_from_value(scope, value))
    {
        return Some(handle);
    }
    let context = callback.get_creation_context(scope)?;
    let global = context.global(scope);
    if let Some(handle) = get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| dom_handle_from_value(scope, value))
    {
        return Some(handle);
    }
    None
}

fn dom_handle_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    let handle = value.number_value(scope)?;
    (handle.is_finite() && handle >= 0.0 && handle.fract() == 0.0)
        .then(|| DomHandle::new(handle as usize))
}

fn event_listener_error_event_from_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    report: &V8ExceptionReport,
) -> Option<v8::Local<'s, v8::Object>> {
    let error_value = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(scope, exception))
        .unwrap_or_else(|| v8::null(scope).into());

    let message = v8_string(scope, &report.summary).unwrap_or_else(|| v8str(scope, ""));
    let filename = v8_string(scope, report.source.as_deref().unwrap_or(""))
        .unwrap_or_else(|| v8str(scope, ""));
    let init = HostErrorEventInitDeclaration::new(
        true,
        false,
        message,
        filename,
        report.line.unwrap_or(0) as f64,
        report.column.unwrap_or(0) as f64,
        error_value,
    )
    .bind(scope)
    .ok()?;

    global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|constructor| {
            constructor.new_instance(scope, &[v8str(scope, "error").into(), init.into()])
        })
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PublicEventDispatchResult {
    pub default_prevented: bool,
}

impl PublicEventDispatchResult {
    pub(crate) fn allows_default(self) -> bool {
        !self.default_prevented
    }

    pub(crate) fn dispatch_event_return_value(self) -> bool {
        self.allows_default()
    }
}

impl HostEventTargetRegistry {
    pub(crate) fn clear_targets_matching(
        &mut self,
        mut should_clear: impl FnMut(EventTargetHandle) -> bool,
    ) -> HashSet<crate::native_bridge::EventCallbackId> {
        let mut retired = HashSet::new();
        let targets_to_clear = self
            .listeners
            .keys()
            .chain(self.handler_properties.keys())
            .chain(self.event_type_registration_ids.keys())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|target| should_clear(*target))
            .collect::<HashSet<_>>();
        self.listeners.retain(|target, listeners| {
            if targets_to_clear.contains(target) {
                retired.extend(listeners.values().flatten().map(|entry| entry.callback_id));
                false
            } else {
                true
            }
        });
        for (target, handlers) in &mut self.handler_properties {
            if targets_to_clear.contains(target) {
                for handler in handlers.values_mut() {
                    if let EventHandlerPropertyEntry::Callback { callback_id, .. } = *handler {
                        retired.insert(callback_id);
                    }
                    *handler = EventHandlerPropertyEntry::Null;
                }
            }
        }
        self.event_type_registration_ids
            .retain(|target, _| !targets_to_clear.contains(target));
        retired
    }

    fn describe_listener(
        scope: &mut v8::PinScope<'_, '_>,
        listener: v8::Local<'_, v8::Function>,
    ) -> (String, i32, Option<String>, Option<u32>, Option<u32>) {
        let function_name = v8::Local::<v8::String>::try_from(listener.get_name(scope))
            .map(|name| name.to_rust_string_lossy(scope))
            .unwrap_or_default();
        let script_id = listener.script_id();
        let script_url = listener
            .get_script_origin(scope)
            .resource_name()
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty());
        let line_number = listener.get_script_line_number();
        let column_number = listener.get_script_column_number();
        (
            function_name,
            script_id,
            script_url,
            line_number,
            column_number,
        )
    }

    pub(crate) fn set_event_handler_property(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: Option<crate::native_bridge::EventCallbackId>,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        let existing_registration_id = self
            .handler_properties
            .get(&target)
            .and_then(|handlers| handlers.get(event_type))
            .and_then(|entry| match entry {
                EventHandlerPropertyEntry::Callback {
                    registration_id, ..
                } => Some(*registration_id),
                EventHandlerPropertyEntry::Null => None,
            });
        let entry = match callback_id {
            Some(callback_id) => {
                let registration_id =
                    existing_registration_id.unwrap_or_else(|| self.allocate_listener_id());
                self.ensure_event_type_registration_id(target, event_type, registration_id);
                EventHandlerPropertyEntry::Callback {
                    callback_id,
                    registration_id,
                }
            }
            None => EventHandlerPropertyEntry::Null,
        };
        let previous = self
            .handler_properties
            .entry(target)
            .or_default()
            .insert(event_type.to_owned(), entry);
        let previous_callback = match previous {
            Some(EventHandlerPropertyEntry::Callback { callback_id, .. }) => Some(callback_id),
            Some(EventHandlerPropertyEntry::Null) | None => None,
        };
        if callback_id.is_none() {
            self.remove_event_type_registration_id_if_unused(target, event_type);
        }
        previous_callback
    }

    pub(crate) fn clear_event_handler_property(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        let removed = self
            .handler_properties
            .get_mut(&target)
            .and_then(|handlers| handlers.remove(event_type));
        let remove_target = self
            .handler_properties
            .get(&target)
            .is_some_and(HashMap::is_empty);
        if remove_target {
            self.handler_properties.remove(&target);
        }
        let callback_id = match removed {
            Some(EventHandlerPropertyEntry::Callback { callback_id, .. }) => Some(callback_id),
            Some(EventHandlerPropertyEntry::Null) | None => None,
        };
        self.remove_event_type_registration_id_if_unused(target, event_type);
        callback_id
    }

    fn event_handler_property_value(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<EventHandlerPropertyValue> {
        match self
            .handler_properties
            .get(&target)
            .and_then(|target_handlers| target_handlers.get(event_type))?
        {
            EventHandlerPropertyEntry::Callback { callback_id, .. } => {
                Some(EventHandlerPropertyValue::Callback(*callback_id))
            }
            EventHandlerPropertyEntry::Null => Some(EventHandlerPropertyValue::Null),
        }
    }

    pub(crate) fn event_handler_property_callback_id(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<Option<crate::native_bridge::EventCallbackId>> {
        match self
            .handler_properties
            .get(&target)
            .and_then(|target_handlers| target_handlers.get(event_type))?
        {
            EventHandlerPropertyEntry::Callback { callback_id, .. } => Some(Some(*callback_id)),
            EventHandlerPropertyEntry::Null => Some(None),
        }
    }

    pub(crate) fn listener_callback_ids(
        &self,
        target: EventTargetHandle,
        event_type: &str,
        capture: bool,
    ) -> Vec<crate::native_bridge::EventCallbackId> {
        self.listeners
            .get(&target)
            .and_then(|target_listeners| target_listeners.get(event_type))
            .into_iter()
            .flatten()
            .filter(|entry| !entry.removed && entry.capture == capture)
            .map(|entry| entry.callback_id)
            .collect()
    }

    pub(crate) fn inspector_listener_snapshots(
        &self,
        target: EventTargetHandle,
    ) -> Vec<EventListenerInspectorSnapshot> {
        let mut snapshots = Vec::new();
        if let Some(target_listeners) = self.listeners.get(&target) {
            for (event_type, entries) in target_listeners {
                let event_type_registration_id = self
                    .event_type_registration_id(target, event_type)
                    .unwrap_or_else(|| entries.first().map(|entry| entry.id).unwrap_or(u64::MAX));
                snapshots.extend(entries.iter().filter(|entry| !entry.removed).map(|entry| {
                    (
                        event_type_registration_id,
                        EventListenerInspectorSnapshot {
                            registration_id: entry.id,
                            event_type: event_type.clone(),
                            callback_id: entry.callback_id,
                            capture: entry.capture,
                            once: entry.once,
                            passive: entry.passive,
                        },
                    )
                }));
            }
        }
        if let Some(target_handlers) = self.handler_properties.get(&target) {
            for (event_type, entry) in target_handlers {
                let EventHandlerPropertyEntry::Callback {
                    callback_id,
                    registration_id,
                } = *entry
                else {
                    continue;
                };
                snapshots.push((
                    self.event_type_registration_id(target, event_type)
                        .unwrap_or(registration_id),
                    EventListenerInspectorSnapshot {
                        registration_id,
                        event_type: event_type.clone(),
                        callback_id,
                        capture: false,
                        once: false,
                        passive: false,
                    },
                ));
            }
        }
        snapshots.sort_by_key(|(event_type_registration_id, snapshot)| {
            (*event_type_registration_id, snapshot.registration_id)
        });
        snapshots
            .into_iter()
            .map(|(_, snapshot)| snapshot)
            .collect()
    }

    pub(crate) fn insert_listener(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        registration: EventListenerRegistration,
    ) {
        let id = self.allocate_listener_id();
        self.ensure_event_type_registration_id(target, event_type, id);
        let entries = self
            .listeners
            .entry(target)
            .or_default()
            .entry(event_type.to_owned())
            .or_default();
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "event_listener_registered",
                id,
                event_type,
                ?target,
                capture = registration.capture,
                passive = registration.passive,
                function_name = registration.function_name.as_str(),
                script_id = registration.script_id,
                script_url = registration.script_url.as_deref().unwrap_or(""),
                line_number = registration.line_number.map(u64::from),
                column_number = registration.column_number.map(u64::from),
            );
        }
        entries.push(EventListenerEntry {
            id,
            callback_id: registration.callback_id,
            function_name: registration.function_name,
            script_id: registration.script_id,
            script_url: registration.script_url,
            line_number: registration.line_number,
            column_number: registration.column_number,
            capture: registration.capture,
            once: registration.once,
            passive: registration.passive,
            removed: false,
        });
    }

    pub(crate) fn remove_listener_by_id(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: crate::native_bridge::EventCallbackId,
        capture: bool,
    ) -> bool {
        let Some(target_listeners) = self.listeners.get_mut(&target) else {
            return false;
        };
        let Some(entries) = target_listeners.get_mut(event_type) else {
            return false;
        };
        let before = entries.len();
        entries.retain(|entry| entry.callback_id != callback_id || entry.capture != capture);
        let removed = entries.len() != before;
        if entries.is_empty() {
            target_listeners.remove(event_type);
        }
        if target_listeners.is_empty() {
            self.listeners.remove(&target);
        }
        self.remove_event_type_registration_id_if_unused(target, event_type);
        removed
    }

    pub(crate) fn remove_callback_registrations(
        &mut self,
        retired: &std::collections::HashSet<crate::native_bridge::EventCallbackId>,
    ) {
        for target_listeners in self.listeners.values_mut() {
            target_listeners.retain(|_, entries| {
                entries.retain(|entry| !retired.contains(&entry.callback_id));
                !entries.is_empty()
            });
        }
        self.listeners
            .retain(|_, target_listeners| !target_listeners.is_empty());
        for target_handlers in self.handler_properties.values_mut() {
            for entry in target_handlers.values_mut() {
                if matches!(
                    entry,
                    EventHandlerPropertyEntry::Callback { callback_id, .. }
                        if retired.contains(callback_id)
                ) {
                    // Absence enables the legacy wrapper fallback, which can
                    // rediscover a callback from its retired realm.
                    *entry = EventHandlerPropertyEntry::Null;
                }
            }
        }
        self.retain_active_event_type_registration_ids();
    }

    pub(crate) fn has_listener(&self, target: EventTargetHandle, event_type: &str) -> bool {
        self.listeners
            .get(&target)
            .and_then(|target_listeners| target_listeners.get(event_type))
            .is_some_and(|entries| entries.iter().any(|entry| !entry.removed))
    }

    fn allocate_listener_id(&mut self) -> u64 {
        let id = self.next_listener_id;
        self.next_listener_id = self
            .next_listener_id
            .checked_add(1)
            .expect("event listener registration id overflow");
        id
    }

    fn ensure_event_type_registration_id(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        registration_id: u64,
    ) {
        self.event_type_registration_ids
            .entry(target)
            .or_default()
            .entry(event_type.to_owned())
            .or_insert(registration_id);
    }

    fn event_type_registration_id(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<u64> {
        self.event_type_registration_ids
            .get(&target)
            .and_then(|event_types| event_types.get(event_type))
            .copied()
    }

    fn event_type_has_live_registration(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> bool {
        self.listeners
            .get(&target)
            .and_then(|listeners| listeners.get(event_type))
            .is_some_and(|entries| entries.iter().any(|entry| !entry.removed))
            || self
                .handler_properties
                .get(&target)
                .and_then(|handlers| handlers.get(event_type))
                .is_some_and(|entry| matches!(entry, EventHandlerPropertyEntry::Callback { .. }))
    }

    fn remove_event_type_registration_id_if_unused(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
    ) {
        if self.event_type_has_live_registration(target, event_type) {
            return;
        }
        let Some(event_types) = self.event_type_registration_ids.get_mut(&target) else {
            return;
        };
        event_types.remove(event_type);
        if event_types.is_empty() {
            self.event_type_registration_ids.remove(&target);
        }
    }

    fn retain_active_event_type_registration_ids(&mut self) {
        let listeners = &self.listeners;
        let handler_properties = &self.handler_properties;
        self.event_type_registration_ids
            .retain(|target, event_types| {
                event_types.retain(|event_type, _| {
                    listeners
                        .get(target)
                        .and_then(|target_listeners| target_listeners.get(event_type))
                        .is_some_and(|entries| entries.iter().any(|entry| !entry.removed))
                        || handler_properties
                            .get(target)
                            .and_then(|handlers| handlers.get(event_type))
                            .is_some_and(|entry| {
                                matches!(entry, EventHandlerPropertyEntry::Callback { .. })
                            })
                });
                !event_types.is_empty()
            });
    }

    fn call_listeners_filtered<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        target: EventTargetHandle,
        invocation_target_in_shadow_tree: bool,
        event_type: &str,
        event: v8::Local<'s, v8::Object>,
        capture_only: bool,
        at_target: bool,
    ) -> std::result::Result<DispatchStatus, String> {
        let snapshot: Vec<(
            u64,
            crate::native_bridge::EventCallbackId,
            String,
            i32,
            Option<String>,
            Option<u32>,
            Option<u32>,
            bool,
            bool,
            bool,
        )> = {
            let Some(target_map) = self.listeners.get(&target) else {
                return Ok(DispatchStatus::Continue);
            };
            let Some(entries) = target_map.get(event_type) else {
                return Ok(DispatchStatus::Continue);
            };
            entries
                .iter()
                .filter(|entry| !entry.removed)
                .filter(|entry| {
                    (at_target && entry.capture == capture_only)
                        || (capture_only && entry.capture)
                        || (!capture_only && !entry.capture)
                })
                .map(|entry| {
                    (
                        entry.id,
                        entry.callback_id,
                        entry.function_name.clone(),
                        entry.script_id,
                        entry.script_url.clone(),
                        entry.line_number,
                        entry.column_number,
                        entry.capture,
                        entry.once,
                        entry.passive,
                    )
                })
                .collect()
        };

        if snapshot.is_empty() {
            return Ok(DispatchStatus::Continue);
        }

        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "event_listener_batch_start",
                event_type,
                ?target,
                capture_only,
                at_target,
                listener_count = snapshot.len(),
            );
        }
        for (
            listener_index,
            (
                listener_id,
                callback_id,
                function_name,
                script_id,
                script_url,
                line_number,
                column_number,
                capture_flag,
                is_once,
                is_passive,
            ),
        ) in snapshot.into_iter().enumerate()
        {
            let still_registered = self
                .listeners
                .get(&target)
                .and_then(|listeners| listeners.get(event_type))
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        !entry.removed
                            && entry.capture == capture_flag
                            && entry.callback_id == callback_id
                    })
                });

            if !still_registered {
                continue;
            }

            let Some(prepared) = unsafe { &*host_ptr }.prepare_event_callback(scope, callback_id)
            else {
                self.remove_listener_by_id(target, event_type, callback_id, capture_flag);
                unsafe {
                    (&mut *host_ptr).unregister_abort_target_listener(callback_id);
                    (&mut *host_ptr).release_event_callback(callback_id);
                }
                tracing::debug!(
                    listener_id,
                    ?target,
                    event_type,
                    "skipped event listener owned by a retired execution context"
                );
                continue;
            };

            if is_once {
                self.remove_listener_by_id(target, event_type, callback_id, capture_flag);
                unsafe {
                    (&mut *host_ptr).unregister_abort_target_listener(callback_id);
                    (&mut *host_ptr).release_event_callback(callback_id);
                }
            }

            let listener_name = format!("{event_type} listener");
            set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, is_passive);
            let listener_started = timing_enabled.then(Instant::now);
            let layout_trace_before =
                timing_enabled.then(|| unsafe { &*host_ptr }.layout_metric_trace_snapshot());
            let _ = invoke_prepared_event_callback(
                scope,
                host_ptr,
                invocation_target_in_shadow_tree,
                event_type,
                &listener_name,
                prepared,
                target,
                event,
                &[event.into()],
            );
            if let Some(listener_started) = listener_started {
                let layout_trace = layout_trace_before
                    .map(|before| {
                        unsafe { &*host_ptr }
                            .layout_metric_trace_snapshot()
                            .saturating_delta(before)
                    })
                    .unwrap_or_default();
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    stage = "event_listener_invoked",
                    listener_id,
                    event_type,
                    ?target,
                    capture_only,
                    at_target,
                    listener_index,
                    function_name = function_name.as_str(),
                    script_id,
                    script_url = script_url.as_deref().unwrap_or(""),
                    line_number = line_number.map(u64::from),
                    column_number = column_number.map(u64::from),
                    capture_flag,
                    is_once,
                    is_passive,
                    layout_client_rect_count = layout_trace.client_rect_count,
                    layout_client_rect_ms = layout_trace.client_rect_ms(),
                    layout_offset_parent_count = layout_trace.offset_parent_count,
                    layout_offset_parent_ms = layout_trace.offset_parent_ms(),
                    layout_offset_position_count = layout_trace.offset_position_count,
                    layout_offset_position_ms = layout_trace.offset_position_ms(),
                    elapsed_ms = listener_started.elapsed().as_millis(),
                );
            }
            set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);

            if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT) {
                return Ok(DispatchStatus::StopImmediate);
            }
        }
        if event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT) {
            Ok(DispatchStatus::StopPropagation)
        } else {
            Ok(DispatchStatus::Continue)
        }
    }
}

pub(crate) fn dispatch_public_event<'s, 'i>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, 'i>,
    host_ptr: *mut JsContextHost,
    dispatch_target: EventTargetHandle,
    propagation_path: &[EventTargetHandle],
    event: v8::Local<'s, v8::Object>,
) -> std::result::Result<PublicEventDispatchResult, String> {
    dispatch_public_event_with_original_target(
        registry,
        scope,
        host_ptr,
        dispatch_target,
        dispatch_target,
        propagation_path,
        event,
    )
}

pub(crate) fn dispatch_public_event_with_original_target<'s, 'i>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, 'i>,
    host_ptr: *mut JsContextHost,
    dispatch_target: EventTargetHandle,
    original_target: EventTargetHandle,
    propagation_path: &[EventTargetHandle],
    event: v8::Local<'s, v8::Object>,
) -> std::result::Result<PublicEventDispatchResult, String> {
    unsafe { &*host_ptr }.debug_assert_not_in_structural_mutation("public event dispatch");
    let target_object = event_target_object(scope, host_ptr, dispatch_target)?;
    let event_type = normalize_public_event_record(scope, event, target_object.into())?;

    let bubbles = event_bool_property(scope, event, "bubbles");
    let original_related_target = event_related_target_handle(scope, host_ptr, event);
    let original_source_target = event_source_target_handle(scope, host_ptr, event);
    let filtered_path;
    let propagation_path = if let Some(related_target) = original_related_target {
        filtered_path = propagation_path_for_related_target(
            host_ptr,
            original_target,
            related_target,
            propagation_path,
        );
        filtered_path.as_slice()
    } else {
        propagation_path
    };
    let post_dispatch_target =
        post_dispatch_retargeted_target(host_ptr, original_target, propagation_path);
    let post_dispatch_related_target = original_related_target.map(|original_related_target| {
        post_dispatch_retargeted_target(host_ptr, original_related_target, propagation_path)
    });
    let post_dispatch_source_target = original_source_target.and_then(|source_target| {
        post_dispatch_retargeted_target(host_ptr, source_target, propagation_path)
    });
    let invocation_targets_in_shadow_tree = propagation_path
        .iter()
        .copied()
        .filter(|target| invocation_target_is_in_shadow_tree(host_ptr, *target))
        .collect::<Vec<_>>();

    set_event_internal_flag(scope, event, EVENT_DISPATCHING_SLOT, true);
    // Per DOM spec § "event dispatch": each invocation of the inner-invoke
    // algorithm bails out when the stop-propagation (or stop-immediate-
    // propagation) flag is set. A test that calls stopPropagation() (or
    // assigns cancelBubble = true) BEFORE dispatchEvent() expects the very
    // first phase to short-circuit, so seed `stopped` from the pre-existing
    // flag rather than always starting at false.
    let mut stopped = event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT)
        || event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT);

    if !stopped && propagation_path.len() > 1 {
        for ancestor in propagation_path[1..].iter().rev() {
            if !event_target_is_current(host_ptr, *ancestor) {
                stopped = true;
                break;
            }
            let ancestor_obj = event_target_object(scope, host_ptr, *ancestor)?;
            set_event_current_target(scope, event, ancestor_obj.into());
            set_event_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_target,
                *ancestor,
            )?;
            set_event_related_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_related_target,
                *ancestor,
            )?;
            set_event_source_for_current_target(
                scope,
                host_ptr,
                event,
                original_source_target,
                *ancestor,
            )?;
            set_event_composed_path_for_current_target(
                scope,
                host_ptr,
                event,
                propagation_path,
                *ancestor,
            )?;
            set_event_phase(
                scope,
                event,
                event_phase_for_current_target(host_ptr, original_target, *ancestor, 1),
            );

            let status = call_event_target_listeners_filtered(
                registry,
                scope,
                host_ptr,
                *ancestor,
                invocation_targets_in_shadow_tree.contains(ancestor),
                &event_type,
                event,
                true,
                false,
            )?;
            if status == DispatchStatus::StopImmediate || status == DispatchStatus::StopPropagation
            {
                stopped = true;
                break;
            }
        }
    }

    if !stopped && !propagation_path.is_empty() {
        set_event_current_target(scope, event, target_object.into());
        set_event_target_for_current_target(
            scope,
            host_ptr,
            event,
            original_target,
            dispatch_target,
        )?;
        set_event_related_target_for_current_target(
            scope,
            host_ptr,
            event,
            original_related_target,
            dispatch_target,
        )?;
        set_event_source_for_current_target(
            scope,
            host_ptr,
            event,
            original_source_target,
            dispatch_target,
        )?;
        set_event_composed_path_for_current_target(
            scope,
            host_ptr,
            event,
            propagation_path,
            dispatch_target,
        )?;
        set_event_phase(scope, event, 2);

        invoke_event_target_handler_property(
            registry,
            scope,
            host_ptr,
            dispatch_target,
            invocation_targets_in_shadow_tree.contains(&dispatch_target),
            target_object,
            &event_type,
            event,
        );

        let stop_immediate = event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT);
        let stop_prop = event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT);
        if !stop_immediate {
            if stop_prop {
                stopped = true;
            }
            let status = call_event_target_listeners_filtered(
                registry,
                scope,
                host_ptr,
                dispatch_target,
                invocation_targets_in_shadow_tree.contains(&dispatch_target),
                &event_type,
                event,
                true,
                true,
            )?;
            if status == DispatchStatus::StopPropagation {
                stopped = true;
            }
            if status == DispatchStatus::StopImmediate {
                stopped = true;
            } else if !stopped {
                let status = call_event_target_listeners_filtered(
                    registry,
                    scope,
                    host_ptr,
                    dispatch_target,
                    invocation_targets_in_shadow_tree.contains(&dispatch_target),
                    &event_type,
                    event,
                    false,
                    true,
                )?;
                if status == DispatchStatus::StopImmediate
                    || status == DispatchStatus::StopPropagation
                {
                    stopped = true;
                }
            }
        } else {
            stopped = true;
        }
    }

    if !stopped && !bubbles && propagation_path.len() > 1 {
        for ancestor in propagation_path[1..].iter() {
            if !event_target_is_current(host_ptr, *ancestor) {
                break;
            }
            if retarget_event_target(host_ptr, original_target, *ancestor) != *ancestor {
                continue;
            }
            let ancestor_obj = event_target_object(scope, host_ptr, *ancestor)?;
            set_event_current_target(scope, event, ancestor_obj.into());
            set_event_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_target,
                *ancestor,
            )?;
            set_event_related_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_related_target,
                *ancestor,
            )?;
            set_event_source_for_current_target(
                scope,
                host_ptr,
                event,
                original_source_target,
                *ancestor,
            )?;
            set_event_composed_path_for_current_target(
                scope,
                host_ptr,
                event,
                propagation_path,
                *ancestor,
            )?;
            set_event_phase(scope, event, 2);

            let status = call_event_target_listeners_filtered(
                registry,
                scope,
                host_ptr,
                *ancestor,
                invocation_targets_in_shadow_tree.contains(ancestor),
                &event_type,
                event,
                true,
                true,
            )?;
            if status == DispatchStatus::StopImmediate || status == DispatchStatus::StopPropagation
            {
                stopped = true;
                break;
            }
            let status = call_event_target_listeners_filtered(
                registry,
                scope,
                host_ptr,
                *ancestor,
                invocation_targets_in_shadow_tree.contains(ancestor),
                &event_type,
                event,
                false,
                true,
            )?;
            if status == DispatchStatus::StopImmediate || status == DispatchStatus::StopPropagation
            {
                stopped = true;
                break;
            }
        }
    }

    if !stopped && bubbles && propagation_path.len() > 1 {
        for ancestor in propagation_path[1..].iter() {
            if !event_target_is_current(host_ptr, *ancestor) {
                break;
            }
            // HTMLFormElement's local event handling stops a nested submit/reset before the
            // ancestor form's bubble listeners run. Capture still traverses the full path.
            if stops_submit_or_reset_at_ancestor_form(
                host_ptr,
                original_target,
                *ancestor,
                &event_type,
            ) {
                break;
            }
            let ancestor_obj = event_target_object(scope, host_ptr, *ancestor)?;
            set_event_current_target(scope, event, ancestor_obj.into());
            set_event_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_target,
                *ancestor,
            )?;
            set_event_related_target_for_current_target(
                scope,
                host_ptr,
                event,
                original_related_target,
                *ancestor,
            )?;
            set_event_source_for_current_target(
                scope,
                host_ptr,
                event,
                original_source_target,
                *ancestor,
            )?;
            set_event_composed_path_for_current_target(
                scope,
                host_ptr,
                event,
                propagation_path,
                *ancestor,
            )?;
            set_event_phase(
                scope,
                event,
                event_phase_for_current_target(host_ptr, original_target, *ancestor, 3),
            );

            invoke_event_target_handler_property(
                registry,
                scope,
                host_ptr,
                *ancestor,
                invocation_targets_in_shadow_tree.contains(ancestor),
                ancestor_obj,
                &event_type,
                event,
            );
            if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT) {
                break;
            }
            let status = call_event_target_listeners_filtered(
                registry,
                scope,
                host_ptr,
                *ancestor,
                invocation_targets_in_shadow_tree.contains(ancestor),
                &event_type,
                event,
                false,
                false,
            )?;
            if status == DispatchStatus::StopImmediate
                || status == DispatchStatus::StopPropagation
                || event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT)
            {
                break;
            }
        }
    }

    set_event_post_dispatch_targets(
        scope,
        host_ptr,
        event,
        post_dispatch_target,
        post_dispatch_related_target,
    )?;
    set_event_post_dispatch_source(scope, host_ptr, event, post_dispatch_source_target)?;
    if event_type == "mouseover"
        && !event_default_prevented(scope, event)
        && let EventTargetHandle::Node(handle) = dispatch_target
    {
        crate::native_bridge::element::perform_hover_interest_default_action_for_dispatched_event(
            scope, host_ptr, handle,
        );
    }
    if event_type == "keydown"
        && event_bool_property(scope, event, "isTrusted")
        && !event_default_prevented(scope, event)
    {
        crate::native_bridge::element::perform_tab_focus_default_action_for_dispatched_event(
            scope, host_ptr, event,
        );
        crate::native_bridge::element::perform_access_key_default_action_for_dispatched_event(
            scope, host_ptr, event,
        );
        crate::native_bridge::element::perform_dialog_escape_default_action_for_dispatched_event(
            scope, host_ptr, event,
        );
    }
    set_event_phase(scope, event, 0);
    set_event_internal_flag(scope, event, EVENT_DISPATCHING_SLOT, false);
    set_event_internal_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT, false);
    set_event_internal_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT, false);
    set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
    let current_target_key = v8str(scope, "currentTarget");
    let _ = event.set(scope, current_target_key.into(), v8::null(scope).into());
    clear_event_composed_path(scope, event);
    Ok(PublicEventDispatchResult {
        default_prevented: event_default_prevented(scope, event),
    })
}

fn stops_submit_or_reset_at_ancestor_form(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    current_target: EventTargetHandle,
    event_type: &str,
) -> bool {
    if !matches!(event_type, "submit" | "reset") || current_target == original_target {
        return false;
    }
    let EventTargetHandle::Node(handle) = current_target else {
        return false;
    };
    unsafe { &*host_ptr }
        .dom_host()
        .is_html_element_named(handle, "form")
}

pub(crate) fn dispatch_host_event(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dispatch_target: EventTargetHandle,
    event_target: EventTargetHandle,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) -> std::result::Result<(), String> {
    unsafe { &*host_ptr }.debug_assert_not_in_structural_mutation("host event dispatch");
    let current_target_key = v8str(scope, "currentTarget");
    let target_object = event_target_object(scope, host_ptr, dispatch_target)?;
    let target_value = event_target_value(scope, host_ptr, event_target)?;
    let event = create_host_event(
        scope,
        event_type,
        target_object.into(),
        target_value,
        bubbles,
        cancelable,
    )?;

    let invocation_target_in_shadow_tree =
        invocation_target_is_in_shadow_tree(host_ptr, dispatch_target);
    let dispatch_result = run_host_event_dispatch(
        registry,
        scope,
        host_ptr,
        dispatch_target,
        invocation_target_in_shadow_tree,
        target_object,
        event_type,
        event,
    );
    let _ = event.set(scope, current_target_key.into(), v8::null(scope).into());
    dispatch_result
}

fn run_host_event_dispatch<'s>(
    registry: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    dispatch_target: EventTargetHandle,
    invocation_target_in_shadow_tree: bool,
    target_object: v8::Local<'s, v8::Object>,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), String> {
    invoke_event_handler_property(
        registry,
        scope,
        host_ptr,
        dispatch_target,
        invocation_target_in_shadow_tree,
        target_object,
        event_type,
        event,
    );

    let status = registry.call_listeners_filtered(
        scope,
        host_ptr,
        dispatch_target,
        invocation_target_in_shadow_tree,
        event_type,
        event,
        true,
        true,
    )?;
    if status != DispatchStatus::StopImmediate {
        let _ = registry.call_listeners_filtered(
            scope,
            host_ptr,
            dispatch_target,
            invocation_target_in_shadow_tree,
            event_type,
            event,
            false,
            true,
        )?;
    }
    Ok(())
}

fn set_event_current_target(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    value: v8::Local<'_, v8::Value>,
) {
    let key = v8str(scope, "currentTarget");
    let _ = event.set(scope, key.into(), value);
}

fn set_event_target(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    value: v8::Local<'_, v8::Value>,
) {
    let key = v8str(scope, "target");
    let _ = event.set(scope, key.into(), value);
    let src_element_key = v8str(scope, "srcElement");
    let _ = event.set(scope, src_element_key.into(), value);
}

fn set_event_phase(scope: &mut v8::PinScope<'_, '_>, event: v8::Local<'_, v8::Object>, phase: u32) {
    let key = v8str(scope, "eventPhase");
    let _ = event.set(
        scope,
        key.into(),
        v8::Integer::new_from_unsigned(scope, phase).into(),
    );
}

fn set_event_internal_flag(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &str,
    value: bool,
) {
    set_private_value(scope, event, key, v8::Boolean::new(scope, value).into());
}

fn event_internal_bool_flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: &str,
) -> bool {
    get_private_value(scope, event, key).is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn event_dispatch_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> DispatchStatus {
    if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_SLOT) {
        DispatchStatus::StopImmediate
    } else if event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT) {
        DispatchStatus::StopPropagation
    } else {
        DispatchStatus::Continue
    }
}

/// Per-spec `(bubbles, cancelable)` defaults for host-dispatched events.
pub(crate) fn host_event_defaults(event_type: &str) -> (bool, bool) {
    match event_type {
        // Events that bubble per spec
        "DOMContentLoaded" => (true, false),
        "readystatechange" => (false, false),
        "scroll" => (true, false),
        "scrollend" => (true, false),
        "visibilitychange" => (true, false),
        // Non-bubbling events (default)
        "load" | "error" | "abort" | "unload" | "beforeunload" => (false, false),
        _ => (false, false),
    }
}

pub(crate) fn create_host_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    current_target: v8::Local<'s, v8::Value>,
    target: v8::Local<'s, v8::Value>,
    bubbles: bool,
    cancelable: bool,
) -> std::result::Result<v8::Local<'s, v8::Object>, String> {
    // The intrinsic constructor owns the complete Event initialization and
    // prototype surface. Host dispatch only adds browser-controlled trust and
    // target state; installing methods or accessors here would incorrectly
    // turn prototype members into per-event own properties.
    let event = construct_event_instance(scope, event_type, bubbles, cancelable)?;
    mark_event_trusted(scope, event);
    set_event_target(scope, event, target);
    set_event_current_target(scope, event, current_target);
    clear_event_composed_path(scope, event);

    Ok(event)
}

fn construct_event_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) -> std::result::Result<v8::Local<'s, v8::Object>, String> {
    let constructor =
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Event")
            .map_err(|error| format!("failed to resolve intrinsic Event constructor: {error}"))?;
    let init = HostEventInitDeclaration::new(bubbles, cancelable)
        .bind(scope)
        .map_err(|error| format!("failed to build EventInit dictionary: {error}"))?;
    let event_type = v8_string(scope, event_type)
        .ok_or_else(|| format!("failed to allocate event type `{event_type}`"))?;
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    constructor
        .new_instance(&scope, &[event_type.into(), init.into()])
        .ok_or_else(|| "intrinsic Event constructor failed for a host event".to_owned())
}

fn normalize_public_event_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    default_target: v8::Local<'s, v8::Value>,
) -> std::result::Result<String, String> {
    let type_key = v8str(scope, "type");
    let target_key = v8str(scope, "target");
    let current_target_key = v8str(scope, "currentTarget");

    let event_type_value = event
        .get(scope, type_key.into())
        .ok_or_else(|| "Failed to execute 'dispatchEvent': event type is required.".to_owned())?;
    if event_type_value.is_null_or_undefined() {
        return Err("Failed to execute 'dispatchEvent': event type is required.".to_owned());
    }
    let event_type = event_type_value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();

    let target_value = event.get(scope, target_key.into());
    if target_value.is_none() || target_value.is_some_and(|value| value.is_null_or_undefined()) {
        let _ = event.set(scope, target_key.into(), default_target);
        let _ = event.set(scope, v8str(scope, "srcElement").into(), default_target);
    }
    let _ = event.set(scope, current_target_key.into(), default_target);
    // `dispatchEvent()` brand-checks the initialized Event private slot before
    // reaching this function. Its methods and legacy accessors therefore come
    // from the intrinsic Event prototype and must not be shadowed here.
    clear_event_composed_path(scope, event);

    Ok(event_type)
}

fn event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    let default_prevented_key = v8str(scope, "defaultPrevented");
    event
        .get(scope, default_prevented_key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn event_target_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) -> std::result::Result<v8::Local<'s, v8::Object>, String> {
    match target {
        EventTargetHandle::Window => Ok(scope.get_current_context().global(scope)),
        EventTargetHandle::ChildWindow(target) => unsafe { &mut *host_ptr }
            .child_window_event_target_wrapper(scope, target)
            .ok_or_else(|| format!("stale child Window event target `{target:?}`")),
        EventTargetHandle::Node(handle) => {
            let wrapper = unsafe { &mut *host_ptr }
                .native_bridge_mut()
                .wrap_handle(scope, host_ptr, handle)
                .ok_or_else(|| format!("failed to resolve event target wrapper `{handle:?}`"))?;
            Ok(get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT).unwrap_or(wrapper))
        }
    }
}

pub(crate) fn event_target_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) -> std::result::Result<v8::Local<'s, v8::Value>, String> {
    Ok(match target {
        EventTargetHandle::Window => scope.get_current_context().global(scope).into(),
        EventTargetHandle::ChildWindow(target) => {
            event_target_object(scope, host_ptr, EventTargetHandle::ChildWindow(target))?.into()
        }
        EventTargetHandle::Node(handle) => {
            event_target_object(scope, host_ptr, EventTargetHandle::Node(handle))?.into()
        }
    })
}

fn event_target_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
    event: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    match target {
        EventTargetHandle::Window => scope.get_current_context().global(scope).into(),
        EventTargetHandle::ChildWindow(target) => unsafe { &mut *host_ptr }
            .child_window_event_target_wrapper(scope, target)
            .map(Into::into)
            .unwrap_or_else(|| v8::undefined(scope).into()),
        EventTargetHandle::Node(_) => {
            let key = v8str(scope, "currentTarget");
            event
                .get(scope, key.into())
                .unwrap_or_else(|| v8::undefined(scope).into())
        }
    }
}

fn set_event_target_for_current_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    original_target: EventTargetHandle,
    current_target: EventTargetHandle,
) -> std::result::Result<(), String> {
    let target = retarget_event_target(host_ptr, original_target, current_target);
    let target_value = event_target_value(scope, host_ptr, target)?;
    set_event_target(scope, event, target_value);
    Ok(())
}

fn set_event_related_target_for_current_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    original_related_target: Option<EventTargetHandle>,
    current_target: EventTargetHandle,
) -> std::result::Result<(), String> {
    let Some(original_related_target) = original_related_target else {
        return Ok(());
    };
    let related_target = retarget_event_target(host_ptr, original_related_target, current_target);
    let related_value = event_target_value(scope, host_ptr, related_target)?;
    let _ = event.set(scope, v8str(scope, "relatedTarget").into(), related_value);
    Ok(())
}

fn set_event_source_for_current_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    original_source_target: Option<EventTargetHandle>,
    current_target: EventTargetHandle,
) -> std::result::Result<(), String> {
    let Some(original_source_target) = original_source_target else {
        return Ok(());
    };
    let source = retarget_event_target(host_ptr, original_source_target, current_target);
    let source_value = event_target_value(scope, host_ptr, source)?;
    let _ = event.set(scope, v8str(scope, "source").into(), source_value);
    Ok(())
}

fn set_event_composed_path_for_current_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    propagation_path: &[EventTargetHandle],
    current_target: EventTargetHandle,
) -> std::result::Result<(), String> {
    let visible_path = visible_event_path(host_ptr, propagation_path, current_target);
    let path = visible_path
        .into_iter()
        .map(|handle| match handle {
            EventTargetHandle::Window => Ok(event_window_value_for_path(
                scope,
                host_ptr,
                propagation_path,
            )),
            EventTargetHandle::ChildWindow(_) => event_target_value(scope, host_ptr, handle),
            _ => event_target_value(scope, host_ptr, handle),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let path = serialize_v8_iter_array(scope, path).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_event_composed_path(scope, event, path);
    Ok(())
}

fn event_window_value_for_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    propagation_path: &[EventTargetHandle],
) -> v8::Local<'s, v8::Value> {
    let document_handle = {
        let runtime = unsafe { &*host_ptr };
        propagation_path.iter().find_map(|entry| {
            let EventTargetHandle::Node(handle) = entry else {
                return None;
            };
            runtime
                .dom_host()
                .node(*handle)
                .is_some_and(moli_dom::native::Node::is_document)
                .then_some(*handle)
        })
    };
    if let Some(document_handle) = document_handle {
        let runtime = unsafe { &mut *host_ptr };
        if let Some(child_handle) =
            runtime.child_browsing_context_handle_by_document_handle(scope, document_handle)
            && let Some(window) = runtime.child_browsing_context_window_wrapper(scope, child_handle)
        {
            return window.into();
        }
    }
    scope.get_current_context().global(scope).into()
}

fn event_phase_for_current_target(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    current_target: EventTargetHandle,
    ancestor_phase: u32,
) -> u32 {
    if retarget_event_target(host_ptr, original_target, current_target) == current_target {
        2
    } else {
        ancestor_phase
    }
}

fn set_event_post_dispatch_targets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    post_dispatch_target: Option<EventTargetHandle>,
    post_dispatch_related_target: Option<Option<EventTargetHandle>>,
) -> std::result::Result<(), String> {
    let target_value = match post_dispatch_target {
        Some(target) => event_target_value(scope, host_ptr, target)?,
        None => v8::null(scope).into(),
    };
    set_event_target(scope, event, target_value);

    if let Some(related_target) = post_dispatch_related_target {
        let related_value = match related_target {
            Some(related_target) => event_target_value(scope, host_ptr, related_target)?,
            None => v8::null(scope).into(),
        };
        let _ = event.set(scope, v8str(scope, "relatedTarget").into(), related_value);
    }

    Ok(())
}

fn set_event_post_dispatch_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'s, v8::Object>,
    post_dispatch_source: Option<EventTargetHandle>,
) -> std::result::Result<(), String> {
    let Some(source_target) = post_dispatch_source else {
        return Ok(());
    };
    let source_value = event_target_value(scope, host_ptr, source_target)?;
    let _ = event.set(scope, v8str(scope, "source").into(), source_value);
    Ok(())
}

fn post_dispatch_retargeted_target(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    propagation_path: &[EventTargetHandle],
) -> Option<EventTargetHandle> {
    let current_target = *propagation_path.last()?;
    let runtime = unsafe { &*host_ptr };
    if matches!(
        current_target,
        EventTargetHandle::Node(handle) if runtime.dom_host().is_shadow_root(handle)
    ) {
        return None;
    }
    Some(retarget_event_target(
        host_ptr,
        original_target,
        current_target,
    ))
}

fn invocation_target_is_in_shadow_tree(
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) -> bool {
    let EventTargetHandle::Node(handle) = target else {
        return false;
    };
    let runtime = unsafe { &*host_ptr };
    runtime.dom_host().is_shadow_root(handle)
        || runtime.dom_host().containing_shadow_root(handle).is_some()
}

fn event_related_target_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'_, v8::Object>,
) -> Option<EventTargetHandle> {
    let value = event.get(scope, v8str(scope, "relatedTarget").into())?;
    if value.is_null_or_undefined() || !value.is_object() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, object)
        && runtime_ptr == host_ptr
    {
        return Some(EventTargetHandle::Node(handle));
    }
    crate::native_bridge::document::detached_native_handle_for_runtime(scope, host_ptr, object)
        .map(EventTargetHandle::Node)
}

fn event_source_target_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'_, v8::Object>,
) -> Option<EventTargetHandle> {
    let value = event.get(scope, v8str(scope, "source").into())?;
    if value.is_null_or_undefined() || !value.is_object() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, object).ok()?;
    (runtime_ptr == host_ptr).then_some(EventTargetHandle::Node(handle))
}

fn propagation_path_for_related_target(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    original_related_target: EventTargetHandle,
    propagation_path: &[EventTargetHandle],
) -> Vec<EventTargetHandle> {
    let mut path = Vec::new();
    for &current_target in propagation_path {
        let target = retarget_event_target(host_ptr, original_target, current_target);
        let related_target =
            retarget_event_target(host_ptr, original_related_target, current_target);
        if target == related_target
            && retargets_are_hidden_by_shadow_boundary(
                host_ptr,
                original_target,
                original_related_target,
                current_target,
            )
        {
            continue;
        }
        path.push(current_target);
    }
    path
}

fn retargets_are_hidden_by_shadow_boundary(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    original_related_target: EventTargetHandle,
    current_target: EventTargetHandle,
) -> bool {
    [original_target, original_related_target]
        .into_iter()
        .any(|target| target_has_inaccessible_shadow_root(host_ptr, target, current_target))
}

fn target_has_inaccessible_shadow_root(
    host_ptr: *mut JsContextHost,
    target: EventTargetHandle,
    current_target: EventTargetHandle,
) -> bool {
    let EventTargetHandle::Node(handle) = target else {
        return false;
    };
    let runtime = unsafe { &*host_ptr };
    let mut current = handle;
    loop {
        let Some(root) = runtime.dom_host().containing_shadow_root(current) else {
            return false;
        };
        if !current_target_can_access_shadow_root(host_ptr, current_target, root) {
            return true;
        }
        let Some(host) = runtime.dom_host().shadow_root_host(root) else {
            return false;
        };
        current = host;
    }
}

fn retarget_event_target(
    host_ptr: *mut JsContextHost,
    original_target: EventTargetHandle,
    current_target: EventTargetHandle,
) -> EventTargetHandle {
    let EventTargetHandle::Node(mut candidate) = original_target else {
        return original_target;
    };
    let runtime = unsafe { &*host_ptr };
    loop {
        let Some(shadow_root) = runtime.dom_host().containing_shadow_root(candidate) else {
            return EventTargetHandle::Node(candidate);
        };
        if current_target_can_access_shadow_root(host_ptr, current_target, shadow_root) {
            return EventTargetHandle::Node(candidate);
        }
        let Some(host) = runtime.dom_host().shadow_root_host(shadow_root) else {
            return EventTargetHandle::Node(candidate);
        };
        candidate = host;
    }
}

fn visible_event_path(
    host_ptr: *mut JsContextHost,
    propagation_path: &[EventTargetHandle],
    current_target: EventTargetHandle,
) -> Vec<EventTargetHandle> {
    let runtime = unsafe { &*host_ptr };
    let inaccessible_closed_roots = propagation_path
        .iter()
        .filter_map(|entry| {
            let EventTargetHandle::Node(handle) = entry else {
                return None;
            };
            (runtime.dom_host().is_shadow_root(*handle)
                && runtime.dom_host().shadow_root_mode(*handle).as_deref() == Some("closed")
                && !current_target_can_access_shadow_root(host_ptr, current_target, *handle))
            .then_some(*handle)
        })
        .collect::<Vec<_>>();

    propagation_path
        .iter()
        .copied()
        .filter(|entry| {
            let EventTargetHandle::Node(handle) = entry else {
                return true;
            };
            !inaccessible_closed_roots.iter().any(|closed_root| {
                shadow_including_tree_contains(runtime.dom_host(), *closed_root, *handle)
            })
        })
        .collect()
}

fn current_target_can_access_shadow_root(
    host_ptr: *mut JsContextHost,
    current_target: EventTargetHandle,
    shadow_root: NativeNodeId,
) -> bool {
    let runtime = unsafe { &*host_ptr };
    match current_target {
        EventTargetHandle::Window | EventTargetHandle::ChildWindow(_) => false,
        EventTargetHandle::Node(handle) => {
            shadow_including_tree_contains(runtime.dom_host(), shadow_root, handle)
        }
    }
}

fn shadow_including_tree_contains(
    dom_host: &moli_dom::native::DomHost,
    root: NativeNodeId,
    handle: NativeNodeId,
) -> bool {
    let mut current = handle;
    loop {
        if current == root {
            return true;
        }
        if let Some(parent) = dom_host
            .node(current)
            .and_then(moli_dom::native::Node::parent_node)
        {
            current = parent;
            continue;
        }
        if dom_host.is_shadow_root(current)
            && let Some(host) = dom_host.shadow_root_host(current)
        {
            current = host;
            continue;
        }
        return false;
    }
}

fn event_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    let Some(key) = v8_string(scope, key) else {
        return false;
    };
    event
        .get(scope, key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

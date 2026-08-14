use super::{
    context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT as WINDOW_CHILD_CONTEXT_HANDLE_SLOT,
    context_bootstrap::DOCUMENT_SELECTION_CHANGE_LISTENER_SLOT,
    context_bootstrap::PERFORMANCE_TIME_ORIGIN_SLOT,
    context_bootstrap::dom_time_since_origin_millis,
    context_bootstrap::event_initialized,
    context_bootstrap::event_is_dispatching,
    context_bootstrap::event_is_mouse_event,
    context_bootstrap::increment_performance_event_count,
    context_bootstrap::mark_event_trusted,
    context_bootstrap::performance_slot_number,
    context_bootstrap::simple_event_target_add_event_listener_callback,
    context_bootstrap::simple_event_target_dispatch_event_callback,
    context_bootstrap::simple_event_target_remove_event_listener_callback,
    context_bootstrap::simple_event_target_slot_name,
    context_bootstrap::trusted_script_string_or_type_error,
    document_runtime::{DocumentContentSecurityPolicyViolation, DomHandle, EventTargetHandle},
    native_bridge::{
        ComputedStyleDescriptor, ComputedStylePseudoKey, ComputedStyleTargetKey, JsContextHost,
        PendingWindowMessage, PendingWindowMessageEndpoint, PendingWindowMessageSource,
        RuntimeObservableContextToken, WindowExecutionContextOwner, WindowTaskTarget,
        active_child_window_handle, active_lightweight_popup_id,
        current_or_live_delegate_node_arg_handle,
        element::{
            ComputedStyleTargetContext, STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT,
            STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT, STYLE_DECLARATION_READ_DOCUMENT_SLOT,
            STYLE_DECLARATION_SCREEN_HEIGHT_SLOT, STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
            STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
            STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT,
            STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, computed_style_target_context,
            dispatched_click_activation_target, finish_legacy_activation_for_dispatched_click,
            iframe_handle_viewport, observable_event_offset,
            perform_click_default_action_for_dispatched_event,
            prepare_legacy_activation_for_dispatched_click,
            queue_animation_start_for_listener_target, queue_scroll_observable_effects,
        },
        enter_active_child_window_scope, enter_top_level_lightweight_popup_scope,
        node_runtime_and_handle_from_object, throw_dom_exception,
    },
    reflector::ReflectorId,
    script_provenance::CompiledStringProvenance,
    util::{
        callback_arg_string, context_host_from_global_bridge, context_host_ptr_from_global_bridge,
        context_host_ptr_from_window_object, define_non_enumerable_static_bool_property,
        get_private_value, object_bool_property, object_number_property,
        script_base_url_from_continuation_data, script_base_url_from_host_defined_options,
        set_private_value, throw_type_error, v8_string, v8str,
    },
    webidl,
};
use moli_webapi_declare::WebApiObject;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const WINDOW_SCROLL_X_SLOT: &str = "__moliWindowScrollX";
const WINDOW_SCROLL_Y_SLOT: &str = "__moliWindowScrollY";
const WINDOW_PENDING_ANIMATION_FRAME_TIMESTAMP_SLOT: &str =
    "__moliWindowPendingAnimationFrameTimestamp";
const IDLE_CALLBACK_BUDGET_MS: f64 = 50.0;
const IDLE_DEADLINE_MS_SLOT: &str = "__moliIdleDeadlineMs";
const IDLE_OPPORTUNITY_DELAY_MS: u32 = 1;
const WINDOW_REQUEST_ANIMATION_FRAME_DELAY_MS: u32 = 16;
pub(crate) const TOP_WINDOW_MESSAGE_ENDPOINT_SLOT: &str = "__moliTopWindowMessageEndpoint";

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct WindowMessageEventInitDeclaration<'scope> {
    data: v8::Local<'scope, v8::Value>,
    origin: v8::Local<'scope, v8::String>,
    ports: v8::Local<'scope, v8::Array>,
    source: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct WindowDocumentEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct IdleDeadlineStateDeclaration {
    #[webapi(slot = IDLE_DEADLINE_MS_SLOT)]
    deadline_ms: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.requestAnimationFrame")]
struct WindowRequestAnimationFrameArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to execute 'requestAnimationFrame' on 'Window': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.requestIdleCallback")]
struct WindowRequestIdleCallbackArgs<'s> {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to execute 'requestIdleCallback' on 'Window': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(index = 1, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "IdleDeadline")]
struct IdleDeadlineDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    did_timeout: bool,
    deadline_state: v8::Local<'scope, v8::Object>,
    #[webapi(
        method,
        enumerable,
        callback = window_idle_deadline_time_remaining_callback,
        data = self.deadline_state
    )]
    time_remaining: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventTarget.addEventListener")]
struct WindowAddEventListenerArgs<'s> {
    #[webidl(with = window_add_event_listener_call)]
    call: webidl::ParseOutcome<WindowAddEventListenerCall<'s>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventTarget.removeEventListener")]
struct WindowRemoveEventListenerArgs<'s> {
    #[webidl(with = window_remove_event_listener_call)]
    call: webidl::ParseOutcome<WindowRemoveEventListenerCall<'s>>,
}

struct WindowAddEventListenerCall<'s> {
    event_type: String,
    callback: v8::Local<'s, v8::Object>,
    callback_relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
    options: webidl::EventListenerOptions,
    signal: Option<v8::Local<'s, v8::Object>>,
}

struct WindowRemoveEventListenerCall<'s> {
    event_type: String,
    callback: v8::Local<'s, v8::Object>,
    options: webidl::EventListenerOptions,
}

fn window_add_event_listener_call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    _index: i32,
) -> Result<webidl::ParseOutcome<WindowAddEventListenerCall<'s>>, webidl::WebIdlError> {
    let Some(event_type) = callback_arg_string(scope, args, 0) else {
        return Ok(webidl::ParseOutcome::Skip);
    };
    let options = webidl::event_listener_options(scope, args, 2, true);
    let listener_arg = args.get(1);
    let current_context = scope.get_current_context();
    let callback = if let Ok(function) = v8::Local::<v8::Function>::try_from(listener_arg) {
        let callback = v8::Local::<v8::Object>::from(function);
        let callback_relevant_context = callback
            .get_creation_context(scope)
            .unwrap_or(current_context);
        Some((callback, callback_relevant_context))
    } else if listener_arg.is_object() && !listener_arg.is_null_or_undefined() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(listener_arg) else {
            return Ok(webidl::ParseOutcome::Skip);
        };
        let callback_relevant_context = object
            .get_creation_context(scope)
            .unwrap_or(current_context);
        Some((object, callback_relevant_context))
    } else {
        None
    };
    let Some((callback, callback_relevant_context)) = callback else {
        return Ok(webidl::ParseOutcome::Skip);
    };
    let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
    Ok(webidl::ParseOutcome::Parsed(WindowAddEventListenerCall {
        event_type,
        callback,
        callback_relevant_context,
        incumbent_context,
        signal: signal_from_options_value(scope, args.get(2)),
        options,
    }))
}

fn window_remove_event_listener_call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    _index: i32,
) -> Result<webidl::ParseOutcome<WindowRemoveEventListenerCall<'s>>, webidl::WebIdlError> {
    let Some(event_type) = callback_arg_string(scope, args, 0) else {
        return Ok(webidl::ParseOutcome::Skip);
    };
    let options = webidl::event_listener_options(scope, args, 2, true);
    let listener_arg = args.get(1);
    let callback = if let Ok(function) = v8::Local::<v8::Function>::try_from(listener_arg) {
        Some(v8::Local::<v8::Object>::from(function))
    } else if listener_arg.is_object() && !listener_arg.is_null_or_undefined() {
        v8::Local::<v8::Object>::try_from(listener_arg).ok()
    } else {
        None
    };
    let Some(callback) = callback else {
        return Ok(webidl::ParseOutcome::Skip);
    };
    Ok(webidl::ParseOutcome::Parsed(
        WindowRemoveEventListenerCall {
            event_type,
            callback,
            options,
        },
    ))
}

fn signal_from_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return None;
    };
    options
        .get(scope, v8str(scope, "signal").into())
        .and_then(|value| {
            if value.is_null_or_undefined() {
                None
            } else {
                v8::Local::<v8::Object>::try_from(value).ok()
            }
        })
}

pub(super) fn event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if simple_event_target_slot_name(scope, args.this()).is_some() {
        simple_event_target_add_event_listener_callback(scope, args, rv);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let parsed = webidl::parse_args::<WindowAddEventListenerArgs>(scope, &args);
    let Some(parsed) = parsed else {
        return;
    };
    let webidl::ParseOutcome::Parsed(call) = parsed.call else {
        return;
    };
    let capture = call.options.capture;
    let once = call.options.once;
    let signal = call.signal;
    if let Some(signal) = signal
        && host.abort_signal_aborted(scope, signal)
    {
        return;
    }
    let child_window_target = child_window_handle(scope, args.this());
    let target = if let Some(handle) = child_window_target {
        host.current_child_window_event_target(handle)
            .map(EventTargetHandle::ChildWindow)
    } else {
        event_target_handle_from_this(scope, &args, host_ptr, host)
    };
    let Some(target) = target else {
        return;
    };
    let passive = call
        .options
        .passive
        .unwrap_or_else(|| default_passive_value(host, target, &call.event_type));
    let Some(callback_id) = host.register_target_event_listener(
        scope,
        target,
        &call.event_type,
        call.callback,
        call.callback_relevant_context,
        call.incumbent_context,
        capture,
        once,
        passive,
    ) else {
        return;
    };
    if call.event_type == "selectionchange"
        && target == EventTargetHandle::Node(host.document_handle())
    {
        define_non_enumerable_static_bool_property(
            scope,
            args.this(),
            DOCUMENT_SELECTION_CHANGE_LISTENER_SLOT,
            true,
        );
    }
    if let Some(signal) = signal {
        host.register_abort_target_listener(
            scope,
            signal,
            target,
            &call.event_type,
            callback_id,
            capture,
        );
    }
    if child_window_target.is_none() && call.event_type == "animationstart" {
        queue_animation_start_for_listener_target(scope, host_ptr, target);
    }
}

fn default_passive_value(
    host: &JsContextHost,
    target: EventTargetHandle,
    event_type: &str,
) -> bool {
    if !matches!(
        event_type,
        "touchstart" | "touchmove" | "wheel" | "mousewheel"
    ) {
        return false;
    }
    match target {
        EventTargetHandle::Window | EventTargetHandle::ChildWindow(_) => true,
        EventTargetHandle::Node(handle) => {
            let dom_host = host.dom_host();
            if dom_host
                .node(handle)
                .is_some_and(moli_dom::native::Node::is_document)
            {
                return true;
            }
            let Some(document) = dom_host.owner_document_handle(handle) else {
                return false;
            };
            dom_host
                .dom()
                .document_element_handle_for_document(document)
                == Some(handle)
                || dom_host.document_body_handle_for_document(document) == Some(handle)
        }
    }
}

pub(super) fn event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if simple_event_target_slot_name(scope, args.this()).is_some() {
        simple_event_target_remove_event_listener_callback(scope, args, rv);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(parsed) = webidl::parse_args::<WindowRemoveEventListenerArgs>(scope, &args) else {
        return;
    };
    let webidl::ParseOutcome::Parsed(call) = parsed.call else {
        return;
    };
    let capture = call.options.capture;
    let target = if let Some(handle) = child_window_handle(scope, args.this()) {
        host.current_child_window_event_target(handle)
            .map(EventTargetHandle::ChildWindow)
    } else {
        event_target_handle_from_this(scope, &args, host_ptr, host)
    };
    let Some(target) = target else {
        return;
    };
    host.remove_registered_event_listener(scope, target, &call.event_type, call.callback, capture);
}

pub(super) fn event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if simple_event_target_slot_name(scope, args.this()).is_some() {
        simple_event_target_dispatch_event_callback(scope, args, rv);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_bool(false);
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let child_window_target = child_window_handle(scope, args.this());
    let target = if child_window_target.is_none() {
        event_target_handle_from_this(scope, &args, host_ptr, host)
    } else {
        None
    };
    if child_window_target.is_none() && target.is_none() {
        rv.set_bool(false);
        return;
    };
    let event_value = args.get(0);
    if !event_value.is_object() || event_value.is_function() {
        throw_type_error(
            scope,
            "Failed to execute 'dispatchEvent': parameter 1 is not an object.",
        );
        return;
    }
    let Ok(event) = v8::Local::<v8::Object>::try_from(event_value) else {
        throw_type_error(
            scope,
            "Failed to execute 'dispatchEvent': parameter 1 is not an object.",
        );
        return;
    };

    let event = v8::Global::new(scope, event);
    let event = v8::Local::new(scope, &event);
    match event_initialized(scope, event) {
        Some(true) => {}
        Some(false) => {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "Failed to execute 'dispatchEvent': event is not initialized.",
            );
            return;
        }
        None => {
            throw_type_error(
                scope,
                "Failed to execute 'dispatchEvent': parameter 1 is not an Event.",
            );
            return;
        }
    }
    if event_is_dispatching(scope, event) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Failed to execute 'dispatchEvent': event is already being dispatched.",
        );
        return;
    }

    let event_type = event_type_string(scope, event);
    if let Some(handle) = child_window_target {
        let event_type = event_type.as_deref().unwrap_or_default();
        host.dispatch_child_window_event(scope, handle, event_type, event);
        rv.set_bool(!object_bool_property(scope, event, "defaultPrevented").unwrap_or(false));
        return;
    }
    let Some(target) = target else {
        rv.set_bool(false);
        return;
    };
    let click_activation_target = if event_type.as_deref() == Some("click")
        && event_is_mouse_event(scope, event)
        && let EventTargetHandle::Node(handle) = target
    {
        dispatched_click_activation_target(
            host,
            handle,
            object_bool_property(scope, event, "bubbles").unwrap_or(false),
            object_bool_property(scope, event, "composed").unwrap_or(false),
        )
    } else {
        None
    };
    let legacy_click_activation = click_activation_target
        .and_then(|handle| prepare_legacy_activation_for_dispatched_click(scope, host_ptr, handle));
    match host.dispatch_public_event(scope, host_ptr, target, event) {
        Ok(dispatch) => {
            let return_value = dispatch.dispatch_event_return_value();
            if let Some(event_type) = event_type.as_deref() {
                increment_performance_event_count(scope, event_type);
            }
            if let Some(activation) = legacy_click_activation {
                finish_legacy_activation_for_dispatched_click(
                    scope,
                    host_ptr,
                    activation,
                    return_value,
                );
            }
            if return_value && let Some(handle) = click_activation_target {
                perform_click_default_action_for_dispatched_event(scope, host_ptr, handle, event);
            }
            rv.set_bool(return_value);
        }
        Err(message) => throw_type_error(scope, &message),
    }
}

fn event_type_string(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> Option<String> {
    event
        .get(scope, v8str(scope, "type").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn window_set_timeout_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(0);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handler) = prepare_window_timer_handler(scope, host_ptr, args.get(0), "setTimeout")
    else {
        return;
    };
    let delay_ms = parse_timer_delay_ms(scope, &args, "Window.setTimeout");
    let runtime = unsafe { &mut *host_ptr };
    let timeout_id = match handler {
        WindowTimerHandler::Function(callback) => {
            let extra_args = collect_timer_extra_args(scope, &args);
            runtime.queue_window_timer_callback(
                scope,
                callback,
                args.this(),
                delay_ms,
                crate::host::HostTimerOwner::Window,
                extra_args,
            )
        }
        WindowTimerHandler::Source(source) => {
            let provenance = window_timer_provenance(scope, host_ptr, args.this());
            let context = scope.get_current_context();
            runtime.queue_source_timeout_with_receiver(
                scope,
                context,
                args.this(),
                source,
                provenance,
                delay_ms,
                crate::host::HostTimerOwner::Window,
                Vec::new(),
            )
        }
    };
    rv.set_uint32(timeout_id);
}

pub(super) fn window_set_interval_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(0);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handler) = prepare_window_timer_handler(scope, host_ptr, args.get(0), "setInterval")
    else {
        return;
    };
    let delay_ms = parse_timer_delay_ms(scope, &args, "Window.setInterval");
    let runtime = unsafe { &mut *host_ptr };
    let interval_id = match handler {
        WindowTimerHandler::Function(callback) => {
            let extra_args = collect_timer_extra_args(scope, &args);
            runtime.queue_window_timer_callback_interval(
                scope,
                callback,
                args.this(),
                delay_ms,
                crate::host::HostTimerOwner::Window,
                extra_args,
            )
        }
        WindowTimerHandler::Source(source) => {
            let provenance = window_timer_provenance(scope, host_ptr, args.this());
            let context = scope.get_current_context();
            runtime.queue_source_interval_with_receiver(
                scope,
                context,
                args.this(),
                source,
                provenance,
                delay_ms,
                crate::host::HostTimerOwner::Window,
                Vec::new(),
            )
        }
    };
    rv.set_uint32(interval_id);
}

enum WindowTimerHandler {
    Function(webidl::WebIdlCallbackFunction),
    Source(String),
}

fn prepare_window_timer_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
    timer_name: &'static str,
) -> Option<WindowTimerHandler> {
    if let Ok(callback) = v8::Local::<v8::Object>::try_from(value)
        && callback.is_callable()
    {
        let current_context = scope.get_current_context();
        let relevant_context = callback
            .get_creation_context(scope)
            .unwrap_or(current_context);
        let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
        let callback = webidl::WebIdlCallbackFunction::try_new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        )
        .expect("a callable timer handler must convert to a callback function");
        return Some(WindowTimerHandler::Function(callback));
    }
    let requirements = unsafe { &*host_ptr }.trusted_types_for_script_requirements(scope);
    let sink = match timer_name {
        "setInterval" => "Window setInterval",
        _ => "Window setTimeout",
    };
    let source = trusted_script_string_or_type_error(scope, value, requirements, sink, timer_name)?;
    let host = unsafe { &mut *host_ptr };
    let allow_trusted_types_eval =
        requirements.is_enforced() && host.allows_trusted_types_eval(scope);
    host.allows_eval_code_generation_by_csp(scope, allow_trusted_types_eval)
        .then_some(WindowTimerHandler::Source(source))
}

fn window_timer_provenance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
) -> CompiledStringProvenance {
    let source_url = window_timer_source_url(scope, host_ptr, receiver);
    let module_base_url = window_timer_script_base_url(scope).unwrap_or_else(|| source_url.clone());
    CompiledStringProvenance::new(source_url, module_base_url)
}

fn window_timer_source_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
) -> Url {
    let host = unsafe { &*host_ptr };
    if let Some(endpoint) = window_message_endpoint_from_receiver(scope, receiver) {
        let owner_url = match endpoint {
            PendingWindowMessageEndpoint::TopWindow => Some(host.document_url().clone()),
            PendingWindowMessageEndpoint::ChildWindow(handle) => {
                host.child_browsing_context_current_url(handle)
            }
            PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
                host.lightweight_popup_document_url(popup_id)
            }
        };
        if let Some(owner_url) = owner_url {
            return owner_url;
        }
    }
    scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "location").into())
        .and_then(|location| v8::Local::<v8::Object>::try_from(location).ok())
        .and_then(|location| location.get(scope, v8str(scope, "href").into()))
        .and_then(|href| href.to_string(scope))
        .map(|href| href.to_rust_string_lossy(scope))
        .filter(|href| !href.is_empty())
        .and_then(|href| Url::parse(&href).ok())
        .unwrap_or_else(|| host.document_url().clone())
}

fn window_timer_script_base_url(scope: &mut v8::PinScope<'_, '_>) -> Option<Url> {
    if let Some(options) = scope.get_current_host_defined_options()
        && let Some(base_url) = script_base_url_from_host_defined_options(scope, options)
    {
        return Some(base_url);
    }
    script_base_url_from_continuation_data(scope)
}

pub(super) fn window_clear_timer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let id_val = args.get(0);
    let id = id_val.number_value(scope).unwrap_or(0.0) as u32;
    if id == 0 {
        return;
    }
    let runtime = unsafe { &mut *host_ptr };
    let _ = runtime.cancel_window_timer_for_receiver(scope, args.this(), id);
}

pub(super) fn window_request_animation_frame_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(0);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WindowRequestAnimationFrameArgs>(scope, &args) else {
        return;
    };

    let global = scope.get_current_context().global(scope);
    let now = current_animation_frame_time_ms(scope);
    let target_timestamp =
        get_private_value(scope, global, WINDOW_PENDING_ANIMATION_FRAME_TIMESTAMP_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|timestamp| *timestamp > now)
            .unwrap_or_else(|| {
                let timestamp = now + WINDOW_REQUEST_ANIMATION_FRAME_DELAY_MS as f64;
                set_private_value(
                    scope,
                    global,
                    WINDOW_PENDING_ANIMATION_FRAME_TIMESTAMP_SLOT,
                    v8::Number::new(scope, timestamp).into(),
                );
                timestamp
            });
    let delay_ms = (target_timestamp - now).ceil().clamp(0.0, u32::MAX as f64) as u32;
    let runtime = unsafe { &mut *host_ptr };
    let timeout_id = runtime.queue_window_animation_frame_callback(
        scope,
        parsed.callback,
        args.this(),
        target_timestamp,
        delay_ms,
        crate::host::HostTimerOwner::Window,
    );
    rv.set_uint32(timeout_id);
}

pub(super) fn window_request_idle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(0);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WindowRequestIdleCallbackArgs>(scope, &args) else {
        return;
    };

    let timeout_deadline_ms = parse_idle_callback_timeout_ms(scope, parsed.options)
        .map(|timeout_ms| current_time_ms() + timeout_ms as f64)
        .unwrap_or(-1.0);
    let runtime = unsafe { &mut *host_ptr };
    let timeout_id = runtime.queue_window_idle_callback(
        scope,
        parsed.callback,
        args.this(),
        timeout_deadline_ms,
        IDLE_OPPORTUNITY_DELAY_MS,
        crate::host::HostTimerOwner::Window,
    );
    rv.set_uint32(timeout_id);
}

pub(crate) fn window_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(target_endpoint) = window_message_endpoint_from_receiver(scope, args.this()) else {
        crate::native_bridge::throw_cross_origin_type_error(
            scope,
            "Failed to execute 'postMessage' on 'Window': Illegal invocation.",
        );
        return;
    };
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'Window': 1 argument required, but only 0 present.",
        );
        return;
    }
    // Blink captures the incumbent DOMWindow at API acceptance. Lightweight
    // popups still share the top-level V8 context, so their active execution
    // scope is the one necessary override; the ambient source marker is only
    // a fallback for legacy execution paths that do not expose an incumbent
    // context.
    let source_identity = active_lightweight_popup_id(scope)
        .map(PendingWindowMessageEndpoint::LightweightPopup)
        .and_then(|endpoint| current_window_message_source_identity(scope, host, endpoint))
        .or_else(|| incumbent_window_message_source_identity(scope, host))
        .or_else(|| {
            host.current_window_message_source()
                .and_then(|endpoint| current_window_message_source_identity(scope, host, endpoint))
        })
        .or_else(|| {
            let endpoint = active_child_window_handle(scope)
                .or_else(|| child_window_handle(scope, scope.get_current_context().global(scope)))
                .map(PendingWindowMessageEndpoint::ChildWindow)
                .unwrap_or(PendingWindowMessageEndpoint::TopWindow);
            current_window_message_source_identity(scope, host, endpoint)
        });
    let Some((source_endpoint, source_owner, source_realm_token)) = source_identity else {
        rv.set_undefined();
        return;
    };
    let source = PendingWindowMessageSource::new(source_endpoint, source_owner, source_realm_token);

    let target_dispatch_scope = target_endpoint.dispatch_scope();
    let Some(target_owner) = host.current_window_execution_context_owner(target_dispatch_scope)
    else {
        rv.set_undefined();
        return;
    };
    let target = WindowTaskTarget::new(target_dispatch_scope, target_owner);
    if let PendingWindowMessageEndpoint::LightweightPopup(popup_id) = target_endpoint
        && !host.ensure_lightweight_popup_execution_context(scope, popup_id)
    {
        rv.set_undefined();
        return;
    }
    let Some(source_origin) = window_message_endpoint_origin(host, source_endpoint) else {
        rv.set_undefined();
        return;
    };
    let source_security =
        crate::context_bootstrap::RuntimeMessageSourceSecurity::window(source_origin.clone());

    let target_origin_value = args.get(1);
    let options = (target_origin_value.is_object() && !target_origin_value.is_null_or_undefined())
        .then(|| v8::Local::<v8::Object>::try_from(target_origin_value).ok())
        .flatten();
    let normalized_target_origin = if let Some(options) = options {
        options
            .get(scope, v8str(scope, "targetOrigin").into())
            .and_then(|value| {
                if value.is_undefined() {
                    None
                } else {
                    value
                        .to_string(scope)
                        .map(|s| s.to_rust_string_lossy(scope))
                }
            })
            .unwrap_or_else(|| "/".to_owned())
    } else if target_origin_value.is_undefined() {
        "/".to_owned()
    } else {
        target_origin_value
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "*".to_owned())
    };

    let Some(target_origin) = window_message_endpoint_origin(host, target_endpoint) else {
        rv.set_undefined();
        return;
    };
    let Some(target_origin_match) = normalized_window_post_message_target_origin(
        scope,
        &normalized_target_origin,
        &source_origin,
    ) else {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                ?source,
                ?target,
                source_origin = %source_origin,
                target_origin = %target_origin,
                requested_target_origin = %normalized_target_origin,
                stage = "post_message_invalid_target_origin",
            );
        }
        return;
    };
    let data = if options.is_some() {
        crate::context_bootstrap::structured_serialize_value_for_window_post_message_options(
            scope,
            args.get(0),
            target_origin_value,
            source_security,
        )
    } else {
        crate::context_bootstrap::structured_serialize_value_for_window_post_message(
            scope,
            args.get(0),
            (args.length() > 2).then(|| args.get(2)),
            source_security,
        )
    };
    let Some(data) = data else {
        return;
    };

    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            ?source,
            ?target,
            source_owner = ?source.owner(),
            source_realm = ?source.realm_token(),
            target_owner = ?target.owner(),
            source_origin = %source_origin,
            target_origin = %target_origin,
            requested_target_origin = %normalized_target_origin,
            stage = "post_message_queued",
        );
    }
    let task_id = host.queue_window_message(PendingWindowMessage {
        target,
        source,
        data,
        origin: source_origin,
        intended_target_origin: target_origin_match,
    });
    let sender = host.page_window_message_sender().clone();
    if sender.send(target, task_id).is_err() {
        let discarded = host.discard_pending_window_message_task(task_id);
        assert!(
            discarded,
            "closed Window.postMessage route lost its local payload"
        );
    }
    rv.set_undefined();
}

pub(crate) fn signal_pending_window_message_reconsideration(host: &JsContextHost) {
    let has_pending = host.has_pending_window_messages();
    if has_pending {
        host.signal_pending_window_message_reconsideration();
    }
    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            has_pending,
            stage = "post_message_task_reconsideration",
        );
    }
}

pub(crate) fn normalized_window_post_message_target_origin(
    scope: &mut v8::PinScope<'_, '_>,
    target_origin: &str,
    source_origin: &str,
) -> Option<Option<String>> {
    if target_origin == "*" {
        return Some(None);
    }
    if target_origin == "/" {
        return Some(Some(source_origin.to_owned()));
    }
    let Ok(url) = Url::parse(target_origin) else {
        throw_dom_exception(
            scope,
            "SyntaxError",
            12,
            "Failed to execute 'postMessage' on 'Window': Invalid target origin.",
        );
        return None;
    };
    Some(Some(moli_url::origin_ascii_serialization(&url)))
}

pub(crate) fn target_origin_matches(
    normalized_target_origin: Option<&str>,
    target_origin: &str,
) -> bool {
    normalized_target_origin
        .map(|origin| origin == target_origin)
        .unwrap_or(true)
}

pub(super) fn window_scroll_to_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let current_x = scroll_x(scope, global);
    let current_y = scroll_y(scope, global);
    let (x, y) = parse_scroll_coordinates(scope, &args, current_x, current_y);
    scroll_window_to(scope, host_ptr, x, y);
    rv.set_undefined();
}

pub(super) fn window_scroll_by_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let current_x = scroll_x(scope, global);
    let current_y = scroll_y(scope, global);
    let (dx, dy) = parse_scroll_coordinates(scope, &args, 0.0, 0.0);
    scroll_window_to(scope, host_ptr, current_x + dx, current_y + dy);
    rv.set_undefined();
}

pub(crate) fn scroll_window_to(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    x: f64,
    y: f64,
) {
    let x = x.max(0.0);
    let y = y.max(0.0);
    let (document, scrolling_element) = {
        let runtime = unsafe { &*host_ptr };
        let document = runtime
            .current_window_document_task_target(scope)
            .and_then(|target| match target.dispatch_scope() {
                crate::native_bridge::OwnerDispatchScope::Top => Some(runtime.document_handle()),
                crate::native_bridge::OwnerDispatchScope::Child(handle) => {
                    runtime.child_browsing_context_document_handle(handle)
                }
                crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
                    runtime.lightweight_popup_document_handle(popup_id)
                }
            });
        let scrolling_element = document.and_then(|document| {
            runtime
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
        });
        (document, scrolling_element)
    };
    let scrolling_element_changed = scrolling_element.is_some_and(|handle| {
        unsafe { &mut *host_ptr }
            .dom_host_mut()
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| element.set_scroll_left(x) | element.set_scroll_top(y))
    });
    let global = scope.get_current_context().global(scope);
    if !set_scroll_position(scope, global, x, y) && !scrolling_element_changed {
        return;
    }
    queue_scroll_observable_effects(scope, host_ptr, document, true);
}

pub(crate) fn current_window_scroll_position(scope: &mut v8::PinScope<'_, '_>) -> (f64, f64) {
    let global = scope.get_current_context().global(scope);
    (scroll_x(scope, global), scroll_y(scope, global))
}

pub(crate) fn window_get_computed_style_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'getComputedStyle' on 'Window': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_window_object(scope, args.this()) else {
        throw_type_error(
            scope,
            "Failed to execute 'getComputedStyle' on 'Window': Illegal invocation.",
        );
        return;
    };
    let Some(handle) = current_or_live_delegate_node_arg_handle(scope, host_ptr, args.get(0))
    else {
        rv.set_null();
        return;
    };
    let pseudo_argument = computed_style_pseudo_argument_from_function_args(scope, &args);
    let child_window_handle = object_child_window_handle(scope, args.this())
        .or_else(|| dom_handle_from_marker_value(scope, args.data()));
    match build_computed_style_object(
        scope,
        host_ptr,
        handle,
        child_window_handle,
        pseudo_argument,
    ) {
        Some(style) => rv.set(style.into()),
        None => rv.set_null(),
    }
}

pub(crate) fn build_computed_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    child_window_handle: Option<DomHandle>,
    pseudo_argument: ComputedStylePseudoArgument,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(child_window_handle) = child_window_handle {
        let _ = unsafe { &mut *host_ptr }
            .child_browsing_context_document_wrapper(scope, child_window_handle);
    }
    unsafe { &*host_ptr }.drain_pending_style_invalidations_for_computed_style_read();
    let host_ref = unsafe { &*host_ptr };
    let target_context =
        computed_style_target_context(scope, host_ref, handle, child_window_handle);
    let cached_target_empty = match target_context {
        ComputedStyleTargetContext::ChildFrameDocument { .. } => {
            Some(target_context.returns_empty_style())
        }
        ComputedStyleTargetContext::ActiveDocument
        | ComputedStyleTargetContext::EmptyForDetached => None,
    };
    let viewport = child_window_handle
        .and_then(|child_handle| iframe_handle_viewport(host_ref, child_handle))
        .unwrap_or_else(|| target_context.viewport(host_ref));
    let ComputedStylePseudoArgument {
        forced_empty,
        pseudo_element,
        pseudo_key,
    } = pseudo_argument;
    let descriptor =
        ComputedStyleDescriptor::new(pseudo_key, computed_style_target_key(target_context));
    let host = unsafe { &mut *host_ptr };
    if let Some(shadow_root) = host.dom_host().containing_shadow_root(handle)
        && let Some(root) = host
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, shadow_root)
    {
        super::native_bridge::element::ensure_shadow_root_adopted_style_sheets_initialized(
            scope,
            host,
            shadow_root,
            root,
        );
    }
    let style = host
        .native_bridge_mut()
        .wrap_computed_style(scope, host_ptr, handle, descriptor)?;
    let empty = v8::Boolean::new(scope, forced_empty);
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT,
        empty.into(),
    );
    let target_empty = cached_target_empty
        .map(|empty| v8::Boolean::new(scope, empty).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT,
        target_empty,
    );
    let target_context_epoch = cached_target_empty
        .map(|_| v8::BigInt::new_from_u64(scope, host.style_target_context_epoch()).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
        target_context_epoch,
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_READ_DOCUMENT_SLOT,
        v8::undefined(scope).into(),
    );
    let pseudo_value = pseudo_element
        .as_deref()
        .and_then(|pseudo_element| v8_string(scope, pseudo_element).map(Into::into))
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
        pseudo_value,
    );
    let width = viewport
        .width
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, width);
    let height = viewport
        .height
        .map(|height| v8::Number::new(scope, height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT, height);
    let screen_width = viewport
        .screen_width
        .map(|screen_width| v8::Number::new(scope, screen_width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
        screen_width,
    );
    let screen_height = viewport
        .screen_height
        .map(|screen_height| v8::Number::new(scope, screen_height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
        screen_height,
    );
    Some(style)
}

fn computed_style_target_key(target_context: ComputedStyleTargetContext) -> ComputedStyleTargetKey {
    match target_context {
        ComputedStyleTargetContext::ChildFrameDocument { frame_handle, .. } => {
            ComputedStyleTargetKey::ChildFrame(frame_handle)
        }
        ComputedStyleTargetContext::ActiveDocument
        | ComputedStyleTargetContext::EmptyForDetached => ComputedStyleTargetKey::Dynamic,
    }
}

pub(crate) struct ComputedStylePseudoArgument {
    pub(crate) forced_empty: bool,
    pub(crate) pseudo_element: Option<String>,
    pub(crate) pseudo_key: ComputedStylePseudoKey,
}

impl ComputedStylePseudoArgument {
    pub(crate) fn originating_element() -> Self {
        Self {
            forced_empty: false,
            pseudo_element: None,
            pseudo_key: ComputedStylePseudoKey::Originating,
        }
    }
}

pub(crate) fn computed_style_pseudo_argument_from_function_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> ComputedStylePseudoArgument {
    computed_style_pseudo_argument(get_computed_style_pseudo_element_argument(scope, args))
}

fn computed_style_pseudo_argument(
    pseudo_element: moli_selector::GetComputedStylePseudoElement,
) -> ComputedStylePseudoArgument {
    match pseudo_element {
        moli_selector::GetComputedStylePseudoElement::OriginatingElement => {
            ComputedStylePseudoArgument {
                forced_empty: false,
                pseudo_element: None,
                pseudo_key: ComputedStylePseudoKey::Originating,
            }
        }
        moli_selector::GetComputedStylePseudoElement::EmptyStyle => ComputedStylePseudoArgument {
            forced_empty: true,
            pseudo_element: None,
            pseudo_key: ComputedStylePseudoKey::ForcedEmpty,
        },
        moli_selector::GetComputedStylePseudoElement::PseudoElement(pseudo_element)
            if computed_style_pseudo_element_supported_by_stylo(&pseudo_element) =>
        {
            ComputedStylePseudoArgument {
                forced_empty: false,
                pseudo_key: ComputedStylePseudoKey::from_stylo_pseudo(&pseudo_element)
                    .unwrap_or(ComputedStylePseudoKey::ForcedEmpty),
                pseudo_element: Some(pseudo_element),
            }
        }
        moli_selector::GetComputedStylePseudoElement::PseudoElement(_) => {
            ComputedStylePseudoArgument {
                forced_empty: true,
                pseudo_element: None,
                pseudo_key: ComputedStylePseudoKey::ForcedEmpty,
            }
        }
    }
}

fn computed_style_pseudo_element_supported_by_stylo(pseudo_element: &str) -> bool {
    ComputedStylePseudoKey::from_stylo_pseudo(pseudo_element).is_some()
}

fn get_computed_style_pseudo_element_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> moli_selector::GetComputedStylePseudoElement {
    if args.length() < 2 || args.get(1).is_undefined() {
        return moli_selector::GetComputedStylePseudoElement::OriginatingElement;
    }
    let Some(value) = args.get(1).to_string(scope) else {
        return moli_selector::GetComputedStylePseudoElement::EmptyStyle;
    };
    moli_selector::get_computed_style_pseudo_element(&value.to_rust_string_lossy(scope))
}

fn mouse_event_offset_getter(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    horizontal: bool,
) -> Option<f64> {
    let client_x = object_number_property(scope, event, "clientX").unwrap_or(0.0);
    let client_y = object_number_property(scope, event, "clientY").unwrap_or(0.0);
    let coordinate = if horizontal { client_x } else { client_y };
    let Some(target) = event
        .get(scope, v8str(scope, "target").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Some(coordinate);
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        return Some(coordinate);
    };
    let offset = match observable_event_offset(
        unsafe { &*runtime_ptr },
        handle,
        moli_layout::LayoutPoint::new(client_x as f32, client_y as f32),
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(offset) => offset,
        Err(error) => {
            let message = v8_string(
                scope,
                &format!("Layout failed while resolving MouseEvent offset: {error}"),
            )?;
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
            return None;
        }
    };
    Some(f64::from(if horizontal { offset.x } else { offset.y }).round())
}

pub(crate) fn mouse_event_offset_x_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_is_mouse_event(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(value) = mouse_event_offset_getter(scope, args.this(), true) {
        rv.set(v8::Number::new(scope, value).into());
    }
}

pub(crate) fn mouse_event_offset_y_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_is_mouse_event(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(value) = mouse_event_offset_getter(scope, args.this(), false) {
        rv.set(v8::Number::new(scope, value).into());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowMessageTaskRunResult {
    Idle,
    Blocked,
    Completed,
}

pub(crate) fn run_current_window_message_task(
    scope: &mut v8::PinScope<'_, '_>,
    task_id: crate::page_task_queue::RendererPageWindowMessageTaskId,
    expected_target: WindowTaskTarget,
) -> WindowMessageTaskRunResult {
    let Some(host) = context_host_from_global_bridge(scope) else {
        return WindowMessageTaskRunResult::Idle;
    };
    if !host.has_pending_window_message_task(task_id) {
        return WindowMessageTaskRunResult::Idle;
    }
    if !host.window_message_target_is_materialized(expected_target) {
        return WindowMessageTaskRunResult::Blocked;
    }
    let message = host
        .take_pending_window_message_task(task_id)
        .expect("materialized Window.postMessage task must retain its local payload");
    assert_eq!(
        message.target, expected_target,
        "stable Window.postMessage task target diverged from its local payload"
    );
    dispatch_current_window_message(scope, host, message)
}

fn dispatch_current_window_message(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    message: PendingWindowMessage,
) -> WindowMessageTaskRunResult {
    let host_ptr = host as *mut JsContextHost;
    let dispatch_scope = message.target.dispatch_scope();
    let (realm_token, target_context) = host
        .window_execution_context(scope, message.target.owner(), dispatch_scope)
        .expect("authorized Window.postMessage target must retain a materialized context");
    let scope = &mut v8::ContextScope::new(scope, target_context);
    assert_eq!(
        crate::native_bridge::current_runtime_observable_context_token(scope),
        Some(realm_token),
        "materialized Window.postMessage binding diverged from its V8 context token"
    );
    let previous_dispatch_scope = dispatch_scope.enter(scope);
    let previous_message_source = host.enter_window_message_source_scope(
        PendingWindowMessageEndpoint::from_dispatch_scope(message.target.dispatch_scope()),
    );
    let outcome = dispatch_window_message_in_current_target_context(scope, host_ptr, host, message);
    host.restore_window_message_source_scope(previous_message_source);
    if outcome == WindowMessageDispatchOutcome::Dispatched {
        // Promise reactions created by a message listener belong to the
        // target Window. Keep that owner scope active through the task's
        // microtask checkpoint, then restore the interrupted scope.
        dispatch_scope.defer_restore(scope, previous_dispatch_scope);
    } else {
        dispatch_scope.restore(scope, previous_dispatch_scope);
    }
    WindowMessageTaskRunResult::Completed
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowMessageDispatchOutcome {
    Consumed,
    Dispatched,
}

fn dispatch_window_message_in_current_target_context(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    host: &mut JsContextHost,
    message: PendingWindowMessage,
) -> WindowMessageDispatchOutcome {
    let target_endpoint =
        PendingWindowMessageEndpoint::from_dispatch_scope(message.target.dispatch_scope());
    let source_endpoint = message.source.endpoint();
    let target_origin = window_message_endpoint_origin(host, target_endpoint);
    if !target_origin.as_deref().is_some_and(|target_origin| {
        target_origin_matches(message.intended_target_origin.as_deref(), target_origin)
    }) {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                target = ?message.target,
                source = ?message.source,
                source_origin = %message.origin,
                target_origin = ?target_origin,
                intended_target_origin = ?message.intended_target_origin,
                stage = "post_message_target_origin_rejected_at_dispatch",
            );
        }
        host.retire_transferred_window_message_ports(&message);
        return WindowMessageDispatchOutcome::Consumed;
    }

    let global = scope.get_current_context().global(scope);
    let Some(message_ctor) = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        host.retire_transferred_window_message_ports(&message);
        return WindowMessageDispatchOutcome::Consumed;
    };
    if window_message_requires_messageerror(&message, target_origin.as_deref()) {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                target = ?message.target,
                source = ?message.source,
                origin = %message.origin,
                target_origin = ?target_origin,
                event = "messageerror",
                stage = "post_message_dispatch",
            );
        }
        dispatch_window_message_event(
            scope,
            host_ptr,
            host,
            message_ctor,
            target_endpoint,
            source_endpoint,
            "messageerror",
            v8::null(scope).into(),
            &message.origin,
            v8::Array::new(scope, 0),
        );
        host.retire_transferred_window_message_ports(&message);
        return WindowMessageDispatchOutcome::Dispatched;
    }
    if let Some((handle, violation)) = window_message_wasm_eval_csp_violation(host, &message) {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                target = ?message.target,
                source = ?message.source,
                origin = %message.origin,
                child_handle = ?handle,
                blocked_uri = violation.blocked_uri.as_str(),
                stage = "post_message_wasm_eval_csp_blocked",
            );
        }
        host.dispatch_child_content_security_policy_violation_event_best_effort(
            scope, handle, &violation,
        );
        host.retire_transferred_window_message_ports(&message);
        return WindowMessageDispatchOutcome::Consumed;
    }
    let deserialized = crate::context_bootstrap::structured_deserialize_value_for_message_event(
        scope,
        &message.data,
    );
    let Some((data, ports)) = deserialized else {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                target = ?message.target,
                source = ?message.source,
                origin = %message.origin,
                event = "messageerror",
                stage = "post_message_deserialize_failed",
            );
        }
        dispatch_window_message_event(
            scope,
            host_ptr,
            host,
            message_ctor,
            target_endpoint,
            source_endpoint,
            "messageerror",
            v8::null(scope).into(),
            &message.origin,
            v8::Array::new(scope, 0),
        );
        host.retire_transferred_window_message_ports(&message);
        return WindowMessageDispatchOutcome::Dispatched;
    };
    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            target = ?message.target,
            source = ?message.source,
            origin = %message.origin,
            target_origin = ?target_origin,
            event = "message",
            stage = "post_message_dispatch",
        );
    }
    dispatch_window_message_event(
        scope,
        host_ptr,
        host,
        message_ctor,
        target_endpoint,
        source_endpoint,
        "message",
        data,
        &message.origin,
        ports,
    );
    WindowMessageDispatchOutcome::Dispatched
}

fn window_message_endpoint_origin(
    host: &JsContextHost,
    endpoint: PendingWindowMessageEndpoint,
) -> Option<String> {
    match endpoint {
        PendingWindowMessageEndpoint::TopWindow => {
            Some(moli_url::origin_ascii_serialization(host.document_url()))
        }
        PendingWindowMessageEndpoint::ChildWindow(handle) => {
            host.child_browsing_context_target_origin(handle)
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
            host.lightweight_popup_origin(popup_id)
        }
    }
}

fn window_message_requires_messageerror(
    message: &PendingWindowMessage,
    target_origin: Option<&str>,
) -> bool {
    let Some(target_origin) = target_origin else {
        return false;
    };
    !crate::context_bootstrap::wasm_module_message_allowed_for_target_origin(
        &message.data,
        Some(target_origin),
    )
}

fn window_message_wasm_eval_csp_violation(
    host: &JsContextHost,
    message: &PendingWindowMessage,
) -> Option<(DomHandle, DocumentContentSecurityPolicyViolation)> {
    if !message.data.metadata.contains_wasm_module {
        return None;
    }
    let PendingWindowMessageEndpoint::ChildWindow(handle) =
        PendingWindowMessageEndpoint::from_dispatch_scope(message.target.dispatch_scope())
    else {
        return None;
    };
    host.child_wasm_eval_csp_violation(handle)
        .map(|violation| (handle, violation))
}

fn dispatch_window_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    host: &mut JsContextHost,
    message_ctor: v8::Local<'s, v8::Function>,
    target: PendingWindowMessageEndpoint,
    source_endpoint: PendingWindowMessageEndpoint,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
    ports: v8::Local<'s, v8::Array>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(origin) = v8_string(scope, origin) else {
        return;
    };
    let source: v8::Local<'_, v8::Value> = match source_endpoint {
        PendingWindowMessageEndpoint::TopWindow => {
            top_window_message_source_for_target(scope, host, target)
                .unwrap_or_else(|| global.into())
        }
        PendingWindowMessageEndpoint::ChildWindow(handle) => {
            child_window_message_source(scope, host, handle)
                .map(Into::into)
                .unwrap_or_else(|| v8::null(scope).into())
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => host
            .lightweight_popup_window(scope, popup_id)
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    };
    let init = WindowMessageEventInitDeclaration::new(data, origin, ports, source)
        .bind(scope)
        .expect("Window MessageEvent init declaration should bind");
    let event_name = event_type;
    let Some(event_type) = v8_string(scope, event_name) else {
        return;
    };
    let Some(event) = message_ctor.new_instance(scope, &[event_type.into(), init.into()]) else {
        return;
    };
    mark_event_trusted(scope, event);

    match target {
        PendingWindowMessageEndpoint::TopWindow => {
            let _previous_active_child = enter_active_child_window_scope(scope, None);
            let _previous_active_popup = enter_top_level_lightweight_popup_scope(scope);
            let _ = host.dispatch_public_event_best_effort(
                scope,
                host_ptr,
                EventTargetHandle::Window,
                event,
                "window message event",
            );
        }
        PendingWindowMessageEndpoint::ChildWindow(handle) => {
            host.dispatch_child_window_event(scope, handle, event_name, event);
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
            host.dispatch_lightweight_popup_window_event(scope, popup_id, event_name, event);
        }
    }
}

fn top_window_message_source_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    target: PendingWindowMessageEndpoint,
) -> Option<v8::Local<'s, v8::Value>> {
    let PendingWindowMessageEndpoint::ChildWindow(handle) = target else {
        return None;
    };
    let top = host.child_browsing_context_top_window_for_current_realm(scope, handle);
    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            handle = handle.index(),
            relation_found = top.is_some(),
            top_is_target_global = top.is_some_and(|top| {
                top.strict_equals(scope.get_current_context().global(scope).into())
            }),
            stage = "top_window_message_source_resolved",
        );
    }
    top.map(Into::into)
}

fn child_window_message_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    host.child_browsing_context_window_proxy_for_top(scope, handle)
}

fn collect_timer_extra_args(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Vec<v8::Global<v8::Value>> {
    (2..args.length())
        .map(|i| v8::Global::new(scope, args.get(i)))
        .collect()
}

fn parse_scroll_coordinates(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    fallback_x: f64,
    fallback_y: f64,
) -> (f64, f64) {
    if args.length() > 0
        && args.get(0).is_object()
        && !args.get(0).is_function()
        && let Some(options) = args.get(0).to_object(scope)
    {
        let x = options
            .get(scope, v8str(scope, "left").into())
            .or_else(|| options.get(scope, v8str(scope, "x").into()))
            .and_then(|value| value.number_value(scope))
            .unwrap_or(fallback_x);
        let y = options
            .get(scope, v8str(scope, "top").into())
            .or_else(|| options.get(scope, v8str(scope, "y").into()))
            .and_then(|value| value.number_value(scope))
            .unwrap_or(fallback_y);
        return (x, y);
    }

    let x = args.get(0).number_value(scope).unwrap_or(fallback_x);
    let y = args.get(1).number_value(scope).unwrap_or(fallback_y);
    (x, y)
}

fn scroll_x<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<'s, v8::Object>) -> f64 {
    get_private_value(scope, global, WINDOW_SCROLL_X_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0)
}

fn scroll_y<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<'s, v8::Object>) -> f64 {
    get_private_value(scope, global, WINDOW_SCROLL_Y_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0)
}

fn set_scroll_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
) -> bool {
    let x = x.max(0.0);
    let y = y.max(0.0);
    if scroll_x(scope, global) == x && scroll_y(scope, global) == y {
        return false;
    }
    let scroll_x = v8::Number::new(scope, x);
    set_private_value(scope, global, WINDOW_SCROLL_X_SLOT, scroll_x.into());
    let scroll_y = v8::Number::new(scope, y);
    set_private_value(scope, global, WINDOW_SCROLL_Y_SLOT, scroll_y.into());
    true
}

pub(crate) fn dispatch_document_event_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    event_type: &str,
) -> bool {
    let runtime = unsafe { &mut *host_ptr };
    let global = scope.get_current_context().global(scope);
    let Some(event_ctor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let Some(event_type_str) = v8_string(scope, event_type) else {
        return false;
    };
    let (bubbles, cancelable) = super::host::host_event_defaults(event_type);
    let init = WindowDocumentEventInitDeclaration::new(bubbles, cancelable)
        .bind(scope)
        .expect("window document event init declaration should bind");
    let Some(event) = event_ctor.new_instance(scope, &[event_type_str.into(), init.into()]) else {
        return false;
    };
    // A listener exception is reported by the public-dispatch boundary, but
    // does not mean that the Document or event target disappeared. The
    // rendering update must still continue its pending event list and finish
    // the host-task checkpoint.
    let _ = runtime.dispatch_public_event_best_effort(
        scope,
        host_ptr,
        EventTargetHandle::Node(document_handle),
        event,
        "rendering-update document event",
    );
    true
}

fn parse_timer_delay_ms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> u32 {
    webidl::timer_milliseconds_arg(scope, args, 1, prefix)
}

fn parse_idle_callback_timeout_ms(
    scope: &mut v8::PinScope<'_, '_>,
    options: Option<v8::Local<'_, v8::Value>>,
) -> Option<u32> {
    let Ok(options) = v8::Local::<v8::Object>::try_from(options?) else {
        return None;
    };
    let timeout = options.get(scope, v8str(scope, "timeout").into())?;
    if timeout.is_null_or_undefined() {
        return None;
    }
    let raw = timeout.number_value(scope).unwrap_or(0.0);
    if !raw.is_finite() || raw <= 0.0 {
        return Some(0);
    }
    Some(raw.min(u32::MAX as f64) as u32)
}

pub(crate) fn finish_animation_frame_callback_batch(
    scope: &mut v8::PinScope<'_, '_>,
    timestamp: f64,
) {
    let global = scope.get_current_context().global(scope);
    let pending_timestamp =
        get_private_value(scope, global, WINDOW_PENDING_ANIMATION_FRAME_TIMESTAMP_SLOT)
            .and_then(|value| value.number_value(scope));
    if pending_timestamp == Some(timestamp) {
        set_private_value(
            scope,
            global,
            WINDOW_PENDING_ANIMATION_FRAME_TIMESTAMP_SLOT,
            v8::undefined(scope).into(),
        );
    }
}

pub(crate) fn build_window_idle_deadline<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    did_timeout: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let now_ms = current_time_ms();
    let deadline_ms = if did_timeout {
        now_ms
    } else {
        now_ms + IDLE_CALLBACK_BUDGET_MS
    };
    let deadline_state = IdleDeadlineStateDeclaration::new(deadline_ms)
        .bind(scope)
        .ok()?;
    IdleDeadlineDeclaration::new(did_timeout, deadline_state)
        .bind(scope)
        .ok()
}

fn window_idle_deadline_time_remaining_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let now_ms = current_time_ms();
    let remaining_ms = if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) {
        let mut deadline_ms = get_private_value(scope, state, IDLE_DEADLINE_MS_SLOT)
            .and_then(|value| value.number_value(scope))
            .unwrap_or(0.0);
        if deadline_ms <= 0.0 {
            deadline_ms = now_ms + IDLE_CALLBACK_BUDGET_MS;
            let value = v8::Number::new(scope, deadline_ms);
            set_private_value(scope, state, IDLE_DEADLINE_MS_SLOT, value.into());
        }
        (deadline_ms - now_ms).max(0.0)
    } else {
        0.0
    };
    rv.set(v8::Number::new(scope, remaining_ms).into());
}

pub(crate) fn current_time_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
        * 1000.0
}

fn current_animation_frame_time_ms(scope: &mut v8::PinScope<'_, '_>) -> f64 {
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "performance").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|performance| {
            performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT)
        })
        .map(dom_time_since_origin_millis)
        .unwrap_or_else(current_time_ms)
}

fn dom_handle_from_marker_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<crate::document_runtime::DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| crate::document_runtime::DomHandle::new(index as usize));
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| crate::document_runtime::DomHandle::new(value as usize))
}

fn event_target_handle_from_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    host_ptr: *mut JsContextHost,
    host: &JsContextHost,
) -> Option<EventTargetHandle> {
    let this = args.this();
    let global = scope.get_current_context().global(scope);
    if this.strict_equals(global.into()) {
        return Some(EventTargetHandle::Window);
    }

    if let Some(handle) =
        crate::native_bridge::document::detached_native_handle_for_runtime(scope, host_ptr, this)
    {
        return Some(EventTargetHandle::Node(handle));
    }
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, this)
        && runtime_ptr == host_ptr
        && host.dom_host().node(handle).is_some()
    {
        return Some(EventTargetHandle::Node(handle));
    }

    let handle_value = this.get_internal_field(scope, 1)?;
    let handle_value = v8::Local::<v8::Value>::try_from(handle_value).ok()?;
    let handle_number = handle_value.number_value(scope)?;
    if !handle_number.is_finite() || handle_number.fract() != 0.0 || handle_number <= 0.0 {
        return None;
    }

    let reflector_id = ReflectorId::from_raw(handle_number as u64);
    let handle = host.resolve_node_wrapper_handle(reflector_id)?;
    host.dom_host().node(handle)?;
    Some(EventTargetHandle::Node(handle))
}

fn child_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    let global = scope.get_current_context().global(scope);
    if object.strict_equals(global.into()) {
        return object_child_window_handle(scope, object);
    }
    let field = object
        .get_internal_field(scope, 1)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| value.number_value(scope))?;
    if !field.is_finite() || field.fract() != 0.0 || field != 0.0 {
        return None;
    }
    object_child_window_handle(scope, object)
}

fn object_child_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    get_private_value(scope, object, WINDOW_CHILD_CONTEXT_HANDLE_SLOT)
        .and_then(|value| dom_handle_from_marker_value(scope, value))
}

fn window_message_endpoint_from_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<PendingWindowMessageEndpoint> {
    let global = scope.get_current_context().global(scope);
    if object.strict_equals(global.into()) {
        return Some(
            object_child_window_handle(scope, object)
                .map(PendingWindowMessageEndpoint::ChildWindow)
                .unwrap_or(PendingWindowMessageEndpoint::TopWindow),
        );
    }

    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, object) {
        return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
    }

    if let Some(popup_id) = crate::native_bridge::cross_origin_lightweight_popup_id(scope, object) {
        return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
    }

    if get_private_value(scope, object, TOP_WINDOW_MESSAGE_ENDPOINT_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
        || crate::native_bridge::is_cross_origin_top_window_proxy(scope, object)
    {
        return Some(PendingWindowMessageEndpoint::TopWindow);
    }

    if let Some(child_handle) = object_child_window_handle(scope, object) {
        return Some(PendingWindowMessageEndpoint::ChildWindow(child_handle));
    }

    None
}

fn current_window_message_source_identity(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    endpoint: PendingWindowMessageEndpoint,
) -> Option<(
    PendingWindowMessageEndpoint,
    WindowExecutionContextOwner,
    RuntimeObservableContextToken,
)> {
    let dispatch_scope = endpoint.dispatch_scope();
    let owner = host.current_window_execution_context_owner(dispatch_scope)?;
    let (realm_token, _) = host.window_execution_context(scope, owner, dispatch_scope)?;
    Some((endpoint, owner, realm_token))
}

fn incumbent_window_message_source_identity(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> Option<(
    PendingWindowMessageEndpoint,
    WindowExecutionContextOwner,
    RuntimeObservableContextToken,
)> {
    let incumbent_context = scope.get_incumbent_context()?;
    let incumbent_global = incumbent_context.global(scope);
    let endpoint = if let Some(popup_id) =
        crate::native_bridge::lightweight_popup_id_from_window(scope, incumbent_global)
    {
        PendingWindowMessageEndpoint::LightweightPopup(popup_id)
    } else {
        object_child_window_handle(scope, incumbent_global)
            .map(PendingWindowMessageEndpoint::ChildWindow)
            .unwrap_or(PendingWindowMessageEndpoint::TopWindow)
    };
    let incumbent_scope = &mut v8::ContextScope::new(scope, incumbent_context);
    let identity = host.current_runtime_window_execution_context_identity(incumbent_scope)?;
    (identity.dispatch_scope() == endpoint.dispatch_scope()).then_some((
        endpoint,
        identity.owner(),
        identity.realm_token(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bridge::element::iframe_width_attribute_viewport_width;
    use crate::structured_clone::{
        RuntimeMessageAgentCluster, StructuredCloneMetadata, V8StructuredClonePayload,
    };

    #[test]
    fn iframe_width_attribute_uses_html_dimension_rules() {
        assert_eq!(iframe_width_attribute_viewport_width("640"), Some(640.0));
        assert_eq!(iframe_width_attribute_viewport_width("1.5"), Some(1.0));
        assert_eq!(iframe_width_attribute_viewport_width("  +24px"), Some(24.0));
        assert_eq!(iframe_width_attribute_viewport_width("0"), None);
        assert_eq!(iframe_width_attribute_viewport_width("abc"), None);
        assert_eq!(
            iframe_width_attribute_viewport_width("50%"),
            Some(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width / 2.0)
        );
    }

    #[test]
    fn wasm_module_window_message_requires_same_origin_delivery() {
        let owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
            crate::frame_owner_model::LocalWindowId(1),
        );
        let endpoint = PendingWindowMessageEndpoint::TopWindow;
        let mut data = V8StructuredClonePayload::default();
        data.metadata = StructuredCloneMetadata {
            contains_wasm_module: true,
            origin_check_required: true,
            locked_to_sender_agent_cluster: true,
            sender_agent_cluster: Some(RuntimeMessageAgentCluster::WindowOrDedicatedWorker),
            sender_origin: Some("https://sender.test".to_owned()),
        };
        let message = PendingWindowMessage {
            target: WindowTaskTarget::new(endpoint.dispatch_scope(), owner),
            source: PendingWindowMessageSource::new(
                endpoint,
                owner,
                crate::native_bridge::RuntimeObservableContextToken::from_raw(1),
            ),
            data,
            origin: "https://sender.test".to_owned(),
            intended_target_origin: None,
        };

        assert!(!window_message_requires_messageerror(
            &message,
            Some("https://sender.test"),
        ));
        assert!(window_message_requires_messageerror(
            &message,
            Some("https://receiver.test"),
        ));
        assert!(!window_message_requires_messageerror(&message, None));

        let non_wasm = PendingWindowMessage {
            data: V8StructuredClonePayload::default(),
            ..message
        };
        assert!(!window_message_requires_messageerror(
            &non_wasm,
            Some("https://receiver.test"),
        ));
    }
}

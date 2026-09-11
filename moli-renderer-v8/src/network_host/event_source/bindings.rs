use super::*;
use crate::context_bootstrap::mark_event_trusted;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::EventSource)]
struct EventSourceObjectDeclaration<'scope> {
    #[webapi(slot = EVENT_SOURCE_URL_SLOT)]
    url: v8::Local<'scope, v8::String>,

    #[webapi(slot = EVENT_SOURCE_WITH_CREDENTIALS_SLOT)]
    with_credentials: bool,

    #[webapi(slot = EVENT_SOURCE_READY_STATE_SLOT)]
    ready_state: f64,

    #[webapi(slot = EVENT_SOURCE_ACTIVE_REQUEST_ID_SLOT)]
    active_request_id: v8::Local<'scope, v8::Value>,

    #[webapi(slot = EVENT_SOURCE_RECONNECT_TIMER_SLOT)]
    reconnect_timer: v8::Local<'scope, v8::Value>,

    #[webapi(slot = EVENT_SOURCE_RECONNECT_DELAY_SLOT)]
    reconnect_delay: f64,

    #[webapi(slot = EVENT_SOURCE_LAST_EVENT_ID_SLOT, init = "")]
    last_event_id: (),

    #[webapi(slot = EVENT_SOURCE_RESPONSE_URL_SLOT, init = "")]
    response_url: (),

    #[webapi(slot = EVENT_SOURCE_ONOPEN_SLOT, init = "null")]
    onopen: (),

    #[webapi(slot = EVENT_SOURCE_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),

    #[webapi(slot = EVENT_SOURCE_ONERROR_SLOT, init = "null")]
    onerror: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = EVENT_SOURCE_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EventSource, enumerable)]
struct EventSourceTemplateDeclaration {
    #[webapi(constant = "CONNECTING", value = EVENT_SOURCE_CONNECTING)]
    connecting: (),

    #[webapi(constant = "OPEN", value = EVENT_SOURCE_OPEN)]
    open: (),

    #[webapi(constant = "CLOSED", value = EVENT_SOURCE_CLOSED)]
    closed: (),

    #[webapi(method, length = 0, callback = event_source_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EventSource)]
struct EventSourceAccessorsDeclaration {
    #[webapi(accessor_property, getter = event_source_url_getter, enumerable)]
    url: (),

    #[webapi(
        accessor_property = "withCredentials",
        getter = event_source_with_credentials_getter,
        enumerable
    )]
    with_credentials: (),

    #[webapi(
        accessor_property = "readyState",
        getter = event_source_ready_state_getter,
        enumerable
    )]
    ready_state: (),

    #[webapi(
        accessor_property,
        getter = event_source_event_handler_getter,
        setter = event_source_event_handler_setter,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    onopen: (),

    #[webapi(
        accessor_property,
        getter = event_source_event_handler_getter,
        setter = event_source_event_handler_setter,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    onmessage: (),

    #[webapi(
        accessor_property,
        getter = event_source_event_handler_getter,
        setter = event_source_event_handler_setter,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    onerror: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct EventSourceMessageEventInitDeclaration<'scope> {
    data: v8::Local<'scope, v8::String>,
    origin: v8::Local<'scope, v8::String>,
    last_event_id: v8::Local<'scope, v8::String>,
}

#[derive(Clone, Copy)]
struct EventSourceEventHandler {
    event_type: &'static str,
    slot_name: &'static str,
}

const EVENT_SOURCE_EVENT_HANDLERS: &[EventSourceEventHandler] = &[
    EventSourceEventHandler {
        event_type: "open",
        slot_name: EVENT_SOURCE_ONOPEN_SLOT,
    },
    EventSourceEventHandler {
        event_type: "message",
        slot_name: EVENT_SOURCE_ONMESSAGE_SLOT,
    },
    EventSourceEventHandler {
        event_type: "error",
        slot_name: EVENT_SOURCE_ONERROR_SLOT,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EventSourceTerminalMode {
    Reconnect,
    Close,
}

pub(crate) fn install_event_source_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    EventSourceTemplateDeclaration::initialize_template(scope, template);
    EventSourceTemplateDeclaration::initialize_prototype_template(scope, prototype);
    EventSourceAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn initialize_event_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    url: &str,
    with_credentials: bool,
) {
    EventSourceObjectDeclaration::new(
        v8_string(scope, url).unwrap_or_else(|| v8::String::empty(scope)),
        with_credentials,
        EVENT_SOURCE_CONNECTING,
        v8::undefined(scope).into(),
        v8::undefined(scope).into(),
        EVENT_SOURCE_DEFAULT_RECONNECT_DELAY_MS as f64,
    )
    .initialize(scope, event_source)
    .expect("EventSource object declaration should initialize");
}

pub(crate) fn event_source_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> Option<String> {
    event_source_string_slot(scope, event_source, EVENT_SOURCE_URL_SLOT)
}

pub(crate) fn event_source_with_credentials<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, event_source, EVENT_SOURCE_WITH_CREDENTIALS_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn event_source_ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> f64 {
    get_private_value(scope, event_source, EVENT_SOURCE_READY_STATE_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(EVENT_SOURCE_CLOSED)
}

pub(crate) fn event_source_active_request_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, event_source, EVENT_SOURCE_ACTIVE_REQUEST_ID_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = value.u64_value();
    lossless.then_some(value)
}

pub(crate) fn event_source_last_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> String {
    event_source_string_slot(scope, event_source, EVENT_SOURCE_LAST_EVENT_ID_SLOT)
        .unwrap_or_default()
}

pub(crate) fn event_source_response_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> String {
    event_source_string_slot(scope, event_source, EVENT_SOURCE_RESPONSE_URL_SLOT)
        .unwrap_or_default()
}

pub(crate) fn event_source_connection_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let response_url = event_source_response_url(scope, event_source);
    if response_url.is_empty() {
        event_source_url(scope, event_source)
    } else {
        Some(response_url)
    }
}

pub(crate) fn event_source_reconnect_delay_ms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> u64 {
    get_private_value(scope, event_source, EVENT_SOURCE_RECONNECT_DELAY_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.min(u64::MAX as f64) as u64)
        .unwrap_or(EVENT_SOURCE_DEFAULT_RECONNECT_DELAY_MS)
}

pub(crate) fn set_event_source_active_request_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    internal_id: Option<u64>,
) {
    let value = internal_id
        .map(|internal_id| v8::BigInt::new_from_u64(scope, internal_id).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        event_source,
        EVENT_SOURCE_ACTIVE_REQUEST_ID_SLOT,
        value,
    );
}

pub(crate) fn open_event_source_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    response_url: &url::Url,
) {
    set_event_source_number_slot(
        scope,
        event_source,
        EVENT_SOURCE_READY_STATE_SLOT,
        EVENT_SOURCE_OPEN,
    );
    set_event_source_string_slot(
        scope,
        event_source,
        EVENT_SOURCE_RESPONSE_URL_SLOT,
        response_url.as_str(),
    );
    dispatch_event_source_named_event(scope, event_source, "open");
}

pub(crate) fn update_event_source_stream_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    last_event_id: &str,
    reconnect_delay_ms: u64,
) {
    set_event_source_string_slot(
        scope,
        event_source,
        EVENT_SOURCE_LAST_EVENT_ID_SLOT,
        last_event_id,
    );
    set_event_source_number_slot(
        scope,
        event_source,
        EVENT_SOURCE_RECONNECT_DELAY_SLOT,
        reconnect_delay_ms as f64,
    );
}

pub(crate) fn fail_event_source_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    mode: EventSourceTerminalMode,
) {
    set_event_source_active_request_id(scope, event_source, None);
    let ready_state = match mode {
        EventSourceTerminalMode::Reconnect => EVENT_SOURCE_CONNECTING,
        EventSourceTerminalMode::Close => EVENT_SOURCE_CLOSED,
    };
    set_event_source_number_slot(
        scope,
        event_source,
        EVENT_SOURCE_READY_STATE_SLOT,
        ready_state,
    );
    dispatch_event_source_named_event(scope, event_source, "error");
    if mode == EventSourceTerminalMode::Reconnect {
        let delay = event_source_reconnect_delay_ms(scope, event_source);
        schedule_event_source_connect(scope, event_source, delay);
    }
}

pub(crate) fn dispatch_event_source_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    message: &EventSourceMessage,
) {
    let global = scope.get_current_context().global(scope);
    let Some(constructor) = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let origin = event_source_response_url(scope, event_source)
        .parse::<url::Url>()
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_default();
    let Some(data) = v8_string(scope, &message.data) else {
        return;
    };
    let Some(origin) = v8_string(scope, &origin) else {
        return;
    };
    let Some(last_event_id) = v8_string(scope, &message.event_id) else {
        return;
    };
    let Ok(init) =
        EventSourceMessageEventInitDeclaration::new(data, origin, last_event_id).bind(scope)
    else {
        return;
    };
    let Some(event_type) = v8_string(scope, &message.event_name) else {
        return;
    };
    let Some(event) = constructor.new_instance(scope, &[event_type.into(), init.into()]) else {
        return;
    };
    mark_event_trusted(scope, event);
    let _ = dispatch_simple_event_target_event(
        scope,
        event_source,
        EVENT_SOURCE_LISTENERS_SLOT,
        &message.event_name,
        event,
    );
}

pub(crate) fn schedule_event_source_connect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    delay_ms: u64,
) -> bool {
    if event_source_ready_state(scope, event_source) == EVENT_SOURCE_CLOSED {
        return false;
    }
    clear_event_source_reconnect_timer(scope, event_source);
    let Some(callback) = v8::Function::builder(event_source_connect_timer_callback)
        .data(event_source.into())
        .build(scope)
    else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let timer_id = unsafe { &mut *host_ptr }.queue_timeout(
        scope,
        callback,
        delay_ms.min(u32::MAX as u64) as u32,
        crate::host::HostTimerOwner::Window,
        Vec::new(),
    );
    if timer_id == 0 {
        return false;
    }
    let timer_id = v8::Integer::new_from_unsigned(scope, timer_id);
    set_private_value(
        scope,
        event_source,
        EVENT_SOURCE_RECONNECT_TIMER_SLOT,
        timer_id.into(),
    );
    true
}

fn event_source_connect_timer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(event_source) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    set_private_value(
        scope,
        event_source,
        EVENT_SOURCE_RECONNECT_TIMER_SLOT,
        v8::undefined(scope).into(),
    );
    if event_source_ready_state(scope, event_source) != EVENT_SOURCE_CLOSED {
        super::request::start_event_source_request(scope, event_source);
    }
    rv.set_undefined();
}

fn event_source_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event_source = args.this();
    if !event_source_is_branded(scope, event_source) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    clear_event_source_reconnect_timer(scope, event_source);
    if let Some(internal_id) = event_source_active_request_id(scope, event_source)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let _ = unsafe { &mut *host_ptr }.abort_event_source_fetch(internal_id);
    }
    set_event_source_active_request_id(scope, event_source, None);
    set_event_source_number_slot(
        scope,
        event_source,
        EVENT_SOURCE_READY_STATE_SLOT,
        EVENT_SOURCE_CLOSED,
    );
    rv.set_undefined();
}

fn clear_event_source_reconnect_timer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) {
    let Some(timer_id) = get_private_value(scope, event_source, EVENT_SOURCE_RECONNECT_TIMER_SLOT)
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.uint32_value(scope))
    else {
        return;
    };
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.cancel_timer(timer_id);
    }
    set_private_value(
        scope,
        event_source,
        EVENT_SOURCE_RECONNECT_TIMER_SLOT,
        v8::undefined(scope).into(),
    );
}

fn event_source_url_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_source_slot_getter(scope, args.this(), EVENT_SOURCE_URL_SLOT, rv);
}

fn event_source_with_credentials_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_source_slot_getter(scope, args.this(), EVENT_SOURCE_WITH_CREDENTIALS_SLOT, rv);
}

fn event_source_ready_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_source_slot_getter(scope, args.this(), EVENT_SOURCE_READY_STATE_SLOT, rv);
}

fn event_source_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = get_private_value(scope, receiver, slot) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

fn event_source_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_source_is_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        EVENT_SOURCE_EVENT_HANDLERS,
        "EventSource event handlers",
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

fn event_source_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_source_is_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        EVENT_SOURCE_EVENT_HANDLERS,
        "EventSource event handlers",
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
        EVENT_SOURCE_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        stored.is_function(),
    );
}

fn dispatch_event_source_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    event_type: &str,
) {
    let global = scope.get_current_context().global(scope);
    let Some(constructor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(event_type_value) = v8_string(scope, event_type) else {
        return;
    };
    let Some(event) = constructor.new_instance(scope, &[event_type_value.into()]) else {
        return;
    };
    mark_event_trusted(scope, event);
    let _ = dispatch_simple_event_target_event(
        scope,
        event_source,
        EVENT_SOURCE_LISTENERS_SLOT,
        event_type,
        event,
    );
}

fn event_source_is_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, event_source, "EventSource")
}

fn event_source_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, event_source, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_event_source_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    let value = v8_string(scope, value).unwrap_or_else(|| v8::String::empty(scope));
    set_private_value(scope, event_source, slot, value.into());
}

fn set_event_source_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(scope, event_source, slot, value.into());
}

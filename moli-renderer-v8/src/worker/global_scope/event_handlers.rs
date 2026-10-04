//! Worker event-handler properties, private slots and fetch-handler classification.

use super::*;

pub(in crate::worker) const WORKER_GLOBAL_LISTENERS_SLOT: &str = "__moliWorkerGlobalListeners";

const WORKER_GLOBAL_ONMESSAGE_SLOT: &str = "__moliWorkerGlobalOnMessage";

const WORKER_GLOBAL_ONMESSAGEERROR_SLOT: &str = "__moliWorkerGlobalOnMessageError";

const WORKER_GLOBAL_ONERROR_SLOT: &str = "__moliWorkerGlobalOnError";

const WORKER_GLOBAL_ONCONNECT_SLOT: &str = "__moliWorkerGlobalOnConnect";

const WORKER_GLOBAL_ONINSTALL_SLOT: &str = "__moliWorkerGlobalOnInstall";

const WORKER_GLOBAL_ONACTIVATE_SLOT: &str = "__moliWorkerGlobalOnActivate";

const WORKER_GLOBAL_ONFETCH_SLOT: &str = "__moliWorkerGlobalOnFetch";

const WORKER_GLOBAL_ONPUSH_SLOT: &str = "__moliWorkerGlobalOnPush";

const WORKER_GLOBAL_ONSYNC_SLOT: &str = "__moliWorkerGlobalOnSync";

const WORKER_GLOBAL_ONPERIODICSYNC_SLOT: &str = "__moliWorkerGlobalOnPeriodicSync";

const WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT: &str = "__moliWorkerGlobalOnNotificationClick";

const WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT: &str = "__moliWorkerGlobalOnNotificationClose";

const WORKER_GLOBAL_ONOFFLINE_SLOT: &str = "__moliWorkerGlobalOnOffline";

const WORKER_GLOBAL_ONONLINE_SLOT: &str = "__moliWorkerGlobalOnOnline";

const WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT: &str = "__moliWorkerGlobalOnUnhandledRejection";

const WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT: &str = "__moliWorkerGlobalOnRejectionHandled";

#[derive(Default, WebApiObject)]
#[webapi(plain, enumerable)]
struct WorkerGlobalCommonEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onerror",
        getter = worker_global_onerror_getter,
        setter = worker_global_onerror_setter
    )]
    onerror: (),
    #[webapi(
        accessor_property = "onoffline",
        getter = worker_global_onoffline_getter,
        setter = worker_global_onoffline_setter
    )]
    onoffline: (),
    #[webapi(
        accessor_property = "ononline",
        getter = worker_global_ononline_getter,
        setter = worker_global_ononline_setter
    )]
    ononline: (),
    #[webapi(
        accessor_property = "onunhandledrejection",
        getter = worker_global_onunhandledrejection_getter,
        setter = worker_global_onunhandledrejection_setter
    )]
    onunhandledrejection: (),
    #[webapi(
        accessor_property = "onrejectionhandled",
        getter = worker_global_onrejectionhandled_getter,
        setter = worker_global_onrejectionhandled_setter
    )]
    onrejectionhandled: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain, enumerable)]
struct DedicatedWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onmessage",
        getter = worker_global_onmessage_getter,
        setter = worker_global_onmessage_setter
    )]
    onmessage: (),
    #[webapi(
        accessor_property = "onmessageerror",
        getter = worker_global_onmessageerror_getter,
        setter = worker_global_onmessageerror_setter
    )]
    onmessageerror: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain, enumerable)]
struct SharedWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onconnect",
        getter = worker_global_onconnect_getter,
        setter = worker_global_onconnect_setter
    )]
    onconnect: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalCommonEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONERROR_SLOT, init = "null")]
    onerror: (),
    #[webapi(slot = WORKER_GLOBAL_ONOFFLINE_SLOT, init = "null")]
    onoffline: (),
    #[webapi(slot = WORKER_GLOBAL_ONONLINE_SLOT, init = "null")]
    ononline: (),
    #[webapi(slot = WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT, init = "null")]
    onunhandledrejection: (),
    #[webapi(slot = WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT, init = "null")]
    onrejectionhandled: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct DedicatedWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct SharedWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONCONNECT_SLOT, init = "null")]
    onconnect: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain, enumerable)]
struct ServiceWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "oninstall",
        getter = worker_global_oninstall_getter,
        setter = worker_global_oninstall_setter
    )]
    oninstall: (),
    #[webapi(
        accessor_property = "onactivate",
        getter = worker_global_onactivate_getter,
        setter = worker_global_onactivate_setter
    )]
    onactivate: (),
    #[webapi(
        accessor_property = "onfetch",
        getter = worker_global_onfetch_getter,
        setter = worker_global_onfetch_setter
    )]
    onfetch: (),
    #[webapi(
        accessor_property = "onpush",
        getter = worker_global_onpush_getter,
        setter = worker_global_onpush_setter
    )]
    onpush: (),
    #[webapi(
        accessor_property = "onsync",
        getter = worker_global_onsync_getter,
        setter = worker_global_onsync_setter
    )]
    onsync: (),
    #[webapi(
        accessor_property = "onperiodicsync",
        getter = worker_global_onperiodicsync_getter,
        setter = worker_global_onperiodicsync_setter
    )]
    onperiodicsync: (),
    #[webapi(
        accessor_property = "onmessage",
        getter = worker_global_onmessage_getter,
        setter = worker_global_onmessage_setter
    )]
    onmessage: (),
    #[webapi(
        accessor_property = "onmessageerror",
        getter = worker_global_onmessageerror_getter,
        setter = worker_global_onmessageerror_setter
    )]
    onmessageerror: (),
    #[webapi(
        accessor_property = "onnotificationclick",
        getter = worker_global_onnotificationclick_getter,
        setter = worker_global_onnotificationclick_setter
    )]
    onnotificationclick: (),
    #[webapi(
        accessor_property = "onnotificationclose",
        getter = worker_global_onnotificationclose_getter,
        setter = worker_global_onnotificationclose_setter
    )]
    onnotificationclose: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct ServiceWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONINSTALL_SLOT, init = "null")]
    oninstall: (),
    #[webapi(slot = WORKER_GLOBAL_ONACTIVATE_SLOT, init = "null")]
    onactivate: (),
    #[webapi(slot = WORKER_GLOBAL_ONFETCH_SLOT, init = "null")]
    onfetch: (),
    #[webapi(slot = WORKER_GLOBAL_ONPUSH_SLOT, init = "null")]
    onpush: (),
    #[webapi(slot = WORKER_GLOBAL_ONSYNC_SLOT, init = "null")]
    onsync: (),
    #[webapi(slot = WORKER_GLOBAL_ONPERIODICSYNC_SLOT, init = "null")]
    onperiodicsync: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror: (),
    #[webapi(slot = WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT, init = "null")]
    onnotificationclick: (),
    #[webapi(slot = WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT, init = "null")]
    onnotificationclose: (),
}

pub(super) fn install_worker_global_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    global_kind: &crate::worker::thread::WorkerGlobalKind,
) -> Result<()> {
    if matches!(
        global_kind,
        crate::worker::thread::WorkerGlobalKind::Dedicated { .. }
    ) {
        DedicatedWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        DedicatedWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    if matches!(
        global_kind,
        crate::worker::thread::WorkerGlobalKind::Shared { .. }
    ) {
        SharedWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        SharedWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    if matches!(
        global_kind,
        crate::worker::thread::WorkerGlobalKind::Service { .. }
    ) {
        ServiceWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        ServiceWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    WorkerGlobalCommonEventHandlersDeclaration::default().initialize(scope, global)?;
    WorkerGlobalCommonEventHandlerStateDeclaration::default().initialize(scope, global)?;
    Ok(())
}

fn worker_global_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    slot_name: &str,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, global, slot_name).unwrap_or_else(|| v8::null(scope).into())
}

fn set_worker_global_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    slot_name: &'static str,
    event_type: Option<&str>,
) {
    let stored = if value.is_object() {
        value
    } else {
        v8::null(scope).into()
    };
    let active = stored.is_object();
    set_private_value(scope, global, slot_name, stored);
    if let Some(event_type) = event_type {
        simple_object_event_set_ordered_handler(
            scope,
            global,
            WORKER_GLOBAL_LISTENERS_SLOT,
            event_type,
            slot_name,
            active,
        );
    }
}

fn worker_global_onmessage_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONMESSAGE_SLOT,
    ));
}

fn worker_global_onmessage_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONMESSAGE_SLOT,
        Some("message"),
    );
}

fn worker_global_onmessageerror_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONMESSAGEERROR_SLOT,
    ));
}

fn worker_global_onmessageerror_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONMESSAGEERROR_SLOT,
        Some("messageerror"),
    );
}

fn worker_global_oninstall_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONINSTALL_SLOT,
    ));
}

fn worker_global_oninstall_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONINSTALL_SLOT,
        Some("install"),
    );
}

fn worker_global_onactivate_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONACTIVATE_SLOT,
    ));
}

fn worker_global_onactivate_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONACTIVATE_SLOT,
        Some("activate"),
    );
}

fn worker_global_onfetch_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONFETCH_SLOT,
    ));
}

fn worker_global_onfetch_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONFETCH_SLOT,
        Some("fetch"),
    );
}

fn worker_global_onpush_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONPUSH_SLOT,
    ));
}

fn worker_global_onpush_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONPUSH_SLOT,
        Some("push"),
    );
}

fn worker_global_onsync_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONSYNC_SLOT,
    ));
}

fn worker_global_onsync_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONSYNC_SLOT,
        Some("sync"),
    );
}

fn worker_global_onperiodicsync_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONPERIODICSYNC_SLOT,
    ));
}

fn worker_global_onperiodicsync_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONPERIODICSYNC_SLOT,
        Some("periodicsync"),
    );
}

fn worker_global_onnotificationclick_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT,
    ));
}

fn worker_global_onnotificationclick_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT,
        Some("notificationclick"),
    );
}

fn worker_global_onnotificationclose_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT,
    ));
}

fn worker_global_onnotificationclose_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT,
        Some("notificationclose"),
    );
}

fn worker_global_onerror_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONERROR_SLOT,
    ));
}

fn worker_global_onerror_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONERROR_SLOT,
        Some("error"),
    );
}

fn worker_global_onconnect_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONCONNECT_SLOT,
    ));
}

fn worker_global_onconnect_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONCONNECT_SLOT,
        Some("connect"),
    );
}

fn worker_global_onoffline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONOFFLINE_SLOT,
    ));
}

fn worker_global_onoffline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONOFFLINE_SLOT,
        Some("offline"),
    );
}

fn worker_global_ononline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONONLINE_SLOT,
    ));
}

fn worker_global_ononline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONONLINE_SLOT,
        Some("online"),
    );
}

fn worker_global_onunhandledrejection_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT,
    ));
}

fn worker_global_onunhandledrejection_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT,
        Some("unhandledrejection"),
    );
}

fn worker_global_onrejectionhandled_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT,
    ));
}

fn worker_global_onrejectionhandled_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT,
        Some("rejectionhandled"),
    );
}

pub(in crate::worker) fn service_worker_fetch_handler_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> WorkerFetchHandlerType {
    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "fetch",
    );
    if listeners.is_empty() {
        return WorkerFetchHandlerType::NoHandler;
    }
    if listeners.iter().all(|listener| {
        listener
            .callable_function()
            .is_some_and(|callback| function_source_has_empty_body(scope, callback))
    }) {
        WorkerFetchHandlerType::EmptyFetchHandler
    } else {
        WorkerFetchHandlerType::NotSkippable
    }
}

fn function_source_has_empty_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(source) = function.to_string(scope) else {
        return false;
    };
    let normalized = source
        .to_rust_string_lossy(scope)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let Some(open_brace) = normalized.find('{') else {
        return false;
    };
    let Some(close_brace) = normalized.rfind('}') else {
        return false;
    };
    open_brace < close_brace && normalized[open_brace + 1..close_brace].is_empty()
}

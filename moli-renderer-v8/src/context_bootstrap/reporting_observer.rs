//! Reporting observers share the native callback residence and browser task
//! scheduler. Reports in the global buffer are immutable serialized snapshots;
//! each observer receives separate application-visible dictionaries.

use crate::host::report_event_callback_exception;
use crate::observer_runtime::{self, ObserverCallbackId, ObserverCallbackResidence};
use crate::util::{
    context_host_ptr_from_global_bridge, define_v8_array_data_property, get_private_value,
    set_private_value, throw_type_error, v8_string, v8_string_from_utf16_units,
};
use crate::web_api_interfaces;
use crate::webidl;
use crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

const CALLBACK_ID: &str = "__moliReportingObserverCallbackId";
const CALLBACK: &str = "__moliReportingObserverCallback";
const CALLBACK_RELEVANT: &str = "__moliReportingObserverCallbackRelevant";
const CALLBACK_INCUMBENT: &str = "__moliReportingObserverCallbackIncumbent";
const TYPES: &str = "__moliReportingObserverTypes";
const BUFFERED: &str = "__moliReportingObserverBuffered";
const PENDING: &str = "__moliReportingObserverPending";
const WORKER_REGISTRY: &str = "__moliReportingObserverWorkerRegistry";
const REPORT_BUFFER: &str = "__moliReportingObserverReportBuffer";
const REPORT_TYPE: &str = "__moliReportingObserverReportType";
const REPORT_JSON: &str = "__moliReportingObserverReportJson";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReportingObserver)]
struct ReportingObserverDeclaration<'s> {
    #[webapi(slot = CALLBACK_ID)]
    callback_id: u32,
    #[webapi(slot = CALLBACK)]
    callback: v8::Local<'s, v8::Object>,
    #[webapi(slot = CALLBACK_RELEVANT)]
    relevant: v8::Local<'s, v8::Object>,
    #[webapi(slot = CALLBACK_INCUMBENT)]
    incumbent: v8::Local<'s, v8::Object>,
    #[webapi(slot = TYPES)]
    types: v8::Local<'s, v8::Array>,
    #[webapi(slot = BUFFERED)]
    buffered: bool,
    #[webapi(slot = PENDING, init = "array")]
    pending: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReportingObserver, enumerable, receiver)]
pub(super) struct ReportingObserverPrototypeDeclaration {
    #[webapi(method, length = 0, callback = observe)]
    observe: (),
    #[webapi(method, length = 0, callback = disconnect)]
    disconnect: (),
    #[webapi(method, length = 0, callback = take_records)]
    take_records: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReportingObserver")]
struct ConstructorArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(index = 1, dictionary)]
    options: ReportingObserverOptions,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ReportingObserverOptions")]
struct ReportingObserverOptions {
    #[webidl(default = false)]
    buffered: bool,
    #[webidl(converter = "raw")]
    types: Option<webidl::Sequence<webidl::DomString16>>,
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "ReportingObserver requires the new operator.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let (id, callback, relevant, incumbent) =
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let (id, callback, relevant, incumbent) =
                observer_runtime::register_callback(scope, host_ptr, args.this(), parsed.callback)
                    .into_parts();
            (id.as_u32(), callback, relevant, incumbent)
        } else {
            let callback = v8::Local::<v8::Object>::try_from(parsed.callback.value(scope))
                .expect("converted callback is an object");
            let relevant = parsed.callback.relevant_context(scope).global(scope);
            let incumbent = parsed.callback.incumbent_context(scope).global(scope);
            (0, callback, relevant, incumbent)
        };
    let types = v8::Array::new(scope, 0);
    for value in parsed.options.types.into_iter().flat_map(|types| types.0) {
        let value =
            v8_string_from_utf16_units(scope, &value.0).expect("report type should allocate");
        append(scope, types, value.into());
    }
    ReportingObserverDeclaration::new(
        id,
        callback,
        relevant,
        incumbent,
        types,
        parsed.options.buffered,
    )
    .initialize(scope, args.this())
    .expect("ReportingObserver state should initialize");
    rv.set(args.this().into());
}

fn observe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer = receiver(scope, args.this());
    let Some(context) = observer.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let Some(id) = callback_id(scope, observer) else {
            return;
        };
        if !observer_runtime::activate_reporting_observer_callback(scope, host_ptr, id, observer) {
            return;
        }
    } else if crate::worker::get_worker_state(scope).is_some() {
        let registry = global_array(scope, WORKER_REGISTRY);
        if !(0..registry.length()).any(|index| {
            registry
                .get_index(scope, index)
                .is_some_and(|value| value.strict_equals(observer.into()))
        }) {
            append(scope, registry, observer.into());
        }
    } else {
        return;
    }
    if !get_private_value(scope, observer, BUFFERED).is_some_and(|value| value.is_true()) {
        return;
    }
    let buffered = v8::Boolean::new(scope, false);
    set_private_value(scope, observer, BUFFERED, buffered.into());
    let buffer = global_array(scope, REPORT_BUFFER);
    for index in 0..buffer.length() {
        let report = buffer.get_index(scope, index).expect("dense report buffer");
        let data = v8::Array::new_with_elements(scope, &[observer.into(), report]);
        queue_task(scope, add_buffered_report, data.into());
    }
}

fn disconnect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer = receiver(scope, args.this());
    let Some(context) = observer.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        if let Some(id) = callback_id(scope, observer) {
            observer_runtime::deactivate_reporting_observer_callback(host_ptr, id);
        }
    } else {
        let registry = global_array(scope, WORKER_REGISTRY);
        let next = v8::Array::new(scope, 0);
        for index in 0..registry.length() {
            let value = registry
                .get_index(scope, index)
                .expect("dense worker registry");
            if !value.strict_equals(observer.into()) {
                append(scope, next, value);
            }
        }
        let global = context.global(scope);
        set_private_value(scope, global, WORKER_REGISTRY, next.into());
    }
    // Disconnect affects future registration only. Queued reports and task
    // snapshots remain alive and deliverable until takeRecords or invocation.
}

fn take_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer = receiver(scope, args.this());
    let records = pending(scope, observer);
    let result = expose_reports(scope, records);
    let next = v8::Array::new(scope, 0);
    set_private_value(scope, observer, PENDING, next.into());
    rv.set(result.into());
}

/// Called by native report producers in the protected Window/Worker realm.
/// There is no author-callable report injection surface.
pub(crate) fn notify_report(
    scope: &mut v8::PinScope<'_, '_>,
    report_type: &str,
    url: &str,
    body: serde_json::Value,
) {
    let report = crate::util::new_null_prototype_object(scope);
    let kind = v8_string(scope, report_type).expect("report type should allocate");
    let json = v8_string(
        scope,
        &serde_json::json!({"type":report_type,"url":url,"body":body}).to_string(),
    )
    .expect("report should serialize");
    set_private_value(scope, report, REPORT_TYPE, kind.into());
    set_private_value(scope, report, REPORT_JSON, json.into());
    for observer in registered(scope) {
        add_report(scope, observer, report);
    }

    let buffer = global_array(scope, REPORT_BUFFER);
    let next = v8::Array::new(scope, 0);
    let matching = (0..buffer.length())
        .filter(|index| {
            let previous = v8::Local::<v8::Object>::try_from(
                buffer
                    .get_index(scope, *index)
                    .expect("dense report buffer"),
            )
            .expect("internal report is an object");
            get_private_value(scope, previous, REPORT_TYPE)
                .is_some_and(|value| value.strict_equals(kind.into()))
        })
        .count();
    let mut drop_first = matching >= 100;
    for index in 0..buffer.length() {
        let previous = buffer.get_index(scope, index).expect("dense report buffer");
        let object =
            v8::Local::<v8::Object>::try_from(previous).expect("internal report is an object");
        if drop_first
            && get_private_value(scope, object, REPORT_TYPE)
                .is_some_and(|value| value.strict_equals(kind.into()))
        {
            drop_first = false;
            continue;
        }
        append(scope, next, previous);
    }
    append(scope, next, report.into());
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, REPORT_BUFFER, next.into());
}

fn add_buffered_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("buffered report task data");
    let observer = v8::Local::<v8::Object>::try_from(data.get_index(scope, 0).unwrap()).unwrap();
    let report = v8::Local::<v8::Object>::try_from(data.get_index(scope, 1).unwrap()).unwrap();
    if callback_current(scope, observer) {
        add_report(scope, observer, report);
    }
}

fn add_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    report: v8::Local<'s, v8::Object>,
) {
    if !callback_current(scope, observer) {
        return;
    }
    let kind = get_private_value(scope, report, REPORT_TYPE).expect("internal report type");
    let types = slot_array(scope, observer, TYPES);
    if types.length() > 0
        && !(0..types.length()).any(|index| {
            types
                .get_index(scope, index)
                .is_some_and(|value| value.strict_equals(kind))
        })
    {
        return;
    }
    let queue = pending(scope, observer);
    append(scope, queue, report.into());
    if queue.length() == 1 {
        let observers = registered(scope)
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let notify_list = v8::Array::new_with_elements(scope, &observers);
        queue_task(scope, invoke_observers, notify_list.into());
    }
}

fn invoke_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let list = v8::Local::<v8::Array>::try_from(args.data()).expect("observer notify list");
    for index in 0..list.length() {
        let observer =
            v8::Local::<v8::Object>::try_from(list.get_index(scope, index).unwrap()).unwrap();
        if !callback_current(scope, observer) {
            continue;
        }
        let reports = pending(scope, observer);
        if reports.length() == 0 {
            continue;
        }
        let next = v8::Array::new(scope, 0);
        set_private_value(scope, observer, PENDING, next.into());
        let callback = slot_object(scope, observer, CALLBACK);
        let relevant = slot_object(scope, observer, CALLBACK_RELEVANT);
        let incumbent = slot_object(scope, observer, CALLBACK_INCUMBENT);
        let host_ptr = context_host_ptr_from_global_bridge(scope);
        let Some(context) = relevant.get_creation_context(scope) else {
            continue;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        // Web IDL converts the sequence and its dictionaries in the callback's
        // realm. The report queue holds
        // immutable native snapshots rather than objects from the producer.
        let arguments = [expose_reports(scope, reports).into(), observer.into()];
        if let Some(host_ptr) = host_ptr {
            let residence = ObserverCallbackResidence::from_parts(
                callback_id(scope, observer).expect("Window callback identity"),
                callback,
                relevant,
                incumbent,
            );
            let Some(callback) = observer_runtime::prepare_callback(scope, host_ptr, residence)
            else {
                continue;
            };
            if let WindowWebIdlCallbackFunctionOutcome::Threw(report) = callback.invoke(
                scope,
                host_ptr,
                "ReportingObserver callback",
                observer.into(),
                &arguments,
            ) {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "reportingobserver",
                    callback.relevant_identity(),
                    None,
                    &report,
                );
            }
        } else {
            let Some(relevant) = relevant.get_creation_context(scope) else {
                continue;
            };
            let Some(incumbent) = incumbent.get_creation_context(scope) else {
                continue;
            };
            let Some(callback) = moli_webidl_callback::PreparedWebIdlCallbackFunction::try_new(
                scope, callback, relevant, incumbent,
            ) else {
                continue;
            };
            let _ = moli_webidl_callback::invoke_webidl_callback_function(
                scope,
                &callback,
                observer.into(),
                &arguments,
                |scope, callback, receiver, arguments| {
                    crate::exception_reporting::invoke_callback(
                        scope,
                        "ReportingObserver callback",
                        callback,
                        receiver,
                        arguments,
                    )
                },
            );
        }
    }
}

fn registered<'s>(scope: &mut v8::PinScope<'s, '_>) -> Vec<v8::Local<'s, v8::Object>> {
    let context = scope.get_current_context();
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        observer_runtime::active_reporting_observer_callbacks(scope, host_ptr)
            .into_iter()
            .filter(|observer| observer.get_creation_context(scope) == Some(context))
            .collect()
    } else {
        let registry = global_array(scope, WORKER_REGISTRY);
        (0..registry.length())
            .map(|index| {
                v8::Local::<v8::Object>::try_from(registry.get_index(scope, index).unwrap())
                    .unwrap()
            })
            .collect()
    }
}

fn callback_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        callback_id(scope, observer)
            .is_some_and(|id| observer_runtime::callback_is_current(host_ptr, id))
    } else {
        crate::worker::get_worker_state(scope).is_some()
    }
}

fn queue_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
) {
    let callback = v8::Function::builder(callback)
        .data(data)
        .build(scope)
        .expect("reporting task should allocate");
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.queue_networking_task(scope, callback);
    } else {
        let _ = crate::worker::queue_worker_networking_task(scope, callback);
    }
}

fn receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    web_api_object_target(scope, receiver).expect("generated receiver validation precedes callback")
}

fn callback_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<ObserverCallbackId> {
    let value = get_private_value(scope, observer, CALLBACK_ID)?;
    ObserverCallbackId::from_number(value.number_value(scope)?)
}

fn slot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Object> {
    v8::Local::<v8::Object>::try_from(
        get_private_value(scope, object, slot).expect("observer object slot"),
    )
    .expect("observer slot is an object")
}

fn slot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Array> {
    v8::Local::<v8::Array>::try_from(
        get_private_value(scope, object, slot).expect("observer array slot"),
    )
    .expect("observer slot is an array")
}

fn pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    slot_array(scope, observer, PENDING)
}

fn expose_reports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    records: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Array> {
    let reports = v8::Array::new(scope, 0);
    for index in 0..records.length() {
        let report = v8::Local::<v8::Object>::try_from(
            records.get_index(scope, index).expect("dense report queue"),
        )
        .expect("internal report is an object");
        let json = v8::Local::<v8::String>::try_from(
            get_private_value(scope, report, REPORT_JSON).expect("internal report JSON"),
        )
        .expect("serialized report is a string");
        let exposed = v8::json::parse(scope, json).expect("native report JSON should parse");
        append(scope, reports, exposed);
    }
    reports
}

fn global_array<'s>(scope: &mut v8::PinScope<'s, '_>, slot: &str) -> v8::Local<'s, v8::Array> {
    let global = scope.get_current_context().global(scope);
    if let Some(value) = get_private_value(scope, global, slot) {
        return v8::Local::<v8::Array>::try_from(value).expect("global report array");
    }
    let array = v8::Array::new(scope, 0);
    set_private_value(scope, global, slot, array.into());
    array
}

fn append<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    value: v8::Local<'s, v8::Value>,
) {
    assert_eq!(
        define_v8_array_data_property(scope, array, array.length(), value),
        Some(())
    );
}

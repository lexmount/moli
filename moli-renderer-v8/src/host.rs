use std::{
    cell::Cell,
    collections::HashMap,
    fmt,
    time::{Duration, Instant},
};

use moli_cookie_jar::SharedBrowserCookieStore;
use url::Url;

use crate::dom::native::NativeNodeId;

use super::{
    callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker},
    context_bootstrap::dispatch_window_error_event_with_details,
    document_runtime::DomHandle,
    exception_reporting::{V8ExceptionReport, build_event_handler_exception_report},
    native_bridge::CALLBACK_ERROR_WINDOW_HANDLE_SLOT,
    native_bridge::JsContextHost,
    util::{v8_string, v8str},
};

mod document;
mod events;
mod scripts;
mod timers;
mod window_callbacks;

pub use self::document::*;
use self::document::{EVENT_STOP_IMMEDIATE_SLOT, EVENT_STOP_PROPAGATION_SLOT};
pub(super) use self::events::{
    DispatchStatus, HostEventTargetRegistry, PublicEventDispatchResult, create_host_event,
    dispatch_host_event, dispatch_public_event, dispatch_public_event_with_original_target,
    event_dispatch_status, event_handler_content_attribute_owner, event_target_value,
    host_event_defaults, invoke_prepared_before_unload_event_handler,
    invoke_prepared_event_callback, invoke_prepared_event_callback_on_object,
    report_event_callback_exception, report_event_listener_exception,
};
pub(crate) use self::events::{EventListenerInspectorSnapshot, EventListenerRegistration};
pub(super) use self::scripts::{
    CommittedInlineClassicScript, FailedDynamicScript, HostScriptScheduler, ModuleFailurePolicy,
    PreparedRuntimeScriptStartCommit, QueuedScriptFailureKind, RuntimeScriptAdmission,
    RuntimeScriptAdmissionPayload, RuntimeScriptPreparationContext, RuntimeScriptStartDecision,
    RuntimeScriptStartPlan, RuntimeScriptStartReservation, RuntimeScriptTextPreparationReservation,
    ScriptElementLoader, ScriptElementLoaderOptions, ScriptEventKind, ScriptEventTask,
    ScriptHandleSource, ScriptPageTaskExecutionKind, apply_parser_script_element_state_transition,
    apply_parser_script_element_state_without_execution,
    begin_prepared_document_write_script_start, build_runtime_prepared_script,
    cancel_runtime_script_start_admission, dispatch_script_event,
    finish_runtime_script_start_admission, plan_script_start, prepare_runtime_script_start_commit,
    script_element_nonce_for_csp,
};
#[cfg(test)]
pub(super) use self::scripts::{
    DynamicScriptBatch, ScriptHandleExecutionSubject, ScriptHostEventSubject,
};
pub(super) use self::timers::{HostTimeoutRunResult, HostTimeoutScheduler, HostTimerOwner};

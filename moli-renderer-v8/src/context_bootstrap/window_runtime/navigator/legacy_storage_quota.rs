//! Deprecated Chromium storage-quota compatibility callbacks.
//!
//! These methods are legacy surfaces, but their JavaScript arguments are
//! still Web IDL callback functions. Admission converts every callback in
//! argument order, captures its relevant/incumbent Realms, and publishes one
//! exact Window/Document task to the miscellaneous-platform task source.
//! Quota state is intentionally synthetic; scheduling, owner authorization,
//! task completion, and retry policy do not live in this module.

use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;
use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    host::report_event_callback_exception,
    native_bridge::JsContextHost,
    page_task_queue::RendererPageMiscPlatformApiTaskKind,
    util::context_host_ptr_from_global_bridge,
    webidl,
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};

use super::super::super::navigation_window::navigation_document_has_opaque_origin;

const STORAGE_QUOTA_BYTES: f64 = 1_073_741_824.0;
const STORAGE_USAGE_BYTES: f64 = 0.0;
const TEMPORARY_STORAGE_TYPE: f64 = 0.0;
const PERSISTENT_STORAGE_TYPE: f64 = 1.0;

#[derive(Default, WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::DeprecatedStorageQuota)]
struct LegacyStorageQuotaObjectDeclaration {
    #[webapi(
        method,
        enumerable,
        length = 1,
        callback = legacy_storage_quota_query_usage_and_quota_callback
    )]
    query_usage_and_quota: (),

    #[webapi(
        method,
        enumerable,
        length = 2,
        callback = legacy_storage_quota_request_quota_callback
    )]
    request_quota: (),
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::DeprecatedStorageInfo)]
struct LegacyStorageInfoObjectDeclaration {
    #[webapi(data_property = "TEMPORARY", enumerable)]
    temporary: f64,

    #[webapi(data_property = "PERSISTENT", enumerable)]
    persistent: f64,

    #[webapi(
        method,
        enumerable,
        length = 2,
        callback = legacy_storage_info_query_usage_and_quota_callback
    )]
    query_usage_and_quota: (),

    #[webapi(
        method,
        enumerable,
        length = 3,
        callback = legacy_storage_info_request_quota_callback
    )]
    request_quota: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DeprecatedStorageQuota.queryUsageAndQuota")]
struct LegacyStorageQuotaQueryArgs {
    #[webidl(required, converter = "callback_function")]
    success_callback: webidl::WebIdlCallbackFunction,
    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DeprecatedStorageQuota.requestQuota")]
struct LegacyStorageQuotaRequestArgs {
    #[webidl(required)]
    requested_quota: u64,
    #[webidl(nullable, converter = "callback_function")]
    success_callback: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

/// Historical `webkitStorageInfo` used an `unsigned short` storage type and
/// made both callbacks optional and nullable. The storage type is now
/// ignored, but its Web IDL conversion still occurs before callback
/// conversion and outcome selection.
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StorageInfo.queryUsageAndQuota")]
struct LegacyStorageInfoQueryArgs {
    #[webidl(required)]
    storage_type: u16,
    #[webidl(nullable, converter = "callback_function")]
    success_callback: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StorageInfo.requestQuota")]
struct LegacyStorageInfoRequestArgs {
    #[webidl(required)]
    storage_type: u16,
    #[webidl(required)]
    requested_quota: u64,
    #[webidl(nullable, converter = "callback_function")]
    success_callback: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

/// Immutable callback result retained until the exact miscellaneous-platform
/// task is selected.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LegacyStorageQuotaCallbackOutcome {
    UsageAndQuota {
        usage: f64,
        quota: f64,
    },
    GrantedQuota {
        granted: f64,
    },
    Error {
        name: &'static str,
        message: &'static str,
    },
}

impl LegacyStorageQuotaCallbackOutcome {
    pub(crate) const fn kind(&self) -> RendererPageMiscPlatformApiTaskKind {
        match self {
            Self::UsageAndQuota { .. } => {
                RendererPageMiscPlatformApiTaskKind::LegacyStorageUsageAndQuota
            }
            Self::GrantedQuota { .. } => {
                RendererPageMiscPlatformApiTaskKind::LegacyStorageGrantedQuota
            }
            Self::Error { .. } => RendererPageMiscPlatformApiTaskKind::LegacyStorageError,
        }
    }
}

/// One-shot typed callback plus the result selected during admission.
pub(crate) struct LegacyStorageQuotaCallbackTask {
    callback: WindowWebIdlCallbackFunction,
    outcome: LegacyStorageQuotaCallbackOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyStorageQuotaCallbackTaskEffect {
    CallbackInvoked,
    CallbackNotInvoked,
}

impl LegacyStorageQuotaCallbackTask {
    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
        outcome: LegacyStorageQuotaCallbackOutcome,
    ) -> Self {
        Self {
            callback: WindowWebIdlCallbackFunction::new(scope, host, callback),
            outcome,
        }
    }

    pub(crate) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> LegacyStorageQuotaCallbackTaskEffect {
        let prepared = self.callback.prepare(scope);
        let relevant_identity = prepared.relevant_identity();
        let host = unsafe { &*host_ptr };
        if !prepared.is_current(host) {
            return LegacyStorageQuotaCallbackTaskEffect::CallbackNotInvoked;
        }
        let Some(relevant_context) = self.callback.relevant_context(scope) else {
            return LegacyStorageQuotaCallbackTaskEffect::CallbackNotInvoked;
        };
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        let arguments: Vec<v8::Local<'_, v8::Value>> = match self.outcome {
            LegacyStorageQuotaCallbackOutcome::UsageAndQuota { usage, quota } => vec![
                v8::Number::new(scope, usage).into(),
                v8::Number::new(scope, quota).into(),
            ],
            LegacyStorageQuotaCallbackOutcome::GrantedQuota { granted } => {
                vec![v8::Number::new(scope, granted).into()]
            }
            LegacyStorageQuotaCallbackOutcome::Error { name, message } => {
                vec![crate::context_bootstrap::new_dom_error_value(
                    scope, name, message,
                )]
            }
        };
        let receiver = v8::undefined(scope);
        match invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            match self.outcome {
                LegacyStorageQuotaCallbackOutcome::UsageAndQuota { .. } => "StorageUsageCallback",
                LegacyStorageQuotaCallbackOutcome::GrantedQuota { .. } => "StorageQuotaCallback",
                LegacyStorageQuotaCallbackOutcome::Error { .. } => "StorageErrorCallback",
            },
            "deprecated storage quota callback threw",
            "deprecated storage quota callback",
            &prepared,
            receiver.into(),
            &arguments,
        ) {
            WindowWebIdlCallbackFunctionOutcome::Returned => {
                LegacyStorageQuotaCallbackTaskEffect::CallbackInvoked
            }
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "deprecatedstoragequota",
                    relevant_identity,
                    None,
                    &report,
                );
                LegacyStorageQuotaCallbackTaskEffect::CallbackInvoked
            }
            WindowWebIdlCallbackFunctionOutcome::Retired => {
                LegacyStorageQuotaCallbackTaskEffect::CallbackNotInvoked
            }
        }
    }
}

pub(in crate::context_bootstrap) fn build_legacy_storage_quota_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    LegacyStorageQuotaObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow::anyhow!("failed to bind legacy StorageQuota object: {error}"))
}

pub(in crate::context_bootstrap) fn build_legacy_storage_info_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    LegacyStorageInfoObjectDeclaration::new(TEMPORARY_STORAGE_TYPE, PERSISTENT_STORAGE_TYPE)
        .bind(scope)
        .map_err(|error| anyhow::anyhow!("failed to bind legacy webkitStorageInfo object: {error}"))
}

fn legacy_storage_quota_query_usage_and_quota_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LegacyStorageQuotaQueryArgs>(scope, &args) else {
        return;
    };
    queue_legacy_storage_query(scope, Some(parsed.success_callback), parsed.error_callback);
}

fn legacy_storage_quota_request_quota_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LegacyStorageQuotaRequestArgs>(scope, &args) else {
        return;
    };
    queue_legacy_storage_request(
        scope,
        parsed.requested_quota,
        parsed.success_callback,
        parsed.error_callback,
    );
}

fn legacy_storage_info_query_usage_and_quota_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LegacyStorageInfoQueryArgs>(scope, &args) else {
        return;
    };
    let _storage_type = parsed.storage_type;
    queue_legacy_storage_query(scope, parsed.success_callback, parsed.error_callback);
}

fn legacy_storage_info_request_quota_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LegacyStorageInfoRequestArgs>(scope, &args) else {
        return;
    };
    let _storage_type = parsed.storage_type;
    queue_legacy_storage_request(
        scope,
        parsed.requested_quota,
        parsed.success_callback,
        parsed.error_callback,
    );
}

fn queue_legacy_storage_query(
    scope: &mut v8::PinScope<'_, '_>,
    success_callback: Option<WebIdlCallbackFunction>,
    error_callback: Option<WebIdlCallbackFunction>,
) {
    let global = scope.get_current_context().global(scope);
    let opaque_origin = navigation_document_has_opaque_origin(scope, global);
    let selected = if opaque_origin {
        error_callback.map(|callback| {
            (
                callback,
                LegacyStorageQuotaCallbackOutcome::Error {
                    name: "NotSupportedError",
                    message:
                        "The implementation did not support the requested type of object or operation.",
                },
            )
        })
    } else {
        success_callback.map(|callback| {
            (
                callback,
                LegacyStorageQuotaCallbackOutcome::UsageAndQuota {
                    usage: STORAGE_USAGE_BYTES,
                    quota: STORAGE_QUOTA_BYTES,
                },
            )
        })
    };
    queue_legacy_storage_callback(scope, selected);
}

fn queue_legacy_storage_request(
    scope: &mut v8::PinScope<'_, '_>,
    requested_quota: u64,
    success_callback: Option<WebIdlCallbackFunction>,
    error_callback: Option<WebIdlCallbackFunction>,
) {
    let global = scope.get_current_context().global(scope);
    let opaque_origin = navigation_document_has_opaque_origin(scope, global);
    let selected = if opaque_origin {
        error_callback.map(|callback| {
            (
                callback,
                LegacyStorageQuotaCallbackOutcome::Error {
                    name: "AbortError",
                    message: "The user aborted a request.",
                },
            )
        })
    } else {
        success_callback.map(|callback| {
            (
                callback,
                LegacyStorageQuotaCallbackOutcome::GrantedQuota {
                    granted: (requested_quota as f64).min(STORAGE_QUOTA_BYTES),
                },
            )
        })
    };
    queue_legacy_storage_callback(scope, selected);
}

fn queue_legacy_storage_callback(
    scope: &mut v8::PinScope<'_, '_>,
    selected: Option<(WebIdlCallbackFunction, LegacyStorageQuotaCallbackOutcome)>,
) {
    let Some((callback, outcome)) = selected else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = unsafe { &mut *host_ptr }
        .queue_legacy_storage_quota_callback_task(scope, callback, outcome);
}

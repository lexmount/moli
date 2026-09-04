//! Chromium's small browser-vendor surface exposed to ordinary web pages.
//!
//! Chrome installs `chrome.loadTimes()` and `chrome.csi()` through a V8
//! extension, while the extensions bindings layer adds the legacy `chrome.app`
//! API to web-page contexts. Keep this separate from WebIDL APIs: these are
//! Chromium compatibility properties, not web-platform interfaces.

use super::{
    dom_time_since_origin_millis, performance_runtime::current_window_navigation_timing_snapshot,
};
use crate::{
    host::HostTimerOwner,
    util::{context_host_ptr_from_global_bridge, throw_type_error, v8str},
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

const INSTALL_STATE_NOT_INSTALLED: &str = "not_installed";
const RUNNING_STATE_CANNOT_RUN: &str = "cannot_run";

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ChromeInstallStateDeclaration {
    #[webapi(data_property = "DISABLED")]
    disabled: &'static str,
    #[webapi(data_property = "INSTALLED")]
    installed: &'static str,
    #[webapi(data_property = "NOT_INSTALLED")]
    not_installed: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ChromeRunningStateDeclaration {
    #[webapi(data_property = "CANNOT_RUN")]
    cannot_run: &'static str,
    #[webapi(data_property = "READY_TO_RUN")]
    ready_to_run: &'static str,
    #[webapi(data_property = "RUNNING")]
    running: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct ChromeAppDeclaration<'scope> {
    #[webapi(
        native_data_property = "isInstalled",
        getter = chrome_value_getter,
        setter = chrome_value_empty_setter,
        data = v8::Boolean::new(scope, false)
    )]
    is_installed: (),
    #[webapi(method = "getDetails", length = 0, callback = chrome_app_get_details)]
    get_details: (),
    #[webapi(
        method = "getIsInstalled",
        length = 0,
        callback = chrome_app_get_is_installed
    )]
    get_is_installed: (),
    #[webapi(method = "installState", length = 0, callback = chrome_app_install_state)]
    install_state_method: (),
    #[webapi(method = "runningState", length = 0, callback = chrome_app_running_state)]
    running_state_method: (),
    #[webapi(data_property = "InstallState")]
    install_state: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "RunningState")]
    running_state: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap) fn install_chrome_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let chrome = v8::Object::new(scope);
    let load_times = v8::Function::builder(chrome_load_times)
        .length(0)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create chrome.loadTimes"))?;
    let csi = v8::Function::builder(chrome_csi)
        .length(0)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create chrome.csi"))?;
    let install_state =
        ChromeInstallStateDeclaration::new("disabled", "installed", INSTALL_STATE_NOT_INSTALLED)
            .bind(scope)
            .map_err(|error| anyhow!("failed to create chrome.app.InstallState: {error}"))?;
    let running_state =
        ChromeRunningStateDeclaration::new(RUNNING_STATE_CANNOT_RUN, "ready_to_run", "running")
            .bind(scope)
            .map_err(|error| anyhow!("failed to create chrome.app.RunningState: {error}"))?;
    let app = ChromeAppDeclaration::new(install_state, running_state)
        .bind(scope)
        .map_err(|error| anyhow!("failed to create chrome.app: {error}"))?;

    define_chrome_member(scope, chrome, "loadTimes", load_times.into())?;
    define_chrome_member(scope, chrome, "csi", csi.into())?;
    define_chrome_member(scope, chrome, "app", app.into())?;

    global
        .define_own_property(
            scope,
            v8str(scope, "chrome").into(),
            chrome.into(),
            v8::PropertyAttribute::DONT_DELETE,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define window.chrome"))
}

fn define_chrome_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chrome: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: v8::Local<'s, v8::Value>,
) -> Result<()> {
    chrome
        .create_data_property(scope, v8str(scope, name).into(), value)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define chrome.{name}"))
}

fn define_chrome_snapshot_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: v8::Local<'s, v8::Value>,
) -> Result<()> {
    object
        .set_native_data_property_with_configuration(
            scope,
            v8str(scope, name).into(),
            v8::NativeDataPropertyConfiguration::new(chrome_value_getter)
                .setter(chrome_value_empty_setter)
                .data(value),
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define chrome timing property {name}"))
}

fn chrome_value_getter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data());
}

fn chrome_value_empty_setter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    _value: v8::Local<'s, v8::Value>,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) {
}

fn chrome_load_times<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_null();
    let Some(timing) = current_window_navigation_timing_snapshot(scope) else {
        return;
    };
    let result = v8::Object::new(scope);
    let start_seconds = timing.time_origin_millis / 1_000.0;
    let finish_document_seconds = completed_epoch_seconds(
        timing.time_origin_millis,
        timing.dom_content_loaded_end_millis,
    );
    let finish_load_seconds =
        completed_epoch_seconds(timing.time_origin_millis, timing.load_event_end_millis);
    let navigation_type = chrome_navigation_type(&timing.navigation_type);

    let properties = [
        ("requestTime", v8::Number::new(scope, start_seconds).into()),
        (
            "startLoadTime",
            v8::Number::new(scope, start_seconds).into(),
        ),
        (
            "commitLoadTime",
            v8::Number::new(scope, start_seconds).into(),
        ),
        (
            "finishDocumentLoadTime",
            v8::Number::new(scope, finish_document_seconds).into(),
        ),
        (
            "finishLoadTime",
            v8::Number::new(scope, finish_load_seconds).into(),
        ),
        ("firstPaintTime", v8::Number::new(scope, 0.0).into()),
        (
            "firstPaintAfterLoadTime",
            v8::Number::new(scope, 0.0).into(),
        ),
        ("navigationType", v8str(scope, navigation_type).into()),
        ("wasFetchedViaSpdy", v8::Boolean::new(scope, false).into()),
        ("wasNpnNegotiated", v8::Boolean::new(scope, false).into()),
        ("npnNegotiatedProtocol", v8str(scope, "").into()),
        (
            "wasAlternateProtocolAvailable",
            v8::Boolean::new(scope, false).into(),
        ),
        ("connectionInfo", v8str(scope, "unknown").into()),
    ];
    for (name, value) in properties {
        if define_chrome_snapshot_property(scope, result, name, value).is_err() {
            return;
        }
    }
    rv.set(result.into());
}

fn chrome_csi<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_null();
    let Some(timing) = current_window_navigation_timing_snapshot(scope) else {
        return;
    };
    let result = v8::Object::new(scope);
    let onload_millis = completed_epoch_millis(
        timing.time_origin_millis,
        timing.dom_content_loaded_end_millis,
    );
    let transition = chrome_csi_transition(&timing.navigation_type);
    let properties = [
        (
            "startE",
            v8::Number::new(scope, timing.time_origin_millis).into(),
        ),
        ("onloadT", v8::Number::new(scope, onload_millis).into()),
        (
            "pageT",
            v8::Number::new(
                scope,
                dom_time_since_origin_millis(timing.time_origin_millis).max(0.0),
            )
            .into(),
        ),
        ("tran", v8::Integer::new(scope, transition).into()),
    ];
    for (name, value) in properties {
        if define_chrome_snapshot_property(scope, result, name, value).is_err() {
            return;
        }
    }
    rv.set(result.into());
}

fn completed_epoch_seconds(time_origin_millis: f64, relative_millis: Option<f64>) -> f64 {
    completed_epoch_millis(time_origin_millis, relative_millis) / 1_000.0
}

fn completed_epoch_millis(time_origin_millis: f64, relative_millis: Option<f64>) -> f64 {
    relative_millis
        .map(|relative| time_origin_millis + relative)
        .unwrap_or(0.0)
}

fn chrome_navigation_type(navigation_type: &str) -> &'static str {
    match navigation_type {
        "reload" => "Reload",
        "traverse" => "BackForward",
        _ => "Other",
    }
}

fn chrome_csi_transition(navigation_type: &str) -> i32 {
    match navigation_type {
        "reload" => 16,
        "traverse" => 6,
        _ => 15,
    }
}

fn reject_chrome_app_arguments(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    method: &'static str,
) -> bool {
    if args.length() == 0 {
        return false;
    }
    throw_type_error(scope, &format!("Error in invocation of app.{method}(): "));
    true
}

fn chrome_app_get_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if reject_chrome_app_arguments(scope, &args, "getDetails") {
        return;
    }
    rv.set_null();
}

fn chrome_app_get_is_installed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if reject_chrome_app_arguments(scope, &args, "getIsInstalled") {
        return;
    }
    rv.set(v8::Boolean::new(scope, false).into());
}

fn chrome_app_running_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if reject_chrome_app_arguments(scope, &args, "runningState") {
        return;
    }
    rv.set(v8str(scope, RUNNING_STATE_CANNOT_RUN).into());
}

fn chrome_app_install_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if args.length() == 0 {
        rv.set_undefined();
        return;
    }
    let callback = if args.length() == 1 {
        v8::Local::<v8::Function>::try_from(args.get(0)).ok()
    } else {
        None
    };
    let Some(callback) = callback else {
        throw_type_error(
            scope,
            "Error in invocation of app.installState(function callback): ",
        );
        return;
    };
    let trampoline = v8::Function::builder(run_chrome_app_install_state_callback)
        .data(callback.into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .expect("chrome.app.installState callback trampoline should allocate");
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.queue_timeout(
            scope,
            trampoline,
            0,
            HostTimerOwner::Window,
            Vec::new(),
        );
    }
    rv.set_undefined();
}

fn run_chrome_app_install_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let callback = v8::Local::<v8::Function>::try_from(args.data())
        .expect("chrome.app.installState trampoline must retain its callback");
    let receiver = v8::undefined(scope);
    let state: v8::Local<'s, v8::Value> = v8str(scope, INSTALL_STATE_NOT_INSTALLED).into();
    let _ = callback.call(scope, receiver.into(), &[state]);
    rv.set_undefined();
}

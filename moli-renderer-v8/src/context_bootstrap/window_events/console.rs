use super::*;
use crate::util::{get_private_value, v8str};

pub(in crate::context_bootstrap) fn console_log_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "log");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_info_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "info");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_warn_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "warn");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "error");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_debug_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "debug");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_trace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "trace");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_table_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "table");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_group_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "group");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_group_collapsed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "groupCollapsed");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_assert_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.get(0).boolean_value(scope) {
        rv.set_undefined();
        return;
    }
    append_and_forward_console_message(scope, &args, "assert");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_profile_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    call_original_console_method(scope, &args, "profile");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_profile_end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    call_original_console_method(scope, &args, "profileEnd");
    rv.set_undefined();
}

fn append_and_forward_console_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method_name: &'static str,
) {
    append_console_message(scope, args, method_name);
    call_original_console_method(scope, args, method_name);
}

fn call_original_console_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method_name: &'static str,
) {
    let global = scope.get_current_context().global(scope);
    let Some(original_console) = get_private_value(scope, global, WINDOW_ORIGINAL_CONSOLE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(method) = original_console
        .get(scope, v8str(scope, method_name).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };

    let mut forwarded_args = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        forwarded_args.push(args.get(index));
    }
    let suppress_page_stack_hook = forwarded_args.iter().any(|value| value.is_native_error())
        && error_prepare_stack_trace_is_function(scope);
    if suppress_page_stack_hook {
        scope.set_prepare_stack_trace_callback(inspector_console_stack_without_page_hook);
    }
    let _ = method.call(scope, original_console.into(), &forwarded_args);
    if suppress_page_stack_hook {
        scope.clear_prepare_stack_trace_callback();
    }
}

fn error_prepare_stack_trace_is_function(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "Error").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|error| error.get(scope, v8str(scope, "prepareStackTrace").into()))
        .is_some_and(|value| value.is_function())
}

fn inspector_console_stack_without_page_hook<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _error: v8::Local<'s, v8::Value>,
    _sites: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Value> {
    v8::String::empty(scope).into()
}

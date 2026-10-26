use super::*;
use crate::util::v8str;

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

pub(in crate::context_bootstrap) fn console_dir_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "dir");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn console_dirxml_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    append_and_forward_console_message(scope, &args, "dirxml");
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

pub(in crate::context_bootstrap) fn console_label_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((method, arguments)) = console_label_arguments(scope, &args) else {
        return;
    };
    forward_native_console_arguments(scope, &arguments, method, WINDOW_ORIGINAL_CONSOLE_SLOT);
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
    let mut forwarded_args = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        forwarded_args.push(args.get(index));
    }
    let method_name = v8str(scope, method_name);
    forward_native_console_arguments(
        scope,
        &forwarded_args,
        method_name,
        WINDOW_ORIGINAL_CONSOLE_SLOT,
    );
}

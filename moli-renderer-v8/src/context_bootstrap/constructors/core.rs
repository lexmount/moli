use super::super::media_queries::install_simple_event_target_methods;
use super::*;

pub(in crate::context_bootstrap) fn event_target_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'EventTarget': Please use the 'new' operator.",
        );
        return;
    }
    let target = args.this();
    install_simple_event_target_methods(scope, target, "__moliEventTargetListeners", false);
    rv.set(target.into());
}

pub(in crate::context_bootstrap) fn xpath_evaluator_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'XPathEvaluator': Please use the 'new' operator.",
        );
        return;
    }
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn illegal_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor");
}

pub(in crate::context_bootstrap) fn shell_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Constructor requires the 'new' operator.");
        return;
    }
    unsupported_constructor_callback(scope, args, rv);
}

pub(in crate::context_bootstrap) fn unsupported_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let exception = dom_exception_value(
        scope,
        "This constructor is not implemented yet.",
        "NotSupportedError",
    );
    scope.throw_exception(exception);
}

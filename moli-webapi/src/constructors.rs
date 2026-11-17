use crate::dom_exception::new_dom_exception_value as dom_exception_value;
use crate::event_target::initialize_simple_event_target;
use moli_v8_util::throw_type_error;
use moli_webapi_declare::WebApiObject;

pub fn event_target_constructor_callback<'s>(
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
    initialize_simple_event_target(scope, target);
    rv.set(target.into());
}

pub fn xpath_evaluator_constructor_callback<'s>(
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

pub fn illegal_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor");
}

pub fn shell_constructor_callback<'s>(
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

pub fn unsupported_constructor_callback(
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

pub fn xml_serializer_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'XMLSerializer': Please use the 'new' operator.",
        );
        return;
    }
    rv.set(args.this().into());
}

#[derive(Default, WebApiObject)]
#[webapi(interface = crate::interfaces::FileReaderSync)]
struct FileReaderSyncObjectDeclaration {}

pub fn file_reader_sync_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'FileReaderSync': Please use the 'new' operator.",
        );
        return;
    }
    FileReaderSyncObjectDeclaration::default()
        .initialize(scope, args.this())
        .expect("FileReaderSync declaration should initialize");
    rv.set(args.this().into());
}

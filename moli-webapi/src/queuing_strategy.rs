//! Count and byte-length strategies, including realm-local SameObject size functions.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value, throw_type_error, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

const COUNT_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT: &str = "__moliCountQueuingStrategySizeFunction";
const BYTE_LENGTH_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT: &str =
    "__moliByteLengthQueuingStrategySizeFunction";
const COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT: &str = "__moliCountQueuingStrategyHighWaterMark";
const BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT: &str =
    "__moliByteLengthQueuingStrategyHighWaterMark";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CountQueuingStrategy)]
struct CountQueuingStrategyObjectDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ByteLengthQueuingStrategy)]
struct ByteLengthQueuingStrategyObjectDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CountQueuingStrategy, enumerable)]
struct CountQueuingStrategyPrototypeDeclaration {
    #[webapi(accessor_property, getter = count_queuing_strategy_high_water_mark_getter)]
    high_water_mark: (),
    #[webapi(accessor_property, getter = count_queuing_strategy_size_getter)]
    size: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ByteLengthQueuingStrategy, enumerable)]
struct ByteLengthQueuingStrategyPrototypeDeclaration {
    #[webapi(accessor_property, getter = byte_length_queuing_strategy_high_water_mark_getter)]
    high_water_mark: (),
    #[webapi(accessor_property, getter = byte_length_queuing_strategy_size_getter)]
    size: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CountQueuingStrategy")]
struct CountQueuingStrategyConstructorArgs {
    #[webidl(required, dictionary)]
    init: QueuingStrategyInit<true>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ByteLengthQueuingStrategy")]
struct ByteLengthQueuingStrategyConstructorArgs {
    #[webidl(required, dictionary)]
    init: QueuingStrategyInit<false>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "QueuingStrategyInit", snapshot)]
struct QueuingStrategyInit<const COUNT: bool> {
    #[webidl(required, converter = "unrestricted_double")]
    #[webidl(snapshot_slot = if COUNT { COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT } else { BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT })]
    high_water_mark: f64,
}

pub fn count_queuing_strategy_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'CountQueuingStrategy': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<CountQueuingStrategyConstructorArgs>(scope, &args)
    else {
        return;
    };
    parsed
        .init
        .initialize_snapshot(scope, args.this())
        .expect("queuing strategy state");
    CountQueuingStrategyObjectDeclaration::new()
        .initialize(scope, args.this())
        .expect("CountQueuingStrategy declaration should initialize object");
    rv.set(args.this().into());
}

pub fn byte_length_queuing_strategy_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ByteLengthQueuingStrategy': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ByteLengthQueuingStrategyConstructorArgs>(scope, &args)
    else {
        return;
    };
    parsed
        .init
        .initialize_snapshot(scope, args.this())
        .expect("queuing strategy state");
    ByteLengthQueuingStrategyObjectDeclaration::new()
        .initialize(scope, args.this())
        .expect("ByteLengthQueuingStrategy declaration should initialize object");
    rv.set(args.this().into());
}

pub fn install_queuing_strategy_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "CountQueuingStrategy" => {
            CountQueuingStrategyPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "ByteLengthQueuingStrategy" => {
            ByteLengthQueuingStrategyPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn count_queuing_strategy_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if get_private_value(
        scope,
        args.this(),
        COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT,
    )
    .is_none()
    {
        throw_type_error(
            scope,
            "CountQueuingStrategy.size called on incompatible receiver",
        );
        return;
    }
    if let Some(function) = cached_count_queuing_strategy_size_function(scope) {
        rv.set(function.into());
        return;
    }
    rv.set_undefined();
}

fn byte_length_queuing_strategy_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if get_private_value(
        scope,
        args.this(),
        BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT,
    )
    .is_none()
    {
        throw_type_error(
            scope,
            "ByteLengthQueuingStrategy.size called on incompatible receiver",
        );
        return;
    }
    if let Some(function) = cached_byte_length_queuing_strategy_size_function(scope) {
        rv.set(function.into());
        return;
    }
    rv.set_undefined();
}

fn count_queuing_strategy_high_water_mark_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = get_private_value(
        scope,
        args.this(),
        COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT,
    ) else {
        throw_type_error(
            scope,
            "CountQueuingStrategy.highWaterMark called on incompatible receiver",
        );
        return;
    };
    rv.set(value);
}

fn byte_length_queuing_strategy_high_water_mark_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = get_private_value(
        scope,
        args.this(),
        BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT,
    ) else {
        throw_type_error(
            scope,
            "ByteLengthQueuingStrategy.highWaterMark called on incompatible receiver",
        );
        return;
    };
    rv.set(value);
}

fn cached_count_queuing_strategy_size_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    if let Some(function) =
        get_private_value(scope, global, COUNT_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        return Some(function);
    }
    let function = v8::Function::builder(count_queuing_strategy_size_callback)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    function.set_name(v8str(scope, "size"));
    set_private_value(
        scope,
        global,
        COUNT_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT,
        function.into(),
    );
    Some(function)
}

fn cached_byte_length_queuing_strategy_size_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    if let Some(function) = get_private_value(
        scope,
        global,
        BYTE_LENGTH_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        return Some(function);
    }
    let function = v8::Function::builder(byte_length_queuing_strategy_size_callback)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    function.set_name(v8str(scope, "size"));
    set_private_value(
        scope,
        global,
        BYTE_LENGTH_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT,
        function.into(),
    );
    Some(function)
}

fn count_queuing_strategy_size_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Number::new(scope, 1.0).into());
}

fn byte_length_queuing_strategy_size_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(chunk) = args.get(0).to_object(scope) else {
        return;
    };
    let Some(byte_length) = chunk.get(scope, v8str(scope, "byteLength").into()) else {
        rv.set_undefined();
        return;
    };
    rv.set(byte_length);
}

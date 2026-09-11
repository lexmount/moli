use super::*;
use crate::web_api_interfaces;
use crate::{
    util::{get_private_value, set_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const COUNT_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT: &str = "__moliCountQueuingStrategySizeFunction";
const BYTE_LENGTH_QUEUING_STRATEGY_SIZE_FUNCTION_SLOT: &str =
    "__moliByteLengthQueuingStrategySizeFunction";
const COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT: &str = "__moliCountQueuingStrategyHighWaterMark";
const BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT: &str =
    "__moliByteLengthQueuingStrategyHighWaterMark";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextEncoderStream)]
struct TextEncoderStreamObjectDeclaration {
    #[webapi(data_property)]
    encoding: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextDecoderStream)]
struct TextDecoderStreamObjectDeclaration {
    #[webapi(data_property)]
    encoding: String,
    #[webapi(data_property)]
    fatal: bool,
    #[webapi(data_property = "ignoreBOM")]
    ignore_bom: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CountQueuingStrategy)]
struct CountQueuingStrategyObjectDeclaration {
    #[webapi(slot = COUNT_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT)]
    high_water_mark: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ByteLengthQueuingStrategy)]
struct ByteLengthQueuingStrategyObjectDeclaration {
    #[webapi(slot = BYTE_LENGTH_QUEUING_STRATEGY_HIGH_WATER_MARK_SLOT)]
    high_water_mark: f64,
}

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
#[webidl(prefix = "ReadableStream")]
struct ReadableStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    source: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, with = parse_readable_stream_strategy_arg)]
    strategy: StreamQueuingStrategy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStream")]
struct WritableStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    sink: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, with = parse_writable_stream_strategy_arg)]
    strategy: StreamQueuingStrategy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TransformStream")]
struct TransformStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    transformer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, with = parse_transform_stream_writable_strategy_arg)]
    writable_strategy: StreamQueuingStrategy,
    #[webidl(index = 2, with = parse_transform_stream_readable_strategy_arg)]
    readable_strategy: StreamQueuingStrategy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoderStream")]
struct TextDecoderStreamConstructorArgs {
    #[webidl(default = "utf-8")]
    label: String,
    #[webidl(index = 1, with = parse_text_decoder_stream_options_arg)]
    options: TextDecoderStreamOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CountQueuingStrategy")]
struct CountQueuingStrategyConstructorArgs {
    #[webidl(required, with = parse_queuing_strategy_init_arg)]
    init: StreamQueuingStrategy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ByteLengthQueuingStrategy")]
struct ByteLengthQueuingStrategyConstructorArgs {
    #[webidl(required, with = parse_queuing_strategy_init_arg)]
    init: StreamQueuingStrategy,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecoderOptions")]
struct TextDecoderStreamOptions {
    #[webidl(default = false)]
    fatal: bool,
    #[webidl(name = "ignoreBOM", default = false)]
    ignore_bom: bool,
}

pub(in crate::context_bootstrap) fn readable_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ReadableStream': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let source = match parsed
        .source
        .map(|source| parse_readable_stream_source_object(scope, source))
        .transpose()
    {
        Ok(source) => source,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let byte_stream = source.as_ref().is_some_and(|source| source.byte_stream);
    if byte_stream && parsed.strategy.size.is_some() {
        throw_range_error(
            scope,
            "A byte ReadableStream cannot use a queuing strategy size function",
        );
        return;
    }
    if byte_stream
        && source
            .as_ref()
            .and_then(|source| source.auto_allocate_chunk_size)
            == Some(0)
    {
        throw_type_error(scope, "autoAllocateChunkSize must be greater than zero");
        return;
    }
    let high_water_mark = if byte_stream && !parsed.strategy.high_water_mark_provided {
        0.0
    } else {
        parsed.strategy.high_water_mark
    };
    initialize_webidl_readable_stream_object(
        scope,
        args.this(),
        source,
        byte_stream,
        high_water_mark,
        parsed.strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn writable_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WritableStream': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<WritableStreamConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let sink = match parsed
        .sink
        .map(|sink| parse_writable_stream_sink_object(scope, sink))
        .transpose()
    {
        Ok(sink) => sink,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    initialize_webidl_writable_stream_object(
        scope,
        args.this(),
        sink,
        parsed.strategy.high_water_mark,
        parsed.strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn transform_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'TransformStream': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<TransformStreamConstructorArgs<'s>>(scope, &args)
    else {
        return;
    };
    let transformer = match parsed
        .transformer
        .map(|transformer| parse_transform_stream_transformer_object(scope, transformer))
        .transpose()
    {
        Ok(transformer) => transformer,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    initialize_webidl_transform_stream_object(
        scope,
        args.this(),
        transformer,
        None,
        parsed.writable_strategy.high_water_mark,
        parsed.writable_strategy.size,
        parsed.readable_strategy.high_water_mark,
        parsed.readable_strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn text_encoder_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'TextEncoderStream': Please use the 'new' operator.",
        );
        return;
    }
    initialize_transform_stream_object(
        scope,
        args.this(),
        None,
        Some("text-encoder"),
        1.0,
        None,
        0.0,
        None,
    );
    TextEncoderStreamObjectDeclaration::new("utf-8")
        .initialize(scope, args.this())
        .expect("TextEncoderStream declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn text_decoder_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'TextDecoderStream': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<TextDecoderStreamConstructorArgs>(scope, &args) else {
        return;
    };
    let label = parsed.label.trim().to_ascii_lowercase();
    let Some(encoding) = super::super::text_decoder_encoding_for_label(&label) else {
        throw_range_error(
            scope,
            &format!(
                "Failed to construct 'TextDecoderStream': The encoding label provided ('{}') is invalid.",
                label
            ),
        );
        return;
    };
    let encoding_name = super::super::canonical_text_decoder_encoding_name(encoding);

    let fatal = parsed.options.fatal;
    let ignore_bom = parsed.options.ignore_bom;

    initialize_transform_stream_object(
        scope,
        args.this(),
        None,
        Some("text-decoder"),
        1.0,
        None,
        0.0,
        None,
    );
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(writable) = object_property_as_object(scope, args.this(), "writable")
    {
        unsafe { &mut *host_ptr }
            .text_codecs_mut()
            .init_decoder(scope, writable, encoding, fatal, ignore_bom);
    }
    TextDecoderStreamObjectDeclaration::new(encoding_name, fatal, ignore_bom)
        .initialize(scope, args.this())
        .expect("TextDecoderStream declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn count_queuing_strategy_constructor_callback<'s>(
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
    CountQueuingStrategyObjectDeclaration::new(parsed.init.high_water_mark)
        .initialize(scope, args.this())
        .expect("CountQueuingStrategy declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn byte_length_queuing_strategy_constructor_callback<'s>(
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
    ByteLengthQueuingStrategyObjectDeclaration::new(parsed.init.high_water_mark)
        .initialize(scope, args.this())
        .expect("ByteLengthQueuingStrategy declaration should initialize object");
    rv.set(args.this().into());
}

pub(super) fn install_queuing_strategy_template_bindings<'s>(
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

fn parse_readable_stream_strategy_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    parse_stream_strategy_arg(scope, args, index, "ReadableStream", 1.0)
}

fn parse_writable_stream_strategy_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    parse_stream_strategy_arg(scope, args, index, "WritableStream", 1.0)
}

fn parse_transform_stream_writable_strategy_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    parse_stream_strategy_arg(scope, args, index, "TransformStream", 1.0)
}

fn parse_transform_stream_readable_strategy_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    parse_stream_strategy_arg(scope, args, index, "TransformStream", 0.0)
}

fn parse_queuing_strategy_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<StreamQueuingStrategy, webidl::WebIdlError> {
    let context = webidl::Context::argument("QueuingStrategyInit", (index + 1) as usize);
    let object = webidl::dictionary_arg(args, index, context)?
        .ok_or_else(|| webidl::WebIdlError::missing_required(context))?;
    let high_water_mark = object
        .get(scope, v8str(scope, "highWaterMark").into())
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))?;
    if high_water_mark.is_undefined() {
        return Err(webidl::WebIdlError::custom_message(
            "QueuingStrategyInit.highWaterMark is required",
        ));
    }
    let high_water_mark = high_water_mark.number_value(scope).ok_or_else(|| {
        webidl::WebIdlError::custom_message("QueuingStrategyInit.highWaterMark must be a number")
    })?;
    Ok(StreamQueuingStrategy::without_size(high_water_mark))
}

fn parse_text_decoder_stream_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<TextDecoderStreamOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("TextDecoderStream", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

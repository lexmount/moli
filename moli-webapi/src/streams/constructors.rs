use super::*;
use crate::interfaces as web_api_interfaces;
use crate::text_decoder::init_text_decoder_state;
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

pub(super) const TEXT_ENCODER_STREAM_ENCODING_SLOT: &str = "__moliTextEncoderStreamEncoding";
pub(super) const TEXT_DECODER_STREAM_ENCODING_SLOT: &str = "__moliTextDecoderStreamEncoding";
pub(super) const TEXT_DECODER_STREAM_FATAL_SLOT: &str = "__moliTextDecoderStreamFatal";
pub(super) const TEXT_DECODER_STREAM_IGNORE_BOM_SLOT: &str = "__moliTextDecoderStreamIgnoreBOM";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextEncoderStream)]
struct TextEncoderStreamObjectDeclaration {
    #[webapi(slot = TEXT_ENCODER_STREAM_ENCODING_SLOT)]
    encoding: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextDecoderStream)]
struct TextDecoderStreamObjectDeclaration {
    #[webapi(slot = TEXT_DECODER_STREAM_ENCODING_SLOT)]
    encoding: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStream")]
struct ReadableStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    source: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, dictionary)]
    strategy: QueuingStrategyMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStream")]
struct WritableStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    sink: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, dictionary)]
    strategy: QueuingStrategyMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TransformStream")]
struct TransformStreamConstructorArgs<'s> {
    #[webidl(index = 0)]
    transformer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, dictionary)]
    writable_strategy: QueuingStrategyMembers,
}

// Writable strategy conversion and validation must finish before reading the
// readable strategy. Keep this second conversion stage ahead of transformer reads.
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TransformStream")]
struct TransformStreamReadableStrategyArgs {
    #[webidl(index = 2, dictionary)]
    readable_strategy: QueuingStrategyMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoderStream")]
struct TextDecoderStreamConstructorArgs {
    #[webidl(default = "utf-8")]
    label: String,
    #[webidl(index = 1, dictionary)]
    options: TextDecoderStreamOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecoderOptions", snapshot)]
struct TextDecoderStreamOptions {
    #[webidl(default = false)]
    #[webidl(snapshot_slot = TEXT_DECODER_STREAM_FATAL_SLOT)]
    fatal: bool,
    #[webidl(name = "ignoreBOM", default = false)]
    #[webidl(snapshot_slot = TEXT_DECODER_STREAM_IGNORE_BOM_SLOT)]
    ignore_bom: bool,
}

pub(in crate::streams) fn readable_stream_constructor_callback<'s>(
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
    let strategy =
        match parsed
            .strategy
            .resolve(scope, webidl::Context::argument("ReadableStream", 2), 1.0)
        {
            Ok(strategy) => strategy,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
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
    if byte_stream && strategy.size.is_some() {
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
    let high_water_mark = if byte_stream && !strategy.high_water_mark_provided {
        0.0
    } else {
        strategy.high_water_mark
    };
    initialize_webidl_readable_stream_object(
        scope,
        args.this(),
        source,
        byte_stream,
        high_water_mark,
        strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::streams) fn writable_stream_constructor_callback<'s>(
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
    let strategy =
        match parsed
            .strategy
            .resolve(scope, webidl::Context::argument("WritableStream", 2), 1.0)
        {
            Ok(strategy) => strategy,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
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
        strategy.high_water_mark,
        strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::streams) fn transform_stream_constructor_callback<'s>(
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
    let writable_strategy = match parsed.writable_strategy.resolve(
        scope,
        webidl::Context::argument("TransformStream", 2),
        1.0,
    ) {
        Ok(strategy) => strategy,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(readable) = webidl::parse_args::<TransformStreamReadableStrategyArgs>(scope, &args)
    else {
        return;
    };
    let readable_strategy = match readable.readable_strategy.resolve(
        scope,
        webidl::Context::argument("TransformStream", 3),
        0.0,
    ) {
        Ok(strategy) => strategy,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
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
        writable_strategy.high_water_mark,
        writable_strategy.size,
        readable_strategy.high_water_mark,
        readable_strategy.size,
    );
    rv.set(args.this().into());
}

pub(in crate::streams) fn text_encoder_stream_constructor_callback<'s>(
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

pub(in crate::streams) fn text_decoder_stream_constructor_callback<'s>(
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
    let label = parsed.label;
    let Some(encoding) = crate::text_decoder::text_decoder_encoding_for_label(&label) else {
        throw_range_error(
            scope,
            &format!(
                "Failed to construct 'TextDecoderStream': The encoding label provided ('{}') is invalid.",
                label
            ),
        );
        return;
    };
    let encoding_name = crate::text_decoder::canonical_text_decoder_encoding_name(encoding);

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
    let writable = stream_slot_object(scope, args.this(), TRANSFORM_STREAM_WRITABLE_SLOT)
        .expect("a new text decoder stream owns its writable endpoint");
    init_text_decoder_state(scope, writable, encoding, fatal, ignore_bom);
    TextDecoderStreamObjectDeclaration::new(encoding_name)
        .initialize(scope, args.this())
        .expect("TextDecoderStream declaration should initialize object");
    parsed
        .options
        .initialize_snapshot(scope, args.this())
        .expect("TextDecoderStream options");
    rv.set(args.this().into());
}

pub(in crate::streams) use crate::queuing_strategy::{
    byte_length_queuing_strategy_constructor_callback, count_queuing_strategy_constructor_callback,
    install_queuing_strategy_template_bindings,
};

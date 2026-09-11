//! Compression uses browser-owned TransformStream callbacks. Queueing,
//! backpressure, cancellation and terminal error propagation remain in the
//! shared Streams implementation; only the binary codec lives here.

use super::super::stream_adapter::{
    EnqueueChunkError, maybe_pull_stream, value_buffer_source_bytes,
};
use super::*;
use crate::web_api_interfaces;
use crate::{util::set_private_value, webidl};
use moli_v8_util::set_static_property;

mod codec;
mod state;

const TRANSFORM_SLOT: &str = "__moliCompressionTransform";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CompressionStream")]
struct ConstructorArgs {
    #[webidl(required)]
    format: String,
}

pub(in crate::context_bootstrap) fn compression_stream_constructor_callback<
    's,
    const DECOMPRESS: bool,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Compression streams must be constructed with 'new'");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(format) = codec::Format::parse(&parsed.format) else {
        throw_type_error(scope, "Unsupported compression format");
        return;
    };
    let transformer = v8::Object::new(scope);
    let _ = transformer.set_prototype(scope, v8::null(scope).into());
    state::initialize(scope, transformer, codec::Codec::new(format, DECOMPRESS));
    let transform = v8::Function::builder(transform_callback)
        .build(scope)
        .expect("native compression transform callback must allocate");
    let flush = v8::Function::builder(flush_callback)
        .build(scope)
        .expect("native compression flush callback must allocate");
    let cancel = v8::Function::builder(cancel_callback)
        .build(scope)
        .expect("native compression cancel callback must allocate");
    set_static_property(scope, transformer, "transform", transform.into());
    set_static_property(scope, transformer, "flush", flush.into());
    set_static_property(scope, transformer, "cancel", cancel.into());
    // The public wrapper is not itself a transferable TransformStream.
    let transform_stream = v8::Object::new(scope);
    initialize_transform_stream_object(
        scope,
        transform_stream,
        Some(transformer),
        None,
        1.0,
        None,
        0.0,
        None,
    );
    set_private_value(scope, args.this(), TRANSFORM_SLOT, transform_stream.into());
    let interface = if DECOMPRESS {
        "DecompressionStream"
    } else {
        "CompressionStream"
    };
    web_api_interfaces::initialize(scope, args.this(), interface)
        .expect("compression stream identity should initialize");
    rv.set(args.this().into());
}

fn transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let state = state::get(scope, args.this());
    // BufferSource conversion must not coerce arbitrary objects or read a
    // typed array's page-overridable properties.
    let value = args.get(0);
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value)
        .ok()
        .or_else(|| {
            v8::Local::<v8::ArrayBufferView>::try_from(value)
                .ok()?
                .buffer(scope)
        });
    let Some(bytes) = buffer
        .filter(|buffer| !buffer.was_detached() && !buffer.get_backing_store().is_shared())
        .and_then(|_| value_buffer_source_bytes(scope, value))
    else {
        state.borrow_mut().take();
        throw_type_error(scope, "Compression stream chunk must be a BufferSource");
        return;
    };
    let controller = v8::Local::<v8::Object>::try_from(args.get(1))
        .expect("native transform must receive a controller");
    process(scope, state, controller, &bytes, false);
}

fn flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let state = state::get(scope, args.this());
    let controller = v8::Local::<v8::Object>::try_from(args.get(0))
        .expect("native flush must receive a controller");
    process(scope, state, controller, &[], true);
}

fn cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    state::get(scope, args.this()).borrow_mut().take();
}

fn process<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: state::State,
    controller: v8::Local<'s, v8::Object>,
    bytes: &[u8],
    finish: bool,
) {
    let (chunks, result) = {
        let mut state = state.borrow_mut();
        let result = state
            .as_mut()
            .expect("active compression algorithm must own a codec")
            .process(bytes, finish);
        if finish || result.1.is_err() {
            state.take();
        }
        result
    };
    let readable = stream_slot_object(scope, controller, STREAM_CONTROLLER_STREAM_SLOT)
        .expect("transform controller must own a readable stream");
    for chunk in chunks {
        let value =
            new_uint8_array_from_bytes(scope, chunk).expect("compression output must allocate");
        if let Err(error) = enqueue_chunk(scope, readable, value.into()) {
            state.borrow_mut().take();
            match error {
                EnqueueChunkError::ClosedOrErrored => {
                    throw_type_error(scope, "Compression stream is closed or errored")
                }
                EnqueueChunkError::Strategy(error) => {
                    scope.throw_exception(error);
                }
            }
            return;
        }
        maybe_pull_stream(scope, readable);
    }
    if let Err(error) = result {
        throw_type_error(scope, error);
    }
}

fn endpoint_getter<'s, const DECOMPRESS: bool, const READABLE: bool>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let interface = if DECOMPRESS {
        "DecompressionStream"
    } else {
        "CompressionStream"
    };
    if !moli_webapi_declare::implements_interface(scope, args.this(), interface) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let slot = if READABLE {
        TRANSFORM_STREAM_READABLE_SLOT
    } else {
        TRANSFORM_STREAM_WRITABLE_SLOT
    };
    let transform = stream_slot_object(scope, args.this(), TRANSFORM_SLOT)
        .expect("compression wrapper must own a transform");
    rv.set(
        stream_slot_object(scope, transform, slot)
            .expect("compression stream must own its endpoints")
            .into(),
    );
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CompressionStream, enumerable)]
struct CompressionPrototype {
    #[webapi(accessor_property, getter = endpoint_getter::<false, true>)]
    readable: (),
    #[webapi(accessor_property, getter = endpoint_getter::<false, false>)]
    writable: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DecompressionStream, enumerable)]
struct DecompressionPrototype {
    #[webapi(accessor_property, getter = endpoint_getter::<true, true>)]
    readable: (),
    #[webapi(accessor_property, getter = endpoint_getter::<true, false>)]
    writable: (),
}

pub(super) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "CompressionStream" => {
            CompressionPrototype::initialize_prototype_template(scope, prototype)
        }
        "DecompressionStream" => {
            DecompressionPrototype::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("only compression interfaces use the compression installer"),
    }
}

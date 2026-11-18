use crate::interfaces as web_api_interfaces;
use adapter::{
    EnqueueChunkError, QueuingStrategyMembers, call_function_result, cancel_readable_stream,
    close_stream, enqueue_chunk, initialize_transform_stream_object,
    initialize_webidl_readable_stream_object, initialize_webidl_transform_stream_object,
    initialize_webidl_writable_stream_object, new_readable_stream_object,
    new_transform_stream_shell_object, parse_readable_stream_source_object,
    parse_transform_stream_transformer_object, parse_writable_stream_sink_object,
    readable_stream_byob_request_respond_callback,
    readable_stream_byob_request_respond_with_new_view_callback,
    readable_stream_byob_request_view_getter, readable_stream_is_byte_stream,
    readable_stream_locked, rejected_promise_value, set_resolved_promise, stream_slot_object,
    suppress_promise_unhandled_rejection, writable_stream_close_internal, writable_stream_locked,
    writable_stream_snapshot,
};
use moli_v8_util::{
    get_private_value, new_uint8_array_from_bytes, set_private_value, throw_range_error,
    throw_type_error, v8_string, v8str,
};
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webapi_declare::WebApiInterfaceDescriptor;
use moli_webidl as webidl;
use objects::{
    new_readable_stream_byob_reader_object, new_readable_stream_reader_object,
    new_writable_stream_writer_object, readable_stream_byob_reader_read_callback,
    readable_stream_reader_cancel_callback, readable_stream_reader_closed_getter,
    readable_stream_reader_read_callback, readable_stream_reader_release_lock_callback,
    release_readable_stream_reader, writable_stream_writer_abort_callback,
    writable_stream_writer_close_callback, writable_stream_writer_closed_getter,
    writable_stream_writer_desired_size_getter, writable_stream_writer_ready_getter,
    writable_stream_writer_release_lock_callback, writable_stream_writer_write_callback,
};
use slots::*;

pub mod adapter;
mod compression;
mod constructors;
mod from;
mod objects;
mod readable;
mod runtime;
pub mod slots;
mod writable;

pub fn tee_fetch_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    adapter::tee_readable_stream_with_cloned_branch(scope, stream).ok()
}

pub fn proxy_fetch_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let transform = new_transform_stream_shell_object(scope);
    initialize_transform_stream_object(scope, transform, None, None, 1.0, None, 0.0, None);
    let readable = stream_slot_object(scope, transform, TRANSFORM_STREAM_READABLE_SLOT)?;
    let writable = stream_slot_object(scope, transform, TRANSFORM_STREAM_WRITABLE_SLOT)?;
    // A proxy disturbs its input immediately, even when backpressure delays
    // the first source read.
    adapter::disturb_readable_stream(scope, stream);
    let pipe = readable::start_internal_readable_stream_pipe_to(scope, stream, writable)?;
    suppress_promise_unhandled_rejection(scope, pipe);
    Some(readable)
}

#[derive(Clone, Copy)]
enum StreamPrototypeInstaller {
    ReadableStream,
    WritableStream,
    DefaultReader,
    ByobReader,
    ByobRequest,
    DefaultWriter,
    Controller,
    QueuingStrategy,
    TransformFamily,
    Compression,
}

#[derive(Clone, Copy)]
pub struct StreamInterfaceSpec {
    pub interface: WebApiInterfaceDescriptor,
    constructor: Option<StreamConstructor>,
    prototype_installer: StreamPrototypeInstaller,
}

type StreamConstructor =
    for<'s, 'i> fn(&mut v8::PinScope<'s, 'i, ()>) -> v8::Local<'s, v8::FunctionTemplate>;

impl StreamInterfaceSpec {
    pub fn has_constructor(&self) -> bool {
        self.constructor.is_some()
    }
}

/// The single catalog for Web-visible Streams interfaces.
///
/// Every entry is exposed in each realm Moli currently supports. The
/// constructor registry, worker realm profiles, and prototype installer consume
/// this table, so those surfaces cannot drift independently.
const STREAM_INTERFACE_SPECS: &[StreamInterfaceSpec] = &[
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(readable_stream_constructor_callback)
                .length(0)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::ReadableStream,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableStreamDefaultReader::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(readable_stream_default_reader_constructor_callback)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::DefaultReader,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableStreamDefaultController::DESCRIPTOR,
        constructor: None,
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::WritableStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(writable_stream_constructor_callback)
                .length(0)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::WritableStream,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::WritableStreamDefaultWriter::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(writable_stream_default_writer_constructor_callback)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::DefaultWriter,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::WritableStreamDefaultController::DESCRIPTOR,
        constructor: None,
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::TransformStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(transform_stream_constructor_callback)
                .length(0)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::TransformStreamDefaultController::DESCRIPTOR,
        constructor: None,
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::TextEncoderStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(text_encoder_stream_constructor_callback)
                .length(0)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::TextDecoderStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(text_decoder_stream_constructor_callback)
                .length(0)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ByteLengthQueuingStrategy::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(byte_length_queuing_strategy_constructor_callback)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::QueuingStrategy,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::CountQueuingStrategy::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(count_queuing_strategy_constructor_callback)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::QueuingStrategy,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::CompressionStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(compression_stream_constructor_callback::<false>)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::Compression,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::DecompressionStream::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(compression_stream_constructor_callback::<true>)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::Compression,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableStreamBYOBReader::DESCRIPTOR,
        constructor: Some(|scope| {
            v8::FunctionTemplate::builder(readable_stream_byob_reader_constructor_callback)
                .length(1)
                .build(scope)
        }),
        prototype_installer: StreamPrototypeInstaller::ByobReader,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableStreamBYOBRequest::DESCRIPTOR,
        constructor: None,
        prototype_installer: StreamPrototypeInstaller::ByobRequest,
    },
    StreamInterfaceSpec {
        interface: web_api_interfaces::ReadableByteStreamController::DESCRIPTOR,
        constructor: None,
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
];

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ReadableStream", enumerable)]
struct ReadableStreamConstructorDeclaration {
    #[webapi(
        static_method = "from",
        length = 1,
        callback = from::readable_stream_from_callback
    )]
    from: (),
}

pub fn interface_specs() -> &'static [StreamInterfaceSpec] {
    STREAM_INTERFACE_SPECS
}

pub fn is_stream_interface(name: &str) -> bool {
    stream_interface_spec(name).is_some()
}

fn stream_interface_spec(name: &str) -> Option<&'static StreamInterfaceSpec> {
    STREAM_INTERFACE_SPECS
        .iter()
        .find(|spec| spec.interface.name() == name)
}

pub fn build_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    name: &str,
) -> Option<v8::Local<'s, v8::FunctionTemplate>> {
    stream_interface_spec(name)?
        .constructor
        .map(|constructor| constructor(scope))
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableStream, enumerable)]
struct ReadableStreamPrototypeDeclaration {
    #[webapi(method, length = 0, callback = readable_stream_get_reader_callback)]
    get_reader: (),
    #[webapi(method, length = 0, callback = readable_stream_cancel_callback)]
    cancel: (),
    #[webapi(method, length = 1, callback = readable_stream_pipe_through_callback)]
    pipe_through: (),
    #[webapi(method, length = 1, callback = readable_stream_pipe_to_callback)]
    pipe_to: (),
    #[webapi(method, length = 0, callback = readable_stream_tee_callback)]
    tee: (),
    #[webapi(accessor_property, getter = readable_stream_locked_getter)]
    locked: (),
    #[webapi(method, length = 0, callback = readable_stream_async_iterator_callback)]
    values: (),
    #[webapi(alias = "values", symbol = "asyncIterator")]
    async_iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WritableStream, enumerable, receiver)]
struct WritableStreamPrototypeDeclaration {
    #[webapi(method, length = 0, callback = writable_stream_get_writer_callback)]
    get_writer: (),
    #[webapi(method, returns_promise, length = 0, callback = writable_stream_abort_callback)]
    abort: (),
    #[webapi(method, returns_promise, length = 0, callback = writable_stream_close_callback)]
    close: (),
    #[webapi(accessor_property, getter = writable_stream_locked_getter)]
    locked: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableStreamDefaultReader, enumerable)]
struct ReadableStreamDefaultReaderPrototypeDeclaration {
    #[webapi(method, length = 0, callback = readable_stream_reader_read_callback)]
    read: (),
    #[webapi(method, length = 0, callback = readable_stream_reader_release_lock_callback)]
    release_lock: (),
    #[webapi(method, length = 0, callback = readable_stream_reader_cancel_callback)]
    cancel: (),
    #[webapi(accessor_property, getter = readable_stream_reader_closed_getter)]
    closed: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableStreamBYOBReader, enumerable)]
struct ReadableStreamByobReaderPrototypeDeclaration {
    #[webapi(method, length = 1, callback = readable_stream_byob_reader_read_callback)]
    read: (),
    #[webapi(method, length = 0, callback = readable_stream_reader_release_lock_callback)]
    release_lock: (),
    #[webapi(method, length = 0, callback = readable_stream_reader_cancel_callback)]
    cancel: (),
    #[webapi(accessor_property, getter = readable_stream_reader_closed_getter)]
    closed: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableStreamBYOBRequest, enumerable)]
struct ReadableStreamByobRequestPrototypeDeclaration {
    #[webapi(accessor_property, getter = readable_stream_byob_request_view_getter)]
    view: (),
    #[webapi(method, length = 1, callback = readable_stream_byob_request_respond_callback)]
    respond: (),
    #[webapi(
        method,
        length = 1,
        callback = readable_stream_byob_request_respond_with_new_view_callback
    )]
    respond_with_new_view: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WritableStreamDefaultWriter, enumerable)]
struct WritableStreamDefaultWriterPrototypeDeclaration {
    #[webapi(method, length = 0, callback = writable_stream_writer_write_callback)]
    write: (),
    #[webapi(method, length = 0, callback = writable_stream_writer_close_callback)]
    close: (),
    #[webapi(method, length = 0, callback = writable_stream_writer_abort_callback)]
    abort: (),
    #[webapi(method, length = 0, callback = writable_stream_writer_release_lock_callback)]
    release_lock: (),
    #[webapi(accessor_property, getter = writable_stream_writer_closed_getter)]
    closed: (),
    #[webapi(accessor_property, getter = writable_stream_writer_desired_size_getter)]
    desired_size: (),
    #[webapi(accessor_property, getter = writable_stream_writer_ready_getter)]
    ready: (),
}

use compression::compression_stream_constructor_callback;
use constructors::{
    byte_length_queuing_strategy_constructor_callback, count_queuing_strategy_constructor_callback,
    readable_stream_constructor_callback, text_decoder_stream_constructor_callback,
    text_encoder_stream_constructor_callback, transform_stream_constructor_callback,
    writable_stream_constructor_callback,
};
use objects::{
    readable_stream_byob_reader_constructor_callback,
    readable_stream_default_reader_constructor_callback,
    writable_stream_default_writer_constructor_callback,
};
pub use readable::{
    is_readable_stream_object, new_readable_stream_from_array_buffer,
    new_readable_stream_from_source, start_internal_readable_stream_pipe_to,
};
use readable::{
    readable_stream_async_iterator_callback, readable_stream_cancel_callback,
    readable_stream_get_reader_callback, readable_stream_locked_getter,
    readable_stream_pipe_through_callback, readable_stream_pipe_to_callback,
    readable_stream_tee_callback,
};
use writable::{
    writable_stream_abort_callback, writable_stream_close_callback,
    writable_stream_get_writer_callback, writable_stream_locked_getter,
};

pub fn is_writable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::WritableStream::is_instance(scope, object)
}

pub fn install_stream_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let Some(spec) = stream_interface_spec(interface_name) else {
        return;
    };
    let prototype = template.prototype_template(scope);
    match spec.prototype_installer {
        StreamPrototypeInstaller::ReadableStream => {
            ReadableStreamConstructorDeclaration::initialize_template(scope, template);
            ReadableStreamPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        StreamPrototypeInstaller::WritableStream => {
            WritableStreamPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        StreamPrototypeInstaller::DefaultReader => {
            ReadableStreamDefaultReaderPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        StreamPrototypeInstaller::ByobReader => {
            ReadableStreamByobReaderPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        StreamPrototypeInstaller::ByobRequest => {
            ReadableStreamByobRequestPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        StreamPrototypeInstaller::DefaultWriter => {
            WritableStreamDefaultWriterPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        StreamPrototypeInstaller::Controller => {
            objects::install_stream_controller_template_bindings(scope, prototype, interface_name);
        }
        StreamPrototypeInstaller::QueuingStrategy => {
            constructors::install_queuing_strategy_template_bindings(
                scope,
                prototype,
                interface_name,
            );
        }
        StreamPrototypeInstaller::TransformFamily => {
            writable::install_transform_stream_template_bindings(scope, prototype, interface_name);
        }
        StreamPrototypeInstaller::Compression => {
            compression::install_template_bindings(scope, prototype, interface_name);
        }
    }
}

use crate::abort_signal::ResolvedAbortSignal;
use runtime::{streams_owner_terminating, structured_clone_value};

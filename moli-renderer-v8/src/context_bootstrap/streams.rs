use super::specs::{ConstructorKind, ConstructorSpec};
use super::stream_adapter::{
    EnqueueChunkError, StreamQueuingStrategy, call_function_result, cancel_readable_stream,
    close_stream, enqueue_chunk, initialize_transform_stream_object,
    initialize_webidl_readable_stream_object, initialize_webidl_transform_stream_object,
    initialize_webidl_writable_stream_object, new_readable_stream_object,
    parse_readable_stream_source_object, parse_stream_strategy_arg,
    parse_transform_stream_transformer_object, parse_writable_stream_sink_object,
    readable_stream_byob_request_respond_callback,
    readable_stream_byob_request_respond_with_new_view_callback,
    readable_stream_byob_request_view_getter, readable_stream_is_byte_stream,
    readable_stream_locked, rejected_promise_value, set_resolved_promise, stream_slot_array,
    stream_slot_object, writable_stream_close_internal, writable_stream_locked,
    writable_stream_snapshot,
};
use super::stream_objects::{
    new_readable_stream_byob_reader_object, new_readable_stream_reader_object,
    new_writable_stream_writer_object, readable_stream_byob_reader_read_callback,
    readable_stream_reader_cancel_callback, readable_stream_reader_closed_getter,
    readable_stream_reader_read_callback, readable_stream_reader_release_lock_callback,
    release_readable_stream_reader, writable_stream_writer_abort_callback,
    writable_stream_writer_close_callback, writable_stream_writer_closed_getter,
    writable_stream_writer_desired_size_getter, writable_stream_writer_ready_getter,
    writable_stream_writer_release_lock_callback, writable_stream_writer_write_callback,
};
use super::*;
use moli_webapi_declare::WebApiFunctionTemplate;

mod constructors;
mod from;
mod readable;
mod transferable;
mod writable;

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
    None,
}

#[derive(Clone, Copy)]
struct StreamInterfaceSpec {
    constructor: ConstructorSpec,
    prototype_installer: StreamPrototypeInstaller,
}

/// The single catalog for Web-visible Streams interfaces.
///
/// Every entry is exposed in each realm Moli currently supports. The
/// constructor registry, worker realm profiles, and prototype installer consume
/// this table, so those surfaces cannot drift independently. `CompressionStream`
/// and `DecompressionStream` intentionally retain their existing
/// illegal-constructor behavior here; the catalog centralizes shape and exposure
/// without claiming their algorithms.
const STREAM_INTERFACE_SPECS: &[StreamInterfaceSpec] = &[
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableStream",
            parent: None,
            kind: ConstructorKind::ReadableStream,
        },
        prototype_installer: StreamPrototypeInstaller::ReadableStream,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableStreamDefaultReader",
            parent: None,
            kind: ConstructorKind::ReadableStreamDefaultReader,
        },
        prototype_installer: StreamPrototypeInstaller::DefaultReader,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableStreamDefaultController",
            parent: None,
            kind: ConstructorKind::ReadableStreamDefaultController,
        },
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "WritableStream",
            parent: None,
            kind: ConstructorKind::WritableStream,
        },
        prototype_installer: StreamPrototypeInstaller::WritableStream,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "WritableStreamDefaultWriter",
            parent: None,
            kind: ConstructorKind::WritableStreamDefaultWriter,
        },
        prototype_installer: StreamPrototypeInstaller::DefaultWriter,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "WritableStreamDefaultController",
            parent: None,
            kind: ConstructorKind::WritableStreamDefaultController,
        },
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "TransformStream",
            parent: None,
            kind: ConstructorKind::TransformStream,
        },
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "TransformStreamDefaultController",
            parent: None,
            kind: ConstructorKind::TransformStreamDefaultController,
        },
        prototype_installer: StreamPrototypeInstaller::Controller,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "TextEncoderStream",
            parent: None,
            kind: ConstructorKind::TextEncoderStream,
        },
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "TextDecoderStream",
            parent: None,
            kind: ConstructorKind::TextDecoderStream,
        },
        prototype_installer: StreamPrototypeInstaller::TransformFamily,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ByteLengthQueuingStrategy",
            parent: None,
            kind: ConstructorKind::ByteLengthQueuingStrategy,
        },
        prototype_installer: StreamPrototypeInstaller::QueuingStrategy,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "CountQueuingStrategy",
            parent: None,
            kind: ConstructorKind::CountQueuingStrategy,
        },
        prototype_installer: StreamPrototypeInstaller::QueuingStrategy,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "CompressionStream",
            parent: None,
            kind: ConstructorKind::Illegal,
        },
        prototype_installer: StreamPrototypeInstaller::None,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "DecompressionStream",
            parent: None,
            kind: ConstructorKind::Illegal,
        },
        prototype_installer: StreamPrototypeInstaller::None,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableStreamBYOBReader",
            parent: None,
            kind: ConstructorKind::ReadableStreamByobReader,
        },
        prototype_installer: StreamPrototypeInstaller::ByobReader,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableStreamBYOBRequest",
            parent: None,
            kind: ConstructorKind::Illegal,
        },
        prototype_installer: StreamPrototypeInstaller::ByobRequest,
    },
    StreamInterfaceSpec {
        constructor: ConstructorSpec {
            name: "ReadableByteStreamController",
            parent: None,
            kind: ConstructorKind::Illegal,
        },
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

pub(in crate::context_bootstrap) fn stream_constructor_specs()
-> impl Iterator<Item = ConstructorSpec> {
    STREAM_INTERFACE_SPECS.iter().map(|spec| spec.constructor)
}

#[cfg(test)]
pub(in crate::context_bootstrap) fn stream_interface_names() -> impl Iterator<Item = &'static str> {
    STREAM_INTERFACE_SPECS
        .iter()
        .map(|spec| spec.constructor.name)
}

pub(in crate::context_bootstrap) fn is_worker_exposed_stream_interface(name: &str) -> bool {
    stream_interface_spec(name).is_some()
}

fn stream_interface_spec(name: &str) -> Option<&'static StreamInterfaceSpec> {
    STREAM_INTERFACE_SPECS
        .iter()
        .find(|spec| spec.constructor.name == name)
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ReadableStream", enumerable)]
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
#[webapi(name = "WritableStream", enumerable)]
struct WritableStreamPrototypeDeclaration {
    #[webapi(method, length = 0, callback = writable_stream_get_writer_callback)]
    get_writer: (),
    #[webapi(method, length = 0, callback = writable_stream_abort_callback)]
    abort: (),
    #[webapi(method, length = 0, callback = writable_stream_close_callback)]
    close: (),
    #[webapi(accessor_property, getter = writable_stream_locked_getter)]
    locked: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ReadableStreamDefaultReader", enumerable)]
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
#[webapi(name = "ReadableStreamBYOBReader", enumerable)]
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
#[webapi(name = "ReadableStreamBYOBRequest", enumerable)]
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
#[webapi(name = "WritableStreamDefaultWriter", enumerable)]
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

pub(super) use super::stream_objects::{
    readable_stream_byob_reader_constructor_callback,
    readable_stream_default_reader_constructor_callback,
    writable_stream_default_writer_constructor_callback,
};
pub(super) use constructors::{
    byte_length_queuing_strategy_constructor_callback, count_queuing_strategy_constructor_callback,
    readable_stream_constructor_callback, text_decoder_stream_constructor_callback,
    text_encoder_stream_constructor_callback, transform_stream_constructor_callback,
    writable_stream_constructor_callback,
};
pub(crate) use readable::{
    is_readable_stream_object, new_readable_stream_from_array_buffer,
    new_readable_stream_from_source,
};
pub(super) use readable::{
    readable_stream_async_iterator_callback, readable_stream_cancel_callback,
    readable_stream_get_reader_callback, readable_stream_locked_getter,
    readable_stream_pipe_through_callback, readable_stream_pipe_to_callback,
    readable_stream_tee_callback,
};
pub(crate) use transferable::{
    ReadableStreamClonePayload, TransformStreamClonePayload, WritableStreamClonePayload,
    build_readable_stream_clone_shell, build_transform_stream_clone_shell,
    build_writable_stream_clone_shell, initialize_readable_stream_clone_shell,
    initialize_transform_stream_clone_shell, initialize_writable_stream_clone_shell,
    prepare_readable_stream_transfer, prepare_transform_stream_transfer,
    prepare_writable_stream_transfer,
};
pub(super) use writable::{
    writable_stream_abort_callback, writable_stream_close_callback,
    writable_stream_get_writer_callback, writable_stream_locked_getter,
};

pub(crate) fn is_writable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, object, WRITABLE_STREAM_STRATEGY_SLOT).is_some()
}

pub(crate) fn is_transform_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_object(scope, object, TRANSFORM_STREAM_READABLE_SLOT)
        .is_some_and(|readable| is_readable_stream_object(scope, readable))
        && stream_slot_object(scope, object, TRANSFORM_STREAM_WRITABLE_SLOT)
            .is_some_and(|writable| is_writable_stream_object(scope, writable))
}

pub(super) fn install_stream_template_bindings<'s>(
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
            super::stream_objects::install_stream_controller_template_bindings(
                scope,
                prototype,
                interface_name,
            );
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
        StreamPrototypeInstaller::None => {}
    }
}

#[cfg(test)]
mod catalog_tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn stream_interface_catalog_drives_registry_and_worker_exposure() {
        let catalog_names = stream_interface_names().collect::<Vec<_>>();
        let unique_names = catalog_names.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique_names.len(), catalog_names.len());
        assert!(
            catalog_names
                .iter()
                .all(|name| is_worker_exposed_stream_interface(name))
        );

        let registered_stream_names = crate::context_bootstrap::specs::constructor_specs()
            .into_iter()
            .filter(|spec| is_worker_exposed_stream_interface(spec.name))
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(registered_stream_names, catalog_names);
    }
}

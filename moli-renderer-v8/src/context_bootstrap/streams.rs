//! Realm exposure and MessagePort transfer for the shared Streams bindings.

use super::specs::{ConstructorKind, ConstructorSpec};
use super::*;
pub(super) use moli_webapi::streams::install_stream_template_bindings;
pub(in crate::context_bootstrap) use moli_webapi::streams::is_stream_interface as is_worker_exposed_stream_interface;
pub(crate) use moli_webapi::streams::{
    is_readable_stream_object, new_readable_stream_from_array_buffer,
    new_readable_stream_from_source, proxy_fetch_body_stream, tee_fetch_body_stream,
};

mod transferable;

pub(crate) use transferable::{
    ReadableStreamClonePayload, RequestBodyOwner, RequestBodyProducer, TransformStreamClonePayload,
    WritableStreamClonePayload, build_readable_stream_clone_shell,
    build_transform_stream_clone_shell, build_writable_stream_clone_shell,
    initialize_readable_stream_clone_shell, initialize_request_body_stream,
    initialize_transform_stream_clone_shell, initialize_writable_stream_clone_shell,
    install_request_body_abort, prepare_readable_stream_transfer,
    prepare_transform_stream_transfer, prepare_writable_stream_transfer, request_body_owner,
    transfer_request_body,
};

pub(in crate::context_bootstrap) fn stream_constructor_specs()
-> impl Iterator<Item = ConstructorSpec> {
    moli_webapi::streams::interface_specs()
        .iter()
        .map(|spec| ConstructorSpec {
            interface: spec.interface,
            kind: if spec.has_constructor() {
                ConstructorKind::Stream
            } else {
                ConstructorKind::Illegal
            },
        })
}

#[cfg(test)]
pub(super) fn stream_interface_names() -> impl Iterator<Item = &'static str> {
    moli_webapi::streams::interface_specs()
        .iter()
        .map(|spec| spec.interface.name())
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
            .filter(|spec| is_worker_exposed_stream_interface(spec.interface.name()))
            .map(|spec| spec.interface.name())
            .collect::<Vec<_>>();
        assert_eq!(registered_stream_names, catalog_names);
    }
}

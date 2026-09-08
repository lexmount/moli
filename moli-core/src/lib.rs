//! Stable facade crate for Moli.
//!
//! This crate is the public entry point for browser runtime APIs such as
//! `runtime`, `page`, `network`, and `protocol_types`, while the heavier
//! implementation now lives in the split internal crates.

pub mod browser;
mod config;
mod dom;
pub mod network;
pub mod page;
mod parser;
mod renderer;
pub mod runtime;
mod selector;
pub mod storage;
#[doc(hidden)]
pub mod testing;

pub use moli_page_types as protocol_types;
pub use moli_page_types::{LayoutPolicy, OptionalResourceFetchMask};
pub use moli_renderer_v8::renderer_output_transport_channel;
pub use moli_renderer_v8::{
    PageId, RendererBrowserContextRuntimeId, RendererDocumentLifecycleIdentity,
    RendererDocumentTitleChanged, RendererOutputCursor, RendererOutputFence,
    RendererOutputFenceLeaseId, RendererOutputItem, RendererOutputPublication,
    RendererOutputPublicationOrdering, RendererOutputRecord, RendererOutputResidenceIdentity,
    RendererOutputStreamCloseReason, RendererOutputStreamControl, RendererOutputStreamEpoch,
    RendererOutputStreamIdentity, RendererOutputTransportDiagnostics,
    RendererOutputTransportMessage, RendererOutputTransportReceiver,
    RendererOutputTransportSendError, RendererOutputTransportSender, RendererOwnerAction,
    RendererOwnerLocalHostId, RendererOwnerResourceActivitySource,
    RendererOwnerRuntimeActivitySource, RendererProtocolObservation,
    RendererRuntimeCommandCausalIdentity, RendererRuntimeInspectorAsyncCompletion,
    RendererRuntimeInspectorResponseChannel, RendererRuntimeInspectorResponseSender,
};

//! Renderer-neutral MessagePort registry.
//!
//! This crate owns the MessagePort state that does not need V8, JavaScript
//! wrappers, pages, workers, or task queues:
//!
//! - port id allocation;
//! - entangled peer tracking;
//! - pending message and close queues;
//! - transfer detach/attach owner state;
//! - cancellation of queued messages when their receiving endpoint closes.
//!
//! Payload and owner wake types are supplied by the embedding layer. The
//! renderer crate uses structured-clone bytes as the payload and page/worker
//! wake handles as the owner.

mod id;
mod registry;
mod wake;

pub use id::MessagePortId;
pub use registry::{
    DiscardedMessagePort, MessagePortCloseKind, MessagePortCloseResult, MessagePortRegistry,
};
pub use wake::MessagePortWake;

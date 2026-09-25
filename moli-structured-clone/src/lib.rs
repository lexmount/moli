//! Renderer-neutral structured clone wire payload types.
//!
//! This crate deliberately does not serialize or deserialize JavaScript values.
//! V8-specific structured clone work remains in `moli-renderer-v8`.
//! The types here are the transport payloads shared by renderer-neutral
//! services such as MessagePort and BroadcastChannel.

mod payload;
mod stored;

pub use payload::{StructuredCloneBytes, TransferredArrayBuffer};
pub use stored::SerializedScriptValue;

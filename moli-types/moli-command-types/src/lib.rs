//! Commands and immutable reply data shared by renderer-facing clients.
//!
//! This crate has no dependency on the renderer, V8, execution queues or
//! protocol adapters. Runtime-owned capabilities are supplied by associated
//! types; their implementations and lifetimes remain in the renderer.

mod runtime;
pub use runtime::{InspectorPauseEffect, RuntimeRealmInfo, RuntimeTaskMode};

#[cfg(feature = "devtools")]
pub mod devtools;

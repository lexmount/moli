//! SQLite connection policy and a thin Tokio runtime.
//!
//! Storage owners define their schemas, transactions and per-database FIFO.
//! Async tasks coordinate work; short blocking tasks own SQLite connections.
//! No JavaScript, profile layout, or quota policy belongs here.

mod connection;
mod runtime;

pub use connection::{
    ConnectionError, ConnectionOptions, ErrorClass, OpenMode, classify_error, open_disk,
    open_private_memory, require_schema_version,
};

pub use runtime::{Runtime, RuntimeError, RuntimeHandle};

pub mod writeback;

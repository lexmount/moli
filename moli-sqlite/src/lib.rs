//! SQLite connection policy and serial background execution.
//!
//! Storage owners define their schemas and transactions. They assign a stable
//! worker key to each database, and keep all access to its connection on that
//! worker. No JavaScript, profile layout, or quota policy belongs here.

mod connection;
mod executor;

pub use connection::{
    ConnectionError, ConnectionOptions, ErrorClass, OpenMode, classify_error, open_disk,
    open_private_memory, require_schema_version,
};
pub use executor::{Executor, ExecutorConfig, ExecutorError, ExecutorHandle};

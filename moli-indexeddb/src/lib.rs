//! A simple IndexedDB backend for Moli.
//!
//! This crate intentionally focuses on a small, renderer-agnostic storage core.
//! It does not expose JS objects or DOM events. The renderer is expected to
//! adapt request results into `IDB*` wrappers and event dispatch.

mod connection_lifecycle;
mod connection_notifications;
mod connection_queue;
pub use connection_lifecycle::DatabaseCloseSignal;
pub use connection_notifications::{
    ConnectionNotifications, VersionChangeBatch, VersionChangeCompletion,
    VersionChangeNotification, VersionChangeWake,
};
pub use connection_queue::{
    ConnectionRequestHandle, ConnectionRequestLease, ConnectionRequestQueues, ConnectionRequestWake,
};
mod cursor;
mod error;
mod key;
mod key_codec;
mod key_path_codec;
mod manager;
mod name;
mod options;
pub mod sqlite;
#[cfg(test)]
mod sqlite_schema_tests;
#[cfg(test)]
mod tests;
mod transaction_queue;
pub use transaction_queue::{
    TransactionRequestHandle, TransactionRequestLease, TransactionRequestQueues,
    TransactionRequestStatus, TransactionRequestWake,
};
mod types;
mod usage;

pub use cursor::{
    CursorDirection, apply_collection_direction, apply_cursor_direction_by_key,
    compare_cursor_direction, compare_cursor_tuple_direction,
};
pub use error::IndexedDbError;
pub use key::{Key, KeyNumber, KeyPath, MAX_KEY_DEPTH};
pub use manager::IndexedDbManager;
pub use name::IndexedDbName;
pub use options::{
    GetAllOptionsCandidate, IndexOptionsValidationError, ObjectStoreOptionsValidationError,
    TransactionModeParseError, parse_regular_transaction_mode, should_parse_get_all_options,
    validate_index_options, validate_object_store_options,
};
pub use types::{
    DatabaseHandle, IndexInfo, IndexOptions, IndexedDbExternalObject,
    IndexedDbFileSystemHandleBucket, IndexedDbFileSystemHandleKind, IndexedDbQuotaCheck,
    IndexedDbValue, KeyRange, ObjectStoreInfo, ObjectStoreOptions, RecordSnapshot,
    TransactionDurability, TransactionHandle, TransactionMode,
};

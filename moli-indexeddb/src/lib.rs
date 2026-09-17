//! A simple IndexedDB backend for Moli.
//!
//! This crate intentionally focuses on a small, renderer-agnostic storage core.
//! It does not expose JS objects or DOM events. The renderer is expected to
//! adapt request results into `IDB*` wrappers and event dispatch.

mod connection_notifications;
mod connection_queue;
mod cursor;
mod error;
mod key;
mod manager;
mod options;
mod persistence;
mod state;
#[cfg(test)]
mod tests;
mod transaction;
mod types;
mod usage;

pub use connection_notifications::{
    ConnectionNotifications, VersionChangeBatch, VersionChangeCompletion, VersionChangeNotification,
};
pub use connection_queue::{
    ConnectionRequestHandle, ConnectionRequestLease, ConnectionRequestQueues, ConnectionRequestWake,
};
pub use cursor::{
    CursorDirection, apply_collection_direction, apply_cursor_direction_by_key,
    compare_cursor_direction, compare_cursor_tuple_direction,
};
pub use error::IndexedDbError;
pub use key::{Key, KeyNumber, KeyPath};
pub use options::{
    GetAllOptionsCandidate, IndexOptionsValidationError, ObjectStoreOptionsValidationError,
    TransactionModeParseError, parse_regular_transaction_mode, should_parse_get_all_options,
    validate_index_options, validate_object_store_options,
};
pub use state::IndexedDbManager;
pub use types::{
    DatabaseHandle, DatabaseInfo, DatabaseNameAndVersion, IndexInfo, IndexOptions,
    IndexedDbExternalObject, IndexedDbFileSystemHandleBucket, IndexedDbFileSystemHandleKind,
    IndexedDbQuotaCheck, IndexedDbValue, ObjectStoreInfo, ObjectStoreOptions, OpenDisposition,
    OpenOptions, OpenResult, RequestOutcome, TransactionHandle, TransactionMode,
};

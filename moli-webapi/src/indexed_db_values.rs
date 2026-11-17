//! IndexedDB value conversion and bindings, without transaction or SQL state.
pub use moli_indexeddb_types::{IndexedDbError, Key, KeyRange as IdbKeyRangeQuery};
use moli_v8_util::{v8_string, v8str};
pub mod errors;
pub mod key;
pub mod range;
pub mod record;
pub use errors::{dom_exception_value, request_error_object};
pub use key::*;

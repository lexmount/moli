//! Storage-independent IndexedDB value models shared by bindings and SQL storage.
mod cursor;
pub use cursor::CursorDirection;
mod error;
mod key;
mod key_codec;
mod key_path_codec;
mod range;
#[cfg(feature = "sqlite-errors")]
mod sqlite_error;
pub use error::IndexedDbError;
pub use key::{Key, KeyNumber, KeyPath, MAX_KEY_DEPTH};
pub use range::KeyRange;

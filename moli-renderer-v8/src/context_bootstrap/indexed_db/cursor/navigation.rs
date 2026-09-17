use super::*;

mod advance;
mod compare;
mod primary_key;
mod result;
mod scan;

enum CursorIteration {
    Advance(u32),
    Continue(Option<Key>),
    ContinuePrimaryKey(Key, Key),
}

pub(in crate::context_bootstrap::indexed_db) use self::advance::{
    idb_cursor_advance_callback, idb_cursor_continue_callback,
};
pub(in crate::context_bootstrap::indexed_db) use self::primary_key::idb_cursor_continue_primary_key_callback;

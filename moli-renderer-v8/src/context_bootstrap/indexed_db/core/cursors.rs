use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbCursorOpenOperation, IndexedDbCursorSnapshot, indexed_db_cursor_state,
    set_indexed_db_cursor_position,
};

mod accessors;
mod direction;
mod request;
mod surface;

const SOURCE: &str = "__moli_idb_cursor_source";
const REQUEST: &str = "__moli_idb_cursor_request";
const KEY_CACHE: &str = "__moli_idb_cursor_key_cache";
const PRIMARY_KEY_CACHE: &str = "__moli_idb_cursor_primary_key_cache";
const VALUE_CACHE: &str = "__moli_idb_cursor_value_cache";

pub(in crate::context_bootstrap::indexed_db) use self::accessors::{
    cursor_request, cursor_source, idb_cursor_direction_getter, idb_cursor_key_getter,
    idb_cursor_primary_key_getter, idb_cursor_request_getter, idb_cursor_source_getter,
    idb_cursor_value_getter,
};

pub(in crate::context_bootstrap::indexed_db) use self::direction::{
    apply_cursor_direction, apply_index_collection_direction,
    apply_object_store_collection_direction, cursor_direction_from_cursor, parse_cursor_direction,
    parse_cursor_direction_with_context,
};
pub(in crate::context_bootstrap::indexed_db) use self::request::prepare_cursor_request;
pub(in crate::context_bootstrap::indexed_db) use self::surface::{
    materialize_cursor_result_in_request_realm, refresh_cursor_surface,
};

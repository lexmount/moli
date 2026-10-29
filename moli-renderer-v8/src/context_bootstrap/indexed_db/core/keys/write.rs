use super::*;

mod extraction;
mod injection;
mod prepare;

use self::injection::can_inject_key_path_into_value;
pub(in crate::context_bootstrap::indexed_db) use self::injection::inject_key_path_into_value;

pub(in crate::context_bootstrap::indexed_db) use self::extraction::{
    extract_index_keys_budgeted, extract_index_keys_from_value, extract_object_store_key_from_value,
};
pub(in crate::context_bootstrap::indexed_db) use self::prepare::prepare_object_store_write;

use super::*;

mod collection;
mod count;
mod single;

pub(in crate::context_bootstrap::indexed_db) use self::collection::execute_index_get_all_request;
pub(in crate::context_bootstrap::indexed_db) use self::count::execute_index_count_request;
pub(in crate::context_bootstrap::indexed_db) use self::single::{
    execute_index_get_key_request, execute_index_get_request,
};

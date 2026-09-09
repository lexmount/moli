use super::*;

mod delete;
mod open;

pub(in crate::context_bootstrap::indexed_db) use self::delete::execute_delete_database_request;
pub(in crate::context_bootstrap::indexed_db) use self::open::execute_open_request;

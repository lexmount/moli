use super::*;

mod accessors;
mod object;
mod parse;
mod predicate;

const LOWER: &str = "__moli_idb_key_range_lower";
const UPPER: &str = "__moli_idb_key_range_upper";
const LOWER_OPEN: &str = "__moli_idb_key_range_lower_open";
const UPPER_OPEN: &str = "__moli_idb_key_range_upper_open";

pub(in crate::context_bootstrap::indexed_db) use self::accessors::{
    idb_key_range_lower_getter, idb_key_range_lower_open_getter, idb_key_range_upper_getter,
    idb_key_range_upper_open_getter,
};
pub(in crate::context_bootstrap::indexed_db) use self::object::create_key_range_object;
pub(in crate::context_bootstrap::indexed_db) use self::parse::{
    parse_key_or_range, parse_key_range_from_value,
};
pub(in crate::context_bootstrap::indexed_db) use self::predicate::key_in_range;

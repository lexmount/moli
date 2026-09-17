use super::*;

mod accessors;
mod object;
mod parse;
mod predicate;

const LOWER: &str = "__moli_idb_key_range_lower";
const UPPER: &str = "__moli_idb_key_range_upper";
const LOWER_VALUE: &str = "__moli_idb_key_range_lower_value";
const UPPER_VALUE: &str = "__moli_idb_key_range_upper_value";

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

/// Range operations preserve author exceptions produced by key conversion.
pub(in crate::context_bootstrap::indexed_db) fn convert_key_range_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    invalid_message: &str,
) -> Option<Key> {
    let result = {
        v8::tc_scope!(let conversion, scope);
        let result = parse_idb_key(conversion, value);
        if conversion.has_caught() {
            conversion.rethrow();
            return None;
        }
        result
    };
    match result {
        Ok(Some(key)) => Some(key),
        _ => {
            let error = crate::context_bootstrap::indexed_db::dom_exception_value(
                scope,
                invalid_message,
                "DataError",
            );
            scope.throw_exception(error);
            None
        }
    }
}

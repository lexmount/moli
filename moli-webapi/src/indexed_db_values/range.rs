use super::*;
use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;

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

pub use self::accessors::{
    idb_key_range_lower_getter, idb_key_range_lower_open_getter, idb_key_range_upper_getter,
    idb_key_range_upper_open_getter,
};
pub use self::object::create_key_range_object;
pub use self::parse::{parse_key_or_range, parse_key_range_from_value};
pub use self::predicate::key_in_range;

/// Range operations preserve author exceptions produced by key conversion.
pub fn convert_key_range_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    invalid_message: &str,
) -> Option<Key> {
    match parse_idb_key(scope, value) {
        Ok(Some(key)) => Some(key),
        Err(
            error @ (KeyConversionError::Exception(_)
            | KeyConversionError::Terminated
            | KeyConversionError::Backend(_)),
        ) => {
            error.throw(scope);
            None
        }
        Ok(None) | Err(KeyConversionError::Invalid(_)) => {
            let error =
                crate::indexed_db_values::dom_exception_value(scope, invalid_message, "DataError");
            scope.throw_exception(error);
            None
        }
    }
}

mod constructors;
mod instance;
pub use self::constructors::{
    idb_key_range_bound_callback, idb_key_range_lower_bound_callback, idb_key_range_only_callback,
    idb_key_range_upper_bound_callback,
};
pub use self::instance::idb_key_range_includes_callback;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable)]
struct IdbKeyRangePrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_key_range_lower_getter)]
    lower: (),
    #[webapi(accessor_property, getter = idb_key_range_upper_getter)]
    upper: (),
    #[webapi(accessor_property, getter = idb_key_range_lower_open_getter)]
    lower_open: (),
    #[webapi(accessor_property, getter = idb_key_range_upper_open_getter)]
    upper_open: (),
    #[webapi(method, length = 1, callback = idb_key_range_includes_callback)]
    includes: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable)]
struct IdbKeyRangeConstructorDeclaration {
    #[webapi(static_method, length = 1, callback = idb_key_range_only_callback)]
    only: (),
    #[webapi(static_method, length = 2, callback = idb_key_range_bound_callback)]
    bound: (),
    #[webapi(
        static_method,
        length = 1,
        callback = idb_key_range_lower_bound_callback
    )]
    lower_bound: (),
    #[webapi(
        static_method,
        length = 1,
        callback = idb_key_range_upper_bound_callback
    )]
    upper_bound: (),
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    IdbKeyRangeConstructorDeclaration::initialize_template(scope, template);
    let prototype = template.prototype_template(scope);
    IdbKeyRangePrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

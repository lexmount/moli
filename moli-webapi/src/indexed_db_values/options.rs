use super::key_path::parse_optional_idb_key_path_member;
use moli_indexeddb_types::{CursorDirection, KeyPath};
use moli_webidl as webidl;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "IDBTransactionMode")]
pub enum TransactionModeWebIdl {
    Readonly,
    Readwrite,
    Versionchange,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBObjectStoreParameters")]
pub struct IdbObjectStoreParameters {
    #[webidl(default = false)]
    pub auto_increment: bool,
    #[webidl(with = parse_optional_idb_key_path_member)]
    pub key_path: Option<KeyPath>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBIndexParameters")]
pub struct IdbIndexParameters {
    #[webidl(default = false)]
    pub unique: bool,
    #[webidl(default = false)]
    pub multi_entry: bool,
}

struct CursorDirectionValue(CursorDirection);

impl webidl::WebIdlEnum for CursorDirectionValue {
    const NAME: &'static str = "IDBCursorDirection";

    fn parse_token(value: &str) -> Option<Self> {
        CursorDirection::parse(value).map(Self)
    }
}

pub fn parse_cursor_direction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation_name: &'static str,
) -> std::result::Result<CursorDirection, webidl::WebIdlError> {
    parse_cursor_direction_with_context(scope, value, webidl::Context::argument(operation_name, 2))
}

fn parse_cursor_direction_with_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> std::result::Result<CursorDirection, webidl::WebIdlError> {
    if value.is_undefined() {
        return Ok(CursorDirection::default_next());
    }
    webidl::convert::<webidl::EnumValue<CursorDirectionValue>>(scope, value, context)
        .map(|webidl::EnumValue(CursorDirectionValue(direction))| direction)
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBGetAllOptions")]
pub struct GetAllOptions<'s> {
    // WebIDL dictionary members are evaluated in lexicographic order.
    #[webidl(converter = "enforce_range_unsigned_long")]
    pub count: Option<u32>,
    #[webidl(with = parse_direction_member)]
    pub direction: CursorDirection,
    pub query: Option<v8::Local<'s, v8::Value>>,
}

fn parse_direction_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<CursorDirection, webidl::WebIdlError> {
    let context = webidl::Context::member("IDBGetAllOptions", name);
    let value = webidl::property_result(scope, object, name, context)?
        .unwrap_or_else(|| v8::undefined(scope).into());
    parse_cursor_direction_with_context(scope, value, context)
}

pub fn parse_get_all_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Result<GetAllOptions<'s>, webidl::WebIdlError> {
    match webidl::dictionary_value(value, webidl::Context::argument(operation, 1))? {
        Some(object) => webidl::parse_dictionary_object(scope, object),
        None => Ok(GetAllOptions {
            count: None,
            direction: CursorDirection::default_next(),
            query: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{CursorDirection, CursorDirectionValue};
    use moli_webidl::WebIdlEnum;

    #[test]
    fn cursor_direction_labels_are_spec_strings() {
        let cases = [
            ("next", CursorDirection::Next),
            ("nextunique", CursorDirection::NextUnique),
            ("prev", CursorDirection::Prev),
            ("prevunique", CursorDirection::PrevUnique),
        ];

        for (label, direction) in cases {
            assert_eq!(
                CursorDirectionValue::parse_token(label).map(|value| value.0),
                Some(direction)
            );
            assert_eq!(direction.as_str(), label);
        }
        assert!(CursorDirectionValue::parse_token("forward").is_none());
    }
}

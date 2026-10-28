use super::*;
use crate::webidl;

#[derive(Clone, Copy, Debug, PartialEq, Eq, webidl::WebIdlEnum)]
#[webidl(name = "IDBCursorDirection")]
enum CursorDirectionWebIdl {
    Next,
    NextUnique,
    Prev,
    PrevUnique,
}

impl From<CursorDirectionWebIdl> for CursorDirection {
    fn from(value: CursorDirectionWebIdl) -> Self {
        match value {
            CursorDirectionWebIdl::Next => Self::Next,
            CursorDirectionWebIdl::NextUnique => Self::NextUnique,
            CursorDirectionWebIdl::Prev => Self::Prev,
            CursorDirectionWebIdl::PrevUnique => Self::PrevUnique,
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn parse_cursor_direction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation_name: &'static str,
) -> std::result::Result<CursorDirection, webidl::WebIdlError> {
    parse_cursor_direction_with_context(scope, value, webidl::Context::argument(operation_name, 2))
}

pub(in crate::context_bootstrap::indexed_db) fn parse_cursor_direction_with_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> std::result::Result<CursorDirection, webidl::WebIdlError> {
    if value.is_undefined() {
        return Ok(CursorDirection::default_next());
    }
    webidl::convert::<webidl::EnumValue<CursorDirectionWebIdl>>(scope, value, context)
        .map(|direction| direction.0.into())
}

pub(in crate::context_bootstrap::indexed_db) fn apply_cursor_direction(
    entries: Vec<CursorSnapshotEntry>,
    direction: CursorDirection,
) -> Vec<CursorSnapshotEntry> {
    moli_indexeddb::apply_cursor_direction_by_key(entries, direction, |entry| &entry.key)
}

pub(in crate::context_bootstrap::indexed_db) fn apply_object_store_collection_direction(
    entries: Vec<(Key, IndexedDbValue)>,
    direction: CursorDirection,
) -> Vec<(Key, IndexedDbValue)> {
    moli_indexeddb::apply_collection_direction(entries, direction)
}

pub(in crate::context_bootstrap::indexed_db) fn apply_index_collection_direction(
    entries: Vec<IndexEntry>,
    direction: CursorDirection,
) -> Vec<IndexEntry> {
    moli_indexeddb::apply_cursor_direction_by_key(entries, direction, |entry| &entry.index_key)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_direction_from_cursor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> CursorDirection {
    indexed_db_cursor_state(scope, cursor)
        .map(|state| state.direction)
        .unwrap_or_else(CursorDirection::default_next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webidl::WebIdlEnum;

    #[test]
    fn cursor_direction_labels_are_spec_strings() {
        let cases = [
            ("next", CursorDirectionWebIdl::Next),
            ("nextunique", CursorDirectionWebIdl::NextUnique),
            ("prev", CursorDirectionWebIdl::Prev),
            ("prevunique", CursorDirectionWebIdl::PrevUnique),
        ];

        for (label, direction) in cases {
            assert_eq!(CursorDirectionWebIdl::parse_token(label), Some(direction));
            assert_eq!(CursorDirection::from(direction).as_str(), label);
        }
        assert_eq!(CursorDirectionWebIdl::parse_token("forward"), None);
    }
}

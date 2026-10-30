use super::*;

pub(in crate::context_bootstrap::indexed_db) fn key_in_range(
    key: &Key,
    range: &IdbKeyRangeQuery,
) -> bool {
    range.contains(key)
}

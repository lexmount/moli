use super::*;
use crate::context_bootstrap::indexed_db::IdbKeyRangeQuery;

pub(in crate::context_bootstrap::indexed_db::tasks::operations) fn parse_collection_query_and_count<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    query_value: v8::Local<'s, v8::Value>,
    count_value: v8::Local<'s, v8::Value>,
    operation_name: &'static str,
) -> Option<(Option<IdbKeyRangeQuery>, Option<usize>)> {
    let query = match parse_key_or_range(scope, query_value) {
        Ok(query) => query,
        Err(error) => {
            let value = error.into_value(scope);
            store_request_error(scope, request, value);
            return None;
        }
    };
    let count =
        match parse_optional_count(scope, count_value, operation_count_prefix(operation_name)) {
            Ok(count) => count,
            Err(error) => {
                let message = error_message_or_fallback(&error);
                let message = v8_string(scope, &message)?;
                let error = v8::Exception::type_error(scope, message);
                store_request_error(scope, request, error);
                return None;
            }
        };
    Some((query, count))
}

fn operation_count_prefix(operation_name: &str) -> &'static str {
    match operation_name {
        "getAllKeys" | "index.getAllKeys" => "IndexedDB.getAllKeys",
        _ => "IndexedDB.getAll",
    }
}

fn error_message_or_fallback(error: &crate::webidl::WebIdlError) -> String {
    if error.is_pending_exception() {
        "Failed to execute IndexedDB collection operation: invalid count.".to_owned()
    } else {
        error.to_string()
    }
}

use super::*;

fn backend_error_name(error: &IndexedDbError) -> &'static str {
    match error {
        IndexedDbError::Constraint(_) => "ConstraintError",
        IndexedDbError::InvalidAccess(_) => "InvalidAccessError",
        IndexedDbError::InvalidState(message)
            if message == "StorageBucket IndexedDB bucket is no longer current" =>
        {
            "UnknownError"
        }
        IndexedDbError::InvalidState(_) => "InvalidStateError",
        IndexedDbError::NotFound(_) => "NotFoundError",
        IndexedDbError::QuotaExceeded { .. } => "QuotaExceededError",
        IndexedDbError::ReadOnly(_) => "ReadOnlyError",
        IndexedDbError::Syntax(_) => "SyntaxError",
        IndexedDbError::TransactionInactive(_) => "TransactionInactiveError",
        IndexedDbError::Version(_) => "VersionError",
        IndexedDbError::Io(_)
        | IndexedDbError::Corruption(_)
        | IndexedDbError::Serialization(_) => "UnknownError",
    }
}

pub(in crate::context_bootstrap::indexed_db) fn dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::new_dom_exception_value(scope, message, name)
}

pub(in crate::context_bootstrap::indexed_db) fn request_error_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: &IndexedDbError,
) -> v8::Local<'s, v8::Value> {
    if let IndexedDbError::QuotaExceeded { quota, requested } = error {
        return crate::context_bootstrap::new_quota_exceeded_error_value(
            scope,
            &error.to_string(),
            Some(*quota as f64),
            Some(*requested as f64),
        );
    }
    dom_exception_value(scope, &error.to_string(), backend_error_name(error))
}

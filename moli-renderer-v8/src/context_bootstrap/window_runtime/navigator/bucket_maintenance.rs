use super::*;
use moli_storage_service::{
    SqlBucketOperationResult, SqlBucketOperationValue, prepare_sql_bucket_maintenance,
};

pub(super) fn dispatch_sql_bucket_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    operation: SqlBucketOperation,
) -> bool {
    let Some(store) = current_storage_bucket_store(scope) else {
        return false;
    };
    match prepare_sql_bucket_maintenance(&store, operation) {
        Ok(Some(operation)) => {
            crate::context_bootstrap::opfs::dispatch_storage_bucket_operation(
                scope, resolver, operation,
            );
            true
        }
        Ok(None) => false,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            true
        }
    }
}

/// Expired handles fail immediately; accepted cleanup survives owner retirement.
/// Opening the same name runs later on this same maintenance sequence.
pub(super) fn dispatch_sql_expired_bucket_cleanup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &StorageBucketIdentity,
) -> bool {
    let Some(store) = current_storage_bucket_store(scope) else {
        return false;
    };
    match prepare_sql_bucket_maintenance(
        &store,
        SqlBucketOperation::Expire {
            identity: identity.clone(),
        },
    ) {
        Ok(Some(operation)) => {
            if let Err(error) = operation.dispatch(|_| {}) {
                tracing::warn!(error = %error, "failed to schedule expired storage bucket cleanup");
            }
            true
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(error = %error, "failed to schedule expired storage bucket cleanup");
            true
        }
    }
}

pub(in crate::context_bootstrap) fn settle_storage_bucket_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: SqlBucketOperationResult,
) {
    match &result.result {
        Ok(SqlBucketOperationValue::Opened(identity)) => {
            let bucket = build_storage_bucket_object(scope, identity.clone());
            let _ = resolver.resolve(scope, bucket.into());
        }
        Ok(SqlBucketOperationValue::Keys { names, .. }) => {
            let array = strings_to_array(scope, names);
            let _ = resolver.resolve(scope, array.into());
        }
        Ok(SqlBucketOperationValue::Complete) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Ok(SqlBucketOperationValue::Stale) => {
            reject_storage_bucket_unknown_error(scope, resolver, "setExpires");
        }
        Err(error) => reject_type_error(scope, resolver, error),
    }
}

use super::*;
use crate::context_bootstrap::storage_buckets::with_storage_bucket_store_entry;
use crate::webidl;
use moli_indexeddb::TransactionDurability;
use moli_storage_service::{StorageBucketDurability, StorageBucketStore};

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "IDBTransactionDurability")]
pub(super) enum IdbTransactionDurability {
    #[default]
    Default,
    Strict,
    Relaxed,
}

impl IdbTransactionDurability {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Strict => "strict",
            Self::Relaxed => "relaxed",
        }
    }
}

pub(super) fn idb_transaction_durability_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("transaction receiver was validated");
    let hint = indexed_db_transaction_durability(scope, receiver)
        .expect("branded transaction retains its durability hint");
    rv.set(v8str(scope, hint.label()).into());
}

pub(super) fn resolve_transaction_durability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    hint: IdbTransactionDurability,
) -> Result<TransactionDurability, IndexedDbError> {
    match hint {
        IdbTransactionDurability::Strict => Ok(TransactionDurability::Strict),
        IdbTransactionDurability::Relaxed => Ok(TransactionDurability::Relaxed),
        IdbTransactionDurability::Default => {
            let storage_scope = indexed_db_typed_storage_scope(scope, database)
                .expect("database has its native storage scope");
            // Resolve before native admission, as Chromium's CreateTransaction
            // does. A waiting writer must not reread bucket policy at COMMIT.
            with_storage_bucket_store_entry(scope, |store| {
                bucket_transaction_durability(store, &storage_scope)
            })
            .unwrap_or_else(|| {
                if storage_scope.bucket_identity().is_none() {
                    Ok(TransactionDurability::Relaxed)
                } else {
                    Err(missing_bucket_policy())
                }
            })
        }
    }
}

fn bucket_transaction_durability(
    store: &StorageBucketStore,
    storage_scope: &IndexedDbStorageScope,
) -> Result<TransactionDurability, IndexedDbError> {
    let policy = if let Some(identity) = storage_scope.bucket_identity() {
        store
            .bucket_durability_for_identity(identity)
            .ok_or_else(missing_bucket_policy)?
    } else {
        // A bucket named "default" is independent of the implicit default.
        store
            .bucket_durability(
                storage_scope.storage_key(),
                moli_storage_service::IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
            )
            .unwrap_or(StorageBucketDurability::Relaxed)
    };
    Ok(match policy {
        StorageBucketDurability::Strict => TransactionDurability::Strict,
        StorageBucketDurability::Relaxed => TransactionDurability::Relaxed,
    })
}

fn missing_bucket_policy() -> IndexedDbError {
    IndexedDbError::InvalidState("StorageBucket durability policy is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idb_durability_bucket_defaults_use_exact_bucket_identity() {
        let shared = moli_storage_service::new_shared_storage_bucket_store();
        let mut store = shared.lock();
        let implicit = IndexedDbStorageScope::new("https://durability.test", "context", "profile");
        let named = store
            .open_bucket_with_options(
                implicit.storage_key(),
                "default",
                None,
                Some(StorageBucketDurability::Strict),
                None,
                None,
            )
            .unwrap();
        let named_scope = implicit.clone().with_bucket_identity(named.clone());
        assert_eq!(
            bucket_transaction_durability(&store, &implicit).unwrap(),
            TransactionDurability::Relaxed
        );
        assert_eq!(
            bucket_transaction_durability(&store, &named_scope).unwrap(),
            TransactionDurability::Strict
        );
        store
            .open_bucket_with_options(
                implicit.storage_key(),
                moli_storage_service::IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
                None,
                Some(StorageBucketDurability::Strict),
                None,
                None,
            )
            .unwrap();
        store
            .open_bucket_with_options(
                implicit.storage_key(),
                "default",
                None,
                Some(StorageBucketDurability::Relaxed),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            bucket_transaction_durability(&store, &implicit).unwrap(),
            TransactionDurability::Strict
        );
        assert_eq!(
            bucket_transaction_durability(&store, &named_scope).unwrap(),
            TransactionDurability::Relaxed
        );
        store
            .delete_bucket(implicit.storage_key(), "default")
            .unwrap();
        // This registry-only fixture has no backend resources to retire.
        store.finish_bucket_deletion(&named).unwrap();
        let recreated = store
            .open_bucket_with_options(
                implicit.storage_key(),
                "default",
                None,
                Some(StorageBucketDurability::Strict),
                None,
                None,
            )
            .unwrap();
        assert_ne!(named, recreated);
        assert!(matches!(
            bucket_transaction_durability(&store, &named_scope),
            Err(IndexedDbError::InvalidState(_))
        ));
        assert_eq!(
            bucket_transaction_durability(&store, &implicit.with_bucket_identity(recreated))
                .unwrap(),
            TransactionDurability::Strict
        );
    }
}

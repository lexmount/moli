use super::*;
use crate::context_bootstrap::storage_buckets::with_storage_bucket_store_entry;
use crate::webidl;
use moli_indexeddb::TransactionDurability;
use moli_storage_service::StorageBucketDurability;

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
    let durability = indexed_db_transaction_durability(scope, args.this())
        .expect("branded transaction durability");
    rv.set(v8str(scope, durability.label()).into());
}

pub(super) fn transaction_durability_for_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Result<TransactionDurability, IndexedDbError> {
    match indexed_db_transaction_durability(scope, transaction)
        .expect("transaction commit durability")
    {
        IdbTransactionDurability::Strict => Ok(TransactionDurability::Strict),
        IdbTransactionDurability::Relaxed => Ok(TransactionDurability::Relaxed),
        IdbTransactionDurability::Default => {
            let Some(storage_scope) = indexed_db_typed_storage_scope(scope, transaction) else {
                return Ok(TransactionDurability::Relaxed);
            };
            let durability = with_storage_bucket_store_entry(scope, |store| {
                if let Some(identity) = storage_scope.bucket_identity() {
                    store.bucket_durability_for_identity(identity)
                } else {
                    // An implicit default bucket has the service's relaxed policy.
                    Some(
                        store
                            .bucket_durability(storage_scope.storage_key(), "default")
                            .unwrap_or(StorageBucketDurability::Relaxed),
                    )
                }
            })
            .flatten()
            .ok_or_else(|| {
                IndexedDbError::InvalidState(
                    "StorageBucket IndexedDB durability policy is unavailable".to_owned(),
                )
            })?;
            Ok(match durability {
                StorageBucketDurability::Strict => TransactionDurability::Strict,
                StorageBucketDurability::Relaxed => TransactionDurability::Relaxed,
            })
        }
    }
}

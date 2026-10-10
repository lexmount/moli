use std::sync::Arc;

use moli_core::runtime::storage_partition::StoragePartitionState;

/// Write-through persistence handle for the CDP scheduler.
///
/// The protocol connection shares the partition's canonical cookie store, so a
/// mutating command leaves the in-memory store current. This handle flushes
/// that store to disk after such commands instead of relying on incidental
/// frontend lifecycle checkpoints.
#[derive(Clone)]
pub(crate) struct CookiePersistence {
    storage_partition: Arc<StoragePartitionState>,
}

impl CookiePersistence {
    pub(crate) fn new(storage_partition: Arc<StoragePartitionState>) -> Self {
        Self { storage_partition }
    }

    /// Returns true when `method` can mutate the browser cookie store.
    pub(crate) fn method_mutates_cookies(method: &str) -> bool {
        matches!(
            method,
            "Network.setCookie"
                | "Network.setCookies"
                | "Network.deleteCookies"
                | "Network.clearBrowserCookies"
                | "Storage.setCookies"
                | "Storage.deleteCookies"
                | "Storage.clearCookies"
                | "Storage.clearDataForOrigin"
        )
    }

    /// Flushes the shared cookie store without blocking the scheduler actor.
    pub(crate) fn flush_async(&self) {
        let storage_partition = self.storage_partition.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = storage_partition.flush() {
                tracing::warn!(?error, "failed to flush cookies after CDP cookie command");
            }
        });
    }
}

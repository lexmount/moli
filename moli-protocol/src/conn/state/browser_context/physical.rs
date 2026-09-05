use std::{collections::HashMap, path::PathBuf};

use moli_cookie_jar::SharedBrowserCookieStore;
use moli_core::{
    browser::BrowserContextId,
    network::SharedWebStorageStore,
    runtime::{
        NavigationEngine, NavigationRuntimeConfig, RendererBrowserContextRuntime,
        RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    },
    storage::{SharedIndexedDbManager, SharedStorageBucketStore},
};

use super::BrowserContextStoragePartitionHandles;

/// Physical context ownership, embedded in the migration wrapper until Commit 24b.
/// The page collection/selection joins this owner at Commit 7b. No projection
/// identity, output transport or session state belongs here.
pub(super) struct BrowserContext {
    pub(super) id: BrowserContextId,
    pub(super) page_navigation_runtime_config: Option<NavigationRuntimeConfig>,
    pub(super) network_policy: ContextNetworkPolicy,
    // The wrapper drops its page/engine residents first. Keep the stores alive
    // while the remaining runtime root stops producers and joins its network.
    renderer_runtime_owner: Option<RendererBrowserContextRuntimeOwner>,
    pub(super) storage_partition: StoragePartition,
}

impl BrowserContext {
    pub(super) fn new(
        handles: BrowserContextStoragePartitionHandles,
        kind: StoragePartitionKind,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self {
            id: BrowserContextId::allocate(),
            page_navigation_runtime_config: None,
            network_policy: ContextNetworkPolicy::default(),
            renderer_runtime_owner: Some(RendererBrowserContextRuntime::new()),
            storage_partition: StoragePartition {
                kind,
                handles,
                storage_quota_overrides: HashMap::new(),
                http_cache_root,
                http_cache_max_bytes,
            },
        }
    }

    pub(super) fn new_page_navigation_engine(
        &self,
        config: NavigationRuntimeConfig,
    ) -> NavigationEngine {
        NavigationEngine::new_with_runtime_config_and_browser_context_access(
            config,
            self.renderer_runtime_owner_access(),
        )
        .expect("live BrowserContext owner must accept a page engine")
    }

    pub(super) fn renderer_runtime(&self) -> RendererBrowserContextRuntime {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
    }

    pub(super) fn renderer_runtime_owner_access(&self) -> RendererBrowserContextRuntimeOwnerAccess {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .owner_access()
    }

    pub(super) fn take_renderer_runtime_owner_for_teardown(
        &mut self,
    ) -> Option<RendererBrowserContextRuntimeOwner> {
        self.renderer_runtime_owner.take()
    }
}

/// Context-scoped request defaults, with no frontend or session attribution.
/// A missing value inherits the process policy; an empty bypass list does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ContextNetworkPolicy {
    pub(crate) http_proxy: Option<String>,
    pub(crate) http_no_proxy: Option<String>,
    pub(crate) tls_verify_host: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoragePartitionKind {
    ProfileBacked,
    Ephemeral,
}

/// The stores and their policy share one owner; only capability handles clone.
pub(super) struct StoragePartition {
    pub(super) kind: StoragePartitionKind,
    pub(super) handles: BrowserContextStoragePartitionHandles,
    storage_quota_overrides: HashMap<String, f64>,
    http_cache_root: Option<PathBuf>,
    http_cache_max_bytes: Option<u64>,
}

impl StoragePartition {
    pub(super) fn kind_label(&self) -> &'static str {
        match self.kind {
            StoragePartitionKind::ProfileBacked => "profile-backed",
            StoragePartitionKind::Ephemeral => "ephemeral",
        }
    }

    pub(super) fn cookie_store(&self) -> &SharedBrowserCookieStore {
        &self.handles.cookie_store
    }

    pub(super) fn web_storage_store(&self) -> &SharedWebStorageStore {
        &self.handles.web_storage_store
    }

    pub(super) fn indexed_db_manager(&self) -> &SharedIndexedDbManager {
        &self.handles.indexed_db_manager
    }

    pub(super) fn storage_bucket_store(&self) -> &SharedStorageBucketStore {
        &self.handles.storage_bucket_store
    }

    #[cfg(test)]
    pub(super) fn replace_storage_bucket_store(
        &mut self,
        storage_bucket_store: SharedStorageBucketStore,
    ) {
        self.handles.storage_bucket_store = storage_bucket_store;
    }

    pub(super) fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        self.storage_quota_overrides
            .get(origin)
            .copied()
            .map(|quota| (quota, true))
            .unwrap_or((
                moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES as f64,
                false,
            ))
    }

    pub(super) fn set_storage_quota_override(&mut self, origin: String, quota: f64) {
        self.storage_quota_overrides.insert(origin, quota);
    }

    pub(super) fn clear_storage_quota_override(&mut self, origin: &str) {
        self.storage_quota_overrides.remove(origin);
    }

    pub(super) fn clear_http_cache(&self) -> Result<(), String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(());
        };
        moli_fetch::clear_http_cache_root(cache_root, self.http_cache_max_bytes)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    pub(super) fn clear_http_cache_for_origin(&self, origin: &url::Url) -> Result<usize, String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(0);
        };
        moli_fetch::clear_http_cache_root_for_origin(cache_root, self.http_cache_max_bytes, origin)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    #[cfg(test)]
    pub(super) fn http_cache_configuration(&self) -> (Option<&std::path::Path>, Option<u64>) {
        (self.http_cache_root.as_deref(), self.http_cache_max_bytes)
    }
}

impl std::fmt::Debug for StoragePartition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoragePartition")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

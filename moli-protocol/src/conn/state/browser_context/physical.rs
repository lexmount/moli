use std::path::PathBuf;

use moli_browser_profile::BrowserIdentityProfile;
use moli_core::{
    browser::BrowserContextId,
    runtime::{
        NavigationEngine, NavigationRuntimeConfig, RendererBrowserContextRuntime,
        RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    },
};

use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
};
use super::{
    BrowserContextStoragePartitionHandles,
    storage_partition::{StoragePartition, StoragePartitionKind},
};

/// Physical context ownership, embedded in the migration wrapper until Commit 24b.
/// The page collection/selection joins this owner at Commit 7b. No projection
/// identity, output transport or session state belongs here.
pub(super) struct BrowserContext {
    pub(super) id: BrowserContextId,
    pub(super) page_navigation_runtime_config: Option<NavigationRuntimeConfig>,
    pub(super) network_policy: ContextNetworkPolicy,
    pub(super) emulation_defaults: ContextEmulationDefaults,
    pub(super) browser_identity_override: Option<BrowserIdentityProfile>,
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
            emulation_defaults: ContextEmulationDefaults::default(),
            browser_identity_override: None,
            renderer_runtime_owner: Some(RendererBrowserContextRuntime::new()),
            storage_partition: StoragePartition::new(
                handles,
                kind,
                http_cache_root,
                http_cache_max_bytes,
            ),
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
    pub(crate) extra_headers: Vec<(String, String)>,
}

/// Installed context defaults; inherited process values are not copied here.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ContextEmulationDefaults {
    pub(crate) locale: Option<String>,
    pub(crate) timezone: Option<String>,
    pub(crate) network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) geolocation: Option<EmulatedGeolocationOverrideState>,
    pub(crate) device_metrics: Option<EmulatedDeviceMetrics>,
}

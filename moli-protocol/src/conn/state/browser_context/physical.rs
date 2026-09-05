use std::path::PathBuf;

use indexmap::IndexMap;
use moli_browser_profile::BrowserIdentityProfile;
use moli_core::{
    browser::{BrowserContextId, WebContentsId},
    runtime::{
        NavigationEngine, NavigationRuntimeConfig, RendererBrowserContextRuntime,
        RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    },
};

use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
};
use super::super::web_contents::{ClosingWebContents, WebContents};
use super::{
    BrowserContextStoragePartitionHandles,
    storage_partition::{StoragePartition, StoragePartitionKind},
};

/// Physical context ownership, embedded in the migration wrapper until Commit 24b.
/// No projection identity, output transport or session state belongs here.
pub(super) struct BrowserContext {
    pub(super) id: BrowserContextId,
    pub(super) page_navigation_runtime_config: Option<NavigationRuntimeConfig>,
    pub(super) network_policy: ContextNetworkPolicy,
    pub(super) emulation_defaults: ContextEmulationDefaults,
    pub(super) browser_identity_override: Option<BrowserIdentityProfile>,
    // The Browser collection and its only selector have the same lifetime.
    // Keep insertion order when choosing a replacement foreground page.
    pub(super) web_contents: IndexMap<WebContentsId, WebContents>,
    selected_web_contents: Option<WebContentsId>,
    // Drop Documents/engines before the runtime root and its storage handles.
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
            web_contents: IndexMap::new(),
            selected_web_contents: None,
            renderer_runtime_owner: Some(RendererBrowserContextRuntime::new()),
            storage_partition: StoragePartition::new(
                handles,
                kind,
                http_cache_root,
                http_cache_max_bytes,
            ),
        }
    }

    pub(super) fn selected_web_contents_id(&self) -> Option<WebContentsId> {
        self.selected_web_contents
    }

    pub(super) fn select_web_contents(&mut self, id: WebContentsId) -> bool {
        if !self.web_contents.contains_key(&id) {
            return false;
        }
        self.selected_web_contents = Some(id);
        true
    }

    pub(super) fn close_web_contents(&mut self, id: WebContentsId) -> Option<ClosingWebContents> {
        let removed = self.web_contents.shift_remove(&id)?;
        if self.selected_web_contents == Some(id) {
            self.selected_web_contents = None;
        }
        for contents in self.web_contents.values_mut() {
            if contents
                .window
                .opener
                .is_some_and(|opener| opener.web_contents_id == id)
            {
                contents.window.opener = None;
            }
        }
        Some(removed.begin_close())
    }

    pub(super) fn close_all_web_contents(&mut self) -> Vec<ClosingWebContents> {
        self.selected_web_contents = None;
        std::mem::take(&mut self.web_contents)
            .into_values()
            .map(WebContents::begin_close)
            .collect()
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

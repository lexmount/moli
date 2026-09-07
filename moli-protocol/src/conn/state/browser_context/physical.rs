use std::path::PathBuf;

use indexmap::IndexMap;
use moli_browser_profile::BrowserIdentityProfile;
use moli_core::{
    browser::{
        BrowserContextId, DocumentHandle, DownloadManager, DownloadPolicy, PermissionOverrides,
        WebContentsId,
    },
    runtime::{
        NavigationEngine, NavigationRuntimeConfig, RendererBrowserContextRuntime,
        RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    },
};

use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
};
use super::super::web_contents::{ClosingWebContents, DocumentHost, WebContents};
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
    pub(super) permission_overrides: PermissionOverrides,
    pub(super) download_policy: Option<DownloadPolicy>,
    pub(super) downloads: DownloadManager,
    // The Browser collection and its only selector have the same lifetime.
    // Keep insertion order when choosing a replacement foreground page.
    pub(super) web_contents: IndexMap<WebContentsId, WebContents>,
    selected_web_contents: Option<WebContentsId>,
    // Drop Documents/engines before the runtime root and its storage handles.
    renderer_runtime_owner: Option<RendererBrowserContextRuntimeOwner>,
    pub(super) storage_partition: StoragePartition,
}

impl BrowserContext {
    pub(super) fn document(&self, handle: DocumentHandle) -> Result<&DocumentHost, String> {
        if handle.web_contents().context() != self.id {
            return Err("Document belongs to a different BrowserContext".into());
        }
        let document = self
            .web_contents
            .get(&handle.web_contents().id())
            .and_then(|contents| contents.main_frame.current_document.as_ref())
            .ok_or("NoDocumentLoaded")?;
        if document.id != handle.id() {
            return Err("Document changed".into());
        }
        Ok(document)
    }

    pub(super) fn document_mut(
        &mut self,
        handle: DocumentHandle,
    ) -> Result<&mut DocumentHost, String> {
        if handle.web_contents().context() != self.id {
            return Err("Document belongs to a different BrowserContext".into());
        }
        let document = self
            .web_contents
            .get_mut(&handle.web_contents().id())
            .and_then(|contents| contents.main_frame.current_document.as_mut())
            .ok_or("NoDocumentLoaded")?;
        if document.id != handle.id() {
            return Err("Document changed".into());
        }
        Ok(document)
    }

    pub(super) fn inherited_document_policy(
        &self,
        fetch_config: moli_fetch::FetchConfig,
        defaults: &moli_core::browser::PermissionDefaults,
        global_headers: &[(String, String)],
        global_network_conditions: Option<EmulatedNetworkConditions>,
        global_geolocation_override: Option<&EmulatedGeolocationOverrideState>,
    ) -> super::super::web_contents::InheritedDocumentPolicy {
        let mut policy =
            self.inherited_resource_policy(fetch_config, global_headers, global_network_conditions);
        policy.permissions = self.permission_overrides.snapshot(defaults);
        policy.emulation.geolocation = policy
            .emulation
            .geolocation
            .or_else(|| global_geolocation_override.cloned());
        policy
    }

    pub(super) fn inherited_resource_policy(
        &self,
        mut fetch_config: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<EmulatedNetworkConditions>,
    ) -> super::super::web_contents::InheritedDocumentPolicy {
        if let Some(identity) = &self.browser_identity_override {
            fetch_config.set_browser_identity(identity.clone());
        }
        if let Some(proxy) = &self.network_policy.http_proxy {
            fetch_config.set_http_proxy(Some(proxy.clone()));
        }
        if let Some(no_proxy) = &self.network_policy.http_no_proxy {
            fetch_config.set_http_no_proxy(Some(no_proxy.clone()));
        }
        if let Some(verify) = self.network_policy.tls_verify_host {
            fetch_config.set_tls_verify_host(verify);
        }
        let mut emulation = self.emulation_defaults.clone();
        emulation.network_conditions = emulation.network_conditions.or(global_network_conditions);
        super::super::web_contents::InheritedDocumentPolicy {
            fetch_config,
            extra_headers: super::super::web_contents::merge_extra_header_layers(&[
                global_headers,
                &self.network_policy.extra_headers,
            ]),
            emulation,
            permissions: Vec::new(),
            storage: self.storage_partition.handles.clone(),
        }
    }

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
            permission_overrides: PermissionOverrides::default(),
            download_policy: None,
            downloads: DownloadManager::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resource_maintenance_never_adopts_a_peers_transport_or_policy() {
        let mut context = BrowserContext::new(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        );
        let mut ids = Vec::new();
        for (agent, verify) in [("Native/first", false), ("Native/peer", true)] {
            let mut contents = WebContents::default();
            contents.install_navigation_engine(
                context.new_page_navigation_engine(NavigationRuntimeConfig::default()),
            );
            contents.browser_identity_override = Some(BrowserIdentityProfile::new(agent, "en"));
            contents.tls_verify_host_override = Some(verify);
            ids.push(contents.id());
            context.web_contents.insert(contents.id(), contents);
        }
        let inherited =
            context.inherited_resource_policy(moli_fetch::FetchConfig::default(), &[], None);
        let first = context
            .web_contents
            .get_mut(&ids[0])
            .unwrap()
            .ensure_resource_request_client(&inherited)
            .unwrap();
        let peer = context
            .web_contents
            .get_mut(&ids[1])
            .unwrap()
            .ensure_resource_request_client(&inherited)
            .unwrap();
        assert!(!first.shares_resource_runtime_with(&peer));
        assert!(std::sync::Arc::ptr_eq(
            &first.cookie_store(),
            &peer.cookie_store()
        ));
        let contents = context.web_contents.get_mut(&ids[0]).unwrap();
        contents.invalidate_resource_runtime();
        assert!(
            contents
                .start_resource_runtime_rebuild(&inherited)
                .unwrap()
                .is_none()
        );
        let rebuilt = contents.ensure_resource_request_client(&inherited).unwrap();
        assert!(!rebuilt.shares_resource_runtime_with(&peer));
        assert!(rebuilt.shares_page_network_policy_with(&first));
        assert!(
            rebuilt
                .browser_resource_runtime()
                .matches_fetch_config(contents.navigation_fetch_config().unwrap())
        );
        assert!(
            !contents
                .navigation_fetch_config()
                .unwrap()
                .tls_verify_host()
        );
        assert_eq!(
            rebuilt
                .browser_resource_runtime()
                .browser_identity()
                .user_agent(),
            "Native/first"
        );
        let contents = context.web_contents.get(&ids[1]).unwrap();
        assert!(
            contents
                .navigation_fetch_config()
                .unwrap()
                .tls_verify_host()
        );
        assert_eq!(
            peer.browser_resource_runtime()
                .browser_identity()
                .user_agent(),
            "Native/peer"
        );
        assert!(
            peer.browser_resource_runtime()
                .matches_fetch_config(contents.navigation_fetch_config().unwrap())
        );
    }

    #[tokio::test]
    async fn document_policy_capture_needs_no_projection_or_current_document() {
        let mut context = BrowserContext::new(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        );
        context.network_policy.extra_headers = vec![("X-Policy".into(), "context".into())];
        context.network_policy.tls_verify_host = Some(false);
        context.emulation_defaults.locale = Some("fr-FR".into());
        let mut contents = WebContents::default();
        contents.install_navigation_engine(
            context.new_page_navigation_engine(NavigationRuntimeConfig::default()),
        );
        contents.network_request_policy.extra_headers = vec![("X-Policy".into(), "page".into())];
        contents.emulation_policy.cpu_throttling_rate = 2.5;
        contents.emulation_policy.script_execution_disabled = true;
        contents.emulation_policy.touch_emulation_enabled = true;
        assert!(
            contents
                .start_fetch_interception_update(true, None)
                .unwrap()
                .is_none()
        );
        let id = contents.id();
        context.web_contents.insert(id, contents);
        let geolocation =
            EmulatedGeolocationOverrideState::Position(crate::conn::EmulatedGeolocationOverride {
                latitude: 48.85837,
                longitude: 2.294481,
                accuracy: 7.0,
                altitude: None,
                altitude_accuracy: None,
                heading: None,
                speed: None,
            });
        let inherited = context.inherited_document_policy(
            moli_fetch::FetchConfig::default(),
            &moli_core::browser::PermissionDefaults::default(),
            &[
                ("X-Global".into(), "global".into()),
                ("X-Policy".into(), "global".into()),
            ],
            Some(EmulatedNetworkConditions::offline()),
            Some(&geolocation),
        );
        let contents = context.web_contents.get_mut(&id).unwrap();
        let policy = contents
            .capture_document_policy(inherited, &url::Url::parse("about:blank").unwrap())
            .unwrap();
        assert_eq!(
            policy.extra_http_headers,
            [
                ("X-Global".into(), "global".into()),
                ("X-Policy".into(), "page".into())
            ]
        );
        assert_eq!(policy.locale_override.as_deref(), Some("fr-FR"));
        assert!(policy.network_offline);
        assert_eq!(policy.navigator_overrides.online, Some(false));
        assert_eq!(policy.navigator_overrides.max_touch_points, 1);
        assert_eq!(
            policy
                .navigator_overrides
                .geolocation
                .as_ref()
                .map(|position| position.latitude),
            Some(48.85837),
        );
        assert!(policy.script_execution_disabled);
        assert_eq!(policy.cpu_throttling_rate, 2.5);
        assert!(policy.fetch_subresource_interception_enabled);
        assert!(
            !contents
                .navigation_engine_for_test()
                .unwrap()
                .fetch_config()
                .tls_verify_host()
        );
        assert!(contents.main_frame.current_document.is_none());
        assert!(
            contents
                .start_fetch_interception_update(false, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(contents.fetch_subresource_interception(), (false, None));
        assert!(
            policy.fetch_subresource_interception_enabled,
            "capture is a value, not a live registration view"
        );
    }
}

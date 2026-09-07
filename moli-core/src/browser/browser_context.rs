use std::path::PathBuf;

use crate::{
    browser::{
        BrowserContextId, DocumentHandle, DownloadManager, DownloadPolicy, PermissionOverrides,
        WebContentsHandle, WebContentsId,
    },
    network::{SharedWebStorageStore, new_shared_web_storage_store},
    runtime::{
        NavigationEngine, NavigationPageStorageHandles, NavigationResourceStorageHandles,
        NavigationRuntimeConfig, RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner,
        RendererBrowserContextRuntimeOwnerAccess, storage_partition::StoragePartitionState,
    },
    storage::{
        SharedIndexedDbManager, SharedStorageBucketStore, WeakIndexedDbManager,
        new_shared_storage_bucket_store_with_indexed_db_manager,
    },
};
use indexmap::IndexMap;
use moli_browser_profile::BrowserIdentityProfile;
use moli_cookie_jar::{
    CookieSource, SharedBrowserCookieStore, StoredCookie, new_shared_browser_cookie_store,
};

use super::web_contents::{ClosingWebContents, DocumentHost, WebContents};
use crate::browser::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
};

mod dialogs;
mod document_commands;
mod document_policy;
mod document_queries;
mod fetch;
mod input;
mod navigation;
mod permissions;
mod residence;
mod resource_commands;
mod resource_runtime;
mod settings;
mod storage_partition;
mod workers;
pub use document_commands::*;
pub use document_policy::*;
pub use fetch::*;
pub use input::*;
pub use navigation::DocumentNavigationMetadata;
pub use permissions::{CompletedContextPermissionUpdate, PendingContextPermissionUpdate};
pub use resource_commands::*;
use storage_partition::StoragePartition;
pub use storage_partition::{OriginStorageUsage, SiteDataClearOptions, StoragePartitionKind};

#[derive(Clone)]
pub struct BrowserContextStoragePartitionHandles {
    pub cookie_store: SharedBrowserCookieStore,
    pub web_storage_store: SharedWebStorageStore,
    pub indexed_db_manager: SharedIndexedDbManager,
    pub storage_bucket_store: SharedStorageBucketStore,
}

#[derive(Clone)]
pub struct BrowserContextResourceStorageHandles {
    pub cookie_store: SharedBrowserCookieStore,
    pub web_storage_store: SharedWebStorageStore,
    pub session_storage_store: SharedWebStorageStore,
}

#[derive(Clone)]
pub struct BrowserContextPageStorageHandles {
    pub cookie_store: SharedBrowserCookieStore,
    pub web_storage_store: SharedWebStorageStore,
    pub session_storage_store: SharedWebStorageStore,
    pub indexed_db_manager: Option<WeakIndexedDbManager>,
    pub storage_bucket_store: Option<SharedStorageBucketStore>,
}

impl BrowserContextStoragePartitionHandles {
    fn from_stores(
        cookie_store: SharedBrowserCookieStore,
        web_storage_store: SharedWebStorageStore,
        indexed_db_manager: SharedIndexedDbManager,
        storage_bucket_store: SharedStorageBucketStore,
    ) -> Self {
        Self {
            cookie_store,
            web_storage_store,
            indexed_db_manager,
            storage_bucket_store,
        }
    }

    pub fn memory() -> Self {
        Self::with_initial_cookies(Vec::new())
    }

    pub fn with_initial_cookies(initial_cookies: impl IntoIterator<Item = StoredCookie>) -> Self {
        let cookie_store = new_shared_browser_cookie_store();
        seed_initial_cookies(&cookie_store, initial_cookies);
        let indexed_db_manager = crate::storage::new_indexed_db_manager(None)
            .expect("in-memory IndexedDB manager should initialize");
        let storage_bucket_store =
            new_shared_storage_bucket_store_with_indexed_db_manager(&indexed_db_manager);
        Self::from_stores(
            cookie_store,
            new_shared_web_storage_store(),
            indexed_db_manager,
            storage_bucket_store,
        )
    }

    fn from_initial_storage_partition(
        cookie_store: SharedBrowserCookieStore,
        local_storage_store: SharedWebStorageStore,
        indexed_db_manager: SharedIndexedDbManager,
        storage_bucket_store: SharedStorageBucketStore,
    ) -> Self {
        Self::from_stores(
            cookie_store,
            local_storage_store,
            indexed_db_manager,
            storage_bucket_store,
        )
    }

    pub fn from_storage_partition(
        initial_cookies: impl IntoIterator<Item = StoredCookie>,
        storage_partition: &StoragePartitionState,
    ) -> Self {
        let cookie_store = new_shared_browser_cookie_store();
        seed_initial_cookies(&cookie_store, initial_cookies);
        let shared_storage = storage_partition.shared_storage_handles();
        Self::from_initial_storage_partition(
            cookie_store,
            shared_storage.web_storage_store(),
            shared_storage.indexed_db_manager(),
            shared_storage.storage_bucket_store(),
        )
    }

    pub fn resource_storage_handles(
        &self,
        session_storage_store: SharedWebStorageStore,
    ) -> BrowserContextResourceStorageHandles {
        BrowserContextResourceStorageHandles {
            cookie_store: self.cookie_store.clone(),
            web_storage_store: self.web_storage_store.clone(),
            session_storage_store,
        }
    }

    pub fn page_storage_handles(
        &self,
        session_storage_store: SharedWebStorageStore,
    ) -> BrowserContextPageStorageHandles {
        BrowserContextPageStorageHandles {
            cookie_store: self.cookie_store.clone(),
            web_storage_store: self.web_storage_store.clone(),
            session_storage_store,
            indexed_db_manager: Some(crate::storage::downgrade_indexed_db_manager(
                &self.indexed_db_manager,
            )),
            storage_bucket_store: Some(self.storage_bucket_store.clone()),
        }
    }
}

impl BrowserContextResourceStorageHandles {
    pub fn into_navigation_storage(self) -> NavigationResourceStorageHandles {
        NavigationResourceStorageHandles::new(
            self.cookie_store,
            self.web_storage_store,
            self.session_storage_store,
        )
    }
}

impl BrowserContextPageStorageHandles {
    pub fn into_navigation_storage(self) -> NavigationPageStorageHandles {
        NavigationPageStorageHandles::new(
            self.cookie_store,
            self.web_storage_store,
            self.session_storage_store,
            self.indexed_db_manager,
            self.storage_bucket_store,
        )
    }
}

fn seed_initial_cookies(
    cookie_store: &SharedBrowserCookieStore,
    initial_cookies: impl IntoIterator<Item = StoredCookie>,
) {
    let mut store = cookie_store.lock();
    for cookie in initial_cookies {
        let _ = store.upsert_with_request_url_report(cookie, None, CookieSource::Management);
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn seed_initial_cookies_for_test(
    cookie_store: &SharedBrowserCookieStore,
    initial_cookies: impl IntoIterator<Item = StoredCookie>,
) {
    seed_initial_cookies(cookie_store, initial_cookies);
}

/// Physical context ownership held only by the Browser owner sequence.
/// No projection identity, output transport or session state belongs here.
pub struct BrowserContext {
    id: BrowserContextId,
    page_navigation_runtime_config: Option<NavigationRuntimeConfig>,
    network_policy: ContextNetworkPolicy,
    emulation_defaults: ContextEmulationDefaults,
    browser_identity_override: Option<BrowserIdentityProfile>,
    permission_overrides: PermissionOverrides,
    download_policy: Option<DownloadPolicy>,
    downloads: DownloadManager,
    // The Browser collection and its only selector have the same lifetime.
    // Keep insertion order when choosing a replacement foreground page.
    web_contents: IndexMap<WebContentsId, WebContents>,
    selected_web_contents: Option<WebContentsId>,
    // Drop Documents/engines before the runtime root and its storage handles.
    renderer_output_transport_sender: Option<crate::RendererOutputTransportSender>,
    renderer_runtime_owner: Option<RendererBrowserContextRuntimeOwner>,
    storage_partition: StoragePartition,
}

impl BrowserContext {
    pub fn id(&self) -> BrowserContextId {
        self.id
    }

    pub(crate) fn web_contents(&self, handle: WebContentsHandle) -> Result<&WebContents, String> {
        if handle.context() != self.id {
            return Err("WebContents belongs to a different BrowserContext".into());
        }
        self.web_contents
            .get(&handle.id())
            .ok_or_else(|| "WebContents unavailable".into())
    }

    pub(crate) fn web_contents_mut(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<&mut WebContents, String> {
        if handle.context() != self.id {
            return Err("WebContents belongs to a different BrowserContext".into());
        }
        self.web_contents
            .get_mut(&handle.id())
            .ok_or_else(|| "WebContents unavailable".into())
    }

    pub(crate) fn document(&self, handle: DocumentHandle) -> Result<&DocumentHost, String> {
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

    pub(crate) fn document_mut(
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

    pub fn inherited_document_policy(
        &self,
        fetch_config: moli_fetch::FetchConfig,
        defaults: &crate::browser::PermissionDefaults,
        global_headers: &[(String, String)],
        global_network_conditions: Option<EmulatedNetworkConditions>,
        global_geolocation: Option<&EmulatedGeolocationOverrideState>,
    ) -> super::web_contents::InheritedDocumentPolicy {
        let mut policy =
            self.inherited_resource_policy(fetch_config, global_headers, global_network_conditions);
        policy.permissions = self.permission_overrides.snapshot(defaults);
        policy.emulation.geolocation = policy
            .emulation
            .geolocation
            .or_else(|| global_geolocation.cloned());
        policy
    }

    pub fn inherited_resource_policy(
        &self,
        mut fetch_config: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<EmulatedNetworkConditions>,
    ) -> super::web_contents::InheritedDocumentPolicy {
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
        super::web_contents::InheritedDocumentPolicy {
            fetch_config,
            extra_headers: super::web_contents::merge_extra_header_layers(&[
                global_headers,
                &self.network_policy.extra_headers,
            ]),
            emulation,
            permissions: Vec::new(),
            storage: self.storage_partition.handles.clone(),
        }
    }

    pub fn new(
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
            renderer_output_transport_sender: None,
            renderer_runtime_owner: Some(RendererBrowserContextRuntime::new()),
            storage_partition: StoragePartition::new(
                handles,
                kind,
                http_cache_root,
                http_cache_max_bytes,
            ),
        }
    }

    pub fn selected_web_contents_id(&self) -> Option<WebContentsId> {
        self.selected_web_contents
    }

    pub fn select_web_contents(&mut self, id: WebContentsId) -> bool {
        if !self.web_contents.contains_key(&id) {
            return false;
        }
        self.selected_web_contents = Some(id);
        true
    }

    pub fn close_web_contents(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<ClosingWebContents, String> {
        self.web_contents(handle)?;
        let id = handle.id();
        let removed = self
            .web_contents
            .shift_remove(&id)
            .expect("validated WebContents must remain resident until close");
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
        Ok(removed.begin_close())
    }

    pub fn close_all_web_contents(&mut self) -> Vec<ClosingWebContents> {
        self.selected_web_contents = None;
        std::mem::take(&mut self.web_contents)
            .into_values()
            .map(WebContents::begin_close)
            .collect()
    }

    fn new_page_navigation_engine(&self, config: NavigationRuntimeConfig) -> NavigationEngine {
        let engine = NavigationEngine::new_with_runtime_config_and_browser_context_access(
            config,
            self.renderer_runtime_owner_access(),
        )
        .expect("live BrowserContext owner must accept a page engine");
        if let Some(sender) = self.renderer_output_transport_sender.clone() {
            engine.set_renderer_output_transport_sender(sender);
        }
        engine
    }

    fn renderer_runtime(&self) -> RendererBrowserContextRuntime {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn renderer_runtime_id_for_test(&self) -> crate::RendererBrowserContextRuntimeId {
        self.renderer_runtime().id()
    }

    fn renderer_runtime_owner_access(&self) -> RendererBrowserContextRuntimeOwnerAccess {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .owner_access()
    }

    pub(crate) fn shutdown(mut self) {
        let mut runtime = self
            .renderer_runtime_owner
            .take()
            .expect("BrowserContext renderer owner must exist until teardown");
        runtime.terminate_renderer_producers_for_owner_shutdown();
        drop(self);
        runtime.shutdown_network_and_join();
    }
}

/// Context-scoped request defaults, with no frontend or session attribution.
/// A missing value inherits the process policy; an empty bypass list does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextNetworkPolicy {
    pub http_proxy: Option<String>,
    pub http_no_proxy: Option<String>,
    pub tls_verify_host: Option<bool>,
    pub extra_headers: Vec<(String, String)>,
}

/// Installed context defaults; inherited process values are not copied here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContextEmulationDefaults {
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub network_conditions: Option<EmulatedNetworkConditions>,
    pub geolocation: Option<EmulatedGeolocationOverrideState>,
    pub device_metrics: Option<EmulatedDeviceMetrics>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn permission_rule(setting: &str) -> crate::page::PermissionOverrideRegistration {
        crate::page::PermissionOverrideRegistration {
            permission: json!({"name": "geolocation"}),
            setting: setting.into(),
            origin: None,
            embedded_origin: None,
        }
    }

    fn install_test_document(
        context: &mut BrowserContext,
        page: crate::page::Page,
    ) -> WebContentsHandle {
        let (handle, _) = context
            .register_web_contents(WebContents::default())
            .expect("test WebContents");
        context
            .replace_document_for_test(
                handle,
                Some(DocumentHost::new(
                    crate::browser::DocumentId::allocate(),
                    page,
                )),
            )
            .expect("test Document");
        handle
    }

    #[tokio::test]
    async fn permission_refresh_visits_all_documents_without_protocol_projection() {
        let browser = crate::runtime::Browser::new(crate::runtime::BrowserConfig::default())
            .expect("test browser");
        let mut defaults = crate::browser::PermissionDefaults::default();
        defaults.set(permission_rule("denied"));
        let mut context = BrowserContext::new(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        );
        let mut handles = Vec::new();
        for title in ["first", "second"] {
            let page = browser
                .fetch(&format!("data:text/html,<title>{title}</title>"))
                .await
                .expect("test Page");
            handles.push(install_test_document(&mut context, page));
        }
        context.set_permission_override(&mut defaults, permission_rule("granted"));

        assert_eq!(context.permission_override_count(), 1);
        for expected in ["granted", "denied"] {
            let pending = context
                .start_permission_update(&defaults)
                .expect("permission update")
                .expect("loaded Documents");
            assert_eq!(pending.page_count_for_test(), 2);
            let completed = pending.wait().await;
            context
                .finish_permission_update(completed)
                .expect("permission completion");
            for (handle, title) in handles.iter().copied().zip(["first", "second"]) {
                let document = context
                    .document_handle_for_web_contents(handle)
                    .expect("test WebContents")
                    .expect("test Document");
                assert_eq!(context.document_title(document).unwrap(), title);
                let result = context
                    .evaluate_document_expression_for_test(
                        document,
                        "navigator.permissions.query({name:'geolocation'}).then(status => status.state)",
                        true,
                    )
                    .await
                    .unwrap();
                assert_eq!(result, json!({"type": "string", "value": expected}));
            }
            context.clear_permission_overrides();
        }
        assert_eq!(defaults.snapshot(), vec![permission_rule("denied")]);
    }

    #[tokio::test]
    async fn permission_completion_cannot_retarget_replacement_web_contents() {
        let browser = crate::runtime::Browser::new(crate::runtime::BrowserConfig::default())
            .expect("test browser");
        let mut context = BrowserContext::new(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        );
        let first = browser
            .fetch("data:text/html,<title>first</title>")
            .await
            .unwrap();
        let old_handle = install_test_document(&mut context, first);
        let mut defaults = crate::browser::PermissionDefaults::default();
        context.set_permission_override(&mut defaults, permission_rule("granted"));
        let completed = context
            .start_permission_update(&defaults)
            .unwrap()
            .unwrap()
            .wait()
            .await;

        context
            .close_web_contents(old_handle)
            .unwrap()
            .close_async()
            .await;
        let replacement = browser
            .fetch("data:text/html,<title>replacement</title>")
            .await
            .unwrap();
        let replacement = install_test_document(&mut context, replacement);
        assert_ne!(replacement, old_handle);
        assert_eq!(
            context.finish_permission_update(completed),
            Err("NoDocumentLoaded".into())
        );
        let document = context
            .document_handle_for_web_contents(replacement)
            .unwrap()
            .unwrap();
        assert_eq!(context.document_title(document).unwrap(), "replacement");
    }

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
        let geolocation = EmulatedGeolocationOverrideState::Position(
            crate::browser::EmulatedGeolocationOverride {
                latitude: 48.85837,
                longitude: 2.294481,
                accuracy: 7.0,
                altitude: None,
                altitude_accuracy: None,
                heading: None,
                speed: None,
            },
        );
        let inherited = context.inherited_document_policy(
            moli_fetch::FetchConfig::default(),
            &crate::browser::PermissionDefaults::default(),
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

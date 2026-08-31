use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

use moli_cookie_jar::{
    BrowserCookieStore, CookieSource, NetworkCookieRequestContext, SharedBrowserCookieStore,
    StoredCookie, StoredCookieQueryReport, new_shared_browser_cookie_store,
};
use moli_core::network::{SharedWebStorageStore, new_shared_web_storage_store};
use moli_core::runtime::{
    NavigationEngine, NavigationPageStorageHandles, NavigationResourceStorageHandles,
    NavigationRuntimeConfig, RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner,
    RendererBrowserContextRuntimeOwnerAccess, RendererSharedWorkerRuntimeDiagnostics,
    storage_partition::StoragePartitionState,
};
use moli_core::storage::{
    SharedIndexedDbManager, SharedStorageBucketStore, StorageBucketIdentity, WeakIndexedDbManager,
    new_shared_storage_bucket_store_with_indexed_db_manager,
};
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::{Value, json};

use super::{
    DevToolsSessionState,
    dedicated_worker_target::DedicatedWorkerTargetState,
    emulation::{
        EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
    },
    javascript_dialog::TargetPreparedJavaScriptDialog,
    page_slot::{DocumentNavigationToken, DocumentStartScript},
    page_target_host::{PageTargetHost, PageTargetRegistry},
    parking::TargetOwnerState,
    service_worker_target::ServiceWorkerTargetState,
    session::TargetPageState,
    shared_worker_target::SharedWorkerTargetState,
};

#[cfg(test)]
const TEST_FIXTURE_PAGE_TARGET_ID: &str = "__moli_test_fixture_page__";

pub struct BrowserContext {
    pub id: String,
    storage_partition: BrowserContextStoragePartition,
    pub(crate) page_targets: PageTargetRegistry,
    pub auxiliary_target_sessions: HashMap<String, String>,
    // Chromium exposes the live creator target, immutable creator frame, and
    // window.opener access as three independent TargetInfo properties.
    pub target_opener_ids: HashMap<String, String>,
    pub target_opener_frame_ids: HashMap<String, String>,
    /// Targets whose DOM Window retains script access to its opener.
    ///
    /// DevTools creator identity is stored separately in the opener maps:
    /// an implicit-noopener `_blank` target still has an `openerId`, but is
    /// intentionally absent from this set.
    pub(crate) target_can_access_opener: HashSet<String>,
    pub target_window_names: HashMap<String, String>,
    pub target_popup_ids: HashMap<String, u64>,
    pending_popup_javascript_dialogs: HashMap<u64, Vec<TargetPreparedJavaScriptDialog>>,
    pub(crate) shared_worker_targets: BTreeMap<SharedWorkerInstanceId, SharedWorkerTargetState>,
    pub(crate) dedicated_worker_targets: BTreeMap<u64, DedicatedWorkerTargetState>,
    pub(crate) service_worker_targets: BTreeMap<u64, ServiceWorkerTargetState>,
    pub(crate) service_worker_domain_sessions: BTreeSet<Option<String>>,
    pub(crate) default_extra_headers: Vec<(String, String)>,
    pub(crate) global_extra_headers: Vec<(String, String)>,
    default_browser_identity: super::BaseBrowserIdentityOverrideState,
    pub(crate) default_locale_override: Option<String>,
    pub(crate) default_timezone_override: Option<String>,
    pub(crate) default_network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) default_geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub(crate) global_network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) global_geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub(crate) global_cache_disabled: bool,
    pub(crate) default_http_proxy_override: Option<String>,
    pub(crate) default_http_no_proxy_override: Option<String>,
    pub(crate) default_tls_verify_host_override: Option<bool>,
    pub proxy_autoconfig_url: Option<String>,
    pub proxy_socks_version: Option<u8>,
    pub(crate) default_emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub(crate) next_default_document_start_script_id: u32,
    pub(crate) default_document_start_scripts: Vec<(String, DocumentStartScript)>,
    pub(crate) storage_quota_overrides: HashMap<String, f64>,
    pub(crate) http_cache_root: Option<PathBuf>,
    pub(crate) http_cache_max_bytes: Option<u64>,
    pub(crate) page_navigation_runtime_config: Option<NavigationRuntimeConfig>,
    renderer_output_transport_sender: Option<moli_core::RendererOutputTransportSender>,
    // Keep the sole strong per-context network owner last. Protocol teardown
    // must drop pages and NavigationEngines before this field is released.
    pub(crate) renderer_runtime_owner: Option<RendererBrowserContextRuntimeOwner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextStoragePartitionKind {
    ProfileBacked,
    Ephemeral,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextStoragePartitionIdentity {
    kind: BrowserContextStoragePartitionKind,
    id: String,
}

#[derive(Clone)]
struct BrowserContextStoragePartition {
    identity: BrowserContextStoragePartitionIdentity,
    handles: BrowserContextStoragePartitionHandles,
}

#[derive(Clone)]
pub(crate) struct BrowserContextStoragePartitionHandles {
    cookie_store: SharedBrowserCookieStore,
    web_storage_store: SharedWebStorageStore,
    indexed_db_manager: SharedIndexedDbManager,
    storage_bucket_store: SharedStorageBucketStore,
}

#[derive(Clone)]
pub(crate) struct BrowserContextResourceStorageHandles {
    pub(crate) cookie_store: SharedBrowserCookieStore,
    pub(crate) web_storage_store: SharedWebStorageStore,
    pub(crate) session_storage_store: SharedWebStorageStore,
}

#[derive(Clone)]
pub(crate) struct BrowserContextPageStorageHandles {
    pub(crate) cookie_store: SharedBrowserCookieStore,
    pub(crate) web_storage_store: SharedWebStorageStore,
    pub(crate) session_storage_store: SharedWebStorageStore,
    pub(crate) indexed_db_manager: Option<WeakIndexedDbManager>,
    pub(crate) storage_bucket_store: Option<SharedStorageBucketStore>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct BrowserContextOriginStorageUsage {
    pub(crate) local_storage_usage: u64,
    pub(crate) indexed_db_usage: u64,
    pub(crate) storage_buckets_usage: u64,
    pub(crate) total_usage: u64,
}

impl BrowserContextStoragePartition {
    fn new(
        identity: BrowserContextStoragePartitionIdentity,
        handles: BrowserContextStoragePartitionHandles,
    ) -> Self {
        Self { identity, handles }
    }

    fn identity(&self) -> &BrowserContextStoragePartitionIdentity {
        &self.identity
    }

    fn cookie_store(&self) -> &SharedBrowserCookieStore {
        &self.handles.cookie_store
    }

    fn web_storage_store(&self) -> &SharedWebStorageStore {
        &self.handles.web_storage_store
    }

    fn indexed_db_manager(&self) -> &SharedIndexedDbManager {
        &self.handles.indexed_db_manager
    }

    fn storage_bucket_store(&self) -> &SharedStorageBucketStore {
        &self.handles.storage_bucket_store
    }

    #[cfg(test)]
    fn replace_storage_bucket_store(&mut self, storage_bucket_store: SharedStorageBucketStore) {
        self.handles.storage_bucket_store = storage_bucket_store;
    }
}

impl std::fmt::Debug for BrowserContextStoragePartition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContextStoragePartition")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl BrowserContextStoragePartitionIdentity {
    pub(crate) fn profile_backed_default() -> Self {
        Self {
            kind: BrowserContextStoragePartitionKind::ProfileBacked,
            id: "default".to_owned(),
        }
    }

    pub(crate) fn ephemeral(id: impl Into<String>) -> Self {
        Self {
            kind: BrowserContextStoragePartitionKind::Ephemeral,
            id: id.into(),
        }
    }

    pub(crate) fn kind(&self) -> BrowserContextStoragePartitionKind {
        self.kind
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn kind_label(&self) -> &'static str {
        match self.kind {
            BrowserContextStoragePartitionKind::ProfileBacked => "profile-backed",
            BrowserContextStoragePartitionKind::Ephemeral => "ephemeral",
        }
    }
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

    pub(crate) fn memory() -> Self {
        Self::with_initial_cookies(Vec::new())
    }

    pub(crate) fn with_initial_cookies(
        initial_cookies: impl IntoIterator<Item = StoredCookie>,
    ) -> Self {
        let cookie_store = new_shared_browser_cookie_store();
        seed_initial_cookies(&cookie_store, initial_cookies);
        let indexed_db_manager = moli_core::storage::new_indexed_db_manager(None)
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

    pub(crate) fn from_storage_partition(
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

    pub(crate) fn resource_storage_handles(
        &self,
        session_storage_store: SharedWebStorageStore,
    ) -> BrowserContextResourceStorageHandles {
        BrowserContextResourceStorageHandles {
            cookie_store: self.cookie_store.clone(),
            web_storage_store: self.web_storage_store.clone(),
            session_storage_store,
        }
    }

    pub(crate) fn page_storage_handles(
        &self,
        session_storage_store: SharedWebStorageStore,
    ) -> BrowserContextPageStorageHandles {
        BrowserContextPageStorageHandles {
            cookie_store: self.cookie_store.clone(),
            web_storage_store: self.web_storage_store.clone(),
            session_storage_store,
            indexed_db_manager: Some(moli_core::storage::downgrade_indexed_db_manager(
                &self.indexed_db_manager,
            )),
            storage_bucket_store: Some(self.storage_bucket_store.clone()),
        }
    }
}

impl BrowserContextResourceStorageHandles {
    pub(crate) fn into_navigation_storage(self) -> NavigationResourceStorageHandles {
        NavigationResourceStorageHandles::new(
            self.cookie_store,
            self.web_storage_store,
            self.session_storage_store,
        )
    }
}

impl BrowserContextPageStorageHandles {
    pub(crate) fn into_navigation_storage(self) -> NavigationPageStorageHandles {
        NavigationPageStorageHandles::new(
            self.cookie_store,
            self.web_storage_store,
            self.session_storage_store,
            self.indexed_db_manager,
            self.storage_bucket_store,
        )
    }
}

impl std::fmt::Debug for BrowserContextStoragePartitionIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContextStoragePartitionIdentity")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SiteDataClearOptions {
    pub cookies: bool,
    pub local_storage: bool,
    pub indexed_db: bool,
    pub storage_buckets: bool,
    pub http_cache: bool,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("id", &self.id)
            .field("storage_partition", &self.storage_partition)
            .field("active_target_id", &self.active_target_id())
            .field("active_session_id", &self.active_session_id())
            .field("has_loaded_page", &self.has_loaded_page())
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for BrowserContext {
    type Target = TargetPageState;

    fn deref(&self) -> &Self::Target {
        self.active_page_state()
    }
}

impl std::ops::DerefMut for BrowserContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.active_page_state_mut()
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl BrowserContext {
    pub(crate) fn active_page_state(&self) -> &TargetPageState {
        self.page_targets
            .active()
            .expect("BrowserContext has no active page target")
            .state()
    }

    pub(crate) fn active_page_state_mut(&mut self) -> &mut TargetPageState {
        self.page_targets
            .active_mut()
            .expect("BrowserContext has no active page target")
            .state_mut()
    }

    /// Parks a dialog only until the matching lightweight-popup target obtains
    /// a concrete protocol attachment.
    ///
    /// This is not a generic activity backlog: every value owns one popup id
    /// and one one-shot renderer completion. Removing the browser context or
    /// forgetting the popup mapping drops and dismisses the value.
    pub(crate) fn park_pending_popup_javascript_dialog(
        &mut self,
        dialog: TargetPreparedJavaScriptDialog,
    ) {
        let popup_id = dialog
            .popup_id()
            .expect("only lightweight-popup dialogs may enter popup attachment residence");
        self.pending_popup_javascript_dialogs
            .entry(popup_id)
            .or_default()
            .push(dialog);
    }

    pub(crate) fn take_pending_popup_javascript_dialogs(
        &mut self,
        popup_id: u64,
    ) -> Vec<TargetPreparedJavaScriptDialog> {
        self.pending_popup_javascript_dialogs
            .remove(&popup_id)
            .unwrap_or_default()
    }

    pub(crate) fn dismiss_pending_popup_javascript_dialogs(&mut self, popup_id: u64) {
        drop(self.take_pending_popup_javascript_dialogs(popup_id));
    }

    pub fn new(id: String) -> Self {
        let context = Self::new_with_initial_cookies(id, Vec::new());
        #[cfg(test)]
        let context = {
            let mut context = context;
            let inserted = context.page_targets.insert(PageTargetHost::empty(
                TEST_FIXTURE_PAGE_TARGET_ID.to_owned(),
            ));
            debug_assert!(inserted);
            let selected = context.page_targets.select(TEST_FIXTURE_PAGE_TARGET_ID);
            debug_assert!(selected);
            context
        };
        context
    }

    pub(crate) fn new_with_initial_cookies(
        id: String,
        initial_cookies: impl IntoIterator<Item = StoredCookie>,
    ) -> Self {
        Self::new_with_storage_partition_and_http_cache(
            id,
            BrowserContextStoragePartitionHandles::with_initial_cookies(initial_cookies),
            None,
            None,
        )
    }

    pub(crate) fn new_ephemeral_with_storage_partition_handles(
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        let storage_partition = BrowserContextStoragePartitionIdentity::ephemeral(id.as_str());
        Self::new_with_storage_partition_and_storage_partition_identity(
            id,
            partition,
            http_cache_root,
            http_cache_max_bytes,
            storage_partition,
        )
    }

    pub(crate) fn new_ephemeral_with_http_cache(
        id: String,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self::new_ephemeral_with_storage_partition_handles(
            id,
            BrowserContextStoragePartitionHandles::memory(),
            http_cache_root,
            http_cache_max_bytes,
        )
    }

    pub(crate) fn new_with_storage_partition_and_http_cache(
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self::new_with_storage_partition_and_storage_partition_identity(
            id,
            partition,
            http_cache_root,
            http_cache_max_bytes,
            BrowserContextStoragePartitionIdentity::profile_backed_default(),
        )
    }

    pub(crate) fn new_with_storage_partition_handles_and_http_cache(
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self::new_with_storage_partition_and_http_cache(
            id,
            partition,
            http_cache_root,
            http_cache_max_bytes,
        )
    }

    fn new_with_storage_partition_and_storage_partition_identity(
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
        identity: BrowserContextStoragePartitionIdentity,
    ) -> Self {
        let storage_partition = BrowserContextStoragePartition::new(identity, partition);

        Self {
            id,
            storage_partition,
            page_targets: PageTargetRegistry::default(),
            auxiliary_target_sessions: HashMap::new(),
            target_opener_ids: HashMap::new(),
            target_opener_frame_ids: HashMap::new(),
            target_can_access_opener: HashSet::new(),
            target_window_names: HashMap::new(),
            target_popup_ids: HashMap::new(),
            pending_popup_javascript_dialogs: HashMap::new(),
            shared_worker_targets: BTreeMap::new(),
            dedicated_worker_targets: BTreeMap::new(),
            service_worker_targets: BTreeMap::new(),
            service_worker_domain_sessions: BTreeSet::new(),
            default_extra_headers: Vec::new(),
            global_extra_headers: Vec::new(),
            default_browser_identity: super::BaseBrowserIdentityOverrideState::default(),
            default_locale_override: None,
            default_timezone_override: None,
            default_network_conditions: None,
            default_geolocation_override: None,
            global_network_conditions: None,
            global_geolocation_override: None,
            global_cache_disabled: false,
            default_http_proxy_override: None,
            default_http_no_proxy_override: None,
            default_tls_verify_host_override: None,
            proxy_autoconfig_url: None,
            proxy_socks_version: None,
            default_emulated_device_metrics: None,
            next_default_document_start_script_id: 0,
            default_document_start_scripts: Vec::new(),
            storage_quota_overrides: HashMap::new(),
            http_cache_root,
            http_cache_max_bytes,
            page_navigation_runtime_config: None,
            renderer_output_transport_sender: None,
            renderer_runtime_owner: Some(RendererBrowserContextRuntime::new()),
        }
    }

    pub(crate) fn new_page_navigation_engine(
        &self,
        config: NavigationRuntimeConfig,
    ) -> NavigationEngine {
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

    pub(crate) fn bind_page_navigation_engines(
        &mut self,
        config: NavigationRuntimeConfig,
        renderer_output_transport_sender: Option<moli_core::RendererOutputTransportSender>,
    ) {
        self.page_navigation_runtime_config = Some(config.clone());
        self.renderer_output_transport_sender = renderer_output_transport_sender;

        let renderer_runtime = self.renderer_runtime_owner_access();
        let sender = self.renderer_output_transport_sender.clone();
        for host in self.page_targets.iter_mut() {
            if host.navigation_engine().is_some() {
                continue;
            }
            let engine = NavigationEngine::new_with_runtime_config_and_browser_context_access(
                config.clone(),
                renderer_runtime.clone(),
            )
            .expect("live BrowserContext owner must accept a page engine");
            if let Some(sender) = sender.clone() {
                engine.set_renderer_output_transport_sender(sender);
            }
            let replaced = host.replace_navigation_engine(engine);
            debug_assert!(replaced.is_none());
        }
    }

    pub(crate) fn set_renderer_output_transport_sender(
        &mut self,
        sender: moli_core::RendererOutputTransportSender,
    ) {
        self.renderer_output_transport_sender = Some(sender.clone());
        for host in self.page_targets.iter() {
            if let Some(engine) = host.navigation_engine() {
                engine.set_renderer_output_transport_sender(sender.clone());
            }
        }
    }

    pub(crate) fn page_navigation_engine(&self, target_id: &str) -> Option<&NavigationEngine> {
        self.page_target(target_id)?.navigation_engine()
    }

    pub(crate) fn page_navigation_engine_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut NavigationEngine> {
        self.page_target_mut(target_id)?.navigation_engine_mut()
    }

    pub(crate) fn is_profile_backed_storage_partition(&self) -> bool {
        self.storage_partition.identity().kind()
            == BrowserContextStoragePartitionKind::ProfileBacked
    }

    pub(crate) fn storage_partition_id(&self) -> &str {
        self.storage_partition.identity().id()
    }

    pub(crate) fn storage_partition_kind_label(&self) -> &'static str {
        self.storage_partition.identity().kind_label()
    }

    pub(crate) fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        let session_storage_store = self
            .page_targets
            .active()
            .map(|target| target.session_storage_namespace.store().clone())
            .unwrap_or_else(new_shared_web_storage_store);
        self.storage_partition
            .handles
            .resource_storage_handles(session_storage_store)
    }

    pub(crate) fn page_storage_handles(&self) -> BrowserContextPageStorageHandles {
        let session_storage_store = self
            .page_targets
            .active()
            .map(|target| target.session_storage_namespace.store().clone())
            .unwrap_or_else(new_shared_web_storage_store);
        self.storage_partition
            .handles
            .page_storage_handles(session_storage_store)
    }

    pub(crate) fn page_storage_handles_for_target(
        &self,
        target_id: &str,
    ) -> Option<BrowserContextPageStorageHandles> {
        if self.is_active_target(target_id) {
            return Some(self.page_storage_handles());
        }
        let target = self.background_target(target_id)?;
        Some(
            self.storage_partition
                .handles
                .page_storage_handles(target.session_storage_store().clone()),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_cookie_store<R>(&self, f: impl FnOnce(&BrowserCookieStore) -> R) -> R {
        let cookie_store = self.storage_partition.cookie_store().lock();
        f(&cookie_store)
    }

    pub(crate) fn with_cookie_store_mut<R>(
        &self,
        f: impl FnOnce(&mut BrowserCookieStore) -> R,
    ) -> R {
        let mut cookie_store = self.storage_partition.cookie_store().lock();
        f(&mut cookie_store)
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_generation(&self) -> u64 {
        self.with_cookie_store(|store| store.document_cookie_generation())
    }

    pub(crate) fn observe_request_cookie_access_report(
        &self,
        request_url: &url::Url,
        request_context: NetworkCookieRequestContext,
    ) -> Option<StoredCookieQueryReport> {
        let mut cookie_store = self.storage_partition.cookie_store().lock();
        let report =
            cookie_store.observe_cookie_access_report_for_request(request_url, request_context);
        (!report.included_cookies.is_empty() || !report.excluded_cookies.is_empty())
            .then_some(report)
    }

    pub(crate) fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        self.storage_quota_overrides
            .get(origin)
            .copied()
            .map(|quota| (quota, true))
            .unwrap_or((
                moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES as f64,
                false,
            ))
    }

    pub(crate) fn set_storage_quota_override(&mut self, origin: String, quota: f64) {
        self.storage_quota_overrides.insert(origin, quota);
    }

    pub(crate) fn clear_storage_quota_override(&mut self, origin: &str) {
        self.storage_quota_overrides.remove(origin);
    }

    pub(crate) fn storage_usage_for_origin(
        &self,
        serialized_origin: &str,
    ) -> Result<BrowserContextOriginStorageUsage, String> {
        let local_storage_usage = {
            let store = self.storage_partition.web_storage_store().lock();
            usize_to_u64_saturating(store.usage_bytes_for_origin_areas(serialized_origin))
        };
        let indexed_db_usage = moli_core::storage::indexed_db_origins_with_prefix_usage_bytes(
            self.storage_partition.indexed_db_manager(),
            &moli_storage_key::storage_key_prefix_for_origin(serialized_origin),
        )?;
        let storage_buckets_usage = self.storage_bucket_usage_for_origin(serialized_origin)?;
        Ok(BrowserContextOriginStorageUsage {
            local_storage_usage,
            indexed_db_usage,
            storage_buckets_usage,
            total_usage: local_storage_usage
                .saturating_add(indexed_db_usage)
                .saturating_add(storage_buckets_usage),
        })
    }

    fn storage_bucket_usage_for_origin(&self, serialized_origin: &str) -> Result<u64, String> {
        let (bucket_identities, cache_usage, storage_service) = {
            let store = self.storage_partition.storage_bucket_store().lock();
            (
                store.bucket_identities_for_origin_areas(serialized_origin),
                store.cache_usage_for_origin_areas(serialized_origin),
                store.storage_service(),
            )
        };
        let mut usage = cache_usage;
        for identity in bucket_identities {
            let indexed_db_usage = moli_core::storage::indexed_db_origin_usage_bytes(
                self.storage_partition.indexed_db_manager(),
                &identity.indexed_db_storage_key(),
            )?;
            let opfs_usage = storage_service
                .opfs_usage(&identity.locator())
                .map_err(|error| format!("FailedToReadStorageBucketOpfsUsage: {error}"))?;
            usage = usage
                .saturating_add(indexed_db_usage)
                .saturating_add(opfs_usage);
        }
        Ok(usage)
    }

    fn complete_storage_bucket_deletions(
        &self,
        cleanups: Vec<StorageBucketIdentity>,
    ) -> Result<(), String> {
        let bucket_store = self.storage_partition.storage_bucket_store();
        for cleanup in cleanups {
            moli_core::storage::complete_storage_bucket_deletion(bucket_store, &cleanup)
                .map_err(|error| format!("FailedToCompleteStorageBucketDeletion: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn renderer_runtime(&self) -> RendererBrowserContextRuntime {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
    }

    pub(crate) fn renderer_runtime_owner_access(&self) -> RendererBrowserContextRuntimeOwnerAccess {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .owner_access()
    }

    pub(crate) fn take_renderer_runtime_owner_for_teardown(
        &mut self,
    ) -> Option<RendererBrowserContextRuntimeOwner> {
        self.renderer_runtime_owner.take()
    }

    pub(crate) fn routes_renderer_browser_context_runtime(
        &self,
        runtime_id: moli_core::RendererBrowserContextRuntimeId,
    ) -> bool {
        self.renderer_runtime().id() == runtime_id
    }

    pub(crate) fn target_id_for_renderer_owner_local_host_id(
        &self,
        owner_local_host_id: moli_core::RendererOwnerLocalHostId,
    ) -> Option<String> {
        if self
            .loaded_page()
            .is_some_and(|page| page.renderer_owner_local_host_id() == owner_local_host_id)
        {
            return self.active_target_id_owned();
        }
        self.background_targets().find_map(|target| {
            target
                .loaded_page()
                .is_some_and(|page| page.renderer_owner_local_host_id() == owner_local_host_id)
                .then(|| target.target_id().to_owned())
        })
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        let target_infos = self.devtools_target_infos();
        let loaded_document_page_count = self.loaded_document_page_count();
        let pending_document_page_build_count = self.pending_document_page_build_count();
        let loaded_document_renderer_owner_count = self
            .loaded_document_renderer_owner_ids_for_diagnostics()
            .len();
        let estimated_document_isolate_count =
            loaded_document_page_count + pending_document_page_build_count;
        let page_target_pending_inspector_await_count =
            self.page_target_pending_inspector_await_count_for_diagnostics();
        let shared_worker_target_pending_inspector_await_count =
            self.shared_worker_target_pending_inspector_await_count_for_diagnostics();
        let service_worker_target_pending_inspector_await_count =
            self.service_worker_target_pending_inspector_await_count_for_diagnostics();
        let active_target = self.page_targets.active();
        let runtime_session_diagnostics = active_target
            .map(|target| {
                let primary = target.devtools_sessions.primary();
                let auxiliary_pending_inspector_await_count: usize = target
                    .devtools_sessions
                    .attached_states()
                    .map(DevToolsSessionState::pending_inspector_await_count)
                    .sum();
                json!({
                    "runtimeEnabled": primary.runtime_session_state.runtime_frontend_enabled,
                    "inspectorEnabled": primary.runtime_session_state.inspector_enabled,
                    "inspectorTargetCrashedDelivered": primary.runtime_session_state.inspector_target_crashed_delivered(),
                    "profilerCommandStateSource": "renderer-v8-inspector-agent",
                    "v8InspectorStateBytes": primary.inspector_session_state.v8_state.as_ref().map_or(0, |state| state.len()),
                    "auxiliaryDevToolsSessionStateCount": target.devtools_sessions.attached_len(),
                    "pendingInspectorAwaitCount": target.devtools_sessions.pending_inspector_await_count(),
                    "primaryPendingInspectorAwaitCount": primary.pending_inspector_await_count(),
                    "auxiliaryPendingInspectorAwaitCount": auxiliary_pending_inspector_await_count,
                })
            })
            .unwrap_or(Value::Null);
        let page_session_diagnostics = active_target
            .map(|target| {
                let primary = target.devtools_sessions.primary();
                json!({
                    "pageLifecycleEvents": primary.page_session_state.page_lifecycle_events,
                    "logEnabled": primary.page_session_state.log_enabled,
                    "consoleEnabled": primary.console_output_session_state.console_enabled,
                    "performanceEnabled": primary
                        .page_session_state
                        .performance
                        .enabled(),
                    "performanceTimeDomain": primary
                        .page_session_state
                        .performance
                        .time_domain()
                        .as_str(),
                    "pageFontFamilyCount": primary.page_session_state.page_font_families.len(),
                })
            })
            .unwrap_or(Value::Null);
        let target_host_state_diagnostics = json!({
            "targetHostCount": self.background_target_count(),
            "pageSessionStateCount": self.background_targets()
                .filter(|target| target.state().has_non_default_session_state())
                .count(),
            "targetOwnerStateWithPendingInspectorAwaitCount": self.background_targets()
                .filter(|target| target.has_pending_inspector_awaits())
                .count(),
            "pendingInspectorAwaitCount": self.background_targets()
                .map(|target| target.pending_inspector_await_count())
                .sum::<usize>(),
            "nonEmptyFetchStateCount": self.background_targets()
                .filter(|target| !target.fetch_owner.pending_state().is_empty())
                .count(),
            "ownerStates": self.background_targets().map(|target| json!({
                "targetId": target.target_id(),
                "ownerState": target.owner_state.moli_memory_diagnostics(),
            })).collect::<Vec<_>>(),
        });
        json!({
            "id": self.id,
            "storagePartition": {
                "kind": self.storage_partition_kind_label(),
                "id": self.storage_partition_id(),
            },
            "hasActiveTarget": self.has_active_target(),
            "activeTargetId": self.active_target_id(),
            "hasActiveSession": self.has_active_session(),
            "activeLoadedPage": self.has_loaded_page(),
            "activePageAttachment": self.page_attachment_id().map(|attachment_id| json!({
                "id": attachment_id.get(),
                "targetId": self.active_target_id(),
            })),
            "backgroundTargetCount": self.background_target_count(),
            "backgroundLoadedPageCount": self
                .background_targets()
                .filter(|target| target.has_loaded_page())
                .count(),
            "targetInfoCount": target_infos.len(),
            "attachedTargetInfoCount": target_infos
                .iter()
                .filter(|info| info.attached)
                .count(),
            "auxiliaryTargetSessionCount": self.auxiliary_target_sessions.len(),
            "targetOpenerCount": self.target_opener_ids.len(),
            "targetOpenerFrameCount": self.target_opener_frame_ids.len(),
            "targetCanAccessOpenerCount": self.target_can_access_opener.len(),
            "targetWindowNameCount": self.target_window_names.len(),
            "defaultDocumentStartScriptCount": self.default_document_start_scripts.len(),
            "domRemoteObjectNodeCacheCount": active_target
                .map_or(0, |target| target.dom_remote_object_node_cache.len()),
            "sharedWorkerTargetCount": self.shared_worker_targets.len(),
            "serviceWorkerTargetCount": self.service_worker_targets.len(),
            "pendingInspectorAwaitCount": page_target_pending_inspector_await_count
                + shared_worker_target_pending_inspector_await_count
                + service_worker_target_pending_inspector_await_count,
            "pageTargetPendingInspectorAwaitCount": page_target_pending_inspector_await_count,
            "pageTargetWithPendingInspectorAwaitCount": self
                .page_target_with_pending_inspector_await_count_for_diagnostics(),
            "sharedWorkerTargetPendingInspectorAwaitCount": shared_worker_target_pending_inspector_await_count,
            "sharedWorkerTargetWithPendingInspectorAwaitCount": self
                .shared_worker_target_with_pending_inspector_await_count_for_diagnostics(),
            "serviceWorkerTargetPendingInspectorAwaitCount": service_worker_target_pending_inspector_await_count,
            "serviceWorkerTargetWithPendingInspectorAwaitCount": self
                .service_worker_target_with_pending_inspector_await_count_for_diagnostics(),
            "isolateScope": {
                "documentPageAccountingModel": "browser-context-page-count",
                "loadedDocumentPageCount": loaded_document_page_count,
                "loadedDocumentRendererOwnerCount": loaded_document_renderer_owner_count,
                "pendingDocumentPageBuildCount": pending_document_page_build_count,
                "estimatedDocumentIsolateCount": estimated_document_isolate_count,
                "sharedWorkerTargetCount": self.shared_worker_targets.len(),
                "serviceWorkerTargetCount": self.service_worker_targets.len(),
                "browserContextRuntime": self.renderer_runtime().moli_memory_diagnostics(),
            },
            "runtimeSession": runtime_session_diagnostics,
            "pageSession": page_session_diagnostics,
            "activeRuntimeSlot": active_target
                .map(|target| target.runtime_slot.moli_memory_diagnostics()),
            "activeFetch": active_target
                .map(|target| target.fetch_owner.moli_memory_diagnostics()),
            "activeOwnerState": active_target
                .map(|target| target.owner_state.moli_memory_diagnostics()),
            "targetHosts": target_host_state_diagnostics,
        })
    }

    pub(crate) fn loaded_document_page_count(&self) -> usize {
        usize::from(self.has_loaded_page())
            + self
                .background_targets()
                .filter(|target| target.has_loaded_page())
                .count()
    }

    pub(crate) fn pending_document_page_build_count(&self) -> usize {
        usize::from(
            self.has_active_target()
                && self
                    .active_target
                    .runtime_slot
                    .has_pending_initial_document_page_build(),
        ) + self
            .background_targets()
            .filter(|target| {
                target
                    .runtime_slot()
                    .has_pending_initial_document_page_build()
            })
            .count()
    }

    pub(crate) fn target_has_pending_initial_document_page_build(&self, target_id: &str) -> bool {
        if self.active_target_id() == Some(target_id) {
            return self
                .active_target
                .runtime_slot
                .has_pending_initial_document_page_build();
        }
        self.background_target(target_id).is_some_and(|target| {
            target
                .runtime_slot()
                .has_pending_initial_document_page_build()
        })
    }

    pub(crate) fn target_transient_no_page_reason_for_protocol_output(
        &self,
        target_id: &str,
    ) -> Option<&'static str> {
        if self.active_target_id() == Some(target_id) {
            return self
                .active_target
                .runtime_slot
                .transient_no_page_reason_for_protocol_output();
        }
        self.background_target(target_id).and_then(|target| {
            target
                .runtime_slot()
                .transient_no_page_reason_for_protocol_output()
        })
    }

    pub(crate) fn assert_target_materialized_initial_empty_document_has_page(
        &self,
        target_id: &str,
    ) -> Result<(), String> {
        if self.active_target_id() == Some(target_id) {
            return materialized_initial_empty_document_missing_page_error(
                target_id,
                &self.active_target.owner_state,
                self.has_loaded_page(),
            )
            .map_or(Ok(()), Err);
        }

        let Some(target) = self.background_target(target_id) else {
            return Ok(());
        };
        let Some(owner_state) = self.parked_target_owner_state(target_id) else {
            return Ok(());
        };
        materialized_initial_empty_document_missing_page_error(
            target_id,
            owner_state,
            target.has_loaded_page(),
        )
        .map_or(Ok(()), Err)
    }

    pub(crate) fn can_install_current_initial_empty_document_page(&self, target_id: &str) -> bool {
        if self.active_target_id() == Some(target_id) {
            return !self.has_loaded_page()
                && self
                    .active_target
                    .owner_state
                    .can_install_current_initial_empty_document_page();
        }

        let Some(target) = self.background_target(target_id) else {
            return false;
        };
        if target.has_loaded_page() {
            return false;
        }
        self.parked_target_owner_state(target_id)
            .is_none_or(TargetOwnerState::can_install_current_initial_empty_document_page)
    }

    pub(crate) fn loaded_document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        let mut owner_ids = HashSet::new();
        if let Some(page) = self.loaded_page() {
            owner_ids.insert(page.renderer_owner_local_host_id().as_u64());
        }
        for target in self.background_targets() {
            if let Some(page) = target.loaded_page() {
                owner_ids.insert(page.renderer_owner_local_host_id().as_u64());
            }
        }
        owner_ids
    }

    pub(crate) fn pending_document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        HashSet::new()
    }

    pub(crate) fn document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        let mut owner_ids = self.loaded_document_renderer_owner_ids_for_diagnostics();
        owner_ids.extend(self.pending_document_renderer_owner_ids_for_diagnostics());
        owner_ids
    }

    pub(crate) fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self.loaded_pages_for_diagnostics()
            .map(
                moli_core::page::Page::dedicated_worker_running_worker_isolate_count_for_diagnostics,
            )
            .sum()
    }

    fn loaded_pages_for_diagnostics(&self) -> impl Iterator<Item = &moli_core::page::Page> {
        self.loaded_page().into_iter().chain(
            self.background_targets()
                .filter_map(|target| target.loaded_page()),
        )
    }

    pub(crate) fn page_target_pending_inspector_await_count_for_diagnostics(&self) -> usize {
        self.page_targets
            .iter()
            .map(|target| target.pending_inspector_await_count())
            .sum()
    }

    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.page_targets
            .iter()
            .any(PageTargetHost::has_pending_javascript_dialog)
    }

    pub(crate) fn page_target_with_pending_inspector_await_count_for_diagnostics(&self) -> usize {
        self.page_targets
            .iter()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn shared_worker_target_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.shared_worker_targets
            .values()
            .map(SharedWorkerTargetState::pending_inspector_await_count_all_sessions)
            .sum()
    }

    pub(crate) fn shared_worker_target_with_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.shared_worker_targets
            .values()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn service_worker_target_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.service_worker_targets
            .values()
            .map(ServiceWorkerTargetState::pending_inspector_await_count_all_sessions)
            .sum()
    }

    pub(crate) fn service_worker_target_with_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.service_worker_targets
            .values()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn shared_worker_runtime_diagnostics_for_diagnostics(
        &self,
    ) -> RendererSharedWorkerRuntimeDiagnostics {
        self.renderer_runtime()
            .shared_worker_runtime_diagnostics_for_diagnostics()
    }

    #[cfg(test)]
    pub(crate) fn start_document_navigation_for_active_target(
        &mut self,
        loader_id: String,
    ) -> Option<DocumentNavigationToken> {
        let target_id = self.active_target_id()?.to_owned();
        let token = self
            .active_target
            .runtime_slot
            .start_document_navigation(target_id.clone(), loader_id);
        self.mark_target_initial_empty_document_pending_cross_document_navigation(&target_id);
        Some(token)
    }

    pub(crate) fn start_document_navigation_for_target(
        &mut self,
        target_id: &str,
        loader_id: String,
    ) -> Option<DocumentNavigationToken> {
        if self.active_target_id() == Some(target_id) {
            let target_id = target_id.to_owned();
            let token = self
                .active_target
                .runtime_slot
                .start_document_navigation(target_id.clone(), loader_id);
            self.mark_target_initial_empty_document_pending_cross_document_navigation(&target_id);
            return Some(token);
        }
        let target = self.background_target_mut(target_id)?;
        let token = target
            .runtime_slot
            .start_document_navigation(target_id.to_owned(), loader_id);
        self.mark_target_initial_empty_document_pending_cross_document_navigation(target_id);
        Some(token)
    }

    pub(crate) fn accepts_pending_document_navigation_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            return self
                .active_target
                .runtime_slot
                .accepts_pending_document_navigation_event(token);
        }
        self.background_target(&token.target_id)
            .is_some_and(|target| {
                target
                    .runtime_slot()
                    .accepts_pending_document_navigation_event(token)
            })
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &DocumentNavigationToken,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            return self
                .active_target
                .runtime_slot
                .document_navigation_cancellation_handle(token);
        }
        self.background_target(&token.target_id).and_then(|target| {
            target
                .runtime_slot()
                .document_navigation_cancellation_handle(token)
        })
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            return self
                .active_target
                .runtime_slot
                .arm_background_navigation_completion(token, additional_cancellation);
        }
        let Some(target) = self.background_target_mut(&token.target_id) else {
            if let Some(cancellation) = additional_cancellation {
                cancellation.cancel();
            }
            return false;
        };
        target
            .runtime_slot
            .arm_background_navigation_completion(token, additional_cancellation)
    }

    pub(crate) fn settle_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            return self
                .active_target
                .runtime_slot
                .settle_background_navigation_completion(token);
        }
        self.background_target_mut(&token.target_id)
            .is_some_and(|target| {
                target
                    .runtime_slot
                    .settle_background_navigation_completion(token)
            })
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.page_targets
            .iter()
            .any(|target| target.runtime_slot().has_inflight_background_navigation())
    }

    pub(crate) fn has_inflight_background_navigation_for_target(&self, target_id: &str) -> bool {
        if self.active_target_id() == Some(target_id) {
            return self
                .active_target
                .runtime_slot
                .has_inflight_background_navigation();
        }
        self.background_target(target_id)
            .is_some_and(|target| target.runtime_slot().has_inflight_background_navigation())
    }

    pub(crate) fn accepts_document_body_completion_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            return self
                .active_target
                .runtime_slot
                .accepts_document_body_completion_event(token);
        }
        self.background_target(&token.target_id)
            .is_some_and(|target| {
                target
                    .runtime_slot()
                    .accepts_document_body_completion_event(token)
            })
    }

    pub(crate) fn has_pending_document_navigation_for_target(
        &self,
        target_id: Option<&str>,
    ) -> bool {
        let Some(target_id) = target_id else {
            return false;
        };
        if self.active_target_id() == Some(target_id) {
            return self
                .active_target
                .runtime_slot
                .has_pending_document_navigation();
        }
        self.background_target(target_id)
            .is_some_and(|target| target.runtime_slot().has_pending_document_navigation())
    }

    pub(crate) fn clear_pending_document_navigation_for_target_if_loader_matches(
        &mut self,
        target_id: Option<&str>,
        loader_id: &str,
    ) {
        let Some(target_id) = target_id else {
            return;
        };
        if self.active_target_id() == Some(target_id) {
            let cleared = self
                .active_target
                .runtime_slot
                .clear_pending_document_navigation_if_loader_matches(loader_id);
            if cleared {
                self.clear_target_initial_empty_document_pending_cross_document_navigation(
                    target_id,
                );
            }
            return;
        }
        if let Some(target) = self.background_target_mut(target_id) {
            let cleared = target
                .runtime_slot
                .clear_pending_document_navigation_if_loader_matches(loader_id);
            if cleared {
                self.clear_target_initial_empty_document_pending_cross_document_navigation(
                    target_id,
                );
            }
        }
    }

    pub(crate) fn commit_document_navigation_if_matches(
        &mut self,
        token: &DocumentNavigationToken,
    ) {
        if self.active_target_id() == Some(token.target_id.as_str()) {
            let committed = self
                .active_target
                .runtime_slot
                .commit_pending_document_navigation_if_matches(token);
            if committed {
                self.mark_target_initial_empty_document_exited(&token.target_id);
            }
            return;
        }
        if let Some(target) = self.background_target_mut(&token.target_id) {
            let committed = target
                .runtime_slot
                .commit_pending_document_navigation_if_matches(token);
            if committed {
                self.mark_target_initial_empty_document_exited(&token.target_id);
            }
        }
    }

    pub(crate) fn clear_document_navigation_state_for_active_target(&mut self) {
        self.active_target
            .runtime_slot
            .clear_document_navigation_state();
    }

    pub(crate) fn clear_indexed_db_origin(&self, origin: &str) -> Result<(), String> {
        moli_core::storage::clear_indexed_db_origin(
            self.storage_partition.indexed_db_manager(),
            origin,
        )
    }

    pub(crate) fn clear_site_data_for_origin(
        &mut self,
        origin: &url::Url,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        if options.cookies
            && let Some(host) = origin.host_str().map(str::to_ascii_lowercase)
        {
            let mut cookie_store = self.storage_partition.cookie_store().lock();
            cookie_store.delete_cookies(None, None, None, Some(host.as_str()));
        }

        let serialized_origin = origin.origin().ascii_serialization();
        if options.local_storage {
            let mut store = self.storage_partition.web_storage_store().lock();
            store
                .try_clear_origin_areas(&serialized_origin)
                .map_err(|error| format!("FailedToClearLocalStorage: {error}"))?;
        }

        if options.indexed_db {
            moli_core::storage::clear_indexed_db_origins_with_prefix(
                self.storage_partition.indexed_db_manager(),
                &moli_storage_key::storage_key_prefix_for_origin(&serialized_origin),
            )?;
        }

        if options.storage_buckets {
            let cleanups = self
                .storage_partition
                .storage_bucket_store()
                .lock()
                .clear_origin_areas(&serialized_origin)
                .map_err(|error| format!("FailedToClearStorageBuckets: {error}"))?;
            self.complete_storage_bucket_deletions(cleanups)?;
        }

        if options.http_cache {
            self.clear_http_cache_for_origin(origin)?;
        }

        Ok(())
    }

    pub(crate) fn clear_site_data_for_storage_key(
        &mut self,
        storage_key: &moli_storage_key::MoliStorageKey,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        let origin = url::Url::parse(storage_key.origin())
            .map_err(|error| format!("UnableToDeserializeStorageKeyOrigin: {error}"))?;
        if origin.origin().ascii_serialization() != storage_key.origin() {
            return Err("UnableToDeserializeStorageKeyOrigin".to_owned());
        }

        if options.cookies
            && let Some(host) = origin.host_str().map(str::to_ascii_lowercase)
        {
            let mut cookie_store = self.storage_partition.cookie_store().lock();
            cookie_store.delete_cookies(None, None, None, Some(host.as_str()));
        }

        let serialized_storage_key = storage_key.serialized_storage_key();
        if options.local_storage {
            let mut store = self.storage_partition.web_storage_store().lock();
            store
                .try_clear_origin(&serialized_storage_key)
                .map_err(|error| format!("FailedToClearLocalStorage: {error}"))?;
        }

        if options.indexed_db {
            self.clear_indexed_db_origin(&serialized_storage_key)?;
        }

        if options.storage_buckets {
            let cleanups = self
                .storage_partition
                .storage_bucket_store()
                .lock()
                .clear_origin(&serialized_storage_key)
                .map_err(|error| format!("FailedToClearStorageBuckets: {error}"))?;
            self.complete_storage_bucket_deletions(cleanups)?;
        }

        if options.http_cache {
            self.clear_http_cache_for_origin(&origin)?;
        }

        Ok(())
    }

    pub(crate) fn clear_http_cache(&self) -> Result<(), String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(());
        };
        moli_fetch::clear_http_cache_root(cache_root, self.http_cache_max_bytes)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    pub(crate) fn clear_http_cache_for_origin(&self, origin: &url::Url) -> Result<usize, String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(0);
        };
        moli_fetch::clear_http_cache_root_for_origin(cache_root, self.http_cache_max_bytes, origin)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    pub(crate) fn snapshot_cookies(&self) -> Vec<StoredCookie> {
        self.storage_partition.cookie_store().lock().cookies()
    }

    #[cfg(test)]
    pub(crate) fn store_response_cookie_headers_for_test(
        &self,
        response_url: &url::Url,
        response_headers: &[(String, String)],
    ) {
        self.with_cookie_store_mut(|store| {
            store.store_response_headers(response_url, response_headers);
        });
    }

    #[cfg(test)]
    pub(crate) fn cookie_store_for_test(&self) -> &SharedBrowserCookieStore {
        self.storage_partition.cookie_store()
    }

    #[cfg(test)]
    pub(crate) fn web_storage_store_for_test(&self) -> &SharedWebStorageStore {
        self.storage_partition.web_storage_store()
    }

    #[cfg(test)]
    pub(crate) fn session_storage_store_for_test(&self) -> &SharedWebStorageStore {
        self.active_target.session_storage_namespace.store()
    }

    #[cfg(test)]
    pub(crate) fn indexed_db_manager_for_test(&self) -> &SharedIndexedDbManager {
        self.storage_partition.indexed_db_manager()
    }

    #[cfg(test)]
    pub(crate) fn storage_bucket_store_for_test(&self) -> &SharedStorageBucketStore {
        self.storage_partition.storage_bucket_store()
    }

    #[cfg(test)]
    pub(crate) fn replace_storage_bucket_store_for_test(
        &mut self,
        storage_bucket_store: SharedStorageBucketStore,
    ) {
        self.storage_partition
            .replace_storage_bucket_store(storage_bucket_store);
    }

    #[cfg(test)]
    pub(crate) fn upsert_cookie_for_test(
        &self,
        cookie: StoredCookie,
    ) -> moli_cookie_jar::StoredCookieSetReport {
        self.with_cookie_store_mut(|store| {
            store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp)
        })
    }

    #[cfg(test)]
    pub(crate) fn test_last_cookie_access_index(
        &self,
        domain: &str,
        path: &str,
        name: &str,
    ) -> Option<u64> {
        self.with_cookie_store(|store| store.test_last_access_index(domain, path, name))
    }

    #[cfg(test)]
    pub(crate) fn delete_cookies(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
    ) {
        self.delete_cookies_with_partition_key(name, domain, path, url_host, None);
    }

    pub(crate) fn delete_cookies_with_partition_key(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
        partition_key: Option<&moli_cookie_jar::StoredCookiePartitionKey>,
    ) {
        let mut cookie_store = self.storage_partition.cookie_store().lock();
        cookie_store.delete_cookies_with_partition_key(name, domain, path, url_host, partition_key);
    }

    pub(crate) fn active_target_id(&self) -> Option<&str> {
        let target_id = self.page_targets.active_target_id()?;
        #[cfg(test)]
        if target_id == TEST_FIXTURE_PAGE_TARGET_ID {
            return None;
        }
        Some(target_id)
    }

    pub(crate) fn active_target_id_owned(&self) -> Option<String> {
        self.active_target_id().map(str::to_owned)
    }

    pub(crate) fn effective_active_browser_identity_override(
        &self,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.page_targets
            .active()
            .and_then(|host| host.network_policy.browser_identity_override())
            .or_else(|| self.default_browser_identity.profile())
    }

    pub(crate) fn effective_active_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.effective_active_browser_identity_override().cloned()
    }

    pub(crate) fn default_browser_identity_override(
        &self,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.default_browser_identity.profile()
    }

    pub(crate) fn default_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.default_browser_identity.profile_owned()
    }

    pub(crate) fn set_default_user_agent_override(
        &mut self,
        user_agent: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.default_browser_identity
            .set_user_agent(user_agent, fallback);
    }

    pub(crate) fn set_default_locale_override(
        &mut self,
        locale: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.default_locale_override = locale.clone();
        self.default_browser_identity
            .set_accept_language(locale, fallback);
    }

    #[cfg(test)]
    pub(crate) fn replace_default_browser_identity_override_for_test(
        &mut self,
        identity: moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.default_browser_identity.replace_profile(identity);
    }

    pub(crate) fn effective_active_locale_override_owned(&self) -> Option<String> {
        self.page_targets
            .active()
            .and_then(|host| host.locale_override.clone())
            .or_else(|| self.default_locale_override.clone())
    }

    pub(crate) fn effective_active_timezone_override_owned(&self) -> Option<String> {
        self.page_targets
            .active()
            .and_then(|host| host.timezone_override.clone())
            .or_else(|| self.default_timezone_override.clone())
    }

    pub(crate) fn effective_active_network_conditions(&self) -> Option<EmulatedNetworkConditions> {
        self.page_targets
            .active()
            .and_then(|host| host.network_conditions)
            .or(self.default_network_conditions)
            .or(self.global_network_conditions)
    }

    pub(crate) fn effective_active_http_proxy_override_owned(&self) -> Option<String> {
        self.page_targets
            .active()
            .and_then(|host| host.http_proxy_override.clone())
            .or_else(|| self.default_http_proxy_override.clone())
    }

    pub(crate) fn effective_active_http_no_proxy_override_owned(&self) -> Option<String> {
        self.page_targets
            .active()
            .and_then(|host| host.http_no_proxy_override.clone())
            .or_else(|| self.default_http_no_proxy_override.clone())
    }

    pub(crate) fn effective_active_tls_verify_host_override(&self) -> Option<bool> {
        self.page_targets
            .active()
            .and_then(|host| host.tls_verify_host_override)
            .or(self.default_tls_verify_host_override)
    }

    pub(crate) fn effective_active_geolocation_override(
        &self,
    ) -> Option<EmulatedGeolocationOverrideState> {
        self.page_targets
            .active()
            .and_then(|host| host.geolocation_override.clone())
            .or_else(|| self.default_geolocation_override.clone())
            .or_else(|| self.global_geolocation_override.clone())
    }

    pub(crate) fn effective_active_network_offline(&self) -> bool {
        self.effective_active_network_conditions()
            .is_some_and(|conditions| !conditions.navigator_online())
    }

    pub(crate) fn effective_parked_network_conditions(
        &self,
        target_id: &str,
    ) -> Option<EmulatedNetworkConditions> {
        self.parked_page_session_state(target_id)
            .and_then(|state| state.network_conditions)
            .or(self.default_network_conditions)
            .or(self.global_network_conditions)
    }

    pub(crate) fn effective_parked_network_offline(&self, target_id: &str) -> bool {
        self.effective_parked_network_conditions(target_id)
            .is_some_and(|conditions| !conditions.navigator_online())
    }

    pub(crate) fn effective_parked_locale_override_owned(&self, target_id: &str) -> Option<String> {
        self.parked_page_session_state(target_id)
            .and_then(|state| state.locale_override.clone())
            .or_else(|| self.default_locale_override.clone())
    }

    pub(crate) fn has_active_target(&self) -> bool {
        self.page_targets.active().is_some()
    }

    pub(crate) fn is_active_target(&self, target_id: &str) -> bool {
        self.active_target_id() == Some(target_id)
    }

    pub(crate) fn set_active_target_id(&mut self, target_id: impl Into<String>) {
        let target_id = target_id.into();
        #[cfg(test)]
        if self.page_targets.active_target_id() == Some(TEST_FIXTURE_PAGE_TARGET_ID) {
            let rekeyed = self.page_targets.rekey_active(target_id.clone());
            debug_assert!(rekeyed, "test fixture page target id must be replaceable");
            return;
        }
        if self.page_targets.get(&target_id).is_none() {
            let inserted = self.insert_page_target_host(PageTargetHost::empty(target_id.clone()));
            debug_assert!(inserted, "new page target id must be unique");
        }
        let selected = self.page_targets.select(&target_id);
        debug_assert!(selected, "registered page target must be selectable");
    }

    pub(crate) fn rekey_active_target(&mut self, target_id: impl Into<String>) -> bool {
        self.page_targets.rekey_active(target_id.into())
    }

    #[cfg(test)]
    pub(crate) fn clear_active_target_id(&mut self) {
        self.page_targets.clear_selection();
    }

    pub(crate) fn active_session_id(&self) -> Option<&str> {
        self.page_targets.active()?.session_id()
    }

    pub(crate) fn active_session_id_owned(&self) -> Option<String> {
        self.active_session_id().map(str::to_owned)
    }

    pub(crate) fn has_active_session(&self) -> bool {
        self.active_session_id().is_some()
    }

    pub(crate) fn active_target_is_unclaimed_default_placeholder(
        &self,
        default_target_id: &str,
    ) -> bool {
        self.active_target_id() == Some(default_target_id)
            && !self.has_active_session()
            && !self.has_loaded_page()
            && self.background_target_count() == 0
            && self.shared_worker_targets.is_empty()
            && self.dedicated_worker_targets.is_empty()
            && self.service_worker_targets.is_empty()
    }

    pub(crate) fn attach_active_session(&mut self, session_id: impl Into<String>) {
        self.page_targets
            .active_mut()
            .expect("cannot attach a session without an active page target")
            .attach_session(session_id.into());
    }

    pub(crate) fn detach_active_session(&mut self) -> Option<String> {
        self.page_targets.active_mut()?.detach_session()
    }

    pub(crate) fn target_url(&self) -> &str {
        self.target_identity.url()
    }

    pub(crate) fn target_security_origin(&self) -> &str {
        self.target_identity.security_origin()
    }

    pub(crate) fn target_secure_context_type(&self) -> &str {
        self.target_identity.secure_context_type()
    }

    pub(crate) fn set_target_url(&mut self, url: String) {
        self.target_identity.set_url(url);
    }

    pub(crate) fn set_target_security_origin(&mut self, security_origin: String) {
        self.target_identity.set_security_origin(security_origin);
    }

    pub(crate) fn set_target_secure_context_type(&mut self, secure_context_type: String) {
        self.target_identity
            .set_secure_context_type(secure_context_type);
    }
}

fn materialized_initial_empty_document_missing_page_error(
    target_id: &str,
    owner_state: &TargetOwnerState,
    has_loaded_page: bool,
) -> Option<String> {
    if owner_state.has_materialized_current_initial_empty_document() && !has_loaded_page {
        return Some(format!(
            "TargetInitialEmptyDocumentMissingPage: target {target_id} has materialized current initial empty document without loaded Page"
        ));
    }
    None
}

pub(crate) fn seed_initial_cookies(
    cookie_store: &SharedBrowserCookieStore,
    initial_cookies: impl IntoIterator<Item = StoredCookie>,
) {
    let mut store = cookie_store.lock();
    for cookie in initial_cookies {
        let _ = store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
    }
}

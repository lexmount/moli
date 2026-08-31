use crate::{
    network::{ResourceRequestClient, SharedWebStorageStore},
    page::{
        CompletedPageCommand, DocumentStartScript, EmulatedMediaOverrides, NavigationResponse,
        Page, PendingPageCommand, PermissionOverrideRegistration,
        RendererInspectorSessionRestoreSnapshot, RendererMainDocumentCommit,
        RendererPageCreationArtifacts, RendererPageCreationDiagnostics,
        RendererPendingDownloadActivation, RuntimeBindingRegistration,
        RuntimeIsolatedWorldDefinition, SubresourceAuthCredentials, SubresourceResourceType,
        ViewportSurface,
    },
    renderer::ExternalRawDocumentBodyStream,
    renderer::JsRuntime,
    renderer::PageVmInitStage,
};
use anyhow::{Context, Result, anyhow};
use moli_cookie_jar::{SharedBrowserCookieStore, new_shared_browser_cookie_store};
use moli_fetch::{
    BrowserNavigationRequestKind, FetchCancelHandle, FetchConfig, NetworkFetchResult, Request,
    StreamingRawResponse, ensure_http_status_success,
};
use moli_page_types::{LayoutPolicy, OptionalResourceFetchMask};
use moli_renderer_v8::{
    RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner,
    RendererBrowserContextRuntimeOwnerAccess, RendererReservedServiceWorkerClient,
    RendererServiceWorkerMainResourceFetch, RendererWebStorageHandles, SharedStorageBucketStore,
    WeakIndexedDbManager,
    network::{
        BrowserResourceRuntime, BrowserResourceRuntimeOwner, PageNetworkPolicy,
        navigation::{DocumentFetchContextSeed, NavigationResourceLoader},
    },
};
use std::rc::Rc;
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone)]
struct DocumentPageLoadOptions {
    pub resource_source: CommittedDocumentResourceSource,
    pub root_frame_id: Option<String>,
    pub main_document_commit: Option<RendererMainDocumentCommit>,
    pub requested_url: Url,
    pub final_url: Url,
    pub navigation_initiator_url: Option<Url>,
    pub redirected: bool,
    pub redirect_count: usize,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub response_body: String,
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub extra_http_headers: Vec<(String, String)>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub cpu_throttling_rate: f64,
    pub emulated_media: EmulatedMediaOverrides,
    pub viewport_surface: Option<ViewportSurface>,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub fetch_subresource_interception_enabled: bool,
    pub fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
}

/// Explicit resource authority source for a newly committed top-level Document.
///
/// A real network navigation must carry the seed minted by its
/// `NavigationResourceLoader`. Synthetic Documents deliberately use the
/// NavigationEngine's current browser runtime and Page policy. Keeping these
/// cases distinct prevents a missing navigation seed from silently falling
/// back to ambient state.
#[derive(Debug, Clone)]
pub enum CommittedDocumentResourceSource {
    Navigation(Box<DocumentFetchContextSeed>),
    Synthetic,
}

fn streaming_raw_response_from_navigation_response(
    response: NavigationResponse,
) -> Result<StreamingRawResponse> {
    let head = response.head();
    let body = response.clone_body_bytes();
    let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
    if !body.is_empty() {
        body_tx
            .send(body)
            .map_err(|_| anyhow!("failed to enqueue service worker main resource body"))?;
    }
    drop(body_tx);
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    let _ = completion_tx.send(Ok(()));
    Ok(StreamingRawResponse::new_with_head(
        head,
        body_rx,
        FetchCancelHandle::new(),
        completion_rx,
    ))
}

#[derive(Clone)]
pub struct NavigationResourceStorageHandles {
    cookie_store: SharedBrowserCookieStore,
    web_storage_store: SharedWebStorageStore,
    session_storage_store: SharedWebStorageStore,
}

impl NavigationResourceStorageHandles {
    pub fn new(
        cookie_store: SharedBrowserCookieStore,
        web_storage_store: SharedWebStorageStore,
        session_storage_store: SharedWebStorageStore,
    ) -> Self {
        Self {
            cookie_store,
            web_storage_store,
            session_storage_store,
        }
    }

    fn into_cookie_store(self) -> SharedBrowserCookieStore {
        self.cookie_store
    }

    fn into_page_parts(self) -> (SharedBrowserCookieStore, RendererWebStorageHandles) {
        (
            self.cookie_store,
            RendererWebStorageHandles::new(self.web_storage_store, self.session_storage_store),
        )
    }
}

#[derive(Clone)]
pub struct NavigationPageStorageHandles {
    resource_storage: NavigationResourceStorageHandles,
    indexed_db_manager: Option<WeakIndexedDbManager>,
    storage_bucket_store: Option<SharedStorageBucketStore>,
}

pub struct NavigationStreamingRawResponse {
    pub fetch_result: NetworkFetchResult<StreamingRawResponse>,
    pub reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
    pub document_fetch_context_seed: DocumentFetchContextSeed,
}

impl NavigationPageStorageHandles {
    pub fn new(
        cookie_store: SharedBrowserCookieStore,
        web_storage_store: SharedWebStorageStore,
        session_storage_store: SharedWebStorageStore,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
    ) -> Self {
        Self {
            resource_storage: NavigationResourceStorageHandles::new(
                cookie_store,
                web_storage_store,
                session_storage_store,
            ),
            indexed_db_manager,
            storage_bucket_store,
        }
    }

    fn into_parts(
        self,
    ) -> (
        SharedBrowserCookieStore,
        RendererWebStorageHandles,
        Option<WeakIndexedDbManager>,
        Option<SharedStorageBucketStore>,
    ) {
        let (cookie_store, web_storage) = self.resource_storage.into_page_parts();
        (
            cookie_store,
            web_storage,
            self.indexed_db_manager,
            self.storage_bucket_store,
        )
    }
}

pub struct BuiltDocumentPage {
    pub page: Page,
    pub page_creation_diagnostics: RendererPageCreationDiagnostics,
    pub page_creation_artifacts: RendererPageCreationArtifacts,
    pub pending_download: Option<RendererPendingDownloadActivation>,
}

/// A renderer document whose isolate and DevTools agent are reserved, but
/// whose parser and execution contexts have not started.
pub struct PreparedDocumentPage {
    prepared: moli_renderer_v8::PreparedRendererDocument,
}

#[derive(Debug, Clone)]
pub struct PreparedDocumentPageCommitConfiguration {
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<PermissionOverrideRegistration>,
    pub extra_http_headers: Vec<(String, String)>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub cpu_throttling_rate: f64,
    pub emulated_media: EmulatedMediaOverrides,
    pub idle_override: Option<crate::page::EmulatedIdleOverride>,
    pub viewport_surface: Option<ViewportSurface>,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub fetch_subresource_interception: (bool, Option<SubresourceResourceType>),
}

impl std::fmt::Debug for PreparedDocumentPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedDocumentPage")
            .field(
                "renderer_devtools_agent_token",
                &self.prepared.renderer_devtools_agent_token(),
            )
            .finish_non_exhaustive()
    }
}

/// Typed authority to start one exact prepared renderer document.
#[derive(Debug)]
pub struct PreparedDocumentPageCommitPermit {
    permit: moli_renderer_v8::RendererDocumentCommitPermit,
}

impl PreparedDocumentPage {
    pub fn renderer_owner_local_host_id(&self) -> moli_renderer_v8::RendererOwnerLocalHostId {
        self.prepared.token().local_host_id()
    }

    pub fn renderer_page_id(&self) -> moli_renderer_v8::PageId {
        self.prepared.token().page_id()
    }

    pub fn renderer_devtools_agent_token(&self) -> crate::page::RendererDevToolsAgentToken {
        self.prepared.renderer_devtools_agent_token()
    }

    pub async fn update_commit_configuration(
        &self,
        configuration: PreparedDocumentPageCommitConfiguration,
    ) -> Result<()> {
        self.prepared
            .update_commit_configuration(
                moli_renderer_v8::RendererPreparedDocumentCommitConfiguration {
                    document_start_scripts: configuration.document_start_scripts,
                    runtime_bindings: configuration.runtime_bindings,
                    runtime_inspector_session_restore_snapshots: configuration
                        .runtime_inspector_session_restore_snapshots,
                    runtime_isolated_worlds: configuration.runtime_isolated_worlds,
                    permission_overrides: configuration.permission_overrides,
                    extra_http_headers: configuration.extra_http_headers,
                    locale_override: configuration.locale_override,
                    timezone_override: configuration.timezone_override,
                    script_execution_disabled: configuration.script_execution_disabled,
                    bypass_content_security_policy: configuration.bypass_content_security_policy,
                    cpu_throttling_rate: configuration.cpu_throttling_rate,
                    emulated_media: configuration.emulated_media,
                    idle_override: configuration.idle_override,
                    viewport_surface: configuration.viewport_surface,
                    network_offline: configuration.network_offline,
                    blocked_url_patterns: configuration.blocked_url_patterns,
                    fetch_subresource_interception_enabled: configuration
                        .fetch_subresource_interception
                        .0,
                    fetch_subresource_interception_resource_type: configuration
                        .fetch_subresource_interception
                        .1,
                },
            )
            .await
    }

    pub fn issue_commit_permit(&self) -> PreparedDocumentPageCommitPermit {
        PreparedDocumentPageCommitPermit {
            permit: self.prepared.issue_commit_permit(),
        }
    }

    pub async fn commit(
        self,
        permit: PreparedDocumentPageCommitPermit,
    ) -> Result<BuiltDocumentPage> {
        let (
            handle,
            page_state,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        ) = self
            .prepared
            .commit(permit.permit)
            .await
            .context("failed to commit prepared streaming raw page")?;
        Ok(BuiltDocumentPage {
            page: Page::from_attached_handle(handle, page_state),
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        })
    }

    pub async fn cancel(self) -> Result<()> {
        self.prepared.cancel().await
    }
}

/// Handle to an in-flight document page build started via
/// [`NavigationEngine::start_build_html_page_from_response`]. The renderer
/// thread is concurrently performing the V8 + parse work; the caller awaits
/// [`Self::await_ready`] when the resulting page is needed.
pub struct PendingBuiltDocumentPage {
    pending: moli_renderer_v8::PendingHtmlPage,
}

impl PendingBuiltDocumentPage {
    pub async fn await_ready(self) -> Result<BuiltDocumentPage> {
        let (
            handle,
            page_state,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        ) = self
            .pending
            .await_ready()
            .await
            .context("failed to build html page")?;
        Ok(BuiltDocumentPage {
            page: Page::from_attached_handle(handle, page_state),
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        })
    }
}

/// Configuration copied into each Page-owned or standalone NavigationEngine.
#[derive(Debug, Clone)]
pub struct NavigationRuntimeConfig {
    fetch_config: FetchConfig,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
    subframe_loading_enabled: bool,
    layout_policy: LayoutPolicy,
}

impl NavigationRuntimeConfig {
    pub fn new(
        fetch_config: FetchConfig,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
        layout_policy: LayoutPolicy,
    ) -> Self {
        Self {
            fetch_config,
            optional_resource_fetch_mask,
            subframe_loading_enabled,
            layout_policy,
        }
    }

    pub fn fetch_config(&self) -> &FetchConfig {
        &self.fetch_config
    }

    pub fn fetch_config_mut(&mut self) -> &mut FetchConfig {
        &mut self.fetch_config
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.optional_resource_fetch_mask
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.subframe_loading_enabled
    }

    pub fn layout_policy(&self) -> LayoutPolicy {
        self.layout_policy
    }
}

impl Default for NavigationRuntimeConfig {
    fn default() -> Self {
        Self::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::default(),
        )
    }
}

impl From<&crate::config::BrowserConfig> for NavigationRuntimeConfig {
    fn from(config: &crate::config::BrowserConfig) -> Self {
        Self::new(
            config.fetch().clone(),
            config.optional_resource_fetch_mask(),
            config.subframe_loading_enabled(),
            config.layout_policy(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct NavigationEngine {
    fetch_config: FetchConfig,
    page_network_policy: PageNetworkPolicy,
    layout_policy: LayoutPolicy,
    js_runtime: JsRuntime,
    resource_runtime: Option<BrowserResourceRuntime>,
    browser_context_access: RendererBrowserContextRuntimeOwnerAccess,
    // Standalone engines share this last-drop owner. BrowserContext engines
    // leave it empty and borrow only the context's weak, bound access.
    standalone_lifetime_owner: Option<Rc<NavigationEngineLifetimeOwner>>,
}

#[derive(Debug)]
struct NavigationEngineLifetimeOwner {
    js_runtime: Option<JsRuntime>,
    browser_context_owner: RendererBrowserContextRuntimeOwner,
}

impl Drop for NavigationEngineLifetimeOwner {
    fn drop(&mut self) {
        if let Some(js_runtime) = self.js_runtime.take() {
            js_runtime.terminate_resource_producers_for_owner_shutdown();
            drop(js_runtime);
        }
        self.browser_context_owner.shutdown_and_join();
    }
}

impl Default for NavigationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationEngine {
    /// Reserves a renderer Page identity before the corresponding creation
    /// command is enqueued.
    ///
    /// A protocol owner can bind this identity to its pending target before
    /// parser or author-script output becomes publishable.
    pub fn reserve_page_for_creation(&self) -> moli_renderer_v8::RendererPageReservationToken {
        self.js_runtime.reserve_page_for_creation()
    }

    pub fn new() -> Self {
        Self::new_with_runtime_config(NavigationRuntimeConfig::default())
    }

    pub fn new_with_fetch_config(fetch_config: FetchConfig) -> Self {
        Self::new_with_runtime_config(NavigationRuntimeConfig::new(
            fetch_config,
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::default(),
        ))
    }

    pub fn new_with_fetch_config_and_image_fetch_enabled(
        fetch_config: FetchConfig,
        image_fetch_enabled: bool,
    ) -> Self {
        let mask = if image_fetch_enabled {
            OptionalResourceFetchMask::IMAGE
        } else {
            OptionalResourceFetchMask::NONE
        };
        Self::new_with_fetch_config_and_resource_loading(fetch_config, mask, true)
    }

    pub fn new_with_fetch_config_and_resource_loading(
        fetch_config: FetchConfig,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> Self {
        Self::new_with_runtime_config(NavigationRuntimeConfig::new(
            fetch_config,
            optional_resource_fetch_mask,
            subframe_loading_enabled,
            LayoutPolicy::default(),
        ))
    }

    pub fn new_with_runtime_config(config: NavigationRuntimeConfig) -> Self {
        let resource_runtime = BrowserResourceRuntimeOwner::new(
            config.fetch_config(),
            new_shared_browser_cookie_store(),
        );
        let browser_context_owner =
            RendererBrowserContextRuntime::new_owned_with_service_worker_resource_store_and_browser_resource_runtime(
                moli_renderer_v8::new_shared_service_worker_resource_store(),
                resource_runtime,
            );
        let browser_context_access = browser_context_owner.owner_access();
        Self::new_with_runtime_config_and_browser_context_access_inner(
            config,
            browser_context_access,
            Some(browser_context_owner),
        )
        .expect("standalone browser context owner must remain live during construction")
    }

    pub fn new_with_fetch_config_and_browser_context_access(
        fetch_config: FetchConfig,
        browser_context_access: RendererBrowserContextRuntimeOwnerAccess,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> Result<Self> {
        Self::new_with_runtime_config_and_browser_context_access(
            NavigationRuntimeConfig::new(
                fetch_config,
                optional_resource_fetch_mask,
                subframe_loading_enabled,
                LayoutPolicy::default(),
            ),
            browser_context_access,
        )
    }

    pub fn new_with_runtime_config_and_browser_context_access(
        config: NavigationRuntimeConfig,
        browser_context_access: RendererBrowserContextRuntimeOwnerAccess,
    ) -> Result<Self> {
        Self::new_with_runtime_config_and_browser_context_access_inner(
            config,
            browser_context_access,
            None,
        )
    }

    fn new_with_runtime_config_and_browser_context_access_inner(
        config: NavigationRuntimeConfig,
        browser_context_access: RendererBrowserContextRuntimeOwnerAccess,
        standalone_browser_context_owner: Option<RendererBrowserContextRuntimeOwner>,
    ) -> Result<Self> {
        let NavigationRuntimeConfig {
            fetch_config,
            optional_resource_fetch_mask,
            subframe_loading_enabled,
            layout_policy,
        } = config;
        let resource_runtime = browser_context_access
            .current_browser_resource_runtime()
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        let js_runtime =
            JsRuntime::initialize_with_browser_context_owner_access(&browser_context_access)?;
        js_runtime
            .renderer_owner_handle()
            .configure_layout_policy(layout_policy)?;
        let standalone_lifetime_owner = standalone_browser_context_owner.map(|owner| {
            Rc::new(NavigationEngineLifetimeOwner {
                js_runtime: Some(js_runtime.clone()),
                browser_context_owner: owner,
            })
        });
        Ok(Self {
            fetch_config,
            page_network_policy: PageNetworkPolicy::new(
                optional_resource_fetch_mask,
                subframe_loading_enabled,
            ),
            layout_policy,
            js_runtime,
            resource_runtime: Some(resource_runtime),
            browser_context_access,
            standalone_lifetime_owner,
        })
    }

    pub fn new_with_fetch_config_and_shared_renderer_owner(
        fetch_config: FetchConfig,
        renderer_owner_source: &Self,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> Result<Self> {
        Self::new_with_runtime_config_and_shared_renderer_owner(
            NavigationRuntimeConfig::new(
                fetch_config,
                optional_resource_fetch_mask,
                subframe_loading_enabled,
                renderer_owner_source.layout_policy,
            ),
            renderer_owner_source,
        )
    }

    pub fn new_with_runtime_config_and_shared_renderer_owner(
        config: NavigationRuntimeConfig,
        renderer_owner_source: &Self,
    ) -> Result<Self> {
        let NavigationRuntimeConfig {
            fetch_config,
            optional_resource_fetch_mask,
            subframe_loading_enabled,
            layout_policy,
        } = config;
        renderer_owner_source
            .js_runtime
            .renderer_owner_handle()
            .configure_layout_policy(layout_policy)?;
        let resource_runtime = renderer_owner_source
            .browser_context_access
            .current_browser_resource_runtime()
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        Ok(Self {
            fetch_config,
            page_network_policy: PageNetworkPolicy::new(
                optional_resource_fetch_mask,
                subframe_loading_enabled,
            ),
            layout_policy,
            js_runtime: renderer_owner_source.js_runtime.clone(),
            resource_runtime: Some(resource_runtime),
            browser_context_access: renderer_owner_source.browser_context_access.clone(),
            standalone_lifetime_owner: renderer_owner_source.standalone_lifetime_owner.clone(),
        })
    }

    pub fn new_with_page_vm_document_isolate_for_diagnostics() -> Self {
        Self::new()
    }

    pub fn browser_context_runtime(&self) -> RendererBrowserContextRuntime {
        self.js_runtime.browser_context_runtime()
    }

    pub fn browser_context_owner_access(&self) -> RendererBrowserContextRuntimeOwnerAccess {
        self.browser_context_access.clone()
    }

    pub fn terminate_renderer_producers_for_owner_shutdown(&self) {
        self.js_runtime
            .terminate_resource_producers_for_owner_shutdown();
    }

    pub fn document_isolate_accounting_for_diagnostics(
        &self,
    ) -> crate::page::RendererDocumentIsolateAccountingDiagnostics {
        self.js_runtime
            .document_isolate_accounting_for_diagnostics()
    }

    pub fn document_isolate_model_for_diagnostics(&self) -> &'static str {
        self.js_runtime.document_isolate_model_for_diagnostics()
    }

    pub fn renderer_owner_id_for_diagnostics(&self) -> u64 {
        self.js_runtime.renderer_owner_id_for_diagnostics()
    }

    pub fn shares_renderer_owner_with(&self, other: &Self) -> bool {
        self.js_runtime
            .shares_renderer_owner_with(&other.js_runtime)
    }

    pub fn set_renderer_output_transport_sender(
        &self,
        sender: crate::RendererOutputTransportSender,
    ) {
        self.js_runtime.set_renderer_output_transport_sender(sender);
    }

    pub fn fetch_config(&self) -> &FetchConfig {
        &self.fetch_config
    }

    pub fn runtime_config(&self) -> NavigationRuntimeConfig {
        NavigationRuntimeConfig::new(
            self.fetch_config.clone(),
            self.optional_resource_fetch_mask(),
            self.subframe_loading_enabled(),
            self.layout_policy,
        )
    }

    pub fn layout_policy(&self) -> LayoutPolicy {
        self.layout_policy
    }

    pub fn image_fetch_enabled(&self) -> bool {
        self.page_network_policy
            .optional_resource_fetch_mask()
            .contains(OptionalResourceFetchMask::IMAGE)
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.page_network_policy.optional_resource_fetch_mask()
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.page_network_policy.subframe_loading_enabled()
    }

    pub fn set_bypass_service_worker(&mut self, bypass: bool) {
        self.page_network_policy.set_bypass_service_worker(bypass);
    }

    fn ensure_resource_runtime(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<BrowserResourceRuntime> {
        let current_runtime = self
            .browser_context_access
            .current_browser_resource_runtime()
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        if self
            .resource_runtime
            .as_ref()
            .is_none_or(|cached| !cached.shares_state_with(&current_runtime))
        {
            // An ambient NavigationEngine cache is not a Document lease. Once
            // another wrapper replaces the context binding, its next request
            // must converge on that current runtime instead of reviving a
            // retired backend merely because the cookie-store Arc matches.
            self.resource_runtime = Some(current_runtime);
        }
        let should_rebuild = self.resource_runtime.as_ref().is_none_or(|runtime| {
            !Arc::ptr_eq(&runtime.cookie_store(), &cookie_store)
                || !runtime.matches_fetch_config(&self.fetch_config)
        });
        if should_rebuild {
            let registration = BrowserResourceRuntimeOwner::new(&self.fetch_config, cookie_store);
            let resource_runtime = self
                .browser_context_access
                .replace_owned(registration)
                .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
            self.resource_runtime = Some(resource_runtime);
            self.browser_context_access.reap_retired_resource_runtimes();
        }
        Ok(self
            .resource_runtime
            .as_ref()
            .expect("resource runtime must exist after ensure")
            .clone())
    }

    fn ensure_resource_request_client(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<ResourceRequestClient> {
        Ok(
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                self.ensure_resource_runtime(cookie_store)?,
                self.page_network_policy.clone(),
            ),
        )
    }

    fn resource_request_client_for_committed_document(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        expected_final_url: &Url,
        source: CommittedDocumentResourceSource,
    ) -> Result<ResourceRequestClient> {
        let CommittedDocumentResourceSource::Navigation(seed) = source else {
            let expected_runtime = self.ensure_resource_runtime(cookie_store)?;
            return Ok(
                ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                    expected_runtime,
                    self.page_network_policy.clone(),
                ),
            );
        };
        if seed.final_url() != expected_final_url {
            anyhow::bail!(
                "navigation commit seed final URL `{}` does not match committed Document `{expected_final_url}`",
                seed.final_url(),
            );
        }
        let committed_runtime = seed.browser_resource_runtime();
        if self
            .browser_context_access
            .validate_registered(&committed_runtime)
            .is_err()
            || !Arc::ptr_eq(&committed_runtime.cookie_store(), &cookie_store)
            || !committed_runtime.matches_fetch_config(&self.fetch_config)
        {
            anyhow::bail!(
                "navigation commit seed does not belong to the NavigationEngine browser resource runtime"
            );
        }
        // Another target sharing this BrowserContext may replace the ambient
        // transport binding while this request is in flight. The immutable
        // navigation seed remains the exact authority for the committed
        // Document as long as its runtime is still registered under this
        // owner and matches the target's captured policy.
        self.resource_runtime = Some(committed_runtime.clone());
        Ok(
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                committed_runtime,
                seed.page_network_policy(),
            ),
        )
    }

    pub fn resource_request_client(&self) -> Option<ResourceRequestClient> {
        self.resource_runtime.as_ref().map(|runtime| {
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                runtime.clone(),
                self.page_network_policy.clone(),
            )
        })
    }

    /// Adopts only the browser-level transport/cache runtime. The target keeps
    /// its own Page network policy.
    pub fn adopt_registered_resource_runtime(
        &mut self,
        resource_runtime: BrowserResourceRuntime,
    ) -> Result<()> {
        self.browser_context_access
            .adopt_registered(resource_runtime.clone())
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        self.resource_runtime = Some(resource_runtime);
        self.browser_context_access.reap_retired_resource_runtimes();
        Ok(())
    }

    fn ensure_resource_runtime_ready(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<()> {
        self.ensure_resource_runtime(cookie_store).map(|_| ())
    }

    pub fn ensure_resource_runtime_ready_for_navigation_storage(
        &mut self,
        storage: NavigationResourceStorageHandles,
    ) -> Result<()> {
        self.ensure_resource_runtime_ready(storage.cookie_store)
    }

    fn ensure_resource_request_client_clone(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<ResourceRequestClient> {
        self.ensure_resource_request_client(cookie_store)
    }

    fn rebuild_resource_request_client(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<ResourceRequestClient> {
        // Rebuild is an explicit transport/config replacement, not an
        // `ensure` cache miss. The cookie-store identity may be unchanged when
        // CDP updates only user agent, proxy, TLS, or another FetchConfig
        // field, so consulting the current binding here would incorrectly
        // preserve the old backend.
        let resource_runtime = self
            .browser_context_access
            .replace_owned(BrowserResourceRuntimeOwner::new(
                &self.fetch_config,
                cookie_store,
            ))
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        self.resource_runtime = Some(resource_runtime.clone());
        self.browser_context_access.reap_retired_resource_runtimes();
        Ok(
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                resource_runtime,
                self.page_network_policy.clone(),
            ),
        )
    }

    pub fn rebuild_resource_request_client_for_navigation_storage(
        &mut self,
        storage: NavigationResourceStorageHandles,
    ) -> Result<ResourceRequestClient> {
        self.rebuild_resource_request_client(storage.cookie_store)
    }

    fn ensure_cookie_store(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<SharedBrowserCookieStore> {
        Ok(self.ensure_resource_runtime(cookie_store)?.cookie_store())
    }

    fn navigation_resource_loader(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        requested_url: Url,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NavigationResourceLoader> {
        Ok(NavigationResourceLoader::new_with_cancel_handle(
            self.ensure_resource_request_client(cookie_store)?,
            requested_url,
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()?,
            cancel_handle,
        ))
    }

    pub fn ensure_cookie_store_for_navigation_storage(
        &mut self,
        storage: NavigationResourceStorageHandles,
    ) -> Result<SharedBrowserCookieStore> {
        self.ensure_cookie_store(storage.cookie_store)
    }

    async fn fetch_navigation_response_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        initiator_url: Option<&Url>,
        browser_navigation_kind: BrowserNavigationRequestKind,
        infer_referrer_from_initiator: bool,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<NavigationResponse>> {
        let mut request = Request::new_bytes(method, raw_url, body, request_headers)
            .map(|request| {
                let request = request.with_top_level_navigation_cookie_context();
                let mut request = request.with_browser_navigation_kind(browser_navigation_kind);
                if !infer_referrer_from_initiator {
                    request = request.without_inferred_referrer();
                }
                if let Some(initiator_url) = initiator_url {
                    request.with_initiator_url(initiator_url)
                } else {
                    request
                }
            })
            .context("failed to build request")?;
        request.set_auth(auth.map(Into::into));
        let navigation_loader = self.navigation_resource_loader(
            cookie_store,
            request.url.clone(),
            FetchCancelHandle::new(),
        )?;
        navigation_loader
            .fetch_with_network_metadata(request)
            .await
            .map(|result| result.map_response(NavigationResponse::from))
    }

    pub async fn fetch_navigation_response_with_storage_async(
        &mut self,
        storage: NavigationResourceStorageHandles,
        initiator_url: Option<&Url>,
        browser_navigation_kind: BrowserNavigationRequestKind,
        infer_referrer_from_initiator: bool,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<NavigationResponse>> {
        let cookie_store = storage.into_cookie_store();
        self.fetch_navigation_response_async(
            cookie_store,
            initiator_url,
            browser_navigation_kind,
            infer_referrer_from_initiator,
            method,
            raw_url,
            body.map(String::into_bytes),
            request_headers,
            auth,
        )
        .await
    }

    async fn fetch_navigation_streaming_raw_response_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        initiator_url: Option<&Url>,
        browser_navigation_kind: BrowserNavigationRequestKind,
        infer_referrer_from_initiator: bool,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NavigationStreamingRawResponse> {
        let mut request = Request::new_bytes(method, raw_url, body, request_headers)
            .map(|request| {
                let request = request.with_top_level_navigation_cookie_context();
                let mut request = request.with_browser_navigation_kind(browser_navigation_kind);
                if !infer_referrer_from_initiator {
                    request = request.without_inferred_referrer();
                }
                if let Some(initiator_url) = initiator_url {
                    request.with_initiator_url(initiator_url)
                } else {
                    request
                }
            })
            .context("failed to build request")?;
        request.set_auth(auth.map(Into::into));
        let navigation_loader =
            self.navigation_resource_loader(cookie_store, request.url.clone(), cancel_handle)?;
        let service_worker_fetch = self
            .browser_context_runtime()
            .fetch_service_worker_main_resource_for_navigation(&request, &navigation_loader)
            .await?;
        let RendererServiceWorkerMainResourceFetch {
            reserved_client,
            response,
        } = service_worker_fetch;
        if let Some(response) = response {
            navigation_loader.note_service_worker_response_ready()?;
            let final_url = response.final_url.clone();
            return Ok(NavigationStreamingRawResponse {
                fetch_result: NetworkFetchResult::without_request_observation(
                    streaming_raw_response_from_navigation_response(response)?,
                ),
                reserved_service_worker_client: reserved_client,
                document_fetch_context_seed: navigation_loader.commit(final_url)?,
            });
        }
        let fetch_result = navigation_loader
            .fetch_raw_stream_with_network_metadata(request)
            .await?;
        let final_url = fetch_result.response().final_url.clone();
        Ok(NavigationStreamingRawResponse {
            fetch_result,
            reserved_service_worker_client: reserved_client,
            document_fetch_context_seed: navigation_loader.commit(final_url)?,
        })
    }

    pub async fn fetch_navigation_streaming_raw_response_with_storage_async(
        &mut self,
        storage: NavigationResourceStorageHandles,
        initiator_url: Option<&Url>,
        browser_navigation_kind: BrowserNavigationRequestKind,
        infer_referrer_from_initiator: bool,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NavigationStreamingRawResponse> {
        let cookie_store = storage.into_cookie_store();
        self.fetch_navigation_streaming_raw_response_async(
            cookie_store,
            initiator_url,
            browser_navigation_kind,
            infer_referrer_from_initiator,
            method,
            raw_url,
            body.map(String::into_bytes),
            request_headers,
            auth,
            FetchCancelHandle::new(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn fetch_navigation_streaming_raw_response_bytes_with_storage_async(
        &mut self,
        storage: NavigationResourceStorageHandles,
        initiator_url: Option<&Url>,
        browser_navigation_kind: BrowserNavigationRequestKind,
        infer_referrer_from_initiator: bool,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NavigationStreamingRawResponse> {
        let cookie_store = storage.into_cookie_store();
        self.fetch_navigation_streaming_raw_response_async(
            cookie_store,
            initiator_url,
            browser_navigation_kind,
            infer_referrer_from_initiator,
            method,
            raw_url,
            body,
            request_headers,
            auth,
            cancel_handle,
        )
        .await
    }

    pub fn ensure_navigation_response_status(
        &self,
        raw_url: &str,
        status: u16,
        allow_auth_challenge: bool,
    ) -> Result<()> {
        ensure_http_status_success(raw_url, status, allow_auth_challenge)
            .map_err(|error| anyhow!(error.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_inline_html_document_page_best_effort_with_inspector_session_restores_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        document_url: Url,
        navigation_initiator_url: Option<Url>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        let built = self
            .build_html_page_from_response_options_async(
                cookie_store,
                web_storage,
                indexed_db_manager,
                storage_bucket_store,
                DocumentPageLoadOptions {
                    resource_source: CommittedDocumentResourceSource::Synthetic,
                    root_frame_id,
                    main_document_commit,
                    requested_url: document_url.clone(),
                    final_url: document_url,
                    navigation_initiator_url,
                    redirected: false,
                    redirect_count: 0,
                    response_status: 200,
                    response_headers: Vec::new(),
                    response_body,
                    document_start_scripts,
                    runtime_bindings,
                    runtime_inspector_session_restore_snapshots,
                    extra_http_headers,
                    locale_override,
                    timezone_override,
                    script_execution_disabled,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate,
                    emulated_media,
                    viewport_surface,
                    network_offline,
                    blocked_url_patterns,
                    fetch_subresource_interception_enabled,
                    fetch_subresource_interception_resource_type,
                },
            )
            .await?;
        if built.pending_download.is_some() {
            return Err(anyhow!(
                "document page creation produced a pending download side-effect for a caller that expects a pure page"
            ));
        }
        Ok(built)
    }

    pub async fn build_inline_html_document_page_with_storage_best_effort_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        document_url: Url,
        navigation_initiator_url: Option<Url>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
    ) -> Result<BuiltDocumentPage> {
        self.build_inline_html_document_page_with_storage_and_inspector_session_restores_best_effort_async(
            storage,
            document_url,
            navigation_initiator_url,
            response_body,
            document_start_scripts,
            runtime_bindings,
            Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_inline_html_document_page_with_storage_and_inspector_session_restores_best_effort_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        document_url: Url,
        navigation_initiator_url: Option<Url>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.build_inline_html_document_page_best_effort_with_inspector_session_restores_async(
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            document_url,
            navigation_initiator_url,
            response_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            root_frame_id,
            main_document_commit,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_html_page_from_response_with_inspector_session_restores_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        self.build_html_page_from_response_options_async(
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            DocumentPageLoadOptions {
                resource_source: CommittedDocumentResourceSource::Synthetic,
                root_frame_id,
                main_document_commit,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                response_body,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
            },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_html_page_from_response_with_storage_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
    ) -> Result<BuiltDocumentPage> {
        self.build_html_page_from_response_with_storage_and_inspector_session_restores_async(
            storage,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            response_body,
            document_start_scripts,
            runtime_bindings,
            Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            false,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_html_page_from_response_with_storage_and_inspector_session_restores_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.build_html_page_from_response_with_inspector_session_restores_async(
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            response_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            root_frame_id,
            main_document_commit,
        )
        .await
    }

    /// Fire-and-defer variant of
    /// [`Self::build_html_page_from_response_with_inspector_session_restores_async`].
    ///
    /// Performs the synchronous loader setup and enqueues the renderer command,
    /// returning a [`PendingBuiltDocumentPage`] without awaiting the renderer
    /// reply. Use this to overlap renderer-side V8/parse work with subsequent
    /// conn-side bookkeeping.
    #[allow(clippy::too_many_arguments)]
    fn start_build_html_page_from_response(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<PendingBuiltDocumentPage> {
        let loader = self.ensure_resource_request_client(cookie_store)?;
        loader.set_extra_http_headers(&extra_http_headers);
        loader.set_network_offline(network_offline);
        loader.set_blocked_url_patterns(&blocked_url_patterns);
        let pending = self
            .js_runtime
            .start_create_html_page_from_response_with_inspector_session_restores(
                page_reservation,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                &loader,
                web_storage,
                response_body,
                indexed_db_manager,
                storage_bucket_store,
                document_start_scripts,
                runtime_bindings,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                runtime_inspector_session_restore_snapshots,
                root_frame_id,
                top_level_storage_key,
                moli_renderer_v8::RendererTopLevelNavigationDispatch::DelegateToBrowser,
                main_document_commit,
            )?;
        Ok(PendingBuiltDocumentPage { pending })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_build_html_page_from_response_with_storage(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
    ) -> Result<PendingBuiltDocumentPage> {
        let page_reservation = self.reserve_page_for_creation();
        self.start_build_html_page_from_response_with_storage_and_inspector_session_restores(
            page_reservation,
            storage,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            response_body,
            document_start_scripts,
            runtime_bindings,
            Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            false,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_build_html_page_from_response_with_storage_and_inspector_session_restores(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<PendingBuiltDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.start_build_html_page_from_response(
            page_reservation,
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            response_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            root_frame_id,
            top_level_storage_key,
            main_document_commit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_streaming_raw_page_from_external_body_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        stage: PageVmInitStage,
        reply_boundary: moli_renderer_v8::RendererReplyBoundary,
        root_frame_id: Option<String>,
        resource_source: CommittedDocumentResourceSource,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        let page_reservation = self.reserve_page_for_creation();
        let prepared = self
            .prepare_streaming_raw_page_from_external_body_async(
                page_reservation,
                cookie_store,
                web_storage,
                indexed_db_manager,
                storage_bucket_store,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                raw_body,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                false,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                stage,
                reply_boundary,
                root_frame_id,
                resource_source,
                reserved_service_worker_client,
                main_document_commit,
            )
            .await?;
        let permit = prepared.issue_commit_permit();
        prepared.commit(permit).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_streaming_raw_page_from_external_body_async(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        stage: PageVmInitStage,
        reply_boundary: moli_renderer_v8::RendererReplyBoundary,
        root_frame_id: Option<String>,
        resource_source: CommittedDocumentResourceSource,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<PreparedDocumentPage> {
        let loader = self.resource_request_client_for_committed_document(
            cookie_store,
            &final_url,
            resource_source,
        )?;
        loader.set_extra_http_headers(&extra_http_headers);
        loader.set_network_offline(network_offline);
        loader.set_blocked_url_patterns(&blocked_url_patterns);
        let prepared = self
            .js_runtime
            .prepare_streaming_raw_document_from_external_body_with_inspector_session_restores(
                page_reservation,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                Vec::new(),
                response_status,
                response_headers,
                &loader,
                web_storage,
                raw_body,
                indexed_db_manager,
                storage_bucket_store,
                document_start_scripts,
                runtime_bindings,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                runtime_inspector_session_restore_snapshots,
                false,
                stage,
                reply_boundary,
                moli_renderer_v8::RendererTopLevelNavigationDispatch::DelegateToBrowser,
                moli_renderer_v8::RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
                root_frame_id,
                reserved_service_worker_client,
                main_document_commit,
                None,
            )
            .await
            .context("failed to prepare streaming raw page")?;
        Ok(PreparedDocumentPage { prepared })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_streaming_raw_page_from_external_body_with_storage_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        stage: PageVmInitStage,
    ) -> Result<BuiltDocumentPage> {
        self.build_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
            storage,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            stage,
            moli_renderer_v8::RendererReplyBoundary::Stage,
            None,
            CommittedDocumentResourceSource::Synthetic,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        stage: PageVmInitStage,
        reply_boundary: moli_renderer_v8::RendererReplyBoundary,
        root_frame_id: Option<String>,
        resource_source: CommittedDocumentResourceSource,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<BuiltDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.build_streaming_raw_page_from_external_body_async(
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            stage,
            reply_boundary,
            root_frame_id,
            resource_source,
            reserved_service_worker_client,
            main_document_commit,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        stage: PageVmInitStage,
        reply_boundary: moli_renderer_v8::RendererReplyBoundary,
        root_frame_id: Option<String>,
        resource_source: CommittedDocumentResourceSource,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<PreparedDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.prepare_streaming_raw_page_from_external_body_async(
            page_reservation,
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            requested_url,
            final_url,
            navigation_initiator_url,
            redirected,
            redirect_count,
            response_status,
            response_headers,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            stage,
            reply_boundary,
            root_frame_id,
            resource_source,
            reserved_service_worker_client,
            main_document_commit,
        )
        .await
    }

    async fn build_html_page_from_response_options_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        options: DocumentPageLoadOptions,
    ) -> Result<BuiltDocumentPage> {
        let loader = self.resource_request_client_for_committed_document(
            cookie_store,
            &options.final_url,
            options.resource_source.clone(),
        )?;
        loader.set_extra_http_headers(&options.extra_http_headers);
        loader.set_network_offline(options.network_offline);
        loader.set_blocked_url_patterns(&options.blocked_url_patterns);
        let (
            handle,
            page_state,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        ) = self
            .js_runtime
            .create_html_page_from_response_with_inspector_session_restores(
                options.requested_url.clone(),
                options.final_url,
                options.navigation_initiator_url,
                options.redirected,
                options.redirect_count,
                options.response_status,
                options.response_headers,
                &loader,
                web_storage,
                options.response_body,
                indexed_db_manager,
                storage_bucket_store,
                options.document_start_scripts,
                options.runtime_bindings,
                options.extra_http_headers,
                options.locale_override,
                options.timezone_override,
                options.script_execution_disabled,
                options.bypass_content_security_policy,
                options.cpu_throttling_rate,
                options.emulated_media,
                options.viewport_surface,
                options.network_offline,
                options.blocked_url_patterns,
                options.fetch_subresource_interception_enabled,
                options.fetch_subresource_interception_resource_type,
                options.runtime_inspector_session_restore_snapshots,
                options.root_frame_id,
                options.main_document_commit,
            )
            .await
            .context("failed to build html page")?;
        Ok(BuiltDocumentPage {
            page: Page::from_attached_handle(handle, page_state),
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        })
    }

    async fn prepare_document_page_from_response_options_best_effort_async(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        options: DocumentPageLoadOptions,
        stage: PageVmInitStage,
    ) -> Result<PreparedDocumentPage> {
        let raw_body =
            ExternalRawDocumentBodyStream::from_bytes(options.response_body.into_bytes());
        self.prepare_streaming_raw_page_from_external_body_async(
            page_reservation,
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            options.requested_url,
            options.final_url,
            options.navigation_initiator_url,
            options.redirected,
            options.redirect_count,
            options.response_status,
            options.response_headers,
            raw_body,
            options.document_start_scripts,
            options.runtime_bindings,
            options.runtime_inspector_session_restore_snapshots,
            options.extra_http_headers,
            options.locale_override,
            options.timezone_override,
            options.script_execution_disabled,
            options.bypass_content_security_policy,
            options.cpu_throttling_rate,
            options.emulated_media,
            options.viewport_surface,
            options.network_offline,
            options.blocked_url_patterns,
            options.fetch_subresource_interception_enabled,
            options.fetch_subresource_interception_resource_type,
            stage,
            moli_renderer_v8::RendererReplyBoundary::Stage,
            options.root_frame_id,
            options.resource_source,
            None,
            options.main_document_commit,
        )
        .await
    }

    async fn build_document_page_from_response_options_best_effort_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        web_storage: RendererWebStorageHandles,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        options: DocumentPageLoadOptions,
    ) -> Result<BuiltDocumentPage> {
        let page_reservation = self.reserve_page_for_creation();
        let prepared = self
            .prepare_document_page_from_response_options_best_effort_async(
                page_reservation,
                cookie_store,
                web_storage,
                indexed_db_manager,
                storage_bucket_store,
                options,
                PageVmInitStage::Load,
            )
            .await?;
        let permit = prepared.issue_commit_permit();
        prepared.commit(permit).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_document_page_from_response_with_storage_and_inspector_session_restores_async(
        &mut self,
        page_reservation: moli_renderer_v8::RendererPageReservationToken,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        root_frame_id: Option<String>,
        resource_source: CommittedDocumentResourceSource,
        main_document_commit: Option<RendererMainDocumentCommit>,
    ) -> Result<PreparedDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.prepare_document_page_from_response_options_best_effort_async(
            page_reservation,
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            DocumentPageLoadOptions {
                resource_source,
                root_frame_id,
                main_document_commit,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                response_body,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
            },
            PageVmInitStage::DomContentLoaded,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_document_page_from_response_with_storage_async(
        &mut self,
        storage: NavigationPageStorageHandles,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::page::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: EmulatedMediaOverrides,
        viewport_surface: Option<ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
        resource_source: CommittedDocumentResourceSource,
    ) -> Result<BuiltDocumentPage> {
        let (cookie_store, web_storage, indexed_db_manager, storage_bucket_store) =
            storage.into_parts();
        self.build_document_page_from_response_options_best_effort_async(
            cookie_store,
            web_storage,
            indexed_db_manager,
            storage_bucket_store,
            DocumentPageLoadOptions {
                resource_source,
                root_frame_id: None,
                main_document_commit: None,
                requested_url,
                final_url,
                navigation_initiator_url,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                response_body,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots: Vec::new(),
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy: false,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
            },
        )
        .await
    }

    fn start_page_child_frame_lifecycle_work_best_effort(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        page: &Page,
        timeout: std::time::Duration,
    ) -> Result<PendingPageCommand> {
        let loader = self.ensure_resource_request_client_clone(cookie_store)?;
        page.start_complete_child_frame_lifecycle_work_best_effort(&loader, timeout)
    }

    pub fn start_page_child_frame_lifecycle_work_with_storage_best_effort(
        &mut self,
        storage: NavigationResourceStorageHandles,
        page: &Page,
        timeout: std::time::Duration,
    ) -> Result<PendingPageCommand> {
        self.start_page_child_frame_lifecycle_work_best_effort(
            storage.into_cookie_store(),
            page,
            timeout,
        )
    }

    pub fn complete_page_child_frame_lifecycle_work_best_effort(
        &mut self,
        page: &mut Page,
        completion: CompletedPageCommand,
    ) -> Result<(bool, moli_renderer_v8::RendererCommandTurnOutput)> {
        page.finish_complete_child_frame_lifecycle_work_best_effort_command_turn(completion)
    }

    pub async fn reset_resource_runtime_async(&mut self, loaded_page: Option<&mut Page>) {
        self.resource_runtime = None;
        self.browser_context_access.reap_retired_resource_runtimes();
        if let Some(page) = loaded_page {
            let _ = page.retire_document_resource_authorities_async().await;
            self.browser_context_access.reap_retired_resource_runtimes();
        }
    }

    pub fn reset_resource_runtime_without_loaded_page(&mut self) {
        self.resource_runtime = None;
        self.browser_context_access.reap_retired_resource_runtimes();
    }

    async fn rebuild_resource_runtime_for_page_async(
        &mut self,
        cookie_store: SharedBrowserCookieStore,
        loaded_page: Option<&mut Page>,
    ) -> Result<()> {
        let resource_runtime = self
            .rebuild_resource_request_client(cookie_store)?
            .browser_resource_runtime();
        if let Some(page) = loaded_page {
            let replacement = page
                .replace_browser_resource_runtime_async(&resource_runtime)
                .await;
            // The Page drops its old document authority at the completion
            // boundary. Reap after either terminal outcome; a still-live old
            // authority simply keeps its owner registered.
            self.browser_context_access.reap_retired_resource_runtimes();
            replacement?;
        }
        Ok(())
    }

    pub async fn rebuild_resource_runtime_for_page_with_storage_async(
        &mut self,
        storage: NavigationResourceStorageHandles,
        loaded_page: Option<&mut Page>,
    ) -> Result<()> {
        self.rebuild_resource_runtime_for_page_async(storage.into_cookie_store(), loaded_page)
            .await
    }

    pub fn set_tls_verify_host(&mut self, enabled: bool) {
        self.fetch_config.set_tls_verify_host(enabled);
    }

    pub fn tls_verify_host(&self) -> bool {
        self.fetch_config.tls_verify_host()
    }

    pub fn set_user_agent_override(&mut self, user_agent: impl Into<String>) {
        self.fetch_config.set_user_agent(user_agent);
    }

    pub fn set_browser_identity_override(
        &mut self,
        browser_identity: moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.fetch_config.set_browser_identity(browser_identity);
    }

    pub fn set_http_proxy_override(&mut self, proxy: Option<String>) {
        self.fetch_config.set_http_proxy(proxy);
    }

    pub fn set_http_no_proxy_override(&mut self, no_proxy: Option<String>) {
        self.fetch_config.set_http_no_proxy(no_proxy);
    }
}

#[cfg(test)]
mod tests {
    use super::{CommittedDocumentResourceSource, NavigationEngine, NavigationRuntimeConfig};
    use crate::{
        LayoutPolicy, OptionalResourceFetchMask,
        network::ResourceRequestClient,
        runtime::{Browser, BrowserConfig, RendererBrowserContextRuntime},
    };
    use moli_cookie_jar::new_shared_browser_cookie_store;
    use moli_fetch::FetchConfig;
    use url::Url;

    static_assertions::assert_not_impl_any!(Browser: Send, Sync);
    static_assertions::assert_not_impl_any!(NavigationEngine: Send, Sync);
    static_assertions::assert_not_impl_any!(
        moli_renderer_v8::RendererBrowserContextRuntimeOwnerAccess: Send,
        Sync
    );

    #[test]
    fn navigation_engine_can_share_browser_context_runtime() {
        let browser_context_owner = RendererBrowserContextRuntime::new();
        let browser_context_runtime = browser_context_owner.handle();
        let engine = NavigationEngine::new_with_fetch_config_and_browser_context_access(
            FetchConfig::default(),
            browser_context_owner.owner_access(),
            OptionalResourceFetchMask::NONE,
            true,
        )
        .expect("browser context owner should remain live");

        assert!(
            engine
                .browser_context_runtime()
                .shares_state_with(&browser_context_runtime)
        );
    }

    #[test]
    fn navigation_engine_default_runtime_isolated() {
        let left = NavigationEngine::new();
        let right = NavigationEngine::new();

        assert!(
            !left
                .browser_context_runtime()
                .shares_state_with(&right.browser_context_runtime())
        );
    }

    #[test]
    fn navigation_engine_preserves_the_complete_optional_resource_mask() {
        let mask = OptionalResourceFetchMask::FONT
            | OptionalResourceFetchMask::VIDEO
            | OptionalResourceFetchMask::TEXT_TRACK;
        let engine = NavigationEngine::new_with_fetch_config_and_resource_loading(
            FetchConfig::default(),
            mask,
            true,
        );

        assert_eq!(engine.optional_resource_fetch_mask(), mask);
        assert!(!engine.image_fetch_enabled());
    }

    #[test]
    fn navigation_runtime_config_preserves_mock_layout_policy_at_renderer_owner() {
        let config = NavigationRuntimeConfig::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::FONT,
            false,
            LayoutPolicy::Mock,
        );
        let engine = NavigationEngine::new_with_runtime_config(config);

        assert_eq!(engine.layout_policy(), LayoutPolicy::Mock);
        assert_eq!(engine.runtime_config().layout_policy(), LayoutPolicy::Mock);
        assert_eq!(
            engine.js_runtime.renderer_owner_handle().layout_policy(),
            LayoutPolicy::Mock
        );
        assert_eq!(
            engine.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::FONT
        );
        assert!(!engine.subframe_loading_enabled());
    }

    #[test]
    fn shared_renderer_owner_rejects_a_conflicting_layout_policy() {
        let source = NavigationEngine::new_with_runtime_config(NavigationRuntimeConfig::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::Mock,
        ));

        let error = NavigationEngine::new_with_runtime_config_and_shared_renderer_owner(
            NavigationRuntimeConfig::new(
                FetchConfig::default(),
                OptionalResourceFetchMask::NONE,
                true,
                LayoutPolicy::OnDemand,
            ),
            &source,
        )
        .expect_err("a renderer owner must not mix browser layout policies");

        assert!(error.to_string().contains("layout policy is configured"));
        assert_eq!(source.layout_policy(), LayoutPolicy::Mock);
    }

    #[test]
    fn browser_configures_layout_policy_before_any_page_creation() {
        let browser = Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Mock))
            .expect("browser should initialize with mock layout");

        assert_eq!(
            browser.js_runtime.renderer_owner_handle().layout_policy(),
            LayoutPolicy::Mock
        );
    }

    #[test]
    fn adopting_shared_backend_does_not_share_page_network_policy() {
        let left = NavigationEngine::new();
        let right = NavigationEngine::new_with_fetch_config_and_shared_renderer_owner(
            FetchConfig::default(),
            &left,
            OptionalResourceFetchMask::NONE,
            true,
        )
        .expect("standalone shared owner should remain live");
        let left_loader = left.resource_request_client().expect("left request client");
        let right_loader = right
            .resource_request_client()
            .expect("right request client");

        assert!(left_loader.shares_resource_runtime_with(&right_loader));
        assert!(!left_loader.shares_page_network_policy_with(&right_loader));

        left_loader.set_network_offline(true);
        assert!(
            !right_loader
                .page_network_policy()
                .snapshot()
                .network_offline()
        );
    }

    #[test]
    fn navigation_engine_clone_keeps_the_standalone_owner_live_after_source_drop() {
        let engine = NavigationEngine::new();
        let expected_runtime_id = engine
            .resource_request_client()
            .expect("standalone engine should start with a resource runtime")
            .resource_runtime_diagnostics()
            .runtime_id;
        let clone = engine.clone();

        drop(engine);

        assert_eq!(
            clone
                .resource_request_client()
                .expect("clone must retain the shared standalone lifetime owner")
                .resource_runtime_diagnostics()
                .runtime_id,
            expected_runtime_id
        );
    }

    #[test]
    fn stale_navigation_engine_cache_converges_on_context_current_resource_runtime() {
        let mut replacing_engine = NavigationEngine::new();
        let mut stale_engine = replacing_engine.clone();
        let old_runtime = stale_engine
            .resource_request_client()
            .expect("stale wrapper should cache the initial runtime");
        let replacement_cookie_store = new_shared_browser_cookie_store();

        let replacement = replacing_engine
            .rebuild_resource_request_client(replacement_cookie_store.clone())
            .expect("peer wrapper should replace the context runtime");
        assert!(
            !old_runtime.shares_resource_runtime_with(&replacement),
            "the peer rebuild must install a new runtime"
        );

        let converged = stale_engine
            .ensure_resource_request_client(replacement_cookie_store)
            .expect("stale wrapper should consult the context binding");
        assert!(
            converged.shares_resource_runtime_with(&replacement),
            "an ambient engine cache must not revive the retired runtime"
        );
    }

    #[test]
    fn explicit_config_rebuild_replaces_runtime_with_unchanged_cookie_store() {
        let mut engine = NavigationEngine::new();
        let old_client = engine
            .resource_request_client()
            .expect("engine should start with a runtime");
        let cookie_store = old_client.browser_resource_runtime().cookie_store();
        engine.set_user_agent_override("Moli/Rebuilt-Config");

        let replacement = engine
            .rebuild_resource_request_client(cookie_store)
            .expect("config rebuild should replace the runtime");

        assert!(!old_client.shares_resource_runtime_with(&replacement));
        assert_eq!(
            replacement.browser_identity().user_agent(),
            "Moli/Rebuilt-Config"
        );
    }

    #[test]
    fn engine_with_different_fetch_config_replaces_context_current_runtime() {
        let base_engine = NavigationEngine::new();
        let mut override_engine = base_engine.clone();
        let mut default_engine = base_engine.clone();
        let cookie_store = base_engine
            .resource_request_client()
            .expect("base engine should start with a runtime")
            .browser_resource_runtime()
            .cookie_store();
        override_engine.set_user_agent_override("Moli/Target-Override");
        let override_client = override_engine
            .rebuild_resource_request_client(cookie_store.clone())
            .expect("override target should install its config");

        let restored = default_engine
            .ensure_resource_request_client(cookie_store)
            .expect("default target should reject another target's current backend");

        assert!(!restored.shares_resource_runtime_with(&override_client));
        assert_eq!(
            restored.browser_identity().user_agent(),
            FetchConfig::DEFAULT_USER_AGENT
        );
    }

    #[test]
    fn synthetic_document_source_uses_the_engine_runtime_and_page_policy() {
        let mut engine = NavigationEngine::new();
        let cookie_store = new_shared_browser_cookie_store();
        let final_url = Url::parse("about:blank").expect("synthetic Document URL");

        let client = engine
            .resource_request_client_for_committed_document(
                cookie_store,
                &final_url,
                CommittedDocumentResourceSource::Synthetic,
            )
            .expect("synthetic Document client");
        let engine_client = engine
            .resource_request_client()
            .expect("engine resource request client");

        assert!(client.shares_resource_runtime_with(&engine_client));
        assert!(client.shares_page_network_policy_with(&engine_client));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn navigation_document_source_rejects_a_foreign_browser_runtime() {
        let cookie_store = new_shared_browser_cookie_store();
        let start_url = Url::parse("https://example.test/start").expect("navigation start URL");
        let final_url = Url::parse("https://example.test/final").expect("navigation final URL");
        let foreign_client_owner = ResourceRequestClient::new_with_cookie_store(
            &FetchConfig::default(),
            cookie_store.clone(),
        )
        .expect("foreign request client");
        let foreign_client = foreign_client_owner.handle();
        let navigation = moli_renderer_v8::network::navigation::NavigationResourceLoader::new(
            foreign_client,
            start_url,
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("test must own a Tokio runtime"),
        );
        navigation
            .note_service_worker_response_ready()
            .expect("navigation response ready");
        let seed = navigation
            .commit(final_url.clone())
            .expect("navigation seed");
        let mut engine = NavigationEngine::new();

        let error = engine
            .resource_request_client_for_committed_document(
                cookie_store,
                &final_url,
                CommittedDocumentResourceSource::Navigation(Box::new(seed)),
            )
            .expect_err("foreign browser runtime must not commit");

        assert!(
            error
                .to_string()
                .contains("does not belong to the NavigationEngine browser resource runtime")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn navigation_commit_keeps_its_registered_runtime_after_peer_target_replacement() {
        let mut target_a = NavigationEngine::new();
        let mut target_b = target_a.clone();
        let cookie_store = target_a
            .resource_request_client()
            .expect("target A initial request client")
            .browser_resource_runtime()
            .cookie_store();
        target_a.set_user_agent_override("Moli/Target-A");
        let target_a_client = target_a
            .rebuild_resource_request_client(cookie_store.clone())
            .expect("target A runtime");
        let start_url = Url::parse("https://example.test/start").expect("navigation start URL");
        let final_url = Url::parse("https://example.test/final").expect("navigation final URL");
        let navigation = moli_renderer_v8::network::navigation::NavigationResourceLoader::new(
            target_a_client.clone(),
            start_url,
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("test must own a Tokio runtime"),
        );
        navigation
            .note_service_worker_response_ready()
            .expect("target A navigation response ready");
        let seed = navigation
            .commit(final_url.clone())
            .expect("target A navigation seed");

        target_b.set_user_agent_override("Moli/Target-B");
        let target_b_client = target_b
            .rebuild_resource_request_client(cookie_store.clone())
            .expect("target B must replace the ambient BrowserContext runtime");
        assert!(!target_a_client.shares_resource_runtime_with(&target_b_client));

        let committed_client = target_a
            .resource_request_client_for_committed_document(
                cookie_store,
                &final_url,
                CommittedDocumentResourceSource::Navigation(Box::new(seed)),
            )
            .expect("target A must commit with the runtime that fetched its response");

        assert!(committed_client.shares_resource_runtime_with(&target_a_client));
        assert_eq!(
            committed_client.browser_identity().user_agent(),
            "Moli/Target-A"
        );
    }
}

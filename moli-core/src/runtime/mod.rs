mod fetch_deadline;
mod lifecycle_fetch;
mod navigation_engine;
pub mod storage_partition;

pub use crate::config::BrowserConfig;

use crate::{
    network::{DocumentFetchContextSeed, NavigationResourceLoader, ResourceRequestClient},
    page::{DocumentStartScript, Page},
    renderer::{JsRuntime, RendererOwnerCommand, materialize_page_created_reply},
    selector::QueryEngine,
};
use anyhow::Result;
use anyhow::{Context, anyhow, bail};
use moli_cookie_jar::StoredCookie;
use moli_fetch::{
    FetchCancelHandle, RawResponse, Request, StreamingRawResponse, ensure_http_status_success,
};
use moli_page_types::NavigationResponse;
use moli_renderer_v8::network::{
    BrowserResourceRuntime, BrowserResourceRuntimeOwner, PageNetworkPolicy,
};
use moli_url::is_about_blank as is_about_blank_url;
use moli_web_mime::response_headers_indicate_raw_document;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;
use std::{rc::Rc, sync::Arc};
use storage_partition::StoragePartitionState;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use url::Url;

pub use crate::renderer::ExternalRawDocumentBodyStream;
pub use crate::renderer::PageVmInitStage;
pub use crate::renderer::RendererReplyBoundary;
pub use fetch_deadline::FetchDeadline;
pub use moli_renderer_v8::{
    DetachedParserScriptFetchContinuation, RendererBrowserContextRuntime,
    RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    RendererLifecycleDecider, RendererLifecycleDecision, RendererLifecycleSnapshot,
    RendererPageReservationToken, RendererReservedServiceWorkerClient,
    RendererServiceWorkerMainResourceFetch, RendererSharedWorkerRuntimeDiagnostics,
};
pub use navigation_engine::{
    BuiltDocumentPage, CommittedDocumentResourceSource, NavigationEngine,
    NavigationPageStorageHandles, NavigationResourceStorageHandles, NavigationRuntimeConfig,
    PendingBuiltDocumentPage, PreparedDocumentPage, PreparedDocumentPageCommitConfiguration,
    PreparedDocumentPageCommitPermit,
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn is_fetch_readiness_timeout(error: &anyhow::Error, wait_until: RenderedDomWaitUntil) -> bool {
    let expected = match wait_until {
        RenderedDomWaitUntil::NetworkIdle => "timed out waiting for networkidle",
        RenderedDomWaitUntil::DomStable => "timed out waiting for domstable",
        RenderedDomWaitUntil::DomContentLoaded
        | RenderedDomWaitUntil::Load
        | RenderedDomWaitUntil::Done => return false,
    };

    error.chain().any(|cause| cause.to_string() == expected)
}

#[derive(Debug, Clone, Default)]
struct AutomationController;

#[derive(Debug, Clone, Default)]
struct WebApiRegistry;

#[derive(Clone)]
pub struct Browser {
    config: BrowserConfig,
    js_runtime: JsRuntime,
    resource_runtime: BrowserResourceRuntime,
    page_network_policy: PageNetworkPolicy,
    partition: Arc<StoragePartitionState>,
    _web_api_registry: WebApiRegistry,
    _selector_engine: QueryEngine,
    _automation: AutomationController,
    // Declared last so each Browser clone releases its request-side handles
    // before the final shared, thread-affine lifetime owner tears down.
    _lifetime_owner: Rc<BrowserLifetimeOwner>,
}

struct BrowserLifetimeOwner {
    js_runtime: Option<JsRuntime>,
    browser_context_owner: moli_renderer_v8::RendererBrowserContextRuntimeOwner,
    partition: Arc<StoragePartitionState>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("config", &self.config)
            .field(
                "partition_indexed_db_manager_strong_count",
                &std::sync::Arc::strong_count(self.partition.indexed_db_manager()),
            )
            .field("js_runtime", &self.js_runtime)
            .field("resource_runtime", &self.resource_runtime)
            .field("partition_persistence", &self.partition.persistence())
            .field("partition_id", &self.partition.id())
            .field("_web_api_registry", &self._web_api_registry)
            .field("_selector_engine", &self._selector_engine)
            .field("_automation", &self._automation)
            .finish()
    }
}

impl Drop for BrowserLifetimeOwner {
    fn drop(&mut self) {
        debug!("terminating browser renderer producers");
        if let Some(js_runtime) = self.js_runtime.take() {
            js_runtime.terminate_resource_producers_for_owner_shutdown();
            // Do not leave RenderRuntimeOwner to implicit field drop after
            // network join/storage flush. Releasing it here establishes the
            // renderer -> network -> persistent-storage teardown order.
            drop(js_runtime);
        }
        self.browser_context_owner.shutdown_and_join();
        if let Err(error) = self.partition.flush() {
            warn!(
                error = %error,
                "failed to flush browser storage partition"
            );
        }
        debug!("browser renderer and resource owners joined");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedDomWaitUntil {
    DomContentLoaded,
    Load,
    NetworkIdle,
    DomStable,
    Done,
}

impl RenderedDomWaitUntil {
    /// The concrete lifecycle boundary at which a live `Page` can first be
    /// handed back to the host. Network-idle and DOM-stable are observations
    /// made on that live Page, after Load and DCL respectively.
    fn base_stage(self) -> PageVmInitStage {
        match self {
            Self::DomContentLoaded | Self::DomStable => PageVmInitStage::DomContentLoaded,
            Self::Load | Self::NetworkIdle | Self::Done => PageVmInitStage::Load,
        }
    }

    fn has_best_effort_page_wait(self) -> bool {
        matches!(self, Self::NetworkIdle | Self::DomStable)
    }
}

#[derive(Debug)]
pub struct RawDocument {
    response: RawResponse,
}

impl RawDocument {
    fn from_response(response: RawResponse) -> Self {
        Self { response }
    }

    pub fn final_url(&self) -> &Url {
        &self.response.final_url
    }

    pub fn status(&self) -> u16 {
        self.response.status
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.response.headers
    }

    pub fn body_bytes(&self) -> &[u8] {
        self.response.body_bytes()
    }
}

#[derive(Debug)]
pub enum FetchedDocument {
    Page(Page),
    Raw(Box<RawDocument>),
}

impl Browser {
    fn resource_request_client(&self) -> ResourceRequestClient {
        ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
            self.resource_runtime.clone(),
            self.page_network_policy.clone(),
        )
    }

    pub fn new(mut config: BrowserConfig) -> Result<Self> {
        let partition = Arc::new(StoragePartitionState::open(config.profile_dir())?);
        if config.fetch().http_cache_dir().is_none()
            && let Some(http_cache_root) = partition.http_cache_root()
        {
            config
                .fetch_mut()
                .set_http_cache_dir(Some(http_cache_root.display().to_string()));
        }
        let resource_runtime =
            BrowserResourceRuntimeOwner::new(config.fetch(), partition.cookie_store());
        let page_network_policy = PageNetworkPolicy::new(
            config.optional_resource_fetch_mask(),
            config.subframe_loading_enabled(),
        );
        let browser_context_owner =
            RendererBrowserContextRuntime::new_owned_with_service_worker_resource_store_and_browser_resource_runtime(
                partition.service_worker_resource_store(),
                resource_runtime,
            );
        let browser_context_access = browser_context_owner.owner_access();
        let resource_runtime = browser_context_access
            .current_browser_resource_runtime()
            .map_err(|error| anyhow!("browser context resource owner unavailable: {error}"))?;
        let js_runtime =
            JsRuntime::initialize_with_browser_context_owner_access(&browser_context_access)?;
        js_runtime
            .renderer_owner_handle()
            .configure_layout_policy(config.layout_policy())?;
        let lifetime_owner = Rc::new(BrowserLifetimeOwner {
            js_runtime: Some(js_runtime.clone()),
            browser_context_owner,
            partition: Arc::clone(&partition),
        });
        Ok(Self {
            js_runtime,
            resource_runtime,
            page_network_policy,
            partition,
            _web_api_registry: WebApiRegistry,
            _selector_engine: QueryEngine,
            _automation: AutomationController,
            config,
            _lifetime_owner: lifetime_owner,
        })
    }

    pub fn session(&self) -> Session {
        Session {
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            browser: self.clone(),
        }
    }

    pub fn cookies(&self) -> Result<Vec<StoredCookie>> {
        self.partition.cookies()
    }

    pub fn clear_indexed_db_origin(&self, origin: &str) -> std::result::Result<(), String> {
        moli_renderer_v8::clear_indexed_db_origin(self.partition.indexed_db_manager(), origin)
    }

    pub fn import_cookies(&self, cookies: impl IntoIterator<Item = StoredCookie>) -> Result<usize> {
        self.partition.import_cookies(cookies)
    }

    pub async fn fetch(&self, raw_url: &str) -> Result<Page> {
        self.fetch_request(Request::get(raw_url)?).await
    }

    pub async fn fetch_with_wait_until(
        &self,
        raw_url: &str,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
    ) -> Result<Page> {
        self.fetch_request_with_wait_until(Request::get(raw_url)?, wait_until, timeout)
            .await
    }

    pub async fn fetch_request(&self, request: Request) -> Result<Page> {
        self.fetch_internal(request, PageVmInitStage::Load).await
    }

    pub async fn fetch_request_with_wait_until(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
    ) -> Result<Page> {
        self.fetch_page_with_wait_until_deadline(
            request,
            wait_until,
            FetchDeadline::new(timeout)?,
            false,
        )
        .await
    }

    pub async fn fetch_allow_http_error(&self, raw_url: &str) -> Result<Page> {
        self.fetch_request_allow_http_error(Request::get(raw_url)?)
            .await
    }

    pub async fn fetch_allow_http_error_with_wait_until(
        &self,
        raw_url: &str,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
    ) -> Result<Page> {
        self.fetch_request_allow_http_error_with_wait_until(
            Request::get(raw_url)?,
            wait_until,
            timeout,
        )
        .await
    }

    pub async fn fetch_request_allow_http_error(&self, request: Request) -> Result<Page> {
        self.fetch_allow_http_error_internal(request, PageVmInitStage::Load)
            .await
    }

    pub async fn fetch_request_document_allow_http_error(
        &self,
        request: Request,
    ) -> Result<FetchedDocument> {
        self.fetch_document_allow_http_error_internal(
            request,
            PageVmInitStage::Load,
            RendererReplyBoundary::Stage,
        )
        .await
    }

    pub async fn fetch_request_allow_http_error_with_wait_until(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
    ) -> Result<Page> {
        self.fetch_page_with_wait_until_deadline(
            request,
            wait_until,
            FetchDeadline::new(timeout)?,
            true,
        )
        .await
    }

    async fn fetch_page_with_wait_until_deadline(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        deadline: FetchDeadline,
        allow_http_error: bool,
    ) -> Result<Page> {
        let raw_url = request.url.as_str().to_owned();
        let stage = wait_until.base_stage();
        debug!(
            url = %raw_url,
            wait_until = ?wait_until,
            timeout_ms = deadline.timeout().as_millis(),
            stage = ?stage,
            "starting fetch_with_wait_until deadline"
        );

        // Materialization only reaches the concrete DCL/Load base stage. The
        // selected stability observation is a separate Page operation so it
        // can consume the remaining budget and soften only its own timeout.
        let mut page = match tokio::time::timeout_at(
            deadline.at(),
            self.fetch_allow_http_error_internal(request, stage),
        )
        .await
        {
            // Preserve transport and policy errors from the fetch itself. The
            // outer deadline should add an error only when it actually wins.
            Ok(result) => result?,
            Err(_) => {
                warn!(
                    url = %raw_url,
                    wait_until = ?wait_until,
                    timeout_ms = deadline.timeout().as_millis(),
                    stage = ?stage,
                    allow_http_error,
                    "fetch_with_wait_until timed out"
                );
                if allow_http_error {
                    bail!(
                        "fetch allow-http-error wait_until {wait_until:?} timed out after {} ms for `{raw_url}`",
                        deadline.timeout().as_millis()
                    );
                }
                bail!(
                    "fetch wait_until {wait_until:?} timed out after {} ms for `{raw_url}`",
                    deadline.timeout().as_millis()
                );
            }
        };

        self.wait_for_page_readiness_with_deadline(&mut page, wait_until, deadline)
            .await?;
        if !allow_http_error {
            ensure_http_status_success(page.final_url().as_str(), page.status(), false)?;
        }
        Ok(page)
    }

    pub async fn fetch_request_document_allow_http_error_with_wait_until(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
    ) -> Result<FetchedDocument> {
        let deadline = FetchDeadline::new(timeout)?;
        self.fetch_request_document_allow_http_error_with_wait_until_deadline(
            request, wait_until, deadline,
        )
        .await
    }

    /// Fetches a document to `wait_until` without starting a new timeout
    /// budget. Callers can reuse `deadline` for later readiness conditions.
    pub async fn fetch_request_document_allow_http_error_with_wait_until_deadline(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        deadline: FetchDeadline,
    ) -> Result<FetchedDocument> {
        let fetched = self
            .fetch_document_to_base_stage(
                request,
                wait_until,
                deadline,
                RendererReplyBoundary::Stage,
                None,
            )
            .await?;

        match fetched {
            FetchedDocument::Page(mut page) => {
                self.wait_for_page_readiness_with_deadline(&mut page, wait_until, deadline)
                    .await?;
                Ok(FetchedDocument::Page(page))
            }
            FetchedDocument::Raw(raw) => Ok(FetchedDocument::Raw(raw)),
        }
    }

    /// Reaches the concrete lifecycle boundary that makes a live Page
    /// available. For NetworkIdle and DomStable this is only the Load or DCL
    /// base stage; their best-effort observation runs outside materialization.
    async fn fetch_document_to_base_stage(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        deadline: FetchDeadline,
        reply_boundary: RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
    ) -> Result<FetchedDocument> {
        let timeout = deadline.timeout();
        let stage = wait_until.base_stage();

        let raw_url = request.url.as_str().to_owned();
        debug!(
            url = %raw_url,
            wait_until = ?wait_until,
            timeout_ms = timeout.as_millis(),
            stage = ?stage,
            "starting fetch_document_allow_http_error_with_wait_until deadline"
        );
        // This deadline is strict: if the response or base DCL/Load stage does
        // not arrive in time, there is no Page on which best-effort readiness
        // can operate. Only the later NetworkIdle/DomStable observation may
        // soften an expired deadline.
        let outer_deadline = deadline.at();
        let requested_url = request.url.clone();
        if is_about_blank_url(&requested_url) {
            return self
                .fetch_document_wait_timeout(
                    &raw_url,
                    wait_until,
                    timeout,
                    stage,
                    outer_deadline,
                    self.materialize_static_html_page(
                        &raw_url,
                        requested_url,
                        stage,
                        reply_boundary,
                        lifecycle_decider,
                        String::new(),
                    ),
                )
                .await
                .map(FetchedDocument::Page);
        }
        let navigation_loader = NavigationResourceLoader::new(
            self.resource_request_client(),
            requested_url.clone(),
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()?,
        );
        let service_worker_fetch = self
            .fetch_document_wait_timeout(
                &raw_url,
                wait_until,
                timeout,
                stage,
                outer_deadline,
                self.fetch_service_worker_main_resource_for_navigation(
                    &request,
                    &navigation_loader,
                ),
            )
            .await?;
        let RendererServiceWorkerMainResourceFetch {
            reserved_client,
            response,
        } = service_worker_fetch;
        if let Some(response) = response {
            navigation_loader.note_service_worker_response_ready()?;
            let document_fetch_context_seed =
                navigation_loader.commit(response.final_url.clone())?;
            if response_headers_indicate_raw_document(&response.headers) {
                let raw_response =
                    RawResponse::from_head_and_body(response.head(), response.clone_body_bytes());
                return Ok(FetchedDocument::Raw(Box::new(RawDocument::from_response(
                    raw_response,
                ))));
            }
            return self
                .fetch_document_wait_timeout(
                    &raw_url,
                    wait_until,
                    timeout,
                    stage,
                    outer_deadline,
                    self.materialize_streaming_raw_response_page(
                        &raw_url,
                        requested_url,
                        stage,
                        reply_boundary,
                        lifecycle_decider,
                        streaming_raw_response_from_navigation_response(response)?,
                        document_fetch_context_seed,
                        reserved_client,
                    ),
                )
                .await
                .map(FetchedDocument::Page);
        }
        let response = self
            .fetch_document_wait_timeout(
                &raw_url,
                wait_until,
                timeout,
                stage,
                outer_deadline,
                navigation_loader.fetch_raw_stream(request),
            )
            .await?;
        if response_headers_indicate_raw_document(&response.headers) {
            // `wait_until` is a DOM lifecycle deadline. Binary/download-like main
            // resources do not have DCL/load milestones, so only the initial
            // document response acquisition is gated above. The body dump is plain
            // network output and should not be reported as a DCL timeout.
            return self
                .materialize_streaming_raw_response_raw_document(&raw_url, response)
                .await
                .map(|raw| FetchedDocument::Raw(Box::new(raw)));
        }

        let document_fetch_context_seed = navigation_loader.commit(response.final_url.clone())?;
        self.fetch_document_wait_timeout(
            &raw_url,
            wait_until,
            timeout,
            stage,
            outer_deadline,
            self.materialize_streaming_raw_response_page(
                &raw_url,
                requested_url,
                stage,
                reply_boundary,
                lifecycle_decider,
                response,
                document_fetch_context_seed,
                reserved_client,
            ),
        )
        .await
        .map(FetchedDocument::Page)
    }

    pub async fn wait_for_page_delay(&self, page: &mut Page, delay: Duration) -> Result<()> {
        if delay.is_zero() {
            return Ok(());
        }

        let deadline = Instant::now()
            .checked_add(delay)
            .unwrap_or_else(Instant::now);
        loop {
            let ms_to_next_timeout = page.ms_to_next_timeout().await?;

            let now = Instant::now();
            if now >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(now);
            let sleep_for = ms_to_next_timeout
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_millis(50))
                .min(Duration::from_millis(50))
                .min(remaining);
            if sleep_for.is_zero() {
                continue;
            }
            tokio::time::sleep(sleep_for).await;
        }

        Ok(())
    }

    pub async fn wait_for_page_network_idle(
        &self,
        page: &mut Page,
        timeout: Duration,
    ) -> Result<()> {
        page.wait_for_network_idle(&self.resource_request_client(), timeout)
            .await
    }

    pub async fn wait_for_page_dom_stable(&self, page: &mut Page, timeout: Duration) -> Result<()> {
        page.wait_for_dom_stable(&self.resource_request_client(), timeout)
            .await
    }

    pub async fn wait_for_selector(
        &self,
        page: &mut Page,
        selector: &str,
        timeout: Duration,
    ) -> Result<crate::page::RendererDocumentQuerySelectorNode> {
        page.wait_for_selector(&self.resource_request_client(), selector, timeout)
            .await
    }

    /// Waits for a selector using the unspent portion of `deadline`.
    pub async fn wait_for_selector_with_deadline(
        &self,
        page: &mut Page,
        selector: &str,
        deadline: FetchDeadline,
    ) -> Result<crate::page::RendererDocumentQuerySelectorNode> {
        let loader = self.resource_request_client();
        let remaining = deadline.remaining();
        deadline
            .wait(
                "waiting for a selector",
                page.wait_for_selector(&loader, selector, remaining),
            )
            .await
    }

    pub async fn wait_for_script_truthy(
        &self,
        page: &mut Page,
        expression: &str,
        timeout: Duration,
    ) -> Result<()> {
        page.wait_for_script_truthy(&self.resource_request_client(), expression, timeout)
            .await
    }

    /// Waits for a truthy script result using the unspent portion of
    /// `deadline`.
    pub async fn wait_for_script_truthy_with_deadline(
        &self,
        page: &mut Page,
        expression: &str,
        deadline: FetchDeadline,
    ) -> Result<()> {
        let loader = self.resource_request_client();
        let remaining = deadline.remaining();
        deadline
            .wait(
                "waiting for a script to become truthy",
                page.wait_for_script_truthy(&loader, expression, remaining),
            )
            .await
    }

    pub async fn wait_for_subresource_response(
        &self,
        page: &mut Page,
        criteria: crate::page::SubresourceResponseWaitCriteria,
        timeout: Duration,
    ) -> Result<()> {
        page.wait_for_subresource_response(&self.resource_request_client(), criteria, timeout)
            .await
    }

    /// Waits for a matching response using the unspent portion of `deadline`.
    pub async fn wait_for_subresource_response_with_deadline(
        &self,
        page: &mut Page,
        criteria: crate::page::SubresourceResponseWaitCriteria,
        deadline: FetchDeadline,
    ) -> Result<()> {
        let loader = self.resource_request_client();
        let remaining = deadline.remaining();
        deadline
            .wait(
                "waiting for a subresource response",
                page.wait_for_subresource_response(&loader, criteria, remaining),
            )
            .await
    }

    /// Observes NetworkIdle or DomStable with the unspent part of `deadline`.
    ///
    /// A timeout in this observation is deliberately best-effort: the Page
    /// has already reached its required Load/DCL base stage, so the current
    /// snapshot is returned with a warning. Errors unrelated to timeout still
    /// fail the fetch. Concrete lifecycle modes are no-ops here.
    pub async fn wait_for_page_readiness_with_deadline(
        &self,
        page: &mut Page,
        wait_until: RenderedDomWaitUntil,
        deadline: FetchDeadline,
    ) -> Result<()> {
        if !wait_until.has_best_effort_page_wait() {
            return Ok(());
        }

        let loader = self.resource_request_client();
        let remaining = deadline.remaining();
        let result = match wait_until {
            RenderedDomWaitUntil::NetworkIdle => {
                tokio::time::timeout_at(
                    deadline.at(),
                    page.wait_for_network_idle(&loader, remaining),
                )
                .await
            }
            RenderedDomWaitUntil::DomStable => {
                tokio::time::timeout_at(deadline.at(), page.wait_for_dom_stable(&loader, remaining))
                    .await
            }
            RenderedDomWaitUntil::DomContentLoaded
            | RenderedDomWaitUntil::Load
            | RenderedDomWaitUntil::Done => unreachable!(
                "concrete lifecycle modes returned before starting a best-effort Page wait"
            ),
        };

        let timeout_error = match result {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) if is_fetch_readiness_timeout(&error, wait_until) => error,
            Ok(Err(error)) => {
                return Err(error).with_context(|| {
                    anyhow!("failed while waiting for page readiness {wait_until:?}")
                });
            }
            Err(error) => anyhow::Error::new(error),
        };

        warn!(
            page_id = page.page_id(),
            url = %page.requested_url(),
            final_url = %page.final_url(),
            wait_until = ?wait_until,
            timeout_ms = deadline.timeout().as_millis(),
            remaining_ms = remaining.as_millis(),
            error = %timeout_error,
            "fetch readiness wait timed out; returning best-effort page"
        );
        Ok(())
    }

    async fn fetch_internal(&self, request: Request, stage: PageVmInitStage) -> Result<Page> {
        let raw_url = request.url.as_str().to_owned();
        let requested_url = request.url.clone();
        if is_about_blank_url(&requested_url) {
            return self
                .materialize_static_html_page(
                    &raw_url,
                    requested_url,
                    stage,
                    RendererReplyBoundary::Stage,
                    None,
                    String::new(),
                )
                .await;
        }
        let navigation_loader = NavigationResourceLoader::new(
            self.resource_request_client(),
            requested_url.clone(),
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()?,
        );
        let service_worker_fetch = self
            .fetch_service_worker_main_resource_for_navigation(&request, &navigation_loader)
            .await?;
        let RendererServiceWorkerMainResourceFetch {
            reserved_client,
            response,
        } = service_worker_fetch;
        if let Some(response) = response {
            navigation_loader.note_service_worker_response_ready()?;
            let document_fetch_context_seed =
                navigation_loader.commit(response.final_url.clone())?;
            let response = streaming_raw_response_from_navigation_response(response)?;
            let page = self
                .materialize_streaming_raw_response_page(
                    &raw_url,
                    requested_url,
                    stage,
                    RendererReplyBoundary::Stage,
                    None,
                    response,
                    document_fetch_context_seed,
                    reserved_client,
                )
                .await?;
            ensure_http_status_success(page.final_url().as_str(), page.status(), false)?;
            return Ok(page);
        }
        let response = navigation_loader.fetch_raw_stream(request).await?;
        let document_fetch_context_seed = navigation_loader.commit(response.final_url.clone())?;
        let page = self
            .materialize_streaming_raw_response_page(
                &raw_url,
                requested_url,
                stage,
                RendererReplyBoundary::Stage,
                None,
                response,
                document_fetch_context_seed,
                reserved_client,
            )
            .await?;
        ensure_http_status_success(page.final_url().as_str(), page.status(), false)?;
        Ok(page)
    }

    async fn fetch_allow_http_error_internal(
        &self,
        request: Request,
        stage: PageVmInitStage,
    ) -> Result<Page> {
        let raw_url = request.url.as_str().to_owned();
        let requested_url = request.url.clone();
        if is_about_blank_url(&requested_url) {
            return self
                .materialize_static_html_page(
                    &raw_url,
                    requested_url,
                    stage,
                    RendererReplyBoundary::Stage,
                    None,
                    String::new(),
                )
                .await;
        }
        let navigation_loader = NavigationResourceLoader::new(
            self.resource_request_client(),
            requested_url.clone(),
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()?,
        );
        let service_worker_fetch = self
            .fetch_service_worker_main_resource_for_navigation(&request, &navigation_loader)
            .await?;
        let RendererServiceWorkerMainResourceFetch {
            reserved_client,
            response,
        } = service_worker_fetch;
        if let Some(response) = response {
            navigation_loader.note_service_worker_response_ready()?;
            let document_fetch_context_seed =
                navigation_loader.commit(response.final_url.clone())?;
            return self
                .materialize_streaming_raw_response_page(
                    &raw_url,
                    requested_url,
                    stage,
                    RendererReplyBoundary::Stage,
                    None,
                    streaming_raw_response_from_navigation_response(response)?,
                    document_fetch_context_seed,
                    reserved_client,
                )
                .await;
        }
        let response = navigation_loader.fetch_raw_stream(request).await?;
        let document_fetch_context_seed = navigation_loader.commit(response.final_url.clone())?;
        self.materialize_streaming_raw_response_page(
            &raw_url,
            requested_url,
            stage,
            RendererReplyBoundary::Stage,
            None,
            response,
            document_fetch_context_seed,
            reserved_client,
        )
        .await
    }

    async fn fetch_document_allow_http_error_internal(
        &self,
        request: Request,
        stage: PageVmInitStage,
        reply_boundary: RendererReplyBoundary,
    ) -> Result<FetchedDocument> {
        let raw_url = request.url.as_str().to_owned();
        let requested_url = request.url.clone();
        if is_about_blank_url(&requested_url) {
            return self
                .materialize_static_html_page(
                    &raw_url,
                    requested_url,
                    stage,
                    reply_boundary,
                    None,
                    String::new(),
                )
                .await
                .map(FetchedDocument::Page);
        }
        let navigation_loader = NavigationResourceLoader::new(
            self.resource_request_client(),
            requested_url.clone(),
            moli_renderer_v8::network::RendererResourceTaskRunner::from_current_tokio()?,
        );
        let service_worker_fetch = self
            .fetch_service_worker_main_resource_for_navigation(&request, &navigation_loader)
            .await?;
        let RendererServiceWorkerMainResourceFetch {
            reserved_client,
            response,
        } = service_worker_fetch;
        if let Some(response) = response {
            navigation_loader.note_service_worker_response_ready()?;
            let document_fetch_context_seed =
                navigation_loader.commit(response.final_url.clone())?;
            if response_headers_indicate_raw_document(&response.headers) {
                let raw_response =
                    RawResponse::from_head_and_body(response.head(), response.clone_body_bytes());
                return Ok(FetchedDocument::Raw(Box::new(RawDocument::from_response(
                    raw_response,
                ))));
            }
            return self
                .materialize_streaming_raw_response_page(
                    &raw_url,
                    requested_url,
                    stage,
                    reply_boundary,
                    None,
                    streaming_raw_response_from_navigation_response(response)?,
                    document_fetch_context_seed,
                    reserved_client,
                )
                .await
                .map(FetchedDocument::Page);
        }
        let response = navigation_loader.fetch_raw_stream(request).await?;
        let document_fetch_context_seed = navigation_loader.commit(response.final_url.clone())?;
        if response_headers_indicate_raw_document(&response.headers) {
            return self
                .materialize_streaming_raw_response_raw_document(&raw_url, response)
                .await
                .map(|raw| FetchedDocument::Raw(Box::new(raw)));
        }
        self.materialize_streaming_raw_response_page(
            &raw_url,
            requested_url,
            stage,
            reply_boundary,
            None,
            response,
            document_fetch_context_seed,
            reserved_client,
        )
        .await
        .map(FetchedDocument::Page)
    }

    async fn fetch_service_worker_main_resource_for_navigation(
        &self,
        request: &Request,
        navigation_loader: &NavigationResourceLoader,
    ) -> Result<RendererServiceWorkerMainResourceFetch> {
        self.js_runtime
            .browser_context_runtime()
            .fetch_service_worker_main_resource_for_navigation(request, navigation_loader)
            .await
    }

    async fn fetch_document_wait_timeout<T, F>(
        &self,
        raw_url: &str,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
        stage: PageVmInitStage,
        deadline: tokio::time::Instant,
        future: F,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match tokio::time::timeout_at(deadline, future).await {
            Ok(result) => result,
            Err(_) => {
                Err(self.fetch_document_wait_timeout_error(raw_url, wait_until, timeout, stage))
            }
        }
    }

    fn fetch_document_wait_timeout_error(
        &self,
        raw_url: &str,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
        stage: PageVmInitStage,
    ) -> anyhow::Error {
        warn!(
            url = %raw_url,
            wait_until = ?wait_until,
            timeout_ms = timeout.as_millis(),
            stage = ?stage,
            "fetch_document_allow_http_error_with_wait_until timed out"
        );
        anyhow!(
            "fetch document allow-http-error wait_until {wait_until:?} timed out after {} ms for `{raw_url}`",
            timeout.as_millis()
        )
    }

    async fn materialize_static_html_page(
        &self,
        raw_url: &str,
        requested_url: Url,
        stage: PageVmInitStage,
        reply_boundary: RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        response_body: String,
    ) -> Result<Page> {
        let started = Instant::now();
        debug!(
            url = %raw_url,
            stage = ?stage,
            "starting static html fetch_internal"
        );

        let document_start_scripts = self
            .config
            .document_start_scripts()
            .iter()
            .cloned()
            .map(|source| DocumentStartScript {
                registry_key: None,
                source,
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            })
            .collect::<Vec<_>>();
        let renderer_owner = self.js_runtime.renderer_owner_handle();
        let mut create_page_request = renderer_owner.build_create_html_page_request(
            requested_url.clone(),
            None,
            false,
            0,
            200,
            Vec::new(),
            &self.resource_request_client(),
            moli_renderer_v8::RendererWebStorageHandles::new(
                self.partition.web_storage_store(),
                self.partition.session_storage_store(),
            ),
            requested_url,
            response_body,
            document_start_scripts,
            vec![],
            vec![],
            vec![],
            false,
            Vec::new(),
            false,
            None,
            stage,
        );
        create_page_request.wpt_extensions_enabled = self.config.wpt_extensions_enabled();
        create_page_request.reply_boundary = reply_boundary;
        create_page_request.lifecycle_decider = lifecycle_decider;
        create_page_request.indexed_db_manager = Some(self.partition.weak_indexed_db_manager());
        create_page_request.storage_bucket_store = Some(self.partition.storage_bucket_store());
        let reply = renderer_owner
            .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
            .await
            .with_context(|| anyhow!("failed to execute scripts for page `{raw_url}`"))?;
        let page = materialize_page_created_reply(&renderer_owner, reply)?;
        info!(
            page_id = page.page_id(),
            url = %page.requested_url(),
            elapsed_ms = started.elapsed().as_millis(),
            "static html fetch_internal completed"
        );
        Ok(page)
    }

    async fn materialize_streaming_raw_response_page(
        &self,
        raw_url: &str,
        requested_url: Url,
        stage: PageVmInitStage,
        reply_boundary: RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        response: StreamingRawResponse,
        document_fetch_context_seed: DocumentFetchContextSeed,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
    ) -> Result<Page> {
        let started = Instant::now();
        debug!(
            url = %raw_url,
            stage = ?stage,
            "starting raw streaming fetch_internal"
        );

        let final_url = response.final_url.clone();
        if document_fetch_context_seed.final_url() != &final_url {
            return Err(anyhow!(
                "navigation commit seed final URL `{}` does not match response `{final_url}`",
                document_fetch_context_seed.final_url()
            ));
        }
        let page_loader =
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                document_fetch_context_seed.browser_resource_runtime(),
                document_fetch_context_seed.page_network_policy(),
            );
        let response_status = response.status;
        let response_headers = response.headers.clone();
        let redirected = response.redirected;
        let redirect_count = response.redirect_chain.len();
        let raw_body = external_raw_document_body_from_streaming_response(response);
        let document_start_scripts = self
            .config
            .document_start_scripts()
            .iter()
            .cloned()
            .map(|source| DocumentStartScript {
                registry_key: None,
                source,
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            })
            .collect::<Vec<_>>();

        let (
            handle,
            page_state,
            _page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        ) = self
            .js_runtime
            .create_streaming_raw_page_from_external_body_with_inspector_session_restores(
                requested_url,
                final_url,
                None,
                redirected,
                redirect_count,
                response_status,
                response_headers,
                &page_loader,
                moli_renderer_v8::RendererWebStorageHandles::new(
                    self.partition.web_storage_store(),
                    self.partition.session_storage_store(),
                ),
                raw_body,
                Some(self.partition.weak_indexed_db_manager()),
                Some(self.partition.storage_bucket_store()),
                document_start_scripts,
                vec![],
                vec![],
                None,
                None,
                false,
                false,
                1.0,
                Default::default(),
                None,
                false,
                Vec::new(),
                false,
                None,
                Vec::new(),
                self.config.wpt_extensions_enabled(),
                stage,
                reply_boundary,
                moli_renderer_v8::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
                moli_renderer_v8::RendererNavigationReplyPolicy::FollowBeforeReply,
                None,
                reserved_service_worker_client,
                None,
                lifecycle_decider,
            )
            .await
            .with_context(|| anyhow!("failed to execute scripts for page `{raw_url}`"))?;
        if pending_download.is_some() {
            return Err(anyhow!(
                "raw streaming page creation produced a pending download for `{raw_url}`"
            ));
        }
        let page = Page::from_attached_handle_with_creation_artifacts(
            handle,
            page_state,
            page_creation_artifacts,
        );
        info!(
            page_id = page.page_id(),
            url = %page.requested_url(),
            elapsed_ms = started.elapsed().as_millis(),
            "raw streaming fetch_internal completed"
        );
        Ok(page)
    }

    async fn materialize_streaming_raw_response_raw_document(
        &self,
        raw_url: &str,
        response: StreamingRawResponse,
    ) -> Result<RawDocument> {
        let started = Instant::now();
        let final_url = response.final_url.clone();
        let status = response.status;
        let raw_response = response
            .into_materialized_raw_response()
            .await
            .with_context(|| anyhow!("failed to read raw document body for `{raw_url}`"))?;
        info!(
            url = %final_url,
            status,
            body_bytes = raw_response.body_bytes().len(),
            elapsed_ms = started.elapsed().as_millis(),
            "raw document fetch_internal completed"
        );
        Ok(RawDocument::from_response(raw_response))
    }

    pub fn config(&self) -> &BrowserConfig {
        &self.config
    }
}

fn external_raw_document_body_from_streaming_response(
    response: StreamingRawResponse,
) -> ExternalRawDocumentBodyStream {
    external_raw_document_body_from_streaming_response_with_body_eof_observer(response, None)
}

fn external_raw_document_body_from_streaming_response_with_body_eof_observer(
    mut response: StreamingRawResponse,
    mut body_eof_observer: Option<oneshot::Sender<()>>,
) -> ExternalRawDocumentBodyStream {
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, body_stream) = ExternalRawDocumentBodyStream::channel(completion_rx);
    tokio::spawn(async move {
        let result = async {
            loop {
                let chunk = tokio::select! {
                    // The renderer may discard the prepared/raw Document
                    // while the network body is stalled. Observe that
                    // receiver closure concurrently with upstream input so
                    // dropping the bridge deterministically drops (and
                    // cancels) the real StreamingRawResponse.
                    _ = body_tx.closed() => return Ok(()),
                    chunk = response.next_chunk() => chunk,
                };
                let Some(chunk) = chunk else {
                    if let Some(observer) = body_eof_observer.take() {
                        let _ = observer.send(());
                    }
                    break;
                };
                if body_tx.send(chunk).await.is_err() {
                    return Ok(());
                }
            }
            tokio::select! {
                // Upstream body EOF does not imply that terminal completion
                // has arrived. Dropping the external body in that interval
                // must still cancel/drop the response and release its exact
                // resource-runtime lease.
                _ = body_tx.closed() => Ok(()),
                completion = response.finish() => completion,
            }
        }
        .await;
        let _ = completion_tx.send(result);
    });
    body_stream
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
    let (completion_tx, completion_rx) = oneshot::channel();
    let _ = completion_tx.send(Ok(()));
    Ok(StreamingRawResponse::new_with_head(
        head,
        body_rx,
        FetchCancelHandle::new(),
        completion_rx,
    ))
}

#[derive(Debug, Clone)]
pub struct Session {
    id: u64,
    browser: Browser,
}

impl Session {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub async fn open(&self, raw_url: &str) -> Result<Page> {
        self.browser.fetch(raw_url).await
    }
}

#[cfg(test)]
mod tests;

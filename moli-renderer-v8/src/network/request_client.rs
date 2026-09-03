use std::{
    fmt,
    sync::{Arc, mpsc},
    thread,
    time::Instant,
};

use anyhow::{Context, Result, anyhow};
use moli_cookie_jar::{
    BrowserCookieFacadeContext, SharedBrowserCookieStore, new_shared_browser_cookie_store,
};
use moli_fetch::{
    FetchCancelHandle, FetchConfig, NetworkFetchResult, RawResponse, Request, Response,
    ResponseHead, StreamingRawResponse,
};

use crate::{protocol_types::OptionalResourceFetchMask, types::SubresourceResourceType};

use super::{
    backend::{
        BrowserResourceRuntime, BrowserResourceRuntimeDiagnostics, BrowserResourceRuntimeOwner,
        BrowserResourceRuntimeOwnerRoot, RawSubresourceCacheKey, ScriptTextCacheLookup,
        SharedMemoryResourceCacheDiagnostics, raw_subresource_memory_cache_expiry,
        raw_subresource_memory_cache_key, script_text_cache_key,
        script_text_request_is_memory_cacheable,
    },
    loads,
    policy::PageNetworkPolicy,
};

#[derive(Clone)]
pub struct ResourceRequestClient {
    resource_runtime: BrowserResourceRuntime,
    page_network_policy: PageNetworkPolicy,
    browser_site_context: Option<Arc<BrowserCookieFacadeContext>>,
}

/// Thread-affine lifetime root for a standalone resource request client.
///
/// Renderer work obtains a clone of the dereferenced [`ResourceRequestClient`]
/// handle; the owner itself is neither `Clone` nor `Send`.
pub struct ResourceRequestClientOwner {
    client: ResourceRequestClient,
    _resource_runtime_owner: BrowserResourceRuntimeOwnerRoot,
}

impl ResourceRequestClient {
    // Standalone clients must return their thread-affine lifetime owner; the
    // owner dereferences to this request-side handle for existing call sites.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(config: &FetchConfig) -> Result<ResourceRequestClientOwner> {
        Self::new_with_cookie_store(config, new_shared_browser_cookie_store())
    }

    pub fn memory_cache_diagnostics(&self) -> SharedMemoryResourceCacheDiagnostics {
        self.resource_runtime.memory_cache_diagnostics()
    }

    pub fn resource_runtime_diagnostics(&self) -> BrowserResourceRuntimeDiagnostics {
        self.resource_runtime.diagnostics()
    }

    pub fn browser_resource_runtime(&self) -> BrowserResourceRuntime {
        self.resource_runtime.clone()
    }

    pub(crate) fn replace_browser_resource_runtime(
        &mut self,
        resource_runtime: BrowserResourceRuntime,
    ) {
        self.resource_runtime = resource_runtime;
    }

    pub fn shares_resource_runtime_with(&self, other: &Self) -> bool {
        self.resource_runtime
            .shares_state_with(&other.resource_runtime)
    }

    pub fn new_with_cookie_store(
        config: &FetchConfig,
        cookie_store: SharedBrowserCookieStore,
    ) -> Result<ResourceRequestClientOwner> {
        let registration = BrowserResourceRuntimeOwner::new(config, cookie_store);
        let (resource_runtime_owner, binding) = BrowserResourceRuntimeOwnerRoot::new(registration);
        let client = Self::from_browser_resource_runtime(binding.current());
        Ok(ResourceRequestClientOwner {
            client,
            _resource_runtime_owner: resource_runtime_owner,
        })
    }

    pub fn from_browser_resource_runtime(resource_runtime: BrowserResourceRuntime) -> Self {
        Self::from_browser_resource_runtime_with_page_network_policy(
            resource_runtime,
            PageNetworkPolicy::default(),
        )
    }

    pub fn from_browser_resource_runtime_with_page_network_policy(
        resource_runtime: BrowserResourceRuntime,
        page_network_policy: PageNetworkPolicy,
    ) -> Self {
        Self {
            resource_runtime,
            page_network_policy,
            browser_site_context: None,
        }
    }

    pub(crate) fn with_browser_site_context(
        mut self,
        browser_site_context: BrowserCookieFacadeContext,
    ) -> Self {
        self.browser_site_context = Some(Arc::new(browser_site_context));
        self
    }

    pub(crate) fn with_shared_browser_site_context(
        mut self,
        browser_site_context: Arc<BrowserCookieFacadeContext>,
    ) -> Self {
        self.browser_site_context = Some(browser_site_context);
        self
    }

    pub(crate) fn browser_site_context(&self) -> Option<&BrowserCookieFacadeContext> {
        self.browser_site_context.as_deref()
    }

    pub(crate) fn shared_browser_site_context(&self) -> Option<Arc<BrowserCookieFacadeContext>> {
        self.browser_site_context.clone()
    }

    pub fn page_network_policy(&self) -> PageNetworkPolicy {
        self.page_network_policy.clone()
    }

    pub(crate) fn frozen_request_client(&self) -> Self {
        let mut client = Self::from_browser_resource_runtime_with_page_network_policy(
            self.resource_runtime.clone(),
            self.page_network_policy.frozen_request_view(),
        );
        client.browser_site_context = self.browser_site_context.clone();
        client
    }

    pub fn shares_page_network_policy_with(&self, other: &Self) -> bool {
        self.page_network_policy
            .shares_state_with(&other.page_network_policy)
    }

    pub fn shares_memory_cache_partition_with(&self, other: &Self) -> bool {
        self.page_network_policy
            .shares_memory_cache_partition_with(&other.page_network_policy)
    }

    /// Creates a target-owned adapter that shares only transport/cache state.
    pub fn fork_with_isolated_page_network_policy(&self) -> Self {
        Self::from_browser_resource_runtime_with_page_network_policy(
            self.resource_runtime.clone(),
            self.page_network_policy.isolated_copy(),
        )
    }

    /// Creates a Document-owned adapter inside this client's Page target.
    pub(crate) fn fork_with_isolated_document_network_policy(&self) -> Self {
        Self::from_browser_resource_runtime_with_page_network_policy(
            self.resource_runtime.clone(),
            self.page_network_policy.isolated_same_page_copy(),
        )
    }

    /// Creates a Worker-owned adapter while retaining the creator Document's
    /// browser-site context for cookie and Fetch Metadata decisions.
    pub(crate) fn fork_with_isolated_worker_network_policy(&self) -> Self {
        let mut client = Self::from_browser_resource_runtime_with_page_network_policy(
            self.resource_runtime.clone(),
            self.page_network_policy.isolated_same_page_copy(),
        );
        client.browser_site_context = self.browser_site_context.clone();
        client
    }

    /// Materializes a text response at callers that require a buffered body.
    ///
    /// New renderer resource loads should prefer `fetch_text_stream()` or
    /// `fetch_text_stream_with_cancel()` so the HTTP cache and network body stay
    /// streaming until the current JS/Web API boundary requires text.
    pub async fn fetch(&self, request: Request) -> Result<Response> {
        self.fetch_with_optional_cancel(request, None).await
    }

    /// Materializes a raw response at callers that require a buffered body.
    ///
    /// Prefer `fetch_raw_stream_with_cancel()` when the caller can consume raw
    /// chunks or pass them into another streaming boundary.
    pub async fn fetch_raw(&self, request: Request) -> Result<RawResponse> {
        let request = self.apply_network_policy(request)?;
        self.resource_runtime.client().fetch_raw(request).await
    }

    pub async fn fetch_raw_with_network_metadata(
        &self,
        request: Request,
    ) -> Result<NetworkFetchResult<RawResponse>> {
        let request = self.apply_network_policy(request)?;
        self.resource_runtime
            .client()
            .fetch_raw_with_network_metadata(request)
            .await
    }

    pub async fn fetch_raw_stream_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<StreamingRawResponse> {
        let request = self.apply_network_policy(request)?;
        self.fetch_raw_stream_with_cancel_after_policy(request, cancel_handle)
            .await
    }

    pub async fn fetch_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<Response> {
        self.fetch_with_optional_cancel(request, Some(cancel_handle))
            .await
    }

    pub async fn fetch_text_stream(&self, request: Request) -> Result<Response> {
        self.fetch_text_stream_with_cancel(request, FetchCancelHandle::new())
            .await
    }

    pub async fn fetch_text_stream_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<Response> {
        let request = self.apply_network_policy(request)?;
        self.fetch_text_stream_with_cancel_after_policy(request, cancel_handle)
            .await
    }

    pub(crate) async fn fetch_text_stream_with_network_metadata(
        &self,
        request: Request,
    ) -> Result<NetworkFetchResult<Response>> {
        self.fetch_text_stream_with_cancel_and_network_metadata(request, FetchCancelHandle::new())
            .await
    }

    pub(crate) async fn fetch_text_stream_with_cancel_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<Response>> {
        let request = self.apply_network_policy(request)?;
        if request.auth_requires_buffered_transport() || !request.follow_redirects {
            return self
                .resource_runtime
                .client()
                .fetch_with_cancel_and_network_metadata(request, cancel_handle)
                .await;
        }

        let observed = self
            .fetch_raw_stream_with_cancel_after_policy_and_network_metadata(request, cancel_handle)
            .await?;
        let (response, observation_journal) = observed.into_parts_with_observation_journal();
        let response = collect_streaming_raw_response_as_text(response).await?;
        Ok(NetworkFetchResult::with_observation_journal(
            response,
            observation_journal,
        ))
    }

    pub(crate) async fn fetch_cacheable_script_text_stream(
        &self,
        request: Request,
    ) -> Result<Response> {
        let request = self.apply_network_policy(request)?;
        if let Some(result) = local_text_response(&request.url) {
            return result;
        }
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let timing_url = timing_enabled.then(|| request.url.to_string());
        if !script_text_request_is_memory_cacheable(&request) {
            let started = timing_enabled.then(Instant::now);
            if let Some(url) = timing_url.as_deref() {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url,
                    cacheable = false,
                    cache_role = "direct",
                    stage = "script_text_request_start",
                );
            }
            let result = self
                .fetch_text_stream_with_cancel_after_policy(request, FetchCancelHandle::new())
                .await;
            if let (Some(started), Some(url)) = (started, timing_url.as_deref()) {
                let status = result.as_ref().ok().map(|response| response.status);
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url,
                    cacheable = false,
                    cache_role = "direct",
                    status,
                    ok = result.is_ok(),
                    elapsed_ms = started.elapsed().as_millis(),
                    stage = "script_text_request_done",
                );
            }
            return result;
        }

        let page_cache_partition_id = self.page_network_policy.memory_cache_partition_id();
        if let Some(url) = timing_url.as_deref() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                page_cache_partition_id,
                credentials_mode = request.credentials_mode.as_ref(),
                cookie_context = ?request.cookie_context,
                stage = "script_text_cache_lookup",
            );
        }
        let key = script_text_cache_key(&request).for_page_cache_partition(page_cache_partition_id);
        let lookup = {
            let mut cache = self.resource_runtime.memory_cache().lock();
            if request.cache_mode().allows_memory_cache_lookup() {
                cache.lookup_script_text(key.clone())
            } else {
                cache.replace_script_text(key.clone())
            }
        };

        let load = match lookup {
            ScriptTextCacheLookup::Owner(load) => load,
            ScriptTextCacheLookup::PendingWaiter(load) => {
                let started = timing_enabled.then(Instant::now);
                if let Some(url) = timing_url.as_deref() {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url,
                        cacheable = true,
                        cache_role = "waiter",
                        stage = "script_text_cache_wait_start",
                    );
                }
                let result = load.wait().await.map_err(anyhow::Error::msg);
                if let (Some(started), Some(url)) = (started, timing_url.as_deref()) {
                    let status = result.as_ref().ok().map(|response| response.status);
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url,
                        cacheable = true,
                        cache_role = "waiter",
                        status,
                        ok = result.is_ok(),
                        elapsed_ms = started.elapsed().as_millis(),
                        stage = "script_text_cache_wait_done",
                    );
                }
                return result;
            }
            ScriptTextCacheLookup::CompletedHit(result) => {
                let result = result
                    .map(response_with_memory_cache_hit)
                    .map_err(anyhow::Error::msg);
                if let Some(url) = timing_url.as_deref() {
                    let status = result.as_ref().ok().map(|response| response.status);
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url,
                        cacheable = true,
                        cache_role = "hit",
                        status,
                        ok = result.is_ok(),
                        stage = "script_text_cache_hit",
                    );
                }
                return result;
            }
        };

        let started = timing_enabled.then(Instant::now);
        if let Some(url) = timing_url.as_deref() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                cache_role = "owner",
                stage = "script_text_request_start",
            );
        }
        let cache_request = request.clone();
        let result = self
            .fetch_text_stream_with_cancel_after_policy(request, FetchCancelHandle::new())
            .await
            .map_err(|error| format!("{error:#}"));
        if let (Some(started), Some(url)) = (started, timing_url.as_deref()) {
            let status = result.as_ref().ok().map(|response| response.status);
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                cache_role = "owner",
                status,
                ok = result.is_ok(),
                elapsed_ms = started.elapsed().as_millis(),
                stage = "script_text_request_done",
            );
        }
        self.resource_runtime
            .memory_cache()
            .lock()
            .complete_script_text(&key, &load, &cache_request, &result);
        load.finish(result.clone());
        result.map_err(anyhow::Error::msg)
    }

    pub(crate) fn fetch_cacheable_script_text_callback_with_load<F>(
        &self,
        request: Request,
        resource_load: loads::ResourceLoadLease,
        callback: F,
    ) -> Result<()>
    where
        F: FnOnce(Result<Response>) + Send + 'static,
    {
        self.fetch_cacheable_script_text_callback_inner(request, resource_load, callback)
    }

    fn fetch_cacheable_script_text_callback_inner<F>(
        &self,
        request: Request,
        resource_load: loads::ResourceLoadLease,
        callback: F,
    ) -> Result<()>
    where
        F: FnOnce(Result<Response>) + Send + 'static,
    {
        let request = self.apply_network_policy(request)?;
        if let Some(result) = local_text_response(&request.url) {
            let task_runner = resource_load.task_runner();
            task_runner.spawn(async move {
                resource_load.finish();
                callback(result);
            });
            return Ok(());
        }
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let timing_url = timing_enabled.then(|| request.url.to_string());
        if !script_text_request_is_memory_cacheable(&request) {
            let started = timing_enabled.then(Instant::now);
            if let Some(url) = timing_url.as_deref() {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url,
                    cacheable = false,
                    cache_role = "direct-callback",
                    stage = "script_text_request_start",
                );
            }
            let cancel_handle = FetchCancelHandle::new();
            resource_load.attach_cancel_handle(cancel_handle.clone());
            let callback_resource_load = resource_load.clone();
            let started_fetch = self.fetch_text_callback_with_cancel_after_policy(
                request,
                cancel_handle,
                move |result| {
                    callback_resource_load.finish();
                    if let (Some(started), Some(url)) = (started, timing_url.as_deref()) {
                        let status = result.as_ref().ok().map(|response| response.status);
                        tracing::info!(
                            target: "moli_cdp_nav_timing",
                            url,
                            cacheable = false,
                            cache_role = "direct-callback",
                            status,
                            ok = result.is_ok(),
                            elapsed_ms = started.elapsed().as_millis(),
                            stage = "script_text_request_done",
                        );
                    }
                    callback(result);
                },
            );
            if started_fetch.is_err() {
                resource_load.finish();
            }
            return started_fetch;
        }

        let page_cache_partition_id = self.page_network_policy.memory_cache_partition_id();
        if let Some(url) = timing_url.as_deref() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                page_cache_partition_id,
                credentials_mode = request.credentials_mode.as_ref(),
                cookie_context = ?request.cookie_context,
                stage = "script_text_cache_lookup_callback",
            );
        }
        let key = script_text_cache_key(&request).for_page_cache_partition(page_cache_partition_id);
        let lookup = {
            let mut cache = self.resource_runtime.memory_cache().lock();
            if request.cache_mode().allows_memory_cache_lookup() {
                cache.lookup_script_text(key.clone())
            } else {
                cache.replace_script_text(key.clone())
            }
        };

        let (load, owns_transport) = match lookup {
            ScriptTextCacheLookup::Owner(load) => (load, true),
            ScriptTextCacheLookup::PendingWaiter(load) => (load, false),
            ScriptTextCacheLookup::CompletedHit(result) => {
                let result = result
                    .map(response_with_memory_cache_hit)
                    .map_err(anyhow::Error::msg);
                resource_load.finish();
                if let Some(url) = timing_url.as_deref() {
                    let status = result.as_ref().ok().map(|response| response.status);
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url,
                        cacheable = true,
                        cache_role = "hit-callback",
                        status,
                        ok = result.is_ok(),
                        stage = "script_text_cache_hit",
                    );
                }
                callback(result);
                return Ok(());
            }
        };

        let cache_role = if owns_transport {
            "owner-callback"
        } else {
            "waiter-callback"
        };
        let started = timing_enabled.then(Instant::now);
        if let Some(url) = timing_url.as_deref() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                cache_role,
                stage = "script_text_cache_wait_start",
            );
        }
        let callback_resource_load = resource_load.clone();
        let callback_timing_url = timing_url.clone();
        let consumer = load.wait_callback(Box::new(move |result| {
            callback_resource_load.finish();
            if let (Some(started), Some(url)) = (started, callback_timing_url.as_deref()) {
                let status = result.as_ref().ok().map(|response| response.status);
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url,
                    cacheable = true,
                    cache_role,
                    status,
                    ok = result.is_ok(),
                    elapsed_ms = started.elapsed().as_millis(),
                    stage = "script_text_cache_wait_done",
                );
            }
            callback(result.map_err(anyhow::Error::msg));
        }));
        if let Some(consumer) = consumer {
            resource_load.attach_consumer_cancel(move || consumer.cancel());
        }
        if !owns_transport {
            return Ok(());
        }

        let request_client = self.clone();
        let owner_load = Arc::clone(&load);
        if let Some(url) = timing_url.as_deref() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url,
                cacheable = true,
                cache_role = "owner-callback",
                stage = "script_text_request_start",
            );
        }
        let cache_request = request.clone();
        let callback_cache_request = cache_request.clone();
        let callback_key = key.clone();
        let cancel_handle = FetchCancelHandle::new();
        load.attach_transport_cancel(cancel_handle.clone());
        if let Err(error) = self.fetch_text_callback_with_cancel_after_policy(
            request,
            cancel_handle,
            move |result| {
                let result = result.map_err(|error| format!("{error:#}"));
                request_client
                    .resource_runtime
                    .memory_cache()
                    .lock()
                    .complete_script_text(
                        &callback_key,
                        &owner_load,
                        &callback_cache_request,
                        &result,
                    );
                owner_load.finish(result.clone());
            },
        ) {
            let result = Err(format!("{error:#}"));
            self.resource_runtime
                .memory_cache()
                .lock()
                .complete_script_text(&key, &load, &cache_request, &result);
            load.finish(result);
        }
        Ok(())
    }

    async fn fetch_text_stream_with_cancel_after_policy(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<Response> {
        if request.auth_requires_buffered_transport() || !request.follow_redirects {
            // Challenge-response schemes still need libcurl's buffered auth
            // retry behavior until the streaming collector models
            // intermediate authentication challenges explicitly. Manual
            // redirect callers need the intermediate 3xx response before raw
            // streaming starts.
            return self
                .resource_runtime
                .client()
                .fetch_with_cancel(request, cancel_handle)
                .await;
        }

        let response = self
            .fetch_raw_stream_with_cancel_after_policy(request, cancel_handle)
            .await?;
        collect_streaming_raw_response_as_text(response).await
    }

    fn fetch_text_callback_with_cancel_after_policy<F>(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
        callback: F,
    ) -> Result<()>
    where
        F: FnOnce(Result<Response>) + Send + 'static,
    {
        if let Some(result) = local_text_response(&request.url) {
            callback(result);
            return Ok(());
        }
        self.resource_runtime
            .client()
            .fetch_with_cancel_callback(request, cancel_handle, callback)
    }

    async fn fetch_raw_stream_with_cancel_after_policy(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<StreamingRawResponse> {
        if let Some(response) = local_text_response(&request.url) {
            return streaming_raw_response_from_local_response(response?);
        }

        let cache_key = raw_subresource_memory_cache_key(&request).map(|key| {
            key.for_page_cache_partition(self.page_network_policy.memory_cache_partition_id())
        });
        if request.cache_mode().allows_memory_cache_lookup()
            && let Some(cache_key) = cache_key.as_ref()
            && let Some(cached) = self
                .resource_runtime
                .memory_cache()
                .lock()
                .lookup_raw_subresource(cache_key)
        {
            return streaming_raw_response_from_cached_subresource(cached);
        }

        let response = self
            .resource_runtime
            .client()
            .fetch_raw_stream_with_cancel(request.clone(), cancel_handle)
            .await?
            .with_lifetime_lease(self.resource_runtime.clone());
        Ok(if let Some(cache_key) = cache_key {
            self.tee_raw_subresource_response_for_memory_cache(request, cache_key, response)
        } else {
            response
        })
    }

    async fn fetch_raw_stream_with_cancel_after_policy_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>> {
        if let Some(response) = local_text_response(&request.url) {
            return Ok(NetworkFetchResult::without_request_observation(
                streaming_raw_response_from_local_response(response?)?,
            ));
        }

        let cache_key = raw_subresource_memory_cache_key(&request).map(|key| {
            key.for_page_cache_partition(self.page_network_policy.memory_cache_partition_id())
        });
        if request.cache_mode().allows_memory_cache_lookup()
            && let Some(cache_key) = cache_key.as_ref()
            && let Some(cached) = self
                .resource_runtime
                .memory_cache()
                .lock()
                .lookup_raw_subresource(cache_key)
        {
            return Ok(NetworkFetchResult::without_request_observation(
                streaming_raw_response_from_cached_subresource(cached)?,
            ));
        }

        let observed = self
            .resource_runtime
            .client()
            .fetch_raw_stream_with_cancel_and_network_metadata(request.clone(), cancel_handle)
            .await?;
        let (response, observation_journal) = observed.into_parts_with_observation_journal();
        let response = response.with_lifetime_lease(self.resource_runtime.clone());
        let response = if let Some(cache_key) = cache_key {
            self.tee_raw_subresource_response_for_memory_cache(request, cache_key, response)
        } else {
            response
        };
        Ok(NetworkFetchResult::with_observation_journal(
            response,
            observation_journal,
        ))
    }

    pub async fn fetch_raw_stream_with_cancel_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>> {
        let request = self.apply_network_policy(request)?;
        self.fetch_raw_stream_with_cancel_after_policy_and_network_metadata(request, cancel_handle)
            .await
    }

    fn tee_raw_subresource_response_for_memory_cache(
        &self,
        request: Request,
        cache_key: RawSubresourceCacheKey,
        response: StreamingRawResponse,
    ) -> StreamingRawResponse {
        self.tee_raw_subresource_response_for_memory_cache_with_body_eof_observer(
            request, cache_key, response, None,
        )
    }

    fn tee_raw_subresource_response_for_memory_cache_with_body_eof_observer(
        &self,
        request: Request,
        cache_key: RawSubresourceCacheKey,
        mut response: StreamingRawResponse,
        mut body_eof_observer: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> StreamingRawResponse {
        let head = response.head();
        let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        let resource_runtime = self.resource_runtime.clone();
        tokio::spawn(async move {
            let mut body = Vec::new();
            let completion = 'forwarding: {
                loop {
                    let chunk = tokio::select! {
                        // Dropping the outward response closes this sender's
                        // receiver. Win that cancellation boundary even when the
                        // upstream transfer has stalled between body chunks.
                        _ = body_tx.closed() => break 'forwarding None,
                        chunk = response.next_chunk() => chunk,
                    };
                    let Some(chunk) = chunk else {
                        if let Some(observer) = body_eof_observer.take() {
                            let _ = observer.send(());
                        }
                        break;
                    };
                    body.extend_from_slice(&chunk);
                    if body_tx.send(chunk).is_err() {
                        break 'forwarding None;
                    }
                }
                let terminal = tokio::select! {
                    // Body EOF and terminal transfer completion are separate
                    // boundaries. If the outward response is discarded in that
                    // interval, do not keep the exact runtime lease alive while
                    // waiting for an upstream completion that may never arrive.
                    _ = body_tx.closed() => break 'forwarding None,
                    completion = response.finish() => completion,
                };
                Some(terminal)
            };

            if completion.as_ref().is_some_and(Result::is_ok) {
                let materialized = RawResponse::from_head_and_body(response.head(), body);
                if let Some(expires_at_unix_ms) =
                    raw_subresource_memory_cache_expiry(&request, &materialized)
                {
                    resource_runtime
                        .memory_cache()
                        .lock()
                        .insert_raw_subresource(cache_key, materialized, expires_at_unix_ms);
                }
            }

            // The outward terminal is also the exact cleanup barrier. Release
            // the cache writer's runtime handle before dropping the inner
            // response (which closes its pending completion sender), and do
            // both before publishing the outward terminal. An observer can
            // therefore reap this retired runtime without racing Tokio's later
            // destruction of the completed task future.
            drop(resource_runtime);
            drop(response);
            if let Some(completion) = completion {
                let _ = completion_tx.send(completion);
            }
        });
        StreamingRawResponse::new_with_head(head, body_rx, FetchCancelHandle::new(), completion_rx)
    }

    pub(crate) fn fetch_text_callback<F>(&self, request: Request, callback: F) -> Result<()>
    where
        F: FnOnce(Result<Response>) + Send + 'static,
    {
        let request = self.apply_network_policy(request)?;
        self.fetch_text_callback_with_cancel_after_policy(
            request,
            FetchCancelHandle::new(),
            callback,
        )
    }

    pub(crate) fn fetch_text_for_worker_blocking_boundary(
        &self,
        request: Request,
    ) -> Result<Response> {
        self.fetch_text_for_worker_blocking_boundary_with_cancel(request, FetchCancelHandle::new())
    }

    pub(crate) fn fetch_text_for_worker_blocking_boundary_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<Response> {
        let request_client = self.clone();
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("lm-worker-script-fetch".to_owned())
            .spawn(move || {
                // importScripts() and module-worker graph loading are
                // synchronous script boundaries. Run the async request client on a
                // helper runtime and use the text streaming path before the
                // final source string materialization.
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to build worker script fetch runtime")
                    .and_then(|runtime| {
                        runtime.block_on(
                            request_client.fetch_text_stream_with_cancel(request, cancel_handle),
                        )
                    });
                let _ = response_tx.send(result);
            })
            .context("failed to spawn worker script fetch thread")?;
        response_rx
            .recv()
            .context("worker script fetch thread dropped response channel")?
    }

    pub(crate) fn fetch_raw_for_blocking_boundary(&self, request: Request) -> Result<RawResponse> {
        let request_client = self.clone();
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("lm-blocking-raw-fetch".to_owned())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to build blocking raw fetch runtime")
                    .and_then(|runtime| runtime.block_on(request_client.fetch_raw(request)));
                let _ = response_tx.send(result);
            })
            .context("failed to spawn blocking raw fetch thread")?;
        response_rx
            .recv()
            .context("blocking raw fetch thread dropped response channel")?
    }

    async fn fetch_with_optional_cancel(
        &self,
        request: Request,
        cancel_handle: Option<FetchCancelHandle>,
    ) -> Result<Response> {
        let request = self.apply_network_policy(request)?;
        if request.auth_requires_buffered_transport() || !request.follow_redirects {
            // Digest auth retries are still completed inside libcurl on the
            // buffered path. Keep auth requests there until the streaming
            // collector can distinguish intermediate auth challenges from
            // final responses. Manual redirect callers also need buffered
            // access to intermediate 3xx responses.
            return match cancel_handle {
                Some(cancel_handle) => {
                    self.resource_runtime
                        .client()
                        .fetch_with_cancel(request, cancel_handle)
                        .await
                }
                None => self.resource_runtime.client().fetch(request).await,
            };
        }

        let cancel_handle = cancel_handle.unwrap_or_default();
        let response = self
            .fetch_raw_stream_with_cancel_after_policy(request, cancel_handle)
            .await?;
        response.into_lossy_materialized_text_response().await
    }

    pub fn user_agent(&self) -> &str {
        self.resource_runtime.client().user_agent()
    }

    pub fn browser_identity(&self) -> &moli_browser_profile::BrowserIdentityProfile {
        self.resource_runtime.client().browser_identity()
    }

    pub fn http_proxy(&self) -> Option<&str> {
        self.resource_runtime.client().http_proxy()
    }

    pub fn http_no_proxy(&self) -> Option<&str> {
        self.resource_runtime.client().http_no_proxy()
    }

    pub fn proxy_bearer_token(&self) -> Option<&str> {
        self.resource_runtime.client().proxy_bearer_token()
    }

    pub fn tls_verify_host(&self) -> bool {
        self.resource_runtime.client().tls_verify_host()
    }

    pub fn request_timeout_ms(&self) -> u64 {
        self.resource_runtime.client().request_timeout_ms()
    }

    pub fn cookie_store(&self) -> SharedBrowserCookieStore {
        self.resource_runtime.client().cookie_store()
    }

    pub fn set_extra_http_headers(&self, headers: &[(String, String)]) {
        self.page_network_policy.set_extra_http_headers(headers);
    }

    pub fn set_network_offline(&self, offline: bool) {
        self.page_network_policy.set_network_offline(offline);
    }

    pub fn set_blocked_url_patterns(&self, patterns: &[String]) {
        self.page_network_policy.set_blocked_url_patterns(patterns);
    }

    pub fn set_image_fetch_enabled(&self, enabled: bool) {
        self.set_optional_resource_fetch_enabled(SubresourceResourceType::Image, enabled);
    }

    pub fn image_fetch_enabled(&self) -> bool {
        self.optional_resource_fetch_enabled(SubresourceResourceType::Image)
    }

    pub fn set_optional_resource_fetch_mask(&self, mask: OptionalResourceFetchMask) {
        self.page_network_policy
            .set_optional_resource_fetch_mask(mask);
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.page_network_policy.optional_resource_fetch_mask()
    }

    pub fn set_optional_resource_fetch_enabled(
        &self,
        resource_type: SubresourceResourceType,
        enabled: bool,
    ) {
        self.page_network_policy
            .set_optional_resource_fetch_enabled(resource_type, enabled);
    }

    pub fn optional_resource_fetch_enabled(&self, resource_type: SubresourceResourceType) -> bool {
        self.page_network_policy
            .optional_resource_fetch_enabled(resource_type)
    }

    pub fn set_subframe_loading_enabled(&self, enabled: bool) {
        self.page_network_policy
            .set_subframe_loading_enabled(enabled);
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.page_network_policy.subframe_loading_enabled()
    }

    pub fn author_styles_disabled(&self) -> bool {
        self.page_network_policy.author_styles_disabled()
    }

    pub fn set_bypass_service_worker(&self, bypass: bool) {
        self.page_network_policy.set_bypass_service_worker(bypass);
    }

    pub fn bypass_service_worker(&self) -> bool {
        self.page_network_policy.bypass_service_worker()
    }

    pub fn set_cache_disabled(&self, disabled: bool) {
        self.page_network_policy.set_cache_disabled(disabled);
    }

    fn apply_network_policy(&self, mut request: Request) -> Result<Request> {
        if let Some(browser_site_context) = self.browser_site_context.as_deref() {
            request = request.with_browser_site_context(browser_site_context.clone());
        }
        self.page_network_policy
            .snapshot()
            .apply_to_request(request)
    }
}

impl ResourceRequestClientOwner {
    pub fn handle(&self) -> ResourceRequestClient {
        self.client.clone()
    }
}

impl std::ops::Deref for ResourceRequestClientOwner {
    type Target = ResourceRequestClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl fmt::Debug for ResourceRequestClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceRequestClient")
            .field("resource_runtime", &self.resource_runtime)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ResourceRequestClientOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceRequestClientOwner")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

async fn collect_streaming_raw_response_as_text(
    response: StreamingRawResponse,
) -> Result<Response> {
    response.into_lossy_materialized_text_response().await
}

fn streaming_raw_response_from_cached_subresource(
    response: RawResponse,
) -> Result<StreamingRawResponse> {
    let mut head = response.head();
    head.from_cache = true;
    for redirect in &mut head.redirect_chain {
        redirect.from_cache = true;
        redirect.network_extra_info_available = false;
    }
    streaming_raw_response_from_head_and_body(head, response.clone_body_bytes())
}

fn local_text_response(url: &url::Url) -> Option<Result<Response>> {
    crate::network_host::local_url_response_result(url)
        .map(|result| result.map_err(anyhow::Error::msg))
}

fn streaming_raw_response_from_local_response(response: Response) -> Result<StreamingRawResponse> {
    let raw_response = response.into_materialized_raw_response();
    streaming_raw_response_from_head_and_body(raw_response.head(), raw_response.clone_body_bytes())
}

fn response_with_memory_cache_hit(mut response: Response) -> Response {
    response.from_cache = true;
    for redirect in &mut response.redirect_chain {
        redirect.from_cache = true;
        redirect.network_extra_info_available = false;
    }
    response
}

fn streaming_raw_response_from_head_and_body(
    head: ResponseHead,
    body: Vec<u8>,
) -> Result<StreamingRawResponse> {
    let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    if !body.is_empty() {
        body_tx
            .send(body)
            .map_err(|_| anyhow!("failed to enqueue cached raw subresource body"))?;
    }
    drop(body_tx);
    let _ = completion_tx.send(Ok(()));
    Ok(StreamingRawResponse::new_with_head(
        head,
        body_rx,
        FetchCancelHandle::new(),
        completion_rx,
    ))
}

#[cfg(test)]
mod tests;

use std::{fmt, marker::PhantomData, ops::Deref, rc::Rc, sync::Arc};

use anyhow::{Context, Result};
use moli_cookie_jar::SharedBrowserCookieStore;

use crate::{
    FetchCancelHandle, FetchConfig, NetworkFetchResult, RawResponse, Request, Response,
    StreamingHtmlResponse, StreamingRawResponse,
    client_hints::ClientHintPreferences,
    network_fetch_result::{NetworkFetchFailureContext, NetworkObservationRecorder},
    runtime::{FetchRuntimeHandle, FetchRuntimeJoinReport, FetchRuntimeOwner},
};

/// Cloneable request-side access to a fetch client.
///
/// The semantic thread owner is intentionally absent from this type, so a
/// completion callback may capture a handle without acquiring join ownership.
#[derive(Clone)]
pub struct FetchClientHandle {
    config: FetchConfig,
    cookie_store: SharedBrowserCookieStore,
    runtime: FetchRuntimeHandle,
}

/// Unique structured-concurrency owner for one fetch client.
///
/// This preserves the established `FetchClient::new(...)` construction
/// surface while keeping the semantic thread's join authority out of every
/// cloneable request handle. Deref exposes [`FetchClientHandle`] for ordinary
/// request operations; `clone()` therefore clones only that handle.
pub struct FetchClient {
    handle: FetchClientHandle,
    runtime_owner: FetchRuntimeOwner,
    _thread_affine: PhantomData<Rc<()>>,
}

impl FetchClientHandle {
    /// Materialized text compatibility API.
    ///
    /// Non-auth requests enter the streaming raw transport first and only
    /// materialize at this API boundary. New call sites should prefer
    /// `fetch_raw_stream_with_cancel()` or `fetch_html_stream()` when they can
    /// consume chunks directly.
    pub async fn fetch(&self, request: Request) -> Result<Response> {
        self.fetch_with_cancel(request, FetchCancelHandle::new())
            .await
    }

    /// Materialized raw compatibility API.
    ///
    /// Non-auth requests enter the streaming raw transport first and only
    /// materialize at this API boundary. Auth challenge-response still uses the
    /// buffered libcurl path so intermediate 401/407 bodies are hidden.
    pub async fn fetch_raw(&self, request: Request) -> Result<RawResponse> {
        if request.auth_requires_buffered_transport() {
            // Digest auth retries are still completed inside libcurl on the
            // buffered path. Keep auth requests there until the raw streaming
            // collector can model intermediate auth challenges without
            // surfacing them as final responses.
            return self
                .runtime
                .submit_auth_raw(request)?
                .await
                .context("fetch runtime task dropped raw response channel")?;
        }

        let response = self
            .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
            .await?;
        response.into_materialized_raw_response().await
    }

    pub async fn fetch_raw_with_network_metadata(
        &self,
        request: Request,
    ) -> Result<NetworkFetchResult<RawResponse>> {
        let recorder = NetworkObservationRecorder::default();
        let request = request.with_network_observation_recorder(recorder.clone());
        network_fetch_result_from_result(self.fetch_raw(request).await, recorder)
    }

    pub async fn fetch_raw_stream_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<StreamingRawResponse> {
        self.runtime
            .submit_raw_stream(request, cancel_handle)?
            .into_response()
            .await
    }

    pub async fn fetch_raw_stream_with_cancel_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>> {
        let recorder = NetworkObservationRecorder::default();
        let request = request.with_network_observation_recorder(recorder.clone());
        network_fetch_result_from_result(
            self.fetch_raw_stream_with_cancel(request, cancel_handle)
                .await,
            recorder,
        )
    }

    pub async fn fetch_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<Response> {
        if request.auth_requires_buffered_transport() || !request.follow_redirects {
            // Auth retries still need the buffered libcurl path so
            // intermediate 401/407 challenge bodies are not exposed as final
            // streaming responses. Manual redirect callers also need the
            // intermediate 3xx response before any raw streaming body starts.
            return self
                .runtime
                .submit_with_cancel(request, cancel_handle)?
                .await
                .context("fetch runtime task dropped response channel")?;
        }

        let response = self
            .fetch_raw_stream_with_cancel(request, cancel_handle)
            .await?;
        response.into_lossy_materialized_text_response().await
    }

    pub async fn fetch_with_cancel_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<Response>> {
        let recorder = NetworkObservationRecorder::default();
        let request = request.with_network_observation_recorder(recorder.clone());
        network_fetch_result_from_result(
            self.fetch_with_cancel(request, cancel_handle).await,
            recorder,
        )
    }

    pub fn fetch_with_cancel_callback<F>(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
        callback: F,
    ) -> Result<()>
    where
        F: FnOnce(Result<Response>) + Send + 'static,
    {
        self.runtime
            .submit_with_cancel_callback(request, cancel_handle, Box::new(callback))
    }

    pub async fn fetch_html_stream(&self, request: Request) -> Result<StreamingHtmlResponse> {
        self.runtime
            .submit_html_stream(request)?
            .into_response()
            .await
    }

    pub fn user_agent(&self) -> &str {
        self.config.user_agent()
    }

    pub fn browser_identity(&self) -> &moli_browser_profile::BrowserIdentityProfile {
        self.config.browser_identity()
    }

    pub fn matches_config(&self, config: &FetchConfig) -> bool {
        &self.config == config
    }
    pub fn http_proxy(&self) -> Option<&str> {
        self.config.http_proxy()
    }

    pub fn http_no_proxy(&self) -> Option<&str> {
        self.config.http_no_proxy()
    }

    pub fn proxy_bearer_token(&self) -> Option<&str> {
        self.config.proxy_bearer_token()
    }

    pub fn tls_verify_host(&self) -> bool {
        self.config.tls_verify_host()
    }

    pub fn request_timeout_ms(&self) -> u64 {
        self.config.request_timeout_ms()
    }

    pub fn cookie_store(&self) -> SharedBrowserCookieStore {
        Arc::clone(&self.cookie_store)
    }

    /// Idempotently asks the semantic owner to stop without joining it.
    /// Structured owner roots use this when the last external runtime lease is
    /// released; only [`FetchClient`] may subsequently join.
    pub fn request_shutdown(&self) {
        self.runtime.request_shutdown();
    }

    #[cfg(test)]
    pub(crate) fn runtime_owner_count_for_testing(&self) -> usize {
        self.runtime.owner_count_for_testing()
    }
}

impl FetchClient {
    pub fn new(config: &FetchConfig, cookie_store: SharedBrowserCookieStore) -> Self {
        if let Err(error) = crate::blocking::trim_http_cache(config) {
            tracing::debug!("failed to trim HTTP cache during fetch client startup: {error}");
        }
        let client_hint_preferences =
            Arc::new(parking_lot::Mutex::new(ClientHintPreferences::default()));
        let runtime_owner = FetchRuntimeOwner::new_with_client_hint_preferences(
            config,
            Arc::clone(&cookie_store),
            Arc::clone(&client_hint_preferences),
        );
        let handle = FetchClientHandle {
            config: config.clone(),
            runtime: runtime_owner.handle(),
            cookie_store,
        };
        Self {
            handle,
            runtime_owner,
            _thread_affine: PhantomData,
        }
    }

    pub fn handle(&self) -> FetchClientHandle {
        self.handle.clone()
    }

    pub fn request_shutdown(&self) {
        self.runtime_owner.request_shutdown();
    }

    pub fn join(&mut self) -> FetchRuntimeJoinReport {
        self.runtime_owner.join()
    }

    pub fn shutdown(mut self) -> FetchRuntimeJoinReport {
        self.request_shutdown();
        self.runtime_owner.join()
    }
}

impl Deref for FetchClient {
    type Target = FetchClientHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

fn network_fetch_result_from_result<R>(
    result: Result<R>,
    recorder: NetworkObservationRecorder,
) -> Result<NetworkFetchResult<R>> {
    let observation_journal = recorder.snapshot();
    match result {
        Ok(response) => Ok(NetworkFetchResult::with_observation_journal(
            response,
            observation_journal,
        )),
        Err(source) if source.is::<NetworkFetchFailureContext>() => Err(source),
        Err(source) => Err(NetworkFetchFailureContext::attach(
            source,
            observation_journal,
        )),
    }
}

impl fmt::Debug for FetchClientHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchClientHandle")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for FetchClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchClient")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

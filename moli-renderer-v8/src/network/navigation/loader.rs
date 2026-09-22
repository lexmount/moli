use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use anyhow::{Result, bail, ensure};
use moli_cookie_jar::BrowserCookieFacadeContext;
use moli_fetch::{
    FetchCancelHandle, NetworkFetchResult, RawResponse, Request, Response, StreamingRawResponse,
};
use moli_url_policy::{LocalFileNavigationAccess, route_navigation_url};
use parking_lot::Mutex;

use crate::network::{RendererResourceTaskRunner, ResourceRequestClient};

use super::DocumentFetchContextSeed;

static NEXT_NAVIGATION_RESOURCE_LOADER_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationResourceLoaderState {
    Created,
    Fetching,
    ResponseReady,
    Committed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationResourceLoaderDiagnostics {
    pub loader_id: u64,
    pub browser_resource_runtime_id: u64,
    pub state: &'static str,
}

struct NavigationResourceLoaderInner {
    id: u64,
    state: Mutex<NavigationResourceLoaderState>,
    cancel: FetchCancelHandle,
}

/// The network authority for exactly one main-resource navigation attempt.
///
/// Dropping an in-flight attempt cancels its transport. Once a streaming
/// response head has been returned, the response also owns the same cancel
/// handle, so replacing or dropping that response still cancels its body
/// without keeping this authority alive artificially.
#[derive(Clone)]
pub struct NavigationResourceLoader {
    request_client: ResourceRequestClient,
    task_runner: RendererResourceTaskRunner,
    requested_url: url::Url,
    committed_document_browser_site_context: Option<Arc<BrowserCookieFacadeContext>>,
    inner: Arc<NavigationResourceLoaderInner>,
}

impl NavigationResourceLoader {
    pub fn new(
        request_client: ResourceRequestClient,
        requested_url: url::Url,
        task_runner: RendererResourceTaskRunner,
    ) -> Self {
        Self::new_with_cancel_handle(
            request_client,
            requested_url,
            task_runner,
            FetchCancelHandle::new(),
        )
    }

    pub fn new_with_cancel_handle(
        request_client: ResourceRequestClient,
        requested_url: url::Url,
        task_runner: RendererResourceTaskRunner,
        cancel: FetchCancelHandle,
    ) -> Self {
        Self::new_with_committed_document_browser_site_context(
            request_client,
            requested_url,
            task_runner,
            None,
            cancel,
        )
    }

    pub(crate) fn new_for_child_document(
        request_client: ResourceRequestClient,
        requested_url: url::Url,
        task_runner: RendererResourceTaskRunner,
    ) -> Self {
        let browser_site_context = request_client.shared_browser_site_context();
        Self::new_with_committed_document_browser_site_context(
            request_client,
            requested_url,
            task_runner,
            browser_site_context,
            FetchCancelHandle::new(),
        )
    }

    fn new_with_committed_document_browser_site_context(
        request_client: ResourceRequestClient,
        requested_url: url::Url,
        task_runner: RendererResourceTaskRunner,
        committed_document_browser_site_context: Option<Arc<BrowserCookieFacadeContext>>,
        cancel: FetchCancelHandle,
    ) -> Self {
        Self {
            request_client,
            task_runner,
            requested_url,
            committed_document_browser_site_context,
            inner: Arc::new(NavigationResourceLoaderInner {
                id: NEXT_NAVIGATION_RESOURCE_LOADER_ID
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1),
                state: Mutex::new(NavigationResourceLoaderState::Created),
                cancel,
            }),
        }
    }

    pub fn loader_id_for_diagnostics(&self) -> u64 {
        self.inner.id
    }

    pub fn state(&self) -> NavigationResourceLoaderState {
        *self.inner.state.lock()
    }

    pub fn diagnostics(&self) -> NavigationResourceLoaderDiagnostics {
        NavigationResourceLoaderDiagnostics {
            loader_id: self.loader_id_for_diagnostics(),
            browser_resource_runtime_id: self
                .request_client
                .resource_runtime_diagnostics()
                .runtime_id,
            state: state_name(self.state()),
        }
    }

    pub fn cancel(&self) {
        let mut state = self.inner.state.lock();
        if matches!(
            *state,
            NavigationResourceLoaderState::Created
                | NavigationResourceLoaderState::Fetching
                | NavigationResourceLoaderState::ResponseReady
        ) {
            self.inner.cancel.cancel();
            *state = NavigationResourceLoaderState::Cancelled;
        }
    }

    pub fn request_client(&self) -> &ResourceRequestClient {
        &self.request_client
    }

    pub(crate) fn task_runner(&self) -> RendererResourceTaskRunner {
        self.task_runner.clone()
    }

    pub(crate) fn spawn_resource_task(
        &self,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        self.task_runner.spawn(task);
    }

    pub async fn fetch(&self, request: Request) -> Result<Response> {
        self.begin_fetch()?;
        match self
            .request_client
            .fetch_with_cancel(
                request.with_page_network_policy(),
                self.inner.cancel.clone(),
            )
            .await
        {
            Ok(response) => {
                self.finish_response_ready()?;
                Ok(response)
            }
            Err(error) => {
                self.finish_failed();
                Err(error)
            }
        }
    }

    pub async fn fetch_with_network_metadata(
        &self,
        request: Request,
    ) -> Result<NetworkFetchResult<Response>> {
        self.begin_fetch()?;
        match self
            .request_client
            .fetch_text_stream_with_cancel_and_network_metadata(
                request.with_page_network_policy(),
                self.inner.cancel.clone(),
            )
            .await
        {
            Ok(response) => {
                self.finish_response_ready()?;
                Ok(response)
            }
            Err(error) => {
                self.finish_failed();
                Err(error)
            }
        }
    }

    pub async fn fetch_raw(&self, request: Request) -> Result<RawResponse> {
        self.begin_fetch()?;
        match self
            .request_client
            .fetch_raw_stream_with_cancel(
                request.with_page_network_policy(),
                self.inner.cancel.clone(),
            )
            .await
        {
            Ok(mut response) => {
                let head = response.head();
                let mut body = Vec::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.extend_from_slice(&chunk);
                }
                if let Err(error) = response.finish().await {
                    self.finish_failed();
                    return Err(error);
                }
                self.finish_response_ready()?;
                Ok(RawResponse::from_head_and_body(head, body))
            }
            Err(error) => {
                self.finish_failed();
                Err(error)
            }
        }
    }

    pub async fn fetch_raw_stream(&self, request: Request) -> Result<StreamingRawResponse> {
        self.fetch_raw_stream_with_network_metadata(request)
            .await
            .map(NetworkFetchResult::into_response)
    }

    pub async fn fetch_raw_stream_with_network_metadata(
        &self,
        request: Request,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>> {
        self.begin_fetch()?;
        match self
            .request_client
            .fetch_raw_stream_with_cancel_and_network_metadata(
                request.with_page_network_policy(),
                self.inner.cancel.clone(),
            )
            .await
        {
            Ok(response) => {
                self.finish_response_ready()?;
                Ok(response)
            }
            Err(error) => {
                self.finish_failed();
                Err(error)
            }
        }
    }

    pub fn note_service_worker_response_ready(&self) -> Result<()> {
        self.begin_fetch()?;
        self.finish_response_ready()
    }

    pub fn commit(&self, final_url: url::Url) -> Result<DocumentFetchContextSeed> {
        let mut state = self.inner.state.lock();
        ensure!(
            *state == NavigationResourceLoaderState::ResponseReady,
            "navigation resource loader cannot commit from state {:?}",
            *state
        );
        *state = NavigationResourceLoaderState::Committed;
        Ok(DocumentFetchContextSeed::new(
            self.requested_url.clone(),
            final_url,
            self.request_client.browser_resource_runtime(),
            self.request_client.page_network_policy(),
            self.committed_document_browser_site_context.clone(),
            self.task_runner.clone(),
        ))
    }

    fn begin_fetch(&self) -> Result<()> {
        route_navigation_url(&self.requested_url, LocalFileNavigationAccess::Denied)?;
        let mut state = self.inner.state.lock();
        ensure!(
            *state == NavigationResourceLoaderState::Created,
            "navigation resource loader can start only once (state: {:?})",
            *state
        );
        *state = NavigationResourceLoaderState::Fetching;
        Ok(())
    }

    fn finish_response_ready(&self) -> Result<()> {
        let mut state = self.inner.state.lock();
        match *state {
            NavigationResourceLoaderState::Fetching => {
                *state = NavigationResourceLoaderState::ResponseReady;
                Ok(())
            }
            NavigationResourceLoaderState::Cancelled => {
                bail!("navigation cancelled")
            }
            state => {
                bail!("navigation response cannot become ready from state {state:?}")
            }
        }
    }

    fn finish_failed(&self) {
        let mut state = self.inner.state.lock();
        if *state != NavigationResourceLoaderState::Cancelled {
            *state = NavigationResourceLoaderState::Failed;
        }
    }
}

impl Drop for NavigationResourceLoaderInner {
    fn drop(&mut self) {
        // The inner allocation is destroyed exactly once after the last outer
        // clone releases it. Performing cancellation here avoids a racy
        // `Arc::strong_count()` last-owner check in the outer wrapper.
        if matches!(
            *self.state.get_mut(),
            NavigationResourceLoaderState::Created | NavigationResourceLoaderState::Fetching
        ) {
            self.cancel.cancel();
        }
    }
}

fn state_name(state: NavigationResourceLoaderState) -> &'static str {
    match state {
        NavigationResourceLoaderState::Created => "created",
        NavigationResourceLoaderState::Fetching => "fetching",
        NavigationResourceLoaderState::ResponseReady => "responseReady",
        NavigationResourceLoaderState::Committed => "committed",
        NavigationResourceLoaderState::Failed => "failed",
        NavigationResourceLoaderState::Cancelled => "cancelled",
    }
}

impl std::fmt::Debug for NavigationResourceLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NavigationResourceLoader")
            .field("diagnostics", &self.diagnostics())
            .field("requested_url", &self.requested_url)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use moli_cookie_jar::BrowserCookieFacadeContext;
    use moli_fetch::{FetchConfig, Request};
    use url::Url;

    use crate::network::ResourceRequestClient;

    use super::{NavigationResourceLoader, NavigationResourceLoaderState};

    struct NavigationTestFixture {
        navigation: NavigationResourceLoader,
        _request_client_owner: crate::network::ResourceRequestClientOwner,
    }

    impl std::ops::Deref for NavigationTestFixture {
        type Target = NavigationResourceLoader;

        fn deref(&self) -> &Self::Target {
            &self.navigation
        }
    }

    fn navigation() -> NavigationTestFixture {
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
        NavigationTestFixture {
            navigation: NavigationResourceLoader::new(
                request_client_owner.handle(),
                Url::parse("https://example.test/start").expect("navigation URL"),
                crate::network::RendererResourceTaskRunner::from_current_tokio()
                    .expect("navigation authority test must own a Tokio runtime"),
            ),
            _request_client_owner: request_client_owner,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_fetching_attempt_cancels_only_that_attempt() {
        let navigation = navigation();
        let cancel = navigation.inner.cancel.clone();
        navigation.begin_fetch().expect("begin navigation");

        drop(navigation);

        assert!(cancel.is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_attempt_cannot_publish_an_uncommittable_response() {
        let navigation = navigation();
        navigation.begin_fetch().expect("begin navigation");
        navigation.cancel();

        let error = navigation
            .finish_response_ready()
            .expect_err("cancelled response must not become ready");

        assert_eq!(error.to_string(), "navigation cancelled");
        assert_eq!(navigation.state(), NavigationResourceLoaderState::Cancelled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn top_level_navigation_commit_resets_previous_document_site_context() {
        let context = BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(&Url::parse("https://old.test/").unwrap())
            .with_top_frame_origin_url(&Url::parse("https://old.test/").unwrap());
        let client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
        let client = client_owner.handle().with_browser_site_context(context);
        let navigation = NavigationResourceLoader::new(
            client,
            Url::parse("https://new.test/start").unwrap(),
            crate::network::RendererResourceTaskRunner::from_current_tokio().unwrap(),
        );

        navigation
            .note_service_worker_response_ready()
            .expect("navigation response");
        let seed = navigation
            .commit(Url::parse("https://new.test/final").unwrap())
            .expect("navigation commit");

        assert!(seed.browser_site_context().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn child_navigation_commit_preserves_top_frame_site_context() {
        let context = BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(&Url::parse("https://top.test/").unwrap())
            .with_top_frame_origin_url(&Url::parse("https://top.test/").unwrap());
        let client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
        let client = client_owner
            .handle()
            .with_browser_site_context(context.clone());
        let navigation = NavigationResourceLoader::new_for_child_document(
            client,
            Url::parse("https://frame.test/start").unwrap(),
            crate::network::RendererResourceTaskRunner::from_current_tokio().unwrap(),
        );

        navigation
            .note_service_worker_response_ready()
            .expect("navigation response");
        let seed = navigation
            .commit(Url::parse("https://frame.test/final").unwrap())
            .expect("navigation commit");

        assert_eq!(seed.browser_site_context(), Some(&context));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_last_clone_drops_cancel_fetching_attempt() {
        let NavigationTestFixture {
            navigation: first,
            _request_client_owner,
        } = navigation();
        first.begin_fetch().expect("begin navigation");
        let cancel = first.inner.cancel.clone();
        let second = first.clone();
        let barrier = Arc::new(Barrier::new(3));

        let first_barrier = Arc::clone(&barrier);
        let first_drop = std::thread::spawn(move || {
            first_barrier.wait();
            drop(first);
        });
        let second_barrier = Arc::clone(&barrier);
        let second_drop = std::thread::spawn(move || {
            second_barrier.wait();
            drop(second);
        });

        barrier.wait();
        first_drop.join().expect("first drop thread");
        second_drop.join().expect("second drop thread");

        assert!(cancel.is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn successful_attempt_commits_one_immutable_document_seed() {
        let navigation = navigation();
        navigation
            .note_service_worker_response_ready()
            .expect("response ready");
        let seed = navigation
            .commit(Url::parse("https://example.test/final").expect("final URL"))
            .expect("commit navigation");

        assert_eq!(navigation.state(), NavigationResourceLoaderState::Committed);
        assert_eq!(seed.requested_url().as_str(), "https://example.test/start");
        assert_eq!(seed.final_url().as_str(), "https://example.test/final");
        assert!(navigation.commit(seed.final_url().clone()).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hosted_navigation_rejects_file_url_before_loader_state_changes() {
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
        let file_url = Url::parse("file:///moli-policy-must-not-open").unwrap();
        let navigation = NavigationResourceLoader::new(
            request_client_owner.handle(),
            file_url.clone(),
            crate::network::RendererResourceTaskRunner::from_current_tokio().unwrap(),
        );

        let error = navigation
            .fetch_raw_stream(Request::get_with_url(file_url))
            .await
            .expect_err("hosted navigation must not receive local file capability");

        assert_eq!(
            error.to_string(),
            "Navigation to a local file URL requires an explicitly granted browser capability."
        );
        assert_eq!(navigation.state(), NavigationResourceLoaderState::Created);
    }
}

use std::{fmt, sync::Arc};

use super::ResourceTransfer;
use moli_fetch::{Response, ResponseHead};
use moli_page_types::SubresourceResponseBody;

#[derive(Clone, Debug)]
pub(crate) struct ResourceResponseHead {
    pub(crate) status_text: Option<String>,
    pub(crate) head: ResponseHead,
    pub(crate) network_request_headers: Option<Vec<(String, String)>>,
}

/// A failed stream still owns the response facts already received. Cache
/// consumers admitted near completion must not lose its head or partial body.
#[derive(Clone, Debug)]
pub(crate) enum ResourceResponseFailure {
    Request(String),
    Network {
        message: String,
        context: Arc<moli_fetch::NetworkFetchFailureContext>,
    },
    PartialBody {
        message: String,
        response: Arc<ResourceResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl ResourceResponseFailure {
    /// Reject at the physical head without waiting for its body. Retain any
    /// bytes already received and cancel the same transport.
    pub(crate) fn from_rejected_response(
        observed: moli_fetch::NetworkFetchResult<moli_fetch::StreamingRawResponse>,
        message: String,
    ) -> Self {
        let (mut response, request) = observed.into_parts();
        response.cancellation_handle().cancel();
        let mut body = moli_page_types::SubresourceResponseBodyWriter::default();
        while let Some(chunk) = response.try_next_chunk() {
            body.append(&chunk);
        }
        Self::PartialBody {
            message,
            response: Arc::new(ResourceResponseHead {
                status_text: None,
                head: response.head(),
                network_request_headers: request.map(|request| request.into_headers()),
            }),
            body: body.finish(),
        }
    }

    pub(crate) fn with_message(mut self, replacement: String) -> Self {
        match &mut self {
            Self::Request(message)
            | Self::Network { message, .. }
            | Self::PartialBody { message, .. } => *message = replacement,
        }
        self
    }
}

impl fmt::Display for ResourceResponseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message)
            | Self::Network { message, .. }
            | Self::PartialBody { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ResourceResponseFailure {}

impl From<String> for ResourceResponseFailure {
    fn from(message: String) -> Self {
        Self::Request(message)
    }
}

impl From<anyhow::Error> for ResourceResponseFailure {
    fn from(error: anyhow::Error) -> Self {
        let message = format!("{error:#}");
        match error.downcast::<moli_fetch::NetworkFetchFailureContext>() {
            Ok(context) => Self::Network {
                message,
                context: Arc::new(context),
            },
            Err(_) => Self::Request(message),
        }
    }
}

pub(crate) type ResourceResponseResult = Result<Response, ResourceResponseFailure>;

/// Synchronous native fact publication only: implementations must not execute
/// script, invoke completion callbacks, or re-enter the resource cache. This
/// lets cache admission replay its current progress before a later chunk wins.
pub(crate) trait ResourceResponseObserver: Send + Sync {
    fn response_started(&self, response: Arc<ResourceResponseHead>);
    fn data_received(&self, bytes: &[u8]);
    fn cancelled(&self, failure: &ResourceResponseFailure);
}

/// Preserve the physical response, including the received prefix on failure.
pub(crate) async fn collect_observed_response(
    observed: moli_fetch::NetworkFetchResult<moli_fetch::StreamingRawResponse>,
    observer: Option<&dyn ResourceResponseObserver>,
) -> ResourceResponseResult {
    let (mut response, request_observation) = observed.into_parts();
    let head = Arc::new(ResourceResponseHead {
        status_text: None,
        head: response.head(),
        network_request_headers: request_observation.map(|request| request.into_headers()),
    });
    if let Some(observer) = observer {
        observer.response_started(head.clone());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        bytes.extend_from_slice(&chunk);
        if let Some(observer) = observer {
            observer.data_received(&chunk);
        }
        // Let the Context owner admit the just-published receipt before a
        // prebuffered body monopolizes the shared resource executor.
        tokio::task::yield_now().await;
    }
    if let Err(error) = response.finish().await {
        return Err(ResourceResponseFailure::PartialBody {
            message: format!("{error:#}"),
            response: head,
            body: moli_page_types::SubresourceResponseBody::from_bytes(bytes),
        });
    }
    Ok(
        moli_fetch::RawResponse::from_head_and_body(head.head.clone(), bytes)
            .into_lossy_materialized_text_response(),
    )
}

#[derive(Clone, Debug)]
pub(crate) struct ResourceBodyResponse {
    pub(crate) head: ResponseHead,
    pub(crate) body: SubresourceResponseBody,
}

impl From<Response> for ResourceBodyResponse {
    fn from(response: Response) -> Self {
        Self {
            head: response.head(),
            body: SubresourceResponseBody::from_fetch_response(&response),
        }
    }
}

impl ResourceBodyResponse {
    pub(crate) fn head(&self) -> ResponseHead {
        self.head.clone()
    }

    pub(crate) fn publish(
        &self,
        network: &ResourceTransfer,
        network_request_headers: Option<Vec<(String, String)>>,
    ) {
        network.body_completed(
            ResourceResponseHead {
                status_text: None,
                head: self.head.clone(),
                network_request_headers,
            },
            self.body.clone(),
        );
    }

    pub(crate) fn failure(
        &self,
        message: String,
        network_request_headers: Option<Vec<(String, String)>>,
    ) -> ResourceResponseFailure {
        ResourceResponseFailure::PartialBody {
            message,
            response: Arc::new(ResourceResponseHead {
                status_text: None,
                head: self.head.clone(),
                network_request_headers,
            }),
            body: self.body.clone(),
        }
    }

    pub(crate) fn body_source(&self) -> Result<moli_fetch::ResponseBody, String> {
        self.body
            .materialize_bytes()
            .map(moli_fetch::ResponseBody::materialized_bytes)
            .map_err(|error| format!("failed to materialize worker XHR body: {error}"))
    }
}

impl From<moli_page_types::NavigationResponse> for ResourceBodyResponse {
    fn from(response: moli_page_types::NavigationResponse) -> Self {
        Self {
            head: response.head(),
            body: SubresourceResponseBody::from_navigation_response(&response),
        }
    }
}

impl ResourceBodyResponse {
    pub(crate) fn into_navigation_response(
        self,
    ) -> Result<moli_page_types::NavigationResponse, String> {
        let body = self
            .body
            .materialize_bytes()
            .map_err(|error| format!("failed to materialize resource body: {error}"))?;
        Ok(
            moli_page_types::NavigationResponse::from_head_and_materialized_body(
                self.head,
                moli_fetch::ResponseBody::materialized_bytes(body),
            ),
        )
    }
}

/// The physical producer and its consumer share received bytes. A cancellation
/// can retain the exact prefix even before queued JS callbacks have run.
/// Response decisions and the first wire request headers belong to this same resource.
pub(crate) struct ResourceResponseStream {
    pub(crate) network: Arc<ResourceTransfer>,
    response: parking_lot::Mutex<ResourceResponseState>,
    storage: ResourceBodyStorage,
    window_fetch_policy: Option<Box<crate::network_host::WindowFetchResponsePolicy>>,
}

enum ResourceBodyStorage {
    Bytes(Option<moli_disk_pool::DiskPool>),
    Image(moli_parkable_image::ParkableImageManager),
}

impl ResourceBodyStorage {
    fn writer(&self) -> moli_page_types::SubresourceResponseBodyWriter {
        match self {
            Self::Bytes(pool) => {
                moli_page_types::SubresourceResponseBodyWriter::with_disk_pool(pool.clone())
            }
            Self::Image(manager) => {
                moli_page_types::SubresourceResponseBodyWriter::for_image(manager.clone())
            }
        }
    }
}

#[derive(Default)]
struct ResourceResponseState {
    body: ResourceStreamBody,
    intercept_response: bool,
    handle_auth_requests: bool,
    network_request_headers: Option<Vec<(String, String)>>,
}

impl ResourceResponseState {
    fn intercepts(&self, head: &ResponseHead) -> bool {
        self.intercept_response
            || (self.handle_auth_requests
                && matches!(head.status, 401 | 407)
                && crate::network_host::extract_subresource_auth_challenge(&head.headers).is_some())
    }

    fn record_request_headers(
        &mut self,
        headers: Option<Vec<(String, String)>>,
    ) -> Option<Vec<(String, String)>> {
        // Authentication retries keep the original browser request's wire headers.
        if self.network_request_headers.is_none() {
            self.network_request_headers = headers;
        }
        self.network_request_headers.clone()
    }
}

#[derive(Default)]
enum ResourceStreamBody {
    #[default]
    Pending,
    Reading(
        Arc<ResourceResponseHead>,
        moli_page_types::SubresourceResponseBodyWriter,
    ),
    Paused(
        Arc<ResourceResponseHead>,
        moli_page_types::SubresourceResponseBodyWriter,
    ),
    Received(Arc<ResourceResponseHead>, SubresourceResponseBody),
    PausedComplete(Arc<ResourceResponseHead>, SubresourceResponseBody),
}

impl ResourceResponseStream {
    pub(crate) async fn collect(
        &self,
        observed: moli_fetch::NetworkFetchResult<moli_fetch::StreamingRawResponse>,
    ) -> Result<ResourceBodyResponse, ResourceResponseFailure> {
        let (mut response, request_observation) = observed.into_parts();
        self.response_started(ResourceResponseHead {
            head: response.head(),
            status_text: None,
            network_request_headers: request_observation.map(|request| request.into_headers()),
        });
        while let Some(bytes) = response.next_chunk().await {
            self.data_received(&bytes);
            tokio::task::yield_now().await;
        }
        response
            .finish()
            .await
            .map_err(|error| self.failure(format!("{error:#}")))?;
        Ok(self
            .finish_response()
            .expect("collected response owns its head"))
    }

    pub(crate) fn for_load(
        network: Arc<ResourceTransfer>,
        load: &super::loads::ResourceLoadLease,
        resource_type: moli_page_types::SubresourceResourceType,
    ) -> Arc<Self> {
        let loader = load.request_client();
        let storage = if resource_type == moli_page_types::SubresourceResourceType::Image {
            ResourceBodyStorage::Image(loader.parkable_image_manager(&load.task_runner()))
        } else {
            ResourceBodyStorage::Bytes(loader.disk_pool())
        };
        Self::with_storage(network, storage)
    }

    #[cfg(test)]
    pub(crate) fn new(network: Arc<ResourceTransfer>) -> Arc<Self> {
        Self::with_storage(network, ResourceBodyStorage::Bytes(None))
    }

    pub(crate) fn with_disk_pool(
        network: Arc<ResourceTransfer>,
        pool: Option<moli_disk_pool::DiskPool>,
    ) -> Arc<Self> {
        Self::with_storage(network, ResourceBodyStorage::Bytes(pool))
    }

    fn with_storage(network: Arc<ResourceTransfer>, storage: ResourceBodyStorage) -> Arc<Self> {
        Arc::new(Self {
            network,
            response: Default::default(),
            storage,
            window_fetch_policy: None,
        })
    }

    pub(crate) fn for_window_fetch(
        network: Arc<ResourceTransfer>,
        load: &super::loads::ResourceLoadLease,
        connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
        report_context: crate::network_host::WindowCspReportRequestContext,
    ) -> Arc<Self> {
        Arc::new(Self {
            network,
            response: Default::default(),
            storage: ResourceBodyStorage::Bytes(load.request_client().disk_pool()),
            window_fetch_policy: Some(Box::new(
                crate::network_host::WindowFetchResponsePolicy::new(connect_policy, report_context),
            )),
        })
    }

    pub(crate) fn window_fetch_policy(
        &self,
    ) -> Option<&crate::network_host::WindowFetchResponsePolicy> {
        self.window_fetch_policy.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn unobserved_for_test() -> Arc<Self> {
        let url = url::Url::parse("https://resource.test/").unwrap();
        let (network, _) = ResourceTransfer::start(
            crate::runtime::RendererNetworkRequest::unobserved_for_test(),
            |_| {},
            |request| {
                moli_page_types::SubresourceRequestStarted::new(
                    request.handle(),
                    None,
                    url.clone(),
                    url,
                    "GET".into(),
                    moli_fetch::RequestHeaders::default(),
                    None,
                    moli_page_types::SubresourceResourceType::Fetch,
                    moli_page_types::SubresourceRequestInitiatorType::Other,
                    None,
                )
            },
        );
        Self::new(network)
    }

    pub(crate) fn configure_interception(
        &self,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) {
        let mut state = self.response.lock();
        state.intercept_response = intercept_response;
        state.handle_auth_requests = handle_auth_requests;
    }

    pub(crate) fn intercept_response(&self) -> bool {
        self.response.lock().intercept_response
    }

    pub(crate) fn handle_auth_requests(&self) -> bool {
        self.response.lock().handle_auth_requests
    }

    pub(crate) fn intercepts_response(&self, head: &ResponseHead) -> bool {
        self.response.lock().intercepts(head)
    }

    pub(crate) fn record_request_headers(
        &self,
        headers: Option<Vec<(String, String)>>,
    ) -> Option<Vec<(String, String)>> {
        self.response.lock().record_request_headers(headers)
    }

    pub(crate) fn response_started(&self, mut response: ResourceResponseHead) {
        let mut state = self.response.lock();
        response.network_request_headers =
            state.record_request_headers(response.network_request_headers);
        let response = Arc::new(response);
        if state.intercepts(&response.head) {
            state.body = ResourceStreamBody::Paused(response, self.storage.writer());
        } else {
            state.body = ResourceStreamBody::Reading(response.clone(), self.storage.writer());
            drop(state);
            self.network.response_started(response);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_body_writer_for_test(
        &self,
        writer: moli_page_types::SubresourceResponseBodyWriter,
    ) {
        let mut response = self.response.lock();
        let (ResourceStreamBody::Reading(_, body) | ResourceStreamBody::Paused(_, body)) =
            &mut response.body
        else {
            panic!("test body requires an admitted response head")
        };
        *body = writer;
    }

    pub(crate) fn data_received(&self, bytes: &[u8]) {
        let mut response = self.response.lock();
        match &mut response.body {
            ResourceStreamBody::Reading(_, body) => {
                body.append(bytes);
                self.network.data_received(bytes.len());
            }
            ResourceStreamBody::Paused(_, body) => body.append(bytes),
            ResourceStreamBody::Pending
            | ResourceStreamBody::Received(..)
            | ResourceStreamBody::PausedComplete(..) => {}
        }
    }

    pub(crate) fn head(&self) -> Arc<ResourceResponseHead> {
        match &self.response.lock().body {
            ResourceStreamBody::Reading(head, _)
            | ResourceStreamBody::Paused(head, _)
            | ResourceStreamBody::Received(head, _)
            | ResourceStreamBody::PausedComplete(head, _) => head.clone(),
            ResourceStreamBody::Pending => panic!("response head has not arrived"),
        }
    }

    pub(crate) fn pause_completed_response(
        &self,
        response: ResourceBodyResponse,
        status_text: Option<String>,
    ) {
        let mut state = self.response.lock();
        let head = Arc::new(ResourceResponseHead {
            head: response.head,
            status_text,
            network_request_headers: state.network_request_headers.clone(),
        });
        state.body = ResourceStreamBody::PausedComplete(head, response.body);
    }

    pub(crate) fn read_received(&self, offset: usize, size: usize) -> Result<Vec<u8>, String> {
        match &mut self.response.lock().body {
            ResourceStreamBody::Reading(_, body) | ResourceStreamBody::Paused(_, body) => {
                body.read_range(offset, size)
            }
            ResourceStreamBody::Received(_, body) | ResourceStreamBody::PausedComplete(_, body) => {
                body.read_chunk(offset, size)
            }
            ResourceStreamBody::Pending => return Ok(Vec::new()),
        }
        .map_err(|error| format!("failed to read response body: {error}"))
    }

    /// Accept the held head without replacing its original body writer. Bytes
    /// consumed through a debugger body command remain available to script.
    pub(crate) fn accept_response(
        &self,
        status: Option<u16>,
        headers: Option<Vec<(String, Vec<u8>)>>,
    ) {
        let mut state = self.response.lock();
        state.intercept_response = false;
        state.handle_auth_requests = false;
        let body = std::mem::take(&mut state.body);
        state.body = match body {
            ResourceStreamBody::Paused(mut head, body) => {
                update_response_head(&mut head, status, headers);
                self.network.response_started(head.clone());
                if !body.is_empty() {
                    self.network.data_received(body.len());
                }
                ResourceStreamBody::Reading(head, body)
            }
            ResourceStreamBody::PausedComplete(mut head, body) => {
                update_response_head(&mut head, status, headers);
                self.network.response_started(head.clone());
                if !body.is_empty() {
                    self.network.data_received(body.len());
                }
                ResourceStreamBody::Received(head, body)
            }
            body => body,
        };
    }

    pub(crate) fn finish_response(&self) -> Option<ResourceBodyResponse> {
        let mut state = self.response.lock();
        if matches!(
            state.body,
            ResourceStreamBody::Reading(..) | ResourceStreamBody::Paused(..)
        ) {
            state.body = match std::mem::take(&mut state.body) {
                ResourceStreamBody::Reading(head, body) => {
                    ResourceStreamBody::Received(head, body.finish())
                }
                ResourceStreamBody::Paused(head, body) => {
                    ResourceStreamBody::PausedComplete(head, body.finish())
                }
                _ => unreachable!(),
            };
        }
        match &state.body {
            ResourceStreamBody::Received(head, body)
            | ResourceStreamBody::PausedComplete(head, body) => Some(ResourceBodyResponse {
                head: head.head.clone(),
                body: body.clone(),
            }),
            ResourceStreamBody::Pending => None,
            ResourceStreamBody::Reading(..) | ResourceStreamBody::Paused(..) => unreachable!(),
        }
    }

    pub(crate) fn failure(&self, message: String) -> ResourceResponseFailure {
        self.finish_response();
        match &self.response.lock().body {
            ResourceStreamBody::Received(response, body)
            | ResourceStreamBody::PausedComplete(response, body) => {
                ResourceResponseFailure::PartialBody {
                    message,
                    response: response.clone(),
                    body: body.clone(),
                }
            }
            ResourceStreamBody::Pending => ResourceResponseFailure::Request(message),
            ResourceStreamBody::Reading(..) | ResourceStreamBody::Paused(..) => unreachable!(),
        }
    }
}

fn update_response_head(
    head: &mut Arc<ResourceResponseHead>,
    status: Option<u16>,
    headers: Option<Vec<(String, Vec<u8>)>>,
) {
    let head = Arc::make_mut(head);
    if let Some(status) = status {
        head.head.status = status;
    }
    if let Some(headers) = headers {
        head.head.headers = headers;
    }
}

impl Drop for ResourceResponseStream {
    fn drop(&mut self) {
        if !matches!(self.response.get_mut().body, ResourceStreamBody::Pending) {
            self.network
                .failed(&self.failure("Resource load cancelled".into()));
        }
    }
}

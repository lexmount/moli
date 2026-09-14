use super::*;
use crate::network::{
    ResourceResponseFailure, ResourceResponseHead, ResourceResponseStream, ResourceTransfer,
};
use crate::page_task_queue::RendererResourceCompletionSender;
use crate::types::{AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, NetworkFetchResult, RedirectInfo,
    RequestCredentialsMode, RequestMode, RequestRedirectMode, ResponseHead, StreamingRawResponse,
    is_cors_safelisted_method,
};
use std::sync::Arc;

/// An async result belongs to the original pending request until it is claimed.
/// If its Page route retires before delivery, the result itself settles the
/// native request rather than losing the physical response with the VM.
pub(crate) struct CompletedResourceFetch {
    response: Arc<ResourceResponseStream>,
    completion: Option<AsyncSubresourceFetchCompletion>,
}

impl std::fmt::Debug for CompletedResourceFetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletedResourceFetch")
            .field("completion", &self.completion)
            .finish_non_exhaustive()
    }
}

impl CompletedResourceFetch {
    pub(crate) fn new(
        response: Arc<ResourceResponseStream>,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Self {
        Self {
            response,
            completion: Some(completion),
        }
    }

    pub(crate) fn internal_id(&self) -> u64 {
        self.completion
            .as_ref()
            .expect("unclaimed resource result")
            .internal_id
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(mut self) -> AsyncSubresourceFetchCompletion {
        let completion = self.completion.take().expect("single result consumer");
        completion.publish_with(&self.response.network, |item| {
            self.response.network.observe(item)
        });
        completion
    }

    pub(crate) fn claim(
        mut self,
        network: &Arc<ResourceTransfer>,
    ) -> Option<AsyncSubresourceFetchCompletion> {
        if !Arc::ptr_eq(network, &self.response.network) {
            return None;
        }
        self.completion.take()
    }
}

impl Drop for CompletedResourceFetch {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            self.response.network.complete_with(
                |request| {
                    let result = completion.network_result();
                    let head = match &result {
                        Ok((head, _)) => Some(head),
                        Err(ResourceResponseFailure::PartialBody { response, .. }) => {
                            Some(response.as_ref())
                        }
                        Err(ResourceResponseFailure::Request(_)) => None,
                    };
                    let failure = self
                        .response
                        .window_fetch_policy()
                        .zip(head)
                        .filter(|(_, head)| !head.head.redirect_chain.is_empty())
                        .and_then(|(policy, head)| {
                            policy.check_unclaimed_response(request, &head.head.final_url)
                        });
                    match (result, failure) {
                        (Ok((head, body)), Some(message)) => {
                            Err(ResourceResponseFailure::PartialBody {
                                message,
                                response: Arc::new(head),
                                body,
                            })
                        }
                        (Err(error), Some(message)) => Err(error.with_message(message)),
                        (result, None) => result,
                    }
                },
                |observation| self.response.network.observe(observation),
            );
        }
    }
}

pub(crate) fn send_resource_completion(
    sender: &RendererResourceCompletionSender,
    response: Arc<ResourceResponseStream>,
    completion: AsyncSubresourceFetchCompletion,
) {
    let _ = sender.send_async_subresource_event(AsyncSubresourceFetchEvent::TransportCompletion(
        Box::new(CompletedResourceFetch::new(response, completion)),
    ));
}

pub(crate) fn resource_request_started(
    network: &crate::runtime::RendererNetworkRequest,
    info: &crate::types::PendingSubresourceFetchInfo,
    initiator: moli_page_types::SubresourceRequestInitiatorType,
    keepalive: bool,
) -> moli_page_types::SubresourceRequestStarted {
    moli_page_types::SubresourceRequestStarted::new(
        network.handle(),
        info.frame_id.clone(),
        info.document_url.clone(),
        info.url.clone(),
        info.method.clone(),
        info.request_headers.clone(),
        info.request_body.clone(),
        info.resource_type,
        initiator,
        info.request_cookie_report.clone(),
    )
    .with_request_body_bytes(info.request_body_bytes.clone())
    .with_keepalive(keepalive)
}

const MAX_MANUAL_CORS_REDIRECTS: usize = 20;

#[cfg(test)]
pub(crate) async fn fetch_browser_subresource_with_preflight(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<Response, String> {
    let preflight_request_headers = request.request_headers.to_byte_strings();
    fetch_browser_subresource_with_preflight_headers_and_observer(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        None,
    )
    .await
    .map(NetworkFetchResult::into_response)
}

#[cfg(test)]
pub(crate) async fn fetch_browser_subresource_with_preflight_headers(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
) -> Result<Response, String> {
    fetch_browser_subresource_with_preflight_headers_and_observer(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        None,
    )
    .await
    .map(NetworkFetchResult::into_response)
}

pub(crate) async fn fetch_browser_subresource_with_preflight_headers_and_observer(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<NetworkFetchResult<Response>, String> {
    if browser_request_needs_manual_preflight_redirects(&request, &preflight_request_headers) {
        return fetch_browser_subresource_with_manual_preflight_redirects(
            loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await;
    }
    run_cors_preflight_if_needed(
        &loader,
        &request,
        cancel_handle.clone(),
        &preflight_request_headers,
        preflight_observer,
    )
    .await?;
    fetch_once_with_network_metadata(&loader, request, cancel_handle).await
}

fn browser_request_needs_manual_preflight_redirects(
    request: &Request,
    preflight_request_headers: &[(String, String)],
) -> bool {
    matches!(
        request.browser_request_metadata(),
        Some(
            BrowserRequestMetadata::Fetch
                | BrowserRequestMetadata::EventSource
                | BrowserRequestMetadata::JsonModule
                | BrowserRequestMetadata::Manifest
                | BrowserRequestMetadata::StyleModule
                | BrowserRequestMetadata::Xhr,
        )
    ) && request.request_mode == RequestMode::Cors
        && request.cookie_context.initiator_url.is_some()
        && (!is_cors_safelisted_method(&request.method)
            || !moli_fetch::cors_unsafe_request_header_names(preflight_request_headers).is_empty())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ManualCorsRedirectTransition {
    FinalResponse,
    ManualResponse,
    FollowedRedirect,
}

struct ManualCorsRedirectState {
    request: Request,
    preflight_request_headers: Vec<(String, String)>,
}

impl ManualCorsRedirectState {
    fn new(request: Request, preflight_request_headers: Vec<(String, String)>) -> Self {
        Self {
            request,
            preflight_request_headers,
        }
    }

    fn request(&self) -> &Request {
        &self.request
    }

    fn preflight_request_headers(&self) -> &[(String, String)] {
        &self.preflight_request_headers
    }

    fn hop_request(&self) -> Request {
        self.request.clone().with_follow_redirects(false)
    }

    async fn run_current_hop_preflight(
        &self,
        loader: &ResourceRequestClient,
        cancel_handle: Option<FetchCancelHandle>,
        preflight_observer: Option<&CorsPreflightNetworkObserver>,
    ) -> Result<(), String> {
        run_cors_preflight_if_needed(
            loader,
            self.request(),
            cancel_handle,
            self.preflight_request_headers(),
            preflight_observer,
        )
        .await
    }

    fn advance(
        &mut self,
        head: ResponseHead,
        network_extra_info_available: bool,
    ) -> Result<ManualCorsRedirectTransition, String> {
        // Keep redirect-mode error precedence aligned with Fetch: an error-mode
        // redirect is rejected before the redirect response is CORS-checked.
        if self.request.redirect_mode == RequestRedirectMode::Error {
            if next_redirect_url(&head.final_url, head.status, &head.headers, 0)?.is_some() {
                return Err(redirect_mode_error_message(&head.final_url));
            }
            validate_actual_cors_response_head(&self.request, &head)?;
            return Ok(ManualCorsRedirectTransition::FinalResponse);
        }

        validate_actual_cors_response_head(&self.request, &head)?;
        let Some(next_url) = next_redirect_url(
            &head.final_url,
            head.status,
            &head.headers,
            self.request.redirect_count(),
        )?
        else {
            return Ok(ManualCorsRedirectTransition::FinalResponse);
        };

        if self.request.redirect_mode == RequestRedirectMode::Manual {
            return Ok(ManualCorsRedirectTransition::ManualResponse);
        }

        match self.request.redirect_mode {
            RequestRedirectMode::Follow => {}
            RequestRedirectMode::Error | RequestRedirectMode::Manual => {
                unreachable!("redirect modes were handled before the follow transition")
            }
        }
        let redirect_status = head.status;
        self.request.record_redirect(RedirectInfo {
            source: moli_fetch::RedirectSource::Network,
            from_url: head.final_url,
            to_url: next_url.clone(),
            status: redirect_status,
            headers: head.headers,
            network_extra_info_available,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: network_extra_info_available,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
        });
        self.request.apply_redirect_status(redirect_status);
        self.request.url = next_url;
        self.preflight_request_headers = self.request.request_headers.to_byte_strings();
        Ok(ManualCorsRedirectTransition::FollowedRedirect)
    }

    fn into_redirect_chain(self) -> Vec<RedirectInfo> {
        self.request.redirect_chain().to_vec()
    }
}

async fn fetch_browser_subresource_with_manual_preflight_redirects(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<NetworkFetchResult<Response>, String> {
    let mut redirects = ManualCorsRedirectState::new(request, preflight_request_headers);

    loop {
        redirects
            .run_current_hop_preflight(&loader, cancel_handle.clone(), preflight_observer)
            .await?;

        let mut observed = fetch_once_with_network_metadata_unvalidated(
            &loader,
            redirects.hop_request(),
            cancel_handle.clone(),
        )
        .await?;
        let network_extra_info_available = observed.request_observation().is_some();
        match redirects.advance(observed.response().head(), network_extra_info_available)? {
            ManualCorsRedirectTransition::FinalResponse => {
                let redirect_chain = redirects.into_redirect_chain();
                observed.response_mut().redirected = !redirect_chain.is_empty();
                observed.response_mut().redirect_chain = redirect_chain;
                return Ok(observed);
            }
            ManualCorsRedirectTransition::ManualResponse => return Ok(observed),
            ManualCorsRedirectTransition::FollowedRedirect => {}
        }
    }
}

async fn fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<NetworkFetchResult<StreamingRawResponse>, ResourceResponseFailure> {
    let mut redirects = ManualCorsRedirectState::new(request, preflight_request_headers);

    loop {
        redirects
            .run_current_hop_preflight(loader, cancel_handle.clone(), preflight_observer)
            .await?;

        let mut observed = loader
            .fetch_raw_stream_with_cancel_and_network_metadata(
                redirects.hop_request(),
                cancel_handle.clone().unwrap_or_default(),
            )
            .await
            .map_err(format_network_error)?;
        let network_extra_info_available = observed.request_observation().is_some();
        let head = observed.response().head();
        match redirects.advance(head, network_extra_info_available) {
            Ok(ManualCorsRedirectTransition::FinalResponse) => {
                let redirect_chain = redirects.into_redirect_chain();
                observed.response_mut().redirected = !redirect_chain.is_empty();
                observed.response_mut().redirect_chain = redirect_chain;
                return Ok(observed);
            }
            Ok(ManualCorsRedirectTransition::ManualResponse) => return Ok(observed),
            Ok(ManualCorsRedirectTransition::FollowedRedirect) => {}
            Err(message) => {
                let redirect_chain = redirects.into_redirect_chain();
                observed.response_mut().redirected = !redirect_chain.is_empty();
                observed.response_mut().redirect_chain = redirect_chain;
                return Err(rejected_resource_response(observed, message));
            }
        }

        // Redirect bodies are not exposed to Fetch/XHR. Finish this hop before
        // reusing the logical request's cancel handle for the redirected hop;
        // the final non-redirect response remains headers-first and streaming.
        observed
            .response_mut()
            .finish()
            .await
            .map_err(format_network_error)?;
    }
}

fn validate_actual_cors_response_head(
    request: &Request,
    response: &ResponseHead,
) -> Result<(), String> {
    request.validate_cors_response_for_url(&response.final_url, &response.headers)
}

fn next_redirect_url(
    final_url: &url::Url,
    status: u16,
    headers: &[(String, Vec<u8>)],
    redirect_count: usize,
) -> Result<Option<url::Url>, String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| moli_fetch::decode_header_value(value))
    else {
        return Ok(None);
    };
    if redirect_count >= MAX_MANUAL_CORS_REDIRECTS {
        return Err(format!("redirect limit exceeded for {final_url}"));
    }
    final_url
        .join(&location)
        .or_else(|_| url::Url::parse(&location))
        .map(Some)
        .map_err(|error| {
            format!("failed to resolve redirect location `{location}` from {final_url}: {error}")
        })
}

pub(crate) async fn fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<NetworkFetchResult<StreamingRawResponse>, ResourceResponseFailure> {
    // Borrow the loader so its fetch runtime stays alive until the caller drains
    // and finishes the returned StreamingRawResponse.
    if browser_request_needs_manual_preflight_redirects(&request, &preflight_request_headers) {
        return fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
            loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await;
    }
    run_cors_preflight_if_needed(
        loader,
        &request,
        cancel_handle.clone(),
        &preflight_request_headers,
        preflight_observer,
    )
    .await?;
    let cancel_handle = cancel_handle.unwrap_or_default();
    let redirect_mode = request.redirect_mode;
    let result = loader
        .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel_handle)
        .await
        .map_err(format_network_error)?;
    if let Err(message) =
        validate_redirect_mode_response_head(&result.response().head(), redirect_mode)
    {
        return Err(rejected_resource_response(result, message));
    }
    Ok(result)
}

/// A policy rejection happens at the received head. Preserve those facts and
/// drop the reader immediately; waiting for a rejected body would delay failure.
fn rejected_resource_response(
    observed: NetworkFetchResult<StreamingRawResponse>,
    message: String,
) -> ResourceResponseFailure {
    let (mut response, request) = observed.into_parts();
    response.cancellation_handle().cancel();
    let mut body = crate::types::SubresourceResponseBodyWriter::default();
    while let Some(chunk) = response.try_next_chunk() {
        body.append(&chunk);
    }
    ResourceResponseFailure::PartialBody {
        message,
        response: Arc::new(ResourceResponseHead {
            status_text: None,
            head: response.head(),
            network_request_headers: request.map(|request| request.into_headers()),
        }),
        body: body.finish(),
    }
}

fn format_network_error(error: anyhow::Error) -> String {
    format!("{error:#}")
}

async fn run_cors_preflight_if_needed(
    loader: &ResourceRequestClient,
    request: &Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: &[(String, String)],
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<(), String> {
    let request_origin = request
        .browser_origin()
        .map_err(|error| error.to_string())?;
    if request.request_mode == RequestMode::Cors
        && let Some(preflight_headers) = cors_preflight_request_headers(
            request.has_cross_origin_url(&request.url),
            &request.url,
            &request.method,
            preflight_request_headers,
        )
    {
        let mut preflight_request = Request::new_browser_bytes(
            "OPTIONS",
            request.url.as_str(),
            None,
            preflight_headers,
            request_origin.clone(),
        )
        .map_err(|error| format!("cors preflight: failed to build request: {error}"))?
        // Preflight performs one HTTP exchange; a redirect is a
        // non-ok response, never another OPTIONS request.
        .with_redirect_mode(RequestRedirectMode::Manual)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_redirect_chain(request.redirect_chain().to_vec())
        .with_network_partition_key(request.network_partition_key().map(str::to_owned));
        if let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() {
            preflight_request = preflight_request.with_initiator_url(initiator_url);
        }
        if let Some(metadata) = request.browser_request_metadata() {
            preflight_request = preflight_request.with_browser_request_metadata(metadata);
        } else {
            preflight_request =
                preflight_request.with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        }

        let preflight_response = match preflight_observer {
            Some(observer) => {
                observer
                    .fetch(loader, preflight_request, cancel_handle)
                    .await?
            }
            None => fetch_response_head_once(loader, preflight_request, cancel_handle).await?,
        };
        validate_cors_preflight_response(
            &request.serialized_origin(),
            request.credentials_mode,
            &request.method,
            preflight_request_headers,
            preflight_response.status,
            &preflight_response.headers,
        )?;
    }
    Ok(())
}

pub(crate) fn spawn_async_subresource_fetch(
    task_runner: crate::network::RendererResourceTaskRunner,
    completion_tx: RendererResourceCompletionSender,
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    internal_id: u64,
    resource: Arc<ResourceResponseStream>,
    preflight_observer: CorsPreflightNetworkObserver,
    request_url: url::Url,
) {
    task_runner.spawn(async move {
        // JS can expose selected responses as streams; every physical response
        // publishes stages and retains its body independently of that consumer.
        let stream_to_js = matches!(
            request.browser_request_metadata(),
            Some(
                BrowserRequestMetadata::Fetch
                    | BrowserRequestMetadata::EventSource
                    | BrowserRequestMetadata::JsonModule
                    | BrowserRequestMetadata::Manifest
                    | BrowserRequestMetadata::StyleModule
                    | BrowserRequestMetadata::Xhr
            )
        ) && request.follow_redirects
            && request.request_mode != RequestMode::NoCors;
        let observed = fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
            &loader,
            request,
            cancel_handle,
            preflight_request_headers,
            Some(&preflight_observer),
        )
        .await;
        let mut body_source_id = None;
        let mut network_request_headers = None;
        let result = match observed {
            Err(error) => Err(error),
            Ok(observed) => {
                let (mut response, request_observation) = observed.into_parts();
                network_request_headers =
                    request_observation.map(|observation| observation.into_headers());
                let head = response.head();
                resource.response_started(ResourceResponseHead {
                    status_text: None,
                    head: head.clone(),
                    network_request_headers: network_request_headers.clone(),
                });
                if stream_to_js {
                    let id = new_network_body_source_id();
                    body_source_id = Some(id);
                    let _ = completion_tx.send_async_subresource_event(
                        AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
                            AsyncSubresourceStreamingStarted {
                                skip_fetch_security_validation: false,
                                response_filter: None,
                                internal_id,
                                request_url: request_url.clone(),
                                body_source_id: id,
                                head: head.clone(),
                            },
                        )),
                    );
                }
                while let Some(bytes) = response.next_chunk().await {
                    resource.data_received(&bytes);
                    if let Some(body_source_id) = body_source_id {
                        let _ = completion_tx.send_async_subresource_event(
                            AsyncSubresourceFetchEvent::StreamingChunk(
                                AsyncSubresourceStreamingChunk {
                                    body_source_id,
                                    bytes,
                                },
                            ),
                        );
                    }
                }
                match response.finish().await {
                    Ok(()) => Ok(resource
                        .finish_response()
                        .expect("physical response head precedes completion")),
                    Err(error) => Err(resource.failure(format_network_error(error))),
                }
            }
        };
        let completion = AsyncSubresourceFetchCompletion {
            internal_id,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            network_request_headers,
            result,
        };
        if let Some(body_source_id) = body_source_id {
            let _ = completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id,
                    completion: Box::new(super::CompletedResourceFetch::new(
                        resource.clone(),
                        completion,
                    )),
                },
            );
        } else {
            super::send_resource_completion(&completion_tx, resource.clone(), completion);
        }
    });
}

pub(crate) async fn collect_image_response_into_parkable(
    observed: NetworkFetchResult<StreamingRawResponse>,
    manager: moli_parkable_image::ParkableImageManager,
) -> Result<
    (
        crate::protocol_types::NavigationResponse,
        moli_parkable_image::ParkableImage,
    ),
    String,
> {
    let (mut response, request_observation) = observed.into_parts();
    let head = response.head();
    let mut encoded = Vec::new();
    while let Some(bytes) = response.next_chunk().await {
        encoded.extend_from_slice(&bytes);
    }
    response.finish().await.map_err(format_network_error)?;
    let encoded = manager.from_frozen_bytes(encoded);
    let response = crate::protocol_types::NavigationResponse::from_head_and_body(
        head,
        String::new(),
        Vec::new(),
    )
    .with_network_request_headers(
        request_observation.map(|observation| observation.into_headers()),
    );
    Ok((response, encoded))
}

async fn fetch_once_with_network_metadata(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<NetworkFetchResult<Response>, String> {
    let redirect_mode = request.redirect_mode;
    let result =
        fetch_once_with_network_metadata_unvalidated(loader, request, cancel_handle).await?;
    let (response, request_observation) = result.into_parts();
    let response = validate_redirect_mode_response(response, redirect_mode)?;
    Ok(NetworkFetchResult::new(response, request_observation))
}

async fn fetch_once_with_network_metadata_unvalidated(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<NetworkFetchResult<Response>, String> {
    let result = match cancel_handle {
        Some(cancel_handle) => loader
            .fetch_text_stream_with_cancel_and_network_metadata(request, cancel_handle)
            .await
            .map_err(format_network_error),
        None => loader
            .fetch_text_stream_with_network_metadata(request)
            .await
            .map_err(format_network_error),
    }?;
    Ok(result)
}

pub(super) async fn fetch_response_head_once(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<ResponseHead, String> {
    let cancel_handle = cancel_handle.unwrap_or_default();
    let mut response = loader
        .fetch_raw_stream_with_cancel(request, cancel_handle)
        .await
        .map_err(format_network_error)?;
    let head = response.head();
    response.finish().await.map_err(format_network_error)?;
    Ok(head)
}

fn validate_redirect_mode_response(
    response: Response,
    redirect_mode: RequestRedirectMode,
) -> Result<Response, String> {
    validate_redirect_mode_parts(
        &response.final_url,
        response.status,
        &response.headers,
        redirect_mode,
    )?;
    Ok(response)
}

fn validate_redirect_mode_response_head(
    head: &ResponseHead,
    redirect_mode: RequestRedirectMode,
) -> Result<(), String> {
    validate_redirect_mode_parts(&head.final_url, head.status, &head.headers, redirect_mode)
}

fn validate_redirect_mode_parts(
    final_url: &url::Url,
    status: u16,
    headers: &[(String, Vec<u8>)],
    redirect_mode: RequestRedirectMode,
) -> Result<(), String> {
    if redirect_mode != RequestRedirectMode::Error {
        return Ok(());
    }
    if next_redirect_url(final_url, status, headers, 0)?.is_some() {
        return Err(redirect_mode_error_message(final_url));
    }
    Ok(())
}

fn redirect_mode_error_message(final_url: &url::Url) -> String {
    format!("redirect mode error blocked redirect from {final_url}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_task_queue::RendererResourceCompletionTestHarness;
    use anyhow::Result;
    use moli_fetch::FetchConfig;
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use url::Url;

    #[test]
    fn rejected_response_retains_queued_bytes_without_waiting_for_completion() {
        let (body, received) = tokio::sync::mpsc::unbounded_channel();
        body.send(b"prefix".to_vec()).unwrap();
        body.send(vec![0, 128, 255]).unwrap();
        let (_complete, completion) = tokio::sync::oneshot::channel();
        let cancel = FetchCancelHandle::new();
        let response = StreamingRawResponse::new(
            Url::parse("https://rejected.test/redirect").unwrap(),
            302,
            vec![("location".into(), "/next".into())],
            None,
            Vec::new(),
            false,
            Vec::new(),
            received,
            cancel.clone(),
            completion,
        );
        let failure = rejected_resource_response(
            NetworkFetchResult::new(response, None),
            "redirect rejected".into(),
        );
        assert!(
            cancel.is_cancelled(),
            "reject the open transport before returning"
        );
        assert!(
            body.send(b"late".to_vec()).is_err(),
            "no late body consumer"
        );
        let ResourceResponseFailure::PartialBody {
            message,
            response,
            body,
        } = failure
        else {
            panic!("rejection retains its physical response")
        };
        assert_eq!(message, "redirect rejected");
        assert_eq!(response.head.status, 302);
        assert_eq!(response.head.headers, [("location".into(), "/next".into())]);
        assert_eq!(body.clone_body_bytes(), b"prefix\0\x80\xff");
    }

    #[test]
    fn cancelled_resource_does_not_run_late_response_policy() {
        let response = ResourceResponseStream::unobserved_for_test();
        response
            .network
            .failed(&ResourceResponseFailure::Request("cancelled".into()));
        response.network.complete_with(
            |_| panic!("a losing response must not admit CSP reports"),
            |_| panic!("cancellation already published the terminal"),
        );
    }

    #[test]
    fn buffered_report_result_survives_a_closed_route_and_a_claim_defers_completion() {
        use moli_page_types::{
            NavigationResponse, ScriptNetworkOutputItem, SubresourceBodyFinishedResult,
        };
        use parking_lot::Mutex;

        for closed_route in [false, true] {
            let source = crate::runtime::RendererWorkerNetworkReporter::unobserved_for_test();
            let records = Arc::new(Mutex::new(Vec::new()));
            let observed = records.clone();
            let url = url::Url::parse("data:text/plain,physical").unwrap();
            let (network, started) = ResourceTransfer::start(
                source.start_request().unwrap(),
                move |receipt| {
                    let crate::runtime::RendererNetworkOutputItem::Resource(item) = receipt.item()
                    else {
                        panic!("resource receipt")
                    };
                    observed.lock().push(item.clone());
                },
                |request| {
                    moli_page_types::SubresourceRequestStarted::new(
                        request.handle(),
                        None,
                        url.clone(),
                        url.clone(),
                        "POST".into(),
                        moli_fetch::RequestHeaders::default(),
                        None,
                        moli_page_types::SubresourceResourceType::CspReport,
                        moli_page_types::SubresourceRequestInitiatorType::Script,
                        None,
                    )
                },
            );
            let crate::runtime::RendererNetworkOutputItem::Resource(started) = started.item()
            else {
                panic!("resource admission")
            };
            records.lock().push(started.clone());
            let response =
                NavigationResponse::from(crate::network_host::local_url_response(&url).unwrap());
            let completion = AsyncSubresourceFetchCompletion {
                internal_id: 1,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
                result: Ok(response.into()),
            };
            if closed_route {
                send_resource_completion(
                    &RendererResourceCompletionSender::closed_for_test(),
                    ResourceResponseStream::new(network.clone()),
                    completion,
                );
            } else {
                let completion = CompletedResourceFetch::new(
                    ResourceResponseStream::new(network.clone()),
                    completion,
                )
                .claim(&network)
                .unwrap();
                assert_eq!(
                    records.lock().len(),
                    1,
                    "the claim transfers the decision to its pending request"
                );
                completion.publish_with(&network, |item| network.observe(item));
            }
            drop(network);
            let records = records.lock();
            assert_eq!(
                records.len(),
                3,
                "one admission, physical head and terminal"
            );
            let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = records[2].as_ref() else {
                panic!("terminal last")
            };
            let SubresourceBodyFinishedResult::Ready(body) = body.result() else {
                panic!("retain the actual completed response")
            };
            assert_eq!(body.clone_body_bytes(), b"physical");
        }
    }

    async fn read_http_request_text(stream: &mut tokio::net::TcpStream) -> Result<String> {
        Ok(String::from_utf8(read_http_request_bytes(stream).await?)?)
    }

    async fn read_http_request_bytes(stream: &mut tokio::net::TcpStream) -> Result<Vec<u8>> {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            let read = stream.read(&mut byte).await?;
            if read == 0 {
                anyhow::bail!("client closed before sending complete request");
            }
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                return Ok(request);
            }
        }
    }

    #[test]
    fn manual_cors_redirect_state_rewrites_request_and_next_preflight_headers() -> Result<()> {
        let request_headers = vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("X-Challenge".to_owned(), "yes".to_owned()),
        ];
        let request = Request::new(
            "POST",
            "https://origin.test/start",
            Some("payload".to_owned()),
            request_headers.clone(),
        )?
        .with_initiator_url(&Url::parse("https://origin.test/page")?)
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
            "https://origin.test/page",
        )?))
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
        let mut redirects = ManualCorsRedirectState::new(request, request_headers);

        let transition = redirects
            .advance(
                ResponseHead {
                    final_url: Url::parse("https://origin.test/start")?,
                    status: 303,
                    headers: vec![("Location".to_owned(), b"https://target.test/final".to_vec())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                true,
            )
            .map_err(anyhow::Error::msg)?;

        assert_eq!(transition, ManualCorsRedirectTransition::FollowedRedirect);
        assert_eq!(
            redirects.request().url.as_str(),
            "https://target.test/final"
        );
        assert_eq!(redirects.request().method, "GET");
        assert!(redirects.request().body.is_none());
        assert_eq!(
            redirects.preflight_request_headers(),
            &[("X-Challenge".to_owned(), "yes".to_owned())]
        );
        assert_eq!(redirects.request().redirect_chain().len(), 1);
        assert_eq!(redirects.request().redirect_chain()[0].status, 303);
        assert!(redirects.request().redirect_chain()[0].network_extra_info_available);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_uses_streaming_head_response_without_a_referrer_url() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut preflight, _) = listener.accept().await.unwrap();
            let preflight_request = read_http_request_text(&mut preflight).await.unwrap();
            assert!(preflight_request.starts_with("OPTIONS /resource HTTP/1.1"));
            assert!(
                preflight_request
                    .to_ascii_lowercase()
                    .contains("\r\norigin: http://origin.test\r\n")
            );
            assert!(
                !preflight_request
                    .to_ascii_lowercase()
                    .contains("\r\nreferer:")
            );
            assert!(
                preflight_request
                    .to_ascii_lowercase()
                    .contains("access-control-request-method: put")
            );
            let preflight_body = "preflight body is not needed by validation";
            let preflight_response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Access-Control-Allow-Origin: http://origin.test\r\n",
                    "Access-Control-Allow-Methods: PUT\r\n",
                    "Access-Control-Allow-Headers: x-test\r\n",
                    "Content-Length: {}\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "{}"
                ),
                preflight_body.len(),
                preflight_body
            );
            preflight
                .write_all(preflight_response.as_bytes())
                .await
                .unwrap();

            let (mut actual, _) = listener.accept().await.unwrap();
            let actual_request = read_http_request_text(&mut actual).await.unwrap();
            assert!(actual_request.starts_with("PUT /resource HTTP/1.1"));
            assert!(
                actual_request
                    .to_ascii_lowercase()
                    .contains("\r\norigin: http://origin.test\r\n")
            );
            let body = "ok";
            let response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Access-Control-Allow-Origin: http://origin.test\r\n",
                    "Content-Length: {}\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            actual.write_all(response.as_bytes()).await.unwrap();
        });

        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request = Request::new(
            "PUT",
            &format!("http://{addr}/resource"),
            None,
            vec![("X-Test".to_owned(), "yes".to_owned())],
        )?
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
            "http://origin.test/page",
        )?))
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);

        let response = fetch_browser_subresource_with_preflight(loader, request, None)
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(response.status, 200);
        assert_eq!(response.body_text(), "ok");

        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unsafe_xhr_redirect_preflights_next_origin_in_buffered_path() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let source_origin = format!("http://127.0.0.1:{}", addr.port());
        let target_url = format!("http://localhost:{}/final", addr.port());
        let target_url_for_server = target_url.clone();
        let source_origin_for_server = source_origin.clone();
        let server = tokio::spawn(async move {
            let (mut redirect, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut redirect).await.unwrap();
            assert!(request.starts_with("POST /start HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("x-challenge: yes"));
            let redirect_response = format!(
                concat!(
                    "HTTP/1.1 307 Temporary Redirect\r\n",
                    "Location: {}\r\n",
                    "Content-Length: 0\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                target_url_for_server
            );
            redirect
                .write_all(redirect_response.as_bytes())
                .await
                .unwrap();
            drop(redirect);

            let (mut preflight, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut preflight).await.unwrap();
            let request_lower = request.to_ascii_lowercase();
            assert!(request.starts_with("OPTIONS /final HTTP/1.1"));
            assert!(request_lower.contains("access-control-request-method: post"));
            assert!(request_lower.contains("access-control-request-headers: x-challenge"));
            let preflight_response = format!(
                concat!(
                    "HTTP/1.1 204 No Content\r\n",
                    "Access-Control-Allow-Origin: {}\r\n",
                    "Access-Control-Allow-Methods: POST\r\n",
                    "Access-Control-Allow-Headers: x-challenge\r\n",
                    "Content-Length: 0\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                source_origin_for_server
            );
            preflight
                .write_all(preflight_response.as_bytes())
                .await
                .unwrap();
            drop(preflight);

            let (mut final_response, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut final_response).await.unwrap();
            assert!(request.starts_with("POST /final HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("x-challenge: yes"));
            let response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Access-Control-Allow-Origin: {}\r\n",
                    "Content-Type: text/plain\r\n",
                    "Content-Length: 12\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "buffered-xhr"
                ),
                source_origin_for_server
            );
            final_response.write_all(response.as_bytes()).await.unwrap();
        });

        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("{source_origin}/start"))?;
        let document_url = Url::parse(&format!("{source_origin}/page"))?;
        let request_headers = vec![("X-Challenge".to_owned(), "yes".to_owned())];
        let request = Request::new(
            "POST",
            request_url.as_str(),
            Some("payload".to_owned()),
            request_headers.clone(),
        )?
        .with_initiator_url(&document_url)
        .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        let response = fetch_browser_subresource_with_preflight_headers(
            loader,
            request,
            Some(FetchCancelHandle::new()),
            request_headers,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert_eq!(response.final_url.as_str(), target_url);
        assert_eq!(response.body_text(), "buffered-xhr");
        assert!(response.redirected);
        assert_eq!(response.redirect_chain.len(), 1);
        assert_eq!(response.redirect_chain[0].from_url, request_url);
        assert_eq!(response.redirect_chain[0].to_url.as_str(), target_url);
        assert_eq!(response.redirect_chain[0].status, 307);

        server.await?;
        Ok(())
    }

    async fn expect_native_preflight(
        queue: &mut RendererResourceCompletionTestHarness,
        document_url: &Url,
        request_url: &Url,
        frame_id: Option<&str>,
        status: u16,
        resource_type: SubresourceResourceType,
    ) -> Result<()> {
        use crate::runtime::RendererNetworkOutputItem;
        use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};
        let mut preflight_handle = None;
        for stage in 0..3 {
            let AsyncSubresourceFetchEvent::NativeNetwork(event) =
                next_async_subresource_event(queue).await?
            else {
                anyhow::bail!("preflight must publish native request, response and terminal first");
            };
            let RendererNetworkOutputItem::Resource(item) = event.item() else {
                anyhow::bail!("expected resource stage");
            };
            match (stage, item.as_ref()) {
                (0, ScriptNetworkOutputItem::SubresourceRequestStarted(request)) => {
                    assert_eq!(request.frame_id(), frame_id);
                    assert_eq!(request.document_url(), document_url);
                    assert_eq!(request.url(), request_url);
                    assert_eq!(request.method(), "OPTIONS");
                    assert_eq!(request.resource_type(), resource_type);
                    preflight_handle = Some(request.handle());
                }
                (1, ScriptNetworkOutputItem::SubresourceResponseStarted(response)) => {
                    assert_eq!(Some(response.handle()), preflight_handle);
                    assert_eq!(response.status(), status);
                }
                (2, ScriptNetworkOutputItem::SubresourceBodyFinished(body)) => {
                    assert_eq!(Some(body.handle()), preflight_handle);
                    assert!(
                        matches!(body.result(), SubresourceBodyFinishedResult::Ready(body) if body.clone_body_bytes().is_empty())
                    );
                }
                _ => anyhow::bail!("unexpected native preflight stage {stage}: {item:?}"),
            }
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_emits_native_stages_before_actual_completion() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut preflight, _) = listener.accept().await.unwrap();
            let preflight_request = read_http_request_text(&mut preflight).await.unwrap();
            assert!(preflight_request.starts_with("OPTIONS /resource HTTP/1.1"));
            assert!(
                preflight_request
                    .to_ascii_lowercase()
                    .contains("access-control-request-method: get")
            );
            assert!(
                preflight_request
                    .to_ascii_lowercase()
                    .contains("access-control-request-headers: content-type")
            );
            preflight
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Access-Control-Allow-Origin: http://origin.test\r\n",
                        "Access-Control-Allow-Headers: content-type\r\n",
                        "Content-Length: 0\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let (mut actual, _) = listener.accept().await.unwrap();
            let actual_request = read_http_request_text(&mut actual).await.unwrap();
            assert!(actual_request.starts_with("GET /resource HTTP/1.1"));
            actual
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Access-Control-Allow-Origin: http://origin.test\r\n",
                        "Content-Type: text/plain\r\n",
                        "Content-Length: 2\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "ok",
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("http://{addr}/resource"))?;
        let document_url = Url::parse("http://origin.test/page")?;
        let request_headers = vec![("Content-Type".to_owned(), "custom/type".to_owned())];
        let request = Request::new("GET", request_url.as_str(), None, request_headers.clone())?
            .with_initiator_url(&document_url)
            .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            request_headers.clone(),
            73,
            ResourceResponseStream::unobserved_for_test(),
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: Some("FRAME-1".to_owned()),
                resource_type: SubresourceResourceType::Fetch,
                keepalive: false,
            },
            request_url.clone(),
        );

        expect_native_preflight(
            &mut queue,
            &document_url,
            &request_url,
            Some("FRAME-1"),
            200,
            SubresourceResourceType::Fetch,
        )
        .await?;

        let body_source_id = match next_async_subresource_event(&mut queue).await? {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 73);
                assert_eq!(started.head.status, 200);
                started.body_source_id
            }
            other => anyhow::bail!("expected actual streaming response second, got {other:?}"),
        };
        let mut body = Vec::new();
        loop {
            match next_async_subresource_event(&mut queue).await? {
                AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                    assert_eq!(chunk.body_source_id, body_source_id);
                    body.extend_from_slice(&chunk.bytes);
                }
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id: finished_body_source_id,
                    completion,
                } => {
                    let finished = completion.complete_for_test();
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert_eq!(finished.internal_id, 73);
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert!(finished.result.is_ok());
                    break;
                }
                other => anyhow::bail!("unexpected actual fetch event: {other:?}"),
            }
        }
        assert_eq!(body, b"ok");

        server.await?;
        Ok(())
    }

    async fn next_async_subresource_event(
        queue: &mut RendererResourceCompletionTestHarness,
    ) -> Result<AsyncSubresourceFetchEvent> {
        for _ in 0..100 {
            if let Some(event) = queue.pop_next_async_subresource_event() {
                return Ok(event);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        anyhow::bail!("timed out waiting for async subresource event")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn image_body_is_received_directly_into_one_parkable_image() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_bytes(&mut stream).await.unwrap();
            assert!(request.starts_with(b"GET /image.png HTTP/1.1"));
            let header = b"\r\nX-Image-Bytes: \xff\xe9\xc3\xa9\r\n";
            assert!(request.windows(header.len()).any(|bytes| bytes == header));
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: image/png\r\n",
                        "Content-Length: 9\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "first"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            tokio::task::yield_now().await;
            stream.write_all(b"tail").await.unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("http://{addr}/image.png"))?;
        let document_url = Url::parse(&format!("http://{addr}/page"))?;
        let request_headers = moli_fetch::RequestHeaders::from_bytes(vec![(
            "X-Image-Bytes".to_owned(),
            vec![0xff, 0xe9, 0xc3, 0xa9],
        )]);
        let request = Request::new("GET", request_url.as_str(), None, request_headers.clone())?
            .with_initiator_url(&document_url)
            .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
            .with_request_mode(RequestMode::NoCors)
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_browser_request_metadata(BrowserRequestMetadata::Image);

        let load = crate::network::loads::resource_load_lease_for_test(loader.clone(), None);
        let resource = ResourceResponseStream::for_load(
            ResourceResponseStream::unobserved_for_test()
                .network
                .clone(),
            &load,
            SubresourceResourceType::Image,
        );
        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            Vec::new(),
            75,
            resource,
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: None,
                resource_type: SubresourceResourceType::Image,
                keepalive: false,
            },
            request_url,
        );

        let AsyncSubresourceFetchEvent::TransportCompletion(completion) =
            next_async_subresource_event(&mut queue).await?
        else {
            anyhow::bail!("image transport must emit one buffered terminal completion");
        };
        let completion = completion.complete_for_test();
        assert_eq!(completion.internal_id, 75);
        let response = completion
            .result
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        assert_eq!(response.head.status, 200);
        let encoded = response
            .body
            .parkable_image()
            .expect("image completion must carry its encoded backing");
        assert_eq!(encoded.snapshot()?.as_ref(), b"firsttail");

        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn service_worker_redirect_chain_prefixes_streaming_network_fallback() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut stream).await.unwrap();
            assert!(request.starts_with("GET /target HTTP/1.1"));
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain\r\n",
                        "Content-Length: 6\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "target",
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let target_url = Url::parse(&format!("http://{addr}/target"))?;
        let source_url = Url::parse(&format!("http://{addr}/synthetic"))?;
        let request = Request::get(target_url.as_str())?
            .with_initiator_url(&Url::parse(&format!("http://{addr}/page"))?)
            .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(&format!(
                "http://{addr}/page"
            ))?))
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let initial_redirect_chain = vec![RedirectInfo {
            source: moli_fetch::RedirectSource::ServiceWorker,
            from_url: source_url.clone(),
            to_url: target_url.clone(),
            status: 302,
            headers: vec![(
                "Location".to_owned(),
                target_url.as_str().as_bytes().to_vec(),
            )],
            network_extra_info_available: false,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }];

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request.with_redirect_chain(initial_redirect_chain),
            Some(FetchCancelHandle::new()),
            Vec::new(),
            74,
            ResourceResponseStream::unobserved_for_test(),
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: None,
                resource_type: SubresourceResourceType::Fetch,
                keepalive: false,
            },
            target_url.clone(),
        );

        let body_source_id = match next_async_subresource_event(&mut queue).await? {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 74);
                assert_eq!(started.head.final_url, target_url);
                assert!(started.head.redirected);
                assert_eq!(started.head.redirect_chain.len(), 1);
                let synthetic_redirect = &started.head.redirect_chain[0];
                assert_eq!(synthetic_redirect.from_url, source_url);
                assert_eq!(synthetic_redirect.to_url, target_url);
                assert!(synthetic_redirect.request_extra_info.is_none());
                assert!(synthetic_redirect.response_extra_info.is_none());
                assert!(!synthetic_redirect.redirect_has_extra_info);
                assert!(synthetic_redirect.negotiated_http_version.is_none());
                started.body_source_id
            }
            other => anyhow::bail!("expected prefixed streaming response, got {other:?}"),
        };
        loop {
            match next_async_subresource_event(&mut queue).await? {
                AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                    assert_eq!(chunk.body_source_id, body_source_id);
                }
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id: finished_body_source_id,
                    completion,
                } => {
                    let finished = completion.complete_for_test();
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert_eq!(finished.internal_id, 74);
                    assert!(finished.result.is_ok());
                    break;
                }
                other => anyhow::bail!("unexpected async subresource event: {other:?}"),
            }
        }

        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn xhr_subresource_uses_streaming_events_until_body_finish() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut stream).await.unwrap();
            assert!(request.starts_with("GET /xhr HTTP/1.1"));
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain\r\n",
                        "Content-Length: 9\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "hello"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            stream.write_all(b"-xhr").await.unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("http://{addr}/xhr"))?;
        let request = Request::get(request_url.as_str())?
            .with_request_origin(moli_url::WebOrigin::from_url(&request_url))
            .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            Vec::new(),
            41,
            ResourceResponseStream::unobserved_for_test(),
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: None,
                resource_type: SubresourceResourceType::Xhr,
                keepalive: false,
            },
            request_url,
        );

        let event = next_async_subresource_event(&mut queue).await?;
        let body_source_id = match event {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 41);
                assert_eq!(started.head.status, 200);
                started.body_source_id
            }
            other => anyhow::bail!("expected streaming start for XHR, got {other:?}"),
        };

        let mut body = Vec::new();
        loop {
            match next_async_subresource_event(&mut queue).await? {
                AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                    assert_eq!(chunk.body_source_id, body_source_id);
                    body.extend_from_slice(&chunk.bytes);
                }
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id: finished_body_source_id,
                    completion,
                } => {
                    let finished = completion.complete_for_test();
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert_eq!(finished.internal_id, 41);
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert!(
                        finished.result.is_ok(),
                        "streaming XHR finish failed: {:?}",
                        finished.result
                    );
                    break;
                }
                other => anyhow::bail!("unexpected async subresource event: {other:?}"),
            }
        }

        assert_eq!(body, b"hello-xhr");
        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_origin_unsafe_xhr_streams_while_redirect_preflight_is_armed() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (head_sent_tx, head_sent_rx) = tokio::sync::oneshot::channel();
        let (send_tail_tx, send_tail_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut stream).await.unwrap();
            assert!(request.starts_with("POST /xhr HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("x-challenge: yes"));
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain\r\n",
                        "Content-Length: 9\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "hello"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let _ = head_sent_tx.send(());
            send_tail_rx
                .await
                .expect("test should release the final response bytes");
            stream.write_all(b"-xhr").await.unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("http://{addr}/xhr"))?;
        let document_url = Url::parse(&format!("http://{addr}/page"))?;
        let request_headers = vec![("X-Challenge".to_owned(), "yes".to_owned())];
        let request = Request::new(
            "POST",
            request_url.as_str(),
            Some("payload".to_owned()),
            request_headers.clone(),
        )?
        .with_initiator_url(&document_url)
        .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
        assert!(browser_request_needs_manual_preflight_redirects(
            &request,
            &request_headers,
        ));

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            request_headers.clone(),
            42,
            ResourceResponseStream::unobserved_for_test(),
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: None,
                resource_type: SubresourceResourceType::Xhr,
                keepalive: false,
            },
            request_url,
        );

        head_sent_rx
            .await
            .expect("server should publish the response head and first bytes");
        let body_source_id = match tokio::time::timeout(
            Duration::from_secs(2),
            next_async_subresource_event(&mut queue),
        )
        .await
        .map_err(|_| anyhow::anyhow!("XHR response head waited for the complete response body"))??
        {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 42);
                assert_eq!(started.head.status, 200);
                started.body_source_id
            }
            other => anyhow::bail!("expected headers-first XHR stream, got {other:?}"),
        };

        send_tail_tx
            .send(())
            .expect("server should still be waiting for final response bytes");
        let mut body = Vec::new();
        loop {
            match next_async_subresource_event(&mut queue).await? {
                AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                    assert_eq!(chunk.body_source_id, body_source_id);
                    body.extend_from_slice(&chunk.bytes);
                }
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id: finished_body_source_id,
                    completion,
                } => {
                    let finished = completion.complete_for_test();
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert_eq!(finished.internal_id, 42);
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert!(finished.result.is_ok());
                    break;
                }
                other => anyhow::bail!("unexpected async subresource event: {other:?}"),
            }
        }

        assert_eq!(body, b"hello-xhr");
        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unsafe_xhr_redirect_preflights_next_origin_then_streams_final_response() -> Result<()>
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let source_origin = format!("http://127.0.0.1:{}", addr.port());
        let target_url = format!("http://localhost:{}/final", addr.port());
        let target_url_for_server = target_url.clone();
        let source_origin_for_server = source_origin.clone();
        let (head_sent_tx, head_sent_rx) = tokio::sync::oneshot::channel();
        let (send_tail_tx, send_tail_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut redirect, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut redirect).await.unwrap();
            assert!(request.starts_with("POST /start HTTP/1.1"));
            let redirect_response = format!(
                concat!(
                    "HTTP/1.1 307 Temporary Redirect\r\n",
                    "Location: {}\r\n",
                    "Content-Length: 0\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                target_url_for_server
            );
            redirect
                .write_all(redirect_response.as_bytes())
                .await
                .unwrap();
            drop(redirect);

            let (mut preflight, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut preflight).await.unwrap();
            let request_lower = request.to_ascii_lowercase();
            assert!(request.starts_with("OPTIONS /final HTTP/1.1"));
            assert!(request_lower.contains("access-control-request-method: post"));
            assert!(request_lower.contains("access-control-request-headers: x-challenge"));
            let preflight_response = format!(
                concat!(
                    "HTTP/1.1 204 No Content\r\n",
                    "Access-Control-Allow-Origin: {}\r\n",
                    "Access-Control-Allow-Methods: POST\r\n",
                    "Access-Control-Allow-Headers: x-challenge\r\n",
                    "Content-Length: 0\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                source_origin_for_server
            );
            preflight
                .write_all(preflight_response.as_bytes())
                .await
                .unwrap();
            drop(preflight);

            let (mut final_response, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut final_response).await.unwrap();
            assert!(request.starts_with("POST /final HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("x-challenge: yes"));
            let response_head = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Access-Control-Allow-Origin: {}\r\n",
                    "Content-Type: text/plain\r\n",
                    "Content-Length: 9\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "hello"
                ),
                source_origin_for_server
            );
            final_response
                .write_all(response_head.as_bytes())
                .await
                .unwrap();
            let _ = head_sent_tx.send(());
            send_tail_rx
                .await
                .expect("test should release the redirected response tail");
            final_response.write_all(b"-xhr").await.unwrap();
        });

        let mut queue = RendererResourceCompletionTestHarness::new();
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_url = Url::parse(&format!("{source_origin}/start"))?;
        let document_url = Url::parse(&format!("{source_origin}/page"))?;
        let request_headers = vec![("X-Challenge".to_owned(), "yes".to_owned())];
        let request = Request::new(
            "POST",
            request_url.as_str(),
            Some("payload".to_owned()),
            request_headers.clone(),
        )?
        .with_initiator_url(&document_url)
        .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            request_headers.clone(),
            43,
            ResourceResponseStream::unobserved_for_test(),
            CorsPreflightNetworkObserver {
                request: crate::runtime::RendererNetworkRequest::unobserved_for_test(),
                observer: queue.sender().network_observer(),
                frame_id: None,
                resource_type: SubresourceResourceType::Xhr,
                keepalive: false,
            },
            request_url.clone(),
        );

        head_sent_rx
            .await
            .expect("server should publish the redirected final response head");
        expect_native_preflight(
            &mut queue,
            &document_url,
            &Url::parse(&target_url)?,
            None,
            204,
            SubresourceResourceType::Xhr,
        )
        .await?;
        let body_source_id = match next_async_subresource_event(&mut queue).await? {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 43);
                assert_eq!(started.head.final_url.as_str(), target_url);
                assert!(started.head.redirected);
                assert_eq!(started.head.redirect_chain.len(), 1);
                assert_eq!(started.head.redirect_chain[0].from_url, request_url);
                assert_eq!(started.head.redirect_chain[0].to_url.as_str(), target_url);
                assert_eq!(started.head.redirect_chain[0].status, 307);
                started.body_source_id
            }
            other => anyhow::bail!("expected redirected final response stream, got {other:?}"),
        };

        send_tail_tx
            .send(())
            .expect("server should still be waiting for redirected response bytes");
        let mut body = Vec::new();
        loop {
            match next_async_subresource_event(&mut queue).await? {
                AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                    assert_eq!(chunk.body_source_id, body_source_id);
                    body.extend_from_slice(&chunk.bytes);
                }
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id: finished_body_source_id,
                    completion,
                } => {
                    let finished = completion.complete_for_test();
                    assert_eq!(finished_body_source_id, body_source_id);
                    assert_eq!(finished.internal_id, 43);
                    assert!(finished.result.is_ok());
                    break;
                }
                other => anyhow::bail!("unexpected async subresource event: {other:?}"),
            }
        }

        assert_eq!(body, b"hello-xhr");
        server.await?;
        Ok(())
    }
}

use super::*;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, NetworkFetchResult, RedirectInfo,
    RequestCredentialsMode, RequestMode, RequestRedirectMode, ResponseHead, StreamingRawResponse,
    is_cors_safelisted_method,
};

const MAX_MANUAL_CORS_REDIRECTS: usize = 20;

#[cfg(test)]
pub(crate) async fn fetch_browser_subresource_with_preflight(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<Response, String> {
    fetch_browser_subresource_with_preflight_and_network_metadata(loader, request, cancel_handle)
        .await
        .map(NetworkFetchResult::into_response)
}

pub(crate) async fn fetch_browser_subresource_with_preflight_and_network_metadata(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<NetworkFetchResult<Response>, String> {
    let preflight_request_headers = request.request_headers.clone();
    fetch_browser_subresource_with_preflight_headers_and_observer(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        None,
    )
    .await
}

pub(crate) async fn fetch_browser_subresource_with_preflight_headers(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
) -> Result<Response, String> {
    fetch_browser_subresource_with_preflight_headers_and_network_metadata(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
    )
    .await
    .map(NetworkFetchResult::into_response)
}

pub(crate) async fn fetch_browser_subresource_with_preflight_headers_and_network_metadata(
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
) -> Result<NetworkFetchResult<Response>, String> {
    fetch_browser_subresource_with_preflight_headers_and_observer(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        None,
    )
    .await
}

async fn fetch_browser_subresource_with_preflight_headers_and_observer(
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

pub(crate) fn browser_request_needs_manual_preflight_redirects(
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
    ) && request.request_mode != RequestMode::NoCors
        && request.cookie_context.initiator_url.is_some()
        && (!is_cors_safelisted_method(&request.method)
            || !moli_fetch::cors_unsafe_request_header_names(preflight_request_headers).is_empty())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ManualCorsRedirectTransition {
    FinalResponse,
    ManualResponse { response_url: url::Url },
    FollowedRedirect,
}

struct ManualCorsRedirectState {
    request: Request,
    preflight_request_headers: Vec<(String, String)>,
    redirect_chain: Vec<RedirectInfo>,
}

impl ManualCorsRedirectState {
    fn new(request: Request, preflight_request_headers: Vec<(String, String)>) -> Self {
        Self {
            request,
            preflight_request_headers,
            redirect_chain: Vec::new(),
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
            self.redirect_chain.len(),
        )?
        else {
            return Ok(ManualCorsRedirectTransition::FinalResponse);
        };

        if self.request.redirect_mode == RequestRedirectMode::Manual {
            return Ok(ManualCorsRedirectTransition::ManualResponse {
                response_url: head.final_url,
            });
        }

        match self.request.redirect_mode {
            RequestRedirectMode::Follow => {}
            RequestRedirectMode::Error | RequestRedirectMode::Manual => {
                unreachable!("redirect modes were handled before the follow transition")
            }
        }
        let redirect_status = head.status;
        self.redirect_chain.push(RedirectInfo {
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
        self.preflight_request_headers = self.request.request_headers.clone();
        Ok(ManualCorsRedirectTransition::FollowedRedirect)
    }

    fn into_redirect_chain(self) -> Vec<RedirectInfo> {
        self.redirect_chain
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
            ManualCorsRedirectTransition::ManualResponse { .. } => return Ok(observed),
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
) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
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
        match redirects.advance(head, network_extra_info_available)? {
            ManualCorsRedirectTransition::FinalResponse => {
                let redirect_chain = redirects.into_redirect_chain();
                observed.response_mut().redirected = !redirect_chain.is_empty();
                observed.response_mut().redirect_chain = redirect_chain;
                return Ok(observed);
            }
            ManualCorsRedirectTransition::ManualResponse { response_url } => {
                return Err(format!(
                    "manual redirect unexpectedly entered follow-mode streaming from {}",
                    response_url
                ));
            }
            ManualCorsRedirectTransition::FollowedRedirect => {}
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
    validate_actual_cors_response_parts(request, &response.final_url, &response.headers)
}

fn validate_actual_cors_response_parts(
    request: &Request,
    response_url: &url::Url,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() else {
        return Ok(());
    };
    if request.request_mode == RequestMode::NoCors {
        return Ok(());
    }
    validate_cors_response(
        initiator_url,
        response_url,
        response_headers,
        request.credentials_mode,
    )
}

fn next_redirect_url(
    final_url: &url::Url,
    status: u16,
    headers: &[(String, String)],
    redirect_count: usize,
) -> Result<Option<url::Url>, String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.as_str())
    else {
        return Ok(None);
    };
    if redirect_count >= MAX_MANUAL_CORS_REDIRECTS {
        return Err(format!("redirect limit exceeded for {final_url}"));
    }
    final_url
        .join(location)
        .or_else(|_| url::Url::parse(location))
        .map(Some)
        .map_err(|error| {
            format!("failed to resolve redirect location `{location}` from {final_url}: {error}")
        })
}

pub(crate) async fn fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
    fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        None,
    )
    .await
}

async fn fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
    // Borrow the loader so its fetch runtime stays alive until the caller drains
    // and finishes the returned StreamingRawResponse.
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
    validate_redirect_mode_response_head(&result.response().head(), redirect_mode)?;
    Ok(result)
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
    if let Some(initiator_url) = request.cookie_context.initiator_url.clone()
        && request.request_mode != RequestMode::NoCors
        && let Some(preflight_headers) = cors_preflight_request_headers(
            &initiator_url,
            &request.url,
            &request.method,
            preflight_request_headers,
        )
    {
        let observable_preflight_headers = preflight_headers.clone();
        let mut preflight_request =
            Request::new("OPTIONS", request.url.as_str(), None, preflight_headers)
                .map_err(|error| format!("cors preflight: failed to build request: {error}"))?
                .with_initiator_url(&initiator_url)
                .with_credentials_mode(RequestCredentialsMode::SameOrigin)
                .with_network_partition_key(request.network_partition_key().map(str::to_owned));
        if let Some(metadata) = request.browser_request_metadata() {
            preflight_request = preflight_request.with_browser_request_metadata(metadata);
        } else {
            preflight_request =
                preflight_request.with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        }

        let preflight_response = match fetch_response_head_once(
            loader,
            preflight_request,
            cancel_handle.clone(),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Some(observer) = preflight_observer {
                    observer.send_preflight_failure(
                        request.url.clone(),
                        observable_preflight_headers,
                        error.clone(),
                    );
                }
                return Err(error);
            }
        };
        if let Some(observer) = preflight_observer {
            observer.send_preflight_success(
                request.url.clone(),
                observable_preflight_headers,
                &preflight_response,
            );
        }
        if preflight_response.redirected {
            return Err(format!(
                "CORS preflight failed: preflight request redirected to {}",
                preflight_response.final_url
            ));
        }
        validate_cors_preflight_response(
            &initiator_url,
            &preflight_response.final_url,
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
    network_context: AsyncSubresourceNetworkContext,
    request_url: url::Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
) {
    spawn_async_subresource_fetch_with_redirect_chain(
        task_runner,
        completion_tx,
        loader,
        request,
        cancel_handle,
        preflight_request_headers,
        Vec::new(),
        internal_id,
        network_context,
        request_url,
        request_method,
        request_headers,
        request_body,
    );
}

pub(crate) fn spawn_async_subresource_fetch_with_redirect_chain(
    task_runner: crate::network::RendererResourceTaskRunner,
    completion_tx: RendererResourceCompletionSender,
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    initial_redirect_chain: Vec<RedirectInfo>,
    internal_id: u64,
    network_context: AsyncSubresourceNetworkContext,
    request_url: url::Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
) {
    let parkable_image_manager = matches!(
        request.browser_request_metadata(),
        Some(BrowserRequestMetadata::Image)
    )
    .then(|| loader.parkable_image_manager(&task_runner));
    task_runner.spawn(async move {
        let preflight_observer =
            CorsPreflightNetworkObserver::new(completion_tx.clone(), network_context);
        let auth_requires_buffered_transport = request.auth_requires_buffered_transport();
        let requires_manual_preflight_redirects =
            browser_request_needs_manual_preflight_redirects(&request, &preflight_request_headers);
        let can_stream_subresource_body = matches!(
            request.browser_request_metadata(),
            Some(
                BrowserRequestMetadata::Fetch
                    | BrowserRequestMetadata::EventSource
                    | BrowserRequestMetadata::JsonModule
                    | BrowserRequestMetadata::Manifest
                    | BrowserRequestMetadata::StyleModule
                    | BrowserRequestMetadata::Xhr,
            )
        ) && request.follow_redirects
            && request.request_mode != RequestMode::NoCors;
        let can_collect_image_body = parkable_image_manager.is_some() && request.follow_redirects;
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %request.url,
                method = %request.method,
                browser_request_metadata = ?request.browser_request_metadata(),
                request_mode = ?request.request_mode,
                redirect_mode = ?request.redirect_mode,
                follow_redirects = request.follow_redirects,
                auth_requires_buffered_transport,
                requires_manual_preflight_redirects,
                can_stream_subresource_body,
                can_collect_image_body,
                stage = "async_subresource_transport_selected",
            );
        }
        if !auth_requires_buffered_transport && can_collect_image_body {
            let result = fetch_browser_image_into_parkable(
                &loader,
                request,
                cancel_handle,
                preflight_request_headers,
                Some(&preflight_observer),
                initial_redirect_chain,
                parkable_image_manager
                    .expect("an image transport selection must retain its parkable manager"),
            )
            .await;
            let (result, parkable_image) = match result {
                Ok((response, image)) => (Ok(response), Some(image)),
                Err(error) => (Err(error), None),
            };
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                parkable_image,
                result,
            });
            return;
        }
        if auth_requires_buffered_transport || !can_stream_subresource_body {
            let result = fetch_browser_subresource_with_preflight_headers_and_observer(
                loader,
                request,
                cancel_handle,
                preflight_request_headers,
                Some(&preflight_observer),
            )
            .await
            .map(|observed| {
                let (mut response, request_observation) = observed.into_parts();
                if !initial_redirect_chain.is_empty() {
                    let mut redirect_chain = initial_redirect_chain;
                    redirect_chain.append(&mut response.redirect_chain);
                    response.redirect_chain = redirect_chain;
                    response.redirected = true;
                }
                crate::protocol_types::NavigationResponse::from(response)
                    .with_network_request_headers(
                        request_observation.map(|observation| observation.into_headers()),
                    )
            });
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                parkable_image: None,
                result,
            });
            return;
        }

        let request_url_for_event = request_url.clone();
        let request_method_for_event = request_method.clone();
        let request_headers_for_event = request_headers.clone();
        let request_body_for_event = request_body.clone();
        let result = fetch_browser_subresource_streaming_with_preflight_headers(
            completion_tx.clone(),
            loader,
            request,
            cancel_handle,
            preflight_request_headers,
            Some(&preflight_observer),
            internal_id,
            request_url_for_event,
            request_method_for_event,
            request_headers_for_event,
            request_body_for_event,
            initial_redirect_chain,
        )
        .await;
        if let Err(error) = result {
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                parkable_image: None,
                result: Err(error),
            });
        }
    });
}

async fn fetch_browser_image_into_parkable(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
    initial_redirect_chain: Vec<RedirectInfo>,
    manager: moli_parkable_image::ParkableImageManager,
) -> Result<
    (
        crate::protocol_types::NavigationResponse,
        moli_parkable_image::ParkableImage,
    ),
    String,
> {
    let requires_manual_preflight_redirects =
        browser_request_needs_manual_preflight_redirects(&request, &preflight_request_headers);
    let observed = if requires_manual_preflight_redirects {
        fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
            loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await?
    } else {
        fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
            loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await?
    };
    collect_image_response_into_parkable(observed, initial_redirect_chain, manager).await
}

pub(crate) async fn collect_image_response_into_parkable(
    observed: NetworkFetchResult<StreamingRawResponse>,
    initial_redirect_chain: Vec<RedirectInfo>,
    manager: moli_parkable_image::ParkableImageManager,
) -> Result<
    (
        crate::protocol_types::NavigationResponse,
        moli_parkable_image::ParkableImage,
    ),
    String,
> {
    let (mut response, request_observation) = observed.into_parts();
    let mut head = response.head();
    if !initial_redirect_chain.is_empty() {
        let mut redirect_chain = initial_redirect_chain;
        redirect_chain.append(&mut head.redirect_chain);
        head.redirect_chain = redirect_chain;
        head.redirected = true;
    }
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

async fn fetch_browser_subresource_streaming_with_preflight_headers(
    completion_tx: RendererResourceCompletionSender,
    loader: ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
    preflight_request_headers: Vec<(String, String)>,
    preflight_observer: Option<&CorsPreflightNetworkObserver>,
    internal_id: u64,
    request_url: url::Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    initial_redirect_chain: Vec<RedirectInfo>,
) -> Result<(), String> {
    let body_source_id = new_network_body_source_id();
    let requires_manual_preflight_redirects =
        browser_request_needs_manual_preflight_redirects(&request, &preflight_request_headers);
    let observed = if requires_manual_preflight_redirects {
        fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
            &loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await?
    } else {
        fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
            &loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await?
    };
    let (mut response, request_observation) = observed.into_parts();
    let mut head = response.head();
    if !initial_redirect_chain.is_empty() {
        let mut redirect_chain = initial_redirect_chain;
        redirect_chain.append(&mut head.redirect_chain);
        head.redirect_chain = redirect_chain;
        head.redirected = true;
    }
    let _ = completion_tx.send_async_subresource_event(
        AsyncSubresourceFetchEvent::StreamingStarted(Box::new(AsyncSubresourceStreamingStarted {
            internal_id,
            request_url,
            request_method,
            request_headers,
            request_body,
            body_source_id,
            head,
            network_request_headers: request_observation
                .map(|observation| observation.into_headers()),
        })),
    );
    while let Some(bytes) = response.next_chunk().await {
        let _ = completion_tx.send_async_subresource_event(
            AsyncSubresourceFetchEvent::StreamingChunk(AsyncSubresourceStreamingChunk {
                body_source_id,
                bytes,
            }),
        );
    }
    let result = response.finish().await.map_err(format_network_error);
    let _ = completion_tx.send_async_subresource_event(
        AsyncSubresourceFetchEvent::StreamingFinished(AsyncSubresourceStreamingFinished {
            internal_id,
            body_source_id,
            result,
        }),
    );
    Ok(())
}

async fn fetch_once(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<Response, String> {
    fetch_once_with_network_metadata(loader, request, cancel_handle)
        .await
        .map(NetworkFetchResult::into_response)
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

async fn fetch_response_head_once(
    loader: &ResourceRequestClient,
    request: Request,
    cancel_handle: Option<FetchCancelHandle>,
) -> Result<ResponseHead, String> {
    // Challenge-response auth retries are completed inside libcurl on the
    // buffered path. Preemptive Basic auth can still use the streaming head
    // path because credentials are already represented as request headers.
    if request.auth_requires_buffered_transport() {
        return fetch_once(loader, request, cancel_handle)
            .await
            .map(|response| response.head());
    }

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
    headers: &[(String, String)],
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

    async fn read_http_request_text(stream: &mut tokio::net::TcpStream) -> Result<String> {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            let read = stream.read(&mut byte).await?;
            if read == 0 {
                anyhow::bail!("client closed before sending complete request");
            }
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                return Ok(String::from_utf8(request)?);
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
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
        let mut redirects = ManualCorsRedirectState::new(request, request_headers);

        let transition = redirects
            .advance(
                ResponseHead {
                    final_url: Url::parse("https://origin.test/start")?,
                    status: 303,
                    headers: vec![(
                        "Location".to_owned(),
                        "https://target.test/final".to_owned(),
                    )],
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
        assert_eq!(redirects.redirect_chain.len(), 1);
        assert_eq!(redirects.redirect_chain[0].status, 303);
        assert!(redirects.redirect_chain[0].network_extra_info_available);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_uses_streaming_head_response() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut preflight, _) = listener.accept().await.unwrap();
            let preflight_request = read_http_request_text(&mut preflight).await.unwrap();
            assert!(preflight_request.starts_with("OPTIONS /resource HTTP/1.1"));
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
        .with_initiator_url(&Url::parse("http://origin.test/page")?)
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

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_emits_observed_record_before_actual_completion() -> Result<()> {
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
            AsyncSubresourceNetworkContext {
                frame_id: Some("FRAME-1".to_owned()),
                document_url: document_url.clone(),
                resource_type: SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            request_url.clone(),
            "GET".to_owned(),
            request_headers,
            None,
        );

        match next_async_subresource_event(&mut queue).await? {
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => {
                assert_eq!(record.frame_id(), Some("FRAME-1"));
                assert_eq!(record.document_url(), &document_url);
                assert_eq!(record.url(), &request_url);
                assert_eq!(record.method(), "OPTIONS");
                assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
                assert!(matches!(
                    record.outcome(),
                    crate::types::SubresourceNetworkOutcome::Success { status: 200, .. }
                ));
            }
            other => anyhow::bail!("expected observed preflight record first, got {other:?}"),
        }

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
                AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                    assert_eq!(finished.internal_id, 73);
                    assert_eq!(finished.body_source_id, body_source_id);
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
            let request = read_http_request_text(&mut stream).await.unwrap();
            assert!(request.starts_with("GET /image.png HTTP/1.1"));
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
        let request = Request::get(request_url.as_str())?
            .with_initiator_url(&document_url)
            .with_request_mode(RequestMode::NoCors)
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_browser_request_metadata(BrowserRequestMetadata::Image);

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            Vec::new(),
            75,
            AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url,
                resource_type: SubresourceResourceType::Image,
                policy_context: Default::default(),
            },
            request_url,
            "GET".to_owned(),
            Vec::new(),
            None,
        );

        let AsyncSubresourceFetchEvent::Completion(completion) =
            next_async_subresource_event(&mut queue).await?
        else {
            anyhow::bail!("image transport must emit one buffered terminal completion");
        };
        assert_eq!(completion.internal_id, 75);
        let response = completion
            .result
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        assert_eq!(response.status, 200);
        assert!(response.body_bytes().is_empty());
        let encoded = completion
            .parkable_image
            .as_ref()
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
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let initial_redirect_chain = vec![RedirectInfo {
            from_url: source_url.clone(),
            to_url: target_url.clone(),
            status: 302,
            headers: vec![("Location".to_owned(), target_url.to_string())],
            network_extra_info_available: false,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }];

        spawn_async_subresource_fetch_with_redirect_chain(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            Vec::new(),
            initial_redirect_chain,
            74,
            AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: Url::parse(&format!("http://{addr}/page"))?,
                resource_type: SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            target_url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
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
                AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
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
            .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        spawn_async_subresource_fetch(
            crate::network::RendererResourceTaskRunner::from_current_tokio()?,
            queue.sender(),
            loader,
            request,
            Some(FetchCancelHandle::new()),
            Vec::new(),
            41,
            AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: Url::parse("http://origin.test/page")?,
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url,
            "GET".to_owned(),
            Vec::new(),
            None,
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
                AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                    assert_eq!(finished.internal_id, 41);
                    assert_eq!(finished.body_source_id, body_source_id);
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url,
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url,
            "POST".to_owned(),
            request_headers,
            Some("payload".to_owned()),
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
                AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                    assert_eq!(finished.internal_id, 42);
                    assert_eq!(finished.body_source_id, body_source_id);
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: document_url.clone(),
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url.clone(),
            "POST".to_owned(),
            request_headers,
            Some("payload".to_owned()),
        );

        head_sent_rx
            .await
            .expect("server should publish the redirected final response head");
        match next_async_subresource_event(&mut queue).await? {
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => {
                assert_eq!(record.document_url(), &document_url);
                assert_eq!(record.url().as_str(), target_url);
                assert_eq!(record.method(), "OPTIONS");
                assert!(matches!(
                    record.outcome(),
                    crate::types::SubresourceNetworkOutcome::Success { status: 204, .. }
                ));
            }
            other => anyhow::bail!("expected redirected-hop preflight first, got {other:?}"),
        }
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
                AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
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

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
    let preflight_request_headers = request.request_headers.to_byte_strings();
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
    let cancel_handle = cancel_handle.unwrap_or_default();
    let response_cancel = cancel_handle.child_for_subrequest();
    let observed =
        fetch_once_with_network_metadata(&loader, request, Some(response_cancel.clone())).await?;
    cancel_handle.adopt_response_progress(&response_cancel);
    Ok(observed)
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
    ) && request.request_mode == RequestMode::Cors
        && request.cookie_context.initiator_url.is_some()
        && (request.use_cors_preflight()
            || !is_cors_safelisted_method(&request.method)
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

    // Construct this copy at transport dispatch, after configuring Fetch
    // policy: a later with_redirect_mode(Follow) would enable runtime following.
    fn single_hop_transport_request(&self) -> Request {
        self.request.clone().with_single_hop()
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
        if self.request.redirect_mode() == RequestRedirectMode::Error {
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

        if self.request.redirect_mode() == RequestRedirectMode::Manual {
            return Ok(ManualCorsRedirectTransition::ManualResponse {
                response_url: head.final_url,
            });
        }

        match self.request.redirect_mode() {
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
    if !request.auth_requires_buffered_transport()
        && request.redirect_mode() == RequestRedirectMode::Follow
    {
        // Materialize only the accepted final response. Waiting for buffered
        // intermediate bodies here lets an unfinished redirect block the next
        // hop and its preflight indefinitely.
        let observed = fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
            &loader,
            request,
            cancel_handle,
            preflight_request_headers,
            preflight_observer,
        )
        .await?;
        let (response, observation_journal) = observed.into_parts_with_observation_journal();
        let response = response
            .into_lossy_materialized_text_response()
            .await
            .map_err(format_network_error)?;
        return Ok(NetworkFetchResult::with_observation_journal(
            response,
            observation_journal,
        ));
    }

    // Auth challenge-response and manual/error redirect compatibility still
    // use the buffered collector, which waits for each response's entire body.
    // Unfinished bodies can stall these paths until cancellation or timeout.
    // Covered by buffered_redirect_body_wait_is_cancellable_without_committing_completion
    // in moli-fetch/src/tests/response_progress.rs.
    let mut redirects = ManualCorsRedirectState::new(request, preflight_request_headers);
    let cancel_handle = cancel_handle.unwrap_or_default();

    loop {
        redirects
            .run_current_hop_preflight(&loader, Some(cancel_handle.clone()), preflight_observer)
            .await?;

        let response_cancel = cancel_handle.child_for_subrequest();
        let mut observed = fetch_once_with_network_metadata_unvalidated(
            &loader,
            redirects.single_hop_transport_request(),
            Some(response_cancel.clone()),
        )
        .await?;
        let network_extra_info_available = observed.request_observation().is_some();
        match redirects.advance(observed.response().head(), network_extra_info_available)? {
            ManualCorsRedirectTransition::FinalResponse => {
                cancel_handle.adopt_response_progress(&response_cancel);
                let redirect_chain = redirects.into_redirect_chain();
                observed.response_mut().redirected = !redirect_chain.is_empty();
                observed.response_mut().redirect_chain = redirect_chain;
                return Ok(observed);
            }
            ManualCorsRedirectTransition::ManualResponse { .. } => {
                cancel_handle.adopt_response_progress(&response_cancel);
                return Ok(observed);
            }
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
    let cancel_handle = cancel_handle.unwrap_or_default();

    loop {
        redirects
            .run_current_hop_preflight(loader, Some(cancel_handle.clone()), preflight_observer)
            .await?;

        let mut observed = loader
            .fetch_raw_stream_with_cancel_and_network_metadata(
                redirects.single_hop_transport_request(),
                cancel_handle.child_for_subrequest(),
            )
            .await
            .map_err(format_network_error)?;
        let network_extra_info_available = observed.request_observation().is_some();
        let head = observed.response().head();
        match redirects.advance(head, network_extra_info_available)? {
            ManualCorsRedirectTransition::FinalResponse => {
                cancel_handle.adopt_response_progress(&observed.response().cancellation_handle());
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

        // Dropping this hop retires only its transport. Its body and late
        // completion cannot cancel or commit the logical request's next hop.
        drop(observed);
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
        .map(|(_, value)| moli_fetch::encode_redirect_location(value))
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
    let redirect_mode = request.redirect_mode();
    let result = loader
        .fetch_raw_stream_with_cancel_and_network_metadata(
            request,
            cancel_handle.child_for_subrequest(),
        )
        .await
        .map_err(format_network_error)?;
    validate_redirect_mode_response_head(&result.response().head(), redirect_mode)?;
    cancel_handle.adopt_response_progress(&result.response().cancellation_handle());
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
    let request_origin = request
        .browser_origin()
        .map_err(|error| error.to_string())?;
    if request.request_mode == RequestMode::Cors
        && let Some(preflight_headers) = cors_preflight_request_headers(
            request.has_cross_origin_url(&request.url),
            &request.url,
            &request.method,
            preflight_request_headers,
            request.use_cors_preflight(),
        )
    {
        let observable_preflight_headers = preflight_headers.clone();
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
        .with_network_partition_key(request.network_partition_key().map(str::to_owned))
        .with_referrer_from(request);
        if let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() {
            preflight_request = preflight_request.with_initiator_url(initiator_url);
        }
        if let Some(metadata) = request.browser_request_metadata() {
            preflight_request = preflight_request.with_browser_request_metadata(metadata);
        } else {
            preflight_request =
                preflight_request.with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        }

        let preflight_response = match fetch_response_head_once(
            loader,
            preflight_request,
            cancel_handle
                .as_ref()
                .map(FetchCancelHandle::child_for_subrequest),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Some(observer) = preflight_observer {
                    observer.send_preflight_failure(
                        request.url.clone(),
                        moli_fetch::RequestHeaders::from_byte_strings(
                            &observable_preflight_headers,
                        )
                        .expect("observed request headers are ByteStrings"),
                        error.clone(),
                    );
                }
                return Err(error);
            }
        };
        if let Some(observer) = preflight_observer {
            observer.send_preflight_success(
                request.url.clone(),
                moli_fetch::RequestHeaders::from_byte_strings(&observable_preflight_headers)
                    .expect("observed request headers are ByteStrings"),
                &preflight_response,
            );
        }
        validate_cors_preflight_response(
            &request.serialized_origin(),
            request.credentials_mode,
            &request.method,
            preflight_request_headers,
            preflight_response.status,
            &preflight_response.headers,
            request.use_cors_preflight(),
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
    request_headers: moli_fetch::RequestHeaders,
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
        ) && request.transport_follows_redirects()
            && request.request_mode != RequestMode::NoCors;
        let can_collect_image_body =
            parkable_image_manager.is_some() && request.transport_follows_redirects();
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %request.url,
                method = %request.method,
                browser_request_metadata = ?request.browser_request_metadata(),
                request_mode = ?request.request_mode,
                redirect_handling = ?request.redirect_handling(),
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
                parkable_image_manager
                    .expect("an image transport selection must retain its parkable manager"),
            )
            .await;
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
                result: crate::types::AsyncSubresourceFetchResult::from_image_result(result),
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
                let (response, request_observation) = observed.into_parts();
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
                result: result.into(),
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
                result: crate::types::AsyncSubresourceFetchResult::Failure(error),
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
    collect_image_response_into_parkable(observed, manager).await
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
    request_headers: moli_fetch::RequestHeaders,
    request_body: Option<String>,
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
    let head = response.head();
    let _ = completion_tx.send_async_subresource_event(
        AsyncSubresourceFetchEvent::StreamingStarted(Box::new(AsyncSubresourceStreamingStarted {
            skip_fetch_security_validation: false,
            response_filter: None,
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
    let redirect_mode = request.redirect_mode();
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
    let response = loader
        .fetch_raw_stream_with_cancel(request, cancel_handle)
        .await
        .map_err(format_network_error)?;
    // Preflight validation consumes only the headers. Its private cancellation
    // authority lets Drop stop a slow body without aborting the actual request.
    Ok(response.head())
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
    fn redirect_location_preserves_raw_bytes_and_existing_escapes() {
        let base = Url::parse("https://redirect.test/start").unwrap();
        for (location, expected) in [
            (b"/final?utf8=\xe2\x98\x83".as_slice(), "utf8=%E2%98%83"),
            (b"/final?bad=\xff".as_slice(), "bad=%FF"),
            (
                b"/final?%e2%98%83\xe2\x98\x83".as_slice(),
                "%e2%98%83%E2%98%83",
            ),
        ] {
            let headers = vec![("Location".to_owned(), location.to_vec())];
            let target = next_redirect_url(&base, 302, &headers, 0).unwrap().unwrap();
            assert_eq!(target.query(), Some(expected));
            assert!(
                next_redirect_url(&base, 200, &headers, 0)
                    .unwrap()
                    .is_none()
            );
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
                    status_text: None,
                    final_url: Url::parse("https://origin.test/start")?,
                    status: 303,
                    headers: vec![("Location".to_owned(), b"https://target.test/final".to_vec())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    cache_state: Default::default(),
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

    #[test]
    fn single_hop_cors_redirects_preserve_follow_manual_and_error_policy() -> Result<()> {
        let current_url = Url::parse("https://origin.test/start")?;
        let next_url = current_url.join("/final")?;
        for mode in [
            RequestRedirectMode::Follow,
            RequestRedirectMode::Manual,
            RequestRedirectMode::Error,
        ] {
            let request = Request::get(current_url.as_str())?
                .with_request_origin(moli_url::WebOrigin::from_url(&current_url))
                .with_redirect_mode(mode);
            let mut redirects = ManualCorsRedirectState::new(request, Vec::new());
            let hop = redirects.single_hop_transport_request();
            assert!(!hop.transport_follows_redirects());
            assert_eq!(hop.redirect_mode(), mode);

            let transition = redirects.advance(
                ResponseHead {
                    final_url: current_url.clone(),
                    status: 307,
                    status_text: Some("Temporary Redirect".to_owned()),
                    headers: vec![("location".to_owned(), b"/final".to_vec())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    cache_state: Default::default(),
                    negotiated_http_version: None,
                },
                false,
            );
            match mode {
                RequestRedirectMode::Follow => {
                    assert_eq!(
                        transition.map_err(anyhow::Error::msg)?,
                        ManualCorsRedirectTransition::FollowedRedirect
                    );
                    assert_eq!(redirects.request().url, next_url);
                }
                RequestRedirectMode::Manual => {
                    assert_eq!(
                        transition.map_err(anyhow::Error::msg)?,
                        ManualCorsRedirectTransition::ManualResponse {
                            response_url: current_url.clone(),
                        }
                    );
                    assert_eq!(redirects.request().url, current_url);
                }
                RequestRedirectMode::Error => {
                    assert!(transition.unwrap_err().contains("redirect mode error"));
                    assert_eq!(redirects.request().url, current_url);
                }
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_preserves_fetch_referrer_and_policy() -> Result<()> {
        let initiator = Url::parse("http://origin.test/context/page?x=1#fragment")?;
        for (referrer, policy, document_policy, expected) in [
            ("", Some("unsafe-url"), None, None),
            (
                "about:client",
                Some("unsafe-url"),
                None,
                Some("http://origin.test/context/page?x=1"),
            ),
            (
                "http://user:secret@origin.test/selected?q=1#private",
                Some("unsafe-url"),
                None,
                Some("http://origin.test/selected?q=1"),
            ),
            (
                "http://origin.test/selected",
                Some("origin"),
                None,
                Some("http://origin.test/"),
            ),
            (
                "http://origin.test/selected",
                None,
                Some("unsafe-url"),
                Some("http://origin.test/selected"),
            ),
            (
                "http://origin.test/selected",
                Some("no-referrer"),
                Some("unsafe-url"),
                None,
            ),
            (
                "http://origin.test/selected",
                None,
                Some("no-referrer"),
                None,
            ),
            (
                "http://origin.test/selected",
                None,
                None,
                Some("http://origin.test/"),
            ),
        ] {
            for streaming in [false, true] {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let url = format!("http://{}/echo", listener.local_addr()?);
                let server = tokio::spawn(async move {
                    let mut requests = Vec::new();
                    for _ in 0..2 {
                        let (mut socket, _) = listener.accept().await?;
                        requests.push(read_http_request_text(&mut socket).await?);
                        socket.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://origin.test\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: x-probe\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await?;
                    }
                    anyhow::Ok(requests)
                });
                let headers = vec![("X-Probe".to_owned(), "preflight".to_owned())];
                let request = Request::new_browser_bytes(
                    "GET",
                    &url,
                    None,
                    headers.clone(),
                    (&initiator).into(),
                )?
                .with_initiator_url(&initiator)
                .with_credentials_mode(RequestCredentialsMode::SameOrigin)
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
                .with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
                    referrer_policy: policy.map(str::to_owned),
                    document_referrer_policy: document_policy.map(str::to_owned),
                    ..Default::default()
                })
                .with_fetch_referrer(referrer)?;
                let mut config = FetchConfig::default();
                config.set_http_no_proxy(Some("*".to_owned()));
                let owner = ResourceRequestClient::new(&config)?;
                let loader = owner.handle();
                let (head, body) = if streaming {
                    fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(&loader, request, None, headers)
                        .await.map_err(anyhow::Error::msg)?.into_response().into_body()
                } else {
                    fetch_browser_subresource_with_preflight_and_network_metadata(
                        loader, request, None,
                    )
                    .await
                    .map_err(anyhow::Error::msg)?
                    .into_response()
                    .into_body()
                };
                assert_eq!(head.status, 200);
                assert_eq!(body.into_materialized_bytes().await?, b"ok");
                let requests = tokio::time::timeout(Duration::from_secs(5), server).await???;
                for (request, method) in requests.iter().zip(["OPTIONS", "GET"]) {
                    assert!(request.starts_with(&format!("{method} /echo HTTP/1.1\r\n")));
                    let header = |wanted: &str| {
                        request
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                            .map(|(_, value)| value.trim())
                    };
                    assert_eq!(
                        header("referer"),
                        expected,
                        "{method}, {referrer}, {policy:?}, {document_policy:?}, streaming={streaming}"
                    );
                    assert_eq!(header("origin"), Some("http://origin.test"));
                    assert_eq!(header("sec-fetch-site"), Some("cross-site"));
                }
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cors_preflight_uses_streaming_head_response_without_a_referrer_url() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut navigation, _) = listener.accept().await.unwrap();
            let initial = read_http_request_text(&mut navigation).await.unwrap();
            assert!(initial.starts_with("GET / HTTP/1.1"));
            assert!(
                initial
                    .to_ascii_lowercase()
                    .contains("\r\ncookie: session=literal\r\n")
            );
            navigation
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            drop(navigation);
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
                !preflight_request
                    .to_ascii_lowercase()
                    .contains("\r\ncookie:"),
                "preflight must omit configured cookie literals: {preflight_request}"
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
                    "Access-Control-Allow-Credentials: true\r\n",
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
                    .contains("\r\ncookie: session=literal\r\n"),
                "the credentialed request must retain the configured cookie: {actual_request}"
            );
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
                    "Access-Control-Allow-Credentials: true\r\n",
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

        let mut config = FetchConfig::default();
        config.set_additional_cookies(Some("session=literal".to_owned()));
        let loader_owner = ResourceRequestClient::new(&config)?;
        let loader = loader_owner.handle();
        loader
            .fetch(
                Request::get(&format!("http://{addr}/"))?
                    .with_request_origin(moli_url::WebOrigin::Opaque),
            )
            .await?;
        let request = Request::new(
            "PUT",
            &format!("http://{addr}/resource"),
            None,
            vec![("X-Test".to_owned(), "yes".to_owned())],
        )?
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
            "http://origin.test/page",
        )?))
        .with_credentials_mode(RequestCredentialsMode::Include)
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
    async fn preflight_body_cancellation_does_not_commit_or_cancel_the_final_response() -> Result<()>
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (send_tail_tx, send_tail_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut preflight, _) = listener.accept().await?;
            assert!(
                read_http_request_text(&mut preflight)
                    .await?
                    .starts_with("OPTIONS ")
            );
            preflight
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Access-Control-Allow-Origin: http://origin.test\r\n",
                        "Access-Control-Allow-Methods: PUT\r\n",
                        "Access-Control-Allow-Headers: x-test\r\n",
                        "Content-Length: 100\r\n",
                        "Connection: close\r\n\r\n",
                    )
                    .as_bytes(),
                )
                .await?;

            // Keep the preflight body unfinished until the actual request is
            // received. Waiting for that body would deadlock this exchange.
            let (mut actual, _) = listener.accept().await?;
            assert!(
                read_http_request_text(&mut actual)
                    .await?
                    .starts_with("PUT ")
            );
            let mut byte = [0_u8; 1];
            assert_eq!(preflight.read(&mut byte).await?, 0);
            actual
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Access-Control-Allow-Origin: http://origin.test\r\n",
                        "Content-Length: 4\r\n",
                        "Connection: close\r\n\r\n",
                        "bo",
                    )
                    .as_bytes(),
                )
                .await?;
            send_tail_rx.await?;
            actual.write_all(b"dy").await?;
            Ok::<_, anyhow::Error>(())
        });

        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let request_headers = vec![("X-Test".to_owned(), "yes".to_owned())];
        let request = Request::new(
            "PUT",
            &format!("http://{addr}/resource"),
            None,
            request_headers.clone(),
        )?
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
            "http://origin.test/page",
        )?))
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let cancel = FetchCancelHandle::new();
        let observed = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                &loader,
                request,
                Some(cancel.clone()),
                request_headers,
            ),
        )
        .await?
        .map_err(anyhow::Error::msg)?;
        let mut response = observed.into_response();
        assert_eq!(response.status, 200);
        assert!(!cancel.is_cancelled());
        assert!(!cancel.response_completion_is_committed());

        send_tail_tx.send(()).unwrap();
        let mut body = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            body.extend_from_slice(&chunk);
        }
        response.finish().await?;
        assert_eq!(body, b"body");
        assert!(cancel.response_completion_is_committed());
        assert!(!cancel.cancel_if_response_pending());
        assert!(!cancel.is_cancelled());
        server.await??;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn discarded_redirect_and_preflight_bodies_do_not_block_final_response_cancellation()
    -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let origin = format!("http://127.0.0.1:{}", addr.port());
        let target = format!("http://localhost:{}/final", addr.port());
        let server_origin = origin.clone();
        let server_target = target.clone();
        let server = tokio::spawn(async move {
            let (mut redirect, _) = listener.accept().await?;
            assert!(
                read_http_request_text(&mut redirect)
                    .await?
                    .starts_with("GET /start ")
            );
            redirect
                .write_all(
                    format!(
                        concat!(
                            "HTTP/1.1 307 Temporary Redirect\r\n",
                            "Location: {}\r\n",
                            "Content-Length: 100\r\n",
                            "Connection: close\r\n\r\n",
                        ),
                        server_target
                    )
                    .as_bytes(),
                )
                .await?;

            let (mut preflight, _) = listener.accept().await?;
            assert!(
                read_http_request_text(&mut preflight)
                    .await?
                    .starts_with("OPTIONS /final ")
            );
            preflight
                .write_all(
                    format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Access-Control-Allow-Origin: {}\r\n",
                            "Access-Control-Allow-Methods: GET\r\n",
                            "Access-Control-Allow-Headers: x-test\r\n",
                            "Content-Length: 100\r\n",
                            "Connection: close\r\n\r\n",
                        ),
                        server_origin
                    )
                    .as_bytes(),
                )
                .await?;

            let (mut actual, _) = listener.accept().await?;
            assert!(
                read_http_request_text(&mut actual)
                    .await?
                    .starts_with("GET /final ")
            );
            let mut byte = [0_u8; 1];
            assert_eq!(redirect.read(&mut byte).await?, 0);
            assert_eq!(preflight.read(&mut byte).await?, 0);
            actual
                .write_all(
                    format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Access-Control-Allow-Origin: {}\r\n",
                            "Content-Length: 4\r\n",
                            "Connection: close\r\n\r\n",
                            "bo",
                        ),
                        server_origin
                    )
                    .as_bytes(),
                )
                .await?;
            assert_eq!(actual.read(&mut byte).await?, 0);
            Ok::<_, anyhow::Error>(())
        });

        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let document = Url::parse(&format!("{origin}/page"))?;
        let headers = vec![("X-Test".to_owned(), "yes".to_owned())];
        let request = Request::new("GET", &format!("{origin}/start"), None, headers.clone())?
            .with_initiator_url(&document)
            .with_request_origin(moli_url::WebOrigin::from_url(&document))
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let cancel = FetchCancelHandle::new();
        let observed = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_browser_subresource_raw_stream_with_manual_preflight_redirects(
                &loader,
                request,
                Some(cancel.clone()),
                headers,
                None,
            ),
        )
        .await?
        .map_err(anyhow::Error::msg)?;
        let mut response = observed.into_response();
        assert_eq!(response.final_url.as_str(), target);
        assert_eq!(response.redirect_chain.len(), 1);
        assert!(!cancel.is_cancelled());
        assert!(!cancel.response_completion_is_committed());
        assert!(cancel.cancel_if_response_pending());
        assert!(response.cancellation_handle().is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), response.finish())
                .await?
                .is_err()
        );
        assert!(!cancel.response_completion_is_committed());
        tokio::time::timeout(Duration::from_secs(5), server).await???;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn buffered_xhr_discards_unfinished_redirect_and_preflight_bodies() -> Result<()> {
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
            let mut request_body = [0_u8; 7];
            redirect.read_exact(&mut request_body).await.unwrap();
            assert_eq!(&request_body, b"payload");
            let redirect_response = format!(
                concat!(
                    "HTTP/1.1 307 Temporary Redirect\r\n",
                    "Location: {}\r\n",
                    "Content-Length: 100\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                target_url_for_server
            );
            redirect
                .write_all(redirect_response.as_bytes())
                .await
                .unwrap();
            // Neither body is sent. The next requests must start from headers.

            let (mut preflight, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut preflight).await.unwrap();
            let request_lower = request.to_ascii_lowercase();
            assert!(request.starts_with("OPTIONS /final HTTP/1.1"));
            assert!(request_lower.contains("access-control-request-method: post"));
            assert!(request_lower.contains("access-control-request-headers: x-challenge"));
            let preflight_response = format!(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Access-Control-Allow-Origin: {}\r\n",
                    "Access-Control-Allow-Methods: POST\r\n",
                    "Access-Control-Allow-Headers: x-challenge\r\n",
                    "Content-Length: 100\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                ),
                source_origin_for_server
            );
            preflight
                .write_all(preflight_response.as_bytes())
                .await
                .unwrap();

            let (mut final_response, _) = listener.accept().await.unwrap();
            let request = read_http_request_text(&mut final_response).await.unwrap();
            assert!(request.starts_with("POST /final HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("x-challenge: yes"));
            final_response.read_exact(&mut request_body).await.unwrap();
            assert_eq!(&request_body, b"payload");
            let mut byte = [0_u8; 1];
            assert_eq!(redirect.read(&mut byte).await.unwrap(), 0);
            assert_eq!(preflight.read(&mut byte).await.unwrap(), 0);
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
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
        // A later policy setter re-enables runtime following. The browser loop
        // must reapply single-hop handling at dispatch to preflight the next URL.
        .with_single_hop()
        .with_redirect_mode(RequestRedirectMode::Follow);
        assert!(request.transport_follows_redirects());

        let cancel = FetchCancelHandle::new();
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_browser_subresource_with_preflight_headers(
                loader,
                request,
                Some(cancel.clone()),
                request_headers,
            ),
        )
        .await?
        .map_err(anyhow::Error::msg)?;
        assert_eq!(response.final_url.as_str(), target_url);
        assert_eq!(response.body_text(), "buffered-xhr");
        assert!(response.redirected);
        assert_eq!(response.redirect_chain.len(), 1);
        assert_eq!(response.redirect_chain[0].from_url, request_url);
        assert_eq!(response.redirect_chain[0].to_url.as_str(), target_url);
        assert_eq!(response.redirect_chain[0].status, 307);
        assert!(cancel.response_completion_is_committed());
        assert!(!cancel.cancel_if_response_pending());
        assert!(!cancel.is_cancelled());

        tokio::time::timeout(Duration::from_secs(5), server).await??;
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
            AsyncSubresourceNetworkContext {
                frame_id: Some("FRAME-1".to_owned()),
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url: document_url.clone(),
                resource_type: SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            request_url.clone(),
            "GET".to_owned(),
            request_headers.into(),
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
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url,
                resource_type: SubresourceResourceType::Image,
                policy_context: Default::default(),
            },
            request_url,
            "GET".to_owned(),
            request_headers.clone(),
            None,
        );

        let AsyncSubresourceFetchEvent::Completion(completion) =
            next_async_subresource_event(&mut queue).await?
        else {
            anyhow::bail!("image transport must emit one buffered terminal completion");
        };
        assert_eq!(completion.internal_id, 75);
        assert_eq!(completion.request_headers, request_headers);
        let response = completion
            .result
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        assert_eq!(response.status, 200);
        assert!(response.body_bytes().is_empty());
        let encoded = completion
            .result
            .encoded()
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(
                    &(Url::parse(&format!("http://{addr}/page"))?),
                ),
                document_url: Url::parse(&format!("http://{addr}/page"))?,
                resource_type: SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            target_url.clone(),
            "GET".to_owned(),
            Vec::new().into(),
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(
                    &(Url::parse("http://origin.test/page")?),
                ),
                document_url: Url::parse("http://origin.test/page")?,
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url,
            "GET".to_owned(),
            Vec::new().into(),
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url,
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url,
            "POST".to_owned(),
            request_headers.into(),
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
            AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url: document_url.clone(),
                resource_type: SubresourceResourceType::Xhr,
                policy_context: Default::default(),
            },
            request_url.clone(),
            "POST".to_owned(),
            request_headers.into(),
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

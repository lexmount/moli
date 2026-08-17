use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerDirectFetchResponse, ServiceWorkerFetchRequest, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::types::{
    AsyncSubresourceFetchEvent, AsyncSubresourceFetchResponseFilter,
    AsyncSubresourceStreamingChunk, AsyncSubresourceStreamingFinished,
    AsyncSubresourceStreamingStarted,
};

const MAX_SERVICE_WORKER_SYNTHETIC_REDIRECTS: usize = 20;

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn service_worker_redirect_target(
    job: &ServiceWorkerFetchJob,
    response: &ServiceWorkerFetchResponse,
) -> Result<Option<Url>, String> {
    if !is_redirect_status(response.status) {
        return Ok(None);
    }
    let Some(location) = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.as_str())
    else {
        return Ok(None);
    };
    if job.redirect_count >= MAX_SERVICE_WORKER_SYNTHETIC_REDIRECTS {
        return Err(format!(
            "redirect limit exceeded for {}",
            response.final_url.as_ref().unwrap_or(&job.request_url)
        ));
    }
    if let Ok(url) = Url::parse(location) {
        return Ok(Some(url));
    }
    let Some(base_url) = response.final_url.as_ref() else {
        return Err(format!(
            "failed to resolve redirect location `{location}` for a generated response without a response URL"
        ));
    };
    base_url.join(location).map(Some).map_err(|error| {
        format!("failed to resolve redirect location `{location}` from {base_url}: {error}")
    })
}

fn fetch_request_for_job(job: &ServiceWorkerFetchJob) -> ServiceWorkerFetchRequest {
    ServiceWorkerFetchRequest {
        client_id: job.client_id,
        resulting_client_id: job.resulting_client_id,
        url: job.request_url.clone(),
        method: job.request_method.clone(),
        headers: job.request_headers.clone(),
        body: job.request_body_bytes.clone(),
        destination: job.destination,
        request_mode: job.request_mode,
        credentials_mode: job.credentials_mode,
        redirect_mode: job.redirect_mode,
        priority: job.priority,
        is_reload: job.is_reload,
        metadata: job.metadata.clone(),
    }
}

fn service_worker_network_fallback_request_for_job(
    job: &ServiceWorkerFetchJob,
) -> Result<moli_fetch::Request, String> {
    let request = moli_fetch::Request::new_bytes(
        &job.request_method,
        job.request_url.as_str(),
        job.request_body_bytes.clone(),
        job.request_headers.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(configure_service_worker_network_fallback_request(
        job, request,
    ))
}

fn configure_service_worker_network_fallback_request(
    job: &ServiceWorkerFetchJob,
    request: moli_fetch::Request,
) -> moli_fetch::Request {
    let mut request = request
        .with_initiator_url(&job.network_context.document_url)
        .with_request_mode(job.request_mode)
        .with_credentials_mode(job.credentials_mode)
        .with_redirect_mode(if service_worker_fetch_is_navigation_request(job) {
            moli_fetch::RequestRedirectMode::Follow
        } else {
            job.redirect_mode
        })
        .with_fetch_priority_hint(job.priority)
        .with_browser_request_metadata(service_worker_fetch_browser_metadata(
            job.network_context.resource_type,
        ))
        .with_subframe_context(job.network_context.frame_id.is_some());
    if service_worker_fetch_is_navigation_request(job) {
        request = if job.network_context.frame_id.is_some() {
            request.with_subframe_navigation_cookie_context()
        } else {
            request.with_top_level_navigation_cookie_context()
        };
    }
    if job.network_context.resource_type == crate::types::SubresourceResourceType::EventSource {
        request = request
            .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
            .without_request_timeout();
    }
    request
}

fn service_worker_fetch_stream_response_head(
    job: &ServiceWorkerFetchJob,
    response_head: &MaterializedServiceWorkerFetchResponseHead,
) -> moli_fetch::ResponseHead {
    moli_fetch::ResponseHead {
        final_url: response_head
            .final_url
            .clone()
            .unwrap_or_else(|| job.request_url.clone()),
        status: response_head.status,
        headers: response_head.headers.clone(),
        request_cookie_report: job.request_cookie_report.clone(),
        cookie_set_reports: Vec::new(),
        redirected: response_head.redirected || !job.redirect_chain.is_empty(),
        redirect_chain: job.redirect_chain.clone(),
        from_cache: false,
        negotiated_http_version: None,
    }
}

fn service_worker_fetch_can_forward_stream(
    job: &ServiceWorkerFetchJob,
    response_head: &MaterializedServiceWorkerFetchResponseHead,
) -> Result<bool, String> {
    let final_url = response_head.final_url.as_ref().unwrap_or(&job.request_url);
    validate_service_worker_fetch_response_head_security_policy(
        job,
        final_url,
        &response_head.headers,
    )?;
    Ok(job.direct_completion_tx.is_none()
        && !matches!(
            job.network_context.resource_type,
            crate::types::SubresourceResourceType::Audio
                | crate::types::SubresourceResourceType::Font
                | crate::types::SubresourceResourceType::Image
                | crate::types::SubresourceResourceType::Media
                | crate::types::SubresourceResourceType::TextTrack
                | crate::types::SubresourceResourceType::Video
        )
        && !is_redirect_status(response_head.status)
        && matches!(response_head.response_type.as_str(), "default" | "basic")
        && !service_worker_fetch_response_requires_body_security_policy(job))
}

fn apply_service_worker_synthetic_redirect(
    job: &mut ServiceWorkerFetchJob,
    response: ServiceWorkerFetchResponse,
    next_url: Url,
) -> Result<(), String> {
    let from_url = response
        .final_url
        .unwrap_or_else(|| job.request_url.clone());
    job.redirect_chain.push(moli_fetch::RedirectInfo {
        from_url,
        to_url: next_url.clone(),
        status: response.status,
        headers: response.headers,
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: job.request_cookie_report.take(),
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    });
    job.redirect_count += 1;

    let mut request = moli_fetch::Request::new_bytes(
        &job.request_method,
        job.request_url.as_str(),
        job.request_body_bytes.clone(),
        job.request_headers.clone(),
    )
    .map_err(|error| error.to_string())?;
    request.apply_redirect_status(response.status);
    job.request_method = request.method;
    job.request_headers = request.request_headers;
    job.request_body_bytes = request.body;
    job.request_body = job
        .request_body_bytes
        .as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned());
    job.cors_preflight_request_headers = job.request_headers.clone();
    job.request_url = next_url;
    Ok(())
}

impl ServiceWorkerRuntimeService {
    pub(crate) async fn fetch_main_resource_for_worker_client(
        &self,
        client_id: ServiceWorkerClientId,
        request: &moli_fetch::Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: crate::network::RendererResourceTaskRunner,
        destination: ServiceWorkerRequestDestination,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) -> Result<Option<crate::protocol_types::NavigationResponse>, String> {
        if !matches!(request.url.scheme(), "http" | "https") {
            return Ok(None);
        }
        if self
            .matching_controller_for_client_fetch(client_id, &request.url)
            .is_none()
        {
            return Ok(None);
        }

        let completion_tx =
            crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only();
        let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
        let request_body_text = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id: 0,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: Some(client_id),
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.clone(),
                body: request.body.clone(),
                destination,
                request_mode: request.request_mode,
                credentials_mode: request.credentials_mode,
                redirect_mode: request.redirect_mode,
                priority: request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(request),
            },
            request_body_text,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: request.url.clone(),
                resource_type: crate::types::SubresourceResourceType::Script,
                policy_context: Default::default(),
            },
            completion_tx,
            request_client: request_client.clone(),
            resource_task_runner,
            cancel_handle,
            direct_completion_tx: Some(direct_completion_tx),
        };

        if !self.dispatch_controlled_fetch(dispatch) {
            return Ok(None);
        }

        match direct_completion_rx.await {
            Ok(ServiceWorkerDirectFetchResult::Fallback) => Ok(None),
            Ok(ServiceWorkerDirectFetchResult::Response(response)) => Ok(Some(*response.response)),
            Ok(ServiceWorkerDirectFetchResult::Failure(message)) => Err(message),
            Err(_) => Err(
                "service worker worker main resource fetch completion channel closed".to_owned(),
            ),
        }
    }

    pub(super) fn dispatch_fetch_fallback(&self, mut job: ServiceWorkerFetchJob) {
        job.cancel_pending_navigation_preload();
        if let Some(completion_tx) = job.direct_completion_tx.take() {
            if job.redirect_count != 0 {
                self.dispatch_direct_fetch_network_fallback(job, completion_tx);
                return;
            }
            let _ = completion_tx.send(ServiceWorkerDirectFetchResult::Fallback);
            return;
        }
        let request_client = job.request_client.clone();
        let request = match moli_fetch::Request::new_bytes(
            &job.request_method,
            job.request_url.as_str(),
            job.request_body_bytes.clone(),
            job.request_headers.clone(),
        ) {
            Ok(request) => configure_service_worker_network_fallback_request(&job, request),
            Err(error) => {
                self.complete_fetch_with_failure(job, error.to_string());
                return;
            }
        };
        crate::network_host::spawn_async_subresource_fetch_with_redirect_chain(
            job.resource_task_runner.clone(),
            job.completion_tx,
            request_client,
            request,
            Some(job.cancel_handle),
            job.cors_preflight_request_headers,
            job.redirect_chain,
            job.internal_id,
            job.network_context,
            job.request_url,
            job.request_method,
            job.request_headers,
            job.request_body,
        );
    }

    fn dispatch_direct_fetch_network_fallback(
        &self,
        job: ServiceWorkerFetchJob,
        completion_tx: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    ) {
        let request_client = job.request_client.clone();
        let request = match service_worker_network_fallback_request_for_job(&job) {
            Ok(request) => request,
            Err(message) => {
                let _ = completion_tx.send(ServiceWorkerDirectFetchResult::Failure(message));
                return;
            }
        };
        let cancel_handle = job.cancel_handle.clone();
        let redirect_chain = job.redirect_chain.clone();
        job.resource_task_runner.spawn(async move {
            let result = match request_client
                .fetch_raw_stream_with_cancel(request, cancel_handle)
                .await
            {
                Ok(response) => response.into_materialized_raw_response().await,
                Err(error) => Err(error),
            };
            let result = match result {
                Ok(response) => {
                    let mut head = response.head();
                    if !redirect_chain.is_empty() {
                        let mut combined_redirect_chain = redirect_chain;
                        combined_redirect_chain.extend(head.redirect_chain);
                        head.redirect_chain = combined_redirect_chain;
                        head.redirected = true;
                    }
                    let body = response.clone_body_bytes();
                    let navigation_response =
                        crate::protocol_types::NavigationResponse::from_head_and_body(
                            head,
                            String::from_utf8_lossy(&body).into_owned(),
                            body,
                        );
                    ServiceWorkerDirectFetchResult::Response(ServiceWorkerDirectFetchResponse {
                        response: Box::new(navigation_response),
                        response_filter: None,
                    })
                }
                Err(error) => ServiceWorkerDirectFetchResult::Failure(error.to_string()),
            };
            let _ = completion_tx.send(result);
        });
    }

    pub(super) fn finish_fetch_stream_started(&self, started: ServiceWorkerFetchStreamStarted) {
        let stream_rejection = {
            let state = self.inner.state.lock();
            state
                .pending_fetch_jobs
                .get(&started.event_id)
                .and_then(|job| {
                    if job.version_id != started.version_id || !job.is_bound_to_run(&started.run) {
                        return None;
                    }
                    service_worker_fetch_can_forward_stream(job, &started.response_head).err()
                })
        };
        if let Some(message) = stream_rejection {
            self.reject_fetch_stream_started(started, message);
            return;
        }

        let Some((completion_tx, streaming_started)) = ({
            let mut state = self.inner.state.lock();
            let Some(job) = state.pending_fetch_jobs.get_mut(&started.event_id) else {
                return;
            };
            if job.version_id != started.version_id || !job.is_bound_to_run(&started.run) {
                return;
            }
            match service_worker_fetch_can_forward_stream(job, &started.response_head) {
                Ok(true) => {}
                Ok(false) => {
                    return;
                }
                Err(_) => {
                    return;
                }
            }
            job.streaming_body_source_id = Some(started.body_source_id);
            Some((
                job.completion_tx.clone(),
                AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
                    AsyncSubresourceStreamingStarted {
                        internal_id: job.internal_id,
                        request_url: job.request_url.clone(),
                        request_method: job.request_method.clone(),
                        request_headers: job.request_headers.clone(),
                        request_body: job.request_body.clone(),
                        body_source_id: started.body_source_id,
                        network_request_headers: None,
                        head: service_worker_fetch_stream_response_head(
                            job,
                            &started.response_head,
                        ),
                    },
                )),
            ))
        }) else {
            return;
        };
        let _ = completion_tx.send_async_subresource_event(streaming_started);
    }

    fn reject_fetch_stream_started(
        &self,
        started: ServiceWorkerFetchStreamStarted,
        message: String,
    ) {
        let rejected = {
            let mut state = self.inner.state.lock();
            {
                let Some(version) = state.versions.get(&started.version_id) else {
                    return;
                };
                if version.run != started.run {
                    return;
                }
            }
            let Some(job) = state.pending_fetch_jobs.remove(&started.event_id) else {
                return;
            };
            if job.version_id != started.version_id || !job.is_bound_to_run(&started.run) {
                return;
            }
            let stream_cancel = state.versions.get(&job.version_id).and_then(|version| {
                if !job.is_bound_to_run(&version.run) {
                    return None;
                }
                let ServiceWorkerVersionRunningState::Running { host } = &version.running_state
                else {
                    return None;
                };
                (host.version_id() == job.version_id && job.is_bound_to_run(&host.run_identity()))
                    .then_some((host.clone(), started.event_id, started.body_source_id))
            });
            if let Some(version) = state.versions.get_mut(&started.version_id) {
                version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            }
            job.cancel_handle.cancel();
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                started.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    started.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, started.version_id);
            let mut lifecycle_progress = unregistration_progress;
            lifecycle_progress.extend(activation_progress);
            Some((job, idle_timeout, lifecycle_progress, stream_cancel))
        };
        let Some((job, idle_timeout, lifecycle_progress, stream_cancel)) = rejected else {
            return;
        };
        if let Some((host, event_id, body_source_id)) = stream_cancel {
            host.cancel_fetch_stream(event_id, body_source_id);
        }
        let result = ServiceWorkerFetchResult::Failure(message.clone());
        let diagnostic =
            super::event_completion::service_worker_fetch_diagnostic_from_job_result(&job, &result);
        self.enqueue_target_fetch_diagnostic(started.version_id, started.run, diagnostic);
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
        self.complete_fetch_with_failure(job, message);
    }

    pub(super) fn finish_fetch_stream_chunk(&self, chunk: ServiceWorkerFetchStreamChunk) {
        let Some(completion_tx) = ({
            let state = self.inner.state.lock();
            state
                .pending_fetch_jobs
                .get(&chunk.event_id)
                .and_then(|job| {
                    (job.streaming_body_source_id == Some(chunk.body_source_id))
                        .then(|| job.completion_tx.clone())
                })
        }) else {
            return;
        };
        let _ = completion_tx.send_async_subresource_event(
            AsyncSubresourceFetchEvent::StreamingChunk(AsyncSubresourceStreamingChunk {
                body_source_id: chunk.body_source_id,
                bytes: chunk.bytes,
            }),
        );
    }

    pub(super) fn complete_fetch_with_service_worker_response(
        &self,
        mut job: ServiceWorkerFetchJob,
        response: ServiceWorkerFetchResponse,
    ) {
        job.cancel_pending_navigation_preload();
        if is_redirect_status(response.status)
            && response.response_type == "opaqueredirect"
            && service_worker_fetch_is_navigation_request(&job)
        {
            match service_worker_redirect_target(&job, &response) {
                Ok(Some(next_url)) => {
                    if let Err(error) =
                        apply_service_worker_synthetic_redirect(&mut job, response, next_url)
                    {
                        self.complete_fetch_with_failure(job, error);
                        return;
                    }
                    let request = fetch_request_for_job(&job);
                    if let Err(job) = self.dispatch_controlled_fetch_job(job, request) {
                        self.dispatch_fetch_fallback(*job);
                    }
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    self.complete_fetch_with_network_failure(
                        job,
                        error,
                        crate::network_host::FAILED_ERROR_TEXT.to_owned(),
                    );
                    return;
                }
            }
        }
        if is_redirect_status(response.status)
            && job.redirect_mode != moli_fetch::RequestRedirectMode::Manual
        {
            match service_worker_redirect_target(&job, &response) {
                Ok(Some(next_url)) => match job.redirect_mode {
                    moli_fetch::RequestRedirectMode::Error => {
                        self.complete_fetch_with_network_failure(
                            job,
                            format!(
                                "FetchEvent.respondWith rejected a redirect Response for a request whose redirect mode is error: {next_url}"
                            ),
                            crate::network_host::FAILED_ERROR_TEXT.to_owned(),
                        );
                        return;
                    }
                    moli_fetch::RequestRedirectMode::Manual => {}
                    moli_fetch::RequestRedirectMode::Follow => {
                        if let Err(error) =
                            apply_service_worker_synthetic_redirect(&mut job, response, next_url)
                        {
                            self.complete_fetch_with_failure(job, error);
                            return;
                        }
                        let request = fetch_request_for_job(&job);
                        if let Err(job) = self.dispatch_controlled_fetch_job(job, request) {
                            self.dispatch_fetch_fallback(*job);
                        }
                        return;
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    self.complete_fetch_with_network_failure(
                        job,
                        error,
                        crate::network_host::FAILED_ERROR_TEXT.to_owned(),
                    );
                    return;
                }
            }
        }
        if let Some(message) = service_worker_fetch_response_rejection(&job, &response) {
            self.complete_fetch_with_network_failure(
                job,
                message,
                crate::network_host::FAILED_ERROR_TEXT.to_owned(),
            );
            return;
        }
        let final_url = response
            .final_url
            .clone()
            .unwrap_or_else(|| job.request_url.clone());
        if let Err(message) =
            validate_service_worker_fetch_response_security_policy(&job, &response, &final_url)
        {
            self.complete_fetch_with_failure(job, message);
            return;
        }
        let response_filter = service_worker_fetch_response_filter(&response);
        let navigation_response = crate::protocol_types::NavigationResponse::from_head_and_body(
            moli_fetch::ResponseHead {
                final_url,
                status: response.status,
                headers: response.headers,
                request_cookie_report: job.request_cookie_report,
                cookie_set_reports: Vec::new(),
                redirected: response.redirected || !job.redirect_chain.is_empty(),
                redirect_chain: job.redirect_chain,
                from_cache: false,
                negotiated_http_version: None,
            },
            String::from_utf8_lossy(&response.body).into_owned(),
            response.body,
        );
        if let Some(completion_tx) = job.direct_completion_tx.take() {
            let _ = completion_tx.send(ServiceWorkerDirectFetchResult::Response(
                ServiceWorkerDirectFetchResponse {
                    response: Box::new(navigation_response),
                    response_filter,
                },
            ));
            return;
        }
        if let Some(body_source_id) = job.streaming_body_source_id.take() {
            let _ = job.completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::StreamingFinished(AsyncSubresourceStreamingFinished {
                    internal_id: job.internal_id,
                    body_source_id,
                    result: Ok(()),
                }),
            );
            return;
        }
        let _ = job
            .completion_tx
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id: job.internal_id,
                request_url: job.request_url,
                request_method: job.request_method,
                request_headers: job.request_headers,
                request_body: job.request_body,
                response_status_text: Some(response.status_text),
                skip_fetch_security_validation: true,
                response_filter,
                network_error_text: None,
                result: Ok(navigation_response),
            });
    }

    pub(super) fn complete_fetch_with_failure(&self, job: ServiceWorkerFetchJob, message: String) {
        self.complete_fetch_with_failure_inner(job, message, None);
    }

    fn complete_fetch_with_network_failure(
        &self,
        job: ServiceWorkerFetchJob,
        message: String,
        network_error_text: String,
    ) {
        self.complete_fetch_with_failure_inner(job, message, Some(network_error_text));
    }

    fn complete_fetch_with_failure_inner(
        &self,
        mut job: ServiceWorkerFetchJob,
        message: String,
        mut network_error_text: Option<String>,
    ) {
        job.cancel_pending_navigation_preload();
        if network_error_text.is_none() {
            network_error_text = service_worker_fetch_failure_network_error_text(&message);
        }
        if let Some(completion_tx) = job.direct_completion_tx.take() {
            let _ = completion_tx.send(ServiceWorkerDirectFetchResult::Failure(message));
            return;
        }
        if let Some(body_source_id) = job.streaming_body_source_id.take() {
            let _ = job.completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::StreamingFinished(AsyncSubresourceStreamingFinished {
                    internal_id: job.internal_id,
                    body_source_id,
                    result: Err(message),
                }),
            );
            return;
        }
        let _ = job
            .completion_tx
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id: job.internal_id,
                request_url: job.request_url,
                request_method: job.request_method,
                request_headers: job.request_headers,
                request_body: job.request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text,
                result: Err(message),
            });
    }
}

fn service_worker_fetch_failure_network_error_text(message: &str) -> Option<String> {
    if matches_service_worker_respond_with_fetch_failure(message) {
        Some(crate::network_host::FAILED_ERROR_TEXT.to_owned())
    } else {
        None
    }
}

fn matches_service_worker_respond_with_fetch_failure(message: &str) -> bool {
    message.starts_with("FetchEvent.respondWith promise rejected:")
        || message == "FetchEvent.respondWith failed to attach promise reactions"
        || message == "FetchEvent.respondWith failed to attach response body reactions"
        || message.starts_with("FetchEvent.respondWith failed to materialize Response body:")
        || message.starts_with("FetchEvent.respondWith rejected ")
        || message.starts_with("FetchEvent.respondWith requires ")
        || message == "FetchEvent.respondWith Response has an invalid URL."
        || message == "FetchEvent.respondWith failed to materialize Response body bytes."
        || message == "FetchEvent.respondWith ReadableStream body chunks must be Uint8Array."
        || message.starts_with("failed to resolve redirect location `")
}

fn service_worker_fetch_browser_metadata(
    resource_type: crate::types::SubresourceResourceType,
) -> moli_fetch::BrowserRequestMetadata {
    match resource_type {
        crate::types::SubresourceResourceType::Audio => moli_fetch::BrowserRequestMetadata::Audio,
        crate::types::SubresourceResourceType::Image => moli_fetch::BrowserRequestMetadata::Image,
        crate::types::SubresourceResourceType::Font => moli_fetch::BrowserRequestMetadata::Font,
        crate::types::SubresourceResourceType::TextTrack => {
            moli_fetch::BrowserRequestMetadata::TextTrack
        }
        crate::types::SubresourceResourceType::Video => moli_fetch::BrowserRequestMetadata::Video,
        crate::types::SubresourceResourceType::EventSource => {
            moli_fetch::BrowserRequestMetadata::EventSource
        }
        crate::types::SubresourceResourceType::Manifest => {
            moli_fetch::BrowserRequestMetadata::Manifest
        }
        crate::types::SubresourceResourceType::Xhr => moli_fetch::BrowserRequestMetadata::Xhr,
        _ => moli_fetch::BrowserRequestMetadata::Fetch,
    }
}

fn service_worker_fetch_is_navigation_request(job: &ServiceWorkerFetchJob) -> bool {
    matches!(
        job.destination,
        ServiceWorkerRequestDestination::Document | ServiceWorkerRequestDestination::Iframe
    )
}

fn validate_service_worker_fetch_response_security_policy(
    job: &ServiceWorkerFetchJob,
    response: &ServiceWorkerFetchResponse,
    final_url: &Url,
) -> Result<(), String> {
    validate_service_worker_fetch_response_body_security_policy(job, response, final_url)?;
    validate_service_worker_fetch_response_head_security_policy(job, final_url, &response.headers)
}

fn service_worker_fetch_response_requires_body_security_policy(
    job: &ServiceWorkerFetchJob,
) -> bool {
    job.request_mode == moli_fetch::RequestMode::NoCors
        && matches!(
            job.network_context.resource_type,
            crate::types::SubresourceResourceType::Fetch
                | crate::types::SubresourceResourceType::Xhr
        )
}

fn validate_service_worker_fetch_response_body_security_policy(
    job: &ServiceWorkerFetchJob,
    response: &ServiceWorkerFetchResponse,
    final_url: &Url,
) -> Result<(), String> {
    if job.request_mode != moli_fetch::RequestMode::NoCors
        || !matches!(
            job.network_context.resource_type,
            crate::types::SubresourceResourceType::Fetch
                | crate::types::SubresourceResourceType::Xhr
        )
    {
        return Ok(());
    }

    crate::network_host::validate_fetch_response_security_policy_with_body(
        &job.network_context.document_url,
        final_url,
        &response.headers,
        &response.body,
        job.request_mode,
        job.credentials_mode,
        job.network_context.policy_context,
    )
}

fn validate_service_worker_fetch_response_head_security_policy(
    job: &ServiceWorkerFetchJob,
    final_url: &Url,
    headers: &[(String, String)],
) -> Result<(), String> {
    if job.request_mode != moli_fetch::RequestMode::NoCors
        || matches!(
            job.network_context.resource_type,
            crate::types::SubresourceResourceType::WebSocket
        )
    {
        return Ok(());
    }

    crate::network_host::validate_cross_origin_resource_policy(
        &job.network_context.document_url,
        final_url,
        headers,
    )?;
    crate::network_host::validate_cross_origin_embedder_and_document_isolation_policy(
        &job.network_context.document_url,
        final_url,
        headers,
        job.request_mode,
        job.credentials_mode,
        job.network_context
            .policy_context
            .cross_origin_embedder_policy,
        job.network_context.policy_context.document_isolation_policy,
    )
}

fn service_worker_fetch_response_rejection(
    job: &ServiceWorkerFetchJob,
    response: &ServiceWorkerFetchResponse,
) -> Option<String> {
    match response.response_type.as_str() {
        "error" => {
            return Some("FetchEvent.respondWith rejected an error Response".to_owned());
        }
        "cors" if job.request_mode == moli_fetch::RequestMode::SameOrigin => {
            return Some(
                "FetchEvent.respondWith rejected a cors Response for a same-origin request"
                    .to_owned(),
            );
        }
        "opaque" => {
            if let Some(message) =
                crate::service_worker_runtime::service_worker_opaque_response_rejection(
                    job.request_mode,
                    job.destination,
                )
            {
                return Some(message);
            }
        }
        "opaqueredirect" if job.redirect_mode != moli_fetch::RequestRedirectMode::Manual => {
            return Some(
                "FetchEvent.respondWith rejected an opaqueredirect Response for a request whose redirect mode is not manual"
                    .to_owned(),
            );
        }
        _ => {}
    }
    if response.redirected && job.redirect_mode != moli_fetch::RequestRedirectMode::Follow {
        return Some(
            "FetchEvent.respondWith rejected a redirected Response for a request whose redirect mode is not follow"
                .to_owned(),
        );
    }
    None
}

fn service_worker_fetch_response_filter(
    response: &ServiceWorkerFetchResponse,
) -> Option<AsyncSubresourceFetchResponseFilter> {
    match response.response_type.as_str() {
        "opaque" => Some(AsyncSubresourceFetchResponseFilter::Opaque),
        "opaqueredirect" => Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect),
        _ if is_redirect_status(response.status) => {
            Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_task_queue::RendererResourceCompletionSender;
    use std::collections::{HashMap, HashSet, VecDeque};
    use url::Url;

    fn test_request_client(service: &ServiceWorkerRuntimeServiceOwner) -> ResourceRequestClient {
        service.request_client()
    }

    fn test_resource_task_runner() -> crate::network::RendererResourceTaskRunner {
        crate::network::RendererResourceTaskRunner::for_test()
    }

    fn expect_direct_fetch_fallback(
        receiver: &mut tokio::sync::oneshot::Receiver<ServiceWorkerDirectFetchResult>,
    ) {
        assert!(matches!(
            receiver.try_recv(),
            Ok(ServiceWorkerDirectFetchResult::Fallback)
        ));
    }

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    fn test_launch_config(
        service: &ServiceWorkerRuntimeServiceOwner,
        scope_url: &Url,
    ) -> ServiceWorkerVersionLaunchConfig {
        let browser_context_runtime = service.browser_context_runtime();
        ServiceWorkerVersionLaunchConfig::restored(
            scope_url.clone(),
            browser_context_runtime.worker_context_runtime(),
            service.inner.browser_resource_runtime.clone(),
        )
    }

    fn insert_active_fetch_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
    ) -> Url {
        insert_active_fetch_job_with_redirect_mode(
            service,
            event_id,
            version_id,
            run,
            internal_id,
            completion_tx,
            moli_fetch::RequestRedirectMode::Follow,
        )
    }

    fn insert_active_fetch_job_with_redirect_mode(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
        redirect_mode: moli_fetch::RequestRedirectMode,
    ) -> Url {
        insert_active_fetch_job_with_modes(
            service,
            event_id,
            version_id,
            run,
            internal_id,
            completion_tx,
            moli_fetch::RequestMode::Cors,
            redirect_mode,
        )
    }

    fn insert_active_fetch_job_with_modes(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
        request_mode: moli_fetch::RequestMode,
        redirect_mode: moli_fetch::RequestRedirectMode,
    ) -> Url {
        insert_active_fetch_job_with_modes_and_destination(
            service,
            event_id,
            version_id,
            run,
            internal_id,
            completion_tx,
            request_mode,
            redirect_mode,
            ServiceWorkerRequestDestination::Empty,
        )
    }

    fn insert_active_fetch_job_with_modes_and_resource_type(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
        request_mode: moli_fetch::RequestMode,
        redirect_mode: moli_fetch::RequestRedirectMode,
        resource_type: crate::types::SubresourceResourceType,
    ) -> Url {
        insert_active_fetch_job_with_modes_destination_and_resource_type(
            service,
            event_id,
            version_id,
            run,
            internal_id,
            completion_tx,
            request_mode,
            redirect_mode,
            ServiceWorkerRequestDestination::Empty,
            resource_type,
        )
    }

    fn insert_active_fetch_job_with_modes_and_destination(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
        request_mode: moli_fetch::RequestMode,
        redirect_mode: moli_fetch::RequestRedirectMode,
        destination: ServiceWorkerRequestDestination,
    ) -> Url {
        insert_active_fetch_job_with_modes_destination_and_resource_type(
            service,
            event_id,
            version_id,
            run,
            internal_id,
            completion_tx,
            request_mode,
            redirect_mode,
            destination,
            crate::types::SubresourceResourceType::Fetch,
        )
    }

    fn insert_active_fetch_job_with_modes_destination_and_resource_type(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        internal_id: u64,
        completion_tx: RendererResourceCompletionSender,
        request_mode: moli_fetch::RequestMode,
        redirect_mode: moli_fetch::RequestRedirectMode,
        destination: ServiceWorkerRequestDestination,
        resource_type: crate::types::SubresourceResourceType,
    ) -> Url {
        let registration_id = ServiceWorkerRegistrationId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let client_id = ServiceWorkerClientId::from_u64_for_test(7);
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
            },
        );
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                exposed_id_generation: 0,
                creation_url: document_url.clone(),
                document_url: document_url.clone(),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                secure_context: true,
                execution_ready: true,
                discarded_or_frozen: false,
                document_epoch: Some(0),
                runtime_generation: 0,
                endpoint: ServiceWorkerClientEndpoint::Page(
                    crate::page_task_queue::RendererPageServiceWorkerTestHarness::new().sender(),
                ),
                focused: false,
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                internal_id,
                version_id,
                run: Some(run.clone()),
                request_url: request_url.clone(),
                request_method: "GET".to_owned(),
                request_headers: vec![("accept".to_owned(), "text/plain".to_owned())],
                request_body: None,
                request_body_bytes: None,
                cors_preflight_request_headers: Vec::new(),
                client_id,
                resulting_client_id: None,
                destination,
                is_reload: false,
                metadata: Default::default(),
                request_mode,
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                redirect_mode,
                priority: None,
                redirect_chain: Vec::new(),
                redirect_count: 0,
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url,
                    resource_type,
                    policy_context: Default::default(),
                },
                completion_tx,
                request_client: test_request_client(service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: None,
            },
        );
        request_url
    }

    fn pop_async_subresource_completion(
        queue: &mut crate::page_task_queue::RendererResourceCompletionTestHarness,
    ) -> AsyncSubresourceFetchCompletion {
        match queue.pop_next_async_subresource_event() {
            Some(crate::types::AsyncSubresourceFetchEvent::Completion(completion)) => *completion,
            other => panic!("expected async subresource completion, got {other:?}"),
        }
    }

    fn pop_async_subresource_event(
        queue: &mut crate::page_task_queue::RendererResourceCompletionTestHarness,
    ) -> AsyncSubresourceFetchEvent {
        match queue.pop_next_async_subresource_event() {
            Some(event) => event,
            other => panic!("expected async subresource event, got {other:?}"),
        }
    }

    #[test]
    fn event_source_network_fallback_preserves_long_lived_request_metadata() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(30);
        let completion_queue = crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            ServiceWorkerVersionId(1),
            &RendererServiceWorkerRunIdentity::fresh(),
            310,
            completion_queue.sender(),
            moli_fetch::RequestMode::Cors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::EventSource,
        );
        let job = service
            .inner
            .state
            .lock()
            .pending_fetch_jobs
            .remove(&event_id)
            .expect("EventSource service worker fetch job");

        let request = service_worker_network_fallback_request_for_job(&job)
            .expect("EventSource fallback request");
        assert_eq!(
            request.browser_request_metadata(),
            Some(moli_fetch::BrowserRequestMetadata::EventSource)
        );
        assert_eq!(request.cache_mode(), moli_fetch::RequestCacheMode::NoStore);
    }

    #[test]
    fn response_completion_projects_service_worker_response_to_subresource_queue() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(21);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let request_url = insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            301,
            completion_queue.sender(),
        );
        let final_url = url("https://example.test/app/final.txt");

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(final_url.clone()),
                response_type: "default".to_owned(),
                redirected: false,
                status: 202,
                status_text: "Accepted".to_owned(),
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: b"service-worker-body".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 301);
        assert_eq!(completion.request_url, request_url);
        assert_eq!(completion.request_method, "GET");
        assert_eq!(completion.response_status_text.as_deref(), Some("Accepted"));
        assert!(completion.skip_fetch_security_validation);
        assert_eq!(completion.response_filter, None);
        let response = completion
            .result
            .expect("service worker response should resolve");
        assert_eq!(response.final_url, final_url);
        assert_eq!(response.status, 202);
        assert!(!response.redirected);
        assert_eq!(
            response.headers,
            vec![("content-type".to_owned(), "text/plain".to_owned())]
        );
        assert_eq!(response.body_text(), "service-worker-body");
    }

    #[test]
    fn streaming_fetch_events_project_to_subresource_stream_queue() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(211);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let body_source_id = 77;
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let request_url = insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            301,
            completion_queue.sender(),
        );
        let final_url = url("https://example.test/app/final.txt");

        service.finish_fetch_stream_started(ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run: run.clone(),
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(final_url.clone()),
                response_type: "default".to_owned(),
                redirected: false,
                status: 202,
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            },
        });

        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 301);
                assert_eq!(started.request_url, request_url);
                assert_eq!(started.body_source_id, body_source_id);
                assert_eq!(started.head.final_url, final_url);
                assert_eq!(started.head.status, 202);
            }
            other => panic!("expected streaming start, got {other:?}"),
        }

        service.finish_fetch_stream_chunk(ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes: b"AB".to_vec(),
        });

        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                assert_eq!(chunk.body_source_id, body_source_id);
                assert_eq!(chunk.bytes, b"AB".to_vec());
            }
            other => panic!("expected streaming chunk, got {other:?}"),
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(final_url),
                response_type: "default".to_owned(),
                redirected: false,
                status: 202,
                status_text: "Accepted".to_owned(),
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: b"AB".to_vec(),
            }),
        });

        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                assert_eq!(finished.internal_id, 301);
                assert_eq!(finished.body_source_id, body_source_id);
                assert_eq!(finished.result, Ok(()));
            }
            other => panic!("expected streaming finish, got {other:?}"),
        }
    }

    #[test]
    fn streaming_fetch_failure_projects_to_subresource_stream_error() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(212);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let body_source_id = 78;
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            302,
            completion_queue.sender(),
        );
        let final_url = url("https://example.test/app/stream.txt");

        service.finish_fetch_stream_started(ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run: run.clone(),
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(final_url),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            },
        });
        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                assert_eq!(started.internal_id, 302);
                assert_eq!(started.body_source_id, body_source_id);
            }
            other => panic!("expected streaming start, got {other:?}"),
        }

        service.finish_fetch_stream_chunk(ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes: b"partial".to_vec(),
        });
        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                assert_eq!(chunk.body_source_id, body_source_id);
                assert_eq!(chunk.bytes, b"partial".to_vec());
            }
            other => panic!("expected streaming chunk, got {other:?}"),
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Failure(
                "FetchEvent.respondWith stream aborted".to_owned(),
            ),
        });

        match pop_async_subresource_event(&mut completion_queue) {
            AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                assert_eq!(finished.internal_id, 302);
                assert_eq!(finished.body_source_id, body_source_id);
                assert_eq!(
                    finished.result,
                    Err("FetchEvent.respondWith stream aborted".to_owned())
                );
            }
            other => panic!("expected streaming finish error, got {other:?}"),
        }
    }

    #[test]
    fn response_completion_projects_opaque_response_filter_to_subresource_queue() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(35);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            315,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "opaque".to_owned(),
                redirected: false,
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 315);
        assert_eq!(
            completion.response_filter,
            Some(AsyncSubresourceFetchResponseFilter::Opaque)
        );
        let response = completion
            .result
            .expect("no-cors opaque response should resolve");
        assert_eq!(response.status, 0);
    }

    #[test]
    fn response_completion_projects_opaqueredirect_response_filter_to_subresource_queue() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(36);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            316,
            completion_queue.sender(),
            moli_fetch::RequestMode::Cors,
            moli_fetch::RequestRedirectMode::Manual,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "opaqueredirect".to_owned(),
                redirected: false,
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 316);
        assert_eq!(
            completion.response_filter,
            Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect)
        );
        let response = completion
            .result
            .expect("manual opaqueredirect response should resolve");
        assert_eq!(response.status, 0);
    }

    #[test]
    fn response_completion_projects_redirected_service_worker_response() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(24);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            304,
            completion_queue.sender(),
        );
        let final_url = url("https://example.test/app/redirect-final.txt");

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(final_url.clone()),
                response_type: "default".to_owned(),
                redirected: true,
                status: 200,
                status_text: "OK".to_owned(),
                headers: Vec::new(),
                body: b"redirected-body".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 304);
        let response = completion
            .result
            .expect("follow redirect mode should accept redirected response");
        assert_eq!(response.final_url, final_url);
        assert!(response.redirected);
        assert!(response.redirect_chain.is_empty());
        assert_eq!(response.body_text(), "redirected-body");
    }

    #[test]
    fn response_completion_rejects_redirected_response_for_non_follow_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(25);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_redirect_mode(
            &service,
            event_id,
            version_id,
            &run,
            305,
            completion_queue.sender(),
            moli_fetch::RequestRedirectMode::Manual,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://example.test/app/manual-final.txt")),
                response_type: "default".to_owned(),
                redirected: true,
                status: 200,
                status_text: "OK".to_owned(),
                headers: Vec::new(),
                body: b"manual-redirected-body".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 305);
        assert_eq!(completion.response_status_text, None);
        assert!(!completion.skip_fetch_security_validation);
        assert_eq!(
            completion.result.err().as_deref(),
            Some(
                "FetchEvent.respondWith rejected a redirected Response for a request whose redirect mode is not follow"
            )
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
    }

    #[test]
    fn response_completion_rejects_redirect_response_for_error_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(30);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_redirect_mode(
            &service,
            event_id,
            version_id,
            &run,
            310,
            completion_queue.sender(),
            moli_fetch::RequestRedirectMode::Error,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![(
                    "location".to_owned(),
                    "https://example.test/redirected.txt".to_owned(),
                )],
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 310);
        assert_eq!(
            completion.result.err().as_deref(),
            Some(
                "FetchEvent.respondWith rejected a redirect Response for a request whose redirect mode is error: https://example.test/redirected.txt"
            )
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_maps_respond_with_promise_rejection_to_failed_network_error() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(33);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            313,
            completion_queue.sender(),
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Failure(
                "FetchEvent.respondWith promise rejected: Error: fetch-boom".to_owned(),
            ),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 313);
        assert_eq!(
            completion.result.err().as_deref(),
            Some("FetchEvent.respondWith promise rejected: Error: fetch-boom")
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_leaves_generic_fetch_failure_network_error_unspecified() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(34);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            314,
            completion_queue.sender(),
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Failure(
                "service worker fetch dispatch failed: worker is not running".to_owned(),
            ),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 314);
        assert_eq!(
            completion.result.err().as_deref(),
            Some("service worker fetch dispatch failed: worker is not running")
        );
        assert_eq!(completion.network_error_text, None);
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_preserves_redirect_response_for_manual_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(31);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let request_url = insert_active_fetch_job_with_redirect_mode(
            &service,
            event_id,
            version_id,
            &run,
            311,
            completion_queue.sender(),
            moli_fetch::RequestRedirectMode::Manual,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![("location".to_owned(), "/manual-target.txt".to_owned())],
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 311);
        assert!(completion.skip_fetch_security_validation);
        assert_eq!(
            completion.response_filter,
            Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect)
        );
        let response = completion
            .result
            .expect("manual redirect response should be delivered");
        assert_eq!(response.final_url, request_url);
        assert_eq!(response.status, 302);
        assert!(!response.redirected);
        assert!(response.redirect_chain.is_empty());
        assert_eq!(
            response.headers,
            vec![("location".to_owned(), "/manual-target.txt".to_owned())]
        );
    }

    #[test]
    fn response_completion_follows_redirect_response_by_dispatching_next_fetch_event() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(32);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            312,
            completion_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.running_state = ServiceWorkerVersionRunningState::Starting {
                host: RendererServiceWorkerHost::new_loading(version_id, &run),
            };
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![(
                    "Location".to_owned(),
                    "https://example.test/app/next.txt".to_owned(),
                )],
                body: Vec::new(),
            }),
        });

        assert!(
            !completion_queue.has_ready_completion(),
            "follow redirect should not complete the original fetch yet"
        );
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.pending_start_events.len(), 1);
        let ServiceWorkerPendingStartEvent::Fetch(event) = &version.pending_start_events[0] else {
            panic!("expected redirected fetch event");
        };
        assert_eq!(event.request.url, url("https://example.test/app/next.txt"));
        let redirected_job = state
            .pending_fetch_jobs
            .get(&event.event_id)
            .expect("redirected fetch job should be pending");
        assert_eq!(redirected_job.internal_id, 312);
        assert_eq!(
            redirected_job.request_url,
            url("https://example.test/app/next.txt")
        );
        assert_eq!(redirected_job.redirect_count, 1);
        assert_eq!(redirected_job.redirect_chain.len(), 1);
        assert_eq!(
            redirected_job.redirect_chain[0].from_url,
            url("https://example.test/app/data.txt")
        );
        assert_eq!(
            redirected_job.redirect_chain[0].to_url,
            url("https://example.test/app/next.txt")
        );
    }

    #[test]
    fn synthetic_redirect_follow_rewrites_job_and_records_redirect_chain() {
        let service = new_service_worker_runtime_service();
        let completion_queue = crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let request_url = url("https://example.test/app/post");
        let mut job = ServiceWorkerFetchJob {
            internal_id: 312,
            version_id: ServiceWorkerVersionId(1),
            run: Some(crate::runtime::RendererServiceWorkerRunIdentity::fresh()),
            request_url: request_url.clone(),
            request_method: "POST".to_owned(),
            request_headers: vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("x-keep".to_owned(), "yes".to_owned()),
            ],
            request_body: Some("payload".to_owned()),
            request_body_bytes: Some(b"payload".to_vec()),
            cors_preflight_request_headers: Vec::new(),
            client_id: ServiceWorkerClientId::from_u64_for_test(0),
            resulting_client_id: None,
            destination: ServiceWorkerRequestDestination::Empty,
            is_reload: false,
            metadata: Default::default(),
            request_mode: moli_fetch::RequestMode::Cors,
            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
            priority: None,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: request_url.clone(),
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            navigation_preload_cancel_handle: None,
            streaming_body_source_id: None,
            direct_completion_tx: None,
        };
        let response = ServiceWorkerFetchResponse {
            final_url: Some(request_url.clone()),
            response_type: "default".to_owned(),
            redirected: false,
            status: 303,
            status_text: "See Other".to_owned(),
            headers: vec![("Location".to_owned(), "next".to_owned())],
            body: Vec::new(),
        };
        let next_url = service_worker_redirect_target(&job, &response)
            .expect("redirect target should resolve")
            .expect("redirect target should be present");

        apply_service_worker_synthetic_redirect(&mut job, response, next_url.clone())
            .expect("synthetic redirect should apply");

        assert_eq!(next_url, url("https://example.test/app/next"));
        assert_eq!(job.request_url, next_url);
        assert_eq!(job.request_method, "GET");
        assert_eq!(job.request_body, None);
        assert_eq!(job.request_body_bytes, None);
        assert_eq!(
            job.request_headers,
            vec![("x-keep".to_owned(), "yes".to_owned())]
        );
        assert_eq!(job.cors_preflight_request_headers, job.request_headers);
        assert_eq!(job.redirect_count, 1);
        assert_eq!(job.redirect_chain.len(), 1);
        assert_eq!(job.redirect_chain[0].from_url, request_url);
        assert_eq!(job.redirect_chain[0].to_url, next_url);
        assert_eq!(job.redirect_chain[0].status, 303);
    }

    #[test]
    fn response_completion_rejects_generated_relative_redirect_for_follow_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(33);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            333,
            completion_queue.sender(),
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![("Location".to_owned(), "relative-target.txt".to_owned())],
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 333);
        assert_eq!(
            completion.result.err().as_deref(),
            Some(
                "failed to resolve redirect location `relative-target.txt` for a generated response without a response URL"
            )
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_resolves_relative_redirect_against_fetched_response_url() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(37);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let request_url = insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            337,
            completion_queue.sender(),
        );
        let response_url = url("https://redirect-source.test/resources/redirect.py");

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(response_url.clone()),
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![("Location".to_owned(), "blank.html".to_owned())],
                body: Vec::new(),
            }),
        });

        assert_eq!(request_url.as_str(), "https://example.test/app/data.txt");
        assert!(
            !completion_queue.has_ready_completion(),
            "follow redirect should not complete the original fetch yet"
        );
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.pending_start_events.len(), 1);
        let ServiceWorkerPendingStartEvent::Fetch(event) = &version.pending_start_events[0] else {
            panic!("expected redirected fetch event");
        };
        assert_eq!(
            event.request.url,
            url("https://redirect-source.test/resources/blank.html")
        );
        let redirected_job = state
            .pending_fetch_jobs
            .get(&event.event_id)
            .expect("redirected fetch job should be pending");
        assert_eq!(redirected_job.internal_id, 337);
        assert_eq!(
            redirected_job.request_url,
            url("https://redirect-source.test/resources/blank.html")
        );
        assert_eq!(redirected_job.redirect_count, 1);
        assert_eq!(redirected_job.redirect_chain.len(), 1);
        assert_eq!(redirected_job.redirect_chain[0].from_url, response_url);
        assert_eq!(
            redirected_job.redirect_chain[0].to_url,
            url("https://redirect-source.test/resources/blank.html")
        );
    }

    #[test]
    fn response_completion_rejects_error_response() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(26);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            306,
            completion_queue.sender(),
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "error".to_owned(),
                redirected: false,
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 306);
        assert_eq!(completion.response_status_text, None);
        assert!(!completion.skip_fetch_security_validation);
        assert_eq!(
            completion.result.err().as_deref(),
            Some("FetchEvent.respondWith rejected an error Response")
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
    }

    #[test]
    fn response_completion_rejects_cors_response_for_same_origin_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(27);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            307,
            completion_queue.sender(),
            moli_fetch::RequestMode::SameOrigin,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cross-origin.test/data.txt")),
                response_type: "cors".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 307);
        assert_eq!(
            completion.result.err().as_deref(),
            Some("FetchEvent.respondWith rejected a cors Response for a same-origin request")
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_rejects_opaque_response_for_non_no_cors_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(28);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            308,
            completion_queue.sender(),
            moli_fetch::RequestMode::Cors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "opaque".to_owned(),
                redirected: false,
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 308);
        assert_eq!(
            completion.result.err().as_deref(),
            Some(
                "FetchEvent.respondWith rejected an opaque Response for a request whose mode is not no-cors"
            )
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn response_completion_rejects_opaque_response_for_client_request() {
        let service = new_service_worker_runtime_service();
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();

        for (index, destination) in [
            ServiceWorkerRequestDestination::Document,
            ServiceWorkerRequestDestination::Iframe,
            ServiceWorkerRequestDestination::Worker,
            ServiceWorkerRequestDestination::SharedWorker,
        ]
        .into_iter()
        .enumerate()
        {
            let event_id = ServiceWorkerEventId(40 + index as u64);
            let mut completion_queue =
                crate::page_task_queue::RendererResourceCompletionTestHarness::new();
            insert_active_fetch_job_with_modes_and_destination(
                &service,
                event_id,
                version_id,
                &run,
                320 + index as u64,
                completion_queue.sender(),
                moli_fetch::RequestMode::NoCors,
                moli_fetch::RequestRedirectMode::Follow,
                destination,
            );

            service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
                event_id,
                version_id,
                run: run.clone(),
                result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                    final_url: None,
                    response_type: "opaque".to_owned(),
                    redirected: false,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                }),
            });

            let completion = pop_async_subresource_completion(&mut completion_queue);
            assert_eq!(completion.internal_id, 320 + index as u64);
            assert_eq!(
                completion.result.err().as_deref(),
                Some("FetchEvent.respondWith rejected an opaque Response for a client request")
            );
            assert_eq!(
                completion.network_error_text.as_deref(),
                Some(crate::network_host::FAILED_ERROR_TEXT)
            );
            assert!(!completion.skip_fetch_security_validation);
        }
    }

    #[test]
    fn response_completion_rejects_no_cors_service_worker_response_blocked_by_corp() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(50);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            330,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/image.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![
                    ("content-type".to_owned(), "image/png".to_owned()),
                    (
                        "cross-origin-resource-policy".to_owned(),
                        "same-origin".to_owned(),
                    ),
                ],
                body: b"blocked".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 330);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("CORP should reject")
                .contains("Cross-Origin-Resource-Policy")
        );
    }

    #[test]
    fn response_completion_rejects_no_cors_image_service_worker_response_blocked_by_corp() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(58);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            338,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/image.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![
                    ("content-type".to_owned(), "image/png".to_owned()),
                    (
                        "cross-origin-resource-policy".to_owned(),
                        "same-origin".to_owned(),
                    ),
                ],
                body: b"blocked".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 338);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("image CORP should reject")
                .contains("Cross-Origin-Resource-Policy")
        );
    }

    #[test]
    fn response_completion_rejects_no_cors_service_worker_response_blocked_by_coep_require_corp() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(53);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            333,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 333);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("COEP require-corp should reject")
                .contains("Cross-Origin-Embedder-Policy")
        );
    }

    #[test]
    fn response_completion_rejects_no_cors_image_service_worker_response_blocked_by_coep_require_corp()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(55);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            335,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 335);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("COEP require-corp should reject image responses")
                .contains("Cross-Origin-Embedder-Policy")
        );
    }

    #[test]
    fn response_completion_allows_no_cors_service_worker_response_with_coep_require_corp_and_corp_cross_origin()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(54);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            334,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![
                    ("content-type".to_owned(), "image/png".to_owned()),
                    (
                        "cross-origin-resource-policy".to_owned(),
                        "cross-origin".to_owned(),
                    ),
                ],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 334);
        assert!(completion.skip_fetch_security_validation);
        assert!(completion.result.is_ok());
    }

    #[test]
    fn response_completion_allows_no_cors_image_service_worker_response_with_coep_require_corp_and_corp_cross_origin()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(56);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            336,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![
                    ("content-type".to_owned(), "image/png".to_owned()),
                    (
                        "cross-origin-resource-policy".to_owned(),
                        "cross-origin".to_owned(),
                    ),
                ],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 336);
        assert!(completion.skip_fetch_security_validation);
        assert!(completion.result.is_ok());
    }

    #[test]
    fn response_completion_allows_no_cors_service_worker_response_with_coep_credentialless_without_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(59);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            339,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::Credentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::SameOrigin;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 339);
        assert!(completion.skip_fetch_security_validation);
        assert!(completion.result.is_ok());
    }

    #[test]
    fn response_completion_rejects_no_cors_service_worker_response_with_coep_credentialless_and_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(60);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            340,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::Credentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 340);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        let error = completion
            .result
            .expect_err("COEP credentialless should reject credentialed no-cors responses");
        assert!(error.contains("Cross-Origin-Embedder-Policy"));
        assert!(error.contains("credentialless"));
    }

    #[test]
    fn response_completion_rejects_no_cors_image_service_worker_response_blocked_by_dip_require_corp()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(70);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            370,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .document_isolation_policy =
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 370);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        let error = completion
            .result
            .expect_err("DIP isolate-and-require-corp should reject image responses");
        assert!(error.contains("Document-Isolation-Policy"));
        assert!(error.contains("isolate-and-require-corp"));
    }

    #[test]
    fn response_completion_allows_no_cors_service_worker_response_with_dip_credentialless_without_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(71);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            371,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context.policy_context.document_isolation_policy =
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndCredentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::SameOrigin;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 371);
        assert!(completion.skip_fetch_security_validation);
        assert!(completion.result.is_ok());
    }

    #[test]
    fn response_completion_rejects_no_cors_service_worker_response_with_dip_credentialless_and_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(72);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            372,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context.policy_context.document_isolation_policy =
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndCredentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
                body: b"png".to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 372);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        let error = completion
            .result
            .expect_err("DIP isolate-and-credentialless should reject credentialed responses");
        assert!(error.contains("Document-Isolation-Policy"));
        assert!(error.contains("isolate-and-credentialless"));
    }

    #[test]
    fn stream_start_rejects_no_cors_service_worker_response_blocked_by_coep_require_corp() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(57);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let body_source_id = 107;
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            337,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist")
                .network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp;
        }

        service.finish_fetch_stream_started(ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
            },
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 337);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("COEP require-corp should reject before stream chunks")
                .contains("Cross-Origin-Embedder-Policy")
        );

        service.finish_fetch_stream_chunk(ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes: b"late".to_vec(),
        });
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn stream_start_rejects_no_cors_service_worker_response_blocked_by_coep_credentialless_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(61);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let body_source_id = 108;
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            341,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context
                .policy_context
                .cross_origin_embedder_policy =
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::Credentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        }

        service.finish_fetch_stream_started(ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
            },
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 341);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        let error = completion
            .result
            .expect_err("COEP credentialless should reject before stream chunks");
        assert!(error.contains("Cross-Origin-Embedder-Policy"));
        assert!(error.contains("credentialless"));

        service.finish_fetch_stream_chunk(ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes: b"late".to_vec(),
        });
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn stream_start_rejects_no_cors_service_worker_response_blocked_by_dip_credentialless_credentials()
     {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(73);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let body_source_id = 109;
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes_and_resource_type(
            &service,
            event_id,
            version_id,
            &run,
            373,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
            crate::types::SubresourceResourceType::Image,
        );
        {
            let mut state = service.inner.state.lock();
            let job = state
                .pending_fetch_jobs
                .get_mut(&event_id)
                .expect("pending fetch job should exist");
            job.network_context.policy_context.document_isolation_policy =
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndCredentialless;
            job.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        }

        service.finish_fetch_stream_started(ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(url("https://cdn.example.test/app/pixel.png")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                headers: vec![("content-type".to_owned(), "image/png".to_owned())],
            },
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 373);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        let error = completion
            .result
            .expect_err("DIP isolate-and-credentialless should reject before stream chunks");
        assert!(error.contains("Document-Isolation-Policy"));
        assert!(error.contains("isolate-and-credentialless"));

        service.finish_fetch_stream_chunk(ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes: b"late".to_vec(),
        });
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn response_completion_rejects_no_cors_service_worker_response_blocked_by_orb() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(51);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            331,
            completion_queue.sender(),
            moli_fetch::RequestMode::NoCors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/data.json")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: br#"{"secret":true}"#.to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 331);
        assert_eq!(completion.response_status_text, None);
        assert_eq!(completion.response_filter, None);
        assert!(!completion.skip_fetch_security_validation);
        assert!(
            completion
                .result
                .expect_err("ORB should reject")
                .contains("OpaqueResponseBlocking")
        );
    }

    #[test]
    fn response_completion_does_not_reapply_cors_to_service_worker_response() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(52);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            332,
            completion_queue.sender(),
            moli_fetch::RequestMode::Cors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: Some(url("https://cdn.example.test/app/data.json")),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: br#"{"visible":true}"#.to_vec(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 332);
        assert!(completion.skip_fetch_security_validation);
        assert_eq!(completion.response_filter, None);
        let response = completion
            .result
            .expect("Service Worker response should not need ACAO");
        assert_eq!(response.status, 200);
        assert_eq!(response.body_text(), r#"{"visible":true}"#);
    }

    #[test]
    fn response_completion_rejects_opaqueredirect_response_for_non_manual_request() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(29);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job_with_modes(
            &service,
            event_id,
            version_id,
            &run,
            309,
            completion_queue.sender(),
            moli_fetch::RequestMode::Cors,
            moli_fetch::RequestRedirectMode::Follow,
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                final_url: None,
                response_type: "opaqueredirect".to_owned(),
                redirected: false,
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            }),
        });

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 309);
        assert_eq!(
            completion.result.err().as_deref(),
            Some(
                "FetchEvent.respondWith rejected an opaqueredirect Response for a request whose redirect mode is not manual"
            )
        );
        assert_eq!(
            completion.network_error_text.as_deref(),
            Some(crate::network_host::FAILED_ERROR_TEXT)
        );
        assert!(!completion.skip_fetch_security_validation);
    }

    #[test]
    fn direct_fallback_does_not_start_a_second_resource_dispatch() {
        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(22);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let mut completion_queue =
            crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            302,
            completion_queue.sender(),
        );
        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        service
            .inner
            .state
            .lock()
            .pending_fetch_jobs
            .get_mut(&event_id)
            .expect("active fetch job")
            .direct_completion_tx = Some(direct_completion_tx);

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            version_id,
            run,
            result: ServiceWorkerFetchResult::Fallback,
        });

        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn network_fallback_uses_the_captured_runner_without_an_ambient_runtime() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fallback server");
        let address = listener.local_addr().expect("fallback server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fallback request");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).expect("read fallback request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nfallback",
                )
                .expect("write fallback response");
        });

        let service = new_service_worker_runtime_service();
        let event_id = ServiceWorkerEventId(22);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let completion_queue = crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        insert_active_fetch_job(
            &service,
            event_id,
            version_id,
            &run,
            302,
            completion_queue.sender(),
        );
        let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
        let mut job = service
            .inner
            .state
            .lock()
            .pending_fetch_jobs
            .remove(&event_id)
            .expect("active fetch job");
        job.request_url =
            Url::parse(&format!("http://{address}/fallback")).expect("fallback request URL");
        job.network_context.document_url =
            Url::parse(&format!("http://{address}/page")).expect("fallback document URL");
        job.redirect_count = 1;
        job.direct_completion_tx = Some(direct_completion_tx);

        // This test intentionally calls from a plain test thread. The
        // fallback must use the runner captured by the original resource
        // authority rather than trying to discover an ambient Tokio runtime.
        service.dispatch_fetch_fallback(job);

        let result = direct_completion_rx
            .blocking_recv()
            .expect("network fallback completion");
        let ServiceWorkerDirectFetchResult::Response(response) = result else {
            panic!("expected network fallback response, got {result:?}");
        };
        assert_eq!(response.response.status, 200);
        assert_eq!(response.response.body_text(), "fallback");
        server.join().expect("fallback server should finish");
    }
}

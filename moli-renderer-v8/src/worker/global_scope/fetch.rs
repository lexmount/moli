use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerClientId, ServiceWorkerDirectFetchResult, ServiceWorkerFetchDispatch,
    ServiceWorkerFetchRequest, ServiceWorkerFetchRequestMetadata, ServiceWorkerRequestDestination,
    ServiceWorkerRuntimeService,
};
use crate::types::{AsyncSubresourceNetworkContext, SubresourcePolicyContext};

pub(in crate::worker) fn record_worker_subresource_failure(
    state: &WorkerGlobalState,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    error_text: String,
) {
    record_worker_subresource_failure_with_handle(
        state,
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body,
        resource_type,
        error_text,
    );
}

pub(in crate::worker) fn record_worker_subresource_failure_with_handle(
    state: &WorkerGlobalState,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    error_text: String,
) {
    let mut record = SubresourceNetworkRecord::failure(
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body,
        resource_type,
        error_text,
    );
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::SubresourceNetwork(record));
}

pub(in crate::worker) fn record_worker_subresource_success(
    state: &WorkerGlobalState,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    head: ResponseHead,
    body: SubresourceResponseBody,
) {
    record_worker_subresource_success_with_handle(
        state,
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body,
        resource_type,
        None,
        head,
        body,
    );
}

pub(in crate::worker) fn record_worker_subresource_success_with_handle(
    state: &WorkerGlobalState,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    network_request_headers: Option<Vec<(String, String)>>,
    head: ResponseHead,
    body: SubresourceResponseBody,
) {
    let mut record = SubresourceNetworkRecord::success_with_body(
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body,
        resource_type,
        head.request_cookie_report.clone(),
        head.redirect_chain
            .clone()
            .into_iter()
            .map(Into::into)
            .collect(),
        head.final_url.clone(),
        head.status,
        head.headers.clone(),
        body,
        head.cookie_set_reports.clone(),
    )
    .with_from_cache(head.from_cache)
    .with_negotiated_http_version(head.negotiated_http_version)
    .with_network_request_headers(network_request_headers);
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::SubresourceNetwork(record));
}

fn worker_network_result_parts<R>(
    observed: moli_fetch::NetworkFetchResult<R>,
) -> (R, Option<Vec<(String, String)>>) {
    let (response, request_observation) = observed.into_parts();
    (
        response,
        request_observation.map(|observation| observation.into_headers()),
    )
}

pub(in crate::worker) fn spawn_worker_fetch_network(
    load: ResourceLoadLease,
    completion_tx: mpsc::UnboundedSender<WorkerFetchEvent>,
    fetch_id: u32,
    cancel_handle: FetchCancelHandle,
    document_url: Url,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    resolved_url: Url,
    method: String,
    body: Option<Vec<u8>>,
    mut headers: Vec<(String, String)>,
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    redirect_mode: RequestRedirectMode,
    priority: Option<moli_fetch::FetchPriorityHint>,
    request_metadata: ServiceWorkerFetchRequestMetadata,
    auth: Option<crate::protocol_types::SubresourceAuthCredentials>,
    suppress_default_content_type: bool,
    allow_headers_first: bool,
) {
    tokio::task::spawn_local(async move {
        let loader = load.request_client();
        let (result, network_request_headers) = if matches!(resolved_url.scheme(), "blob" | "data")
        {
            (
                local_url_response(&resolved_url)
                    .map(|response| WorkerFetchResponse::Materialized(Box::new(response)))
                    .ok_or_else(|| format!("fetch: local url `{resolved_url}` is unavailable")),
                None,
            )
        } else {
            let cors_preflight_request_headers = headers.clone();
            if suppress_default_content_type {
                // An empty header value is intentional: the fetch transport serializes this
                // as `Content-Type:` so the HTTP stack does not synthesize its own upload default.
                headers.push(("Content-Type".to_owned(), String::new()));
            }
            match Request::new_bytes(&method, resolved_url.as_str(), body, headers) {
                Ok(request) => {
                    let mut request = request
                        .with_initiator_url(&document_url)
                        .with_request_mode(request_mode)
                        .with_credentials_mode(credentials_mode)
                        .with_redirect_mode(redirect_mode)
                        .with_cache_mode(worker_fetch_cache_mode(&request_metadata.cache))
                        .with_fetch_priority_hint(priority)
                        .with_network_partition_key(network_partition_key.clone())
                        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
                    if request_metadata.referrer.is_empty() {
                        request = request.without_inferred_referrer();
                    }
                    if let Some(metadata) =
                        worker_fetch_script_metadata(referrer_policy, &request_metadata)
                    {
                        request = request.with_script_fetch_metadata(metadata);
                    }
                    if let Some(auth) = auth {
                        request = request.with_auth(auth.into());
                    }
                    if request.auth_requires_buffered_transport()
                        || request.request_mode == RequestMode::NoCors
                        || !request.follow_redirects
                        || browser_request_needs_manual_preflight_redirects(
                            &request,
                            &cors_preflight_request_headers,
                        )
                    {
                        match fetch_browser_subresource_with_preflight_headers_and_network_metadata(
                            loader.clone(),
                            request,
                            Some(cancel_handle),
                            cors_preflight_request_headers,
                        )
                        .await
                        {
                            Ok(observed) => {
                                let (response, network_request_headers) =
                                    worker_network_result_parts(observed);
                                (
                                    Ok(WorkerFetchResponse::Materialized(Box::new(response))),
                                    network_request_headers,
                                )
                            }
                            Err(error) => (Err(format!("fetch: {error}")), None),
                        }
                    } else if !allow_headers_first {
                        // Worker fetch still resolves after the full response, but ordinary
                        // transfers can keep their body in the same chunked/spooled carrier that
                        // CDP Network capture uses instead of forcing a single buffered Response.
                        match fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                            &loader,
                            request,
                            Some(cancel_handle),
                            cors_preflight_request_headers,
                        )
                        .await
                        {
                            Ok(observed) => {
                                let (mut response, network_request_headers) =
                                    worker_network_result_parts(observed);
                                let head = response.head();
                                let mut body_writer = SubresourceResponseBodyWriter::default();
                                while let Some(chunk) = response.next_chunk().await {
                                    body_writer.append(&chunk);
                                }
                                let result = match response.finish().await {
                                    Ok(()) => Ok(WorkerFetchResponse::Streamed {
                                        head: Box::new(head),
                                        body: body_writer.finish(),
                                    }),
                                    Err(error) => Err(format!("fetch: {error}")),
                                };
                                (result, network_request_headers)
                            }
                            Err(error) => (Err(format!("fetch: {error}")), None),
                        }
                    } else {
                        match fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                            &loader,
                            request,
                            Some(cancel_handle),
                            cors_preflight_request_headers,
                        )
                        .await
                        {
                            Ok(observed) => {
                                let (mut response, network_request_headers) =
                                    worker_network_result_parts(observed);
                                let body_source_id =
                                    crate::network_host::new_network_body_source_id();
                                let head = response.head();
                                let _ = completion_tx.send(WorkerFetchEvent::StreamingStarted(
                                    WorkerFetchStreamingStarted {
                                        fetch_id,
                                        body_source_id,
                                        head: head.clone(),
                                        network_request_headers,
                                    },
                                ));
                                let mut body_writer = SubresourceResponseBodyWriter::default();
                                while let Some(chunk) = response.next_chunk().await {
                                    body_writer.append(&chunk);
                                    let _ = completion_tx.send(WorkerFetchEvent::StreamingChunk(
                                        WorkerFetchStreamingChunk {
                                            body_source_id,
                                            bytes: chunk,
                                        },
                                    ));
                                }
                                let result = response
                                    .finish()
                                    .await
                                    .map(|()| body_writer.finish())
                                    .map_err(|error| format!("fetch: {error}"));
                                let _ = completion_tx.send(WorkerFetchEvent::StreamingFinished(
                                    WorkerFetchStreamingFinished {
                                        fetch_id,
                                        body_source_id,
                                        head,
                                        result,
                                    },
                                ));
                                return;
                            }
                            Err(error) => (Err(format!("fetch: {error}")), None),
                        }
                    }
                }
                Err(error) => (
                    Err(format!("fetch: failed to build request: {error}")),
                    None,
                ),
            }
        };
        let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
            WorkerFetchCompletion {
                fetch_id,
                network_request_headers,
                result,
            },
        )));
    });
}

fn worker_fetch_cache_mode(cache: &str) -> moli_fetch::RequestCacheMode {
    match cache {
        "no-cache" | "no-store" | "reload" => moli_fetch::RequestCacheMode::Validate,
        _ => moli_fetch::RequestCacheMode::Default,
    }
}

fn worker_fetch_script_metadata(
    document_referrer_policy: Option<String>,
    request_metadata: &ServiceWorkerFetchRequestMetadata,
) -> Option<moli_fetch::ScriptFetchRequestMetadata> {
    let referrer_policy = (!request_metadata.referrer_policy.is_empty())
        .then(|| request_metadata.referrer_policy.clone());
    let integrity =
        (!request_metadata.integrity.is_empty()).then(|| request_metadata.integrity.clone());
    if referrer_policy.is_none() && document_referrer_policy.is_none() && integrity.is_none() {
        return None;
    }
    Some(moli_fetch::ScriptFetchRequestMetadata {
        referrer_policy,
        document_referrer_policy,
        integrity,
        ..moli_fetch::ScriptFetchRequestMetadata::default()
    })
}

fn worker_service_worker_controller(
    state: &WorkerGlobalState,
    resolved_url: &Url,
) -> Option<(ServiceWorkerRuntimeService, ServiceWorkerClientId)> {
    if !matches!(resolved_url.scheme(), "http" | "https") {
        return None;
    }
    let runtime = state.service_worker_runtime.clone()?;
    let client_id = state.service_worker_client_id?;
    runtime.matching_controller_for_client_fetch(client_id, resolved_url)?;
    Some((runtime, client_id))
}

#[allow(clippy::too_many_arguments)]
fn spawn_worker_fetch_service_worker(
    runtime: ServiceWorkerRuntimeService,
    client_id: ServiceWorkerClientId,
    load: ResourceLoadLease,
    completion_tx: mpsc::UnboundedSender<WorkerFetchEvent>,
    fetch_id: u32,
    cancel_handle: FetchCancelHandle,
    document_url: Url,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    policy_context: SubresourcePolicyContext,
    resolved_url: Url,
    method: String,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    redirect_mode: RequestRedirectMode,
    priority: Option<moli_fetch::FetchPriorityHint>,
    request_metadata: ServiceWorkerFetchRequestMetadata,
    suppress_default_content_type: bool,
) {
    let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
    let request_body_text = request_body_text(&body);
    let dispatch = ServiceWorkerFetchDispatch {
        internal_id: u64::from(fetch_id),
        request: ServiceWorkerFetchRequest {
            client_id,
            resulting_client_id: None,
            url: resolved_url.clone(),
            method: method.clone(),
            headers: headers.clone(),
            body: body.clone(),
            destination: ServiceWorkerRequestDestination::Empty,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            is_reload: false,
            metadata: request_metadata.clone(),
        },
        request_body_text,
        cors_preflight_request_headers: headers.clone(),
        request_cookie_report: None,
        network_context: AsyncSubresourceNetworkContext {
            frame_id: None,
            document_url: document_url.clone(),
            resource_type: SubresourceResourceType::Fetch,
            policy_context,
        },
        completion_tx:
            crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only(),
        request_client: load.request_client(),
        resource_task_runner: load.task_runner(),
        cancel_handle: cancel_handle.clone(),
        direct_completion_tx: Some(direct_completion_tx),
    };

    if !runtime.dispatch_controlled_fetch(dispatch) {
        let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
            WorkerFetchCompletion {
                fetch_id,
                network_request_headers: None,
                result: Err("service worker fetch dispatch failed".to_owned()),
            },
        )));
        return;
    }

    tokio::task::spawn_local(async move {
        match direct_completion_rx.await {
            Ok(ServiceWorkerDirectFetchResult::Fallback) => spawn_worker_fetch_network(
                load,
                completion_tx,
                fetch_id,
                cancel_handle,
                document_url,
                referrer_policy,
                network_partition_key,
                resolved_url,
                method,
                body,
                headers,
                request_mode,
                credentials_mode,
                redirect_mode,
                priority,
                request_metadata,
                None,
                suppress_default_content_type,
                true,
            ),
            Ok(ServiceWorkerDirectFetchResult::Response(response)) => {
                let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
                    WorkerFetchCompletion {
                        fetch_id,
                        network_request_headers: None,
                        result: Ok(WorkerFetchResponse::Materialized(Box::new(
                            (*response.response).into(),
                        ))),
                    },
                )));
            }
            Ok(ServiceWorkerDirectFetchResult::Failure(message)) => {
                let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
                    WorkerFetchCompletion {
                        fetch_id,
                        network_request_headers: None,
                        result: Err(message),
                    },
                )));
            }
            Err(_) => {
                let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
                    WorkerFetchCompletion {
                        fetch_id,
                        network_request_headers: None,
                        result: Err("service worker fetch completion channel closed".to_owned()),
                    },
                )));
            }
        }
    });
}

pub(in crate::worker) fn spawn_worker_xhr_network(
    load: ResourceLoadLease,
    completion_tx: mpsc::UnboundedSender<WorkerXhrCompletion>,
    xhr_id: u32,
    cancel_handle: FetchCancelHandle,
    document_url: Url,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    resolved_url: Url,
    method: String,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    credentials_mode: RequestCredentialsMode,
    auth: Option<crate::protocol_types::SubresourceAuthCredentials>,
) {
    tokio::task::spawn_local(async move {
        let loader = load.request_client();
        let cors_preflight_request_headers = headers.clone();
        let request =
            Request::new_bytes(&method, resolved_url.as_str(), body, headers).map(|request| {
                let mut request = request
                    .with_initiator_url(&document_url)
                    .with_credentials_mode(credentials_mode)
                    .with_network_partition_key(network_partition_key.clone())
                    .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
                if let Some(referrer_policy) = referrer_policy {
                    request = request.with_script_fetch_metadata(
                        moli_fetch::ScriptFetchRequestMetadata {
                            document_referrer_policy: Some(referrer_policy),
                            ..moli_fetch::ScriptFetchRequestMetadata::default()
                        },
                    );
                }
                if let Some(auth) = auth {
                    request = request.with_auth(auth.into());
                }
                request
            });

        let (result, network_request_headers) = match request {
            Ok(request)
                if request.auth_requires_buffered_transport()
                    || browser_request_needs_manual_preflight_redirects(
                        &request,
                        &cors_preflight_request_headers,
                    ) =>
            {
                match fetch_browser_subresource_with_preflight_headers_and_network_metadata(
                    loader.clone(),
                    request,
                    Some(cancel_handle),
                    cors_preflight_request_headers,
                )
                .await
                {
                    Ok(observed) => {
                        let (response, network_request_headers) =
                            worker_network_result_parts(observed);
                        (
                            Ok(WorkerXhrResponse::Materialized(Box::new(response))),
                            network_request_headers,
                        )
                    }
                    Err(error) => (Err(format!("xhr: {error}")), None),
                }
            }
            Ok(request) => {
                // Ordinary worker XHR can keep the network/cache path streaming
                // and spool large bodies until the XHR DONE boundary.
                match fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                    &loader,
                    request,
                    Some(cancel_handle),
                    cors_preflight_request_headers,
                )
                .await
                {
                    Ok(observed) => {
                        let (mut response, network_request_headers) =
                            worker_network_result_parts(observed);
                        let head = response.head();
                        let mut body_writer = SubresourceResponseBodyWriter::default();
                        while let Some(chunk) = response.next_chunk().await {
                            body_writer.append(&chunk);
                        }
                        let result = match response.finish().await {
                            Ok(()) => Ok(WorkerXhrResponse::Streamed {
                                head: Box::new(head),
                                body: body_writer.finish(),
                            }),
                            Err(error) => Err(format!("xhr: {error}")),
                        };
                        (result, network_request_headers)
                    }
                    Err(error) => (Err(format!("xhr: {error}")), None),
                }
            }
            Err(error) => (Err(format!("xhr: failed to build request: {error}")), None),
        };
        let _ = completion_tx.send(WorkerXhrCompletion {
            xhr_id,
            network_request_headers,
            result,
        });
    });
}

pub(in crate::worker) fn continue_pending_worker_fetch(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
) {
    let allow_headers_first = !request.intercept_response && !request.handle_auth_requests;
    let (
        load,
        completion_tx,
        cancel_handle,
        document_url,
        request_mode,
        credentials_mode,
        redirect_mode,
        network_partition_key,
        fetch_id,
        resolved_url,
        method,
        body,
        headers,
        priority,
        request_metadata,
        auth,
    ) = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        let network_partition_key = state.network_partition_key.clone();
        let Some(pending) = state.pending_fetches.get_mut(&request.fetch_id) else {
            return;
        };
        let cancel_handle = FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        pending.network_request_handle = request.network_request_handle;
        let initial_network_request_headers = pending
            .network_record
            .as_ref()
            .and_then(|record| record.initial_network_request_headers.clone());
        pending.network_record = Some(PendingWorkerFetchNetworkRecord {
            internal_id: request.internal_id,
            url: request.url.clone(),
            method: request.method.clone(),
            request_headers: request.headers.clone(),
            request_body: request.body.clone(),
            initial_network_request_headers,
            intercept_response: request.intercept_response,
            handle_auth_requests: request.handle_auth_requests,
        });
        (
            pending.load.clone(),
            completion_tx,
            cancel_handle,
            pending.document_url.clone(),
            pending.request_mode,
            pending.credentials_mode,
            pending.redirect_mode,
            network_partition_key,
            request.fetch_id,
            request.url,
            request.method,
            request.body,
            request.headers,
            pending.request_priority,
            pending.request_metadata.clone(),
            request.auth,
        )
    };

    spawn_worker_fetch_network(
        load,
        completion_tx,
        fetch_id,
        cancel_handle,
        document_url,
        state.borrow().referrer_policy.clone(),
        network_partition_key,
        resolved_url,
        method,
        body.map(|body| body.into_bytes()),
        headers,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        request_metadata,
        auth,
        false,
        allow_headers_first,
    );
}

pub(in crate::worker) fn fail_pending_worker_fetch(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    error_text: String,
) {
    let fetch_id = request.fetch_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        if let Some(pending) = state.pending_fetches.get_mut(&fetch_id)
            && request.internal_id != 0
        {
            pending.network_request_handle = request.network_request_handle;
            pending.network_record = Some(PendingWorkerFetchNetworkRecord {
                internal_id: request.internal_id,
                url: request.url,
                method: request.method,
                request_headers: request.headers,
                request_body: request.body,
                initial_network_request_headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            });
        }
        completion_tx
    };
    let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id,
            network_request_headers: None,
            result: Err(error_text),
        },
    )));
}

pub(in crate::worker) fn fail_pending_worker_fetch_auth(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    error_text: String,
) {
    let fetch_id = request.fetch_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        if let Some(pending) = state.pending_fetches.get_mut(&fetch_id) {
            pending.network_request_handle = request.network_request_handle;
            pending.network_record = None;
            pending.paused_response = None;
        }
        completion_tx
    };
    let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id,
            network_request_headers: None,
            result: Err(error_text),
        },
    )));
}

pub(in crate::worker) fn fulfill_pending_worker_fetch(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    response_code: u16,
    response_headers: Vec<(String, String)>,
    response_body: RendererSyntheticResponseBody,
) {
    let fetch_id = request.fetch_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        let Some(pending) = state.pending_fetches.get_mut(&fetch_id) else {
            return;
        };
        pending.network_request_handle = request.network_request_handle;
        if request.internal_id != 0 {
            pending.network_record = Some(PendingWorkerFetchNetworkRecord {
                internal_id: request.internal_id,
                url: request.url.clone(),
                method: request.method.clone(),
                request_headers: request.headers.clone(),
                request_body: request.body.clone(),
                initial_network_request_headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            });
        }
        let response =
            worker_response_from_body(request.url, response_code, response_headers, response_body);
        (completion_tx, response, fetch_id)
    };
    let _ = completion.0.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id: completion.2,
            network_request_headers: None,
            result: Ok(WorkerFetchResponse::Materialized(Box::new(completion.1))),
        },
    )));
}

pub(in crate::worker) fn continue_pending_worker_fetch_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    response_code: Option<u16>,
    response_headers: Option<Vec<(String, String)>>,
) {
    let fetch_id = request.fetch_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        let Some(pending) = state.pending_fetches.get_mut(&fetch_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        let Some(mut response) = pending.paused_response.take() else {
            return;
        };
        if let Some(response_code) = response_code {
            response.head.status = response_code;
        }
        if let Some(response_headers) = response_headers {
            response.head.headers = response_headers;
        }
        (completion_tx, response, fetch_id)
    };
    let _ = completion.0.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id: completion.2,
            network_request_headers: None,
            result: Ok(WorkerFetchResponse::Streamed {
                head: Box::new(completion.1.head),
                body: completion.1.body,
            }),
        },
    )));
}

pub(in crate::worker) fn fail_pending_worker_fetch_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    error_text: String,
) {
    let fetch_id = request.fetch_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        let Some(pending) = state.pending_fetches.get_mut(&fetch_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        pending.paused_response = None;
        completion_tx
    };
    let _ = completion_tx.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id,
            network_request_headers: None,
            result: Err(error_text),
        },
    )));
}

pub(in crate::worker) fn fulfill_pending_worker_fetch_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingFetchContinue,
    response_code: u16,
    response_headers: Vec<(String, String)>,
    response_body: RendererSyntheticResponseBody,
) {
    let fetch_id = request.fetch_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.fetch_completion_tx.clone();
        let Some(pending) = state.pending_fetches.get_mut(&fetch_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        pending.paused_response = None;
        let response =
            worker_response_from_body(request.url, response_code, response_headers, response_body);
        (completion_tx, response, fetch_id)
    };
    let _ = completion.0.send(WorkerFetchEvent::Completion(Box::new(
        WorkerFetchCompletion {
            fetch_id: completion.2,
            network_request_headers: None,
            result: Ok(WorkerFetchResponse::Materialized(Box::new(completion.1))),
        },
    )));
}

pub(in crate::worker) fn continue_pending_worker_xhr(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
) {
    let (
        load,
        completion_tx,
        cancel_handle,
        document_url,
        network_partition_key,
        credentials_mode,
        xhr_id,
        resolved_url,
        method,
        body,
        headers,
        auth,
    ) = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        let network_partition_key = state.network_partition_key.clone();
        let Some(pending) = state.pending_xhrs.get_mut(&request.xhr_id) else {
            return;
        };
        let cancel_handle = FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        pending.request_paused = false;
        pending.network_request_handle = request.network_request_handle;
        let initial_network_request_headers = pending
            .network_record
            .as_ref()
            .and_then(|record| record.initial_network_request_headers.clone());
        pending.network_record = Some(PendingWorkerFetchNetworkRecord {
            internal_id: request.internal_id,
            url: request.url.clone(),
            method: request.method.clone(),
            request_headers: request.headers.clone(),
            request_body: request.body.clone(),
            initial_network_request_headers,
            intercept_response: request.intercept_response,
            handle_auth_requests: request.handle_auth_requests,
        });
        (
            pending.load.clone(),
            completion_tx,
            cancel_handle,
            pending.document_url.clone(),
            network_partition_key,
            pending.credentials_mode,
            request.xhr_id,
            request.url,
            request.method,
            request.body,
            request.headers,
            request.auth,
        )
    };

    spawn_worker_xhr_network(
        load,
        completion_tx,
        xhr_id,
        cancel_handle,
        document_url,
        state.borrow().referrer_policy.clone(),
        network_partition_key,
        resolved_url,
        method,
        body.map(|body| body.into_bytes()),
        headers,
        credentials_mode,
        auth,
    );
}

pub(in crate::worker) fn fail_pending_worker_xhr(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    error_text: String,
) {
    let xhr_id = request.xhr_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        if let Some(pending) = state.pending_xhrs.get_mut(&xhr_id)
            && request.internal_id != 0
        {
            pending.request_paused = false;
            pending.network_request_handle = request.network_request_handle;
            pending.network_record = Some(PendingWorkerFetchNetworkRecord {
                internal_id: request.internal_id,
                url: request.url,
                method: request.method,
                request_headers: request.headers,
                request_body: request.body,
                initial_network_request_headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            });
        }
        completion_tx
    };
    let _ = completion_tx.send(WorkerXhrCompletion {
        xhr_id,
        network_request_headers: None,
        result: Err(error_text),
    });
}

pub(in crate::worker) fn fail_pending_worker_xhr_auth(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    error_text: String,
) {
    let xhr_id = request.xhr_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        if let Some(pending) = state.pending_xhrs.get_mut(&xhr_id) {
            pending.request_paused = false;
            pending.network_request_handle = request.network_request_handle;
            pending.network_record = None;
            pending.paused_response = None;
        }
        completion_tx
    };
    let _ = completion_tx.send(WorkerXhrCompletion {
        xhr_id,
        network_request_headers: None,
        result: Err(error_text),
    });
}

pub(in crate::worker) fn fulfill_pending_worker_xhr(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    response_code: u16,
    response_headers: Vec<(String, String)>,
    response_body: RendererSyntheticResponseBody,
) {
    let xhr_id = request.xhr_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        let Some(pending) = state.pending_xhrs.get_mut(&xhr_id) else {
            return;
        };
        pending.request_paused = false;
        pending.network_request_handle = request.network_request_handle;
        if request.internal_id != 0 {
            pending.network_record = Some(PendingWorkerFetchNetworkRecord {
                internal_id: request.internal_id,
                url: request.url.clone(),
                method: request.method.clone(),
                request_headers: request.headers.clone(),
                request_body: request.body.clone(),
                initial_network_request_headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            });
        }
        let response =
            worker_response_from_body(request.url, response_code, response_headers, response_body);
        (completion_tx, response, xhr_id)
    };
    let _ = completion.0.send(WorkerXhrCompletion {
        xhr_id: completion.2,
        network_request_headers: None,
        result: Ok(WorkerXhrResponse::Materialized(Box::new(completion.1))),
    });
}

pub(in crate::worker) fn continue_pending_worker_xhr_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    response_code: Option<u16>,
    response_headers: Option<Vec<(String, String)>>,
) {
    let xhr_id = request.xhr_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        let Some(pending) = state.pending_xhrs.get_mut(&xhr_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        let Some(mut response) = pending.paused_response.take() else {
            return;
        };
        if let Some(response_code) = response_code {
            response.head.status = response_code;
        }
        if let Some(response_headers) = response_headers {
            response.head.headers = response_headers;
        }
        (completion_tx, response, xhr_id)
    };
    let _ = completion.0.send(WorkerXhrCompletion {
        xhr_id: completion.2,
        network_request_headers: None,
        result: Ok(WorkerXhrResponse::Streamed {
            head: Box::new(completion.1.head),
            body: completion.1.body,
        }),
    });
}

pub(in crate::worker) fn fail_pending_worker_xhr_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    error_text: String,
) {
    let xhr_id = request.xhr_id;
    let completion_tx = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        let Some(pending) = state.pending_xhrs.get_mut(&xhr_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        pending.paused_response = None;
        completion_tx
    };
    let _ = completion_tx.send(WorkerXhrCompletion {
        xhr_id,
        network_request_headers: None,
        result: Err(error_text),
    });
}

pub(in crate::worker) fn fulfill_pending_worker_xhr_response(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: WorkerPendingXhrContinue,
    response_code: u16,
    response_headers: Vec<(String, String)>,
    response_body: RendererSyntheticResponseBody,
) {
    let xhr_id = request.xhr_id;
    let completion = {
        let mut state = state.borrow_mut();
        let completion_tx = state.xhr_completion_tx.clone();
        let Some(pending) = state.pending_xhrs.get_mut(&xhr_id) else {
            return;
        };
        if let Some(record) = pending.network_record.as_mut() {
            record.intercept_response = false;
        }
        pending.network_request_handle = request.network_request_handle;
        pending.network_record = None;
        pending.paused_response = None;
        let response =
            worker_response_from_body(request.url, response_code, response_headers, response_body);
        (completion_tx, response, xhr_id)
    };
    let _ = completion.0.send(WorkerXhrCompletion {
        xhr_id: completion.2,
        network_request_headers: None,
        result: Ok(WorkerXhrResponse::Materialized(Box::new(completion.1))),
    });
}

pub(in crate::worker) fn record_worker_websocket_subresource_failure(
    state: &WorkerGlobalState,
    socket_id: u64,
    document_url: Url,
    url: Url,
    request_headers: Vec<(String, String)>,
    error_text: String,
) {
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::WebSocketSubresource(
            SubresourceNetworkRecord::failure(
                None,
                document_url,
                url,
                "GET".to_owned(),
                request_headers,
                None,
                SubresourceResourceType::WebSocket,
                error_text,
            )
            .with_websocket_socket_id(socket_id),
        ));
}

#[must_use = "the caller must stop mixed-content processing when worker CSP blocks the request"]
pub(crate) enum WorkerWebSocketCspOutcome {
    Allowed,
    Blocked(String),
}

impl WorkerWebSocketCspOutcome {
    pub(crate) fn blocks_request(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }

    fn into_failure_message(self) -> Option<String> {
        match self {
            Self::Allowed => None,
            Self::Blocked(message) => Some(message),
        }
    }
}

pub(crate) fn check_worker_websocket_csp(
    scope: &mut v8::PinScope<'_, '_>,
    document_url: &Url,
    url: &Url,
) -> Option<WorkerWebSocketCspOutcome> {
    let state = get_worker_state(scope)?;
    dispatch_worker_content_security_policy_report_only_violation_for_state(
        scope,
        &state,
        document_url,
        url,
        crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
    );
    let violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_violation(
            &state_ref,
            document_url,
            url,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
        )
    };
    Some(match violation {
        Some(violation) => {
            let message = worker_content_security_policy_error_message(&violation, "WebSocket");
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, &state, &violation,
            );
            WorkerWebSocketCspOutcome::Blocked(message)
        }
        None => WorkerWebSocketCspOutcome::Allowed,
    })
}

pub(crate) fn register_worker_websocket<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    document_url: Url,
    url: Url,
    protocols: Vec<String>,
    csp_outcome: WorkerWebSocketCspOutcome,
) -> Option<u64> {
    let state = get_worker_state(scope)?;
    let (
        socket_id,
        loader,
        extra_http_headers,
        network_offline,
        blocked_url_patterns,
        websocket_event_tx,
    ) = {
        let mut state = state.borrow_mut();
        let socket_id = next_websocket_id(&mut state);
        (
            socket_id,
            state.loader.clone(),
            state.extra_http_headers.clone(),
            state.network_offline,
            state.blocked_url_patterns.clone(),
            state.websocket_event_tx.clone(),
        )
    };

    let cookie_url = websocket_cookie_url(&url);
    let cookie_context = moli_cookie_jar::NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&cookie_url, &document_url);
    let cookie_header = moli_fetch::cookie_header_for_request(
        &loader.request_client().cookie_store(),
        &cookie_url,
        cookie_context,
    );
    let cookie_header_for_context = cookie_header
        .as_ref()
        .ok()
        .and_then(|header| header.clone());
    let context = WebSocketConnectOptions {
        origin: moli_url::origin_ascii_serialization(&document_url),
        user_agent: loader.request_client().user_agent().to_owned(),
        extra_headers: extra_http_headers,
        http_proxy: loader.request_client().http_proxy().map(ToOwned::to_owned),
        http_no_proxy: loader
            .request_client()
            .http_no_proxy()
            .map(ToOwned::to_owned),
        proxy_bearer_token: loader
            .request_client()
            .proxy_bearer_token()
            .map(ToOwned::to_owned),
        tls_verify_host: loader.request_client().tls_verify_host(),
        cookie_header: cookie_header_for_context,
        pause_after_handshake: false,
    };

    let csp_failure = csp_outcome.into_failure_message();
    let blocked = worker_url_blocked(&blocked_url_patterns, &url);
    let failure_message = if csp_failure.is_some() {
        csp_failure
    } else if blocked {
        Some(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else if network_offline {
        Some("Network emulation offline".to_owned())
    } else {
        cookie_header
            .err()
            .map(|error| format!("failed to build WebSocket cookie header: {error}"))
    };
    let network_recorded = failure_message.is_some();
    let (command_tx, load) = if let Some(error_text) = failure_message {
        record_worker_websocket_subresource_failure(
            &state.borrow(),
            socket_id,
            document_url.clone(),
            url.clone(),
            context.extra_headers.clone(),
            error_text.clone(),
        );
        (
            spawn_failed_connection(socket_id, error_text, websocket_event_tx),
            None,
        )
    } else {
        let load = loader.register_load(
            ResourceLoadKind::WebSocket,
            ResourceLoadDisposition::Ordinary,
            None,
        )?;
        let command_tx = spawn_connection(
            socket_id,
            url.to_string(),
            protocols,
            context,
            websocket_event_tx,
        );
        let cancel_tx = command_tx.clone();
        load.attach_consumer_cancel(move || {
            let _ = cancel_tx.send(WebSocketCommand::Close {
                code: Some(1001),
                reason: "WorkerGlobalScope terminated".to_owned(),
            });
        });
        (command_tx, Some(load))
    };

    state.borrow_mut().websockets.insert(
        socket_id,
        WorkerWebSocketState {
            wrapper: v8::Global::new(scope, wrapper),
            command_tx,
            document_url,
            url,
            loader,
            load,
            opened: false,
            network_recorded,
        },
    );
    Some(socket_id)
}

pub(crate) fn send_worker_websocket_text(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    text: String,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let command_tx = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .command_tx
        .clone();
    Some(command_tx.send(WebSocketCommand::SendText(text)).is_ok())
}

pub(crate) fn send_worker_websocket_binary(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    bytes: Vec<u8>,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let command_tx = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .command_tx
        .clone();
    Some(command_tx.send(WebSocketCommand::SendBinary(bytes)).is_ok())
}

pub(crate) fn close_worker_websocket(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    code: Option<u16>,
    reason: String,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let command_tx = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .command_tx
        .clone();
    Some(
        command_tx
            .send(WebSocketCommand::Close { code, reason })
            .is_ok(),
    )
}

pub(in crate::worker) fn dispatch_worker_websocket_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    event: WebSocketEvent,
) -> bool {
    let socket_id = match &event {
        WebSocketEvent::HandshakeResponse { socket_id, .. }
        | WebSocketEvent::Open { socket_id, .. }
        | WebSocketEvent::TextMessage { socket_id, .. }
        | WebSocketEvent::BinaryMessage { socket_id, .. }
        | WebSocketEvent::FrameSent { socket_id, .. }
        | WebSocketEvent::BufferedAmountConsumed { socket_id, .. }
        | WebSocketEvent::Error { socket_id, .. }
        | WebSocketEvent::Closing { socket_id }
        | WebSocketEvent::Close { socket_id, .. } => *socket_id,
    };
    let socket_context = {
        let mut state = state.borrow_mut();
        let parent_tx = state.parent_tx.clone();
        let socket_context = state.websockets.get_mut(&socket_id).map(|entry| {
            let was_opened = entry.opened;
            let mut should_record_error_failure = false;
            match &event {
                WebSocketEvent::Open { .. } => {
                    entry.opened = true;
                    entry.network_recorded = true;
                }
                WebSocketEvent::Error { .. } if !entry.opened && !entry.network_recorded => {
                    entry.network_recorded = true;
                    should_record_error_failure = true;
                }
                _ => {}
            }
            (
                v8::Local::new(scope, &entry.wrapper),
                entry.document_url.clone(),
                entry.url.clone(),
                entry.loader.clone(),
                was_opened,
                should_record_error_failure,
                parent_tx,
            )
        });
        if socket_context.is_none() {
            state.websockets.remove(&socket_id);
        }
        socket_context
    };
    let Some((
        socket,
        document_url,
        socket_url,
        loader,
        was_opened,
        should_record_error_failure,
        parent_tx,
    )) = socket_context
    else {
        return false;
    };

    let mut parent_messages = Vec::new();
    match &event {
        WebSocketEvent::Open {
            request_headers,
            response_status,
            response_headers,
            ..
        } => {
            let cookie_set_reports = store_worker_websocket_response_cookies(
                socket_id,
                &socket_url,
                &loader,
                response_headers,
            );
            parent_messages.push(WorkerToParentMessage::WebSocketLifecycle(
                WorkerWebSocketLifecycleEvent::Open {
                    socket_id,
                    document_url: document_url.clone(),
                    url: socket_url.clone(),
                },
            ));
            parent_messages.push(WorkerToParentMessage::WebSocketSubresource(
                SubresourceNetworkRecord::success(
                    None,
                    document_url.clone(),
                    socket_url.clone(),
                    "GET".to_owned(),
                    request_headers.clone(),
                    None,
                    SubresourceResourceType::WebSocket,
                    None,
                    Vec::new(),
                    socket_url.clone(),
                    *response_status,
                    response_headers.clone(),
                    String::new(),
                    cookie_set_reports,
                )
                .with_websocket_socket_id(socket_id),
            ));
        }
        WebSocketEvent::FrameSent {
            opcode,
            payload_length,
            ..
        } => {
            parent_messages.push(WorkerToParentMessage::WebSocketFrame(
                WorkerWebSocketFrameEvent {
                    socket_id,
                    document_url: document_url.clone(),
                    url: socket_url.clone(),
                    direction: WebSocketFrameDirection::Sent,
                    opcode: worker_websocket_frame_opcode(*opcode),
                    payload_length: *payload_length,
                },
            ));
        }
        WebSocketEvent::Error { message, .. } => {
            parent_messages.push(WorkerToParentMessage::WebSocketLifecycle(
                WorkerWebSocketLifecycleEvent::Error {
                    socket_id,
                    document_url: document_url.clone(),
                    url: socket_url.clone(),
                    error_text: message.clone(),
                },
            ));
            if !was_opened && should_record_error_failure {
                parent_messages.push(WorkerToParentMessage::WebSocketSubresource(
                    SubresourceNetworkRecord::failure(
                        None,
                        document_url.clone(),
                        socket_url.clone(),
                        "GET".to_owned(),
                        Vec::new(),
                        None,
                        SubresourceResourceType::WebSocket,
                        message.clone(),
                    )
                    .with_websocket_socket_id(socket_id),
                ));
            }
        }
        WebSocketEvent::Closing { .. } => {
            parent_messages.push(WorkerToParentMessage::WebSocketLifecycle(
                WorkerWebSocketLifecycleEvent::Closing {
                    socket_id,
                    document_url: document_url.clone(),
                    url: socket_url.clone(),
                },
            ));
        }
        WebSocketEvent::Close {
            code,
            reason,
            was_clean,
            ..
        } => {
            parent_messages.push(WorkerToParentMessage::WebSocketLifecycle(
                WorkerWebSocketLifecycleEvent::Close {
                    socket_id,
                    document_url: document_url.clone(),
                    url: socket_url.clone(),
                    code: *code,
                    reason: reason.clone(),
                    was_clean: *was_clean,
                },
            ));
        }
        WebSocketEvent::HandshakeResponse { .. }
        | WebSocketEvent::TextMessage { .. }
        | WebSocketEvent::BinaryMessage { .. }
        | WebSocketEvent::BufferedAmountConsumed { .. } => {}
    }
    for message in parent_messages {
        let _ = parent_tx.send(message);
    }

    let dispatch_result = crate::context_bootstrap::dispatch_websocket_event(scope, socket, &event);
    if !matches!(
        dispatch_result,
        crate::context_bootstrap::WebSocketDispatchResult::Backpressured
    ) {
        let frame_event = match &event {
            WebSocketEvent::TextMessage { data, .. } => Some(WorkerWebSocketFrameEvent {
                socket_id,
                document_url: document_url.clone(),
                url: socket_url.clone(),
                direction: WebSocketFrameDirection::Received,
                opcode: WebSocketFrameOpcode::Text,
                payload_length: data.len(),
            }),
            WebSocketEvent::BinaryMessage { data, .. } => Some(WorkerWebSocketFrameEvent {
                socket_id,
                document_url,
                url: socket_url,
                direction: WebSocketFrameDirection::Received,
                opcode: WebSocketFrameOpcode::Binary,
                payload_length: data.len(),
            }),
            _ => None,
        };
        if let Some(frame_event) = frame_event {
            let _ = parent_tx.send(WorkerToParentMessage::WebSocketFrame(frame_event));
        }
    }
    if matches!(event, WebSocketEvent::Close { .. })
        && let Some(socket) = state.borrow_mut().websockets.remove(&socket_id)
        && let Some(load) = socket.load
    {
        load.finish();
    }
    dispatch_result.dispatched()
}

pub(in crate::worker) fn worker_websocket_frame_opcode(
    opcode: moli_websocket::FrameOpcode,
) -> WebSocketFrameOpcode {
    match opcode {
        moli_websocket::FrameOpcode::Text => WebSocketFrameOpcode::Text,
        moli_websocket::FrameOpcode::Binary => WebSocketFrameOpcode::Binary,
    }
}

pub(in crate::worker) fn store_worker_websocket_response_cookies(
    _socket_id: u64,
    socket_url: &Url,
    request_client: &crate::network::context::WorkerResourceLoader,
    response_headers: &[(String, String)],
) -> Vec<StoredCookieSetReport> {
    if !response_headers
        .iter()
        .any(|(name, _)| response_header_name_is(name, &HeaderName::from_static("set-cookie")))
    {
        return Vec::new();
    }
    let response_cookie_url = websocket_cookie_url(socket_url);
    let cookie_store = request_client.request_client().cookie_store();
    let mut store = cookie_store.lock();
    store.store_response_headers_with_reports(&response_cookie_url, response_headers)
}

pub(in crate::worker) fn response_header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}

pub(in crate::worker) fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}

pub(in crate::worker) fn make_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("worker fetch resolver");
    let promise = resolver.get_promise(scope);
    if let Some(message) = v8_string(scope, message) {
        resolver.reject(scope, v8::Exception::type_error(scope, message));
    } else {
        resolver.reject(scope, v8::undefined(scope).into());
    }
    promise
}

pub(in crate::worker) fn make_rejected_promise_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("worker fetch resolver");
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    promise
}

pub(in crate::worker) fn validate_worker_fetch_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Object>>, String> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(
            "Failed to execute 'fetch' on 'DedicatedWorkerGlobalScope': signal must be an AbortSignal."
                .to_owned(),
        );
    };
    if worker_abort_signal_id(scope, signal).is_none() {
        return Err(
            "Failed to execute 'fetch' on 'DedicatedWorkerGlobalScope': signal must be an AbortSignal."
                .to_owned(),
        );
    }
    Ok(Some(signal))
}

pub(in crate::worker) fn worker_fetch_signal_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    request_like: Option<v8::Local<'s, v8::Object>>,
) -> Result<Option<v8::Local<'s, v8::Object>>, String> {
    let signal_key = v8str(scope, "signal");
    if args.length() > 1 {
        let init_arg = args.get(1);
        if !init_arg.is_null_or_undefined()
            && let Ok(init) = v8::Local::<v8::Object>::try_from(init_arg)
            && init.has(scope, signal_key.into()).unwrap_or(false)
        {
            let signal = init
                .get(scope, signal_key.into())
                .unwrap_or_else(|| v8::undefined(scope).into());
            return validate_worker_fetch_signal(scope, signal);
        }
    }
    if let Some(request_like) = request_like
        && let Some(signal) = request_like.get(scope, signal_key.into())
    {
        return validate_worker_fetch_signal(scope, signal);
    }
    Ok(None)
}

pub(in crate::worker) struct ResolvedWorkerFetchInput<'s> {
    resolved_url: Url,
    method: String,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    suppress_default_content_type: bool,
    request_mode: moli_fetch::RequestMode,
    credentials_mode: RequestCredentialsMode,
    redirect_mode: RequestRedirectMode,
    priority: Option<moli_fetch::FetchPriorityHint>,
    metadata: ServiceWorkerFetchRequestMetadata,
    signal: Option<v8::Local<'s, v8::Object>>,
}

pub(in crate::worker) fn resolve_worker_fetch_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    base_url: &Url,
) -> Result<ResolvedWorkerFetchInput<'s>, String> {
    if args.length() < 1 {
        return Err(
            webidl::WebIdlError::missing_required(webidl::Context::argument("fetch", 1))
                .to_string(),
        );
    }
    let arg0 = args.get(0);
    let mut request_like = None;
    let mut consumes_request_body = false;
    let inherited = request_input_snapshot(scope, arg0).map_err(|error| error.to_string())?;
    let (
        url_input,
        method,
        body,
        headers,
        suppress_default_content_type,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        metadata,
    ) = if let Some(inherited) = inherited {
        let req_obj = v8::Local::<v8::Object>::try_from(arg0).expect("request-like object");
        request_like = Some(req_obj);
        let url = inherited.url.clone();
        let init = parse_fetch_init(scope, args, 1)?;
        consumes_request_body = !init.body_present && inherited.body.is_some();
        let method = if init.method_present {
            init.method.clone()
        } else {
            inherited.method.clone()
        };
        let body = if init.body_present {
            init.body.clone()
        } else {
            inherited.body.clone()
        };
        let mut headers = if init.headers_present {
            init.headers.clone()
        } else {
            inherited.headers.clone()
        };
        let suppress_default_content_type = if init.body_present {
            if !init.headers_present {
                append_default_body_content_type(&mut headers, init.body_content_type.as_deref());
            }
            init.suppress_default_content_type
                || (body.is_some()
                    && init.body_content_type.is_none()
                    && !has_header(&headers, "content-type"))
        } else {
            body.is_some() && !has_header(&headers, "content-type")
        };
        let inherited_credentials = request_object_credentials_mode(scope, req_obj)?;
        let request_mode = init
            .request_mode
            .or_else(|| moli_fetch::RequestMode::from_str(&inherited.mode).ok())
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_worker_no_cors_method(request_mode, &method)?;
        let headers = if request_mode == moli_fetch::RequestMode::NoCors {
            filter_headers_for_guard(&headers, HeadersGuard::RequestNoCors)
        } else {
            headers
        };
        let credentials_mode = init
            .credentials_mode
            .or(inherited_credentials)
            .unwrap_or(RequestCredentialsMode::SameOrigin);
        let redirect_mode = init
            .redirect_mode
            .or_else(|| crate::network_host::parse_request_redirect_mode_label(&inherited.redirect))
            .unwrap_or(RequestRedirectMode::Follow);
        let priority = init.priority.or_else(|| {
            (inherited.priority != moli_fetch::FetchPriorityHint::Auto)
                .then_some(inherited.priority)
        });
        let metadata = ServiceWorkerFetchRequestMetadata {
            cache: init.cache.unwrap_or(inherited.cache),
            referrer: init.referrer.unwrap_or(inherited.referrer),
            referrer_policy: init.referrer_policy.unwrap_or(inherited.referrer_policy),
            integrity: init.integrity.unwrap_or(inherited.integrity),
            keepalive: init.keepalive.unwrap_or(inherited.keepalive),
        };
        (
            url,
            method,
            body,
            headers,
            suppress_default_content_type,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            metadata,
        )
    } else {
        let url = webidl::convert::<webidl::UsvString>(
            scope,
            arg0,
            webidl::Context::argument("fetch", 1),
        )
        .map(String::from)
        .map_err(|error| error.to_string())?;
        let init = parse_fetch_init(scope, args, 1)?;
        let request_mode = init.request_mode.unwrap_or(moli_fetch::RequestMode::Cors);
        validate_worker_no_cors_method(request_mode, &init.method)?;
        let headers = if request_mode == moli_fetch::RequestMode::NoCors {
            filter_headers_for_guard(&init.headers, HeadersGuard::RequestNoCors)
        } else {
            init.headers
        };
        let credentials_mode = init
            .credentials_mode
            .unwrap_or(RequestCredentialsMode::SameOrigin);
        let redirect_mode = init.redirect_mode.unwrap_or(RequestRedirectMode::Follow);
        let metadata = ServiceWorkerFetchRequestMetadata {
            cache: init.cache.unwrap_or_else(|| "default".to_owned()),
            referrer: init.referrer.unwrap_or_else(|| "about:client".to_owned()),
            referrer_policy: init.referrer_policy.unwrap_or_default(),
            integrity: init.integrity.unwrap_or_default(),
            keepalive: init.keepalive.unwrap_or(false),
        };
        (
            url,
            init.method,
            init.body,
            headers,
            init.suppress_default_content_type,
            request_mode,
            credentials_mode,
            redirect_mode,
            init.priority,
            metadata,
        )
    };
    let resolved_url = resolve_context_url(base_url, &url_input, None)?;
    let signal = worker_fetch_signal_option(scope, args, request_like)?;
    if consumes_request_body && let Some(request_like) = request_like {
        crate::network_host::mark_request_input_body_used_for_fetch(scope, request_like);
    }
    Ok(ResolvedWorkerFetchInput {
        resolved_url,
        method,
        body,
        headers,
        suppress_default_content_type,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        metadata,
        signal,
    })
}

pub(in crate::worker) fn validate_worker_no_cors_method(
    request_mode: RequestMode,
    method: &str,
) -> Result<(), String> {
    if request_mode == RequestMode::NoCors && !moli_fetch::is_cors_safelisted_method(method) {
        return Err(format!(
            "Failed to execute 'fetch' on 'DedicatedWorkerGlobalScope': method `{method}` is unsupported in no-cors mode."
        ));
    }
    Ok(())
}

pub(in crate::worker) fn worker_fetch_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        rv.set(make_rejected_promise(scope, "fetch: worker runtime state is unavailable").into());
        return;
    };

    let (
        loader,
        document_url,
        extra_http_headers,
        network_offline,
        blocked_url_patterns,
        fetch_subresource_interception_enabled,
        fetch_subresource_interception_resource_type,
        policy_context,
    ) = {
        let state = state.borrow();
        let Some(document_url) = state.current_script_url.clone() else {
            rv.set(make_rejected_promise(scope, "fetch: worker script url is unavailable").into());
            return;
        };
        (
            state.loader.clone(),
            document_url,
            state.extra_http_headers.clone(),
            state.network_offline,
            state.blocked_url_patterns.clone(),
            state.fetch_subresource_interception_enabled,
            state.fetch_subresource_interception_resource_type,
            state.policy_context,
        )
    };

    let ResolvedWorkerFetchInput {
        resolved_url,
        method,
        body,
        headers: request_headers,
        suppress_default_content_type,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        metadata: request_metadata,
        signal,
    } = match resolve_worker_fetch_input(scope, &args, &document_url) {
        Ok(resolved) => resolved,
        Err(message) => {
            rv.set(make_rejected_promise(scope, &message).into());
            return;
        }
    };
    let headers = merge_worker_request_headers(&extra_http_headers, &request_headers);
    if let Some(signal) = signal
        && worker_abort_signal_aborted(scope, signal)
    {
        let reason = worker_abort_signal_reason(scope, signal)
            .unwrap_or_else(|| worker_abort_error_value(scope));
        rv.set(make_rejected_promise_with_value(scope, reason).into());
        return;
    }

    dispatch_worker_content_security_policy_report_only_violation_for_state(
        scope,
        &state,
        &document_url,
        &resolved_url,
        crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
    );
    let csp_violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_violation(
            &state_ref,
            &document_url,
            &resolved_url,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
        )
    };
    if let Some(violation) = csp_violation {
        dispatch_worker_content_security_policy_violation_event_for_state(
            scope, &state, &violation,
        );
        let message = worker_content_security_policy_error_message(&violation, "fetch");
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            SubresourceResourceType::Fetch,
            message.clone(),
        );
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if let Err(error) = moli_url_policy::route_fetch_url(&resolved_url) {
        let message = error.to_string();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            SubresourceResourceType::Fetch,
            message.clone(),
        );
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if should_request_be_blocked_due_to_bad_port(&resolved_url) {
        let message = format!("fetch: blocked bad port for `{resolved_url}`");
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            SubresourceResourceType::Fetch,
            message.clone(),
        );
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if worker_url_blocked(&blocked_url_patterns, &resolved_url) {
        let message = BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            SubresourceResourceType::Fetch,
            message.clone(),
        );
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if fetch_subresource_interception_enabled
        && fetch_subresource_interception_resource_type.is_none_or(|expected| {
            expected.has_same_cdp_fetch_interception_type(SubresourceResourceType::Fetch)
        })
    {
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            rv.set_undefined();
            return;
        };
        let promise = resolver.get_promise(scope);
        let signal_id = signal.and_then(|signal| worker_abort_signal_id(scope, signal));
        let cancel_handle = FetchCancelHandle::new();
        let disposition = if request_metadata.keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        };
        let Some(load) = loader.register_load(
            ResourceLoadKind::Fetch,
            disposition,
            Some(cancel_handle.clone()),
        ) else {
            rv.set(make_rejected_promise(scope, "fetch: worker global is shutting down").into());
            return;
        };
        let request_body = request_body_text(&body);
        let fetch_id = {
            let mut state = state.borrow_mut();
            let fetch_id = next_fetch_id(&mut state);
            state.pending_fetches.insert(
                fetch_id,
                PendingWorkerFetch {
                    resolver: v8::Global::new(scope, resolver),
                    document_url: document_url.clone(),
                    credentials_mode,
                    request_mode,
                    redirect_mode,
                    request_priority: priority,
                    request_metadata: request_metadata.clone(),
                    policy_context,
                    signal_id,
                    load: load.clone(),
                    request_url: resolved_url.clone(),
                    request_method: method.clone(),
                    request_headers: headers.clone(),
                    request_body: request_body.clone(),
                    network_request_handle: None,
                    network_record: None,
                    paused_response: None,
                    streaming_body_source_id: None,
                },
            );
            fetch_id
        };
        let info = PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url,
            url: resolved_url,
            websocket_socket_id: None,
            method,
            request_headers: headers,
            request_body_bytes: request_body.as_ref().map(|body| body.as_bytes().to_vec()),
            request_body,
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: None,
        };
        let _ = state
            .borrow()
            .parent_tx
            .send(WorkerToParentMessage::PendingSubresourceFetch(
                WorkerPendingSubresourceFetch {
                    fetch_id,
                    load,
                    credentials_mode,
                    request_mode,
                    network_partition_key: state.borrow().network_partition_key.clone(),
                    info,
                },
            ));
        rv.set(promise.into());
        return;
    }

    if network_offline {
        let message = "Network emulation offline".to_owned();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            SubresourceResourceType::Fetch,
            message.clone(),
        );
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    let promise = {
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            rv.set_undefined();
            return;
        };
        let promise = resolver.get_promise(scope);
        let signal_id = signal.and_then(|signal| worker_abort_signal_id(scope, signal));
        let cancel_handle = FetchCancelHandle::new();
        let disposition = if request_metadata.keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        };
        let Some(load) = loader.register_load(
            ResourceLoadKind::Fetch,
            disposition,
            Some(cancel_handle.clone()),
        ) else {
            rv.set(make_rejected_promise(scope, "fetch: worker global is shutting down").into());
            return;
        };
        let request_body = request_body_text(&body);
        let fetch_id = {
            let mut state = state.borrow_mut();
            let fetch_id = next_fetch_id(&mut state);
            state.pending_fetches.insert(
                fetch_id,
                PendingWorkerFetch {
                    resolver: v8::Global::new(scope, resolver),
                    document_url: document_url.clone(),
                    credentials_mode,
                    request_mode,
                    redirect_mode,
                    request_priority: priority,
                    request_metadata: request_metadata.clone(),
                    policy_context,
                    signal_id,
                    load: load.clone(),
                    request_url: resolved_url.clone(),
                    request_method: method.clone(),
                    request_headers: headers.clone(),
                    request_body,
                    network_request_handle: None,
                    network_record: None,
                    paused_response: None,
                    streaming_body_source_id: None,
                },
            );
            fetch_id
        };
        let (completion_tx, referrer_policy, network_partition_key, service_worker_controller) = {
            let state = state.borrow();
            (
                state.fetch_completion_tx.clone(),
                state.referrer_policy.clone(),
                state.network_partition_key.clone(),
                worker_service_worker_controller(&state, &resolved_url),
            )
        };

        if let Some((service_worker_runtime, service_worker_client_id)) = service_worker_controller
        {
            spawn_worker_fetch_service_worker(
                service_worker_runtime,
                service_worker_client_id,
                load,
                completion_tx,
                fetch_id,
                cancel_handle,
                document_url,
                referrer_policy,
                network_partition_key,
                policy_context,
                resolved_url,
                method,
                body,
                headers,
                request_mode,
                credentials_mode,
                redirect_mode,
                priority,
                request_metadata,
                suppress_default_content_type,
            );
        } else {
            spawn_worker_fetch_network(
                load,
                completion_tx,
                fetch_id,
                cancel_handle,
                document_url,
                referrer_policy,
                network_partition_key,
                resolved_url,
                method,
                body,
                headers,
                request_mode,
                credentials_mode,
                redirect_mode,
                priority,
                request_metadata,
                None,
                suppress_default_content_type,
                true,
            );
        }

        promise
    };
    rv.set(promise.into());
}

pub(in crate::worker) fn reject_worker_fetches_for_signal(
    scope: &mut v8::PinScope<'_, '_>,
    signal_id: u32,
    reason: v8::Local<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let rejected = {
        let mut state = state.borrow_mut();
        let fetch_ids = state
            .pending_fetches
            .iter()
            .filter_map(|(fetch_id, pending)| {
                (pending.signal_id == Some(signal_id)).then_some(*fetch_id)
            })
            .collect::<Vec<_>>();
        let mut rejected = Vec::with_capacity(fetch_ids.len());
        for fetch_id in fetch_ids {
            if let Some(pending) = state.pending_fetches.remove(&fetch_id) {
                rejected.push((
                    fetch_id,
                    pending.resolver,
                    pending.load,
                    pending.document_url,
                    pending.network_record,
                    pending.network_request_handle,
                    pending.streaming_body_source_id,
                ));
            }
        }
        rejected
    };
    for (
        fetch_id,
        resolver,
        load,
        document_url,
        network_record,
        network_request_handle,
        body_source_id,
    ) in rejected
    {
        load.cancel();
        if let Some(runtime) = state.borrow().service_worker_runtime.clone() {
            runtime.abort_controlled_fetch(u64::from(fetch_id));
        }
        if let Some(record) = network_record {
            let state = state.borrow();
            record_worker_subresource_failure_with_handle(
                &state,
                network_request_handle,
                document_url,
                record.url,
                record.method,
                record.request_headers,
                record.request_body,
                SubresourceResourceType::Fetch,
                ABORTED_ERROR_TEXT.to_owned(),
            );
            let _ = state
                .parent_tx
                .send(WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: record.internal_id,
                    },
                ));
        } else {
            let _ = state.borrow().parent_tx.send(
                WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id,
                    error_text: ABORTED_ERROR_TEXT.to_owned(),
                },
            );
        }
        if let Some(body_source_id) = body_source_id {
            let abort_reason = worker_abort_error_value(scope);
            error_pending_network_body_stream_with_reason(
                scope,
                body_source_id,
                ABORTED_ERROR_TEXT.to_owned(),
                abort_reason,
            );
        } else {
            let resolver = v8::Local::new(scope, &resolver);
            let _ = resolver.reject(scope, reason);
        }
    }
}

pub(in crate::worker) fn worker_response_from_body(
    final_url: Url,
    status: u16,
    headers: Vec<(String, String)>,
    body: RendererSyntheticResponseBody,
) -> Response {
    body.into_fetch_response(ResponseHead {
        final_url,
        status,
        headers,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    })
}

pub(in crate::worker) fn drain_worker_fetch_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    event: WorkerFetchEvent,
) {
    match event {
        WorkerFetchEvent::Completion(completion) => {
            drain_worker_fetch_completion_result(scope, state, *completion)
        }
        WorkerFetchEvent::StreamingStarted(started) => {
            start_worker_streaming_fetch(scope, state, started)
        }
        WorkerFetchEvent::StreamingChunk(chunk) => {
            enqueue_pending_network_body_chunk(scope, chunk.body_source_id, chunk.bytes)
        }
        WorkerFetchEvent::StreamingFinished(finished) => {
            finish_worker_streaming_fetch(scope, state, finished)
        }
    }
}

pub(in crate::worker) fn start_worker_streaming_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    started: WorkerFetchStreamingStarted,
) {
    let mut reject = None;
    if let Some(network_request_headers) = started.network_request_headers.as_ref()
        && let Some(record) = state
            .borrow_mut()
            .pending_fetches
            .get_mut(&started.fetch_id)
            .and_then(|pending| pending.network_record.as_mut())
    {
        record
            .initial_network_request_headers
            .get_or_insert_with(|| network_request_headers.clone());
    }
    let redirect_status = if started.head.redirect_chain.is_empty() {
        crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
    } else {
        crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
    };
    if redirect_status
        == crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
    {
        let (document_url, request_url) = {
            let state_ref = state.borrow();
            let Some(pending) = state_ref.pending_fetches.get(&started.fetch_id) else {
                return;
            };
            (pending.document_url.clone(), pending.request_url.clone())
        };
        dispatch_worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status_for_state(
            scope,
            state,
            &document_url,
            &started.head.final_url,
            &request_url,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
            redirect_status,
        );
    }
    let csp_failure = {
        let state_ref = state.borrow();
        let Some(pending) = state_ref.pending_fetches.get(&started.fetch_id) else {
            return;
        };
        worker_content_security_policy_violation_for_checked_url_with_redirect_status(
            &state_ref,
            &pending.document_url,
            &started.head.final_url,
            &pending.request_url,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
            redirect_status,
        )
    };
    let response_input = if let Some(violation) = csp_failure {
        let resolver = {
            let mut state_ref = state.borrow_mut();
            let Some(pending) = state_ref.pending_fetches.get_mut(&started.fetch_id) else {
                return;
            };
            pending.load.cancel();
            pending.resolver.clone()
        };
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
        let message = worker_content_security_policy_error_message(&violation, "fetch");
        reject = Some((resolver, message));
        None
    } else {
        let mut state_ref = state.borrow_mut();
        let Some(pending) = state_ref.pending_fetches.get_mut(&started.fetch_id) else {
            return;
        };
        if let Err(message) = validate_fetch_response_security_policy(
            &pending.document_url,
            &started.head.final_url,
            &started.head.headers,
            pending.request_mode,
            pending.credentials_mode,
            pending.policy_context,
        ) {
            pending.load.cancel();
            reject = Some((pending.resolver.clone(), message));
            None
        } else {
            let mut observable_head = started.head.clone();
            observable_head.headers = filter_cors_exposed_response_headers(
                &pending.document_url,
                &observable_head.final_url,
                &observable_head.headers,
                pending.credentials_mode,
            );
            pending.streaming_body_source_id = Some(started.body_source_id);
            Some((
                pending.resolver.clone(),
                pending.document_url.clone(),
                pending.request_mode,
                observable_head,
            ))
        }
    };
    if let Some((resolver, message)) = reject {
        let pending = state.borrow_mut().pending_fetches.remove(&started.fetch_id);
        if let Some(pending) = pending {
            record_worker_fetch_failure(&state.borrow(), &pending, message.clone());
        }
        if let Some(message) = v8_string(scope, &message) {
            let resolver = v8::Local::new(scope, &resolver);
            let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        }
        return;
    }
    if let Some((resolver, document_url, request_mode, observable_head)) = response_input {
        let response_obj = build_fetch_response_object_from_stream_for_request_mode(
            scope,
            &document_url,
            request_mode,
            observable_head,
            started.body_source_id,
        );
        let resolver = v8::Local::new(scope, &resolver);
        let _ = resolver.resolve(scope, response_obj.into());
    }
}

pub(in crate::worker) fn finish_worker_streaming_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    finished: WorkerFetchStreamingFinished,
) {
    let parent_tx = state.borrow().parent_tx.clone();
    let Some(pending) = state
        .borrow_mut()
        .pending_fetches
        .remove(&finished.fetch_id)
    else {
        return;
    };
    pending.load.finish();
    let head = finished.head;
    match finished.result {
        Ok(body) => {
            close_pending_network_body_stream(scope, finished.body_source_id);
            if let Some(record) = pending.network_record {
                let internal_id = record.internal_id;
                record_worker_subresource_success_with_handle(
                    &state.borrow(),
                    pending.network_request_handle,
                    pending.document_url,
                    record.url,
                    record.method,
                    record.request_headers,
                    record.request_body,
                    SubresourceResourceType::Fetch,
                    record.initial_network_request_headers,
                    head,
                    body,
                );
                let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed { internal_id },
                ));
            }
        }
        Err(error_text) => {
            let reason = v8_string(scope, &error_text)
                .map(|message| v8::Exception::type_error(scope, message))
                .unwrap_or_else(|| v8::undefined(scope).into());
            error_pending_network_body_stream_with_reason(
                scope,
                finished.body_source_id,
                error_text.clone(),
                reason,
            );
            record_worker_fetch_failure(&state.borrow(), &pending, error_text);
        }
    }
}

pub(in crate::worker) fn record_worker_fetch_failure(
    state: &WorkerGlobalState,
    pending: &PendingWorkerFetch,
    message: String,
) {
    let network_error_text = if is_cors_policy_failure_message(&message) {
        FAILED_ERROR_TEXT.to_owned()
    } else {
        message
    };
    if let Some(record) = pending.network_record.as_ref() {
        record_worker_subresource_failure_with_handle(
            state,
            pending.network_request_handle,
            pending.document_url.clone(),
            record.url.clone(),
            record.method.clone(),
            record.request_headers.clone(),
            record.request_body.clone(),
            SubresourceResourceType::Fetch,
            network_error_text,
        );
        let _ = state
            .parent_tx
            .send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::Completed {
                    internal_id: record.internal_id,
                },
            ));
        return;
    }

    record_worker_subresource_failure_with_handle(
        state,
        pending.network_request_handle,
        pending.document_url.clone(),
        pending.request_url.clone(),
        pending.request_method.clone(),
        pending.request_headers.clone(),
        pending.request_body.clone(),
        SubresourceResourceType::Fetch,
        network_error_text,
    );
}

pub(in crate::worker) fn drain_worker_fetch_completion_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerFetchCompletion,
) {
    let global = scope.get_current_context().global(scope);
    let _ = global;
    let parent_tx = state.borrow().parent_tx.clone();
    if let Some(network_request_headers) = completion.network_request_headers.as_ref()
        && let Some(record) = state
            .borrow_mut()
            .pending_fetches
            .get_mut(&completion.fetch_id)
            .and_then(|pending| pending.network_record.as_mut())
    {
        record
            .initial_network_request_headers
            .get_or_insert_with(|| network_request_headers.clone());
    }
    if let Ok(response) = &completion.result {
        let response_head = response.head();
        let redirect_status = if response_head.redirect_chain.is_empty() {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
        } else {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        };
        if redirect_status
            == crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        {
            let (document_url, request_url) = {
                let state_ref = state.borrow();
                let Some(pending) = state_ref.pending_fetches.get(&completion.fetch_id) else {
                    return;
                };
                (pending.document_url.clone(), pending.request_url.clone())
            };
            dispatch_worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status_for_state(
                scope,
                state,
                &document_url,
                &response_head.final_url,
                &request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                redirect_status,
            );
        }
        let csp_failure = {
            let state_ref = state.borrow();
            let Some(pending) = state_ref.pending_fetches.get(&completion.fetch_id) else {
                return;
            };
            worker_content_security_policy_violation_for_checked_url_with_redirect_status(
                &state_ref,
                &pending.document_url,
                &response_head.final_url,
                &pending.request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                if response_head.redirect_chain.is_empty() {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
                } else {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
                },
            )
        };
        if let Some(violation) = csp_failure {
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, state, &violation,
            );
            let message = worker_content_security_policy_error_message(&violation, "fetch");
            let Some(pending) = state
                .borrow_mut()
                .pending_fetches
                .remove(&completion.fetch_id)
            else {
                return;
            };
            pending.load.finish();
            let resolver = v8::Local::new(scope, &pending.resolver);
            record_worker_fetch_failure(&state.borrow(), &pending, message.clone());
            if let Some(message) = v8_string(scope, &message) {
                let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
            } else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
            }
            return;
        }
        let auth_required = {
            let mut state_ref = state.borrow_mut();
            let Some(pending) = state_ref.pending_fetches.get_mut(&completion.fetch_id) else {
                return;
            };
            pending.network_record.clone().and_then(|record| {
                if record.handle_auth_requests
                    && matches!(response_head.status, 401 | 407)
                    && let Some(challenge) =
                        extract_subresource_auth_challenge(&response_head.headers)
                {
                    let response_body = response.subresource_response_body();
                    pending.paused_response = Some(PausedWorkerSubresourceResponse {
                        head: response_head.clone(),
                        body: response_body.clone(),
                    });
                    Some(PendingSubresourceAuthInfo {
                        internal_id: record.internal_id,
                        url: record.url.clone(),
                        method: record.method.clone(),
                        request_headers: record.request_headers.clone(),
                        request_body: record.request_body.clone(),
                        resource_type: SubresourceResourceType::Fetch,
                        request_cookie_report: response_head.request_cookie_report.clone(),
                        network_request_headers: record.initial_network_request_headers.clone(),
                        challenge,
                        intercept_response: record.intercept_response,
                        response_final_url: response_head.final_url.clone(),
                        response_status: response_head.status,
                        response_headers: response_head.headers.clone(),
                        response_body,
                        response_from_cache: response_head.from_cache,
                    })
                } else {
                    None
                }
            })
        };
        if let Some(info) = auth_required {
            let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::AuthRequired(info),
            ));
            return;
        }
        let response_paused = {
            let mut state = state.borrow_mut();
            let Some(pending) = state.pending_fetches.get_mut(&completion.fetch_id) else {
                return;
            };
            match pending.network_record.as_ref() {
                Some(record) if record.intercept_response => {
                    let response_body = response.subresource_response_body();
                    let info = PendingSubresourceResponseInfo {
                        internal_id: record.internal_id,
                        url: record.url.clone(),
                        final_url: response_head.final_url.clone(),
                        method: record.method.clone(),
                        request_headers: record.request_headers.clone(),
                        request_body: record.request_body.clone(),
                        resource_type: SubresourceResourceType::Fetch,
                        request_cookie_report: response_head.request_cookie_report.clone(),
                        network_request_headers: record.initial_network_request_headers.clone(),
                        response_status: response_head.status,
                        response_headers: response_head.headers.clone(),
                        response_body: response_body.clone(),
                        from_cache: response_head.from_cache,
                    };
                    pending.paused_response = Some(PausedWorkerSubresourceResponse {
                        head: response_head,
                        body: response_body,
                    });
                    Some(info)
                }
                _ => None,
            }
        };
        if let Some(info) = response_paused {
            let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::ResponsePaused(info),
            ));
            return;
        }
    }
    let Some(pending) = state
        .borrow_mut()
        .pending_fetches
        .remove(&completion.fetch_id)
    else {
        return;
    };
    pending.load.finish();
    let resolver = v8::Local::new(scope, &pending.resolver);
    match completion.result {
        Ok(response) => {
            let response_head = response.head();
            let security_validation = match &response {
                WorkerFetchResponse::Materialized(response) => {
                    validate_fetch_response_security_policy_with_body_classified(
                        &pending.document_url,
                        &response_head.final_url,
                        &response_head.headers,
                        response.body_bytes(),
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )
                }
                WorkerFetchResponse::Streamed { body, .. } => body
                    .try_bytes()
                    .map_err(|error| {
                        FetchResponseSecurityViolation::Rejected(format!(
                            "fetch: failed to read response body: {error}"
                        ))
                    })
                    .and_then(|body_bytes| {
                        validate_fetch_response_security_policy_with_body_classified(
                            &pending.document_url,
                            &response_head.final_url,
                            &response_head.headers,
                            &body_bytes,
                            pending.request_mode,
                            pending.credentials_mode,
                            pending.policy_context,
                        )
                    }),
            };
            let opaque_response_blocked = match security_validation {
                Ok(()) => false,
                Err(FetchResponseSecurityViolation::OpaqueResponseBlocked(_)) => true,
                Err(violation) => {
                    let message = violation.into_message();
                    record_worker_fetch_failure(&state.borrow(), &pending, message.clone());
                    if let Some(message) = v8_string(scope, &message) {
                        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
                    } else {
                        let _ = resolver.reject(scope, v8::undefined(scope).into());
                    }
                    return;
                }
            };
            if opaque_response_blocked {
                record_worker_fetch_failure(
                    &state.borrow(),
                    &pending,
                    ABORTED_ERROR_TEXT.to_owned(),
                );
            }
            if !opaque_response_blocked && let Some(record) = pending.network_record.as_ref() {
                let response_body = response.subresource_response_body();
                record_worker_subresource_success_with_handle(
                    &state.borrow(),
                    pending.network_request_handle,
                    pending.document_url.clone(),
                    record.url.clone(),
                    record.method.clone(),
                    record.request_headers.clone(),
                    record.request_body.clone(),
                    SubresourceResourceType::Fetch,
                    record.initial_network_request_headers.clone(),
                    response_head.clone(),
                    response_body.clone(),
                );
                let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: record.internal_id,
                    },
                ));
            }
            let filtered_headers = filter_cors_exposed_response_headers(
                &pending.document_url,
                &response_head.final_url,
                &response_head.headers,
                pending.credentials_mode,
            );
            let response_obj = match response.into_fetch_parts() {
                WorkerFetchResponseParts::Materialized { mut head, body } => {
                    head.headers = filtered_headers;
                    let body = if opaque_response_blocked {
                        ResponseBody::materialized_bytes(Vec::new())
                    } else {
                        *body
                    };
                    build_fetch_response_object_from_body_source_for_request_mode(
                        scope,
                        &pending.document_url,
                        pending.request_mode,
                        head,
                        body,
                    )
                }
                WorkerFetchResponseParts::Subresource { mut head, body } => {
                    head.headers = filtered_headers;
                    let body = if opaque_response_blocked {
                        SubresourceResponseBody::from_bytes(Vec::new())
                    } else {
                        body
                    };
                    build_fetch_response_object_from_subresource_body_for_request_mode(
                        scope,
                        &pending.document_url,
                        pending.request_mode,
                        head,
                        body,
                    )
                }
            };
            let _ = resolver.resolve(scope, response_obj.into());
        }
        Err(message) => {
            record_worker_fetch_failure(&state.borrow(), &pending, message.clone());
            if let Some(message) = v8_string(scope, &message) {
                let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
            } else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
            }
        }
    }
}

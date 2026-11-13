//! Request-owned network and fetch-handler competition.

use super::super::events::{ServiceWorkerDirectFetchCompletion, ServiceWorkerDirectFetchResult};
use super::super::static_router::race::{
    RouterRaceCandidate, RouterRaceCompletionTarget, RouterRaceEntry, RouterRaceResponse,
    RouterRaceResponseHead,
};
use super::*;
use std::sync::Arc;

pub(super) struct RouterRaceLaunch {
    event_id: ServiceWorkerEventId,
    owner: ServiceWorkerRunOwner,
    delivery: ServiceWorkerFetchJob,
    response: Arc<RouterRaceResponse>,
    sender: tokio::sync::mpsc::UnboundedSender<RouterRaceCandidate>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<RouterRaceCandidate>,
}

impl ServiceWorkerRuntimeService {
    pub(super) fn prepare_router_race(
        &self,
        event_id: ServiceWorkerEventId,
        job: &mut ServiceWorkerFetchJob,
        request: super::super::ServiceWorkerFetchRequest,
    ) -> RouterRaceLaunch {
        let owner = job.owner().clone();
        let root_cancel = job.cancel_handle.clone();
        let delivery = job.fork_router_race_delivery();
        job.router_race_request_cancel_handle = Some(root_cancel.clone());
        job.cancel_handle = root_cancel.child_for_subrequest();
        let response = RouterRaceResponse::new(root_cancel.child_for_subrequest());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        job.direct_completion_tx = Some(ServiceWorkerDirectFetchCompletion::for_router_race(
            RouterRaceCompletionTarget {
                sender: sender.clone(),
                response: response.clone(),
                runner: job.resource_task_runner.clone(),
                cancel: root_cancel,
            },
            event_id,
        ));
        self.inner.router_races.lock().insert(
            event_id,
            RouterRaceEntry {
                owner: owner.clone(),
                response: response.clone(),
                request,
            },
        );
        RouterRaceLaunch {
            event_id,
            owner,
            delivery,
            response,
            sender,
            receiver,
        }
    }

    pub(super) fn cancel_discarded_router_stream(
        &self,
        event_id: ServiceWorkerEventId,
        owner: &ServiceWorkerRunOwner,
        body_source_id: crate::types::NetworkBodySourceId,
    ) {
        let host = {
            let state = self.inner.state.lock();
            state.versions.get(&owner.version_id()).and_then(|version| {
                if version.run_owner() != *owner {
                    return None;
                }
                match &version.running_state {
                    ServiceWorkerVersionRunningState::Running { host }
                        if host.run_owner() == *owner =>
                    {
                        Some(host.clone())
                    }
                    _ => None,
                }
            })
        };
        if let Some(host) = host {
            host.cancel_fetch_stream(event_id, body_source_id);
        }
    }
}

impl RouterRaceLaunch {
    pub(super) fn start(self, service: &ServiceWorkerRuntimeService) {
        let request = super::fetch_settlement::service_worker_network_fallback_request_for_job(
            &self.delivery,
        );
        let loader = self.delivery.request_client.clone();
        let preflight_headers = self.delivery.cors_preflight_request_headers.clone();
        let context = self.delivery.network_context.clone();
        let integrity = self.delivery.metadata.integrity.clone();
        let network_cancel = self.response.network_cancel_handle();
        let response = self.response.clone();
        let sender = self.sender.clone();
        let runner = self.delivery.resource_task_runner.clone();
        runner.spawn(async move {
            let mode = request.request_mode;
            let credentials = request.credentials_mode;
            let redirect = request.redirect_mode();
            let method = request.method.clone();
            let result = crate::network_host::fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                &loader, request, Some(network_cancel.clone()), preflight_headers,
            ).await;
            match result {
                Err(message) => response.finish(Err(message)),
                Ok(observed) => {
                    let (mut transfer, observation) = observed.into_parts();
                    let head = transfer.head();
                    if let Err(message) = crate::network_host::validate_fetch_response_headers(
                        &context.request_origin, &head, mode, credentials, context.policy_context,
                    ) {
                        response.finish(Err(message));
                        return;
                    }
                    let filter = crate::network_host::network_response_filter(&context.request_origin, &head, mode, redirect);
                    let network_is_eligible = (200..=299).contains(&head.status)
                        && !matches!(filter, Some(crate::types::AsyncSubresourceFetchResponseFilter::Opaque | crate::types::AsyncSubresourceFetchResponseFilter::OpaqueRedirect));
                    let response_head = RouterRaceResponseHead {
                        head,
                        response_filter: filter,
                        network_request_headers: observation.map(|value| value.into_headers()),
                    };
                    if !integrity.is_empty() {
                        // Inner Fetch applies SRI before processResponse. Its
                        // candidate must not beat a handler with unchecked bytes.
                        let mut body = Vec::new();
                        while let Some(chunk) = transfer.next_chunk().await {
                            body.extend_from_slice(&chunk);
                        }
                        if let Err(error) = transfer.finish().await {
                            response.finish(Err(format!("{error:#}")));
                            return;
                        }
                        let integrity_filter = response_head.response_filter.clone()
                            .unwrap_or(crate::types::AsyncSubresourceFetchResponseFilter::Basic);
                        if let Err(message) = crate::network_host::validate_fetch_response_integrity(
                            &integrity, &method, response_head.head.status, &integrity_filter, &body,
                        ) {
                            response.finish(Err(message));
                            return;
                        }
                        response.push_chunk(body.into());
                        response.publish_head(response_head);
                        if network_is_eligible {
                            let _ = sender.send(RouterRaceCandidate::Network);
                        }
                        response.finish(Ok(()));
                        return;
                    }
                    response.publish_head(response_head);
                    if network_is_eligible {
                        let _ = sender.send(RouterRaceCandidate::Network);
                    }
                    while let Some(chunk) = transfer.next_chunk().await {
                        response.push_chunk(chunk.into());
                    }
                    response.finish(transfer.finish().await.map_err(|error| format!("{error:#}")));
                }
            }
        });
        let runtime = service.clone();
        runner.spawn(async move {
            let Self { event_id, owner, delivery, response, sender, mut receiver } = self;
            drop(sender);
            let candidate = tokio::select! {
                biased;
                () = delivery.cancel_handle.cancelled() => {
                    runtime.close_router_race(event_id, &owner);
                    runtime.complete_fetch_with_failure(delivery, moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned());
                    return;
                }
                candidate = receiver.recv() => candidate,
            };
            match candidate {
                Some(candidate @ (RouterRaceCandidate::Network | RouterRaceCandidate::HandlerNetwork)) => {
                    let handler_remains = matches!(candidate, RouterRaceCandidate::Network);
                    delivery.cancel_handle.mark_service_worker_router_final_source(
                        if handler_remains { moli_fetch::ServiceWorkerRouterSource::Network }
                        else { moli_fetch::ServiceWorkerRouterSource::FetchEvent }
                    );
                    delivery.cancel_handle.adopt_response_progress(&response.network_cancel_handle());
                    // A network winner leaves handler execution and waitUntil
                    // alive. Discard only its eventual response body.
                    let cleanup = runtime.clone();
                    let cleanup_runner = delivery.resource_task_runner.clone();
                    if handler_remains { cleanup_runner.spawn(async move {
                        while let Some(candidate) = receiver.recv().await {
                            match candidate {
                                RouterRaceCandidate::Handler(result) => {
                                    if let ServiceWorkerDirectFetchResult::Stream(stream) = result {
                                        cleanup.cancel_discarded_router_stream(event_id, &owner, stream.body_source_id);
                                    }
                                    break;
                                }
                                RouterRaceCandidate::HandlerNetwork => break,
                                RouterRaceCandidate::Network => {}
                            }
                        }
                    }); }
                    runtime.deliver_router_race_network(delivery, response.network_reader()).await;
                }
                Some(RouterRaceCandidate::Handler(result)) => {
                    delivery.cancel_handle.mark_service_worker_router_final_source(moli_fetch::ServiceWorkerRouterSource::FetchEvent);
                    if matches!(result, ServiceWorkerDirectFetchResult::Response(_) | ServiceWorkerDirectFetchResult::Stream(_)) {
                        response.handler_won();
                    }
                    runtime.deliver_router_race_handler(delivery, event_id, owner, result).await;
                }
                None => runtime.complete_fetch_with_failure(delivery, "Static router response queue closed".to_owned()),
            }
        });
    }
}

impl ServiceWorkerRuntimeService {
    async fn deliver_router_race_handler(
        &self,
        mut job: ServiceWorkerFetchJob,
        event_id: ServiceWorkerEventId,
        owner: ServiceWorkerRunOwner,
        result: ServiceWorkerDirectFetchResult,
    ) {
        match result {
            ServiceWorkerDirectFetchResult::Failure(message) => {
                self.complete_fetch_with_failure(job, message)
            }
            ServiceWorkerDirectFetchResult::Response(response) => {
                let Some(head) = response.service_worker_response_head else {
                    self.complete_fetch_with_failure(
                        job,
                        "Missing raced handler response metadata".to_owned(),
                    );
                    return;
                };
                let body = response.response.body_bytes().to_vec();
                self.complete_fetch_with_service_worker_response(
                    job,
                    response_from_head(
                        *head,
                        response.service_worker_body_is_null.unwrap_or(false),
                        body,
                    ),
                );
            }
            ServiceWorkerDirectFetchResult::Stream(mut stream) => {
                let Some(head) = stream.service_worker_response_head.take() else {
                    self.complete_fetch_with_failure(
                        job,
                        "Missing raced handler stream metadata".to_owned(),
                    );
                    return;
                };
                if matches!(head.status, 301 | 302 | 303 | 307 | 308)
                    && job.request.redirect_mode() != moli_fetch::RequestRedirectMode::Manual
                {
                    self.cancel_discarded_router_stream(event_id, &owner, stream.body_source_id);
                    self.complete_fetch_with_service_worker_response(
                        job,
                        response_from_head(*head, false, Vec::new()),
                    );
                    return;
                }
                let streaming = match self.start_detached_service_worker_stream(
                    &mut job,
                    &head,
                    stream.body_source_id,
                ) {
                    Ok(value) => value,
                    Err(message) => {
                        self.cancel_discarded_router_stream(
                            event_id,
                            &owner,
                            stream.body_source_id,
                        );
                        self.complete_fetch_with_failure(job, message);
                        return;
                    }
                };
                let mut body = Vec::new();
                loop {
                    let event = tokio::select! {
                        biased;
                        () = job.cancel_handle.cancelled() => {
                            self.cancel_discarded_router_stream(event_id, &owner, stream.body_source_id);
                            self.complete_fetch_with_failure(job, moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned());
                            return;
                        }
                        event = stream.events.recv() => event,
                    };
                    match event {
                        Some(super::super::events::ServiceWorkerDirectFetchStreamEvent::Chunk(
                            bytes,
                        )) => {
                            if streaming {
                                forward_chunk(&job, bytes);
                            } else {
                                body.extend_from_slice(&bytes);
                            }
                        }
                        Some(
                            super::super::events::ServiceWorkerDirectFetchStreamEvent::Finished(
                                Ok(()),
                            ),
                        ) => {
                            self.complete_fetch_with_service_worker_response(
                                job,
                                response_from_head(*head, false, body),
                            );
                            return;
                        }
                        Some(
                            super::super::events::ServiceWorkerDirectFetchStreamEvent::Finished(
                                Err(message),
                            ),
                        ) => {
                            self.complete_fetch_with_failure(job, message);
                            return;
                        }
                        None => {
                            self.complete_fetch_with_failure(
                                job,
                                "Raced handler response body channel closed".to_owned(),
                            );
                            return;
                        }
                    }
                }
            }
            ServiceWorkerDirectFetchResult::Fallback
            | ServiceWorkerDirectFetchResult::NetworkFallback { .. } => {
                self.dispatch_fetch_fallback(job);
            }
            ServiceWorkerDirectFetchResult::FallbackStream(_) => {
                self.complete_fetch_with_failure(
                    job,
                    "A raced GET request cannot contain an upload stream".to_owned(),
                );
            }
        }
    }

    async fn deliver_router_race_network(
        &self,
        mut job: ServiceWorkerFetchJob,
        mut reader: super::super::static_router::race::RouterRaceResponseReader,
    ) {
        let head = match reader.head(&job.cancel_handle).await {
            Ok(head) => head,
            Err(message) => {
                self.complete_fetch_with_failure(job, message);
                return;
            }
        };
        let streaming = network_can_stream(&job, &head);
        if streaming {
            let id = crate::network_host::new_network_body_source_id();
            job.streaming_body_source_id = Some(id);
            if let Some(completion) = job.direct_completion_tx.take() {
                let (sender, events) = tokio::sync::mpsc::unbounded_channel();
                job.direct_streaming_tx = Some(sender);
                let _ = completion.send(ServiceWorkerDirectFetchResult::Stream(Box::new(
                    super::super::events::ServiceWorkerDirectFetchStream {
                        service_worker_response_head: None,
                        body_source_id: id,
                        head: head.head.clone(),
                        response_filter: head.response_filter.clone(),
                        events,
                    },
                )));
            } else {
                let _ = job.completion_tx.send_async_subresource_event(
                    crate::types::AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
                        crate::types::AsyncSubresourceStreamingStarted {
                            skip_fetch_security_validation: false,
                            response_filter: head.response_filter.clone(),
                            internal_id: job.internal_id,
                            request_url: job.request.url.clone(),
                            request_method: job.request.method.clone(),
                            request_headers: job.request.request_headers.clone(),
                            request_body: None,
                            body_source_id: id,
                            head: head.head.clone(),
                            network_request_headers: head.network_request_headers.clone(),
                        },
                    )),
                );
            }
        }
        let mut body = Vec::new();
        loop {
            match reader.next_chunk(&job.cancel_handle).await {
                Ok(Some(bytes)) => {
                    if streaming {
                        forward_chunk(&job, bytes.to_vec());
                    } else {
                        body.extend_from_slice(&bytes);
                    }
                }
                Err(message) => {
                    self.complete_fetch_with_failure(job, message);
                    return;
                }
                Ok(None) => break,
            }
        }
        if !streaming
            && let Err(message) =
                crate::network_host::validate_fetch_response_security_policy_with_body(
                    &job.network_context.request_origin,
                    &head.head,
                    &body,
                    job.request.request_mode,
                    job.request.credentials_mode,
                    job.network_context.policy_context,
                )
        {
            self.complete_fetch_with_failure(job, message);
            return;
        }
        if let Some(sender) = job.direct_streaming_tx.take() {
            let _ = sender
                .send(super::super::events::ServiceWorkerDirectFetchStreamEvent::Finished(Ok(())));
        } else if let Some(id) = job.streaming_body_source_id {
            let _ = job.completion_tx.send_async_subresource_event(
                crate::types::AsyncSubresourceFetchEvent::StreamingFinished(
                    crate::types::AsyncSubresourceStreamingFinished {
                        internal_id: job.internal_id,
                        body_source_id: id,
                        result: Ok(()),
                    },
                ),
            );
        } else {
            let response = crate::protocol_types::NavigationResponse::from_head_and_body(
                head.head.clone(),
                String::from_utf8_lossy(&body).into_owned(),
                body,
            );
            if let Some(completion) = job.direct_completion_tx.take() {
                let _ = completion.send(ServiceWorkerDirectFetchResult::Response(
                    super::super::events::ServiceWorkerDirectFetchResponse {
                        service_worker_response_head: None,
                        service_worker_body_is_null: None,
                        response_url: Some(response.final_url.clone()),
                        response: Box::new(response),
                        response_filter: head.response_filter.clone(),
                        from_network_fallback: true,
                    },
                ));
            } else {
                let _ = job.completion_tx.send_async_subresource(
                    crate::types::AsyncSubresourceFetchCompletion {
                        body_is_null: None,
                        internal_id: job.internal_id,
                        request_url: job.request.url.clone(),
                        request_method: job.request.method.clone(),
                        request_headers: job.request.request_headers.clone(),
                        request_body: None,
                        response_status_text: head.head.status_text.clone(),
                        skip_fetch_security_validation: false,
                        response_filter: head.response_filter.clone(),
                        network_error_text: None,
                        result: Ok(response).into(),
                    },
                );
            }
        }
    }
}

fn response_from_head(
    head: super::super::events::MaterializedServiceWorkerFetchResponseHead,
    body_is_null: bool,
    body: Vec<u8>,
) -> super::super::ServiceWorkerFetchResponse {
    super::super::ServiceWorkerFetchResponse {
        cors_exposed_header_names: head.cors_exposed_header_names,
        request_include_credentials: head.request_include_credentials,
        final_url: head.final_url,
        response_type: head.response_type,
        redirected: head.redirected,
        status: head.status,
        status_text: head.status_text,
        headers: head.headers,
        body_is_null,
        body,
    }
}

fn forward_chunk(job: &ServiceWorkerFetchJob, bytes: Vec<u8>) {
    if let Some(sender) = &job.direct_streaming_tx {
        let _ =
            sender.send(super::super::events::ServiceWorkerDirectFetchStreamEvent::Chunk(bytes));
    } else if let Some(body_source_id) = job.streaming_body_source_id {
        let _ = job.completion_tx.send_async_subresource_event(
            crate::types::AsyncSubresourceFetchEvent::StreamingChunk(
                crate::types::AsyncSubresourceStreamingChunk {
                    body_source_id,
                    bytes,
                },
            ),
        );
    }
}

fn network_can_stream(job: &ServiceWorkerFetchJob, head: &RouterRaceResponseHead) -> bool {
    job.direct_completion_tx
        .as_ref()
        .is_none_or(|completion| completion.stream_response)
        && job.metadata.integrity.is_empty()
        && !matches!(
            job.network_context.resource_type,
            crate::types::SubresourceResourceType::Audio
                | crate::types::SubresourceResourceType::Font
                | crate::types::SubresourceResourceType::Image
                | crate::types::SubresourceResourceType::Media
                | crate::types::SubresourceResourceType::TextTrack
                | crate::types::SubresourceResourceType::Video
        )
        && !matches!(
            head.response_filter,
            Some(
                crate::types::AsyncSubresourceFetchResponseFilter::Opaque
                    | crate::types::AsyncSubresourceFetchResponseFilter::OpaqueRedirect
            )
        )
}

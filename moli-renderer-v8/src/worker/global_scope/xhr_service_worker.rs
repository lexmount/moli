use super::*;
use crate::network_host::RedirectOwner;
use crate::service_worker_runtime::{
    ServiceWorkerClientId, ServiceWorkerDirectFetchCompletion, ServiceWorkerDirectFetchResult,
    ServiceWorkerDirectFetchStreamEvent, ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest,
    ServiceWorkerRequestDestination, ServiceWorkerRuntimeService,
    service_worker_fetch_request_metadata,
};
use crate::types::SubresourcePolicyContext;

pub(in crate::worker) struct WorkerXhrServiceWorkerController {
    runtime: ServiceWorkerRuntimeService,
    client_id: ServiceWorkerClientId,
    policy_context: SubresourcePolicyContext,
}

pub(super) fn take_controller(
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr_id: u32,
    url: &Url,
) -> Option<WorkerXhrServiceWorkerController> {
    let mut state = state.borrow_mut();
    let pending = state.pending_xhrs.get_mut(&xhr_id)?;
    if std::mem::replace(&mut pending.service_worker_checked, true) {
        return None;
    }
    let (runtime, client_id) = super::fetch::worker_service_worker_controller(&state, url)?;
    Some(WorkerXhrServiceWorkerController {
        runtime,
        client_id,
        policy_context: state.policy_context,
    })
}

pub(super) async fn load_response(
    load: &ResourceLoadLease,
    mut request: Request,
    cancel: FetchCancelHandle,
    mut preflight_headers: Vec<(String, String)>,
    document_url: &Url,
    timing: &WorkerResourceTiming,
    redirect_owner: &RedirectOwner,
    controller: Option<WorkerXhrServiceWorkerController>,
    xhr_id: u32,
) -> WorkerXhrCompletion {
    let failure = |message| WorkerXhrCompletion {
        xhr_id,
        network_request_headers: None,
        response_filter: None,
        skip_fetch_security_validation: false,
        result: Err(message),
    };
    if let Some(controller) = controller {
        let (reply, result) = tokio::sync::oneshot::channel();
        let redirect_request = request.clone();
        let dispatch = ServiceWorkerFetchDispatch {
            request_body_owner: None,
            redirect_check: Some(moli_fetch::RequestRedirectCheck::new(move |url| {
                redirect_request.check_redirect_target(url)
            })),
            // Direct consumers use their owning completion channel. Worker
            // XHR ids are local to the Worker, not browser resource ids.
            internal_id: 0,
            request: ServiceWorkerFetchRequest {
                body_stream: None,
                client_id: controller.client_id,
                resulting_client_id: None,
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.to_byte_strings(),
                body: request.body.clone(),
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: RequestMode::Cors,
                credentials_mode: request.credentials_mode,
                redirect_mode: request.redirect_mode(),
                priority: request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(&request),
            },
            cors_preflight_request_headers: preflight_headers.clone(),
            request_cookie_report: None,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: timing.origin().clone(),
                document_url: document_url.clone(),
                resource_type: SubresourceResourceType::Xhr,
                policy_context: controller.policy_context,
            },
            completion_tx:
                crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only(),
            request_client: load.request_client(),
            resource_task_runner: load.task_runner(),
            cancel_handle: cancel.clone(),
            direct_completion_tx: Some(
                ServiceWorkerDirectFetchCompletion::streaming_with_network_fallback(reply),
            ),
        };
        if controller.runtime.dispatch_controlled_fetch(dispatch) {
            let outcome = tokio::select! {
                biased;
                _ = cancel.cancelled() => return failure(ABORTED_ERROR_TEXT.to_owned()),
                result = result => result,
            };
            match outcome {
                Ok(ServiceWorkerDirectFetchResult::Fallback) => {}
                Ok(ServiceWorkerDirectFetchResult::NetworkFallback {
                    request: next,
                    preflight_headers: next_headers,
                }) => {
                    let observer = request.upload_observer().cloned();
                    request = *next;
                    if request.body.is_some()
                        && let Some(observer) = observer
                    {
                        request = request.with_upload_observer(observer);
                    }
                    preflight_headers = next_headers;
                    if let Err(message) = redirect_owner.admit(&cancel).await {
                        return failure(message);
                    }
                }
                Ok(ServiceWorkerDirectFetchResult::Stream(mut stream)) => {
                    timing.begin_synthetic_response();
                    timing.observe_head(&stream.head);
                    let mut body = SubresourceResponseBodyWriter::with_disk_pool(
                        load.request_client().disk_pool(),
                    );
                    let result = loop {
                        let event = tokio::select! {
                            biased;
                            _ = cancel.cancelled() => break Err(ABORTED_ERROR_TEXT.to_owned()),
                            event = stream.events.recv() => event,
                        };
                        match event {
                            Some(ServiceWorkerDirectFetchStreamEvent::Chunk(bytes)) => {
                                timing.observe_chunk(bytes.len());
                                body.append(&bytes);
                            }
                            Some(ServiceWorkerDirectFetchStreamEvent::Finished(result)) => {
                                break result;
                            }
                            None => {
                                break Err("service worker XHR response stream closed".to_owned());
                            }
                        }
                    };
                    let head = Box::new(stream.head);
                    let response_filter = stream.response_filter;
                    return WorkerXhrCompletion {
                        xhr_id,
                        network_request_headers: None,
                        response_filter,
                        skip_fetch_security_validation: true,
                        result: result.map(|()| WorkerXhrResponse::Streamed {
                            head,
                            body: body.finish(),
                        }),
                    };
                }
                Ok(ServiceWorkerDirectFetchResult::Response(response)) => {
                    let synthetic = !response.from_network_fallback;
                    if synthetic {
                        timing.begin_synthetic_response();
                    }
                    let response_filter = response.response_filter;
                    let response: Response = (*response.response).into();
                    timing.observe_head(&response.head());
                    timing.observe_chunk(response.body_bytes().len());
                    return WorkerXhrCompletion {
                        xhr_id,
                        network_request_headers: None,
                        response_filter,
                        skip_fetch_security_validation: synthetic,
                        result: Ok(WorkerXhrResponse::Materialized(Box::new(response))),
                    };
                }
                Ok(ServiceWorkerDirectFetchResult::Failure(message)) => return failure(message),
                Ok(ServiceWorkerDirectFetchResult::FallbackStream(_)) => {
                    return failure("XMLHttpRequest cannot upload an author stream".to_owned());
                }
                Err(_) => {
                    return failure("service worker XHR completion channel closed".to_owned());
                }
            }
        }
        let credentials_mode = controller
            .policy_context
            .network_credentials_mode(RequestMode::Cors, request.credentials_mode);
        request = request.with_credentials_mode(credentials_mode);
    }
    let (result, network_request_headers) = super::fetch::load_worker_xhr_response(
        load,
        request,
        cancel,
        preflight_headers,
        timing,
        Some(redirect_owner),
    )
    .await;
    WorkerXhrCompletion {
        xhr_id,
        network_request_headers,
        response_filter: None,
        skip_fetch_security_validation: false,
        result,
    }
}

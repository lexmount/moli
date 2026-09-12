use super::*;
use crate::runtime::{
    RendererWorkerFetchPause, RendererWorkerFetchStage, WorkerFetchDecision,
    WorkerFetchDecisionDispatch, WorkerFetchPhase, WorkerFetchTarget,
};

pub(in crate::worker) fn publish_worker_fetch_pause(
    state: &WorkerGlobalState,
    target: WorkerFetchTarget,
    handle: SubresourceNetworkRequestHandle,
    load: ResourceLoadLease,
    mut stage: RendererWorkerFetchStage,
) {
    if let RendererWorkerFetchStage::Request(info) = &mut stage {
        info.internal_id = handle.get();
    }
    debug_assert_eq!(stage.internal_id(), handle.get());
    let document_url = match target {
        WorkerFetchTarget::Fetch(id) => &state.pending_fetches[&id].document_url,
        WorkerFetchTarget::Xhr(id) => &state.pending_xhrs[&id].document_url,
        WorkerFetchTarget::CspReport(id) => &state.pending_csp_reports[&id].document_url,
    };
    let pause = RendererWorkerFetchPause::new(
        stage,
        document_url.clone(),
        target,
        handle,
        load,
        state.worker_wake_tx.clone(),
        state.global_kind.network().clone(),
    );
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::FetchInterception(pause));
}

pub(in crate::worker) fn decide_intercepted_worker_request(
    state: &Rc<RefCell<WorkerGlobalState>>,
    dispatch: WorkerFetchDecisionDispatch,
) {
    let WorkerFetchDecisionDispatch {
        target,
        handle,
        phase,
        decision,
        reply,
    } = dispatch;
    let result = apply_worker_fetch_decision(state, target, handle, phase, decision);
    if let Some(reply) = reply {
        let _ = reply.send(result);
    }
}

fn current_request(
    state: &WorkerGlobalState,
    target: WorkerFetchTarget,
    handle: SubresourceNetworkRequestHandle,
    phase: WorkerFetchPhase,
) -> Result<WorkerPendingFetchContinue, String> {
    let unavailable = || "Worker request is no longer paused".to_owned();
    let (id, url, method, headers, body, record) = match target {
        WorkerFetchTarget::Fetch(id) => {
            let pending = state.pending_fetches.get(&id).ok_or_else(unavailable)?;
            if pending.load.is_cancelled()
                || pending.network.handle() != handle
                || (phase != WorkerFetchPhase::Request && pending.paused_response.is_none())
            {
                return Err(unavailable());
            }
            (
                id,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                pending.network_record.as_ref(),
            )
        }
        WorkerFetchTarget::Xhr(id) => {
            let pending = state.pending_xhrs.get(&id).ok_or_else(unavailable)?;
            if pending.load.is_cancelled()
                || pending.network_request_handle != Some(handle)
                || (phase != WorkerFetchPhase::Request && pending.paused_response.is_none())
            {
                return Err(unavailable());
            }
            (
                id,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                pending.network_record.as_ref(),
            )
        }
        WorkerFetchTarget::CspReport(id) => {
            let pending = state.pending_csp_reports.get(&id).ok_or_else(unavailable)?;
            if pending.load.is_cancelled()
                || pending.handle != handle
                || phase != WorkerFetchPhase::Request
            {
                return Err(unavailable());
            }
            return Ok(WorkerPendingFetchContinue {
                fetch_id: id,
                internal_id: handle.get(),
                network_request_handle: Some(handle),
                url: pending.request.url.clone(),
                method: pending.request.method.clone(),
                headers: pending.request.request_headers.clone(),
                body: pending.request.body.clone(),
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            });
        }
    };
    Ok(WorkerPendingFetchContinue {
        fetch_id: id,
        internal_id: handle.get(),
        network_request_handle: Some(handle),
        url: record.map_or(url, |record| &record.url).clone(),
        method: record.map_or(method, |record| &record.method).clone(),
        headers: record
            .map_or(headers, |record| &record.request_headers)
            .clone(),
        body: record.map_or(body, |record| &record.request_body).clone(),
        intercept_response: record.is_some_and(|record| record.intercept_response),
        handle_auth_requests: record.is_some_and(|record| record.handle_auth_requests),
        auth: None,
    })
}

fn xhr_request(request: WorkerPendingFetchContinue) -> WorkerPendingXhrContinue {
    WorkerPendingXhrContinue {
        xhr_id: request.fetch_id,
        internal_id: request.internal_id,
        network_request_handle: request.network_request_handle,
        url: request.url,
        method: request.method,
        body: request.body,
        headers: request.headers,
        intercept_response: request.intercept_response,
        handle_auth_requests: request.handle_auth_requests,
        auth: request.auth,
    }
}

fn apply_worker_fetch_decision(
    state: &Rc<RefCell<WorkerGlobalState>>,
    target: WorkerFetchTarget,
    handle: SubresourceNetworkRequestHandle,
    phase: WorkerFetchPhase,
    mut decision: WorkerFetchDecision,
) -> Result<(), String> {
    let mut request = current_request(&state.borrow(), target, handle, phase)?;
    if matches!(decision, WorkerFetchDecision::Release) {
        decision = match phase {
            WorkerFetchPhase::Request => WorkerFetchDecision::ContinueRequest {
                url: None,
                method: None,
                body: None,
                headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            },
            WorkerFetchPhase::Auth => {
                cancel_worker_auth(state, target, false)?;
                return Ok(());
            }
            WorkerFetchPhase::Response => WorkerFetchDecision::ContinueResponse {
                response_code: None,
                response_headers: None,
            },
        };
    }
    match decision {
        WorkerFetchDecision::ContinueRequest {
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        } => {
            if let Some(url) = url {
                request.url = url;
            }
            if let Some(method) = method {
                request.method = method;
            }
            if let Some(body) = body {
                request.body = body.map(String::into_bytes);
            }
            if let Some(headers) = headers {
                request.headers = headers;
            }
            request.intercept_response = intercept_response;
            request.handle_auth_requests = handle_auth_requests;
            continue_request(state, target, request);
        }
        WorkerFetchDecision::ProvideAuth(auth) => {
            request.auth = Some(auth);
            continue_request(state, target, request);
        }
        WorkerFetchDecision::CancelAuth => {
            cancel_worker_auth(state, target, request.intercept_response)?;
        }
        WorkerFetchDecision::ContinueResponse {
            response_code,
            response_headers,
        } => match target {
            WorkerFetchTarget::Fetch(_) => continue_pending_worker_fetch_response(
                state,
                request,
                response_code,
                response_headers,
            ),
            WorkerFetchTarget::Xhr(_) => continue_pending_worker_xhr_response(
                state,
                xhr_request(request),
                response_code,
                response_headers,
            ),
            WorkerFetchTarget::CspReport(_) => {
                return Err("CSP report has no response pause".into());
            }
        },
        WorkerFetchDecision::Fail(error) => match (target, phase) {
            (WorkerFetchTarget::Fetch(_), WorkerFetchPhase::Request) => {
                fail_pending_worker_fetch(state, request, error);
            }
            (WorkerFetchTarget::Fetch(_), WorkerFetchPhase::Auth) => {
                fail_pending_worker_fetch_auth(state, request, error);
            }
            (WorkerFetchTarget::Fetch(_), WorkerFetchPhase::Response) => {
                fail_pending_worker_fetch_response(state, request, error);
            }
            (WorkerFetchTarget::Xhr(_), WorkerFetchPhase::Request) => {
                fail_pending_worker_xhr(state, xhr_request(request), error);
            }
            (WorkerFetchTarget::Xhr(_), WorkerFetchPhase::Auth) => {
                fail_pending_worker_xhr_auth(state, xhr_request(request), error);
            }
            (WorkerFetchTarget::Xhr(_), WorkerFetchPhase::Response) => {
                fail_pending_worker_xhr_response(state, xhr_request(request), error);
            }
            (WorkerFetchTarget::CspReport(_), _) => {
                fail_pending_worker_csp_report(state, request, error);
            }
        },
        WorkerFetchDecision::Fulfill {
            response_code,
            response_headers,
            response_body,
        } => match (target, phase) {
            (WorkerFetchTarget::Fetch(_), WorkerFetchPhase::Request) => {
                fulfill_pending_worker_fetch(
                    state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            (WorkerFetchTarget::Fetch(_), _) => {
                fulfill_pending_worker_fetch_response(
                    state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            (WorkerFetchTarget::Xhr(_), WorkerFetchPhase::Request) => {
                fulfill_pending_worker_xhr(
                    state,
                    xhr_request(request),
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            (WorkerFetchTarget::Xhr(_), _) => {
                fulfill_pending_worker_xhr_response(
                    state,
                    xhr_request(request),
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            (WorkerFetchTarget::CspReport(_), _) => {
                fulfill_pending_worker_csp_report(
                    state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
        },
        WorkerFetchDecision::Release => unreachable!("neutral decision normalized above"),
    }
    Ok(())
}

fn continue_request(
    state: &Rc<RefCell<WorkerGlobalState>>,
    target: WorkerFetchTarget,
    request: WorkerPendingFetchContinue,
) {
    match target {
        WorkerFetchTarget::Fetch(_) => continue_pending_worker_fetch(state, request),
        WorkerFetchTarget::Xhr(_) => continue_pending_worker_xhr(state, xhr_request(request)),
        WorkerFetchTarget::CspReport(_) => continue_pending_worker_csp_report(state, request),
    }
}

fn cancel_worker_auth(
    state: &Rc<RefCell<WorkerGlobalState>>,
    target: WorkerFetchTarget,
    intercept_response: bool,
) -> Result<(), String> {
    let mut state = state.borrow_mut();
    let unavailable = || "Worker authentication response is unavailable".to_owned();
    match target {
        WorkerFetchTarget::Fetch(fetch_id) => {
            let pending = state
                .pending_fetches
                .get_mut(&fetch_id)
                .ok_or_else(unavailable)?;
            let response = pending.paused_response.take().ok_or_else(unavailable)?;
            if let Some(record) = pending.network_record.as_mut() {
                record.handle_auth_requests = false;
                record.intercept_response = intercept_response;
            }
            let _ = state
                .fetch_completion_tx
                .send(WorkerFetchEvent::Completion(Box::new(
                    WorkerFetchCompletion {
                        fetch_id,
                        network_request_headers: None,
                        result: Ok(WorkerFetchResponse::Streamed {
                            head: Box::new(response.head),
                            body: response.body,
                        }),
                    },
                )));
        }
        WorkerFetchTarget::Xhr(xhr_id) => {
            let pending = state
                .pending_xhrs
                .get_mut(&xhr_id)
                .ok_or_else(unavailable)?;
            let response = pending.paused_response.take().ok_or_else(unavailable)?;
            if let Some(record) = pending.network_record.as_mut() {
                record.handle_auth_requests = false;
                record.intercept_response = intercept_response;
            }
            let _ = state.xhr_completion_tx.send(WorkerXhrCompletion {
                xhr_id,
                network_request_headers: None,
                result: Ok(WorkerXhrResponse::Streamed {
                    head: Box::new(response.head),
                    body: response.body,
                }),
            });
        }
        WorkerFetchTarget::CspReport(_) => {
            return Err("CSP report has no authentication pause".into());
        }
    }
    Ok(())
}

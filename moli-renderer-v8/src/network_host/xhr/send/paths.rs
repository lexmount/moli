use super::super::delivery::queue_xhr_response_delivery;
use super::super::*;
use super::request::PreparedXhrSendRequest;
use crate::network_host::controlled_resource::ControlledXhrContext;
use crate::service_worker_runtime::{ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination};
use moli_fetch::{FetchCancelHandle, RequestRedirectMode};

// Standalone ScriptVm's public pending-request API has no protocol journal.
// Keep that compatibility adapter separate from the native Page loading path.
pub(super) fn record_compatibility_intercepted_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut PageScriptHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) -> u64 {
    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        moli_fetch::RequestMode::Cors,
        prepared.credentials_mode,
    );
    let xhr = v8::Global::new(scope, xhr);
    host.record_pending_subresource_xhr(
        scope,
        prepared.execution_context,
        xhr,
        prepared.use_cors_preflight,
        prepared.initiator_url.clone(),
        prepared.credentials_mode,
        prepared.network_partition_key,
        prepared.policy_context,
        prepared.blob_url_entry,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id,
            document_url: prepared.document_url,
            url: prepared.resolved_url,
            websocket_socket_id: None,
            method: prepared.method,
            request_headers: prepared.request_headers,
            request_body: request_body_text(&prepared.send_body),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report,
        },
    )
}

pub(super) fn native_xhr_service_worker_context(
    host: &PageScriptHost,
    owner: crate::native_bridge::OwnerDispatchScope,
    document_url: &url::Url,
    request_url: &url::Url,
    policy_context: crate::types::SubresourcePolicyContext,
    csp: crate::network_host::WindowCspRequestContext,
) -> Option<crate::network_host::controlled_resource::XhrServiceWorkerContext> {
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    host.service_worker_controller_for_fetch(client_id, document_url, request_url)?;
    let runtime = host
        .browser_context_runtime()
        .service_worker_runtime_if_initialized()?;
    let document_url = document_url.clone();
    let request_url = request_url.clone();
    let completion = host.resource_completion_sender();
    Some(
        crate::network_host::controlled_resource::XhrServiceWorkerContext::new(
            runtime,
            client_id,
            policy_context,
            csp.policy.clone(),
            move |head, source| {
                csp.check_response(
                    &document_url,
                    &request_url,
                    head,
                    source,
                    completion.clone(),
                    "XMLHttpRequest",
                )
            },
        ),
    )
}

pub(super) fn dispatch_service_worker_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut PageScriptHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: &PreparedXhrSendRequest,
) -> Option<u64> {
    if !matches!(prepared.resolved_url.scheme(), "http" | "https") {
        return None;
    }
    let client_id = host.service_worker_client_id_for_subresource_owner(prepared.owner);
    host.service_worker_controller_for_fetch(
        client_id,
        &prepared.document_url,
        &prepared.resolved_url,
    )?;

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        moli_fetch::RequestMode::Cors,
        prepared.credentials_mode,
    );
    let cancel_handle = FetchCancelHandle::new();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: prepared.frame_id.clone(),
        request_origin: prepared.request_origin.clone(),
        document_url: prepared.document_url.clone(),
        resource_type: SubresourceResourceType::Xhr,
        policy_context: prepared.policy_context,
    };
    let request_body_text = request_body_text(&prepared.send_body);
    let execution_context = prepared.execution_context.duplicate(scope);
    let xhr = v8::Global::new(scope, xhr);
    let internal_id = host.record_async_subresource_xhr(
        scope,
        execution_context,
        xhr,
        prepared.use_cors_preflight,
        prepared.initiator_url.clone(),
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
        prepared.network_partition_key.clone(),
        prepared.policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id.clone(),
            document_url: prepared.document_url.clone(),
            url: prepared.resolved_url.clone(),
            websocket_socket_id: None,
            method: prepared.method.clone(),
            request_headers: prepared.request_headers.clone(),
            request_body: request_body_text.clone(),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report: request_cookie_report.clone(),
        },
        true,
    );
    let request = host.service_worker_fetch_request(
        client_id,
        prepared.resolved_url.clone(),
        prepared.method.clone(),
        prepared.request_headers.clone(),
        prepared.send_body.clone(),
        ServiceWorkerRequestDestination::Empty,
        moli_fetch::RequestMode::Cors,
        prepared.credentials_mode,
        RequestRedirectMode::Follow,
        None,
        crate::service_worker_runtime::service_worker_fetch_request_metadata(&prepared.request()),
    );
    let dispatch = ServiceWorkerFetchDispatch {
        request_body_owner: None,
        redirect_check: host.window_fetch_redirect_check(internal_id),
        internal_id,
        request,
        cors_preflight_request_headers: prepared.cors_preflight_request_headers.clone(),
        request_cookie_report,
        network_context,
        completion_tx: host.resource_completion_sender(),
        request_client: prepared.resource_loader.request_client().clone(),
        resource_task_runner: prepared.resource_loader.task_runner(),
        cancel_handle,
        direct_completion_tx: None,
    };
    if host.dispatch_service_worker_fetch(dispatch) {
        return Some(internal_id);
    }

    let _ =
        host.resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                body_is_null: None,
                internal_id,
                request_url: prepared.resolved_url.clone(),
                request_method: prepared.method.clone(),
                request_headers: prepared.request_headers.clone(),
                request_body: request_body_text,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: Err("service worker xhr dispatch failed".to_owned()).into(),
            });
    Some(internal_id)
}

pub(super) fn queue_local_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut PageScriptHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    response: Response,
) {
    record_xhr_response_success(host, &prepared, &response);
    queue_xhr_response_delivery(scope, xhr, response);
}

pub(super) fn record_xhr_response_success(
    host: &mut PageScriptHost,
    prepared: &PreparedXhrSendRequest,
    response: &Response,
) {
    host.record_subresource_network(
        SubresourceNetworkRecord::success_with_body(
            prepared.frame_id.clone(),
            prepared.document_url.clone(),
            prepared.resolved_url.clone(),
            prepared.method.clone(),
            prepared.request_headers.clone(),
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            response.request_cookie_report.clone(),
            response
                .redirect_chain
                .clone()
                .into_iter()
                .map(Into::into)
                .collect(),
            response.final_url.clone(),
            response.status,
            response.headers.clone(),
            crate::protocol_types::SubresourceResponseBody::from_fetch_response(response),
            response.cookie_set_reports.clone(),
        )
        .with_from_cache(response.from_cache)
        .with_negotiated_http_version(response.negotiated_http_version),
    );
}

pub(super) fn spawn_network_xhr_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut PageScriptHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    request_client: ResourceRequestClient,
    control: Option<ControlledXhrContext>,
) -> u64 {
    let mut request = prepared
        .request()
        .with_request_mode(moli_fetch::RequestMode::Cors)
        .with_subframe_context(prepared.frame_id.is_some());

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        moli_fetch::RequestMode::Cors,
        prepared.credentials_mode,
    );
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: prepared.frame_id.clone(),
        request_origin: prepared.request_origin.clone(),
        document_url: prepared.document_url.clone(),
        resource_type: SubresourceResourceType::Xhr,
        policy_context: prepared.policy_context,
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let xhr = v8::Global::new(scope, xhr);
    let internal_id = host.record_async_subresource_xhr(
        scope,
        prepared.execution_context,
        xhr,
        prepared.use_cors_preflight,
        prepared.initiator_url.clone(),
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
        prepared.network_partition_key.clone(),
        prepared.policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id.clone(),
            document_url: prepared.document_url.clone(),
            url: prepared.resolved_url.clone(),
            websocket_socket_id: None,
            method: prepared.method.clone(),
            request_headers: prepared.request_headers.clone(),
            request_body: request_body_text(&prepared.send_body),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report,
        },
        control.is_none(),
    );
    if let Some(check) = host.window_fetch_redirect_check(internal_id) {
        request = request.with_redirect_check(check);
    }
    if let Some(control) = control {
        let (load, info) = host
            .pending_subresource_native_inputs(internal_id)
            .expect("registered XHR must retain its resource authority");
        let completion_tx = host.resource_completion_sender();
        let runner = request_client.resource_task_runner();
        let csp = host
            .window_subresource_csp_context(internal_id)
            .expect("registered native XHR owns its captured Document policy");
        let service_worker = native_xhr_service_worker_context(
            host,
            prepared.owner,
            &prepared.document_url,
            &prepared.resolved_url,
            prepared.policy_context,
            csp,
        );
        let mut controlled = control
            .load(
                request,
                info,
                load,
                cancel_handle,
                prepared.cors_preflight_request_headers,
            )
            .with_service_worker(service_worker)
            .with_redirect_owner(host.window_redirect_owner(internal_id));

        host.append_live_turn_owner_action(controlled.prepare_initial_pause());
        runner.spawn(async move {
            use crate::network_host::controlled_resource::{
                ControlledResourceStream, ControlledXhrCompletion,
            };
            let completed = controlled
                .run_with_stream(Some(ControlledResourceStream::new(completion_tx.clone())))
                .await;
            let event = match completed {
                ControlledXhrCompletion::Buffered(completed) => {
                    crate::types::AsyncSubresourceFetchEvent::InterceptedXhrCompleted(completed)
                }
                ControlledXhrCompletion::Streamed {
                    internal_id,
                    body_source_id,
                    result,
                } => crate::types::AsyncSubresourceFetchEvent::StreamingFinished(
                    crate::types::AsyncSubresourceStreamingFinished {
                        internal_id,
                        body_source_id,
                        result,
                    },
                ),
            };
            let _ = completion_tx.send_async_subresource_event(event);
        });
        return internal_id;
    }
    spawn_async_subresource_fetch(
        request_client.resource_task_runner(),
        host.resource_completion_sender(),
        request_client,
        request,
        Some(cancel_handle),
        prepared.cors_preflight_request_headers,
        internal_id,
        network_context,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        host.window_redirect_owner(internal_id),
    );
    internal_id
}

pub(super) fn request_body_text(body: &Option<bytes::Bytes>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}

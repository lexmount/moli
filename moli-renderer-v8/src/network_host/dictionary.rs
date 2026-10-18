mod links;

pub(crate) use links::dictionary_links;

use super::*;
use crate::{
    native_bridge::WindowDocumentTaskTarget,
    service_worker_runtime::{
        ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, PendingDictionaryFetch,
        PendingSubresourceFetchInfo, SubresourceResourceType,
    },
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};
use url::Url;

/// Header hints have a Document load lease but no DOM element or author
/// callback. Retiring that exact Document cancels its outstanding downloads.
pub(crate) fn start_response_dictionary_loads(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    target: WindowDocumentTaskTarget,
    links: &[Url],
) {
    for url in links {
        if !host.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            break;
        }
        if let Err(error) = start_dictionary_fetch(scope, host, target, url.clone()) {
            tracing::debug!(%url, error, "compression dictionary hint was not loaded");
        }
    }
}

fn start_dictionary_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    target: WindowDocumentTaskTarget,
    url: Url,
) -> Result<(), String> {
    let owner = target.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_window_owner(target.owner())
        .ok_or("dictionary Document resource authority is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or("dictionary Document request environment is unavailable")?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    if !matches!(url.scheme(), "http" | "https")
        || !resource_loader
            .request_client()
            .optional_resource_fetch_enabled(SubresourceResourceType::Dictionary)
    {
        return Err("dictionary resource URL or loading policy is unsupported".into());
    }
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .ok_or("dictionary policy owner is unavailable")?;
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .ok_or("dictionary reporting owner is unavailable")?;
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    let (reports, enforced) = policy
        .check_url(
            &document_url,
            &url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let blocked = !enforced.is_empty();
    let host_ptr = host as *mut crate::native_bridge::JsContextHost;
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope, host_ptr, report_context.identity(), &violation,
        );
    }
    if blocked {
        return Err("dictionary request blocked by Content Security Policy".into());
    }
    let mut request = Request::get_with_url(url.clone())
        .with_initiator_url(&document_url)
        .with_request_origin(request_origin.clone())
        .with_resource_type(RequestResourceType::Dictionary)
        .with_browser_request_metadata(BrowserRequestMetadata::Dictionary)
        .with_page_network_policy()
        .with_request_mode(RequestMode::Cors)
        .with_credentials_mode(RequestCredentialsMode::Omit)
        .with_network_partition_key(network_partition_key.clone())
        .with_subframe_context(frame_id.is_some())
        .without_inferred_referrer();
    let cancel_handle = FetchCancelHandle::new();
    let dictionary = PendingDictionaryFetch {
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
    };
    let (internal_id, load) = host
        .record_async_dictionary_fetch(
            target,
            dictionary,
            cancel_handle.clone(),
            network_partition_key,
            policy_context,
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: frame_id.clone(),
                document_url: document_url.clone(),
                url: url.clone(),
                websocket_socket_id: None,
                method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Dictionary,
                request_cookie_report: None,
            },
        )
        .ok_or("dictionary Document was retired before request admission")?;
    let redirect_check = host.window_fetch_redirect_check(internal_id);
    if let Some(check) = redirect_check.clone() {
        request = request.with_redirect_check(check);
    }
    let loader = load.request_client();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id,
        request_origin,
        document_url: document_url.clone(),
        resource_type: SubresourceResourceType::Dictionary,
        policy_context,
    };
    if host
        .service_worker_controller_for_fetch(client_id, &document_url, &url)
        .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            redirect_check,
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                url.clone(),
                "GET".into(),
                Vec::new(),
                None,
                ServiceWorkerRequestDestination::Dictionary,
                RequestMode::Cors,
                RequestCredentialsMode::Omit,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context,
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: load.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url: url,
                    request_method: "GET".into(),
                    request_headers: Default::default(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker dictionary dispatch failed".to_owned()).into(),
                },
            );
        }
    } else if host.should_intercept_subresource(SubresourceResourceType::Dictionary) {
        host.pause_pending_subresource_fetch(internal_id);
    } else {
        spawn_async_subresource_fetch(
            load.task_runner(),
            host.resource_completion_sender(),
            loader,
            request,
            Some(cancel_handle),
            Vec::new(),
            internal_id,
            network_context,
            url,
            "GET".into(),
            Default::default(),
            None,
        );
    }
    Ok(())
}

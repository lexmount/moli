use super::*;
use crate::document_runtime::DocumentSubresourceCspKind;
use crate::native_bridge::{JsContextHost, OwnerDispatchScope, TextTrackLoadSequenceId};
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, PendingSubresourceFetchInfo,
    SubresourceRequestInitiatorType, SubresourceResourceType,
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TextTrackResourceFetchStart {
    Local(Result<String, String>),
    PolicySkipped,
    Pending,
}

pub(crate) fn start_text_track_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    track_handle: crate::document_runtime::DomHandle,
    sequence: TextTrackLoadSequenceId,
) -> Result<TextTrackResourceFetchStart, String> {
    let pending = host
        .pending_text_track_load_sequence(track_handle)
        .filter(|pending| pending.id() == sequence)
        .ok_or_else(|| "text-track request sequence is no longer pending".to_owned())?;
    if !host.pending_text_track_load_sequence_is_current(track_handle, sequence) {
        return Err("text-track request owner is stale".to_owned());
    }
    let target = pending.target();
    let owner = target.dispatch_scope();
    let document_owner = target
        .owner()
        .frame_document_owner()
        .ok_or_else(|| "text-track request does not target a frame Document".to_owned())?;
    let (frame_id, document_url) = match owner {
        OwnerDispatchScope::Top if host.main_document_task_owner_is_current(document_owner) => {
            (None, host.document_url().clone())
        }
        OwnerDispatchScope::Child(child_handle) => {
            let snapshot = host
                .frame_owner_current_child_snapshot(child_handle)
                .filter(|snapshot| {
                    snapshot.scheduler_lane_id == document_owner.scheduler_lane_id
                        && snapshot.local_window_id == document_owner.local_window_id
                        && snapshot.document_id == document_owner.document_id
                })
                .ok_or_else(|| "text-track child request scope is stale".to_owned())?;
            (Some(snapshot.frame_id.0), snapshot.document_url)
        }
        OwnerDispatchScope::Top => {
            return Err("text-track request has no current document owner".to_owned());
        }
    };
    let base_url = host.document_base_url_for_handle(pending.owner_document_handle());
    if pending.source().trim().is_empty() {
        return Ok(TextTrackResourceFetchStart::Local(Err(
            "text-track element has no source".to_owned(),
        )));
    }
    let request_url = url::Url::options()
        .base_url(Some(&base_url))
        .parse(pending.source())
        .map_err(|error| error.to_string())?;
    let media = host
        .dom_host()
        .node(pending.media_handle())
        .and_then(crate::dom::native::Node::as_element)
        .filter(|element| element.is_html_element("audio") || element.is_html_element("video"))
        .ok_or_else(|| "text-track media parent is unavailable".to_owned())?;
    let (request_mode, credentials_mode) = text_track_cross_origin_request_modes(media);
    if host
        .check_top_document_subresource_csp(scope, &request_url, DocumentSubresourceCspKind::Media)
        .blocks_request()
    {
        return Ok(TextTrackResourceFetchStart::Local(Err(
            "text-track request blocked by Content Security Policy".to_owned(),
        )));
    }

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        let result = text_track_response_result(response.status, response.body_text());
        host.record_get_subresource_network_result_with_initiator(
            frame_id,
            document_url,
            request_url,
            SubresourceResourceType::TextTrack,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(TextTrackResourceFetchStart::Local(result));
    }

    let Some(resource_loader) = host.document_resource_loader_for_window_owner(target.owner())
    else {
        return Ok(TextTrackResourceFetchStart::PolicySkipped);
    };
    let loader = resource_loader.request_client().clone();
    if !loader.optional_resource_fetch_enabled(SubresourceResourceType::TextTrack) {
        return Ok(TextTrackResourceFetchStart::PolicySkipped);
    }
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let request_cookie_report = observe_subresource_request_cookie_report(
        &loader,
        &document_url,
        &request_url,
        "GET",
        credentials_mode,
    );
    let request = Request::new("GET", request_url.as_str(), None, Vec::new())
        .map_err(|error| error.to_string())?
        .with_initiator_url(&document_url)
        .with_resource_type(RequestResourceType::TextTrack)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(BrowserRequestMetadata::TextTrack)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let Some(internal_id) = host.record_async_text_track_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        track_handle,
        sequence,
        owner,
        cancel_handle.clone(),
        credentials_mode,
        request_mode,
        network_partition_key,
        policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: frame_id.clone(),
            document_url: document_url.clone(),
            url: request_url.clone(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::TextTrack,
            request_cookie_report: request_cookie_report.clone(),
        },
    ) else {
        return Err("text-track sequence changed before request binding".to_owned());
    };

    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    if matches!(request_url.scheme(), "http" | "https")
        && host
            .service_worker_controller_for_fetch(client_id, &document_url, &request_url)
            .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                request_url.clone(),
                "GET".to_owned(),
                Vec::new(),
                None,
                ServiceWorkerRequestDestination::Track,
                request_mode,
                credentials_mode,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            request_body_text: None,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id,
                document_url,
                resource_type: SubresourceResourceType::TextTrack,
                policy_context,
            },
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: resource_loader.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url,
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker text-track fetch dispatch failed".to_owned()),
                },
            );
        }
        return Ok(TextTrackResourceFetchStart::Pending);
    }

    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id,
            document_url,
            resource_type: SubresourceResourceType::TextTrack,
            policy_context,
        },
        request_url,
        "GET".to_owned(),
        Vec::new(),
        None,
    );
    Ok(TextTrackResourceFetchStart::Pending)
}

pub(crate) fn text_track_response_result(status: u16, body: &str) -> Result<String, String> {
    if !(200..300).contains(&status) {
        return Err(format!("text-track fetch returned HTTP {status}"));
    }
    Ok(body.to_owned())
}

fn text_track_cross_origin_request_modes(
    media: &crate::dom::native::Element,
) -> (RequestMode, RequestCredentialsMode) {
    match media
        .attribute("crossorigin")
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => (RequestMode::SameOrigin, RequestCredentialsMode::SameOrigin),
        Some("use-credentials") => (RequestMode::Cors, RequestCredentialsMode::Include),
        Some(_) => (RequestMode::Cors, RequestCredentialsMode::SameOrigin),
    }
}

use super::*;
use crate::native_bridge::{
    JsContextHost, MediaLoadSequenceId, OwnerDispatchScope, PendingMediaLoadOwner,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaElementResourceFetchStart {
    Local { successful: bool },
    PolicySkipped,
    Pending,
}

pub(crate) fn start_media_element_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    media_handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    request_url: url::Url,
) -> Result<MediaElementResourceFetchStart, String> {
    let pending = host
        .pending_media_load_sequence(media_handle)
        .filter(|pending| pending.id() == sequence)
        .ok_or_else(|| "media lifecycle sequence is no longer pending".to_owned())?;
    if !host.pending_media_load_sequence_is_current(media_handle, sequence) {
        return Err("media lifecycle sequence owner is stale".to_owned());
    }
    let (owner, frame_id, document_url) = match pending.owner() {
        PendingMediaLoadOwner::Main { owner, .. }
            if host.main_document_task_owner_is_current(owner) =>
        {
            (OwnerDispatchScope::Top, None, host.document_url().clone())
        }
        PendingMediaLoadOwner::Child { child_handle, .. } => {
            let snapshot = host
                .frame_owner_current_child_snapshot(child_handle)
                .ok_or_else(|| "media child request scope is unavailable".to_owned())?;
            (
                OwnerDispatchScope::Child(child_handle),
                Some(snapshot.frame_id.0),
                snapshot.document_url,
            )
        }
        PendingMediaLoadOwner::Main { .. } | PendingMediaLoadOwner::LoadNeutral => {
            return Err("media lifecycle sequence has no current request owner".to_owned());
        }
    };
    let element = host
        .dom_host()
        .node(media_handle)
        .and_then(crate::dom::native::Node::as_element)
        .ok_or_else(|| "media element is unavailable".to_owned())?;
    let (resource_type, browser_metadata, destination) = if element.is_html_element("audio") {
        (
            SubresourceResourceType::Audio,
            BrowserRequestMetadata::Audio,
            ServiceWorkerRequestDestination::Audio,
        )
    } else if element.is_html_element("video") {
        (
            SubresourceResourceType::Video,
            BrowserRequestMetadata::Video,
            ServiceWorkerRequestDestination::Video,
        )
    } else {
        return Err("media lifecycle target is not an audio or video element".to_owned());
    };
    let (request_mode, credentials_mode) = media_cross_origin_request_modes(element);

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        let successful = media_response_status_is_successful(response.status);
        host.record_get_subresource_network_result_with_initiator(
            frame_id,
            document_url,
            request_url,
            resource_type,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(MediaElementResourceFetchStart::Local { successful });
    }

    let Some(resource_loader) = host.document_resource_loader_for_dispatch_scope(owner) else {
        return Ok(MediaElementResourceFetchStart::PolicySkipped);
    };
    let loader = resource_loader.request_client().clone();
    if !loader.optional_resource_fetch_enabled(resource_type) {
        return Ok(MediaElementResourceFetchStart::PolicySkipped);
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
        .with_resource_type(RequestResourceType::Media)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(browser_metadata)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let Some(internal_id) = host.record_async_media_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        media_handle,
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
            resource_type,
            request_cookie_report: request_cookie_report.clone(),
        },
    ) else {
        return Err("media lifecycle sequence changed before request binding".to_owned());
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
                destination,
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
                resource_type,
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
                    result: Err("service worker media fetch dispatch failed".to_owned()).into(),
                },
            );
        }
        return Ok(MediaElementResourceFetchStart::Pending);
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
            resource_type,
            policy_context,
        },
        request_url,
        "GET".to_owned(),
        Vec::new(),
        None,
    );
    Ok(MediaElementResourceFetchStart::Pending)
}

pub(crate) fn media_response_status_is_successful(status: u16) -> bool {
    (200..300).contains(&status)
}

fn media_cross_origin_request_modes(
    element: &crate::dom::native::Element,
) -> (RequestMode, RequestCredentialsMode) {
    match element
        .attribute("crossorigin")
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => (RequestMode::NoCors, RequestCredentialsMode::Include),
        Some("use-credentials") => (RequestMode::Cors, RequestCredentialsMode::Include),
        Some(_) => (RequestMode::Cors, RequestCredentialsMode::SameOrigin),
    }
}

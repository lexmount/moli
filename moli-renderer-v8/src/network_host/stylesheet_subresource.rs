use super::*;
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource,
        StylesheetLoadBlockingResourceKind,
    },
    frame_owner_model::StylesheetSubresourceLoadDelayBinding,
    native_bridge::{JsContextHost, OwnerDispatchScope},
    service_worker_runtime::{
        ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext,
        PendingSubresourceFetchInfo, SubresourceRequestInitiatorType, SubresourceResourceType,
    },
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

#[derive(Debug)]
pub(crate) enum StylesheetSubresourceFetchStart {
    Pending,
    /// The request reached a synchronous terminal and has no main-Document
    /// font state to commit.
    Settled,
    /// A main-Document font reached a synchronous terminal. Network failure
    /// is represented by `CompletedStylesheetWebFont` with no body bytes.
    WebFontSettled(CompletedStylesheetWebFont),
}

pub(crate) fn start_stylesheet_subresource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    binding: StylesheetSubresourceLoadDelayBinding,
    resource: StylesheetLoadBlockingResource,
    css_image: Option<crate::native_bridge::CssImageResourceRequestIdentity>,
) -> Result<StylesheetSubresourceFetchStart, String> {
    if !host.stylesheet_subresource_load_delay_is_current(binding) {
        return Err("stylesheet subresource document owner is stale".to_owned());
    }
    let (owner, frame_id, document_url) = match binding.child_handle() {
        None => (OwnerDispatchScope::Top, None, host.document_url().clone()),
        Some(child_handle) => {
            let task_owner = binding.owner();
            let snapshot = host
                .frame_owner_current_child_snapshot(child_handle)
                .filter(|snapshot| {
                    snapshot.scheduler_lane_id == task_owner.scheduler_lane_id
                        && snapshot.local_window_id == task_owner.local_window_id
                        && snapshot.document_id == task_owner.document_id
                })
                .ok_or_else(|| "stylesheet subresource child owner is stale".to_owned())?;
            (
                OwnerDispatchScope::Child(child_handle),
                Some(snapshot.frame_id.0),
                snapshot.document_url,
            )
        }
    };
    let resource_kind = resource.kind();
    if css_image.is_some() && resource_kind != StylesheetLoadBlockingResourceKind::Image {
        return Err("a stylesheet CSS image identity was bound to a non-image resource".to_owned());
    }
    let is_main_web_font = binding.child_handle().is_none()
        && resource_kind == StylesheetLoadBlockingResourceKind::Font;
    if is_main_web_font {
        let web_font = resource.web_font().ok_or_else(|| {
            "main Document stylesheet font is missing parsed @font-face metadata".to_owned()
        })?;
        if web_font.request_id().is_none() {
            return Err(
                "main Document stylesheet font is missing its admitted request identity".to_owned(),
            );
        }
    }
    let (request_url, web_font) = resource.into_parts();
    let (
        resource_type,
        request_resource_type,
        metadata,
        destination,
        request_mode,
        credentials_mode,
    ) = match resource_kind {
        StylesheetLoadBlockingResourceKind::Image => (
            SubresourceResourceType::Image,
            RequestResourceType::Image,
            BrowserRequestMetadata::Image,
            ServiceWorkerRequestDestination::Image,
            RequestMode::NoCors,
            RequestCredentialsMode::Include,
        ),
        StylesheetLoadBlockingResourceKind::Font => (
            SubresourceResourceType::Font,
            RequestResourceType::Font,
            BrowserRequestMetadata::Font,
            ServiceWorkerRequestDestination::Font,
            RequestMode::Cors,
            RequestCredentialsMode::SameOrigin,
        ),
    };

    if let Some(response) = local_url_response(&request_url) {
        let mut response: crate::protocol_types::NavigationResponse = response.into();
        let encoded = (resource_kind == StylesheetLoadBlockingResourceKind::Image).then(|| {
            let manager = host
                .document_resource_loader_for_owner(binding.owner())
                .map_or_else(
                    moli_parkable_image::ParkableImageManager::default,
                    |loader| {
                        let runner = loader.task_runner();
                        loader.request_client().parkable_image_manager(&runner)
                    },
                );
            manager.from_frozen_bytes(response.take_body_bytes())
        });
        if let Some(identity) = css_image.as_ref() {
            let descriptor = crate::network_host::image_response_descriptor_from_parkable(
                &response,
                encoded
                    .as_ref()
                    .expect("a local CSS image must have parkable bytes"),
            );
            let _ = host.complete_stylesheet_css_image_response(
                identity,
                descriptor,
                encoded
                    .as_ref()
                    .expect("a local CSS image must have parkable bytes")
                    .clone(),
            );
        }
        let terminal = if is_main_web_font {
            let font = web_font.expect("main web-font metadata was validated before consumption");
            StylesheetSubresourceFetchStart::WebFontSettled(
                if (200..=299).contains(&response.status) {
                    CompletedStylesheetWebFont::response(font, response.clone_body_bytes())
                } else {
                    CompletedStylesheetWebFont::failure(font)
                },
            )
        } else {
            StylesheetSubresourceFetchStart::Settled
        };
        let result = Ok(response);
        if let Some(encoded) = encoded {
            host.record_get_subresource_network_result_with_body_and_initiator(
                frame_id,
                document_url,
                request_url,
                resource_type,
                SubresourceRequestInitiatorType::Css,
                &result,
                crate::types::SubresourceResponseBody::from_parkable_image(encoded),
            );
        } else {
            host.record_get_subresource_network_result_with_initiator(
                frame_id,
                document_url,
                request_url,
                resource_type,
                SubresourceRequestInitiatorType::Css,
                &result,
            );
        }
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(terminal);
    }

    let Some(resource_loader) = host.document_resource_loader_for_owner(binding.owner()) else {
        if let Some(identity) = css_image.as_ref() {
            let _ = host.fail_stylesheet_css_image(identity);
        }
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(synchronous_failure_terminal(is_main_web_font, web_font));
    };
    let loader = resource_loader.request_client().clone();
    if !loader.optional_resource_fetch_enabled(resource_type) {
        if let Some(identity) = css_image.as_ref() {
            let _ = host.fail_stylesheet_css_image(identity);
        }
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(synchronous_failure_terminal(is_main_web_font, web_font));
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
        .with_resource_type(request_resource_type)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(metadata)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let Some(internal_id) = host.record_async_stylesheet_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        binding,
        owner,
        cancel_handle.clone(),
        credentials_mode,
        request_mode,
        network_partition_key,
        policy_context,
        web_font,
        css_image,
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
        return Err("stylesheet subresource owner changed before request binding".to_owned());
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
                    result: Err("service worker stylesheet subresource dispatch failed".to_owned())
                        .into(),
                },
            );
        }
        return Ok(StylesheetSubresourceFetchStart::Pending);
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
    Ok(StylesheetSubresourceFetchStart::Pending)
}

fn synchronous_failure_terminal(
    is_main_web_font: bool,
    web_font: Option<crate::css_resource_urls::StylesheetWebFont>,
) -> StylesheetSubresourceFetchStart {
    if is_main_web_font {
        StylesheetSubresourceFetchStart::WebFontSettled(CompletedStylesheetWebFont::failure(
            web_font.expect("main web-font metadata was validated before consumption"),
        ))
    } else {
        StylesheetSubresourceFetchStart::Settled
    }
}

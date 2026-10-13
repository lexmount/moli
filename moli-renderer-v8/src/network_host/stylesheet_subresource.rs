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
    /// The request reached a synchronous terminal and has no Document
    /// font state to commit.
    Settled,
    /// A Document font reached a synchronous terminal. Network failure
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
    let owner = binding
        .child_handle()
        .map_or(OwnerDispatchScope::Top, OwnerDispatchScope::Child);
    let resource_loader = host
        .document_resource_loader_for_owner(binding.owner())
        .ok_or_else(|| "stylesheet Document resource authority is unavailable".to_owned())?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or_else(|| "stylesheet Document request environment is unavailable".to_owned())?;
    let resource_kind = resource.kind();
    if css_image.is_some() && resource_kind != StylesheetLoadBlockingResourceKind::Image {
        return Err("a stylesheet CSS image identity was bound to a non-image resource".to_owned());
    }
    let is_web_font = resource_kind == StylesheetLoadBlockingResourceKind::Font;
    if is_web_font {
        let web_font = resource.web_font().ok_or_else(|| {
            "Document stylesheet font is missing parsed @font-face metadata".to_owned()
        })?;
        if web_font.request_id().is_none() {
            return Err(
                "Document stylesheet font is missing its admitted request identity".to_owned(),
            );
        }
    }
    let (request_url, web_font) = resource.into_parts();
    let resource_type = match resource_kind {
        StylesheetLoadBlockingResourceKind::Image => SubresourceResourceType::Image,
        StylesheetLoadBlockingResourceKind::Font => SubresourceResourceType::Font,
    };

    if let Some(response) = local_url_response(&request_url) {
        let crate::network::context::SubresourceRequestEnvironment {
            document_url,
            frame_id,
            ..
        } = environment;
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
        let terminal = if is_web_font {
            let font = web_font.expect("web-font metadata was validated before consumption");
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

    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(resource_type)
    {
        if let Some(identity) = css_image.as_ref() {
            let _ = host.fail_stylesheet_css_image(identity);
        }
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(synchronous_failure_terminal(is_web_font, web_font));
    }
    dispatch_stylesheet_resource_fetch(
        scope,
        host,
        resource_loader,
        environment,
        request_url,
        resource_kind,
        StylesheetRequestDestination::Live {
            binding,
            web_font,
            css_image,
        },
    )?;
    Ok(StylesheetSubresourceFetchStart::Pending)
}

fn synchronous_failure_terminal(
    is_web_font: bool,
    web_font: Option<crate::css_resource_urls::StylesheetWebFont>,
) -> StylesheetSubresourceFetchStart {
    if is_web_font {
        StylesheetSubresourceFetchStart::WebFontSettled(CompletedStylesheetWebFont::failure(
            web_font.expect("web-font metadata was validated before consumption"),
        ))
    } else {
        StylesheetSubresourceFetchStart::Settled
    }
}

enum StylesheetRequestDestination {
    Live {
        binding: StylesheetSubresourceLoadDelayBinding,
        web_font: Option<crate::css_resource_urls::StylesheetWebFont>,
        css_image: Option<crate::native_bridge::CssImageResourceRequestIdentity>,
    },
    Capture(crate::types::PendingCaptureResource),
}

/// Shared request construction and dispatch. Capture changes only the terminal
/// destination; credentials, partition, SW interception and transport stay here.
fn dispatch_stylesheet_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    resource_loader: crate::network::context::DocumentResourceLoader,
    environment: crate::network::context::SubresourceRequestEnvironment,
    request_url: url::Url,
    resource_kind: StylesheetLoadBlockingResourceKind,
    request_destination: StylesheetRequestDestination,
) -> Result<u64, String> {
    let owner = match &request_destination {
        StylesheetRequestDestination::Live { binding, .. } => binding
            .child_handle()
            .map_or(OwnerDispatchScope::Top, OwnerDispatchScope::Child),
        StylesheetRequestDestination::Capture(capture) => capture.target.dispatch_scope(),
    };
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    let loader = resource_loader.request_client().clone();
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
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let request_cookie_report = observe_subresource_request_cookie_report(
        &loader,
        &document_url,
        &request_origin,
        &request_url,
        "GET",
        credentials_mode,
    );
    let request = Request::new("GET", request_url.as_str(), None, Vec::new())
        .map_err(|error| error.to_string())?
        .with_initiator_url(&document_url)
        .with_request_origin(request_origin.clone())
        .with_resource_type(request_resource_type)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(metadata)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let context = v8::Global::new(scope, scope.get_current_context());
    let info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: frame_id.clone(),
        document_url: document_url.clone(),
        url: request_url.clone(),
        websocket_socket_id: None,
        method: "GET".to_owned(),
        request_headers: Default::default(),
        request_body: None,
        request_body_bytes: None,
        resource_type,
        request_cookie_report: request_cookie_report.clone(),
    };
    let internal_id = match request_destination {
        StylesheetRequestDestination::Live {
            binding,
            web_font,
            css_image,
        } => host.record_async_stylesheet_subresource_fetch(
            context,
            binding,
            owner,
            cancel_handle.clone(),
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            web_font,
            css_image,
            info,
        ),
        StylesheetRequestDestination::Capture(capture) => host.record_async_capture_resource_fetch(
            context,
            capture,
            cancel_handle.clone(),
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            info,
        ),
    }
    .ok_or_else(|| "stylesheet subresource owner changed before request binding".to_owned())?;

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
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id,
                request_origin: request_origin.clone(),
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
                    request_headers: Default::default(),
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
        return Ok(internal_id);
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
            request_origin: request_origin.clone(),
            document_url,
            resource_type,
            policy_context,
        },
        request_url,
        "GET".to_owned(),
        Default::default(),
        None,
    );
    Ok(internal_id)
}

pub(crate) fn start_capture_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    request_url: url::Url,
    capture: crate::types::PendingCaptureResource,
) -> Result<Option<u64>, String> {
    use crate::capture_resources::CaptureResourceKind;
    let target = capture.target;
    let owner = target.dispatch_scope();
    if !capture.endpoint.is_active()
        || host.current_window_document_task_target_for_dispatch_scope(owner) != Some(target)
    {
        return Err("capture resource Document owner is stale".to_owned());
    }
    let resource_loader = host
        .document_resource_loader_for_window_owner(target.owner())
        .ok_or_else(|| "capture Document resource authority is unavailable".to_owned())?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or_else(|| "capture Document request environment is unavailable".to_owned())?;
    let (resource_kind, resource_type, request_mode, credentials_mode) = match capture.kind {
        CaptureResourceKind::Image => (
            StylesheetLoadBlockingResourceKind::Image,
            SubresourceResourceType::Image,
            RequestMode::NoCors,
            RequestCredentialsMode::Include,
        ),
        CaptureResourceKind::Font => (
            StylesheetLoadBlockingResourceKind::Font,
            SubresourceResourceType::Font,
            RequestMode::Cors,
            RequestCredentialsMode::SameOrigin,
        ),
    };
    // As with live stylesheet resources, local responses precede optional
    // network-family admission. They still use an owned, exact-target terminal.
    if let Some(response) = local_url_response(&request_url) {
        let mut response: crate::protocol_types::NavigationResponse = response.into();
        let encoded = (capture.kind == CaptureResourceKind::Image).then(|| {
            let runner = resource_loader.task_runner();
            resource_loader
                .request_client()
                .parkable_image_manager(&runner)
                .from_frozen_bytes(response.take_body_bytes())
        });
        let context = v8::Global::new(scope, scope.get_current_context());
        let network_partition_key = active_subresource_network_partition_key(host, owner);
        let policy_context = effective_subresource_policy_context(scope, host, owner);
        let internal_id = host
            .record_async_capture_resource_fetch(
                context,
                capture,
                FetchCancelHandle::new(),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                PendingSubresourceFetchInfo {
                    internal_id: 0,
                    network_request_handle: None,
                    frame_id: environment.frame_id,
                    document_url: environment.document_url,
                    url: request_url.clone(),
                    websocket_socket_id: None,
                    method: "GET".to_owned(),
                    request_headers: Default::default(),
                    request_body: None,
                    request_body_bytes: None,
                    resource_type,
                    request_cookie_report: None,
                },
            )
            .ok_or_else(|| "capture resource owner changed before request binding".to_owned())?;
        let result = match encoded {
            Some(encoded) => crate::types::AsyncSubresourceFetchResult::Image { response, encoded },
            None => crate::types::AsyncSubresourceFetchResult::Response(response),
        };
        if host
            .resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result,
            })
            .is_err()
        {
            let _ = host.abort_subresource_fetch(internal_id);
            return Err("capture resource completion route is closed".to_owned());
        }
        return Ok(Some(internal_id));
    }
    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(resource_type)
    {
        return Ok(None);
    }
    dispatch_stylesheet_resource_fetch(
        scope,
        host,
        resource_loader,
        environment,
        request_url,
        resource_kind,
        StylesheetRequestDestination::Capture(capture),
    )
    .map(Some)
}

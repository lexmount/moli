use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetSubresourceRequest, StylesheetSubresourceTarget,
    },
    frame_owner_model::StylesheetSubresourceLoadDelayBinding,
    native_bridge::{JsContextHost, OwnerDispatchScope},
    types::{AsyncSubresourceFetchCompletion, SubresourceRequestInitiatorType},
};
use anyhow::{Context, Result, ensure};
use moli_fetch::FetchCancelHandle;

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
    request: StylesheetSubresourceRequest,
) -> Result<StylesheetSubresourceFetchStart> {
    ensure!(
        host.stylesheet_subresource_load_delay_is_current(binding),
        "stylesheet subresource document owner is stale"
    );
    let owner = binding
        .child_handle()
        .map_or(OwnerDispatchScope::Top, OwnerDispatchScope::Child);
    let resource_loader = host
        .document_resource_loader_for_owner(binding.owner())
        .context("stylesheet Document resource authority is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .context("stylesheet Document request environment is unavailable")?;
    let (request_url, target) = request.into_parts();
    let options = match &target {
        StylesheetSubresourceTarget::Image(_) => SubresourceFetchOptions::CSS_IMAGE,
        StylesheetSubresourceTarget::Font(_) => SubresourceFetchOptions::FONT,
    };
    let resource_type = options.resource_type;

    if let Some(response) = local_url_response(&request_url) {
        let mut response: crate::protocol_types::NavigationResponse = response.into();
        let terminal = match target {
            StylesheetSubresourceTarget::Image(identity) => {
                let runner = resource_loader.task_runner();
                let encoded = resource_loader
                    .request_client()
                    .parkable_image_manager(&runner)
                    .from_frozen_bytes(response.take_body_bytes());
                if let Some(identity) = identity {
                    let descriptor = crate::network_host::image_response_descriptor_from_parkable(
                        &response, &encoded,
                    );
                    let _ = host.complete_stylesheet_css_image_response(
                        &identity,
                        descriptor,
                        encoded.clone(),
                    );
                }
                host.record_get_subresource_network_result_with_body_and_initiator(
                    environment.frame_id,
                    environment.document_url,
                    request_url,
                    resource_type,
                    SubresourceRequestInitiatorType::Css,
                    &Ok(response),
                    crate::types::SubresourceResponseBody::from_parkable_image(encoded),
                );
                StylesheetSubresourceFetchStart::Settled
            }
            StylesheetSubresourceTarget::Font(font) => {
                let terminal = if (200..=299).contains(&response.status) {
                    CompletedStylesheetWebFont::response(font, response.clone_body_bytes())
                } else {
                    CompletedStylesheetWebFont::failure(font)
                };
                host.record_get_subresource_network_result_with_initiator(
                    environment.frame_id,
                    environment.document_url,
                    request_url,
                    resource_type,
                    SubresourceRequestInitiatorType::Css,
                    &Ok(response),
                );
                StylesheetSubresourceFetchStart::WebFontSettled(terminal)
            }
        };
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(terminal);
    }

    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(resource_type)
    {
        let terminal = synchronous_failure_terminal(host, target);
        host.settle_stylesheet_subresource_load_delay(binding);
        return Ok(terminal);
    }
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url,
        options,
    )?;
    let internal_id = host
        .record_async_stylesheet_subresource_fetch(
            v8::Global::new(scope, scope.get_current_context()),
            binding,
            owner,
            prepared.cancel_handle.clone(),
            options.credentials_mode,
            options.request_mode,
            prepared.network_partition_key.clone(),
            prepared.network_context.policy_context,
            target,
            prepared.pending_info.clone(),
        )
        .context("stylesheet subresource owner changed before request binding")?;

    prepared.dispatch(
        host,
        internal_id,
        "service worker stylesheet subresource dispatch failed",
    );
    Ok(StylesheetSubresourceFetchStart::Pending)
}

fn synchronous_failure_terminal(
    host: &mut JsContextHost,
    target: StylesheetSubresourceTarget,
) -> StylesheetSubresourceFetchStart {
    match target {
        StylesheetSubresourceTarget::Image(identity) => {
            if let Some(identity) = identity {
                let _ = host.fail_stylesheet_css_image(&identity);
            }
            StylesheetSubresourceFetchStart::Settled
        }
        StylesheetSubresourceTarget::Font(font) => StylesheetSubresourceFetchStart::WebFontSettled(
            CompletedStylesheetWebFont::failure(font),
        ),
    }
}

pub(crate) fn start_capture_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    request_url: url::Url,
    capture: crate::types::PendingCaptureResource,
) -> std::result::Result<Option<u64>, String> {
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
    let options = match capture.kind {
        CaptureResourceKind::Image => SubresourceFetchOptions::CSS_IMAGE,
        CaptureResourceKind::Font => SubresourceFetchOptions::FONT,
    };
    let resource_type = options.resource_type;
    let request_mode = options.request_mode;
    let credentials_mode = options.credentials_mode;
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
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url,
        options,
    )
    .map_err(|error| error.to_string())?;
    // Keep capture resources paused until the existing Fetch continuation
    // resumes them; preparation itself starts no network or Service Worker task.
    if host.should_intercept_subresource(resource_type) {
        return host
            .record_intercepted_capture_resource_fetch(
                v8::Global::new(scope, scope.get_current_context()),
                capture,
                credentials_mode,
                request_mode,
                prepared.network_partition_key,
                prepared.network_context.policy_context,
                prepared.pending_info,
            )
            .map(Some)
            .ok_or_else(|| "capture resource owner changed before request binding".to_owned());
    }
    let internal_id = host
        .record_async_capture_resource_fetch(
            v8::Global::new(scope, scope.get_current_context()),
            capture,
            prepared.cancel_handle.clone(),
            credentials_mode,
            request_mode,
            prepared.network_partition_key.clone(),
            prepared.network_context.policy_context,
            prepared.pending_info.clone(),
        )
        .ok_or_else(|| "capture resource owner changed before request binding".to_owned())?;
    prepared.dispatch(
        host,
        internal_id,
        "service worker capture subresource dispatch failed",
    );
    Ok(Some(internal_id))
}

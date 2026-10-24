use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetSubresourceRequest, StylesheetSubresourceTarget,
    },
    frame_owner_model::StylesheetSubresourceLoadDelayBinding,
    native_bridge::{JsContextHost, OwnerDispatchScope},
    types::SubresourceRequestInitiatorType,
};
use anyhow::{Context, Result, ensure};

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
                        &response,
                        &encoded,
                        crate::network_host::image_response_origin_clean(
                            &response,
                            &environment.request_origin,
                            options.request_mode,
                            options.credentials_mode,
                            None,
                        ),
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

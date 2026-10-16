use super::cross_origin_request_modes as media_cross_origin_request_modes;
use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::native_bridge::{MediaLoadSequenceId, PageScriptHost};
use crate::types::SubresourceRequestInitiatorType;
use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaElementResourceFetchStart {
    Local { successful: bool },
    PolicySkipped,
    Pending,
}

pub(crate) fn start_media_element_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut PageScriptHost,
    media_handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    request_url: url::Url,
) -> Result<MediaElementResourceFetchStart> {
    let pending = host
        .pending_media_load_sequence(media_handle)
        .filter(|pending| pending.id() == sequence)
        .context("media lifecycle sequence is no longer pending")?;
    if !host.pending_media_load_sequence_is_current(media_handle, sequence) {
        bail!("media lifecycle sequence owner is stale");
    }
    let target = pending.task_target();
    let owner = target.dispatch_scope();
    let execution_context = host
        .clone_window_execution_context_binding(scope, target.owner(), owner)
        .filter(|binding| binding.is_current(host))
        .context("media request relevant Window realm is stale")?;
    let Some(resource_loader) = host.document_resource_loader_for_dispatch_scope(owner) else {
        return Ok(MediaElementResourceFetchStart::PolicySkipped);
    };
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .context("Document request environment is unavailable")?;
    let element = host
        .dom_host()
        .node(media_handle)
        .and_then(crate::dom::native::Node::as_element)
        .context("media element is unavailable")?;
    let options = if element.is_html_element("audio") {
        SubresourceFetchOptions::AUDIO
    } else if element.is_html_element("video") {
        SubresourceFetchOptions::VIDEO
    } else {
        bail!("media lifecycle target is not an audio or video element");
    };
    let (request_mode, credentials_mode) = media_cross_origin_request_modes(element);

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        // A completed local response cannot provide media when it has no
        // bytes, even if the URL resolved successfully with a 2xx status.
        let mut successful = media_response_status_is_successful(response.status)
            && !response.body_bytes().is_empty();
        let mut decoding = false;
        if successful {
            let origin_clean = super::image_response_origin_clean(
                &response,
                &environment.request_origin,
                request_mode,
                credentials_mode,
                None,
            );
            match host.begin_media_resource(
                scope,
                media_handle,
                sequence,
                response.body_bytes(),
                origin_clean,
                None,
            ) {
                Ok(pending) => decoding = pending,
                Err(_) => successful = false,
            }
        }
        host.record_get_subresource_network_result_with_initiator(
            environment.frame_id,
            environment.document_url,
            request_url,
            options.resource_type,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(if decoding {
            MediaElementResourceFetchStart::Pending
        } else {
            MediaElementResourceFetchStart::Local { successful }
        });
    }

    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(options.resource_type)
    {
        return Ok(MediaElementResourceFetchStart::PolicySkipped);
    }
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url,
        options.with_request_modes(request_mode, credentials_mode),
    );
    let internal_id = host
        .record_media_subresource_fetch(
            execution_context,
            &resource_loader,
            media_handle,
            sequence,
            prepared.registration.clone(),
            &prepared.dispatch,
            credentials_mode,
            request_mode,
            prepared.network_partition_key.clone(),
            prepared.network_context.policy_context,
            prepared.pending_info.clone(),
        )
        .context("resource owner changed before request binding")?;
    prepared.dispatch(host, internal_id);
    Ok(MediaElementResourceFetchStart::Pending)
}

pub(crate) fn media_response_status_is_successful(status: u16) -> bool {
    (200..300).contains(&status)
}

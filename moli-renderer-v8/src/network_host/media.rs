use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::native_bridge::{
    JsContextHost, MediaLoadSequenceId, OwnerDispatchScope, PendingMediaLoadOwner,
};
use crate::types::SubresourceRequestInitiatorType;
use moli_fetch::{RequestCredentialsMode, RequestMode};

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
    let owner = match pending.owner() {
        PendingMediaLoadOwner::Main { owner, .. }
            if host.main_document_task_owner_is_current(owner) =>
        {
            OwnerDispatchScope::Top
        }
        PendingMediaLoadOwner::Child { child_handle, .. } => {
            OwnerDispatchScope::Child(child_handle)
        }
        _ => return Err("media request owner is stale".to_owned()),
    };
    let Some(resource_loader) = host.document_resource_loader_for_dispatch_scope(owner) else {
        return Ok(MediaElementResourceFetchStart::PolicySkipped);
    };
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or_else(|| "media Document request environment is unavailable".to_owned())?;
    let element = host
        .dom_host()
        .node(media_handle)
        .and_then(crate::dom::native::Node::as_element)
        .ok_or_else(|| "media element is unavailable".to_owned())?;
    let options = if element.is_html_element("audio") {
        SubresourceFetchOptions::AUDIO
    } else if element.is_html_element("video") {
        SubresourceFetchOptions::VIDEO
    } else {
        return Err("media lifecycle target is not an audio or video element".to_owned());
    };
    let (request_mode, credentials_mode) = media_cross_origin_request_modes(element);

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        // A completed local response cannot provide media when it has no
        // bytes, even if the URL resolved successfully with a 2xx status.
        let successful = media_response_status_is_successful(response.status)
            && !response.body_bytes().is_empty();
        host.record_get_subresource_network_result_with_initiator(
            environment.frame_id,
            environment.document_url,
            request_url,
            options.resource_type,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(MediaElementResourceFetchStart::Local { successful });
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
    )
    .map_err(|error| error.to_string())?;
    let Some(internal_id) = host.record_media_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        media_handle,
        sequence,
        owner,
        prepared.registration.clone(),
        &prepared.dispatch,
        credentials_mode,
        request_mode,
        prepared.network_partition_key.clone(),
        prepared.network_context.policy_context,
        prepared.pending_info.clone(),
    ) else {
        return Err("media lifecycle sequence changed before request binding".to_owned());
    };
    prepared.dispatch(host, internal_id);
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

use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::document_runtime::DocumentSubresourceCspKind;
use crate::native_bridge::{JsContextHost, PendingTextTrackLoadTerminal, TextTrackLoadSequenceId};
use crate::types::{SubresourceRequestInitiatorType, SubresourceResourceType};
use anyhow::{Context, Result};
use moli_fetch::{RequestCredentialsMode, RequestMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TextTrackResourceFetchStart {
    Local(PendingTextTrackLoadTerminal),
    PolicySkipped,
    Pending,
}

pub(crate) fn start_text_track_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    track_handle: crate::document_runtime::DomHandle,
    sequence: TextTrackLoadSequenceId,
) -> Result<TextTrackResourceFetchStart> {
    let pending = host
        .pending_text_track_load_sequence(track_handle)
        .filter(|pending| pending.id() == sequence)
        .context(ResourceFetchError::SequenceNotPending)?;
    if !host.pending_text_track_load_sequence_is_current(track_handle, sequence) {
        return Err(ResourceFetchError::StaleOwner.into());
    }
    let target = pending.target();
    let owner = target.dispatch_scope();
    let Some(resource_loader) = host.document_resource_loader_for_window_owner(target.owner())
    else {
        return Ok(TextTrackResourceFetchStart::PolicySkipped);
    };
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .context(ResourceFetchError::NoRequestEnvironment)?;
    if pending.source().trim().is_empty() {
        return Ok(TextTrackResourceFetchStart::Local(
            PendingTextTrackLoadTerminal::FetchFailed,
        ));
    }
    let request_url = url::Url::options()
        .base_url(Some(&environment.base_url))
        .parse(pending.source())?;
    let media = host
        .dom_host()
        .node(pending.media_handle())
        .and_then(crate::dom::native::Node::as_element)
        .filter(|element| element.is_html_element("audio") || element.is_html_element("video"))
        .context(ResourceFetchError::NoMediaElement)?;
    let (request_mode, credentials_mode) = text_track_cross_origin_request_modes(media);
    moli_fetch::FetchUrlList::new(&request_url, &[])
        .validate_request_mode(request_mode, &environment.request_origin)?;
    if host
        .check_top_document_subresource_csp(scope, &request_url, DocumentSubresourceCspKind::Media)
        .blocks_request()
    {
        return Ok(TextTrackResourceFetchStart::Local(
            PendingTextTrackLoadTerminal::FetchFailed,
        ));
    }

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        let terminal = text_track_response_terminal(response.status, response.body_text());
        host.record_get_subresource_network_result_with_initiator(
            environment.frame_id,
            environment.document_url,
            request_url,
            SubresourceResourceType::TextTrack,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(TextTrackResourceFetchStart::Local(terminal));
    }

    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(SubresourceResourceType::TextTrack)
    {
        return Ok(TextTrackResourceFetchStart::PolicySkipped);
    }
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url,
        SubresourceFetchOptions::TEXT_TRACK.with_request_modes(request_mode, credentials_mode),
    );
    let Some(internal_id) = host.record_text_track_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        track_handle,
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
        return Err(ResourceFetchError::OwnerChangedBeforeBinding.into());
    };
    prepared.dispatch(host, internal_id);
    Ok(TextTrackResourceFetchStart::Pending)
}

pub(crate) fn text_track_response_terminal(
    status: u16,
    body: &str,
) -> PendingTextTrackLoadTerminal {
    if (200..300).contains(&status) {
        PendingTextTrackLoadTerminal::Ready(body.to_owned())
    } else {
        PendingTextTrackLoadTerminal::FetchFailed
    }
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

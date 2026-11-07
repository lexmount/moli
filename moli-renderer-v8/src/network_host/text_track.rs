use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::document_runtime::DocumentSubresourceCspKind;
use crate::native_bridge::{JsContextHost, TextTrackLoadSequenceId};
use crate::types::{SubresourceRequestInitiatorType, SubresourceResourceType};
use moli_fetch::{RequestCredentialsMode, RequestMode};

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
    let Some(resource_loader) = host.document_resource_loader_for_window_owner(target.owner())
    else {
        return Ok(TextTrackResourceFetchStart::PolicySkipped);
    };
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or_else(|| "text-track Document request environment is unavailable".to_owned())?;
    if pending.source().trim().is_empty() {
        return Ok(TextTrackResourceFetchStart::Local(Err(
            "text-track element has no source".to_owned(),
        )));
    }
    let request_url = url::Url::options()
        .base_url(Some(&environment.base_url))
        .parse(pending.source())
        .map_err(|error| error.to_string())?;
    let media = host
        .dom_host()
        .node(pending.media_handle())
        .and_then(crate::dom::native::Node::as_element)
        .filter(|element| element.is_html_element("audio") || element.is_html_element("video"))
        .ok_or_else(|| "text-track media parent is unavailable".to_owned())?;
    let (request_mode, credentials_mode) = text_track_cross_origin_request_modes(media);
    moli_fetch::FetchUrlList::new(&request_url, &[])
        .validate_request_mode(request_mode, &environment.request_origin)?;
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
            environment.frame_id,
            environment.document_url,
            request_url,
            SubresourceResourceType::TextTrack,
            SubresourceRequestInitiatorType::Other,
            &Ok(response),
        );
        return Ok(TextTrackResourceFetchStart::Local(result));
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
    )
    .map_err(|error| error.to_string())?;
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
        return Err("text-track sequence changed before request binding".to_owned());
    };
    prepared.dispatch(host, internal_id);
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

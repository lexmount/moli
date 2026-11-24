use super::cross_origin_request_modes as media_cross_origin_request_modes;
use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::native_bridge::{
    MediaLoadSequenceId, OwnerDispatchScope, PageScriptHost, PendingMediaLoadOwner,
};
use crate::types::SubresourceRequestInitiatorType;
use anyhow::{Context, Result, bail, ensure};

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
        .context("resource load sequence is no longer pending")?;
    ensure!(
        host.pending_media_load_sequence_is_current(media_handle, sequence),
        "resource request owner is stale"
    );
    let owner = match pending.owner() {
        PendingMediaLoadOwner::Main { owner, .. }
            if host.main_document_task_owner_is_current(owner) =>
        {
            OwnerDispatchScope::Top
        }
        PendingMediaLoadOwner::Child { child_handle, .. } => {
            OwnerDispatchScope::Child(child_handle)
        }
        _ => bail!("resource request owner is stale"),
    };
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

    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    if host.try_attach_media_source(scope, media_handle, sequence, &request_url)? {
        return Ok(MediaElementResourceFetchStart::Pending);
    }

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        // A completed local response cannot provide media when it has no
        // bytes, even if the URL resolved successfully with a 2xx status.
        let successful = media_response_status_is_successful(response.status)
            && !response.body_bytes().is_empty();
        #[allow(unused_mut)]
        let mut result = Ok(response);
        host.record_get_subresource_network_result_with_initiator(
            environment.frame_id,
            environment.document_url,
            request_url,
            options.resource_type,
            SubresourceRequestInitiatorType::Other,
            &result,
        );
        #[cfg(all(
            feature = "media-ext",
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        ))]
        if successful
            && host.manages_video(media_handle)
            && let Ok(response) = &mut result
        {
            if is_mp4_video_response(response) {
                ensure!(
                    host.start_buffered_video(
                        scope,
                        media_handle,
                        sequence,
                        None,
                        response.body_bytes().len(),
                        || bytes::Bytes::from(response.take_body_bytes()),
                    ),
                    "video resource owner changed before decoding"
                );
                return Ok(MediaElementResourceFetchStart::Pending);
            }
            host.reset_video_playback(scope, media_handle);
        }
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
    );
    let internal_id = host
        .record_media_subresource_fetch(
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
        )
        .context("resource owner changed before request binding")?;
    prepared.dispatch(host, internal_id);
    Ok(MediaElementResourceFetchStart::Pending)
}

pub(crate) fn media_response_status_is_successful(status: u16) -> bool {
    (200..300).contains(&status)
}

#[cfg(all(
    feature = "media-ext",
    any(target_os = "linux", target_os = "macos"),
    target_pointer_width = "64"
))]
pub(crate) fn is_mp4_video_response(response: &crate::protocol_types::NavigationResponse) -> bool {
    matches!(response.body_bytes().get(4..8), Some(b"ftyp" | b"moov"))
        || moli_web_mime::response_header_value(&response.headers, "content-type").is_some_and(
            |mime| {
                let essence = mime.split(';').next().unwrap_or_default().trim();
                essence.eq_ignore_ascii_case("video/mp4")
                    || essence.eq_ignore_ascii_case("application/mp4")
            },
        )
}

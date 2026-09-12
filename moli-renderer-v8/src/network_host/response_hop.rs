//! Response-header rules shared by manual Fetch and intercepted XHR.
//! Transport pauses, body consumption and owner delivery remain with callers.

use moli_fetch::{
    NetworkObservationJournal, NetworkRequestExtraInfo, NetworkResponseExtraInfo, RedirectSource,
    Request, RequestRedirectMode, ResponseHead,
};

use super::next_redirect_url;
use moli_fetch::RequestMode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubresourceResponseTransition {
    FinalResponse,
    ManualResponse { response_url: url::Url },
    FollowedRedirect,
}

/// Keep the wire response separate from headers changed by interception or
/// cache validation. Synthetic responses and local cache hits have no exchange.
pub(super) fn network_response_extra_info(
    head: &ResponseHead,
    observations: &NetworkObservationJournal,
) -> Option<NetworkResponseExtraInfo> {
    let request = observations.final_request_observation()?;
    let response = observations.final_response_observation()?;
    Some(NetworkResponseExtraInfo {
        request_extra_info: NetworkRequestExtraInfo {
            headers: request.headers().to_vec(),
            cookie_report: request.cookie_report().cloned().unwrap_or_default(),
        },
        status: response.status(),
        headers: response.headers().to_vec(),
        cookie_set_reports: head.cookie_set_reports.clone(),
    })
}

pub(super) fn advance_subresource_response(
    request: &mut Request,
    head: ResponseHead,
    network_response_extra_info: Option<NetworkResponseExtraInfo>,
    source: RedirectSource,
    web_security_disabled: bool,
) -> Result<SubresourceResponseTransition, String> {
    // Redirect request records describe the next hop's cookie decision,
    // just as they do when the transport follows redirects internally.
    if source == moli_fetch::RedirectSource::Network
        && let Some(previous) = request.redirect_chain_mut().last_mut()
        && previous.source == moli_fetch::RedirectSource::Network
    {
        previous.request_cookie_report = head.request_cookie_report.clone();
        previous.request_extra_info = network_response_extra_info
            .as_ref()
            .map(|extra| extra.request_extra_info.clone());
    }
    // The single-hop transport refines request snapshots, including cache
    // hits and internal redirects. Keep them when carrying the chain on.
    if source == moli_fetch::RedirectSource::Network {
        request.observe_redirect_requests(&head.redirect_chain);
    }
    if source == moli_fetch::RedirectSource::Network && !web_security_disabled {
        request.validate_cors_response_for_url(&head.final_url, &head.headers)?;
    }
    advance_validated_subresource_response(
        request,
        head,
        network_response_extra_info,
        source,
        web_security_disabled,
    )
}

pub(super) fn advance_validated_subresource_response(
    request: &mut Request,
    head: ResponseHead,
    network_response_extra_info: Option<NetworkResponseExtraInfo>,
    source: RedirectSource,
    web_security_disabled: bool,
) -> Result<SubresourceResponseTransition, String> {
    if !is_redirect_status(head.status) {
        return Ok(SubresourceResponseTransition::FinalResponse);
    }
    // Fetch selects manual/error handling from the status before parsing
    // Location. A missing or invalid Location must not change those modes.
    match request.redirect_mode() {
        RequestRedirectMode::Error => return Err(redirect_mode_error_message(&head.final_url)),
        RequestRedirectMode::Manual => {
            return Ok(SubresourceResponseTransition::ManualResponse {
                response_url: head.final_url,
            });
        }
        RequestRedirectMode::Follow => {}
    }
    let Some(next_url) = next_redirect_url(
        &head.final_url,
        head.status,
        &head.headers,
        request.redirect_count(),
    )?
    else {
        return Ok(SubresourceResponseTransition::FinalResponse);
    };
    if !matches!(next_url.scheme(), "http" | "https") {
        return Err(format!("CORS redirect requires an HTTP(S) URL: {next_url}"));
    }
    if request.request_mode == RequestMode::Cors
        && (!next_url.username().is_empty()
            || next_url
                .password()
                .is_some_and(|password| !password.is_empty()))
        && request.request_origin().is_some_and(|origin| {
            request.has_cross_origin_url(&head.final_url) || !origin.same_origin_url(&next_url)
        })
    {
        return Err("CORS redirect URL must not include credentials".to_owned());
    }
    if !moli_url::same_origin(&head.final_url, &next_url) {
        request
            .request_headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        if request
            .auth()
            .is_some_and(|auth| auth.target == moli_fetch::RequestAuthTarget::Server)
        {
            request.set_auth(None);
        }
    }

    let mut redirect = head.into_redirect(next_url, source, network_response_extra_info.is_some());
    redirect.response_extra_info = network_response_extra_info;
    request
        .follow_redirect_with_web_security(redirect, web_security_disabled)
        .map_err(|error| error.to_string())?;

    Ok(SubresourceResponseTransition::FollowedRedirect)
}

pub(super) fn redirect_mode_error_message(url: &url::Url) -> String {
    format!("redirect mode error blocked redirect from {url}")
}

pub(super) fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

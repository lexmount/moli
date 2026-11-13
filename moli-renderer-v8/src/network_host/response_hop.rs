//! Response-header rules shared by manual Fetch and intercepted XHR.
//! Transport pauses, body consumption and owner delivery remain with callers.

use moli_fetch::{RedirectSource, Request, RequestRedirectMode, ResponseHead};

use super::next_redirect_url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SubresourceResponseTransition {
    FinalResponse,
    ManualResponse { response_url: url::Url },
    FollowedRedirect,
}

pub(super) fn advance_subresource_response(
    request: &mut Request,
    head: ResponseHead,
    network_extra_info_available: bool,
) -> Result<SubresourceResponseTransition, String> {
    // Preserve transport-refined snapshots, including cached/internal hops.
    request.observe_redirect_requests(&head.redirect_chain);
    // Fetch gives redirect:error precedence over CORS validation of a redirect.
    if request.redirect_mode() == RequestRedirectMode::Error {
        if next_redirect_url(&head.final_url, head.status, &head.headers, 0)?.is_some() {
            return Err(redirect_mode_error_message(&head.final_url));
        }
        request.validate_cors_response_for_url(&head.final_url, &head.headers)?;
        return Ok(SubresourceResponseTransition::FinalResponse);
    }

    request.validate_cors_response_for_url(&head.final_url, &head.headers)?;
    let Some(next_url) = next_redirect_url(
        &head.final_url,
        head.status,
        &head.headers,
        request.redirect_count(),
    )?
    else {
        return Ok(SubresourceResponseTransition::FinalResponse);
    };
    match request.redirect_mode() {
        RequestRedirectMode::Manual => {
            return Ok(SubresourceResponseTransition::ManualResponse {
                response_url: head.final_url,
            });
        }
        RequestRedirectMode::Follow => {}
        RequestRedirectMode::Error => unreachable!("error-mode responses returned before CORS"),
    }

    request
        .follow_redirect(head.into_redirect(
            next_url,
            RedirectSource::Network,
            network_extra_info_available,
        ))
        .map_err(|error| error.to_string())?;
    Ok(SubresourceResponseTransition::FollowedRedirect)
}

pub(super) fn redirect_mode_error_message(url: &url::Url) -> String {
    format!("redirect mode error blocked redirect from {url}")
}

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
    web_security_disabled: bool,
) -> Result<SubresourceResponseTransition, String> {
    // Preserve transport-refined snapshots, including cached/internal hops.
    request.observe_redirect_requests(&head.redirect_chain);
    if !web_security_disabled {
        request.validate_cors_response_for_url(&head.final_url, &head.headers)?;
    }
    advance_validated_subresource_response(
        request,
        head,
        network_extra_info_available,
        web_security_disabled,
    )
}

pub(super) fn advance_validated_subresource_response(
    request: &mut Request,
    head: ResponseHead,
    network_extra_info_available: bool,
    web_security_disabled: bool,
) -> Result<SubresourceResponseTransition, String> {
    // Error mode rejects redirect statuses without consulting Location.
    if request.redirect_mode() == RequestRedirectMode::Error {
        if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
            return Err(redirect_mode_error_message(&head.final_url));
        }
        return Ok(SubresourceResponseTransition::FinalResponse);
    }

    // Manual responses are filtered without following Location. Parsing it or
    // applying the redirect limit would reject an otherwise valid response.
    if request.redirect_mode() == RequestRedirectMode::Manual {
        return Ok(if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
            SubresourceResponseTransition::ManualResponse {
                response_url: head.final_url,
            }
        } else {
            SubresourceResponseTransition::FinalResponse
        });
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
    request
        .follow_redirect_with_web_security(
            head.into_redirect(
                next_url,
                RedirectSource::Network,
                network_extra_info_available,
            ),
            web_security_disabled,
        )
        .map_err(|error| error.to_string())?;
    Ok(SubresourceResponseTransition::FollowedRedirect)
}

pub(super) fn redirect_mode_error_message(url: &url::Url) -> String {
    format!("redirect mode error blocked redirect from {url}")
}

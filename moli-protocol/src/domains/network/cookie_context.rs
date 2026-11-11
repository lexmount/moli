use moli_cookie_jar::{
    NetworkCookieRequestContext, NetworkSiteContextMetadata, NetworkSiteContextTrackMetadata,
    redirect_types_for_request, site_context_downgrade_type,
};
use url::Url;

fn cookie_request_context_from_initiator(
    base: NetworkCookieRequestContext,
    request_url: &Url,
    previous_request_url: Option<&Url>,
    initiator_url: Option<&Url>,
) -> NetworkCookieRequestContext {
    let Some(initiator_url) = initiator_url else {
        return base;
    };

    let current = base.clone().with_initiator_url(request_url, initiator_url);
    let (context_downgrade_type, schemeful_context_downgrade_type) = previous_request_url
        .map(|previous_request_url| {
            let previous = base
                .clone()
                .with_initiator_url(previous_request_url, initiator_url);
            (
                site_context_downgrade_type(
                    previous.site_context().context,
                    current.site_context().context,
                ),
                site_context_downgrade_type(
                    previous.site_context().schemeful_context,
                    current.site_context().schemeful_context,
                ),
            )
        })
        .unwrap_or((None, None));
    let redirect_types = previous_request_url
        .map(|previous_request_url| {
            redirect_types_for_request(
                previous_request_url,
                request_url,
                current.initiator_url.as_ref(),
                &current.browser_context,
            )
        })
        .unwrap_or_else(NetworkSiteContextMetadata::none);

    current.with_site_context_metadata(NetworkSiteContextMetadata::new(
        NetworkSiteContextTrackMetadata::new(
            context_downgrade_type.is_some(),
            context_downgrade_type,
        )
        .with_redirect_type(redirect_types.context.redirect_type),
        NetworkSiteContextTrackMetadata::new(
            schemeful_context_downgrade_type.is_some(),
            schemeful_context_downgrade_type,
        )
        .with_redirect_type(redirect_types.schemeful_context.redirect_type),
    ))
}

pub(crate) fn navigation_cookie_request_context(
    request_url: &Url,
    method: &str,
    previous_request_url: Option<&Url>,
    initiator_url: Option<&Url>,
) -> NetworkCookieRequestContext {
    cookie_request_context_from_initiator(
        NetworkCookieRequestContext::top_level_navigation(method),
        request_url,
        previous_request_url,
        initiator_url,
    )
}

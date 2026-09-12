mod bindings;
mod input;
mod promise;

use super::request::parse_fetch_init;
use super::*;

pub(crate) use self::bindings::window_fetch_callback;

/// Select the HTTP no-cors fetch path using the logical request URL, before
/// Service Worker or DevTools interception. A DevTools transport URL rewrite
/// does not restart this selection. Local URLs use their scheme-fetch path.
pub(crate) fn validate_no_cors_http_redirect_mode(
    origin: &moli_url::WebOrigin,
    url: &url::Url,
    mode: moli_fetch::RequestMode,
    redirect: moli_fetch::RequestRedirectMode,
) -> Result<(), String> {
    if matches!(url.scheme(), "http" | "https")
        && mode == moli_fetch::RequestMode::NoCors
        && redirect != moli_fetch::RequestRedirectMode::Follow
        && !origin.same_origin_url(url)
    {
        return Err("cross-origin no-cors HTTP fetch requires redirect mode 'follow'".to_owned());
    }
    Ok(())
}

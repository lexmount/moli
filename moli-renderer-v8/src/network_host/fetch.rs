mod bindings;
mod input;
mod integrity;
mod promise;

use super::request::parse_fetch_init;
use super::*;

pub(crate) use self::bindings::window_fetch_callback;

/// Dispatch an internal Fetch operation through the realm's native pipeline.
/// The caller supplies an intrinsic Request, so no mutable global fetch or
/// Request binding participates in this operation.
pub(crate) fn fetch_native_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if crate::worker::get_worker_state(scope).is_some() {
        return crate::worker::fetch_native_request(scope, request);
    }
    let function = v8::Function::new(scope, window_fetch_callback)?;
    let global = scope.get_current_context().global(scope);
    v8::Local::<v8::Promise>::try_from(crate::script_execution::call_function(
        scope,
        function,
        global.into(),
        &[request.into()],
    )?)
    .ok()
}
pub(crate) use self::integrity::validate_fetch_response_integrity;

#[derive(Debug)]
pub(crate) struct NoCorsRedirectModeError;

impl std::fmt::Display for NoCorsRedirectModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cross-origin no-cors HTTP fetch requires redirect mode 'follow'")
    }
}

impl std::error::Error for NoCorsRedirectModeError {}

/// Select the HTTP no-cors fetch path using the logical request URL, before
/// Service Worker or DevTools interception. A DevTools transport URL rewrite
/// does not restart this selection. Local URLs use their scheme-fetch path.
pub(crate) fn validate_no_cors_http_redirect_mode(
    origin: &moli_url::WebOrigin,
    url: &url::Url,
    mode: moli_fetch::RequestMode,
    redirect: moli_fetch::RequestRedirectMode,
) -> Result<(), NoCorsRedirectModeError> {
    if matches!(url.scheme(), "http" | "https")
        && mode == moli_fetch::RequestMode::NoCors
        && redirect != moli_fetch::RequestRedirectMode::Follow
        && !origin.same_origin(&moli_url::WebOrigin::from_url(url))
    {
        return Err(NoCorsRedirectModeError);
    }
    Ok(())
}

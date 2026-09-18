use moli_curl::ProxyRoute;
use url::Url;

use crate::ConnectOptions;

pub(crate) fn websocket_proxy_route(
    url: &Url,
    context: &ConnectOptions,
) -> Result<ProxyRoute, String> {
    moli_curl::select_proxy_route(
        url,
        context.http_proxy.as_deref(),
        context.http_no_proxy.as_deref(),
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn websocket_proxy_route_with_env(
    url: &Url,
    context: &ConnectOptions,
    env: impl FnMut(&str) -> Option<String>,
) -> Result<ProxyRoute, String> {
    moli_curl::select_proxy_route_with_env(
        url,
        context.http_proxy.as_deref(),
        context.http_no_proxy.as_deref(),
        env,
    )
    .map_err(|error| error.to_string())
}

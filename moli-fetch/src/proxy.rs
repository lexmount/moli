use anyhow::Result;
use url::Url;

use crate::FetchConfig;

pub(crate) use moli_curl::ProxyRoute as HttpProxyRoute;

pub(crate) fn resolve_http_proxy_route(config: &FetchConfig, url: &Url) -> Result<HttpProxyRoute> {
    moli_curl::select_proxy_route(url, config.http_proxy(), config.http_no_proxy())
}

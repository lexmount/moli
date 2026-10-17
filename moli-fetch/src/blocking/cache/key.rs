use std::path::Path;

use moli_http_cache::HttpCacheStore;
use moli_url::origin_ascii_serialization;
use url::Url;

use crate::{FetchConfig, Request};

use super::vary::request_headers_allow_http_cache;

pub(super) fn cache_store_and_key_for_request(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
) -> Option<(HttpCacheStore, String)> {
    let cache_dir = config.http_cache_dir()?;
    if !request.cache_mode().allows_http_cache()
        || !request.method.eq_ignore_ascii_case("GET")
        || request.body.is_some()
        || request.auth().is_some()
        || config.web_bot_auth().is_some()
        || !config.default_request_headers().is_empty()
        || !subresource_validation_allows_http_cache(request)
        || cookie_header.is_some()
        || !request_headers_allow_http_cache(&request.request_headers)
        || !matches!(request_url.scheme(), "http" | "https")
        || !request_url.username().is_empty()
        || request_url.password().is_some()
    {
        return None;
    }

    Some((
        HttpCacheStore::with_max_bytes(Path::new(cache_dir), config.http_cache_max_bytes()),
        cache_key_for_request(request, request_url),
    ))
}

fn subresource_validation_allows_http_cache(request: &Request) -> bool {
    request
        .subresource_request_metadata()
        .is_none_or(|metadata| metadata.integrity.is_none())
}

pub(super) fn cache_key_for_request(request: &Request, request_url: &Url) -> String {
    HttpCacheStore::key_for_url(&cache_key_material_for_request(request, request_url))
}

pub(super) fn normalized_cache_url(url: &Url) -> Url {
    let mut normalized = url.clone();
    // HTTP cache keys model the request target, not document-local fragments or
    // URL userinfo. This mirrors Chromium's request-URL keying boundary.
    normalized.set_fragment(None);
    let _ = normalized.set_username("");
    let _ = normalized.set_password(None);
    normalized
}

pub(super) fn normalized_cache_url_string(url: &Url) -> String {
    normalized_cache_url(url).to_string()
}

fn cache_key_material_for_request(request: &Request, request_url: &Url) -> String {
    let browser_context = &request.cookie_context.browser_context;
    let normalized_request_url = normalized_cache_url_string(request_url);
    // Keep this material explicit and versioned. The current cache eligibility
    // is narrow, but baking browser partition dimensions into the key prevents
    // accidental cross-frame reuse when the cacheable request surface expands.
    format!(
        "moli-http-cache-key-v3\nurl:{}\nsite-for-cookies:{}\ntop-frame-origin:{}\nnetwork-partition-key:{}\nrequest-type:{:?}\ncredentials:{}\nstorage-access:{:?}",
        normalized_request_url,
        cache_partition_url_component(browser_context.site_for_cookies_url.as_ref(), request_url),
        cache_partition_url_component(
            browser_context
                .top_frame_origin_url
                .as_ref()
                .or(browser_context.site_for_cookies_url.as_ref()),
            request_url,
        ),
        request.network_partition_key().unwrap_or(""),
        request.cookie_context.request_type,
        request.credentials_mode.as_ref(),
        browser_context.storage_access_status,
    )
}

fn cache_partition_url_component(url: Option<&Url>, default_url: &Url) -> String {
    url.map(origin_ascii_serialization)
        .unwrap_or_else(|| origin_ascii_serialization(default_url))
}

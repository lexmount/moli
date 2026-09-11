use moli_http_cache::{
    HttpCacheEntryMetadata, cacheable_response_parts_policy, response_cache_policy, unix_now_ms,
};
use url::Url;

use crate::{FetchConfig, Request};

use super::{key::normalized_cache_url, vary::vary_headers_for_response};

pub(super) fn cache_metadata_for_response_parts(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    final_url: &Url,
    status: u16,
    status_text: Option<&str>,
    headers: &[(String, String)],
    redirected: bool,
) -> Option<HttpCacheEntryMetadata> {
    let cache_request_url = normalized_cache_url(request_url);
    let cache_final_url = normalized_cache_url(final_url);
    let policy = cacheable_response_parts_policy(
        &cache_request_url,
        &cache_final_url,
        status,
        headers,
        redirected,
    )?;
    let vary_headers = vary_headers_for_response(config, request, request_url, headers)?;
    let mut metadata = HttpCacheEntryMetadata::new(
        cache_request_url.to_string(),
        cache_final_url.to_string(),
        status,
        headers.to_vec(),
        unix_now_ms(),
        policy.expires_at_unix_ms,
        vary_headers,
    );
    metadata.status_text = status_text.map(str::to_owned);
    Some(metadata)
}

pub(crate) fn response_headers_forbid_cache_storage(headers: &[(String, String)]) -> bool {
    !response_cache_policy(headers).store
}

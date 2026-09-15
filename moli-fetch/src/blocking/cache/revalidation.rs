use anyhow::{Context, Result, anyhow};
use moli_http_cache::{merge_not_modified_headers, validation_headers_from_headers};
use url::Url;

use crate::{FetchConfig, Request};

use super::{
    CachedStreamingResponseLookup, key::cache_store_and_key_for_request,
    metadata::cache_metadata_for_response_parts,
};

pub(crate) fn validation_headers_for_cached_streaming_response_lookup(
    lookup: &CachedStreamingResponseLookup,
) -> Vec<(String, String)> {
    validation_headers_from_headers(&lookup.metadata.headers)
}

pub(crate) fn merge_cached_not_modified_streaming_response_lookup(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
    mut cached: CachedStreamingResponseLookup,
    not_modified_headers: &[(String, String)],
) -> Result<CachedStreamingResponseLookup> {
    let headers = merge_not_modified_headers(&cached.headers, not_modified_headers);
    cached.headers = headers.clone();
    cached.metadata.headers = headers.clone();

    let final_url = Url::parse(&cached.final_url).with_context(|| {
        anyhow!(
            "failed to parse cached response final url `{}`",
            cached.final_url
        )
    })?;
    if let Some(metadata) = cache_metadata_for_response_parts(
        config,
        request,
        request_url,
        &final_url,
        cached.status,
        cached.metadata.status_text.as_deref(),
        &headers,
        false,
    ) {
        cached.metadata.request_url = metadata.request_url;
        cached.metadata.final_url = metadata.final_url;
        cached.metadata.status = metadata.status;
        cached.metadata.headers = metadata.headers;
        cached.metadata.stored_at_unix_ms = metadata.stored_at_unix_ms;
        cached.metadata.last_used_at_unix_ms = metadata.last_used_at_unix_ms;
        cached.metadata.expires_at_unix_ms = metadata.expires_at_unix_ms;
        cached.metadata.vary_headers = metadata.vary_headers;
        cached.expires_at_unix_ms = cached.metadata.expires_at_unix_ms;
        if let Some((store, key)) =
            cache_store_and_key_for_request(config, request, request_url, cookie_header)
            && key == cached.key
        {
            store.refresh_loaded_entry_metadata(&key, &cached.metadata)?;
        }
    }

    Ok(cached)
}

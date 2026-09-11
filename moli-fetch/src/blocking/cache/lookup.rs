use anyhow::Result;
use moli_http_cache::{
    HttpCacheBodyWriter, HttpCachedEntryReader, cached_response_is_fresh_immutable,
};
use url::Url;

use crate::{FetchConfig, Request};

use super::{
    CachedStreamingResponseLookup,
    key::{cache_store_and_key_for_request, normalized_cache_url_string},
    metadata::cache_metadata_for_response_parts,
    request_cache_control_requires_validation,
    vary::vary_headers_match,
};

pub(crate) fn load_cached_streaming_response_lookup(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
) -> Result<Option<CachedStreamingResponseLookup>> {
    if !request.cache_mode().allows_http_cache_lookup() {
        return Ok(None);
    }
    let Some((store, key)) =
        cache_store_and_key_for_request(config, request, request_url, cookie_header)
    else {
        return Ok(None);
    };
    let Some(cached) = store.load_reader(&key)? else {
        return Ok(None);
    };
    let cache_request_url = normalized_cache_url_string(request_url);
    if cached.metadata.request_url != cache_request_url
        || !vary_headers_match(config, request, request_url, &cached.metadata.vary_headers)
    {
        return Ok(None);
    }
    if entry_reader_body_exceeds_response_limit(config, &cached) {
        tracing::debug!(
            url = %request_url,
            "cached streaming response body exceeds configured response limit; treating as cache miss"
        );
        return Ok(None);
    }
    let mut cached = cached;
    // Normalized cache keys should not leak the first request's fragment into
    // later streaming hits.
    cached.metadata.final_url = request_url.to_string();
    let force_validate = cache_lookup_force_validate(config, request, request_url, &cached);
    let _ = store.touch_loaded_entry(&key, &cached.metadata);
    Ok(Some(cached_streaming_record_from_entry(
        key,
        cached,
        force_validate,
    )))
}

pub(crate) fn cached_streaming_response_body_exceeds_response_limit(
    config: &FetchConfig,
    cached: &CachedStreamingResponseLookup,
) -> bool {
    config
        .http_max_response_size()
        .zip(
            cached
                .body
                .get_ref()
                .metadata()
                .ok()
                .map(|metadata| metadata.len()),
        )
        .is_some_and(|(limit, body_len)| body_len > limit as u64)
}

pub(crate) fn remove_cached_response(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
) -> Result<()> {
    let Some((store, key)) =
        cache_store_and_key_for_request(config, request, request_url, cookie_header)
    else {
        return Ok(());
    };
    store.remove_entry(&key)
}

pub(crate) fn create_streaming_cache_body_writer_for_response_parts(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
    status: u16,
    headers: &[(String, String)],
) -> Result<Option<HttpCacheBodyWriter>> {
    let Some((store, key)) =
        cache_store_and_key_for_request(config, request, request_url, cookie_header)
    else {
        return Ok(None);
    };
    let Some(_) = cache_metadata_for_response_parts(
        config,
        request,
        request_url,
        request_url,
        status,
        None,
        headers,
        false,
    ) else {
        return Ok(None);
    };
    Ok(Some(store.create_body_writer(&key)?))
}

pub(crate) fn finish_streaming_cached_response(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cookie_header: Option<&str>,
    final_url: &Url,
    status: u16,
    status_text: Option<&str>,
    headers: &[(String, String)],
    redirected: bool,
    writer: HttpCacheBodyWriter,
) -> Result<()> {
    let Some((_, _)) = cache_store_and_key_for_request(config, request, request_url, cookie_header)
    else {
        return Ok(());
    };
    let Some(metadata) = cache_metadata_for_response_parts(
        config,
        request,
        request_url,
        final_url,
        status,
        status_text,
        headers,
        redirected,
    ) else {
        return Ok(());
    };

    writer.finish(metadata)?;
    tracing::debug!(url = %request_url, "stored streaming response in disk cache");
    Ok(())
}

fn cached_streaming_record_from_entry(
    key: String,
    entry: HttpCachedEntryReader,
    force_validate: bool,
) -> CachedStreamingResponseLookup {
    let metadata = entry.metadata.clone();
    CachedStreamingResponseLookup {
        key,
        metadata,
        final_url: entry.metadata.final_url,
        status: entry.metadata.status,
        headers: entry.metadata.headers,
        body: entry.body,
        expires_at_unix_ms: entry.metadata.expires_at_unix_ms,
        force_validate,
    }
}

fn entry_reader_body_exceeds_response_limit(
    config: &FetchConfig,
    cached: &HttpCachedEntryReader,
) -> bool {
    config
        .http_max_response_size()
        .zip(cached.body_len().ok())
        .is_some_and(|(limit, body_len)| body_len > limit as u64)
}

fn cache_lookup_force_validate(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    cached: &HttpCachedEntryReader,
) -> bool {
    request.cache_mode().requires_http_cache_validation()
        || (request_cache_control_requires_validation(config, request, request_url)
            && !cached_response_is_fresh_immutable(
                &cached.metadata.headers,
                cached.metadata.expires_at_unix_ms,
            ))
}

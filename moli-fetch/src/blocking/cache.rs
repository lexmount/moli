mod key;
mod lookup;
mod metadata;
mod revalidation;
mod vary;

use anyhow::{Context, Result, anyhow, bail};
use moli_http_cache::{
    HttpCacheEntryMetadata, HttpCacheStats, HttpCacheStore, cached_response_is_stale,
    request_header_requires_validation,
};
use moli_url_policy::ensure_http_network_transport_url;
use url::Url;

use std::path::Path;

use crate::{FetchConfig, Request};

use super::outgoing_request_headers_for_url;

pub fn clear_http_cache(config: &FetchConfig) -> Result<()> {
    let Some(cache_dir) = config.http_cache_dir() else {
        return Ok(());
    };
    clear_http_cache_root(cache_dir, config.http_cache_max_bytes())
}

pub fn clear_http_cache_for_origin(config: &FetchConfig, origin: &Url) -> Result<usize> {
    let Some(cache_dir) = config.http_cache_dir() else {
        return Ok(0);
    };
    clear_http_cache_root_for_origin(cache_dir, config.http_cache_max_bytes(), origin)
}

pub fn clear_http_cache_root(cache_dir: impl AsRef<Path>, max_bytes: Option<u64>) -> Result<()> {
    HttpCacheStore::with_max_bytes(cache_dir.as_ref(), max_bytes).clear()
}

pub fn clear_http_cache_root_for_origin(
    cache_dir: impl AsRef<Path>,
    max_bytes: Option<u64>,
    origin: &Url,
) -> Result<usize> {
    HttpCacheStore::with_max_bytes(cache_dir.as_ref(), max_bytes).remove_entries_for_origin(origin)
}

pub fn trim_http_cache(config: &FetchConfig) -> Result<()> {
    let Some(cache_dir) = config.http_cache_dir() else {
        return Ok(());
    };
    HttpCacheStore::with_max_bytes(
        std::path::Path::new(cache_dir),
        config.http_cache_max_bytes(),
    )
    .trim_to_max_bytes();
    Ok(())
}

pub fn http_cache_stats(config: &FetchConfig) -> Result<HttpCacheStats> {
    let Some(cache_dir) = config.http_cache_dir() else {
        return Ok(HttpCacheStats::default());
    };
    HttpCacheStore::with_max_bytes(
        std::path::Path::new(cache_dir),
        config.http_cache_max_bytes(),
    )
    .stats()
}

pub(crate) use self::{
    lookup::{
        cached_streaming_response_body_exceeds_response_limit,
        create_streaming_cache_body_writer_for_response_parts, finish_streaming_cached_response,
        load_cached_streaming_response_lookup, remove_cached_response,
    },
    metadata::response_headers_forbid_cache_storage,
    revalidation::{
        merge_cached_not_modified_streaming_response_lookup,
        validation_headers_for_cached_streaming_response_lookup,
    },
    vary::{vary_headers_for_response, vary_headers_match},
};

#[derive(Debug)]
pub(crate) struct CachedStreamingResponseLookup {
    pub(crate) key: String,
    pub(crate) metadata: HttpCacheEntryMetadata,
    pub(crate) final_url: String,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: std::io::BufReader<std::fs::File>,
    pub(crate) expires_at_unix_ms: Option<u64>,
    pub(crate) force_validate: bool,
}

pub(crate) fn cached_streaming_response_is_stale(record: &CachedStreamingResponseLookup) -> bool {
    cached_response_is_stale(record.expires_at_unix_ms, record.force_validate)
}

pub(super) fn request_cache_control_requires_validation(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
) -> bool {
    outgoing_request_headers_for_url(config, request, request_url, &[], None)
        .iter()
        .any(|(name, value)| request_header_requires_validation(name, value))
}

pub(crate) fn next_redirect_url_from_parts(
    final_url: &Url,
    status: u16,
    headers: &[(String, String)],
    redirect_count: usize,
) -> Result<Option<Url>> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }

    let Some(location) = headers
        .iter()
        .find(|(name, _)| name == "location")
        .map(|(_, value)| value.as_str())
    else {
        return Ok(None);
    };

    if redirect_count >= super::MAX_REDIRECTS {
        bail!("redirect limit exceeded for {}", final_url);
    }

    final_url
        .join(location)
        .or_else(|_| Url::parse(location))
        .map(Some)
        .with_context(|| {
            anyhow!(
                "failed to resolve redirect location `{location}` from {}",
                final_url
            )
        })
}

pub(crate) fn next_followed_redirect_url_from_parts(
    final_url: &Url,
    status: u16,
    headers: &[(String, String)],
    redirect_count: usize,
    follow_redirects: bool,
) -> Result<Option<Url>> {
    let next_url = next_redirect_url_from_parts(final_url, status, headers, redirect_count)?;
    if follow_redirects && let Some(next_url) = next_url.as_ref() {
        ensure_http_network_transport_url(next_url)?;
    }
    Ok(next_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_includes_browser_partition_context() -> Result<()> {
        let request_url = Url::parse("https://cdn.example.test/app.css")?;
        let first_top_frame = Url::parse("https://app.example.test/")?;
        let second_top_frame = Url::parse("https://other.example.test/")?;
        let first =
            Request::get_with_url(request_url.clone()).with_top_frame_origin_url(&first_top_frame);
        let second =
            Request::get_with_url(request_url.clone()).with_top_frame_origin_url(&second_top_frame);

        assert_ne!(
            key::cache_key_for_request(&first, &request_url),
            key::cache_key_for_request(&second, &request_url)
        );
        Ok(())
    }

    #[test]
    fn followed_redirect_rejects_non_http_target_before_request_state_changes() -> Result<()> {
        let current = Url::parse("https://example.test/start")?;
        let headers = vec![(
            "location".to_owned(),
            "file:///moli-policy-must-not-open".to_owned(),
        )];

        let error = next_followed_redirect_url_from_parts(&current, 302, &headers, 0, true)
            .expect_err("followed file redirect must be rejected");
        assert_eq!(
            error.to_string(),
            "URL scheme \"file\" is not supported by the HTTP network transport."
        );

        let manual = next_followed_redirect_url_from_parts(&current, 302, &headers, 0, false)?
            .expect("manual redirect URL should remain observable");
        assert_eq!(manual.as_str(), "file:///moli-policy-must-not-open");
        Ok(())
    }

    #[test]
    fn cache_key_includes_network_partition_key() -> Result<()> {
        let request_url = Url::parse("https://cdn.example.test/app.css")?;
        let first = Request::get_with_url(request_url.clone()).with_network_partition_key(Some(
            "storage-key:v1;origin=https://cdn.example.test;top-level-site=https://app.example.test;opaque-nonce=1"
                .to_owned(),
        ));
        let second = Request::get_with_url(request_url.clone()).with_network_partition_key(Some(
            "storage-key:v1;origin=https://cdn.example.test;top-level-site=https://app.example.test;opaque-nonce=2"
                .to_owned(),
        ));

        assert_ne!(
            key::cache_key_for_request(&first, &request_url),
            key::cache_key_for_request(&second, &request_url)
        );
        Ok(())
    }

    #[test]
    fn cache_key_ignores_fragment_and_userinfo() -> Result<()> {
        let first_url = Url::parse("https://user:secret@cdn.example.test/app.css#one")?;
        let second_url = Url::parse("https://cdn.example.test/app.css#two")?;
        let first = Request::get_with_url(first_url.clone());
        let second = Request::get_with_url(second_url.clone());

        assert_eq!(
            key::cache_key_for_request(&first, &first_url),
            key::cache_key_for_request(&second, &second_url)
        );
        assert_eq!(
            key::normalized_cache_url_string(&first_url),
            "https://cdn.example.test/app.css"
        );
        Ok(())
    }
}

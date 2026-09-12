use std::str;

use moli_http_cache::{HttpCacheVaryHeader, response_vary_header_names};
use url::Url;

use crate::{FetchConfig, Request};

use super::super::outgoing_request_headers_for_url;

const SUPPORTED_CACHE_REQUEST_HEADERS: &[&str] = &[
    "accept",
    "accept-encoding",
    "accept-language",
    "referer",
    "sec-ch-ua",
    "sec-ch-ua-arch",
    "sec-ch-ua-bitness",
    "sec-ch-ua-form-factors",
    "sec-ch-ua-full-version",
    "sec-ch-ua-full-version-list",
    "sec-ch-ua-mobile",
    "sec-ch-ua-model",
    "sec-ch-ua-platform",
    "sec-ch-ua-platform-version",
    "sec-ch-ua-wow64",
    "sec-fetch-dest",
    "sec-fetch-mode",
    "sec-fetch-site",
    "sec-fetch-user",
    "upgrade-insecure-requests",
    "user-agent",
];

pub(super) fn vary_headers_for_response(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    response_headers: &[(String, String)],
) -> Option<Vec<HttpCacheVaryHeader>> {
    let mut out = Vec::new();
    for normalized_name in response_vary_header_names(response_headers)? {
        if !supported_vary_header(&normalized_name) {
            return None;
        }
        out.push(HttpCacheVaryHeader {
            value: vary_request_header_value(config, request, request_url, &normalized_name),
            name: normalized_name,
        });
    }
    Some(out)
}

pub(super) fn vary_headers_match(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    vary_headers: &[HttpCacheVaryHeader],
) -> bool {
    vary_headers.iter().all(|header| {
        vary_request_header_value(config, request, request_url, &header.name) == header.value
    })
}

pub(super) fn request_headers_allow_http_cache(headers: &[(String, String)]) -> bool {
    headers.iter().enumerate().all(|(index, (name, _))| {
        supported_cache_request_header(name)
            && !headers[..index]
                .iter()
                .any(|(previous, _)| previous.eq_ignore_ascii_case(name))
    })
}

fn vary_request_header_value(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    normalized_name: &str,
) -> Option<String> {
    let explicit_value = || {
        outgoing_request_headers_for_url(config, request, request_url, None)
            .into_iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case(normalized_name))
            .map(|(_, value)| value)
    };
    if normalized_name == "user-agent" {
        return explicit_value().or_else(|| Some(config.user_agent().to_owned()));
    }
    if normalized_name == "accept-encoding" {
        // libcurl synthesizes this header from `easy.accept_encoding("")`.
        return explicit_value().or_else(|| Some("libcurl-auto".to_owned()));
    }
    explicit_value()
}

fn supported_vary_header(normalized_name: &str) -> bool {
    supported_cache_request_header(normalized_name)
}

fn supported_cache_request_header(name: &str) -> bool {
    // Keep the allowlist to non-sensitive browser headers whose effective
    // values this module can reproduce for Vary matching. Arbitrary custom,
    // cookie, authorization, and range headers remain cache-ineligible.
    SUPPORTED_CACHE_REQUEST_HEADERS
        .iter()
        .any(|supported| name.eq_ignore_ascii_case(supported))
}

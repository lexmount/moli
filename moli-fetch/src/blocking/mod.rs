mod cache;
mod collectors;

use std::{
    ffi::{c_char, c_long, c_void},
    net::{IpAddr, ToSocketAddrs},
    str,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use curl::easy::{Auth, Easy2, Handler, HttpVersion, List};
use moli_cookie_jar::{
    NetworkCookieRequestContext, SharedBrowserCookieStore, StoredCookieQueryReport,
    StoredCookieSetReport, same_site_urls,
};
use moli_url::{
    is_potentially_trustworthy_url, origin_ascii_serialization, same_origin, tuple_origin_url,
};
use moli_url_policy::ensure_http_network_transport_url;
use tracing::debug;
use url::Url;

pub(crate) use self::cache::{
    CachedStreamingResponseLookup, cached_streaming_response_body_exceeds_response_limit,
    cached_streaming_response_is_stale, create_streaming_cache_body_writer_for_response_parts,
    finish_streaming_cached_response, load_cached_streaming_response_lookup,
    merge_cached_not_modified_streaming_response_lookup, next_followed_redirect_url_from_parts,
    next_redirect_url_from_parts, remove_cached_response, response_headers_forbid_cache_storage,
    validation_headers_for_cached_streaming_response_lookup,
};
pub use self::cache::{
    clear_http_cache, clear_http_cache_for_origin, clear_http_cache_root,
    clear_http_cache_root_for_origin, http_cache_stats, trim_http_cache,
};
pub(crate) use self::collectors::{
    RawStreamingResponseCollector, RequestTransferMetrics, ResponseCollector, StreamingCachePlan,
    log_request_completion, transfer_metrics_from_easy,
};

use crate::{
    BrowserRequestMetadata, FetchConfig, NegotiatedHttpVersion, NetworkRequestExtraInfo,
    RedirectInfo, Request, RequestAuthScheme, RequestAuthTarget, ResponseHead,
};

const MAX_REDIRECTS: usize = 10;
// curl-sys in the pinned curl-rust fork does not yet export the string options
// added in libcurl 7.85.0. CURLoption values are stable public ABI values; keep
// these definitions adjacent to the only raw setopt call that needs them.
const CURL_OPTION_PROTOCOLS_STR: curl_sys::CURLoption = curl_sys::CURLOPTTYPE_OBJECTPOINT + 318;
const CURL_OPTION_REDIR_PROTOCOLS_STR: curl_sys::CURLoption =
    curl_sys::CURLOPTTYPE_OBJECTPOINT + 319;
const CURL_HTTP_PROTOCOL_ALLOWLIST: &[u8; 11] = b"http,https\0";
// Leave enough of the default page deadline to settle a failed DCL-critical
// child request and continue parsing. Main navigations retain caller policy.
const DEFAULT_BROWSER_SUBRESOURCE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RequestHttpVersion {
    #[default]
    PreferHttp2,
    Http1Only,
}

#[derive(Debug)]
pub struct StreamingHtmlResponseStart {
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    pub network_request_extra_info: Option<NetworkRequestExtraInfo>,
}

impl StreamingHtmlResponseStart {
    pub fn into_head(self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain,
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        }
    }
}

pub use collectors::StreamingResponseCollector;

pub fn cookie_header_for_request(
    cookie_store: &SharedBrowserCookieStore,
    request_url: &Url,
    request_context: NetworkCookieRequestContext,
) -> Result<Option<String>> {
    let request_cookie_report =
        cookie_access_report_for_request(cookie_store, request_url, request_context)?;
    Ok(cookie_header_from_report(request_cookie_report.as_ref()))
}

pub(crate) fn cookie_access_report_for_request(
    cookie_store: &SharedBrowserCookieStore,
    request_url: &Url,
    request_context: NetworkCookieRequestContext,
) -> Result<Option<StoredCookieQueryReport>> {
    let mut cookie_store = cookie_store.lock();
    let report = cookie_store.cookie_access_report_for_request(request_url, request_context);
    Ok(
        (!report.included_cookies.is_empty() || !report.excluded_cookies.is_empty())
            .then_some(report),
    )
}

pub fn observe_cookie_access_report_for_request(
    cookie_store: &SharedBrowserCookieStore,
    request_url: &Url,
    request_context: NetworkCookieRequestContext,
) -> Result<Option<StoredCookieQueryReport>> {
    let mut cookie_store = cookie_store.lock();
    let report =
        cookie_store.observe_cookie_access_report_for_request(request_url, request_context);
    Ok(
        (!report.included_cookies.is_empty() || !report.excluded_cookies.is_empty())
            .then_some(report),
    )
}

pub(crate) fn cookie_header_from_report(
    report: Option<&StoredCookieQueryReport>,
) -> Option<String> {
    let report = report?;
    if report.included_cookies.is_empty() {
        return None;
    }
    Some(
        report
            .included_cookies
            .iter()
            .map(|entry| format!("{}={}", entry.cookie.name, entry.cookie.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn outgoing_request_headers(
    config: &FetchConfig,
    request: &Request,
    cookie_header: Option<&str>,
) -> Vec<(String, String)> {
    outgoing_request_headers_for_url(config, request, &request.url, &[], cookie_header)
}

pub(crate) fn outgoing_request_headers_for_url(
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
    cookie_header: Option<&str>,
) -> Vec<(String, String)> {
    let mut outgoing = Vec::new();

    if let Some(proxy_bearer_token) = config.proxy_bearer_token() {
        outgoing.push((
            "Proxy-Authorization".to_owned(),
            format!("Bearer {proxy_bearer_token}"),
        ));
    }

    if let Some(cookie_header) = cookie_header {
        outgoing.push(("Cookie".to_owned(), cookie_header.to_owned()));
    }

    for (name, value) in config.default_request_headers() {
        if cookie_header.is_some() && name.eq_ignore_ascii_case("cookie") {
            continue;
        }
        outgoing.push((name.clone(), value.clone()));
    }

    for (name, value) in &request.request_headers {
        if cookie_header.is_some() && name.eq_ignore_ascii_case("cookie") {
            continue;
        }
        outgoing.push((name.clone(), value.clone()));
    }

    append_browser_navigation_headers(&mut outgoing, config, request, request_url, redirect_chain);
    append_browser_subresource_headers(&mut outgoing, config, request, request_url, redirect_chain);
    append_browser_storage_access_header(&mut outgoing, request, request_url);

    if !header_present(&outgoing, "referer")
        && let Some(referer) = referrer_header_value_for_request(request, request_url)
    {
        outgoing.push(("Referer".to_owned(), referer));
    }

    if !header_present(&outgoing, "authorization")
        && let Some((username, password)) =
            request.preemptive_server_basic_auth_for_url(request_url)
    {
        // Basic auth is just a deterministic request header. Sending it
        // preemptively lets streaming transports avoid libcurl's intermediate
        // 401 retry body while Digest/NTLM/Negotiate stay on the buffered path.
        outgoing.push((
            "Authorization".to_owned(),
            format!("Basic {}", encode_basic_auth(username, password)),
        ));
    }

    if let Some(auth) = request.auth()
        && auth.target == RequestAuthTarget::Server
        && auth.scheme == RequestAuthScheme::Basic
        && !header_present(&outgoing, "authorization")
    {
        outgoing.push((
            "Authorization".to_owned(),
            format!(
                "Basic {}",
                encode_basic_auth(&auth.username, &auth.password)
            ),
        ));
    }

    if let Some(auth) = request.auth()
        && auth.target == RequestAuthTarget::ProxyHeader
        && !header_present(&outgoing, "proxy-authorization")
    {
        outgoing.push((
            "Proxy-Authorization".to_owned(),
            format!(
                "Basic {}",
                encode_basic_auth(&auth.username, &auth.password)
            ),
        ));
    }

    outgoing
}

pub(crate) fn network_request_extra_info_from_headers(
    config: &FetchConfig,
    outgoing_headers: &[(String, String)],
    cookie_report: Option<&StoredCookieQueryReport>,
) -> NetworkRequestExtraInfo {
    let mut headers = outgoing_headers.to_vec();
    append_header_if_missing(&mut headers, "User-Agent", config.user_agent().to_owned());
    NetworkRequestExtraInfo {
        headers,
        cookie_report: cookie_report.cloned().unwrap_or_default(),
    }
}

fn header_present(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
}

fn append_header_if_missing(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if !header_present(headers, name) {
        headers.push((name.to_owned(), value));
    }
}

fn append_browser_navigation_headers(
    outgoing: &mut Vec<(String, String)>,
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
) {
    if !request.is_navigation_request() || !matches!(request_url.scheme(), "http" | "https") {
        return;
    }

    append_header_if_missing(
        outgoing,
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".to_owned(),
    );
    append_header_if_missing(
        outgoing,
        "Accept-Language",
        config.browser_identity().accept_language().to_owned(),
    );
    append_header_if_missing(outgoing, "Upgrade-Insecure-Requests", "1".to_owned());
    append_header_if_missing(outgoing, "Sec-Fetch-Mode", "navigate".to_owned());
    append_header_if_missing(
        outgoing,
        "Sec-Fetch-Dest",
        if request.is_subframe_navigation_request() {
            "iframe"
        } else {
            "document"
        }
        .to_owned(),
    );
    append_header_if_missing(
        outgoing,
        "Sec-Fetch-Site",
        request_sec_fetch_site(request, request_url),
    );

    if request.is_top_level_navigation_request() && request.cookie_context.initiator_url.is_none() {
        append_header_if_missing(outgoing, "Sec-Fetch-User", "?1".to_owned());
    }

    if request.browser_navigation_kind() == crate::BrowserNavigationRequestKind::Reload {
        append_header_if_missing(outgoing, "Cache-Control", "max-age=0".to_owned());
    }

    if let Some(origin) = request_origin_header_value(request, request_url, redirect_chain) {
        append_header_if_missing(outgoing, "Origin", origin);
    }

    append_browser_client_hints(outgoing, config);
}

fn append_browser_subresource_headers(
    outgoing: &mut Vec<(String, String)>,
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
) {
    let Some(metadata) = request.browser_request_metadata() else {
        return;
    };
    if !matches!(request_url.scheme(), "http" | "https") {
        return;
    }

    match metadata {
        BrowserRequestMetadata::Audio
        | BrowserRequestMetadata::AudioWorklet
        | BrowserRequestMetadata::Beacon
        | BrowserRequestMetadata::EventSource
        | BrowserRequestMetadata::Fetch
        | BrowserRequestMetadata::Font
        | BrowserRequestMetadata::Image
        | BrowserRequestMetadata::JsonModule
        | BrowserRequestMetadata::Manifest
        | BrowserRequestMetadata::Ping
        | BrowserRequestMetadata::Style
        | BrowserRequestMetadata::StyleModule
        | BrowserRequestMetadata::TextTrack
        | BrowserRequestMetadata::Video
        | BrowserRequestMetadata::Xhr => {
            let accept = match metadata {
                BrowserRequestMetadata::Image => {
                    "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"
                }
                BrowserRequestMetadata::EventSource => "text/event-stream",
                BrowserRequestMetadata::Manifest => "*/*",
                BrowserRequestMetadata::TextTrack => "text/vtt,*/*;q=0.1",
                _ => "*/*",
            };
            append_header_if_missing(outgoing, "Accept", accept.to_owned());
            append_header_if_missing(
                outgoing,
                "Accept-Language",
                config.browser_identity().accept_language().to_owned(),
            );
            append_header_if_missing(
                outgoing,
                "Sec-Fetch-Site",
                request_sec_fetch_site(request, request_url),
            );
            append_header_if_missing(
                outgoing,
                "Sec-Fetch-Mode",
                request.request_mode.as_ref().to_owned(),
            );
            let destination = match metadata {
                BrowserRequestMetadata::Audio => "audio",
                BrowserRequestMetadata::AudioWorklet => "audioworklet",
                BrowserRequestMetadata::Font => "font",
                BrowserRequestMetadata::Image => "image",
                BrowserRequestMetadata::JsonModule => "json",
                BrowserRequestMetadata::Manifest => "manifest",
                BrowserRequestMetadata::Style | BrowserRequestMetadata::StyleModule => "style",
                BrowserRequestMetadata::TextTrack => "track",
                BrowserRequestMetadata::Video => "video",
                BrowserRequestMetadata::Beacon
                | BrowserRequestMetadata::EventSource
                | BrowserRequestMetadata::Fetch
                | BrowserRequestMetadata::Ping
                | BrowserRequestMetadata::Xhr => "empty",
            };
            append_header_if_missing(outgoing, "Sec-Fetch-Dest", destination.to_owned());
            let origin = if matches!(metadata, BrowserRequestMetadata::Style)
                && !matches!(request.request_mode, crate::RequestMode::Cors)
            {
                None
            } else {
                request_origin_header_value(request, request_url, redirect_chain)
            };
            if let Some(origin) = origin {
                append_header_if_missing(outgoing, "Origin", origin);
            }
            append_browser_client_hints(outgoing, config);
        }
    }
}

fn request_origin_header_value(
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
) -> Option<String> {
    if !request_needs_origin_header(request, request_url, redirect_chain) {
        return None;
    }
    let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() else {
        return Some("null".to_owned());
    };
    let Some(initiator_origin_url) = tuple_origin_url(initiator_url) else {
        return Some("null".to_owned());
    };
    Some(
        if request_has_redirect_tainted_origin(
            initiator_origin_url.as_ref(),
            &request.url,
            redirect_chain,
        ) {
            "null".to_owned()
        } else {
            origin_ascii_serialization(initiator_origin_url.as_ref())
        },
    )
}

fn request_needs_origin_header(
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
) -> bool {
    if !matches!(request_url.scheme(), "http" | "https") {
        return false;
    }
    if !request.method.eq_ignore_ascii_case("GET") && !request.method.eq_ignore_ascii_case("HEAD") {
        return true;
    }
    if !matches!(request.request_mode, crate::RequestMode::Cors) {
        return false;
    }

    let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() else {
        return true;
    };
    let Some(initiator_origin_url) = tuple_origin_url(initiator_url) else {
        return true;
    };
    !same_origin(initiator_origin_url.as_ref(), request_url)
        || request_has_redirect_tainted_origin(
            initiator_origin_url.as_ref(),
            &request.url,
            redirect_chain,
        )
}

fn request_has_redirect_tainted_origin(
    request_origin_url: &Url,
    original_request_url: &Url,
    redirect_chain: &[RedirectInfo],
) -> bool {
    let mut last_url = original_request_url;

    for redirect in redirect_chain {
        let next_url = &redirect.to_url;
        if !same_origin(next_url, last_url) && !same_origin(request_origin_url, last_url) {
            return true;
        }
        last_url = next_url;
    }

    false
}

fn append_browser_client_hints(outgoing: &mut Vec<(String, String)>, config: &FetchConfig) {
    let identity = config.browser_identity();
    let Some(sec_ch_ua) = identity.sec_ch_ua_value() else {
        return;
    };
    append_header_if_missing(outgoing, "Sec-CH-UA", sec_ch_ua);
    append_header_if_missing(
        outgoing,
        "Sec-CH-UA-Mobile",
        if identity.mobile() { "?1" } else { "?0" }.to_owned(),
    );
    append_header_if_missing(
        outgoing,
        "Sec-CH-UA-Platform",
        format!("\"{}\"", identity.platform()),
    );
}

fn append_browser_storage_access_header(
    outgoing: &mut Vec<(String, String)>,
    request: &Request,
    request_url: &Url,
) {
    if request.credentials_mode != crate::RequestCredentialsMode::Include
        || (!request.is_navigation_request() && request.browser_request_metadata().is_none())
        || !matches!(request_url.scheme(), "http" | "https")
        || !is_potentially_trustworthy_url(request_url)
    {
        return;
    }

    let Some(site_for_cookies) = request.cookie_context.browser_context.site_basis_url() else {
        return;
    };
    if same_site_urls(site_for_cookies, request_url, true) {
        return;
    }

    // Chromium reports `active` whenever a third-party, credentialed request
    // has unpartitioned cookie access, even when no cookie happens to match.
    // Moli does not currently expose a third-party-cookie blocking mode,
    // so its network cookie policy has the same effective state. If such a
    // blocker is added, this value must be derived from that policy instead.
    append_header_if_missing(outgoing, "Sec-Fetch-Storage-Access", "active".to_owned());
}

fn request_sec_fetch_site(request: &Request, request_url: &Url) -> String {
    let Some(initiator_url) = request.cookie_context.initiator_url.as_ref() else {
        return "none".to_owned();
    };
    let initiator_url =
        tuple_origin_url(initiator_url).unwrap_or(std::borrow::Cow::Borrowed(initiator_url));
    if same_origin(initiator_url.as_ref(), request_url) {
        "same-origin".to_owned()
    } else if same_site_urls(initiator_url.as_ref(), request_url, true) {
        "same-site".to_owned()
    } else {
        "cross-site".to_owned()
    }
}

fn referrer_header_value_for_request(request: &Request, request_url: &Url) -> Option<String> {
    if !request.infers_referrer_from_initiator() {
        return None;
    }
    let referrer_url = request.cookie_context.initiator_url.as_ref()?;
    let (referrer_policy, document_referrer_policy) = request
        .subresource_request_metadata()
        .map(|metadata| {
            (
                metadata.referrer_policy.as_deref(),
                metadata.document_referrer_policy.as_deref(),
            )
        })
        .unwrap_or((None, None));
    crate::referrer_header_value(
        referrer_url,
        request_url,
        referrer_policy,
        document_referrer_policy,
    )
}

pub(crate) fn store_response_cookies(
    cookie_store: &SharedBrowserCookieStore,
    response_url: &Url,
    headers: &[(String, String)],
    request_context: &NetworkCookieRequestContext,
) -> Result<Vec<StoredCookieSetReport>> {
    let mut cookie_store = cookie_store.lock();
    Ok(cookie_store.store_response_headers_with_context_reports(
        response_url,
        headers,
        request_context,
    ))
}

pub(crate) fn configure_easy<H: Handler>(
    easy: &mut Easy2<H>,
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
    redirect_chain: &[RedirectInfo],
    cookie_header: Option<&str>,
    http_version: RequestHttpVersion,
    validation_headers: Option<Vec<(String, String)>>,
) -> Result<Vec<(String, String)>> {
    ensure_http_network_transport_url(request_url)?;
    enforce_request_target_policy(config, request_url)?;
    configure_curl_http_protocol_allowlist(easy)?;

    // Keep proxy CONNECT handshake headers out of the response callbacks so
    // the collector only processes real HTTP responses.
    let setopt_result = unsafe {
        curl_sys::curl_easy_setopt(
            easy.raw(),
            curl_sys::CURLOPT_SUPPRESS_CONNECT_HEADERS,
            1 as c_long,
        )
    };
    if setopt_result != curl_sys::CURLE_OK {
        return Err(curl::Error::new(setopt_result))
            .context("failed to suppress proxy CONNECT headers");
    }

    let request_timeout = request.effective_request_timeout(config);
    easy.timeout(request_timeout)
        .context("failed to set curl request timeout")?;
    easy.progress(true)
        .context("failed to enable curl progress callback")?;
    if let Some(connect_timeout) = effective_connect_timeout(config, request) {
        easy.connect_timeout(connect_timeout)
            .context("failed to set curl connect timeout")?;
    }

    let curl_http_version = match http_version {
        RequestHttpVersion::PreferHttp2 => HttpVersion::V2TLS,
        RequestHttpVersion::Http1Only => HttpVersion::V11,
    };
    if let Err(error) = easy.http_version(curl_http_version) {
        debug!(url = %request_url, ?http_version, "failed to configure HTTP version for request: {error}");
    }
    // Do not enable CURLOPT_PIPEWAIT here. Moli often discovers a burst
    // of same-origin script/modulepreload requests during parsing, and DCL can
    // depend on one of those scripts. Waiting for a pending HTTPS connection to
    // become multiplexable can put critical script fetches behind a slow TLS/H2
    // setup or slow stream. Let libcurl start eligible transfers immediately;
    // the runtime-level max-active / max-host caps still bound concurrency.
    if let Err(error) = easy.tcp_keepalive(true) {
        debug!(url = %request_url, "failed to enable TCP keepalive: {error}");
    }
    if let Err(error) = easy.dns_cache_timeout(Duration::from_secs(60)) {
        debug!(url = %request_url, "failed to configure curl DNS cache timeout: {error}");
    }
    if let Some(max_connects) = config
        .http_max_total_connections()
        .or_else(|| config.effective_http_max_host_connections().map(u16::from))
        .filter(|value| *value > 0)
        && let Err(error) = easy.max_connects(u32::from(max_connects))
    {
        debug!(url = %request_url, "failed to configure curl max_connects: {error}");
    }

    easy.follow_location(false)
        .context("failed to disable curl redirect following")?;
    easy.accept_encoding("")
        .context("failed to enable curl response decompression")?;

    easy.ssl_verify_peer(config.tls_verify_host())
        .context("failed to configure curl TLS peer verification")?;
    easy.ssl_verify_host(config.tls_verify_host())
        .context("failed to configure curl TLS host verification")?;
    easy.useragent(config.user_agent())
        .context("failed to set curl user-agent")?;
    easy.url(request_url.as_str())
        .with_context(|| anyhow!("failed to set curl request url to {}", request_url))?;

    match request.method.as_str() {
        "GET" => easy.get(true).context("failed to configure GET request")?,
        "HEAD" => easy
            .nobody(true)
            .context("failed to configure HEAD request")?,
        "POST" => {
            easy.post(true)
                .context("failed to configure POST request")?;
            let body_bytes = request.body.as_deref().unwrap_or(&[]);
            easy.post_fields_copy(body_bytes)
                .context("failed to set POST body")?;
        }
        method => {
            easy.custom_request(method)
                .with_context(|| anyhow!("failed to configure {method} request"))?;
            if let Some(ref body) = request.body {
                easy.post_fields_copy(body)
                    .context("failed to set custom request body")?;
            }
        }
    }

    if let Some(proxy) = config.http_proxy() {
        easy.proxy(proxy)
            .with_context(|| anyhow!("failed to configure HTTP proxy `{proxy}`"))?;
    }
    if let Some(no_proxy) = config.http_no_proxy() {
        easy.noproxy(no_proxy)
            .with_context(|| anyhow!("failed to configure HTTP no_proxy `{no_proxy}`"))?;
    }
    if !config.http_host_resolve().is_empty() {
        let mut resolve = List::new();
        for entry in normalized_http_host_resolve_entries(config.http_host_resolve())? {
            resolve
                .append(&entry)
                .with_context(|| anyhow!("failed to build curl host resolve entry `{entry}`"))?;
        }
        easy.resolve(resolve)
            .context("failed to configure curl host resolve overrides")?;
    }

    let mut headers = List::new();
    let mut outgoing_headers = outgoing_request_headers_for_url(
        config,
        request,
        request_url,
        redirect_chain,
        cookie_header,
    );
    if let Some(web_bot_auth) = config.web_bot_auth() {
        web_bot_auth
            .append_request_headers(&mut outgoing_headers, &request.method, request_url)
            .with_context(|| anyhow!("failed to sign web bot auth request for {request_url}"))?;
    }
    let mut has_headers = false;

    let mut has_content_type_header = false;
    for (name, value) in &outgoing_headers {
        has_content_type_header |= name.eq_ignore_ascii_case("content-type");
        let header_line = if value.is_empty() {
            format!("{name}:")
        } else {
            format!("{name}: {value}")
        };
        headers
            .append(&header_line)
            .context("failed to build request header")?;
        has_headers = true;
    }
    if let Some(validation_headers) = validation_headers {
        for (name, value) in validation_headers {
            has_content_type_header |= name.eq_ignore_ascii_case("content-type");
            headers
                .append(&format!("{name}: {value}"))
                .context("failed to build cache validation request header")?;
            has_headers = true;
        }
    }
    if request.method.eq_ignore_ascii_case("POST") && !has_content_type_header {
        // libcurl otherwise synthesizes `Content-Type: application/x-www-form-urlencoded`
        // for POST bodies. Browser fetch/sendBeacon only send Content-Type when
        // BodyInit or caller headers produce one, so suppress curl's transport default.
        headers
            .append("Content-Type:")
            .context("failed to suppress curl default POST content-type")?;
        has_headers = true;
    }

    if has_headers {
        easy.http_headers(headers)
            .context("failed to attach curl request headers")?;
    }

    if let Some(auth) = request.auth()
        && request.auth_requires_buffered_transport()
    {
        let mut methods = Auth::new();
        match auth.scheme {
            RequestAuthScheme::Basic => {
                methods.basic(true);
            }
            RequestAuthScheme::Digest => {
                methods.digest(true);
            }
            RequestAuthScheme::Negotiate => {
                methods.gssnegotiate(true);
            }
            RequestAuthScheme::Ntlm => {
                methods.ntlm(true);
            }
        }
        match auth.target {
            RequestAuthTarget::Server => {
                easy.username(&auth.username)
                    .context("failed to set server auth username")?;
                easy.password(&auth.password)
                    .context("failed to set server auth password")?;
                easy.http_auth(&methods)
                    .context("failed to configure server auth scheme")?;
            }
            RequestAuthTarget::Proxy => {
                easy.proxy_username(&auth.username)
                    .context("failed to set proxy auth username")?;
                easy.proxy_password(&auth.password)
                    .context("failed to set proxy auth password")?;
                easy.proxy_auth(&methods)
                    .context("failed to configure proxy auth scheme")?;
            }
            RequestAuthTarget::ProxyHeader => {}
        }
    }

    Ok(outgoing_headers)
}

pub(crate) fn configure_openssl_tls_context(
    ssl_ctx: *mut c_void,
) -> std::result::Result<(), curl::Error> {
    // WPT's Python TLS server and some real servers close TLS sockets without a
    // close_notify alert. Browsers tolerate that EOF, while OpenSSL 3 reports it
    // as SSL_ERROR_SSL unless this compatibility option is set.
    unsafe {
        openssl_sys::SSL_CTX_set_options(
            ssl_ctx.cast::<openssl_sys::SSL_CTX>(),
            openssl_sys::SSL_OP_IGNORE_UNEXPECTED_EOF,
        );
    }
    Ok(())
}

fn enforce_request_target_policy(config: &FetchConfig, request_url: &Url) -> Result<()> {
    if crate::should_request_be_blocked_due_to_bad_port(request_url) {
        bail!("blocked bad port for `{request_url}`");
    }

    if !config.block_private_networks() && config.block_cidrs().is_empty() {
        return Ok(());
    }

    let Some(host) = request_url.host_str() else {
        return Ok(());
    };
    let port = request_url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("could not determine port for request url `{request_url}`"))?;

    let resolved_ips = resolve_target_ips(config, host, port)
        .with_context(|| anyhow!("failed to resolve request host `{host}` for `{request_url}`"))?;
    for ip in resolved_ips {
        if config.block_private_networks() && is_private_or_internal_ip(ip) {
            bail!("blocked private network address `{ip}` for `{request_url}`");
        }
        if let Some(cidr) = config.block_cidrs().iter().find(|cidr| cidr.contains(&ip)) {
            bail!("blocked address `{ip}` for `{request_url}` because it matches `{cidr}`");
        }
    }

    Ok(())
}

fn resolve_target_ips(config: &FetchConfig, host: &str, port: u16) -> Result<Vec<IpAddr>> {
    if let Some(resolved_ips) =
        resolve_host_resolve_override_ips(config.http_host_resolve(), host, port)?
    {
        return Ok(resolved_ips);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }

    resolve_system_target_ips(host, port)
}

fn resolve_host_resolve_override_ips(
    entries: &[String],
    host: &str,
    port: u16,
) -> Result<Option<Vec<IpAddr>>> {
    let mut exact_match: Option<Vec<IpAddr>> = None;
    let mut wildcard_match: Option<Vec<IpAddr>> = None;
    for entry in entries {
        match parse_http_host_resolve_entry(entry)? {
            HttpHostResolveEntry::Add {
                host: entry_host,
                port: entry_port,
                addresses,
                ..
            } if entry_port == port => {
                if entry_host == "*" {
                    wildcard_match = Some(addresses);
                } else if entry_host.eq_ignore_ascii_case(host) {
                    exact_match = Some(addresses);
                }
            }
            HttpHostResolveEntry::Remove {
                host: entry_host,
                port: entry_port,
            } if entry_port == port => {
                if entry_host == "*" {
                    wildcard_match = None;
                } else if entry_host.eq_ignore_ascii_case(host) {
                    exact_match = None;
                }
            }
            _ => {}
        }
    }

    Ok(exact_match.or(wildcard_match))
}

fn resolve_system_target_ips(host: &str, port: u16) -> Result<Vec<IpAddr>> {
    let mut resolved = Vec::new();
    for addr in (host, port).to_socket_addrs()? {
        let ip = addr.ip();
        if !resolved.contains(&ip) {
            resolved.push(ip);
        }
    }
    if resolved.is_empty() {
        bail!("no addresses resolved");
    }
    Ok(resolved)
}

enum HttpHostResolveEntry {
    Add {
        plus_prefix: bool,
        host: String,
        port: u16,
        addresses: Vec<IpAddr>,
    },
    Remove {
        host: String,
        port: u16,
    },
}

impl HttpHostResolveEntry {
    fn curl_entry(&self) -> String {
        match self {
            Self::Add {
                plus_prefix,
                host,
                port,
                addresses,
            } => {
                let prefix = if *plus_prefix { "+" } else { "" };
                let addresses = addresses
                    .iter()
                    .map(format_http_host_resolve_ip)
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{prefix}{}:{port}:{addresses}",
                    format_http_host_resolve_host(host)
                )
            }
            Self::Remove { host, port } => {
                format!("-{}:{port}", format_http_host_resolve_host(host))
            }
        }
    }
}

fn normalized_http_host_resolve_entries(entries: &[String]) -> Result<Vec<String>> {
    entries
        .iter()
        .map(|entry| parse_http_host_resolve_entry(entry).map(|entry| entry.curl_entry()))
        .collect()
}

fn parse_http_host_resolve_entry(entry: &str) -> Result<HttpHostResolveEntry> {
    let entry = entry.trim();
    if let Some(entry) = entry.strip_prefix('-') {
        let (host, port) = split_http_host_resolve_remove_entry(entry)?;
        return Ok(HttpHostResolveEntry::Remove {
            host: normalize_http_host_resolve_host(host),
            port: parse_http_host_resolve_port(port, entry)?,
        });
    }

    let (plus_prefix, entry) = if let Some(entry) = entry.strip_prefix('+') {
        (true, entry)
    } else {
        (false, entry)
    };
    let (host, port, address) = split_http_host_resolve_add_entry(entry)?;
    let port = parse_http_host_resolve_port(port, entry)?;
    let mut addresses = Vec::new();
    for address in address.split(',').map(str::trim) {
        if address.is_empty() {
            bail!("--http-host-resolve entries must not contain empty addresses");
        }
        let address = address.trim_matches(['[', ']']);
        let address = address
            .parse::<IpAddr>()
            .with_context(|| format!("invalid --http-host-resolve address in `{entry}`"))?;
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }

    Ok(HttpHostResolveEntry::Add {
        plus_prefix,
        host: normalize_http_host_resolve_host(host),
        port,
        addresses,
    })
}

fn split_http_host_resolve_add_entry(entry: &str) -> Result<(&str, &str, &str)> {
    if let Some(entry) = entry.strip_prefix('[') {
        let Some(host_end) = entry.find(']') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        let host = &entry[..host_end];
        let remainder = &entry[host_end + 1..];
        let Some(remainder) = remainder.strip_prefix(':') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        let Some((port, address)) = remainder.split_once(':') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        if host.trim().is_empty() || port.trim().is_empty() || address.trim().is_empty() {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        }
        return Ok((host.trim(), port.trim(), address.trim()));
    }

    let mut parts = entry.splitn(3, ':');
    let host = parts.next().unwrap_or_default().trim();
    let port = parts.next().unwrap_or_default().trim();
    let address = parts.next().unwrap_or_default().trim();

    if host.is_empty() || port.is_empty() || address.is_empty() {
        bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
    }

    Ok((host, port, address))
}

fn split_http_host_resolve_remove_entry(entry: &str) -> Result<(&str, &str)> {
    if let Some(entry) = entry.strip_prefix('[') {
        let Some(host_end) = entry.find(']') else {
            bail!("--http-host-resolve removal must be in HOST:PORT form");
        };
        let host = &entry[..host_end];
        let remainder = &entry[host_end + 1..];
        let Some(port) = remainder.strip_prefix(':') else {
            bail!("--http-host-resolve removal must be in HOST:PORT form");
        };
        if host.trim().is_empty() || port.trim().is_empty() {
            bail!("--http-host-resolve removal must be in HOST:PORT form");
        }
        return Ok((host.trim(), port.trim()));
    }

    let Some((host, port)) = entry.split_once(':') else {
        bail!("--http-host-resolve removal must be in HOST:PORT form");
    };
    if host.trim().is_empty() || port.trim().is_empty() {
        bail!("--http-host-resolve removal must be in HOST:PORT form");
    }
    Ok((host.trim(), port.trim()))
}

fn parse_http_host_resolve_port(port: &str, entry: &str) -> Result<u16> {
    port.parse::<u16>()
        .with_context(|| format!("invalid --http-host-resolve port in `{entry}`"))
}

fn normalize_http_host_resolve_host(host: &str) -> String {
    host.trim_matches(['[', ']']).to_owned()
}

fn format_http_host_resolve_host(host: &str) -> String {
    if host != "*" && host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn format_http_host_resolve_ip(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

fn is_private_or_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_private()
                || ipv4.is_loopback()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
                || ipv4.is_unspecified()
                || ipv4.is_multicast()
                || matches!(ipv4.octets(), [100, second, ..] if (64..=127).contains(&second))
                || matches!(ipv4.octets(), [198, 18 | 19, ..])
                || matches!(ipv4.octets(), [240..=255, ..])
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_multicast()
                || ipv6.is_unicast_link_local()
                || ipv6.is_unique_local()
                || (ipv6.segments()[0] == 0x2001 && ipv6.segments()[1] == 0x0db8)
        }
    }
}

fn configure_curl_http_protocol_allowlist<H: Handler>(easy: &mut Easy2<H>) -> Result<()> {
    for (option, description) in [
        (CURL_OPTION_PROTOCOLS_STR, "request protocols"),
        (CURL_OPTION_REDIR_PROTOCOLS_STR, "redirect protocols"),
    ] {
        let setopt_result = unsafe {
            curl_sys::curl_easy_setopt(
                easy.raw(),
                option,
                CURL_HTTP_PROTOCOL_ALLOWLIST.as_ptr().cast::<c_char>(),
            )
        };
        if setopt_result != curl_sys::CURLE_OK {
            return Err(curl::Error::new(setopt_result))
                .with_context(|| format!("failed to restrict curl {description} to HTTP(S)"));
        }
    }
    Ok(())
}

fn encode_basic_auth(username: &str, password: &str) -> String {
    const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = format!("{username}:{password}");
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let word = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(BASE64[((word >> 18) & 0x3f) as usize] as char);
        out.push(BASE64[((word >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64[((word >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64[(word & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn effective_connect_timeout(config: &FetchConfig, request: &Request) -> Option<Duration> {
    config
        .http_connect_timeout_ms()
        .map(Duration::from_millis)
        .or_else(|| {
            (!request.is_top_level_navigation_request())
                .then_some(DEFAULT_BROWSER_SUBRESOURCE_CONNECT_TIMEOUT)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestMode;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("valid URL")
    }

    fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
        headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }

    struct ProtocolAllowlistHandler;

    impl Handler for ProtocolAllowlistHandler {}

    #[test]
    fn curl_string_protocol_allowlist_rejects_file_transfer() {
        let mut easy = Easy2::new(ProtocolAllowlistHandler);
        configure_curl_http_protocol_allowlist(&mut easy)
            .expect("libcurl should accept the string protocol allowlist options");
        easy.url("file:///moli-policy-must-not-open")
            .expect("file URL should parse as a curl URL");

        let error = easy
            .perform()
            .expect_err("the curl backstop must reject a file transfer");
        assert_eq!(error.code(), curl_sys::CURLE_UNSUPPORTED_PROTOCOL);
    }

    fn redirect(from_url: &Url, to_url: &Url) -> RedirectInfo {
        RedirectInfo {
            from_url: from_url.clone(),
            to_url: to_url.clone(),
            status: 302,
            headers: Vec::new(),
            network_extra_info_available: true,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: true,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn default_connect_timeout_only_bounds_subresources() {
        let config = FetchConfig::default();
        let navigation = Request::get("https://example.test/").unwrap();
        let subresource =
            Request::new("GET", "https://example.test/script.js", None, Vec::new()).unwrap();

        assert_eq!(effective_connect_timeout(&config, &navigation), None);
        assert_eq!(
            effective_connect_timeout(&config, &subresource),
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn explicit_connect_timeout_overrides_navigation_and_subresource_defaults() {
        let mut config = FetchConfig::default();
        config.set_connect_timeout_ms(Some(1_234));
        let navigation = Request::get("https://example.test/").unwrap();
        let subresource =
            Request::new("GET", "https://example.test/script.js", None, Vec::new()).unwrap();

        assert_eq!(
            effective_connect_timeout(&config, &navigation),
            Some(Duration::from_millis(1_234))
        );
        assert_eq!(
            effective_connect_timeout(&config, &subresource),
            Some(Duration::from_millis(1_234))
        );
    }

    #[test]
    fn browser_subresource_origin_header_is_method_mode_and_origin_aware() {
        struct Case {
            name: &'static str,
            method: &'static str,
            mode: RequestMode,
            initiator: Option<&'static str>,
            request_url: &'static str,
            expected: Option<&'static str>,
        }

        let cases = [
            Case {
                name: "same-origin CORS GET",
                method: "GET",
                mode: RequestMode::Cors,
                initiator: Some("https://app.test/page"),
                request_url: "https://app.test/data",
                expected: None,
            },
            Case {
                name: "same-origin CORS HEAD",
                method: "HEAD",
                mode: RequestMode::Cors,
                initiator: Some("https://app.test/page"),
                request_url: "https://app.test/data",
                expected: None,
            },
            Case {
                name: "same-origin CORS POST",
                method: "POST",
                mode: RequestMode::Cors,
                initiator: Some("https://app.test/page"),
                request_url: "https://app.test/data",
                expected: Some("https://app.test"),
            },
            Case {
                name: "cross-origin CORS GET",
                method: "GET",
                mode: RequestMode::Cors,
                initiator: Some("https://app.test/page"),
                request_url: "https://api.test/data",
                expected: Some("https://app.test"),
            },
            Case {
                name: "cross-origin CORS HEAD",
                method: "HEAD",
                mode: RequestMode::Cors,
                initiator: Some("https://app.test/page"),
                request_url: "https://api.test/data",
                expected: Some("https://app.test"),
            },
            Case {
                name: "cross-origin no-CORS GET",
                method: "GET",
                mode: RequestMode::NoCors,
                initiator: Some("https://app.test/page"),
                request_url: "https://api.test/data",
                expected: None,
            },
            Case {
                name: "same-origin no-CORS POST",
                method: "POST",
                mode: RequestMode::NoCors,
                initiator: Some("https://app.test/page"),
                request_url: "https://app.test/beacon",
                expected: Some("https://app.test"),
            },
            Case {
                name: "opaque initiator POST",
                method: "POST",
                mode: RequestMode::Cors,
                initiator: Some("data:text/html,opaque"),
                request_url: "https://api.test/data",
                expected: Some("null"),
            },
            Case {
                name: "missing initiator POST",
                method: "POST",
                mode: RequestMode::Cors,
                initiator: None,
                request_url: "https://api.test/data",
                expected: Some("null"),
            },
        ];

        let config = FetchConfig::default();
        for case in cases {
            let request_url = url(case.request_url);
            let mut request = Request::new(case.method, case.request_url, None, Vec::new())
                .unwrap()
                .with_request_mode(case.mode)
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
            if let Some(initiator) = case.initiator {
                request = request.with_initiator_url(&url(initiator));
            }

            let headers = outgoing_request_headers_for_url(
                &config,
                &request,
                &request_url,
                &Vec::new(),
                None,
            );
            assert_eq!(
                header_value(&headers, "origin").as_deref(),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn explicit_origin_header_is_preserved() {
        let config = FetchConfig::default();
        let request_url = url("https://app.test/data");
        let request = Request::new(
            "POST",
            request_url.as_str(),
            None,
            vec![("oRiGiN".to_owned(), "https://explicit.test".to_owned())],
        )
        .unwrap()
        .with_initiator_url(&url("https://app.test/page"))
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);
        assert_eq!(
            headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("origin"))
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["https://explicit.test"]
        );
    }

    #[test]
    fn post_navigation_without_an_initiator_serializes_opaque_origin() {
        let config = FetchConfig::default();
        let request_url = url("https://app.test/submit");
        let request = Request::new("POST", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_top_level_navigation_cookie_context();

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);
        assert_eq!(header_value(&headers, "origin").as_deref(), Some("null"));
    }

    #[test]
    fn browser_subresource_origin_uses_redirect_target_url() {
        let config = FetchConfig::default();
        let initiator_url = url("http://app.test/page");
        let original_url = url("http://app.test/redirect");
        let redirected_url = url("http://api.test/data");
        let request = Request::new("GET", original_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&initiator_url)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let redirect_chain = vec![redirect(&original_url, &redirected_url)];

        let headers = outgoing_request_headers_for_url(
            &config,
            &request,
            &redirected_url,
            &redirect_chain,
            None,
        );

        assert_eq!(
            header_value(&headers, "origin"),
            Some("http://app.test".to_owned())
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("cross-site".to_owned())
        );
    }

    #[test]
    fn browser_subresource_origin_uses_blob_initiator_tuple_origin() {
        let config = FetchConfig::default();
        let initiator_url = url("blob:https://flights.ctrip.com/1");
        let request_url = url("https://m.ctrip.com/restapi/soa2/20589/SaveTraceInfo");
        let request = Request::new(
            "POST",
            request_url.as_str(),
            Some(String::new()),
            Vec::new(),
        )
        .unwrap()
        .with_initiator_url(&initiator_url)
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(
            header_value(&headers, "origin"),
            Some("https://flights.ctrip.com".to_owned())
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("same-site".to_owned())
        );
    }

    #[test]
    fn browser_subresource_origin_serializes_null_after_cross_site_redirect() {
        let config = FetchConfig::default();
        let initiator_url = url("http://app.test/page");
        let original_url = url("http://api.test/redirect");
        let redirected_url = url("http://app.test/data");
        let request = Request::new("GET", original_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&initiator_url)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let redirect_chain = vec![redirect(&original_url, &redirected_url)];

        let headers = outgoing_request_headers_for_url(
            &config,
            &request,
            &redirected_url,
            &redirect_chain,
            None,
        );

        assert_eq!(header_value(&headers, "origin"), Some("null".to_owned()));
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("same-origin".to_owned())
        );
    }

    #[test]
    fn browser_subresource_origin_serializes_null_after_same_site_redirect_chain() {
        let config = FetchConfig::default();
        let initiator_url = url("http://app.test:8000/page");
        let original_url = url("http://app.test:8000/redirect");
        let intermediate_url = url("http://app.test:8001/redirect");
        let final_url = url("http://app.test:8000/data");
        let request = Request::new("GET", original_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&initiator_url)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let redirect_chain = vec![
            redirect(&original_url, &intermediate_url),
            redirect(&intermediate_url, &final_url),
        ];

        let headers =
            outgoing_request_headers_for_url(&config, &request, &final_url, &redirect_chain, None);

        assert_eq!(header_value(&headers, "origin"), Some("null".to_owned()));
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("same-origin".to_owned())
        );
    }

    #[test]
    fn browser_subresource_origin_serializes_null_after_cross_site_redirect_chain_returns_to_origin()
     {
        let config = FetchConfig::default();
        let initiator_url = url("http://app.test/page");
        let original_url = url("http://app.test/redirect");
        let cross_site_url = url("http://cross.test/redirect");
        let final_url = url("http://app.test/data");
        let request = Request::new("GET", original_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&initiator_url)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let redirect_chain = vec![
            redirect(&original_url, &cross_site_url),
            redirect(&cross_site_url, &final_url),
        ];

        let headers =
            outgoing_request_headers_for_url(&config, &request, &final_url, &redirect_chain, None);

        assert_eq!(header_value(&headers, "origin"), Some("null".to_owned()));
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("same-origin".to_owned())
        );
    }

    #[test]
    fn browser_subresource_origin_serializes_null_after_cross_site_redirect_chain_stays_cross_site()
    {
        let config = FetchConfig::default();
        let initiator_url = url("http://app.test/page");
        let original_url = url("http://app.test/redirect");
        let cross_site_redirect_url = url("http://cross.test/redirect");
        let final_url = url("http://api.test/data");
        let request = Request::new("GET", original_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&initiator_url)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let redirect_chain = vec![
            redirect(&original_url, &cross_site_redirect_url),
            redirect(&cross_site_redirect_url, &final_url),
        ];

        let headers =
            outgoing_request_headers_for_url(&config, &request, &final_url, &redirect_chain, None);

        assert_eq!(header_value(&headers, "origin"), Some("null".to_owned()));
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("cross-site".to_owned())
        );
    }

    #[test]
    fn browser_subresource_sec_fetch_mode_uses_request_mode() {
        let config = FetchConfig::default();
        let request_url = url("https://app.test/assets/script.js");
        let request = Request::new("GET", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_request_mode(RequestMode::NoCors)
            .with_initiator_url(&url("https://app.test/page"))
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(
            header_value(&headers, "sec-fetch-mode"),
            Some("no-cors".to_owned())
        );
    }

    #[test]
    fn subframe_navigation_uses_document_headers_without_top_level_activation() {
        let config = FetchConfig::default();
        let request_url = url("https://frame.test/challenge");
        let request = Request::new("GET", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&url("https://page.test/"))
            .with_subframe_navigation_cookie_context();

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(
            header_value(&headers, "sec-fetch-mode").as_deref(),
            Some("navigate")
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-dest").as_deref(),
            Some("iframe")
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-site").as_deref(),
            Some("cross-site")
        );
        assert_eq!(header_value(&headers, "sec-fetch-user"), None);
        assert_eq!(
            header_value(&headers, "sec-fetch-storage-access").as_deref(),
            Some("active")
        );
        assert!(header_value(&headers, "accept").is_some());
        assert!(header_value(&headers, "sec-ch-ua").is_some());
    }

    #[test]
    fn http_host_resolve_entries_are_normalized_before_curl_configuration() {
        let entries = normalized_http_host_resolve_entries(&[
            " localhost:80: 1.1.1.1 , [2001:db8::1] ".to_owned(),
            " - localhost:80 ".to_owned(),
        ])
        .unwrap();

        assert_eq!(
            entries,
            vec![
                "localhost:80:1.1.1.1,[2001:db8::1]".to_owned(),
                "-localhost:80".to_owned()
            ]
        );
    }

    #[test]
    fn third_party_credentialed_request_reports_active_storage_access() {
        let config = FetchConfig::default();
        let request_url = url("https://frame.test/challenge-response");
        let request = Request::new("POST", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&url("https://frame.test/challenge"))
            .with_site_for_cookies_url(&url("https://page.test/"))
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
            .with_credentials_mode(crate::RequestCredentialsMode::Include)
            .with_request_mode(crate::RequestMode::Cors);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(
            header_value(&headers, "sec-fetch-site").as_deref(),
            Some("same-origin")
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-storage-access").as_deref(),
            Some("active")
        );
    }

    #[test]
    fn third_party_same_origin_credentials_mode_omits_storage_access() {
        let config = FetchConfig::default();
        let request_url = url("https://frame.test/challenge-response");
        let request = Request::new("POST", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&url("https://frame.test/challenge"))
            .with_site_for_cookies_url(&url("https://page.test/"))
            .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
            .with_credentials_mode(crate::RequestCredentialsMode::SameOrigin)
            .with_request_mode(crate::RequestMode::Cors);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(
            header_value(&headers, "sec-fetch-site").as_deref(),
            Some("same-origin")
        );
        assert_eq!(header_value(&headers, "sec-fetch-storage-access"), None);
    }

    #[test]
    fn untrustworthy_request_omits_storage_access() {
        let config = FetchConfig::default();
        let request_url = url("http://frame.test/challenge-response");
        let request = Request::new("POST", request_url.as_str(), None, Vec::new())
            .unwrap()
            .with_initiator_url(&url("http://frame.test/challenge"))
            .with_site_for_cookies_url(&url("http://page.test/"))
            .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
            .with_credentials_mode(crate::RequestCredentialsMode::Include);

        let headers =
            outgoing_request_headers_for_url(&config, &request, &request_url, &Vec::new(), None);

        assert_eq!(header_value(&headers, "sec-fetch-storage-access"), None);
    }
}

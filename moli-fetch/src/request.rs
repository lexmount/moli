use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use moli_cookie_jar::{
    BrowserCookieFacadeContext, NetworkCookieRequestContext, NetworkStorageAccessStatus,
};
#[cfg(any(test, feature = "test-support"))]
use moli_cookie_jar::{
    NetworkSiteContextMetadata, NetworkSiteContextTrackMetadata, redirect_types_for_request,
    site_context_downgrade_type,
};
use moli_url::same_origin;
use url::Url;

use crate::{FetchConfig, network_fetch_result::NetworkObservationRecorder};

#[derive(Debug, Clone)]
pub struct Request {
    pub url: Url,
    pub method: String,
    pub body: Option<Vec<u8>>,
    pub request_headers: Vec<(String, String)>,
    cache_mode: RequestCacheMode,
    pub resource_type: RequestResourceType,
    subresource_request_metadata: Option<SubresourceRequestMetadata>,
    script_scheduler_priority: Option<ScriptFetchSchedulerPriority>,
    pub priority_hints: RequestPriorityHints,
    pub browser_request_metadata: Option<BrowserRequestMetadata>,
    browser_navigation_kind: BrowserNavigationRequestKind,
    infer_referrer_from_initiator: bool,
    pub use_page_network_policy: bool,
    pub follow_redirects: bool,
    pub request_mode: RequestMode,
    pub redirect_mode: RequestRedirectMode,
    pub credentials_mode: RequestCredentialsMode,
    network_partition_key: Option<String>,
    auth: Option<RequestAuth>,
    pub cookie_context: NetworkCookieRequestContext,
    timeout_policy: RequestTimeoutPolicy,
    network_observation_recorder: Option<NetworkObservationRecorder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRequestMetadata {
    Audio,
    AudioWorklet,
    Beacon,
    EventSource,
    Fetch,
    Font,
    Image,
    JsonModule,
    Manifest,
    Ping,
    Style,
    StyleModule,
    TextTrack,
    Video,
    Xhr,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BrowserNavigationRequestKind {
    #[default]
    Navigate,
    Reload,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RequestCacheMode {
    #[default]
    Default,
    Validate,
    Bypass,
    NoStore,
}

impl RequestCacheMode {
    pub fn requires_http_cache_validation(self) -> bool {
        matches!(self, Self::Validate)
    }

    pub fn allows_memory_cache_lookup(self) -> bool {
        matches!(self, Self::Default)
    }

    pub fn allows_memory_cache_store(self) -> bool {
        !matches!(self, Self::NoStore)
    }

    pub fn allows_http_cache(self) -> bool {
        !matches!(self, Self::NoStore)
    }

    pub fn allows_http_cache_lookup(self) -> bool {
        !matches!(self, Self::Bypass | Self::NoStore)
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum FetchPriorityHint {
    Low,
    #[default]
    Auto,
    High,
}

impl FetchPriorityHint {
    pub fn from_attribute(value: Option<&str>) -> Option<Self> {
        // HTML `fetchpriority` attributes are ASCII case-insensitive, while
        // RequestInit.priority uses the strum-derived, case-sensitive WebIDL
        // enum parser below.
        let value = value?.trim();
        if value.eq_ignore_ascii_case("low") {
            Some(Self::Low)
        } else if value.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else if value.eq_ignore_ascii_case("high") {
            Some(Self::High)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestPriorityHints {
    /// Author-provided `fetchpriority` / RequestInit priority hint.
    ///
    /// Chromium carries this on ResourceRequest and applies it after the base
    /// resource-type priority is computed, so it is intentionally not nested
    /// under script-specific metadata.
    pub fetch_priority: Option<FetchPriorityHint>,
    /// True for requests initiated by `<link rel=preload>`.
    ///
    /// Chromium keeps this as an initiator flag separate from resource type;
    /// font preloads use it to avoid outranking parser-blocking scripts and
    /// critical CSS.
    pub link_preload: bool,
    /// True when the request originates inside a child frame.
    ///
    /// Chromium's optional subframe deprioritization lowers high-priority
    /// subframe resources to low and delayable subframe resources to lowest.
    /// Moli models that as a request flag so frame context stays separate
    /// from resource type and author `fetchpriority`.
    pub subframe_context: bool,
    /// True for parser/preload-scanner discovered in-document images that
    /// qualify for Chromium's first-N non-small image priority boost.
    ///
    /// This is deliberately narrower than "important image" or "visible image":
    /// Chromium has separate layout/LCP observer paths that can later boost
    /// visible or predicted-LCP images. This flag only represents the
    /// ResourceFetcher first-N rule for in-document images, before Moli
    /// has any useful layout visibility signal.
    pub in_document_image_priority_boost: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceLoadPriority {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

impl ResourceLoadPriority {
    pub fn scheduler_rank(self) -> u8 {
        match self {
            Self::VeryLow => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::VeryHigh => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RequestResourceType {
    CssStyleSheet,
    Font,
    Raw,
    #[default]
    Script,
    ParserBlockingScript,
    ClassicAsyncOrDeferScript,
    LatePreloadScript,
    LatePreloadCssStyleSheet,
    Manifest,
    Image,
    TextTrack,
    Media,
    SvgDocument,
    Beacon,
    Ping,
    CspReport,
    LinkPrefetch,
    SpeculationRules,
    Dictionary,
}

impl RequestResourceType {
    pub fn default_load_priority(self) -> ResourceLoadPriority {
        match self {
            Self::CssStyleSheet | Self::Font | Self::ParserBlockingScript => {
                ResourceLoadPriority::VeryHigh
            }
            Self::Raw | Self::Script => ResourceLoadPriority::High,
            Self::Manifest | Self::LatePreloadScript | Self::LatePreloadCssStyleSheet => {
                ResourceLoadPriority::Medium
            }
            Self::Image
            | Self::TextTrack
            | Self::Media
            | Self::SvgDocument
            | Self::ClassicAsyncOrDeferScript => ResourceLoadPriority::Low,
            Self::Beacon
            | Self::Ping
            | Self::CspReport
            | Self::LinkPrefetch
            | Self::SpeculationRules
            | Self::Dictionary => ResourceLoadPriority::VeryLow,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestCredentialsMode {
    Include,
    SameOrigin,
    Omit,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestMode {
    Navigate,
    SameOrigin,
    NoCors,
    Cors,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::AsRefStr, strum::EnumString, strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestRedirectMode {
    Follow,
    Error,
    Manual,
}

impl RequestRedirectMode {
    pub fn follows_redirects(self) -> bool {
        matches!(self, Self::Follow)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptFetchRequestMetadata {
    pub cross_origin: Option<String>,
    pub referrer_policy: Option<String>,
    pub document_referrer_policy: Option<String>,
    pub charset: Option<String>,
    pub integrity: Option<String>,
    pub nonce: Option<String>,
    pub fetch_priority: Option<FetchPriorityHint>,
    pub scheduler_priority: Option<ScriptFetchSchedulerPriority>,
}

/// Request properties consumed below the renderer's resource owner.
///
/// Producer-only inputs such as `crossorigin`, CSP nonces, and decoding
/// charsets stay with the renderer. Fetch priority and script scheduling also
/// have dedicated request fields instead of duplicating them here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubresourceRequestMetadata {
    pub referrer_policy: Option<String>,
    pub document_referrer_policy: Option<String>,
    pub integrity: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RequestTimeoutPolicy {
    minimum_request_timeout: Option<Duration>,
    disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptFetchSchedulerPriority {
    Low,
    Auto,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAuthTarget {
    Server,
    Proxy,
    ProxyHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAuthScheme {
    Basic,
    Digest,
    Negotiate,
    Ntlm,
}

#[derive(Debug, Clone)]
pub struct RequestAuth {
    pub target: RequestAuthTarget,
    pub scheme: RequestAuthScheme,
    pub username: String,
    pub password: String,
}

impl RequestAuth {
    fn can_use_header_transport(&self) -> bool {
        matches!(
            (self.target, self.scheme),
            (RequestAuthTarget::Server, RequestAuthScheme::Basic)
                | (RequestAuthTarget::ProxyHeader, RequestAuthScheme::Basic)
        )
    }
}

impl Request {
    pub fn get(raw_url: &str) -> Result<Self> {
        let url = Url::parse(raw_url)
            .with_context(|| anyhow!("failed to parse request url `{raw_url}`"))?;
        Ok(Self {
            url,
            method: "GET".to_owned(),
            body: None,
            request_headers: vec![],
            cache_mode: RequestCacheMode::Default,
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            use_page_network_policy: false,
            follow_redirects: true,
            request_mode: RequestMode::Navigate,
            redirect_mode: RequestRedirectMode::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::top_level_navigation("GET"),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
        })
    }

    pub fn get_with_url(url: Url) -> Self {
        Self {
            url,
            method: "GET".to_owned(),
            body: None,
            request_headers: vec![],
            cache_mode: RequestCacheMode::Default,
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            use_page_network_policy: false,
            follow_redirects: true,
            request_mode: RequestMode::Navigate,
            redirect_mode: RequestRedirectMode::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::top_level_navigation("GET"),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
        }
    }

    pub fn new(
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
    ) -> Result<Self> {
        Self::new_bytes(
            method,
            raw_url,
            body.map(String::into_bytes),
            request_headers,
        )
    }

    pub fn new_bytes(
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
    ) -> Result<Self> {
        let url = Url::parse(raw_url)
            .with_context(|| anyhow!("failed to parse request url `{raw_url}`"))?;
        Ok(Self {
            url,
            method: method.to_owned(),
            body,
            request_headers,
            cache_mode: RequestCacheMode::Default,
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            use_page_network_policy: false,
            follow_redirects: true,
            request_mode: RequestMode::Cors,
            redirect_mode: RequestRedirectMode::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::subresource(method),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
        })
    }

    pub fn with_min_request_timeout(mut self, minimum_request_timeout: Duration) -> Self {
        self.timeout_policy.minimum_request_timeout = Some(minimum_request_timeout);
        self
    }

    pub fn without_request_timeout(mut self) -> Self {
        self.timeout_policy.disabled = true;
        self
    }

    pub(crate) fn effective_request_timeout(&self, config: &FetchConfig) -> Duration {
        if self.timeout_policy.disabled {
            return Duration::ZERO;
        }
        let configured = Duration::from_millis(config.request_timeout_ms());
        if configured.is_zero() {
            return configured;
        }
        match self.timeout_policy.minimum_request_timeout {
            Some(minimum) => configured.max(minimum),
            None => configured,
        }
    }

    pub fn with_script_fetch_metadata(mut self, metadata: ScriptFetchRequestMetadata) -> Self {
        if self.resource_type == RequestResourceType::Raw {
            self.resource_type = RequestResourceType::Script;
        }
        if metadata.fetch_priority.is_some() {
            self.priority_hints.fetch_priority = metadata.fetch_priority;
        }
        self.script_scheduler_priority = metadata.scheduler_priority;
        self.subresource_request_metadata = Some(SubresourceRequestMetadata {
            referrer_policy: metadata.referrer_policy,
            document_referrer_policy: metadata.document_referrer_policy,
            integrity: metadata.integrity,
        });
        self
    }

    pub fn with_subresource_request_metadata(
        mut self,
        metadata: SubresourceRequestMetadata,
    ) -> Self {
        self.subresource_request_metadata = Some(metadata);
        self
    }

    pub fn with_resource_type(mut self, resource_type: RequestResourceType) -> Self {
        self.resource_type = resource_type;
        self
    }

    pub fn with_cache_mode(mut self, cache_mode: RequestCacheMode) -> Self {
        self.cache_mode = cache_mode;
        self
    }

    pub fn cache_mode(&self) -> RequestCacheMode {
        self.cache_mode
    }

    pub fn with_fetch_priority_hint(mut self, fetch_priority: Option<FetchPriorityHint>) -> Self {
        self.priority_hints.fetch_priority = fetch_priority;
        self
    }

    pub fn with_link_preload(mut self) -> Self {
        self.priority_hints.link_preload = true;
        self
    }

    pub fn with_subframe_context(mut self, subframe_context: bool) -> Self {
        self.priority_hints.subframe_context = subframe_context;
        self
    }

    pub fn with_in_document_image_priority_boost(mut self, boost: bool) -> Self {
        self.priority_hints.in_document_image_priority_boost = boost;
        self
    }

    pub fn subresource_request_metadata(&self) -> Option<&SubresourceRequestMetadata> {
        self.subresource_request_metadata.as_ref()
    }

    pub fn script_scheduler_priority(&self) -> Option<ScriptFetchSchedulerPriority> {
        self.script_scheduler_priority
    }

    pub fn with_browser_request_metadata(mut self, metadata: BrowserRequestMetadata) -> Self {
        self.browser_request_metadata = Some(metadata);
        self
    }

    pub fn browser_request_metadata(&self) -> Option<BrowserRequestMetadata> {
        self.browser_request_metadata
    }

    pub fn with_browser_navigation_kind(mut self, kind: BrowserNavigationRequestKind) -> Self {
        self.browser_navigation_kind = kind;
        self
    }

    pub fn browser_navigation_kind(&self) -> BrowserNavigationRequestKind {
        self.browser_navigation_kind
    }

    pub fn without_inferred_referrer(mut self) -> Self {
        self.infer_referrer_from_initiator = false;
        self
    }

    pub fn infers_referrer_from_initiator(&self) -> bool {
        self.infer_referrer_from_initiator
    }

    pub fn with_page_network_policy(mut self) -> Self {
        self.use_page_network_policy = true;
        self
    }

    pub fn uses_page_network_policy(&self) -> bool {
        self.use_page_network_policy
    }

    pub(crate) fn with_network_observation_recorder(
        mut self,
        recorder: NetworkObservationRecorder,
    ) -> Self {
        self.network_observation_recorder = Some(recorder);
        self
    }

    pub(crate) fn network_observation_recorder(&self) -> Option<&NetworkObservationRecorder> {
        self.network_observation_recorder.as_ref()
    }

    pub fn with_follow_redirects(mut self, follow_redirects: bool) -> Self {
        self.follow_redirects = follow_redirects;
        self
    }

    pub fn with_redirect_mode(mut self, redirect_mode: RequestRedirectMode) -> Self {
        self.redirect_mode = redirect_mode;
        self.follow_redirects = redirect_mode.follows_redirects();
        self
    }

    pub fn with_request_mode(mut self, request_mode: RequestMode) -> Self {
        self.request_mode = request_mode;
        self
    }

    pub fn apply_redirect_status(&mut self, status: u16) {
        if redirect_status_rewrites_to_get(status, &self.method) {
            self.method = "GET".to_owned();
            self.body = None;
            self.request_headers
                .retain(|(name, _)| !is_request_body_header_name(name));
        }
    }

    pub fn with_credentials_mode(mut self, credentials_mode: RequestCredentialsMode) -> Self {
        self.credentials_mode = credentials_mode;
        self
    }

    pub fn with_network_partition_key(mut self, key: Option<String>) -> Self {
        if let Some(serialized_key) = key.as_deref() {
            self.cookie_context.browser_context = self
                .cookie_context
                .browser_context
                .clone()
                .with_serialized_storage_key(serialized_key);
        }
        self.network_partition_key = key;
        self
    }

    pub fn network_partition_key(&self) -> Option<&str> {
        self.network_partition_key.as_deref()
    }

    pub fn allows_credentials_for_url(&self, request_url: &Url) -> bool {
        match self.credentials_mode {
            RequestCredentialsMode::Include => true,
            RequestCredentialsMode::Omit => false,
            RequestCredentialsMode::SameOrigin => self
                .cookie_context
                .initiator_url
                .as_ref()
                .is_none_or(|initiator_url| same_origin(initiator_url, request_url)),
        }
    }

    pub fn auth(&self) -> Option<&RequestAuth> {
        self.auth.as_ref()
    }

    pub fn auth_requires_buffered_transport(&self) -> bool {
        self.auth
            .as_ref()
            .is_some_and(|auth| !auth.can_use_header_transport())
    }

    pub fn preemptive_server_basic_auth_for_url(&self, request_url: &Url) -> Option<(&str, &str)> {
        let auth = self.auth.as_ref()?;
        (auth.target == RequestAuthTarget::Server
            && auth.scheme == RequestAuthScheme::Basic
            && same_origin(&self.url, request_url))
        .then_some((auth.username.as_str(), auth.password.as_str()))
    }

    pub fn set_auth(&mut self, auth: Option<RequestAuth>) {
        self.auth = auth;
    }

    pub fn with_auth(mut self, auth: RequestAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn with_cookie_context(mut self, cookie_context: NetworkCookieRequestContext) -> Self {
        self.cookie_context = cookie_context;
        self
    }

    pub fn with_browser_site_context(
        mut self,
        browser_context: BrowserCookieFacadeContext,
    ) -> Self {
        let preserve_explicit_cross_site = self.cookie_context.site_context.is_cross_site();
        self.cookie_context.browser_context = browser_context;
        self.cookie_context = self
            .cookie_context
            .recompute_site_context_for_request(&self.url);
        if preserve_explicit_cross_site {
            self.cookie_context = self.cookie_context.with_cross_site_context();
        }
        self
    }

    pub fn with_cross_site_cookie_context(mut self) -> Self {
        self.cookie_context = self.cookie_context.with_cross_site_context();
        self
    }

    pub fn with_top_level_navigation_cookie_context(mut self) -> Self {
        let browser_context = self.cookie_context.browser_context;
        self.cookie_context = NetworkCookieRequestContext::top_level_navigation(&self.method);
        self.cookie_context.browser_context = browser_context;
        self.request_mode = RequestMode::Navigate;
        self
    }

    pub fn with_subframe_navigation_cookie_context(mut self) -> Self {
        let previous = self.cookie_context;
        let mut cookie_context = NetworkCookieRequestContext::subresource(&self.method);
        cookie_context.site_context = previous.site_context;
        cookie_context.site_context_metadata = previous.site_context_metadata;
        cookie_context.initiator_url = previous.initiator_url;
        cookie_context.browser_context = previous.browser_context;
        self.cookie_context = cookie_context;
        self.request_mode = RequestMode::Navigate;
        self.priority_hints.subframe_context = true;
        self
    }

    pub(crate) fn is_navigation_request(&self) -> bool {
        self.request_mode == RequestMode::Navigate
    }

    pub(crate) fn is_subframe_navigation_request(&self) -> bool {
        self.is_navigation_request() && self.priority_hints.subframe_context
    }

    pub(crate) fn is_top_level_navigation_request(&self) -> bool {
        std::mem::discriminant(&self.cookie_context.request_type)
            == std::mem::discriminant(
                &NetworkCookieRequestContext::top_level_navigation(&self.method).request_type,
            )
    }

    pub fn with_initiator_url(mut self, initiator_url: &Url) -> Self {
        self.cookie_context = self
            .cookie_context
            .with_initiator_url(&self.url, initiator_url);
        self
    }

    pub fn with_site_for_cookies_url(mut self, site_for_cookies_url: &Url) -> Self {
        self.cookie_context = self
            .cookie_context
            .with_site_for_cookies_url(&self.url, site_for_cookies_url);
        self
    }

    pub fn with_top_frame_origin_url(mut self, top_frame_origin_url: &Url) -> Self {
        self.cookie_context = self
            .cookie_context
            .with_top_frame_origin_url(&self.url, top_frame_origin_url);
        self
    }

    pub fn with_storage_access_status(
        mut self,
        storage_access_status: NetworkStorageAccessStatus,
    ) -> Self {
        self.cookie_context = self
            .cookie_context
            .with_storage_access_status(storage_access_status);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn effective_cookie_context(&self, request_url: &Url) -> NetworkCookieRequestContext {
        let recomputed = self
            .cookie_context
            .clone()
            .recompute_site_context_for_request(request_url);
        let redirect_types = redirect_types_for_request(
            &self.url,
            request_url,
            self.cookie_context.initiator_url.as_ref(),
            &self.cookie_context.browser_context,
            self.cookie_context.request_type,
            self.cookie_context.is_method_safe,
        );
        let context_downgrade_type = site_context_downgrade_type(
            self.cookie_context.site_context.context,
            recomputed.site_context.context,
        )
        .or(self
            .cookie_context
            .site_context_metadata
            .context
            .downgrade_type);
        let schemeful_context_downgrade_type = site_context_downgrade_type(
            self.cookie_context.site_context.schemeful_context,
            recomputed.site_context.schemeful_context,
        )
        .or(self
            .cookie_context
            .site_context_metadata
            .schemeful_context
            .downgrade_type);
        recomputed.with_site_context_metadata(NetworkSiteContextMetadata::new(
            NetworkSiteContextTrackMetadata::new(
                self.cookie_context
                    .site_context_metadata
                    .context
                    .downgraded_by_cross_site_redirect
                    || context_downgrade_type.is_some(),
                context_downgrade_type,
            )
            .with_http_method(
                self.cookie_context
                    .site_context_metadata
                    .context
                    .http_method,
            )
            .with_redirect_type(redirect_types.context.redirect_type),
            NetworkSiteContextTrackMetadata::new(
                self.cookie_context
                    .site_context_metadata
                    .schemeful_context
                    .downgraded_by_cross_site_redirect
                    || schemeful_context_downgrade_type.is_some(),
                schemeful_context_downgrade_type,
            )
            .with_http_method(
                self.cookie_context
                    .site_context_metadata
                    .schemeful_context
                    .http_method,
            )
            .with_redirect_type(redirect_types.schemeful_context.redirect_type),
        ))
    }
}

fn redirect_status_rewrites_to_get(status: u16, method: &str) -> bool {
    status == 303 && !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD")
        || matches!(status, 301 | 302) && method.eq_ignore_ascii_case("POST")
}

fn is_request_body_header_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-encoding"
            | "content-language"
            | "content-length"
            | "content-location"
            | "content-type"
    )
}

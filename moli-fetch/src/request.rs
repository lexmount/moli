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
use moli_url::{WebOrigin, same_origin};
use url::Url;

use crate::{
    FetchConfig, FetchUrlList, RedirectInfo, network_fetch_result::NetworkObservationRecorder,
};

/// Plain HTTP exchanges and browser Fetch requests have different security
/// contracts. A browser request always carries its environment's origin.
#[derive(Debug, Clone)]
enum RequestContext {
    Http,
    Browser(WebOrigin),
}

/// How explicit Cookie headers coexist with cookies selected from the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CookieHeaderPolicy {
    /// Chromium's browser behavior gives a generated Cookie header precedence.
    #[default]
    PreferStore,
    /// curl-style HTTP clients retain generated cookies and send explicit Cookie
    /// headers only to the initial origin, including after returning to it.
    PreserveExplicit,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub url: Url,
    pub method: String,
    /// Immutable upload bytes, shared when a request is cloned or redirected.
    pub body: Option<bytes::Bytes>,
    /// Wire bytes, with the source encoding resolved before header merging.
    pub request_headers: crate::RequestHeaders,
    redirect_headers: Option<crate::RequestHeaders>,
    additional_cookies: Option<String>,
    cookie_header_policy: CookieHeaderPolicy,
    cache_mode: RequestCacheMode,
    mixed_content_policy: crate::RequestMixedContentPolicy,
    pub resource_type: RequestResourceType,
    subresource_request_metadata: Option<SubresourceRequestMetadata>,
    script_scheduler_priority: Option<ScriptFetchSchedulerPriority>,
    pub priority_hints: RequestPriorityHints,
    pub browser_request_metadata: Option<BrowserRequestMetadata>,
    browser_navigation_kind: BrowserNavigationRequestKind,
    infer_referrer_from_initiator: bool,
    context: RequestContext,
    referrer_url: Option<Url>,
    pub use_page_network_policy: bool,
    pub request_mode: RequestMode,
    use_cors_preflight: bool,
    redirect_handling: RequestRedirectHandling,
    pub credentials_mode: RequestCredentialsMode,
    pub(crate) redirect_chain: Vec<RedirectInfo>,
    network_partition_key: Option<String>,
    auth: Option<ScopedRequestAuth>,
    pub cookie_context: NetworkCookieRequestContext,
    timeout_policy: RequestTimeoutPolicy,
    network_observation_recorder: Option<NetworkObservationRecorder>,
    browser_identity: Option<std::sync::Arc<moli_browser_profile::BrowserIdentityProfile>>,
}

#[derive(Debug, Clone)]
struct ScopedRequestAuth {
    credentials: RequestAuth,
    origin: WebOrigin,
}

/// The lifetime of request headers supplied by an interception command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestHeaderOverride {
    /// CDP overrides expire when the request follows a redirect.
    CurrentRequest {
        headers: crate::RequestHeaders,
        /// A preceding persistent override, or the renderer's original headers when absent.
        redirect_headers: Option<crate::RequestHeaders>,
    },
    /// BiDi modifies the underlying request header list.
    RedirectChain(crate::RequestHeaders),
}

impl RequestHeaderOverride {
    pub fn current_request(headers: crate::RequestHeaders, previous: Option<&Self>) -> Self {
        let redirect_headers = match previous {
            Some(Self::RedirectChain(headers)) => Some(headers.clone()),
            Some(Self::CurrentRequest {
                redirect_headers, ..
            }) => redirect_headers.clone(),
            None => None,
        };
        Self::CurrentRequest {
            headers,
            redirect_headers,
        }
    }

    pub fn headers(&self) -> &crate::RequestHeaders {
        match self {
            Self::CurrentRequest { headers, .. } | Self::RedirectChain(headers) => headers,
        }
    }
}

impl From<crate::RequestHeaders> for RequestHeaderOverride {
    fn from(headers: crate::RequestHeaders) -> Self {
        Self::RedirectChain(headers)
    }
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
    Script,
    SharedWorker,
    Style,
    StyleModule,
    TextTrack,
    Video,
    Worker,
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

/// The valid combinations of Fetch redirect policy and redirect ownership.
/// Keeping them in one value prevents manual/error requests from accidentally
/// enabling automatic transport redirects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRedirectHandling {
    /// The fetch runtime follows redirects.
    Follow,
    /// Return each hop to the caller, which follows under Fetch's follow policy.
    FollowByCaller,
    /// Return the redirect response under Fetch's manual policy.
    Manual,
    /// Return the hop for the browser to reject under Fetch's error policy.
    Error,
}

impl RequestRedirectHandling {
    pub fn redirect_mode(self) -> RequestRedirectMode {
        match self {
            Self::Follow | Self::FollowByCaller => RequestRedirectMode::Follow,
            Self::Manual => RequestRedirectMode::Manual,
            Self::Error => RequestRedirectMode::Error,
        }
    }

    /// Whether the fetch runtime follows redirects itself. `FollowByCaller`
    /// retains Fetch's follow policy but returns false here.
    pub fn transport_follows_redirects(self) -> bool {
        matches!(self, Self::Follow)
    }

    fn single_hop(self) -> Self {
        match self {
            Self::Follow => Self::FollowByCaller,
            other => other,
        }
    }
}

impl From<RequestRedirectMode> for RequestRedirectHandling {
    fn from(mode: RequestRedirectMode) -> Self {
        match mode {
            RequestRedirectMode::Follow => Self::Follow,
            RequestRedirectMode::Manual => Self::Manual,
            RequestRedirectMode::Error => Self::Error,
        }
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
    pub fn with_cookie_header_policy(mut self, policy: CookieHeaderPolicy) -> Self {
        self.cookie_header_policy = policy;
        self
    }

    pub fn cookie_header_policy(&self) -> CookieHeaderPolicy {
        self.cookie_header_policy
    }

    /// Literal Cookie option data, kept separate from the browser cookie store.
    /// It follows this HTTP exchange's redirects without entering stored state.
    pub fn with_additional_cookies(mut self, cookies: Option<String>) -> Self {
        self.additional_cookies = cookies;
        self
    }

    pub fn additional_cookies(&self) -> Option<&str> {
        self.additional_cookies.as_deref()
    }

    /// Constructs a plain HTTP GET. Browser callers must supply an environment
    /// origin using `new_browser` or `with_request_origin`.
    pub fn get(raw_url: &str) -> Result<Self> {
        let url = Url::parse(raw_url)
            .with_context(|| anyhow!("failed to parse request url `{raw_url}`"))?;
        Ok(Self {
            url,
            method: "GET".to_owned(),
            body: None,
            request_headers: crate::RequestHeaders::default(),
            redirect_headers: None,
            additional_cookies: None,
            cookie_header_policy: CookieHeaderPolicy::default(),
            cache_mode: RequestCacheMode::Default,
            mixed_content_policy: crate::RequestMixedContentPolicy::default(),
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            context: RequestContext::Http,
            referrer_url: None,
            use_page_network_policy: false,
            request_mode: RequestMode::Navigate,
            use_cors_preflight: false,
            redirect_handling: RequestRedirectHandling::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            redirect_chain: Vec::new(),
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::top_level_navigation("GET"),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
            browser_identity: None,
        })
    }

    /// Constructs a plain HTTP GET from an already parsed URL.
    pub fn get_with_url(url: Url) -> Self {
        Self {
            url,
            method: "GET".to_owned(),
            body: None,
            request_headers: crate::RequestHeaders::default(),
            redirect_headers: None,
            additional_cookies: None,
            cookie_header_policy: CookieHeaderPolicy::default(),
            cache_mode: RequestCacheMode::Default,
            mixed_content_policy: crate::RequestMixedContentPolicy::default(),
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            context: RequestContext::Http,
            referrer_url: None,
            use_page_network_policy: false,
            request_mode: RequestMode::Navigate,
            use_cors_preflight: false,
            redirect_handling: RequestRedirectHandling::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            redirect_chain: Vec::new(),
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::top_level_navigation("GET"),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
            browser_identity: None,
        }
    }

    /// String header values use UTF-8. To supply HTTP bytes or WebIDL
    /// ByteStrings, pass an explicitly constructed `RequestHeaders` instead.
    pub fn new(
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: impl Into<crate::RequestHeaders>,
    ) -> Result<Self> {
        Self::new_bytes(
            method,
            raw_url,
            body.map(bytes::Bytes::from),
            request_headers,
        )
    }

    /// Constructs a plain HTTP request without browser origin policy.
    /// Browser callers use `new_browser_bytes` instead.
    pub fn new_bytes(
        method: &str,
        raw_url: &str,
        body: Option<bytes::Bytes>,
        request_headers: impl Into<crate::RequestHeaders>,
    ) -> Result<Self> {
        let url = Url::parse(raw_url)
            .with_context(|| anyhow!("failed to parse request url `{raw_url}`"))?;
        Ok(Self::new_http_with_url(method, url, body, request_headers))
    }

    fn new_http_with_url(
        method: &str,
        url: Url,
        body: Option<bytes::Bytes>,
        request_headers: impl Into<crate::RequestHeaders>,
    ) -> Self {
        Self {
            url,
            method: method.to_owned(),
            body,
            request_headers: request_headers.into(),
            redirect_headers: None,
            additional_cookies: None,
            cookie_header_policy: CookieHeaderPolicy::default(),
            cache_mode: RequestCacheMode::Default,
            mixed_content_policy: crate::RequestMixedContentPolicy::default(),
            resource_type: RequestResourceType::Raw,
            subresource_request_metadata: None,
            script_scheduler_priority: None,
            priority_hints: RequestPriorityHints::default(),
            browser_request_metadata: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_referrer_from_initiator: true,
            context: RequestContext::Http,
            referrer_url: None,
            use_page_network_policy: false,
            request_mode: RequestMode::Cors,
            use_cors_preflight: false,
            redirect_handling: RequestRedirectHandling::Follow,
            credentials_mode: RequestCredentialsMode::Include,
            redirect_chain: Vec::new(),
            network_partition_key: None,
            auth: None,
            cookie_context: NetworkCookieRequestContext::subresource(method),
            timeout_policy: RequestTimeoutPolicy::default(),
            network_observation_recorder: None,
            browser_identity: None,
        }
    }

    /// Captures the execution context's identity for this request and redirects.
    /// Updating a Worker must not rebuild the shared transport or mutate an
    /// already admitted request's identity.
    pub fn with_browser_identity(
        mut self,
        identity: std::sync::Arc<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Self {
        self.browser_identity = Some(identity);
        self
    }

    pub(crate) fn browser_identity<'a>(
        &'a self,
        config: &'a FetchConfig,
    ) -> &'a moli_browser_profile::BrowserIdentityProfile {
        self.browser_identity
            .as_deref()
            .unwrap_or_else(|| config.browser_identity())
    }

    pub(crate) fn has_browser_identity_override(&self) -> bool {
        self.browser_identity.is_some()
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

    pub fn with_mixed_content_policy(mut self, policy: crate::RequestMixedContentPolicy) -> Self {
        self.mixed_content_policy = policy;
        self
    }

    pub fn mixed_content_policy(&self) -> crate::RequestMixedContentPolicy {
        self.mixed_content_policy
    }

    pub fn check_mixed_content(&self) -> std::result::Result<(), String> {
        self.mixed_content_policy.check(&self.url)
    }

    pub(crate) fn validate_network_target(&self, target: &Url) -> Result<()> {
        self.mixed_content_policy
            .check(target)
            .map_err(anyhow::Error::msg)?;
        self.validate_request_mode_for_url(target)
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

    /// Require a CORS preflight even for a safelisted method and header list.
    pub fn with_use_cors_preflight(mut self, use_cors_preflight: bool) -> Self {
        self.use_cors_preflight = use_cors_preflight;
        self
    }

    pub fn use_cors_preflight(&self) -> bool {
        self.use_cors_preflight
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

    /// Sets a validated Fetch referrer without changing the request's origin
    /// or cookie initiator. Policy is applied when selecting the outgoing header.
    pub fn with_fetch_referrer(mut self, referrer: &str) -> Result<Self> {
        self.referrer_url = match referrer {
            "" | "about:client" => None,
            _ => Some(Url::parse(referrer).context("invalid Fetch referrer URL")?),
        };
        self.infer_referrer_from_initiator = !referrer.is_empty();
        Ok(self)
    }

    /// Copies referrer state and policy for a derived request, such as a CORS
    /// preflight. Other metadata, including integrity, belongs to that request.
    pub fn with_referrer_from(mut self, source: &Self) -> Self {
        self.referrer_url = source.referrer_url.clone();
        self.infer_referrer_from_initiator = source.infer_referrer_from_initiator;
        let source_metadata = source.subresource_request_metadata();
        let metadata = self.subresource_request_metadata.get_or_insert_default();
        metadata.referrer_policy =
            source_metadata.and_then(|metadata| metadata.referrer_policy.clone());
        metadata.document_referrer_policy =
            source_metadata.and_then(|metadata| metadata.document_referrer_policy.clone());
        self
    }

    pub fn infers_referrer_from_initiator(&self) -> bool {
        self.infer_referrer_from_initiator
    }

    /// The nonempty policy used for both Fetch event metadata and network headers.
    pub fn effective_referrer_policy(&self) -> &str {
        let metadata = self.subresource_request_metadata();
        crate::effective_referrer_policy(
            metadata.and_then(|metadata| metadata.referrer_policy.as_deref()),
            metadata.and_then(|metadata| metadata.document_referrer_policy.as_deref()),
        )
    }

    /// Computes the inferred Referer header without changing the initiator
    /// used for cookie-site and request-origin decisions.
    pub fn referrer_header_value(&self, request_url: &Url) -> Option<String> {
        if !self.infers_referrer_from_initiator() {
            return None;
        }
        let referrer_url = self
            .referrer_url
            .as_ref()
            .or(self.cookie_context.initiator_url.as_ref())?;
        crate::referrer_header_value(
            referrer_url,
            request_url,
            Some(self.effective_referrer_policy()),
            None,
        )
    }

    /// Retains the referrer already selected for this hop, then applies the
    /// redirect response's policy. A later, more permissive policy cannot
    /// restore an omitted referrer or its stripped path.
    pub fn update_referrer_for_redirect(
        &mut self,
        response_url: &Url,
        response_headers: &[(String, Vec<u8>)],
    ) {
        self.referrer_url = self
            .referrer_header_value(response_url)
            .and_then(|value| Url::parse(&value).ok());
        if self.referrer_url.is_none() {
            self.infer_referrer_from_initiator = false;
        }
        if let Some(policy) = crate::response_referrer_policy_from_headers(response_headers) {
            self.subresource_request_metadata
                .get_or_insert_default()
                .referrer_policy = Some(policy);
        }
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

    pub fn redirect_handling(&self) -> RequestRedirectHandling {
        self.redirect_handling
    }

    pub fn redirect_mode(&self) -> RequestRedirectMode {
        self.redirect_handling.redirect_mode()
    }

    /// Whether the fetch runtime follows redirects itself. A caller-owned loop
    /// returns false while [`Self::redirect_mode`] still returns `Follow`.
    pub fn transport_follows_redirects(&self) -> bool {
        self.redirect_handling.transport_follows_redirects()
    }

    pub fn with_redirect_handling(mut self, handling: RequestRedirectHandling) -> Self {
        self.redirect_handling = handling;
        self
    }

    /// Stops at this hop while preserving the browser's redirect policy.
    /// Apply this at transport dispatch, after configuring the policy:
    /// a later [`Self::with_redirect_mode`] call resets redirect ownership.
    pub fn with_single_hop(mut self) -> Self {
        self.redirect_handling = self.redirect_handling.single_hop();
        self
    }

    /// Sets the browser's redirect policy and its default transport handling.
    /// `Follow` enables runtime following, including after `with_single_hop()`.
    /// Caller-owned redirect loops must apply [`Self::with_single_hop`] last.
    pub fn with_redirect_mode(mut self, redirect_mode: RequestRedirectMode) -> Self {
        self.redirect_handling = redirect_mode.into();
        self
    }

    pub fn with_request_mode(mut self, request_mode: RequestMode) -> Self {
        self.request_mode = request_mode;
        self
    }

    /// Constructs a browser Fetch request with an explicit environment origin.
    /// URL resolution and referrer/cookie context are supplied independently.
    pub fn new_browser_bytes(
        method: &str,
        raw_url: &str,
        body: Option<bytes::Bytes>,
        headers: impl Into<crate::RequestHeaders>,
        origin: WebOrigin,
    ) -> Result<Self> {
        Ok(Self::new_bytes(method, raw_url, body, headers)?.with_request_origin(origin))
    }

    pub fn new_browser(
        method: &str,
        url: Url,
        body: Option<bytes::Bytes>,
        headers: impl Into<crate::RequestHeaders>,
        origin: WebOrigin,
    ) -> Self {
        Self::new_http_with_url(method, url, body, headers).with_request_origin(origin)
    }

    /// Explicitly enters browser Fetch semantics. Setting an initiator/referrer
    /// URL alone never supplies a browser origin.
    pub fn with_request_origin(mut self, origin: WebOrigin) -> Self {
        self.context = RequestContext::Browser(origin);
        self
    }

    pub fn request_origin(&self) -> Option<&WebOrigin> {
        match &self.context {
            RequestContext::Http => None,
            RequestContext::Browser(origin) => Some(origin),
        }
    }

    /// Renderer entry points require browser semantics even when the request
    /// happens to use a mode that would not otherwise run a CORS check.
    pub fn browser_origin(&self) -> Result<&WebOrigin> {
        self.request_origin()
            .context("browser fetch requires an explicit environment origin")
    }

    /// Restore request-scoped interception headers before applying redirect rules.
    pub fn apply_redirect_status(&mut self, status: u16) {
        if let Some(headers) = self.redirect_headers.take() {
            self.request_headers = headers;
        }
        if redirect_status_rewrites_to_get(status, &self.method) {
            self.method = "GET".to_owned();
            self.body = None;
            self.request_headers
                .retain(|(name, _)| !is_request_body_header_name(name));
        }
    }

    pub fn with_redirect_headers(mut self, headers: Option<crate::RequestHeaders>) -> Self {
        self.redirect_headers = headers;
        self
    }

    pub fn with_credentials_mode(mut self, credentials_mode: RequestCredentialsMode) -> Self {
        self.credentials_mode = credentials_mode;
        self
    }

    /// Continues a fetch at `url` with redirects already followed by its caller.
    /// Keep synthetic redirects too: their URLs affect Origin and credentials
    /// even though their response headers are not subject to network CORS checks.
    pub fn with_redirect_chain(mut self, redirect_chain: Vec<RedirectInfo>) -> Self {
        self.redirect_chain = redirect_chain;
        self
    }

    pub fn redirect_chain(&self) -> &[RedirectInfo] {
        &self.redirect_chain
    }

    pub(crate) fn initial_url(&self) -> &Url {
        self.redirect_chain
            .first()
            .map_or(&self.url, |redirect| &redirect.from_url)
    }

    /// Records a followed redirect. The caller controls method and current URL
    /// updates so this also supports transports that retain the original URL.
    pub fn record_redirect(&mut self, redirect: RedirectInfo) {
        self.redirect_chain.push(redirect);
    }

    /// Follows an authorized redirect without rebuilding the Fetch request.
    pub fn follow_redirect(&mut self, redirect: RedirectInfo) -> Result<()> {
        self.validate_network_target(&redirect.to_url)?;
        self.apply_redirect_status(redirect.status);
        self.url = redirect.to_url.clone();
        self.record_redirect(redirect);
        Ok(())
    }

    pub fn redirect_count(&self) -> usize {
        self.url_list(&self.url).redirect_count()
    }

    pub fn url_list<'a>(&'a self, request_url: &'a Url) -> FetchUrlList<'a> {
        FetchUrlList::new(request_url, &self.redirect_chain)
    }

    /// Checks mode restrictions before using a cache entry or starting a hop.
    pub fn validate_request_mode_for_url(&self, request_url: &Url) -> Result<()> {
        if self.request_mode != RequestMode::SameOrigin || request_url.scheme() == "data" {
            return Ok(());
        }
        let origin = self
            .request_origin()
            .context("same-origin request mode requires an initiating origin")?;
        self.url_list(request_url)
            .validate_request_mode(self.request_mode, origin)
            .map_err(anyhow::Error::msg)
    }

    /// Checks one network response against the request's state before that
    /// response's redirect is recorded. Synthetic responses must not use this.
    pub fn validate_cors_response_for_url(
        &self,
        response_url: &Url,
        response_headers: &[(String, Vec<u8>)],
    ) -> Result<(), String> {
        if self.request_mode != RequestMode::Cors
            || !matches!(response_url.scheme(), "http" | "https")
        {
            return Ok(());
        }
        let Some(origin) = self.request_origin() else {
            // Standalone HTTP clients have no browser request origin.
            return Ok(());
        };
        let urls = self.url_list(response_url);
        if !urls.has_cross_origin_url(origin) {
            return Ok(());
        }
        crate::validate_cors_response_for_origin(
            &urls.serialized_origin(origin),
            response_headers,
            self.credentials_mode,
        )
    }

    /// Whether the URL list has left the initiating origin. For subresource
    /// fetches, returning to that origin cannot restore basic response tainting.
    pub fn has_cross_origin_url(&self, request_url: &Url) -> bool {
        self.request_origin().is_some_and(|origin| {
            !origin.same_origin(&(&self.url).into())
                || self.url_list(request_url).has_cross_origin_url(origin)
        })
    }

    /// Fetch's serialized request origin, including redirect taint. A first
    /// same-origin-to-cross-origin hop retains the origin; crossing origins
    /// again after leaving it serializes as null.
    pub fn serialized_origin(&self) -> String {
        let Some(origin) = self.request_origin() else {
            return "null".to_owned();
        };
        self.url_list(&self.url).serialized_origin(origin)
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
            RequestCredentialsMode::SameOrigin => !self.has_cross_origin_url(request_url),
        }
    }

    /// Cookie policy uses the environment origin, independently of the URL used
    /// to produce Referer. Adapt tuple origins to the cookie layer's URL-based
    /// initiator API without replacing the request's referrer context.
    pub fn network_cookie_context(&self) -> NetworkCookieRequestContext {
        let mut context = self.cookie_context.clone();
        if let Some(WebOrigin::Tuple(origin)) = self.request_origin() {
            let origin_url =
                Url::parse(origin.ascii_serialization()).expect("a tuple origin is a valid URL");
            if context
                .initiator_url
                .as_ref()
                .is_none_or(|url| !same_origin(url, &origin_url))
            {
                // Keep the embedding page's site-for-cookies/partition.
                let browser_context = context.browser_context.clone();
                context = context.with_initiator_url(&self.url, &origin_url);
                context.browser_context = browser_context;
                context = context.recompute_site_context_for_request(&self.url);
            }
        }
        context
    }

    pub fn auth(&self) -> Option<&RequestAuth> {
        self.auth.as_ref().map(|auth| &auth.credentials)
    }

    pub(crate) fn auth_for_url(&self, request_url: &Url) -> Option<&RequestAuth> {
        let auth = self.auth.as_ref()?;
        (auth.credentials.target != RequestAuthTarget::Server
            || auth.origin.same_origin(&WebOrigin::from_url(request_url)))
        .then_some(&auth.credentials)
    }

    pub fn auth_requires_buffered_transport(&self) -> bool {
        self.auth()
            .is_some_and(|auth| !auth.can_use_header_transport())
    }

    pub fn preemptive_server_basic_auth_for_url(&self, request_url: &Url) -> Option<(&str, &str)> {
        let auth = self.auth_for_url(request_url)?;
        (auth.target == RequestAuthTarget::Server && auth.scheme == RequestAuthScheme::Basic)
            .then_some((auth.username.as_str(), auth.password.as_str()))
    }

    pub fn set_auth(&mut self, auth: Option<RequestAuth>) {
        // Pin credentials to where they were supplied, rather than the mutable
        // current URL. A later CDP auth response grants new credentials only to
        // that challenged origin, without authorizing subsequent redirects.
        self.auth = auth.map(|credentials| ScopedRequestAuth {
            credentials,
            origin: WebOrigin::from_url(&self.url),
        });
    }

    pub fn with_auth(mut self, auth: RequestAuth) -> Self {
        self.set_auth(Some(auth));
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

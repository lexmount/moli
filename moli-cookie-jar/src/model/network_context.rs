//! Network/request context types used to evaluate browser cookie policy.

use cookie_store::{
    BrowserSiteContext as CoreBrowserSiteContext, CookiePartitionKey as CoreCookiePartitionKey,
    SameSiteContextDowngradeType as CoreSameSiteContextDowngradeType,
    SameSiteContextHttpMethod as CoreSameSiteContextHttpMethod, SameSiteContextMetadata,
    SameSiteContextRedirectType as CoreSameSiteContextRedirectType,
    SameSiteContextTrackMetadata as CoreSameSiteContextTrackMetadata,
    StorageAccessStatus as CoreStorageAccessStatus,
};
use moli_site::same_site_hosts;
use moli_storage_key::{MoliStorageKey, deserialize_serialized_storage_key, site_for_url};
use url::Url;

use super::stored_cookie::StoredCookiePartitionKey;

/// Origin of a cookie insertion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSource {
    #[cfg(any(test, feature = "test-support"))]
    Http,
    Management,
}

/// SameSite request context computed at the network boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkSameSiteContext {
    SameSiteStrict,
    SameSiteLax,
    SameSiteLaxMethodUnsafe,
    CrossSite,
}

/// Pair of schemeless and schemeful SameSite contexts for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkSiteContext {
    /// Legacy-compatible site context that ignores scheme differences.
    pub context: NetworkSameSiteContext,
    /// Modern site context that treats `http` and `https` as different sites.
    pub schemeful_context: NetworkSameSiteContext,
}

impl NetworkSiteContext {
    /// Builds the default same-site context used before initiator data is known.
    pub const fn same_site() -> Self {
        Self::new(
            NetworkSameSiteContext::SameSiteStrict,
            NetworkSameSiteContext::SameSiteStrict,
        )
    }

    pub const fn cross_site() -> Self {
        Self::new(
            NetworkSameSiteContext::CrossSite,
            NetworkSameSiteContext::CrossSite,
        )
    }

    /// Builds an explicit schemeless/schemeful SameSite context pair.
    pub const fn new(
        context: NetworkSameSiteContext,
        schemeful_context: NetworkSameSiteContext,
    ) -> Self {
        Self {
            context,
            schemeful_context,
        }
    }

    /// Return whether both SameSite tracks are cross-site.
    pub const fn is_cross_site(self) -> bool {
        matches!(self.context, NetworkSameSiteContext::CrossSite)
            && matches!(self.schemeful_context, NetworkSameSiteContext::CrossSite)
    }
}

/// Request shape relevant to SameSite Lax handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCookieRequestType {
    Subresource,
    TopLevelNavigation,
}

/// Storage Access API status supplied by the embedding browser facade.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NetworkStorageAccessStatus {
    #[default]
    None,
    Granted,
}

/// Browser-frame context used by SameSite and storage-access policy checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkBrowserSiteContext {
    /// Site-for-cookies basis from the request's document or frame tree.
    pub site_for_cookies_url: Option<Url>,
    /// Top-frame origin used when site-for-cookies is unavailable.
    pub top_frame_origin_url: Option<Url>,
    /// Storage Access API state for third-party cookie decisions.
    pub storage_access_status: NetworkStorageAccessStatus,
    /// Browser-computed CHIPS key. This is independent from SameSite and
    /// survives request-initiator updates.
    pub cookie_partition_key: Option<StoredCookiePartitionKey>,
}

impl NetworkBrowserSiteContext {
    /// Sets the site-for-cookies URL used to recompute SameSite context.
    pub fn with_site_for_cookies_url(mut self, url: &Url) -> Self {
        self.site_for_cookies_url = Some(url.clone());
        self
    }

    /// Sets the top-frame origin fallback used for browser policy diagnostics.
    pub fn with_top_frame_origin_url(mut self, url: &Url) -> Self {
        self.top_frame_origin_url = Some(url.clone());
        self
    }

    /// Sets the Storage Access API status for this request.
    pub fn with_storage_access_status(
        mut self,
        storage_access_status: NetworkStorageAccessStatus,
    ) -> Self {
        self.storage_access_status = storage_access_status;
        self
    }

    /// Sets an explicit CHIPS key for this browsing context.
    pub fn with_cookie_partition_key(mut self, partition_key: StoredCookiePartitionKey) -> Self {
        self.cookie_partition_key = Some(partition_key);
        self
    }

    /// Extracts the CHIPS key from Moli's serialized network/storage key.
    pub fn with_serialized_storage_key(mut self, serialized_key: &str) -> Self {
        if let Some(storage_key) = deserialize_serialized_storage_key(serialized_key) {
            self.cookie_partition_key = cookie_partition_key_from_storage_key(&storage_key);
        }
        self
    }

    /// Returns the URL that should drive site comparison, if one is known.
    pub fn site_basis_url(&self) -> Option<&Url> {
        self.site_for_cookies_url
            .as_ref()
            .or(self.top_frame_origin_url.as_ref())
    }
}

/// Alias used by document-cookie callers that expose a browser facade.
pub type BrowserCookieFacadeContext = NetworkBrowserSiteContext;
/// Alias used by facade code so callers do not depend on network naming.
pub type BrowserCookieStorageAccessStatus = NetworkStorageAccessStatus;

/// Per-request browser-context overrides supplied by a facade.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowserCookieFacadeContextOverrides {
    /// Override for the site-for-cookies URL.
    pub site_for_cookies_url: Option<Url>,
    /// Override for the top-frame origin URL.
    pub top_frame_origin_url: Option<Url>,
    /// Override for the Storage Access API status.
    pub storage_access_status: Option<BrowserCookieStorageAccessStatus>,
}

impl BrowserCookieFacadeContextOverrides {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_site_for_cookies_url(mut self, url: &Url) -> Self {
        self.site_for_cookies_url = Some(url.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_top_frame_origin_url(mut self, url: &Url) -> Self {
        self.top_frame_origin_url = Some(url.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_storage_access_status(
        mut self,
        storage_access_status: BrowserCookieStorageAccessStatus,
    ) -> Self {
        self.storage_access_status = Some(storage_access_status);
        self
    }
}

/// Global facade switches and default context values for cookie access.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowserCookieFacadeOverrides {
    /// Optional global cookie-enable switch.
    pub cookies_enabled: Option<bool>,
    /// Default site-for-cookies URL exposed by the facade.
    pub site_for_cookies_url: Option<Url>,
    /// Default top-frame origin URL exposed by the facade.
    pub top_frame_origin_url: Option<Url>,
    /// Default Storage Access API status exposed by the facade.
    pub storage_access_status: Option<BrowserCookieStorageAccessStatus>,
}

impl BrowserCookieFacadeOverrides {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_cookies_enabled(mut self, enabled: bool) -> Self {
        self.cookies_enabled = Some(enabled);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_storage_access_status(
        mut self,
        storage_access_status: BrowserCookieStorageAccessStatus,
    ) -> Self {
        self.storage_access_status = Some(storage_access_status);
        self
    }

    /// Extracts only the browser-context overrides from the full facade config.
    pub fn browser_context_overrides(&self) -> BrowserCookieFacadeContextOverrides {
        BrowserCookieFacadeContextOverrides {
            site_for_cookies_url: self.site_for_cookies_url.clone(),
            top_frame_origin_url: self.top_frame_origin_url.clone(),
            storage_access_status: self.storage_access_status,
        }
    }
}

/// Complete cookie policy context for one outgoing network request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkCookieRequestContext {
    /// Current schemeless and schemeful SameSite context.
    pub site_context: NetworkSiteContext,
    /// Method/redirect metadata attached to both SameSite tracks.
    pub site_context_metadata: NetworkSiteContextMetadata,
    /// Whether this lookup is a subresource or top-level navigation.
    pub request_type: NetworkCookieRequestType,
    /// Whether the HTTP method is safe for SameSite Lax navigation handling.
    pub is_method_safe: bool,
    /// Request initiator, when the caller can identify one.
    pub initiator_url: Option<Url>,
    /// Browser-frame context used by cookie policy.
    pub browser_context: NetworkBrowserSiteContext,
}

impl NetworkCookieRequestContext {
    /// Builds a subresource request context with method metadata.
    pub fn subresource(method: &str) -> Self {
        let is_method_safe = is_safe_http_method(method);
        Self {
            site_context: NetworkSiteContext::same_site(),
            site_context_metadata: NetworkSiteContextMetadata::none(),
            request_type: NetworkCookieRequestType::Subresource,
            is_method_safe,
            initiator_url: None,
            browser_context: NetworkBrowserSiteContext::default(),
        }
        .with_http_method(parse_network_same_site_http_method(method))
    }

    /// Builds a top-level navigation request context with method metadata.
    pub fn top_level_navigation(method: &str) -> Self {
        let is_method_safe = is_safe_http_method(method);
        Self {
            site_context: NetworkSiteContext::same_site(),
            site_context_metadata: NetworkSiteContextMetadata::none(),
            request_type: NetworkCookieRequestType::TopLevelNavigation,
            is_method_safe,
            initiator_url: None,
            browser_context: NetworkBrowserSiteContext::default(),
        }
        .with_http_method(parse_network_same_site_http_method(method))
    }

    /// Records the initiator and recomputes SameSite context for the request URL.
    pub fn with_initiator_url(mut self, request_url: &Url, initiator_url: &Url) -> Self {
        self.initiator_url = Some(initiator_url.clone());
        self.browser_context = self
            .browser_context
            .with_site_for_cookies_url(initiator_url)
            .with_top_frame_origin_url(initiator_url);
        self.site_context = self.site_context_for_request(request_url);
        if self.browser_context.cookie_partition_key.is_none() {
            self.browser_context.cookie_partition_key = Some(StoredCookiePartitionKey::site(
                site_for_url(initiator_url),
                false,
            ));
        }
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_same_site_context(mut self, same_site_context: NetworkSameSiteContext) -> Self {
        self.site_context = NetworkSiteContext::new(same_site_context, same_site_context);
        self
    }

    /// Treat this request as cross-site for SameSite cookie inclusion.
    pub fn with_cross_site_context(mut self) -> Self {
        self.site_context = NetworkSiteContext::cross_site();
        self
    }

    /// Replaces the already-computed SameSite context.
    pub fn with_site_context(mut self, site_context: NetworkSiteContext) -> Self {
        self.site_context = site_context;
        self
    }

    /// Merges explicit SameSite metadata while preserving existing unset tracks.
    pub fn with_site_context_metadata(
        mut self,
        site_context_metadata: NetworkSiteContextMetadata,
    ) -> Self {
        self.site_context_metadata = NetworkSiteContextMetadata::new(
            site_context_metadata
                .context
                .with_http_method(
                    if matches!(
                        site_context_metadata.context.http_method,
                        NetworkSameSiteHttpMethod::Unset
                    ) {
                        self.site_context_metadata.context.http_method
                    } else {
                        site_context_metadata.context.http_method
                    },
                )
                .with_redirect_type(
                    if matches!(
                        site_context_metadata.context.redirect_type,
                        NetworkSameSiteRedirectType::Unset
                    ) {
                        self.site_context_metadata.context.redirect_type
                    } else {
                        site_context_metadata.context.redirect_type
                    },
                ),
            site_context_metadata
                .schemeful_context
                .with_http_method(
                    if matches!(
                        site_context_metadata.schemeful_context.http_method,
                        NetworkSameSiteHttpMethod::Unset
                    ) {
                        self.site_context_metadata.schemeful_context.http_method
                    } else {
                        site_context_metadata.schemeful_context.http_method
                    },
                )
                .with_redirect_type(
                    if matches!(
                        site_context_metadata.schemeful_context.redirect_type,
                        NetworkSameSiteRedirectType::Unset
                    ) {
                        self.site_context_metadata.schemeful_context.redirect_type
                    } else {
                        site_context_metadata.schemeful_context.redirect_type
                    },
                ),
        );
        self
    }

    /// Attaches an HTTP method to both SameSite metadata tracks.
    pub fn with_http_method(mut self, method: NetworkSameSiteHttpMethod) -> Self {
        self.site_context_metadata.context = self
            .site_context_metadata
            .context
            .with_http_method(method)
            .with_redirect_type(
                if matches!(
                    self.site_context_metadata.context.redirect_type,
                    NetworkSameSiteRedirectType::Unset
                ) {
                    NetworkSameSiteRedirectType::NoRedirect
                } else {
                    self.site_context_metadata.context.redirect_type
                },
            );
        self.site_context_metadata.schemeful_context = self
            .site_context_metadata
            .schemeful_context
            .with_http_method(method)
            .with_redirect_type(
                if matches!(
                    self.site_context_metadata.schemeful_context.redirect_type,
                    NetworkSameSiteRedirectType::Unset
                ) {
                    NetworkSameSiteRedirectType::NoRedirect
                } else {
                    self.site_context_metadata.schemeful_context.redirect_type
                },
            );
        self
    }

    /// Sets site-for-cookies and recomputes SameSite context for the request.
    pub fn with_site_for_cookies_url(
        mut self,
        request_url: &Url,
        site_for_cookies_url: &Url,
    ) -> Self {
        self.browser_context = self
            .browser_context
            .with_site_for_cookies_url(site_for_cookies_url);
        self.site_context = self.site_context_for_request(request_url);
        self
    }

    /// Sets top-frame origin and recomputes SameSite context for the request.
    pub fn with_top_frame_origin_url(
        mut self,
        request_url: &Url,
        top_frame_origin_url: &Url,
    ) -> Self {
        self.browser_context = self
            .browser_context
            .with_top_frame_origin_url(top_frame_origin_url);
        self.site_context = self.site_context_for_request(request_url);
        self
    }

    /// Sets Storage Access API state without changing SameSite context.
    pub fn with_storage_access_status(
        mut self,
        storage_access_status: NetworkStorageAccessStatus,
    ) -> Self {
        self.browser_context = self
            .browser_context
            .with_storage_access_status(storage_access_status);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_site_context_metadata_for_redirects(
        mut self,
        original_request_url: &Url,
        current_request_url: &Url,
    ) -> Self {
        let redirect_metadata = redirect_types_for_request(
            original_request_url,
            current_request_url,
            self.initiator_url.as_ref(),
            &self.browser_context,
            self.request_type,
            self.is_method_safe,
        );
        self.site_context_metadata = NetworkSiteContextMetadata::new(
            redirect_metadata
                .context
                .with_http_method(self.site_context_metadata.context.http_method),
            redirect_metadata
                .schemeful_context
                .with_http_method(self.site_context_metadata.schemeful_context.http_method),
        );
        self
    }

    /// Recomputes SameSite context for a redirected or otherwise updated URL.
    pub fn recompute_site_context_for_request(mut self, request_url: &Url) -> Self {
        self.site_context = self.site_context_for_request(request_url);
        self
    }

    fn site_context_for_request(&self, request_url: &Url) -> NetworkSiteContext {
        let mut combined: Option<NetworkSiteContext> = None;

        if let Some(site_basis_url) = self.browser_context.site_basis_url() {
            combined = Some(approximate_site_context(
                request_url,
                site_basis_url,
                self.request_type,
                self.is_method_safe,
            ));
        }

        if let Some(initiator_url) = self.initiator_url.as_ref() {
            let initiator_context = approximate_site_context(
                request_url,
                initiator_url,
                self.request_type,
                self.is_method_safe,
            );
            combined = Some(match combined {
                Some(existing) => NetworkSiteContext::new(
                    persist_redirect_downgraded_same_site_context(
                        existing.context,
                        initiator_context.context,
                    ),
                    persist_redirect_downgraded_same_site_context(
                        existing.schemeful_context,
                        initiator_context.schemeful_context,
                    ),
                ),
                None => initiator_context,
            });
        }

        combined.unwrap_or(self.site_context)
    }
}

/// Metadata for one SameSite track across methods and redirects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkSiteContextTrackMetadata {
    /// True once a redirect chain made the request less same-site.
    pub downgraded_by_cross_site_redirect: bool,
    /// Exact downgrade classification, if the context weakened.
    pub downgrade_type: Option<NetworkSameSiteContextDowngradeType>,
    /// HTTP method that influenced SameSite Lax handling.
    pub http_method: NetworkSameSiteHttpMethod,
    /// Redirect-chain classification for diagnostics.
    pub redirect_type: NetworkSameSiteRedirectType,
}

impl NetworkSiteContextTrackMetadata {
    /// Builds track metadata before method and redirect labels are attached.
    pub fn new(
        downgraded_by_cross_site_redirect: bool,
        downgrade_type: Option<NetworkSameSiteContextDowngradeType>,
    ) -> Self {
        Self {
            downgraded_by_cross_site_redirect,
            downgrade_type,
            http_method: NetworkSameSiteHttpMethod::Unset,
            redirect_type: NetworkSameSiteRedirectType::Unset,
        }
    }

    /// Builds empty metadata for a request with no downgrade information.
    pub fn none() -> Self {
        Self::new(false, None)
    }

    /// Attaches the HTTP method observed for this track.
    pub fn with_http_method(mut self, http_method: NetworkSameSiteHttpMethod) -> Self {
        self.http_method = http_method;
        self
    }

    /// Attaches the redirect-chain classification for this track.
    pub fn with_redirect_type(mut self, redirect_type: NetworkSameSiteRedirectType) -> Self {
        self.redirect_type = redirect_type;
        self
    }
}

/// Method and redirect metadata for both SameSite evaluation tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkSiteContextMetadata {
    /// Schemeless SameSite track metadata.
    pub context: NetworkSiteContextTrackMetadata,
    /// Schemeful SameSite track metadata.
    pub schemeful_context: NetworkSiteContextTrackMetadata,
}

impl NetworkSiteContextMetadata {
    /// Builds metadata for both SameSite tracks.
    pub fn new(
        context: NetworkSiteContextTrackMetadata,
        schemeful_context: NetworkSiteContextTrackMetadata,
    ) -> Self {
        Self {
            context,
            schemeful_context,
        }
    }

    /// Builds empty metadata for both SameSite tracks.
    pub fn none() -> Self {
        Self::new(
            NetworkSiteContextTrackMetadata::none(),
            NetworkSiteContextTrackMetadata::none(),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn schemeful_only(
        downgraded_by_cross_site_redirect: bool,
        downgrade_type: Option<NetworkSameSiteContextDowngradeType>,
    ) -> Self {
        Self::new(
            NetworkSiteContextTrackMetadata::none(),
            NetworkSiteContextTrackMetadata::new(downgraded_by_cross_site_redirect, downgrade_type),
        )
    }
}

/// How a request became less same-site across redirect handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkSameSiteContextDowngradeType {
    StrictToLax,
    StrictToCross,
    LaxToCross,
}

/// HTTP method classification preserved for cookie-access diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkSameSiteHttpMethod {
    Unset,
    Unknown,
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
}

/// Redirect-chain classification preserved for cookie-access diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkSameSiteRedirectType {
    Unset,
    NoRedirect,
    CrossSiteRedirect,
    PartialSameSiteRedirect,
    AllSameSiteRedirect,
}

fn is_safe_http_method(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET")
        || method.eq_ignore_ascii_case("HEAD")
        || method.eq_ignore_ascii_case("OPTIONS")
        || method.eq_ignore_ascii_case("TRACE")
}

fn parse_network_same_site_http_method(method: &str) -> NetworkSameSiteHttpMethod {
    if method.eq_ignore_ascii_case("GET") {
        NetworkSameSiteHttpMethod::Get
    } else if method.eq_ignore_ascii_case("HEAD") {
        NetworkSameSiteHttpMethod::Head
    } else if method.eq_ignore_ascii_case("POST") {
        NetworkSameSiteHttpMethod::Post
    } else if method.eq_ignore_ascii_case("PUT") {
        NetworkSameSiteHttpMethod::Put
    } else if method.eq_ignore_ascii_case("DELETE") {
        NetworkSameSiteHttpMethod::Delete
    } else if method.eq_ignore_ascii_case("CONNECT") {
        NetworkSameSiteHttpMethod::Connect
    } else if method.eq_ignore_ascii_case("OPTIONS") {
        NetworkSameSiteHttpMethod::Options
    } else if method.eq_ignore_ascii_case("TRACE") {
        NetworkSameSiteHttpMethod::Trace
    } else if method.eq_ignore_ascii_case("PATCH") {
        NetworkSameSiteHttpMethod::Patch
    } else {
        NetworkSameSiteHttpMethod::Unknown
    }
}

fn approximate_site_context(
    request_url: &Url,
    initiator_url: &Url,
    request_type: NetworkCookieRequestType,
    is_method_safe: bool,
) -> NetworkSiteContext {
    NetworkSiteContext::new(
        approximate_same_site_context(
            request_url,
            initiator_url,
            false,
            request_type,
            is_method_safe,
        ),
        approximate_same_site_context(
            request_url,
            initiator_url,
            true,
            request_type,
            is_method_safe,
        ),
    )
}

fn approximate_same_site_context(
    request_url: &Url,
    initiator_url: &Url,
    schemeful: bool,
    request_type: NetworkCookieRequestType,
    is_method_safe: bool,
) -> NetworkSameSiteContext {
    let Some(request_host) = request_url.host_str() else {
        return NetworkSameSiteContext::CrossSite;
    };
    let Some(initiator_host) = initiator_url.host_str() else {
        return NetworkSameSiteContext::CrossSite;
    };

    let same_registrable_site = same_site_hosts(request_host, initiator_host);
    let same_site = same_registrable_site
        && (!schemeful
            || normalized_network_same_site_scheme(request_url.scheme())
                == normalized_network_same_site_scheme(initiator_url.scheme()));

    if same_site {
        NetworkSameSiteContext::SameSiteStrict
    } else if matches!(request_type, NetworkCookieRequestType::TopLevelNavigation) {
        if is_method_safe {
            NetworkSameSiteContext::SameSiteLax
        } else {
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        }
    } else {
        NetworkSameSiteContext::CrossSite
    }
}

fn normalized_network_same_site_scheme(scheme: &str) -> &str {
    match scheme {
        "ws" => "http",
        "wss" => "https",
        other => other,
    }
}

/// Classifies whether a SameSite context transition became less permissive.
pub fn site_context_downgrade_type(
    previous: NetworkSameSiteContext,
    current: NetworkSameSiteContext,
) -> Option<NetworkSameSiteContextDowngradeType> {
    match (
        same_site_inclusion_class(previous),
        same_site_inclusion_class(current),
    ) {
        (NetworkSameSiteInclusionClass::Strict, NetworkSameSiteInclusionClass::Lax) => {
            Some(NetworkSameSiteContextDowngradeType::StrictToLax)
        }
        (NetworkSameSiteInclusionClass::Strict, NetworkSameSiteInclusionClass::Cross) => {
            Some(NetworkSameSiteContextDowngradeType::StrictToCross)
        }
        (NetworkSameSiteInclusionClass::Lax, NetworkSameSiteInclusionClass::Cross) => {
            Some(NetworkSameSiteContextDowngradeType::LaxToCross)
        }
        _ => None,
    }
}

fn approximate_is_same_site(request_url: &Url, initiator_url: &Url, schemeful: bool) -> bool {
    let Some(request_host) = request_url.host_str() else {
        return false;
    };
    let Some(initiator_host) = initiator_url.host_str() else {
        return false;
    };

    let same_registrable_site = same_site_hosts(request_host, initiator_host);
    same_registrable_site && (!schemeful || request_url.scheme() == initiator_url.scheme())
}

fn redirect_type_for_track(
    original_request_url: &Url,
    current_request_url: &Url,
    initiator_url: &Url,
    schemeful: bool,
) -> NetworkSameSiteRedirectType {
    let original_same_site =
        approximate_is_same_site(original_request_url, initiator_url, schemeful);
    let current_same_site = approximate_is_same_site(current_request_url, initiator_url, schemeful);

    match (original_same_site, current_same_site) {
        (true, true) => NetworkSameSiteRedirectType::AllSameSiteRedirect,
        (false, true) => NetworkSameSiteRedirectType::PartialSameSiteRedirect,
        _ => NetworkSameSiteRedirectType::CrossSiteRedirect,
    }
}

/// Computes schemeless and schemeful redirect diagnostics for a request chain.
pub fn redirect_types_for_request(
    original_request_url: &Url,
    current_request_url: &Url,
    initiator_url: Option<&Url>,
    browser_context: &NetworkBrowserSiteContext,
    request_type: NetworkCookieRequestType,
    is_method_safe: bool,
) -> NetworkSiteContextMetadata {
    if original_request_url == current_request_url {
        return NetworkSiteContextMetadata::new(
            NetworkSiteContextTrackMetadata::none()
                .with_redirect_type(NetworkSameSiteRedirectType::NoRedirect),
            NetworkSiteContextTrackMetadata::none()
                .with_redirect_type(NetworkSameSiteRedirectType::NoRedirect),
        );
    }

    let Some(site_basis_url) = browser_context.site_basis_url().or(initiator_url) else {
        return NetworkSiteContextMetadata::new(
            NetworkSiteContextTrackMetadata::none()
                .with_redirect_type(NetworkSameSiteRedirectType::Unset),
            NetworkSiteContextTrackMetadata::none()
                .with_redirect_type(NetworkSameSiteRedirectType::Unset),
        );
    };

    let _ = (request_type, is_method_safe);
    let mut context_redirect_type = redirect_type_for_track(
        original_request_url,
        current_request_url,
        site_basis_url,
        false,
    );
    let mut schemeful_redirect_type = redirect_type_for_track(
        original_request_url,
        current_request_url,
        site_basis_url,
        true,
    );

    if let Some(initiator_url) = initiator_url {
        context_redirect_type = merge_redirect_type(
            context_redirect_type,
            redirect_type_for_track(
                original_request_url,
                current_request_url,
                initiator_url,
                false,
            ),
        );
        schemeful_redirect_type = merge_redirect_type(
            schemeful_redirect_type,
            redirect_type_for_track(
                original_request_url,
                current_request_url,
                initiator_url,
                true,
            ),
        );
    }

    NetworkSiteContextMetadata::new(
        NetworkSiteContextTrackMetadata::none().with_redirect_type(context_redirect_type),
        NetworkSiteContextTrackMetadata::none().with_redirect_type(schemeful_redirect_type),
    )
}

fn same_site_context_rank(context: NetworkSameSiteContext) -> u8 {
    match same_site_inclusion_class(context) {
        NetworkSameSiteInclusionClass::Strict => 2,
        NetworkSameSiteInclusionClass::Lax => 1,
        NetworkSameSiteInclusionClass::Cross => 0,
    }
}

fn persist_redirect_downgraded_same_site_context(
    previous: NetworkSameSiteContext,
    recomputed: NetworkSameSiteContext,
) -> NetworkSameSiteContext {
    if same_site_context_rank(previous) < same_site_context_rank(recomputed) {
        previous
    } else {
        recomputed
    }
}

fn merge_redirect_type(
    previous: NetworkSameSiteRedirectType,
    current: NetworkSameSiteRedirectType,
) -> NetworkSameSiteRedirectType {
    use NetworkSameSiteRedirectType::*;

    fn rank(value: NetworkSameSiteRedirectType) -> u8 {
        match value {
            CrossSiteRedirect => 4,
            PartialSameSiteRedirect => 3,
            AllSameSiteRedirect => 2,
            NoRedirect => 1,
            Unset => 0,
        }
    }

    if rank(previous) >= rank(current) {
        previous
    } else {
        current
    }
}

/// Advances request context after a redirect while preserving prior downgrades.
pub fn advance_cookie_request_context(
    previous_context: NetworkCookieRequestContext,
    original_request_url: &Url,
    current_request_url: &Url,
) -> NetworkCookieRequestContext {
    let recomputed = previous_context
        .clone()
        .recompute_site_context_for_request(current_request_url);
    let site_context = NetworkSiteContext::new(
        persist_redirect_downgraded_same_site_context(
            previous_context.site_context.context,
            recomputed.site_context.context,
        ),
        persist_redirect_downgraded_same_site_context(
            previous_context.site_context.schemeful_context,
            recomputed.site_context.schemeful_context,
        ),
    );
    let site_context = if let Some(site_for_cookies_url) = previous_context
        .browser_context
        .site_for_cookies_url
        .as_ref()
    {
        NetworkSiteContext::new(
            if current_request_is_cross_site_to_explicit_site_for_cookies(
                current_request_url,
                site_for_cookies_url,
                false,
            ) {
                NetworkSameSiteContext::CrossSite
            } else {
                site_context.context
            },
            if current_request_is_cross_site_to_explicit_site_for_cookies(
                current_request_url,
                site_for_cookies_url,
                true,
            ) {
                NetworkSameSiteContext::CrossSite
            } else {
                site_context.schemeful_context
            },
        )
    } else {
        site_context
    };
    let redirect_types = redirect_types_for_request(
        original_request_url,
        current_request_url,
        previous_context.initiator_url.as_ref(),
        &previous_context.browser_context,
        previous_context.request_type,
        previous_context.is_method_safe,
    );
    let context_downgrade_type =
        site_context_downgrade_type(previous_context.site_context.context, site_context.context)
            .or(previous_context
                .site_context_metadata
                .context
                .downgrade_type);
    let schemeful_context_downgrade_type = site_context_downgrade_type(
        previous_context.site_context.schemeful_context,
        site_context.schemeful_context,
    )
    .or(previous_context
        .site_context_metadata
        .schemeful_context
        .downgrade_type);

    recomputed
        .with_site_context(site_context)
        .with_site_context_metadata(NetworkSiteContextMetadata::new(
            NetworkSiteContextTrackMetadata::new(
                previous_context
                    .site_context_metadata
                    .context
                    .downgraded_by_cross_site_redirect
                    || context_downgrade_type.is_some(),
                context_downgrade_type,
            )
            .with_http_method(previous_context.site_context_metadata.context.http_method)
            .with_redirect_type(merge_redirect_type(
                previous_context.site_context_metadata.context.redirect_type,
                redirect_types.context.redirect_type,
            )),
            NetworkSiteContextTrackMetadata::new(
                previous_context
                    .site_context_metadata
                    .schemeful_context
                    .downgraded_by_cross_site_redirect
                    || schemeful_context_downgrade_type.is_some(),
                schemeful_context_downgrade_type,
            )
            .with_http_method(
                previous_context
                    .site_context_metadata
                    .schemeful_context
                    .http_method,
            )
            .with_redirect_type(merge_redirect_type(
                previous_context
                    .site_context_metadata
                    .schemeful_context
                    .redirect_type,
                redirect_types.schemeful_context.redirect_type,
            )),
        ))
}

fn current_request_is_cross_site_to_explicit_site_for_cookies(
    request_url: &Url,
    site_for_cookies_url: &Url,
    schemeful: bool,
) -> bool {
    matches!(
        approximate_same_site_context(
            request_url,
            site_for_cookies_url,
            schemeful,
            NetworkCookieRequestType::Subresource,
            true
        ),
        NetworkSameSiteContext::CrossSite
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkSameSiteInclusionClass {
    Strict,
    Lax,
    Cross,
}

fn same_site_inclusion_class(context: NetworkSameSiteContext) -> NetworkSameSiteInclusionClass {
    match context {
        NetworkSameSiteContext::SameSiteStrict => NetworkSameSiteInclusionClass::Strict,
        NetworkSameSiteContext::SameSiteLax => NetworkSameSiteInclusionClass::Lax,
        NetworkSameSiteContext::SameSiteLaxMethodUnsafe | NetworkSameSiteContext::CrossSite => {
            NetworkSameSiteInclusionClass::Cross
        }
    }
}

/// Converts Moli SameSite metadata into the canonical store type.
pub fn core_same_site_context_metadata_from_stored(
    metadata: NetworkSiteContextMetadata,
) -> SameSiteContextMetadata {
    SameSiteContextMetadata::new(
        core_same_site_context_track_metadata_from_stored(metadata.context),
        core_same_site_context_track_metadata_from_stored(metadata.schemeful_context),
    )
}

fn core_same_site_context_track_metadata_from_stored(
    metadata: NetworkSiteContextTrackMetadata,
) -> CoreSameSiteContextTrackMetadata {
    CoreSameSiteContextTrackMetadata::new(
        metadata.downgraded_by_cross_site_redirect,
        metadata
            .downgrade_type
            .map(|downgrade_type| match downgrade_type {
                NetworkSameSiteContextDowngradeType::StrictToLax => {
                    CoreSameSiteContextDowngradeType::StrictToLax
                }
                NetworkSameSiteContextDowngradeType::StrictToCross => {
                    CoreSameSiteContextDowngradeType::StrictToCross
                }
                NetworkSameSiteContextDowngradeType::LaxToCross => {
                    CoreSameSiteContextDowngradeType::LaxToCross
                }
            }),
    )
    .with_http_method(match metadata.http_method {
        NetworkSameSiteHttpMethod::Unset => CoreSameSiteContextHttpMethod::Unset,
        NetworkSameSiteHttpMethod::Unknown => CoreSameSiteContextHttpMethod::Unknown,
        NetworkSameSiteHttpMethod::Get => CoreSameSiteContextHttpMethod::Get,
        NetworkSameSiteHttpMethod::Head => CoreSameSiteContextHttpMethod::Head,
        NetworkSameSiteHttpMethod::Post => CoreSameSiteContextHttpMethod::Post,
        NetworkSameSiteHttpMethod::Put => CoreSameSiteContextHttpMethod::Put,
        NetworkSameSiteHttpMethod::Delete => CoreSameSiteContextHttpMethod::Delete,
        NetworkSameSiteHttpMethod::Connect => CoreSameSiteContextHttpMethod::Connect,
        NetworkSameSiteHttpMethod::Options => CoreSameSiteContextHttpMethod::Options,
        NetworkSameSiteHttpMethod::Trace => CoreSameSiteContextHttpMethod::Trace,
        NetworkSameSiteHttpMethod::Patch => CoreSameSiteContextHttpMethod::Patch,
    })
    .with_redirect_type(match metadata.redirect_type {
        NetworkSameSiteRedirectType::Unset => CoreSameSiteContextRedirectType::Unset,
        NetworkSameSiteRedirectType::NoRedirect => CoreSameSiteContextRedirectType::NoRedirect,
        NetworkSameSiteRedirectType::CrossSiteRedirect => {
            CoreSameSiteContextRedirectType::CrossSiteRedirect
        }
        NetworkSameSiteRedirectType::PartialSameSiteRedirect => {
            CoreSameSiteContextRedirectType::PartialSameSiteRedirect
        }
        NetworkSameSiteRedirectType::AllSameSiteRedirect => {
            CoreSameSiteContextRedirectType::AllSameSiteRedirect
        }
    })
}

/// Converts facade browser-site context into the canonical store type.
pub fn core_browser_site_context_from_facade(
    context: &NetworkBrowserSiteContext,
) -> CoreBrowserSiteContext {
    CoreBrowserSiteContext {
        site_for_cookies_url: context.site_for_cookies_url.clone(),
        top_frame_origin_url: context.top_frame_origin_url.clone(),
        storage_access_status: match context.storage_access_status {
            NetworkStorageAccessStatus::None => CoreStorageAccessStatus::None,
            NetworkStorageAccessStatus::Granted => CoreStorageAccessStatus::Granted,
        },
        cookie_partition_key: context
            .cookie_partition_key
            .as_ref()
            .map(core_cookie_partition_key_from_stored),
    }
}

pub(crate) fn cookie_partition_key_for_url(
    context: &NetworkBrowserSiteContext,
    url: &Url,
) -> StoredCookiePartitionKey {
    if let Some(key) = context.cookie_partition_key.as_ref() {
        return key.clone();
    }

    let basis = context.site_basis_url().unwrap_or(url);
    StoredCookiePartitionKey::site(site_for_url(basis), false)
}

pub(crate) fn core_cookie_partition_key_for_url(
    context: &NetworkBrowserSiteContext,
    url: &Url,
) -> CoreCookiePartitionKey {
    core_cookie_partition_key_from_stored(&cookie_partition_key_for_url(context, url))
}

fn cookie_partition_key_from_storage_key(
    storage_key: &MoliStorageKey,
) -> Option<StoredCookiePartitionKey> {
    if storage_key.top_level_site() == "null" {
        return storage_key.opaque_nonce().map(|nonce| {
            StoredCookiePartitionKey::opaque(nonce.get(), storage_key.has_cross_site_ancestor())
        });
    }
    Some(StoredCookiePartitionKey::site(
        storage_key.top_level_site().to_owned(),
        storage_key.has_cross_site_ancestor(),
    ))
}

fn core_cookie_partition_key_from_stored(key: &StoredCookiePartitionKey) -> CoreCookiePartitionKey {
    match key {
        StoredCookiePartitionKey::Site {
            top_level_site,
            has_cross_site_ancestor,
        } => CoreCookiePartitionKey::site(top_level_site.clone(), *has_cross_site_ancestor),
        StoredCookiePartitionKey::Opaque {
            nonce,
            has_cross_site_ancestor,
        } => CoreCookiePartitionKey::opaque(*nonce, *has_cross_site_ancestor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(url: &str) -> Url {
        Url::parse(url).unwrap()
    }

    #[test]
    fn redirect_chain_keeps_subresource_request_cross_site_when_original_hop_was_cross_site() {
        let initiator_url = parse("https://example.com/index.html");
        let original_request_url = parse("https://other.test/start");
        let current_request_url = parse("https://example.com/final");

        let initial = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata.context.http_method,
            NetworkSameSiteHttpMethod::Get
        );
    }

    #[test]
    fn redirect_chain_downgrades_top_level_safe_request_to_lax_like_chromium() {
        let initiator_url = parse("https://example.com/index.html");
        let original_request_url = parse("https://other.test/start");
        let current_request_url = parse("https://example.com/final");

        let initial = NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
    }

    #[test]
    fn redirect_chain_downgrades_top_level_unsafe_request_to_lax_unsafe_like_chromium() {
        let initiator_url = parse("https://example.com/index.html");
        let original_request_url = parse("https://other.test/start");
        let current_request_url = parse("https://example.com/final");

        let initial = NetworkCookieRequestContext::top_level_navigation("POST")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata.context.http_method,
            NetworkSameSiteHttpMethod::Post
        );
    }

    #[test]
    fn redirect_chain_reports_all_same_site_when_all_hops_match_initiator() {
        let initiator_url = parse("https://www.example.com/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://api.example.com/final");

        let initial = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::SameSiteStrict
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::SameSiteStrict
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::AllSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::AllSameSiteRedirect
        );
        assert!(
            !advanced
                .site_context_metadata
                .context
                .downgraded_by_cross_site_redirect
        );
        assert!(
            !advanced
                .site_context_metadata
                .schemeful_context
                .downgraded_by_cross_site_redirect
        );
    }

    #[test]
    fn redirect_chain_reports_cross_site_when_initiator_is_cross_site_for_all_hops() {
        let initiator_url = parse("https://other.test/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://api.example.com/final");

        let initial = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert!(
            !advanced
                .site_context_metadata
                .context
                .downgraded_by_cross_site_redirect
        );
        assert!(
            !advanced
                .site_context_metadata
                .schemeful_context
                .downgraded_by_cross_site_redirect
        );
    }

    #[test]
    fn redirect_chain_keeps_top_level_safe_request_lax_when_site_for_cookies_is_same_site_but_initiator_is_cross_site()
     {
        let initiator_url = parse("https://other.test/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://api.example.com/final");
        let same_site_frame_url = parse("https://www.example.com/frame.html");

        let initial = NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&original_request_url, &initiator_url)
            .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
            .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
    }

    #[test]
    fn redirect_chain_keeps_top_level_unsafe_request_lax_unsafe_when_site_for_cookies_is_same_site_but_initiator_is_cross_site()
     {
        let initiator_url = parse("https://other.test/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://api.example.com/final");
        let same_site_frame_url = parse("https://www.example.com/frame.html");

        let initial = NetworkCookieRequestContext::top_level_navigation("POST")
            .with_initiator_url(&original_request_url, &initiator_url)
            .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
            .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata.context.http_method,
            NetworkSameSiteHttpMethod::Post
        );
    }

    #[test]
    fn redirect_chain_keeps_top_level_request_cross_site_when_final_url_is_cross_site_to_site_for_cookies()
     {
        let initiator_url = parse("https://app.example.com/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://other.test/final");
        let same_site_frame_url = parse("https://www.example.com/frame.html");

        let initial = NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&original_request_url, &initiator_url)
            .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
            .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
        let advanced =
            advance_cookie_request_context(initial, &original_request_url, &current_request_url);

        assert_eq!(
            advanced.site_context.context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context.schemeful_context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context_metadata.context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
    }

    #[test]
    fn cross_site_request_does_not_turn_current_resource_into_cross_site_ancestor() {
        let request_url = parse("https://widget.example/resource");
        let top_level_url = parse("https://top.example/page");
        let context = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &top_level_url);

        assert_eq!(
            cookie_partition_key_for_url(&context.browser_context, &request_url),
            StoredCookiePartitionKey::site("https://top.example".to_owned(), false)
        );
    }

    #[test]
    fn explicit_ancestor_chain_bit_survives_cross_site_resource_lookup() {
        let request_url = parse("https://widget.example/resource");
        let context = NetworkBrowserSiteContext::default().with_cookie_partition_key(
            StoredCookiePartitionKey::site("https://top.example".to_owned(), true),
        );

        assert_eq!(
            cookie_partition_key_for_url(&context, &request_url),
            StoredCookiePartitionKey::site("https://top.example".to_owned(), true)
        );
    }
}

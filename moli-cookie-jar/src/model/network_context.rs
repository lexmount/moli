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
    Cdp,
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
    fn without_method(self) -> Self {
        let neutral = |value| match value {
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe => NetworkSameSiteContext::SameSiteLax,
            other => other,
        };
        Self::new(neutral(self.context), neutral(self.schemeful_context))
    }

    fn for_method(self, safe: bool) -> Self {
        let outcome = |value| match value {
            NetworkSameSiteContext::SameSiteLax if !safe => {
                NetworkSameSiteContext::SameSiteLaxMethodUnsafe
            }
            other => other,
        };
        Self::new(outcome(self.context), outcome(self.schemeful_context))
    }

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

    fn restricted_to(self, other: Self) -> Self {
        Self::new(
            persist_redirect_downgraded_same_site_context(self.context, other.context),
            persist_redirect_downgraded_same_site_context(
                self.schemeful_context,
                other.schemeful_context,
            ),
        )
    }
}

/// Request shape relevant to SameSite Lax handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkCookieRequestType {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SiteHistoryTrack {
    downgraded_by_redirect: bool,
    downgrade: Option<NetworkSameSiteContextDowngradeType>,
    redirect_type: NetworkSameSiteRedirectType,
}

impl SiteHistoryTrack {
    fn initial() -> Self {
        Self {
            downgraded_by_redirect: false,
            downgrade: None,
            redirect_type: NetworkSameSiteRedirectType::NoRedirect,
        }
    }

    fn from_snapshot(
        snapshot: NetworkSiteContextTrackMetadata,
        method: NetworkSameSiteHttpMethod,
        site: NetworkSameSiteContext,
    ) -> Self {
        use NetworkSameSiteContextDowngradeType::{LaxToCross, StrictToCross, StrictToLax};
        let downgrade = if site == NetworkSameSiteContext::SameSiteLax && !method.is_safe() {
            match snapshot.downgrade_type {
                Some(StrictToCross) => Some(StrictToLax),
                Some(LaxToCross) => None,
                other => other,
            }
        } else {
            snapshot.downgrade_type
        };
        Self {
            downgraded_by_redirect: snapshot.downgraded_by_cross_site_redirect,
            downgrade,
            redirect_type: snapshot.redirect_type,
        }
    }

    fn advance(
        &mut self,
        previous: NetworkSameSiteContext,
        current: NetworkSameSiteContext,
        method: NetworkSameSiteHttpMethod,
        redirect_type: NetworkSameSiteRedirectType,
    ) {
        use NetworkSameSiteContext::{CrossSite, SameSiteLax, SameSiteStrict};
        use NetworkSameSiteContextDowngradeType::{LaxToCross, StrictToCross, StrictToLax};
        let downgrade = match (previous, current) {
            (SameSiteStrict, SameSiteLax) => Some(StrictToLax),
            (SameSiteStrict, CrossSite) => Some(StrictToCross),
            (SameSiteLax, CrossSite) if method.is_safe() => Some(LaxToCross),
            _ => None,
        };
        self.downgrade = downgrade.or(self.downgrade);
        self.downgraded_by_redirect |= self.downgrade.is_some();
        self.redirect_type = merge_redirect_type(self.redirect_type, redirect_type);
    }

    fn snapshot(
        self,
        method: NetworkSameSiteHttpMethod,
        site: NetworkSameSiteContext,
    ) -> NetworkSiteContextTrackMetadata {
        use NetworkSameSiteContextDowngradeType::{StrictToCross, StrictToLax};
        let downgrade = if site == NetworkSameSiteContext::SameSiteLax
            && !method.is_safe()
            && self.downgrade == Some(StrictToLax)
        {
            Some(StrictToCross)
        } else {
            self.downgrade
        };
        NetworkSiteContextTrackMetadata::new(self.downgraded_by_redirect, downgrade)
            .with_http_method(method)
            .with_redirect_type(self.redirect_type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CookieSiteHistory {
    /// None until a policy or a followed redirect establishes a site limit.
    /// These limits never include a restriction from the HTTP method.
    limits: Option<NetworkSiteContext>,
    context: SiteHistoryTrack,
    schemeful_context: SiteHistoryTrack,
}

impl CookieSiteHistory {
    fn initial() -> Self {
        Self {
            limits: None,
            context: SiteHistoryTrack::initial(),
            schemeful_context: SiteHistoryTrack::initial(),
        }
    }
}

/// Request inputs and method-independent site history used by cookie policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkCookieRequestContext {
    /// Current site relation, before applying the current HTTP method.
    site: NetworkSiteContext,
    method: NetworkSameSiteHttpMethod,
    history: CookieSiteHistory,
    request_type: NetworkCookieRequestType,
    pub initiator_url: Option<Url>,
    pub browser_context: NetworkBrowserSiteContext,
}

impl NetworkCookieRequestContext {
    fn new(method: &str, request_type: NetworkCookieRequestType) -> Self {
        Self {
            site: NetworkSiteContext::same_site(),
            method: parse_network_same_site_http_method(method),
            history: CookieSiteHistory::initial(),
            request_type,
            initiator_url: None,
            browser_context: NetworkBrowserSiteContext::default(),
        }
    }

    pub fn subresource(method: &str) -> Self {
        Self::new(method, NetworkCookieRequestType::Subresource)
    }

    pub fn top_level_navigation(method: &str) -> Self {
        Self::new(method, NetworkCookieRequestType::TopLevelNavigation)
    }

    /// Computes this hop's result from the site relation and current method.
    pub fn site_context(&self) -> NetworkSiteContext {
        self.site.for_method(self.method.is_safe())
    }

    /// Materializes diagnostic output; HTTP method is stored only once.
    pub fn site_context_metadata(&self) -> NetworkSiteContextMetadata {
        NetworkSiteContextMetadata::new(
            self.history
                .context
                .snapshot(self.method, self.site.context),
            self.history
                .schemeful_context
                .snapshot(self.method, self.site.schemeful_context),
        )
    }

    pub fn is_method_safe(&self) -> bool {
        self.method.is_safe()
    }

    /// Initializes an initiator and its default browser cookie scope.
    pub fn with_initiator_url(mut self, request_url: &Url, initiator_url: &Url) -> Self {
        self.browser_context = self
            .browser_context
            .with_site_for_cookies_url(initiator_url)
            .with_top_frame_origin_url(initiator_url);
        if self.browser_context.cookie_partition_key.is_none() {
            self.browser_context.cookie_partition_key = Some(StoredCookiePartitionKey::site(
                site_for_url(initiator_url),
                false,
            ));
        }
        self.with_cookie_initiator_origin(request_url, initiator_url)
    }

    /// Updates the cookie initiator without changing frame scope or partition.
    pub fn with_cookie_initiator_origin(mut self, request_url: &Url, origin: &Url) -> Self {
        self.initiator_url = Some(origin.clone());
        self.recompute_site_context_for_request(request_url)
    }

    pub fn set_subresource_request_type(&mut self) {
        self.request_type = NetworkCookieRequestType::Subresource;
    }

    pub fn is_top_level_navigation(&self) -> bool {
        matches!(
            self.request_type,
            NetworkCookieRequestType::TopLevelNavigation
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_same_site_context(self, same_site_context: NetworkSameSiteContext) -> Self {
        self.with_site_context(NetworkSiteContext::new(
            same_site_context,
            same_site_context,
        ))
    }

    pub fn with_cross_site_context(mut self) -> Self {
        self.history.limits = Some(NetworkSiteContext::cross_site());
        self.replace_site(NetworkSiteContext::cross_site());
        self
    }

    pub fn with_site_context(mut self, site_context: NetworkSiteContext) -> Self {
        self.replace_site(site_context.without_method());
        self
    }

    /// Accepts diagnostic history; current method has a single source of truth.
    pub fn with_site_context_metadata(mut self, metadata: NetworkSiteContextMetadata) -> Self {
        let previous = self.site_context_metadata();
        let metadata = NetworkSiteContextMetadata::new(
            metadata.context.inherit_unset(previous.context),
            metadata
                .schemeful_context
                .inherit_unset(previous.schemeful_context),
        );
        self.method = metadata.context.http_method;
        self.history.context =
            SiteHistoryTrack::from_snapshot(metadata.context, self.method, self.site.context);
        self.history.schemeful_context = SiteHistoryTrack::from_snapshot(
            metadata.schemeful_context,
            self.method,
            self.site.schemeful_context,
        );
        self
    }

    /// Only the method input changes; site history remains method-independent.
    pub fn set_request_method(&mut self, method: &str) {
        self.method = parse_network_same_site_http_method(method);
    }

    pub fn with_site_for_cookies_url(
        mut self,
        request_url: &Url,
        site_for_cookies_url: &Url,
    ) -> Self {
        self.browser_context = self
            .browser_context
            .with_site_for_cookies_url(site_for_cookies_url);
        self.recompute_site_context_for_request(request_url)
    }

    pub fn with_top_frame_origin_url(
        mut self,
        request_url: &Url,
        top_frame_origin_url: &Url,
    ) -> Self {
        self.browser_context = self
            .browser_context
            .with_top_frame_origin_url(top_frame_origin_url);
        self.recompute_site_context_for_request(request_url)
    }

    pub fn with_storage_access_status(mut self, status: NetworkStorageAccessStatus) -> Self {
        self.browser_context = self.browser_context.with_storage_access_status(status);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_site_context_metadata_for_redirects(
        mut self,
        original: &Url,
        current: &Url,
    ) -> Self {
        let metadata = redirect_types_for_request(
            original,
            current,
            self.initiator_url.as_ref(),
            &self.browser_context,
        );
        self.history.context =
            SiteHistoryTrack::from_snapshot(metadata.context, self.method, self.site.context);
        self.history.schemeful_context = SiteHistoryTrack::from_snapshot(
            metadata.schemeful_context,
            self.method,
            self.site.schemeful_context,
        );
        self
    }

    pub fn recompute_site_context_for_request(mut self, request_url: &Url) -> Self {
        let site = self.site_context_for_request(request_url);
        self.replace_site(site);
        self
    }

    fn replace_site(&mut self, site: NetworkSiteContext) {
        let site = match self.history.limits {
            Some(limit) => limit.restricted_to(site),
            None => site,
        };
        // Leaving Lax closes the method-dependent classification of that
        // historical transition. A later method cannot rewrite a real Cross.
        if self.site.context == NetworkSameSiteContext::SameSiteLax
            && site.context != self.site.context
        {
            self.history.context.downgrade = self
                .history
                .context
                .snapshot(self.method, self.site.context)
                .downgrade_type;
        }
        if self.site.schemeful_context == NetworkSameSiteContext::SameSiteLax
            && site.schemeful_context != self.site.schemeful_context
        {
            self.history.schemeful_context.downgrade = self
                .history
                .schemeful_context
                .snapshot(self.method, self.site.schemeful_context)
                .downgrade_type;
        }
        self.site = site;
        if self.history.limits.is_some() {
            self.history.limits = Some(site);
        }
    }

    fn site_context_for_request(&self, request_url: &Url) -> NetworkSiteContext {
        let mut combined = self
            .browser_context
            .site_basis_url()
            .map(|basis| approximate_site_context(request_url, basis, self.request_type));
        if let Some(initiator) = &self.initiator_url {
            let site = approximate_site_context(request_url, initiator, self.request_type);
            combined = Some(match combined {
                Some(previous) => previous.restricted_to(site),
                None => site,
            });
        }
        combined.unwrap_or(self.site)
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
    fn inherit_unset(mut self, previous: Self) -> Self {
        if self.http_method == NetworkSameSiteHttpMethod::Unset {
            self.http_method = previous.http_method;
        }
        if self.redirect_type == NetworkSameSiteRedirectType::Unset {
            self.redirect_type = previous.redirect_type;
        }
        self
    }

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

impl NetworkSameSiteHttpMethod {
    fn is_safe(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options | Self::Trace)
    }
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
) -> NetworkSiteContext {
    NetworkSiteContext::new(
        approximate_same_site_context(request_url, initiator_url, false, request_type),
        approximate_same_site_context(request_url, initiator_url, true, request_type),
    )
}

fn approximate_same_site_context(
    request_url: &Url,
    initiator_url: &Url,
    schemeful: bool,
    request_type: NetworkCookieRequestType,
) -> NetworkSameSiteContext {
    let Some(request_host) = request_url.host_str() else {
        return NetworkSameSiteContext::CrossSite;
    };
    let Some(initiator_host) = initiator_url.host_str() else {
        return NetworkSameSiteContext::CrossSite;
    };
    let same_site = same_site_hosts(request_host, initiator_host)
        && (!schemeful
            || normalized_network_same_site_scheme(request_url.scheme())
                == normalized_network_same_site_scheme(initiator_url.scheme()));
    if same_site {
        NetworkSameSiteContext::SameSiteStrict
    } else if matches!(request_type, NetworkCookieRequestType::TopLevelNavigation) {
        NetworkSameSiteContext::SameSiteLax
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
    // The unsafe navigation allowance can include recent default cookies.
    // An explicit CrossSite restriction must remain stricter across redirects.
    match context {
        NetworkSameSiteContext::SameSiteStrict => 3,
        NetworkSameSiteContext::SameSiteLax => 2,
        NetworkSameSiteContext::SameSiteLaxMethodUnsafe => 1,
        NetworkSameSiteContext::CrossSite => 0,
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

/// Advances a redirect using the new hop's method while preserving site/chain limits.
pub fn advance_cookie_request_context(
    mut previous_context: NetworkCookieRequestContext,
    original_request_url: &Url,
    current_request_url: &Url,
    current_request_method: &str,
) -> NetworkCookieRequestContext {
    previous_context.set_request_method(current_request_method);
    let previous_site = previous_context.site;
    previous_context.history.limits = Some(match previous_context.history.limits {
        Some(limit) => limit.restricted_to(previous_site),
        None => previous_site,
    });
    let mut context = previous_context.recompute_site_context_for_request(current_request_url);
    let redirect_types = redirect_types_for_request(
        original_request_url,
        current_request_url,
        context.initiator_url.as_ref(),
        &context.browser_context,
    );
    context.history.context.advance(
        previous_site.context,
        context.site.context,
        context.method,
        redirect_types.context.redirect_type,
    );
    context.history.schemeful_context.advance(
        previous_site.schemeful_context,
        context.site.schemeful_context,
        context.method,
        redirect_types.schemeful_context.redirect_type,
    );
    context
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
    fn site_history_is_independent_of_method_round_trips() {
        use NetworkSameSiteContext::{SameSiteLax, SameSiteLaxMethodUnsafe};
        use NetworkSameSiteContextDowngradeType::{StrictToCross, StrictToLax};

        let original = parse("https://same.test/start");
        let next = parse("https://other.test/final");
        let initial = NetworkCookieRequestContext::top_level_navigation("POST")
            .with_initiator_url(&original, &original);
        let mut context = advance_cookie_request_context(initial, &original, &next, "POST");
        let history = context.history;
        assert_eq!(
            history.limits,
            Some(NetworkSiteContext::new(SameSiteLax, SameSiteLax))
        );
        for (method, site, downgrade, http_method) in [
            (
                "GET",
                SameSiteLax,
                StrictToLax,
                NetworkSameSiteHttpMethod::Get,
            ),
            (
                "POST",
                SameSiteLaxMethodUnsafe,
                StrictToCross,
                NetworkSameSiteHttpMethod::Post,
            ),
            (
                "HEAD",
                SameSiteLax,
                StrictToLax,
                NetworkSameSiteHttpMethod::Head,
            ),
            (
                "PUT",
                SameSiteLaxMethodUnsafe,
                StrictToCross,
                NetworkSameSiteHttpMethod::Put,
            ),
        ] {
            context.set_request_method(method);
            assert_eq!(context.history, history);
            assert_eq!(context.site_context(), NetworkSiteContext::new(site, site));
            let metadata = context.site_context_metadata();
            for track in [metadata.context, metadata.schemeful_context] {
                assert_eq!(track.downgrade_type, Some(downgrade));
                assert_eq!(track.http_method, http_method);
                assert!(track.downgraded_by_cross_site_redirect);
            }
        }
    }

    #[test]
    fn site_history_retains_diagnostics_when_scope_becomes_cross_site() {
        use NetworkSameSiteContextDowngradeType::{StrictToCross, StrictToLax};

        let original = parse("https://same.test/start");
        let next = parse("https://other.test/final");
        for (initial_method, method, expected_downgrade) in
            [("POST", "GET", StrictToCross), ("GET", "POST", StrictToLax)]
        {
            let initial = NetworkCookieRequestContext::top_level_navigation(initial_method)
                .with_initiator_url(&original, &original);
            let mut context =
                advance_cookie_request_context(initial, &original, &next, initial_method)
                    .with_site_for_cookies_url(&next, &parse("about:blank"));
            context.set_request_method(method);
            assert_eq!(context.site_context(), NetworkSiteContext::cross_site());
            assert_eq!(
                context.history.limits,
                Some(NetworkSiteContext::cross_site())
            );
            let metadata = context.site_context_metadata();
            for track in [metadata.context, metadata.schemeful_context] {
                assert_eq!(track.downgrade_type, Some(expected_downgrade));
                assert!(track.downgraded_by_cross_site_redirect);
            }
        }
    }

    #[test]
    fn cookie_initiator_update_preserves_browser_scope() {
        let target = parse("https://example.test/data");
        let origin = parse("https://example.test/");
        let mut context = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&target, &parse("about:blank"));
        let scope = NetworkBrowserSiteContext::default()
            .with_site_for_cookies_url(&origin)
            .with_top_frame_origin_url(&origin)
            .with_storage_access_status(NetworkStorageAccessStatus::Granted)
            .with_cookie_partition_key(StoredCookiePartitionKey::site(
                "https://partition.test".to_owned(),
                true,
            ));
        context.browser_context = scope.clone();
        let updated = context.with_cookie_initiator_origin(&target, &origin);
        assert_eq!(updated.browser_context, scope);
        assert_eq!(updated.initiator_url.as_ref(), Some(&origin));
        assert_eq!(updated.site_context(), NetworkSiteContext::same_site());
    }

    #[test]
    fn redirect_advancement_synchronizes_method_before_preserving_site_limits() {
        use NetworkSameSiteContext::{SameSiteLax, SameSiteLaxMethodUnsafe};

        let initiator = parse("https://example.test/page");
        let original = parse("https://other.test/start");
        let next = parse("https://other.test/final");
        for (initial_method, method, expected_site, expected_http_method) in [
            ("POST", "GET", SameSiteLax, NetworkSameSiteHttpMethod::Get),
            (
                "GET",
                "POST",
                SameSiteLaxMethodUnsafe,
                NetworkSameSiteHttpMethod::Post,
            ),
        ] {
            for cross_site in [false, true] {
                let mut initial = NetworkCookieRequestContext::top_level_navigation(initial_method)
                    .with_initiator_url(&original, &initiator);
                if cross_site {
                    initial = initial.with_cross_site_context();
                }
                let advanced = advance_cookie_request_context(initial, &original, &next, method);
                let expected_site = if cross_site {
                    NetworkSiteContext::cross_site()
                } else {
                    NetworkSiteContext::new(expected_site, expected_site)
                };
                assert_eq!(advanced.site_context(), expected_site);
                assert_eq!(advanced.is_method_safe(), method == "GET");
                for metadata in [
                    advanced.site_context_metadata().context,
                    advanced.site_context_metadata().schemeful_context,
                ] {
                    assert_eq!(metadata.http_method, expected_http_method);
                }
            }
        }
    }

    #[test]
    fn redirect_chain_keeps_subresource_request_cross_site_when_original_hop_was_cross_site() {
        let initiator_url = parse("https://example.com/index.html");
        let original_request_url = parse("https://other.test/start");
        let current_request_url = parse("https://example.com/final");

        let initial = NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&original_request_url, &initiator_url);
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata().context.http_method,
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "POST",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::PartialSameSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata().context.http_method,
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteStrict
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteStrict
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::AllSameSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::AllSameSiteRedirect
        );
        assert!(
            !advanced
                .site_context_metadata()
                .context
                .downgraded_by_cross_site_redirect
        );
        assert!(
            !advanced
                .site_context_metadata()
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::CrossSite
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert!(
            !advanced
                .site_context_metadata()
                .context
                .downgraded_by_cross_site_redirect
        );
        assert!(
            !advanced
                .site_context_metadata()
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
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
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "POST",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
                .schemeful_context
                .redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced.site_context_metadata().context.http_method,
            NetworkSameSiteHttpMethod::Post
        );
    }

    #[test]
    fn redirect_chain_keeps_top_level_request_lax_when_final_url_is_cross_site_to_site_for_cookies()
    {
        let initiator_url = parse("https://app.example.com/index.html");
        let original_request_url = parse("https://cdn.example.com/start");
        let current_request_url = parse("https://other.test/final");
        let same_site_frame_url = parse("https://www.example.com/frame.html");

        let initial = NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&original_request_url, &initiator_url)
            .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
            .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
        let advanced = advance_cookie_request_context(
            initial,
            &original_request_url,
            &current_request_url,
            "GET",
        );

        assert_eq!(
            advanced.site_context().context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context().schemeful_context,
            NetworkSameSiteContext::SameSiteLax
        );
        assert_eq!(
            advanced.site_context_metadata().context.redirect_type,
            NetworkSameSiteRedirectType::CrossSiteRedirect
        );
        assert_eq!(
            advanced
                .site_context_metadata()
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

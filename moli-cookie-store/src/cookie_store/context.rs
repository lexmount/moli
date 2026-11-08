use crate::CookiePartitionKey;
use url::Url;

/// The logical browser/API source performing a cookie read or write.
///
/// This is intentionally separate from the URL scheme. For example, Chromium's
/// `CookieOptions` and Servo's `CookieSource` both distinguish "non-HTTP API"
/// from "HTTP response/request" even when the URL itself is secure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieAccessSource {
    /// Network-driven cookie access such as an HTTP request/response path.
    Http,
    /// Script-driven access such as `document.cookie`.
    Document,
    /// Privileged browser-side access such as DevTools/CDP.
    Cdp,
}

/// Browser-side request context that accompanies a cookie read or write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSiteContext {
    /// Browser-computed site-for-cookies basis URL, when available.
    pub site_for_cookies_url: Option<Url>,
    /// Browser-computed top-frame origin, when available.
    pub top_frame_origin_url: Option<Url>,
    /// Storage-access state associated with the current browser context.
    pub storage_access_status: StorageAccessStatus,
    /// Browser-computed CHIPS key for the current browsing context.
    pub cookie_partition_key: Option<CookiePartitionKey>,
}

impl BrowserSiteContext {
    /// Construct an empty browser-side site context.
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Browser-side storage access state associated with a cookie query/write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StorageAccessStatus {
    /// No storage-access grant is currently attached to this browser context.
    #[default]
    None,
    /// The browser context has an explicit storage-access grant.
    Granted,
}

/// Context used when inserting a cookie into the store.
#[derive(Debug, Clone)]
pub struct InsertContext<'a> {
    /// The request/source URL associated with the write.
    pub url: &'a Url,
    /// The logical API source performing the write.
    pub source: CookieAccessSource,
    /// Additional browser-side context associated with this write.
    pub browser_context: BrowserSiteContext,
    /// SameSite relationship used to decide whether a cookie may be created.
    pub same_site_context: SameSiteContext,
    /// Top-level HTTP responses may create SameSite cookies across sites.
    pub request_type: HttpRequestType,
    pub(super) enforce_browser_policy: bool,
}

impl<'a> InsertContext<'a> {
    /// Construct a context for network-driven cookie writes.
    pub fn http(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Http,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            request_type: HttpRequestType::Subresource,
            enforce_browser_policy: true,
        }
    }

    /// Construct a context for script-driven writes such as `document.cookie`.
    pub fn document(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Document,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            request_type: HttpRequestType::Subresource,
            enforce_browser_policy: true,
        }
    }

    /// Construct a context for privileged browser-side writes such as CDP.
    pub fn cdp(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Cdp,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            request_type: HttpRequestType::Subresource,
            enforce_browser_policy: true,
        }
    }
}

/// SameSite relationship information carried by a cookie query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SameSiteContext {
    /// The schemeless same-site relationship.
    pub context: SameSiteRequestContext,
    /// The schemeful same-site relationship.
    pub schemeful_context: SameSiteRequestContext,
}

/// The redirect downgrade class that produced the current SameSite context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSiteContextDowngradeType {
    /// A redirect chain downgraded the request from a strict same-site context
    /// to a lax one.
    StrictToLax,
    /// A redirect chain downgraded the request from a strict same-site context
    /// to a fully cross-site one.
    StrictToCross,
    /// A redirect chain downgraded the request from a lax context to a fully
    /// cross-site one.
    LaxToCross,
}

/// HTTP method metadata attached to a SameSite context track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SameSiteContextHttpMethod {
    /// The method is not applicable for this context.
    #[default]
    Unset,
    /// A method existed but could not be mapped to a stable bucket.
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

/// Redirect classification metadata attached to a SameSite context track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SameSiteContextRedirectType {
    /// The redirect classification is not applicable for this context.
    #[default]
    Unset,
    /// No redirect occurred before this access.
    NoRedirect,
    /// A redirect occurred and the overall chain is treated as cross-site.
    CrossSiteRedirect,
    /// A redirect occurred and only part of the chain stayed same-site.
    PartialSameSiteRedirect,
    /// A redirect occurred but all observed hops stayed same-site.
    AllSameSiteRedirect,
}

/// Additional metadata describing how the current SameSite context was reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SameSiteContextTrackMetadata {
    /// Whether a redirect chain downgraded the request from same-site to
    /// cross-site before this access.
    pub downgraded_by_cross_site_redirect: bool,
    /// The downgrade class, when a redirect caused one.
    pub downgrade_type: Option<SameSiteContextDowngradeType>,
    /// The HTTP method associated with this access.
    pub http_method: SameSiteContextHttpMethod,
    /// The redirect classification associated with this access.
    pub redirect_type: SameSiteContextRedirectType,
}

impl SameSiteContextTrackMetadata {
    /// Construct track-local SameSite metadata.
    pub const fn new(
        downgraded_by_cross_site_redirect: bool,
        downgrade_type: Option<SameSiteContextDowngradeType>,
    ) -> Self {
        Self {
            downgraded_by_cross_site_redirect,
            downgrade_type,
            http_method: SameSiteContextHttpMethod::Unset,
            redirect_type: SameSiteContextRedirectType::Unset,
        }
    }

    /// Return metadata with no recorded redirect downgrade.
    pub const fn none() -> Self {
        Self::new(false, None)
    }

    /// Attach a specific redirect downgrade class to this metadata.
    pub const fn with_downgrade_type(
        mut self,
        downgrade_type: SameSiteContextDowngradeType,
    ) -> Self {
        self.downgraded_by_cross_site_redirect = true;
        self.downgrade_type = Some(downgrade_type);
        self
    }

    /// Attach the HTTP method associated with this SameSite context track.
    pub const fn with_http_method(mut self, http_method: SameSiteContextHttpMethod) -> Self {
        self.http_method = http_method;
        self
    }

    /// Attach the redirect classification associated with this track.
    pub const fn with_redirect_type(mut self, redirect_type: SameSiteContextRedirectType) -> Self {
        self.redirect_type = redirect_type;
        self
    }
}

/// Metadata attached to both schemeless and schemeful SameSite context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SameSiteContextMetadata {
    /// Metadata for the schemeless context track.
    pub context: SameSiteContextTrackMetadata,
    /// Metadata for the schemeful context track.
    pub schemeful_context: SameSiteContextTrackMetadata,
}

impl SameSiteContextMetadata {
    /// Construct SameSite metadata for both schemeless and schemeful tracks.
    pub const fn new(
        context: SameSiteContextTrackMetadata,
        schemeful_context: SameSiteContextTrackMetadata,
    ) -> Self {
        Self {
            context,
            schemeful_context,
        }
    }

    /// Return metadata with no recorded redirect downgrade on either track.
    pub const fn none() -> Self {
        Self::new(
            SameSiteContextTrackMetadata::none(),
            SameSiteContextTrackMetadata::none(),
        )
    }

    /// Construct metadata that only records a schemeful downgrade.
    pub const fn schemeful_only(
        downgraded_by_cross_site_redirect: bool,
        downgrade_type: Option<SameSiteContextDowngradeType>,
    ) -> Self {
        Self::new(
            SameSiteContextTrackMetadata::none(),
            SameSiteContextTrackMetadata::new(downgraded_by_cross_site_redirect, downgrade_type),
        )
    }
}

impl SameSiteContext {
    /// Construct a context that is strict same-site under both schemeless and
    /// schemeful evaluation.
    pub const fn same_site() -> Self {
        Self::new(
            SameSiteRequestContext::SameSiteStrict,
            SameSiteRequestContext::SameSiteStrict,
        )
    }

    /// Construct a context that is cross-site under both schemeless and
    /// schemeful evaluation.
    pub const fn cross_site() -> Self {
        Self::new(
            SameSiteRequestContext::CrossSite,
            SameSiteRequestContext::CrossSite,
        )
    }

    /// Construct a context with explicit schemeless and schemeful relations.
    pub const fn new(
        context: SameSiteRequestContext,
        schemeful_context: SameSiteRequestContext,
    ) -> Self {
        Self {
            context,
            schemeful_context,
        }
    }

    /// Return the SameSite relation currently used for request inclusion.
    pub const fn for_inclusion(self) -> SameSiteRequestContext {
        self.schemeful_context
    }
}

/// Context used when querying cookies from the store.
#[derive(Debug, Clone)]
pub struct QueryContext<'a> {
    /// The URL being queried.
    pub url: &'a Url,
    /// The logical API source performing the read.
    pub source: CookieAccessSource,
    /// Additional browser-side context associated with this query.
    pub browser_context: BrowserSiteContext,
    /// SameSite relationship information for this query.
    pub same_site_context: SameSiteContext,
    /// Additional metadata describing how the current SameSite context was
    /// reached.
    pub same_site_context_metadata: SameSiteContextMetadata,
    /// The broad HTTP request shape being modeled.
    pub request_type: HttpRequestType,
    /// Whether the HTTP method is "safe" for SameSite purposes.
    pub is_method_safe: bool,
    /// The concrete HTTP method classification associated with this query.
    pub http_method: SameSiteContextHttpMethod,
    /// The redirect classification associated with this query.
    pub redirect_type: SameSiteContextRedirectType,
    /// Whether `HttpOnly` cookies should be included in the result set.
    pub include_http_only: bool,
    /// Whether matched cookies should have their access time updated.
    pub update_access_time: bool,
    /// Whether excluded cookies should be returned alongside included ones.
    pub return_excluded_cookies: bool,
}

/// The SameSite relationship of the current cookie query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSiteRequestContext {
    /// A strict same-site request. `SameSite=Strict` and `SameSite=Lax` are
    /// both eligible for inclusion.
    SameSiteStrict,
    /// A cross-site top-level navigation with a safe method. `SameSite=Lax`
    /// remains eligible, while `SameSite=Strict` does not.
    SameSiteLax,
    /// A top-level navigation that only has Chromium's weaker
    /// `Lax-allow-unsafe` shape available.
    SameSiteLaxMethodUnsafe,
    /// A fully cross-site request.
    CrossSite,
}

/// The high-level HTTP request type used for SameSite query decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpRequestType {
    /// A subresource request such as fetch/XHR/image/script loading.
    Subresource,
    /// A top-level navigation request.
    TopLevelNavigation,
}

impl<'a> QueryContext<'a> {
    /// Construct a context for network request reads.
    pub fn http(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Http,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            same_site_context_metadata: SameSiteContextMetadata::none(),
            request_type: HttpRequestType::Subresource,
            is_method_safe: true,
            http_method: SameSiteContextHttpMethod::Get,
            redirect_type: SameSiteContextRedirectType::NoRedirect,
            include_http_only: true,
            update_access_time: true,
            return_excluded_cookies: true,
        }
    }

    /// Construct a context for cross-site network request reads.
    pub fn http_cross_site(url: &'a Url) -> Self {
        Self {
            same_site_context: SameSiteContext::cross_site(),
            ..Self::http(url)
        }
    }

    /// Construct a context for cross-site top-level navigation reads with a
    /// safe method.
    pub fn http_cross_site_top_level(url: &'a Url) -> Self {
        Self {
            same_site_context: SameSiteContext::new(
                SameSiteRequestContext::SameSiteLax,
                SameSiteRequestContext::SameSiteLax,
            ),
            request_type: HttpRequestType::TopLevelNavigation,
            is_method_safe: true,
            ..Self::http(url)
        }
    }

    /// Construct a context for cross-site top-level navigation reads with an
    /// unsafe method.
    pub fn http_cross_site_top_level_unsafe(url: &'a Url) -> Self {
        Self {
            same_site_context: SameSiteContext::new(
                SameSiteRequestContext::SameSiteLaxMethodUnsafe,
                SameSiteRequestContext::SameSiteLaxMethodUnsafe,
            ),
            request_type: HttpRequestType::TopLevelNavigation,
            is_method_safe: false,
            http_method: SameSiteContextHttpMethod::Post,
            ..Self::http(url)
        }
    }

    /// Construct a context for script reads such as `document.cookie`.
    pub fn document(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Document,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            same_site_context_metadata: SameSiteContextMetadata::none(),
            request_type: HttpRequestType::Subresource,
            is_method_safe: true,
            http_method: SameSiteContextHttpMethod::Unset,
            redirect_type: SameSiteContextRedirectType::Unset,
            include_http_only: false,
            update_access_time: true,
            return_excluded_cookies: true,
        }
    }

    /// Construct a context for browser-side introspection such as CDP.
    pub fn cdp(url: &'a Url) -> Self {
        Self {
            url,
            source: CookieAccessSource::Cdp,
            browser_context: BrowserSiteContext::empty(),
            same_site_context: SameSiteContext::same_site(),
            same_site_context_metadata: SameSiteContextMetadata::none(),
            request_type: HttpRequestType::Subresource,
            is_method_safe: true,
            http_method: SameSiteContextHttpMethod::Unset,
            redirect_type: SameSiteContextRedirectType::Unset,
            include_http_only: true,
            update_access_time: false,
            return_excluded_cookies: false,
        }
    }

    /// Configure whether excluded cookies should be returned.
    pub fn with_return_excluded_cookies(mut self, return_excluded_cookies: bool) -> Self {
        self.return_excluded_cookies = return_excluded_cookies;
        self
    }

    /// Configure whether successful matches should update access metadata.
    pub fn with_update_access_time(mut self, update_access_time: bool) -> Self {
        self.update_access_time = update_access_time;
        self
    }

    /// Configure whether `HttpOnly` cookies are visible in the result set.
    pub fn with_include_http_only(mut self, include_http_only: bool) -> Self {
        self.include_http_only = include_http_only;
        self
    }
}

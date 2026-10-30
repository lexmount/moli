use crate::cookie_domain::CookieDomain;
use crate::cookie_expiration::CookieExpiration;
use crate::cookie_path::CookiePath;

use crate::utils::{is_http_scheme, is_secure};
use cookie::{Cookie as RawCookie, CookieBuilder as RawCookieBuilder, ParseError};
use std::borrow::Cow;
use std::convert::TryFrom;
use std::fmt;
use std::ops::Deref;
use time::OffsetDateTime;
use url::Url;

pub use cookie::CookiePriority;

/// Browser-computed partition key attached to a `Partitioned` cookie.
///
/// A normal key is the schemeful top-level site plus Chromium's ancestor-chain
/// bit. Opaque top-level sites use a transient nonce so unrelated opaque
/// browsing contexts cannot share cookies accidentally.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CookiePartitionKey {
    Site {
        top_level_site: String,
        has_cross_site_ancestor: bool,
    },
    Opaque {
        nonce: u64,
        has_cross_site_ancestor: bool,
    },
}

impl CookiePartitionKey {
    pub fn site(top_level_site: String, has_cross_site_ancestor: bool) -> Self {
        Self::Site {
            top_level_site,
            has_cross_site_ancestor,
        }
    }

    pub fn opaque(nonce: u64, has_cross_site_ancestor: bool) -> Self {
        Self::Opaque {
            nonce,
            has_cross_site_ancestor,
        }
    }

    pub fn top_level_site(&self) -> Option<&str> {
        match self {
            Self::Site { top_level_site, .. } => Some(top_level_site),
            Self::Opaque { .. } => None,
        }
    }

    pub fn has_cross_site_ancestor(&self) -> bool {
        match self {
            Self::Site {
                has_cross_site_ancestor,
                ..
            }
            | Self::Opaque {
                has_cross_site_ancestor,
                ..
            } => *has_cross_site_ancestor,
        }
    }

    pub fn opaque_nonce(&self) -> Option<u64> {
        match self {
            Self::Site { .. } => None,
            Self::Opaque { nonce, .. } => Some(*nonce),
        }
    }

    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieSourceScheme {
    #[default]
    Unset,
    NonSecure,
    Secure,
}

impl CookieSourceScheme {
    pub fn from_url(url: &Url) -> Self {
        if matches!(url.scheme(), "https" | "wss") {
            Self::Secure
        } else {
            Self::NonSecure
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Cookie had attribute HttpOnly but was received from a request-uri which was not an http
    /// scheme
    NonHttpScheme,
    /// Cookie had attribute Secure but was received from a non-secure context
    SecureOnly,
    /// Cookie did not specify domain but was received from non-relative-scheme request-uri from
    /// which host could not be determined
    NonRelativeScheme,
    /// Cookie received from a request-uri that does not domain-match
    DomainMismatch,
    /// `SameSite=None` cookies must also be marked Secure
    SameSiteNoneRequiresSecure,
    /// Cookie violated one of the browser prefix rules
    PrefixViolation,
    /// An insecure write attempted to overlap an existing secure cookie
    SecureOverlay,
    /// The combined cookie name and value exceed the browser compatibility
    /// limit this fork models
    NameValueTooLarge,
    /// A `Partitioned` cookie was written without a browser-computed partition
    /// key.
    PartitionedMissingPartitionKey,
    /// A `Partitioned` cookie did not also specify `Secure`.
    PartitionedRequiresSecure,
    /// Compatibility-only rejection retained for callers that still deserialize
    /// an older result taxonomy.
    UnsupportedPartitioned,
    /// The cookie store could not make room for the new cookie under the
    /// configured quota and eviction policy
    StorageFull,
    /// Cookie is Expired
    Expired,
    /// `cookie::Cookie` Parse error
    Parse,
    #[cfg(feature = "public_suffix")]
    /// Cookie specified a public suffix domain-attribute that does not match the canonicalized
    /// request-uri host
    PublicSuffix,
    /// Tried to use a CookieDomain variant of `Empty` or `NotPresent` in a context requiring a Domain value
    UnspecifiedDomain,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match *self {
                Error::NonHttpScheme =>
                    "request-uri is not an http scheme but HttpOnly attribute set",
                Error::SecureOnly => "request-uri is not secure but Secure attribute set",
                Error::NonRelativeScheme => {
                    "request-uri is not a relative scheme; cannot determine host"
                }
                Error::DomainMismatch => "request-uri does not domain-match the cookie",
                Error::SameSiteNoneRequiresSecure =>
                    "SameSite=None cookies must also specify Secure",
                Error::PrefixViolation => "cookie violates a protected prefix rule",
                Error::SecureOverlay => "insecure cookie cannot overlay an existing secure cookie",
                Error::NameValueTooLarge => "cookie name and value exceed the supported size limit",
                Error::PartitionedMissingPartitionKey =>
                    "partitioned cookie write is missing a partition key",
                Error::PartitionedRequiresSecure => "partitioned cookies must also specify Secure",
                Error::UnsupportedPartitioned => "partitioned cookie input is unsupported",
                Error::StorageFull => "cookie store is at capacity and could not evict a cookie",
                Error::Expired => "attempted to utilize an Expired Cookie",
                Error::Parse => "unable to parse string as cookie::Cookie",
                #[cfg(feature = "public_suffix")]
                Error::PublicSuffix => "domain-attribute value is a public suffix",
                Error::UnspecifiedDomain => "domain-attribute is not specified",
            }
        )
    }
}

// cookie::Cookie::parse returns Result<Cookie, ()>
impl From<ParseError> for Error {
    fn from(_: ParseError) -> Error {
        Error::Parse
    }
}

pub type CookieResult<'a> = Result<Cookie<'a>, Error>;

/// Structured input for constructing a canonical cookie without first
/// serializing a raw `Set-Cookie` string.
///
/// This is intended for privileged/browser-side mutation paths such as DevTools,
/// where the caller already has parsed cookie fields and should not need to
/// reassemble an HTTP header just to enter the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalCookieInput {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<cookie::SameSite>,
    pub expires: CookieExpiration,
    pub partition_key: Option<CookiePartitionKey>,
    pub priority: Option<CookiePriority>,
    pub source_scheme: CookieSourceScheme,
    pub source_port: i32,
    /// Creation time restored from browser storage, or `None` when the age is unknown.
    pub creation_time: Option<OffsetDateTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreationTime {
    Pending,
    Unknown,
    Known(OffsetDateTime),
}

/// A cookie conforming more closely to [IETF RFC6265](https://datatracker.ietf.org/doc/html/rfc6265)
#[derive(PartialEq, Clone, Debug)]
pub struct Cookie<'a> {
    /// The parsed Set-Cookie data
    raw_cookie: RawCookie<'a>,
    /// The Path attribute from a Set-Cookie header or the default-path as
    /// determined from
    /// the request-uri
    pub path: CookiePath,
    /// The Domain attribute from a Set-Cookie header, or a HostOnly variant if no
    /// non-empty Domain attribute
    /// found
    pub domain: CookieDomain,
    /// For a persistent Cookie (see [IETF RFC6265 Section
    /// 5.3](https://datatracker.ietf.org/doc/html/rfc6265#section-5.3)),
    /// the expiration time as defined by the Max-Age or Expires attribute,
    /// otherwise SessionEnd,
    /// indicating a non-persistent `Cookie` that should expire at the end of the
    /// session
    pub expires: CookieExpiration,
    /// Monotonic creation order for the canonical cookie entry.
    ///
    /// This is the tie-break used by browser-style request projection after
    /// path length, and it must survive updates to an existing cookie so
    /// higher layers do not need their own parallel ordering state.
    pub(crate) creation_index: u64,
    /// Wall-clock creation time used by the Lax-allow-unsafe age limit.
    /// Unstored parsed cookies have no creation time yet.
    creation_time: CreationTime,
    /// Monotonic last-access order for the canonical cookie entry.
    ///
    /// Browser-style eviction uses this as the primary recency signal, so
    /// reads and writes update one canonical counter in the core instead of
    /// syncing wrapper-local metadata.
    pub(crate) last_access_index: u64,
    /// Declared `Priority` metadata from the source cookie, if the attribute was
    /// explicitly present.
    pub(crate) priority: Option<CookiePriority>,
    /// Metadata describing the source scheme of the write that created this cookie.
    pub(crate) source_scheme: CookieSourceScheme,
    /// Metadata describing the source port of the write that created this cookie.
    pub(crate) source_port: i32,
    /// Partition identity for a `Partitioned` cookie. `None` denotes an
    /// unpartitioned cookie.
    pub(crate) partition_key: Option<CookiePartitionKey>,
}

impl<'a> Cookie<'a> {
    /// Whether this `Cookie` should be included for `request_url`
    pub fn matches(&self, request_url: &Url) -> bool {
        self.path.matches(request_url)
            && self.domain.matches(request_url)
            && (!self.raw_cookie.secure().unwrap_or(false) || is_secure(request_url))
            && (!self.raw_cookie.http_only().unwrap_or(false) || is_http_scheme(request_url))
    }

    /// Should this `Cookie` be persisted across sessions?
    pub fn is_persistent(&self) -> bool {
        match self.expires {
            CookieExpiration::AtUtc(_) => true,
            CookieExpiration::SessionEnd => false,
        }
    }

    /// Expire this cookie
    pub fn expire(&mut self) {
        self.expires = CookieExpiration::from(0u64);
    }

    /// Return whether the `Cookie` is expired *now*
    pub fn is_expired(&self) -> bool {
        self.expires.is_expired()
    }

    pub fn creation_index(&self) -> u64 {
        self.creation_index
    }

    pub fn creation_time(&self) -> Option<OffsetDateTime> {
        match self.creation_time {
            CreationTime::Known(created) => Some(created),
            CreationTime::Pending | CreationTime::Unknown => None,
        }
    }

    pub fn last_access_index(&self) -> u64 {
        self.last_access_index
    }

    pub fn priority(&self) -> Option<CookiePriority> {
        self.priority
    }

    /// Returns the eviction priority, defaulting omitted `Priority` to `Medium`.
    pub fn effective_priority(&self) -> CookiePriority {
        self.priority.unwrap_or(CookiePriority::Medium)
    }

    pub fn source_scheme(&self) -> CookieSourceScheme {
        self.source_scheme
    }

    pub fn source_port(&self) -> i32 {
        self.source_port
    }

    pub fn partition_key(&self) -> Option<&CookiePartitionKey> {
        self.partition_key.as_ref()
    }

    /// Trusted persistence restore only; ordinary insertion owns its counters.
    pub fn with_persistent_ordering(mut self, creation: u64, access: u64) -> Self {
        self.creation_index = creation;
        self.last_access_index = access;
        self
    }

    pub(crate) fn set_creation_index(&mut self, creation_index: u64) {
        self.creation_index = creation_index;
    }

    pub(crate) fn set_creation_time(&mut self, creation_time: Option<OffsetDateTime>) {
        self.creation_time = match creation_time {
            Some(created) => CreationTime::Known(created),
            None => CreationTime::Unknown,
        };
    }

    pub(crate) fn initialize_creation_time(&mut self) {
        if self.creation_time == CreationTime::Pending {
            self.creation_time = CreationTime::Known(OffsetDateTime::now_utc());
        }
    }

    pub(crate) fn restore_creation_time(&mut self) {
        if self.creation_time == CreationTime::Pending {
            self.creation_time = CreationTime::Unknown;
        }
    }

    pub(crate) fn touch_with_access_index(&mut self, access_index: u64) {
        self.last_access_index = access_index;
    }

    pub fn set_priority(&mut self, priority: CookiePriority) {
        self.priority = Some(priority);
    }

    pub fn set_source_metadata(&mut self, source_scheme: CookieSourceScheme, source_port: i32) {
        self.source_scheme = source_scheme;
        self.source_port = source_port;
    }

    pub(crate) fn set_partition_key(&mut self, partition_key: Option<CookiePartitionKey>) {
        self.partition_key = partition_key;
    }

    /// Indicates if the `Cookie` expires as of `utc_tm`.
    pub fn expires_by(&self, utc_tm: &time::OffsetDateTime) -> bool {
        self.expires.expires_by(utc_tm)
    }

    /// Parses a new `cookie_store::Cookie` from `cookie_str`.
    pub fn parse<S>(cookie_str: S, request_url: &Url) -> CookieResult<'a>
    where
        S: Into<Cow<'a, str>>,
    {
        Cookie::try_from_raw_cookie(&crate::cookie_date::parse_cookie(cookie_str)?, request_url)
    }

    /// Create a canonical cookie from structured fields.
    ///
    /// This keeps raw-header assembly inside the core so higher layers can
    /// provide parsed browser-facing DTOs without reimplementing cookie syntax
    /// details.
    pub fn try_from_canonical_input(
        input: CanonicalCookieInput,
        request_url: &Url,
    ) -> CookieResult<'static> {
        let mut builder = RawCookieBuilder::new(input.name, input.value).path(input.path);
        if !input.host_only {
            builder = builder.domain(input.domain.clone());
        }
        if input.secure {
            builder = builder.secure(true);
        }
        if input.http_only {
            builder = builder.http_only(true);
        }
        if input.partition_key.is_some() {
            builder = builder.partitioned(true);
        }
        if let Some(same_site) = input.same_site {
            builder = builder.same_site(same_site);
        }
        if let CookieExpiration::AtUtc(expires) = input.expires {
            builder = builder.expires(expires);
        }

        let mut cookie = Cookie::try_from_raw_cookie(&builder.build(), request_url)?.into_owned();
        if input.host_only {
            cookie.domain =
                match CookieDomain::try_from(input.domain.as_str()).map_err(|_| Error::Parse)? {
                    CookieDomain::Suffix(domain) => CookieDomain::HostOnly(domain),
                    CookieDomain::Empty if request_url.scheme() == "file" => {
                        CookieDomain::HostOnly(String::new())
                    }
                    CookieDomain::Empty | CookieDomain::NotPresent | CookieDomain::HostOnly(_) => {
                        return Err(Error::Parse);
                    }
                };
        }
        cookie.priority = input.priority;
        cookie.partition_key = input.partition_key;
        cookie.set_source_metadata(input.source_scheme, input.source_port);
        cookie.set_creation_time(input.creation_time);
        Ok(cookie)
    }

    /// Create a new `cookie_store::Cookie` from a `cookie::Cookie` (from the `cookie` crate)
    /// received from `request_url`.
    pub fn try_from_raw_cookie(raw_cookie: &RawCookie<'a>, request_url: &Url) -> CookieResult<'a> {
        if raw_cookie.http_only().unwrap_or(false) && !is_http_scheme(request_url) {
            // If the cookie was received from a "non-HTTP" API and the
            // cookie's http-only-flag is set, abort these steps and ignore the
            // cookie entirely.
            return Err(Error::NonHttpScheme);
        }

        let domain = match CookieDomain::try_from(raw_cookie) {
            // 6.   If the domain-attribute is non-empty:
            Ok(d @ CookieDomain::Suffix(_)) => {
                if !d.matches(request_url) {
                    //    If the canonicalized request-host does not domain-match the
                    //    domain-attribute:
                    //       Ignore the cookie entirely and abort these steps.
                    Err(Error::DomainMismatch)
                } else {
                    //    Otherwise:
                    //       Set the cookie's host-only-flag to false.
                    //       Set the cookie's domain to the domain-attribute.
                    Ok(d)
                }
            }
            Err(_) => Err(Error::Parse),
            // Otherwise:
            //    Set the cookie's host-only-flag to true.
            //    Set the cookie's domain to the canonicalized request-host.
            _ => CookieDomain::host_only(request_url),
        }?;

        let path = raw_cookie
            .path()
            .as_ref()
            .and_then(|p| CookiePath::parse(p))
            .unwrap_or_else(|| CookiePath::default_path(request_url));

        // per RFC6265, Max-Age takes precedence, then Expires, otherwise is Session
        // only
        let expires = if let Some(max_age) = raw_cookie.max_age() {
            CookieExpiration::from(max_age)
        } else if let Some(expiration) = raw_cookie.expires() {
            CookieExpiration::from(expiration)
        } else {
            CookieExpiration::SessionEnd
        };

        let priority = raw_cookie.priority();

        Ok(Cookie {
            raw_cookie: raw_cookie.clone(),
            path,
            expires,
            domain,
            creation_index: 0,
            creation_time: CreationTime::Pending,
            last_access_index: 0,
            priority,
            source_scheme: CookieSourceScheme::from_url(request_url),
            source_port: request_url.port_or_known_default().map_or(-1, i32::from),
            partition_key: None,
        })
    }

    pub fn into_owned(self) -> Cookie<'static> {
        Cookie {
            raw_cookie: self.raw_cookie.into_owned(),
            path: self.path,
            domain: self.domain,
            expires: self.expires,
            creation_index: self.creation_index,
            creation_time: self.creation_time,
            last_access_index: self.last_access_index,
            priority: self.priority,
            source_scheme: self.source_scheme,
            source_port: self.source_port,
            partition_key: self.partition_key,
        }
    }
}

impl<'a> Deref for Cookie<'a> {
    type Target = RawCookie<'a>;
    fn deref(&self) -> &Self::Target {
        &self.raw_cookie
    }
}

impl<'a> From<Cookie<'a>> for RawCookie<'a> {
    fn from(cookie: Cookie<'a>) -> RawCookie<'static> {
        let mut builder =
            RawCookieBuilder::new(cookie.name().to_owned(), cookie.value().to_owned());

        // Max-Age is relative, will not have same meaning now, so only set `Expires`.
        match cookie.expires {
            CookieExpiration::AtUtc(utc_tm) => {
                builder = builder.expires(utc_tm);
            }
            CookieExpiration::SessionEnd => {}
        }

        if cookie.path.is_from_path_attr() {
            builder = builder.path(String::from(cookie.path));
        }

        if let CookieDomain::Suffix(s) = cookie.domain {
            builder = builder.domain(s);
        }

        if let Some(priority) = cookie.priority {
            builder = builder.priority(priority);
        }

        if cookie.partition_key.is_some() {
            builder = builder.partitioned(true);
        }

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::Cookie;
    use crate::cookie_domain::CookieDomain;
    use crate::cookie_expiration::CookieExpiration;
    use cookie::Cookie as RawCookie;
    use time::{Duration, OffsetDateTime};
    use url::Url;

    use crate::utils::test as test_utils;

    fn cmp_domain(cookie: &str, url: &str, exp: CookieDomain) {
        let ua = test_utils::make_cookie(cookie, url, None, None);
        assert!(ua.domain == exp, "\n{ua:?}");
    }

    #[test]
    fn no_domain() {
        let url = test_utils::url("http://example.com/foo/bar");
        cmp_domain(
            "cookie1=value1",
            "http://example.com/foo/bar",
            CookieDomain::host_only(&url).expect("unable to parse domain"),
        );
    }

    // per RFC6265:
    // If the attribute-value is empty, the behavior is undefined.  However,
    //   the user agent SHOULD ignore the cookie-av entirely.
    #[test]
    fn empty_domain() {
        let url = test_utils::url("http://example.com/foo/bar");
        cmp_domain(
            "cookie1=value1; Domain=",
            "http://example.com/foo/bar",
            CookieDomain::host_only(&url).expect("unable to parse domain"),
        );
    }

    #[test]
    fn mismatched_domain() {
        let ua = Cookie::parse(
            "cookie1=value1; Domain=notmydomain.com",
            &test_utils::url("http://example.com/foo/bar"),
        );
        assert!(ua.is_err(), "{ua:?}");
    }

    #[test]
    fn domains() {
        fn domain_from(domain: &str, request_url: &str, is_some: bool) {
            let cookie_str = format!("cookie1=value1; Domain={domain}");
            let raw_cookie = RawCookie::parse(cookie_str).unwrap();
            let cookie = Cookie::try_from_raw_cookie(&raw_cookie, &test_utils::url(request_url));
            assert_eq!(is_some, cookie.is_ok())
        }
        //        The user agent will reject cookies unless the Domain attribute
        // specifies a scope for the cookie that would include the origin
        // server.  For example, the user agent will accept a cookie with a
        // Domain attribute of "example.com" or of "foo.example.com" from
        // foo.example.com, but the user agent will not accept a cookie with a
        // Domain attribute of "bar.example.com" or of "baz.foo.example.com".
        domain_from("example.com", "http://foo.example.com", true);
        domain_from(".example.com", "http://foo.example.com", true);
        domain_from("foo.example.com", "http://foo.example.com", true);
        domain_from(".foo.example.com", "http://foo.example.com", true);

        domain_from("oo.example.com", "http://foo.example.com", false);
        domain_from("myexample.com", "http://foo.example.com", false);
        domain_from("bar.example.com", "http://foo.example.com", false);
        domain_from("baz.foo.example.com", "http://foo.example.com", false);
    }

    #[test]
    fn httponly() {
        let c = RawCookie::parse("cookie1=value1; HttpOnly").unwrap();
        let url = Url::parse("ftp://example.com/foo/bar").unwrap();
        let ua = Cookie::try_from_raw_cookie(&c, &url);
        assert!(ua.is_err(), "{ua:?}");
    }

    #[test]
    fn raw_cookie_priority_is_projected_into_core_cookie() {
        let raw_cookie = RawCookie::parse("sid=1; Path=/; Priority=High").unwrap();
        let cookie =
            Cookie::try_from_raw_cookie(&raw_cookie, &test_utils::url("https://example.com/"))
                .expect("cookie should parse");

        assert_eq!(cookie.priority(), Some(super::CookiePriority::High));
        assert_eq!(cookie.effective_priority(), super::CookiePriority::High);
    }

    #[test]
    fn raw_cookie_conversion_does_not_synthesize_default_priority_attribute() {
        let cookie = Cookie::parse("sid=1; Path=/", &test_utils::url("https://example.com/"))
            .expect("cookie should parse")
            .into_owned();
        let raw_cookie: RawCookie<'static> = cookie.into();

        assert_eq!(raw_cookie.priority(), None);
        assert!(!raw_cookie.to_string().contains("Priority="));
    }

    #[test]
    fn raw_cookie_conversion_preserves_explicit_medium_priority_attribute() {
        let cookie = Cookie::parse(
            "sid=1; Path=/; Priority=Medium",
            &test_utils::url("https://example.com/"),
        )
        .expect("cookie should parse")
        .into_owned();
        let raw_cookie: RawCookie<'static> = cookie.into();

        assert_eq!(raw_cookie.priority(), Some(cookie::CookiePriority::Medium));
        assert!(raw_cookie.to_string().contains("Priority=Medium"));
    }

    #[test]
    fn identical_domain() {
        cmp_domain(
            "cookie1=value1; Domain=example.com",
            "http://example.com/foo/bar",
            CookieDomain::Suffix(String::from("example.com")),
        );
    }

    #[test]
    fn identical_domain_leading_dot() {
        cmp_domain(
            "cookie1=value1; Domain=.example.com",
            "http://example.com/foo/bar",
            CookieDomain::Suffix(String::from("example.com")),
        );
    }

    #[test]
    fn identical_domain_two_leading_dots() {
        cmp_domain(
            "cookie1=value1; Domain=..example.com",
            "http://..example.com/foo/bar",
            CookieDomain::Suffix(String::from(".example.com")),
        );
    }

    #[test]
    fn upper_case_domain() {
        cmp_domain(
            "cookie1=value1; Domain=EXAMPLE.com",
            "http://example.com/foo/bar",
            CookieDomain::Suffix(String::from("example.com")),
        );
    }

    fn cmp_path(cookie: &str, url: &str, exp: &str) {
        let ua = test_utils::make_cookie(cookie, url, None, None);
        assert!(String::from(ua.path.clone()) == exp, "\n{ua:?}");
    }

    #[test]
    fn no_path() {
        // no Path specified
        cmp_path("cookie1=value1", "http://example.com/foo/bar/", "/foo/bar");
        cmp_path("cookie1=value1", "http://example.com/foo/bar", "/foo");
        cmp_path("cookie1=value1", "http://example.com/foo", "/");
        cmp_path("cookie1=value1", "http://example.com/", "/");
        cmp_path("cookie1=value1", "http://example.com", "/");
    }

    #[test]
    fn empty_path() {
        // Path specified with empty value
        cmp_path(
            "cookie1=value1; Path=",
            "http://example.com/foo/bar/",
            "/foo/bar",
        );
        cmp_path(
            "cookie1=value1; Path=",
            "http://example.com/foo/bar",
            "/foo",
        );
        cmp_path("cookie1=value1; Path=", "http://example.com/foo", "/");
        cmp_path("cookie1=value1; Path=", "http://example.com/", "/");
        cmp_path("cookie1=value1; Path=", "http://example.com", "/");
    }

    #[test]
    fn invalid_path() {
        // Invalid Path specified (first character not /)
        cmp_path(
            "cookie1=value1; Path=baz",
            "http://example.com/foo/bar/",
            "/foo/bar",
        );
        cmp_path(
            "cookie1=value1; Path=baz",
            "http://example.com/foo/bar",
            "/foo",
        );
        cmp_path("cookie1=value1; Path=baz", "http://example.com/foo", "/");
        cmp_path("cookie1=value1; Path=baz", "http://example.com/", "/");
        cmp_path("cookie1=value1; Path=baz", "http://example.com", "/");
    }

    #[test]
    fn path() {
        // Path specified, single /
        cmp_path(
            "cookie1=value1; Path=/baz",
            "http://example.com/foo/bar/",
            "/baz",
        );
        // Path specified, multiple / (for valid attribute-value on path, take full
        // string)
        cmp_path(
            "cookie1=value1; Path=/baz/",
            "http://example.com/foo/bar/",
            "/baz/",
        );
    }

    // expiry-related tests
    #[inline]
    fn in_days(days: i64) -> OffsetDateTime {
        OffsetDateTime::now_utc() + Duration::days(days)
    }
    #[inline]
    fn in_minutes(mins: i64) -> OffsetDateTime {
        OffsetDateTime::now_utc() + Duration::minutes(mins)
    }

    #[test]
    fn max_age_bounds() {
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            None,
            Some(9223372036854776),
        );
        assert!(matches!(ua.expires, CookieExpiration::AtUtc(_)));
    }

    #[test]
    fn max_age() {
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            None,
            Some(60),
        );
        assert!(!ua.is_expired());
        assert!(ua.expires_by(&in_minutes(2)));
    }

    #[test]
    fn expired() {
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            None,
            Some(0u64),
        );
        assert!(ua.is_expired());
        assert!(ua.expires_by(&in_days(-1)));
        let ua = test_utils::make_cookie(
            "cookie1=value1; Max-Age=0",
            "http://example.com/foo/bar",
            None,
            None,
        );
        assert!(ua.is_expired());
        assert!(ua.expires_by(&in_days(-1)));
        let ua = test_utils::make_cookie(
            "cookie1=value1; Max-Age=-1",
            "http://example.com/foo/bar",
            None,
            None,
        );
        assert!(ua.is_expired());
        assert!(ua.expires_by(&in_days(-1)));
    }

    #[test]
    fn session_end() {
        let ua =
            test_utils::make_cookie("cookie1=value1", "http://example.com/foo/bar", None, None);
        assert!(matches!(ua.expires, CookieExpiration::SessionEnd));
        assert!(!ua.is_expired());
        assert!(!ua.expires_by(&in_days(1)));
        assert!(!ua.expires_by(&in_days(-1)));
    }

    #[test]
    fn expires_tmrw_at_utc() {
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some(in_days(1)),
            None,
        );
        assert!(!ua.is_expired());
        assert!(ua.expires_by(&in_days(2)));
    }

    #[test]
    fn expired_yest_at_utc() {
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some(in_days(-1)),
            None,
        );
        assert!(ua.is_expired());
        assert!(!ua.expires_by(&in_days(-2)));
    }

    #[test]
    fn is_persistent() {
        let ua =
            test_utils::make_cookie("cookie1=value1", "http://example.com/foo/bar", None, None);
        assert!(!ua.is_persistent()); // SessionEnd
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some(in_days(1)),
            None,
        );
        assert!(ua.is_persistent()); // AtUtc from Expires
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some(in_days(1)),
            Some(60),
        );
        assert!(ua.is_persistent()); // AtUtc from Max-Age
    }

    #[test]
    fn max_age_overrides_expires() {
        // Expires indicates expiration yesterday, but Max-Age indicates expiry in 1
        // minute
        let ua = test_utils::make_cookie(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some(in_days(-1)),
            Some(60),
        );
        assert!(!ua.is_expired());
        assert!(ua.expires_by(&in_minutes(2)));
    }

    // A request-path path-matches a given cookie-path if at least one of
    // the following conditions holds:
    // o  The cookie-path and the request-path are identical.
    // o  The cookie-path is a prefix of the request-path, and the last
    //    character of the cookie-path is %x2F ("/").
    // o  The cookie-path is a prefix of the request-path, and the first
    //    character of the request-path that is not included in the cookie-
    //    path is a %x2F ("/") character.
    #[test]
    fn matches() {
        fn do_match(exp: bool, cookie: &str, src_url: &str, request_url: Option<&str>) {
            let ua = test_utils::make_cookie(cookie, src_url, None, None);
            let request_url = request_url.unwrap_or(src_url);
            assert!(
                exp == ua.matches(&Url::parse(request_url).unwrap()),
                "\n>> {:?}\nshould{}match\n>> {:?}\n",
                ua,
                if exp { " " } else { " NOT " },
                request_url
            );
        }
        fn is_match(cookie: &str, url: &str, request_url: Option<&str>) {
            do_match(true, cookie, url, request_url);
        }
        fn is_mismatch(cookie: &str, url: &str, request_url: Option<&str>) {
            do_match(false, cookie, url, request_url);
        }

        // match: request-path & cookie-path (defaulted from request-uri) identical
        is_match("cookie1=value1", "http://example.com/foo/bar", None);
        // mismatch: request-path & cookie-path do not match
        is_mismatch(
            "cookie1=value1",
            "http://example.com/bus/baz/",
            Some("http://example.com/foo/bar"),
        );
        is_mismatch(
            "cookie1=value1; Path=/bus/baz",
            "http://example.com/foo/bar",
            None,
        );
        // match: cookie-path a prefix of request-path and last character of
        // cookie-path is /
        is_match(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some("http://example.com/foo/bar"),
        );
        is_match(
            "cookie1=value1; Path=/foo/",
            "http://example.com/foo/bar",
            None,
        );
        // mismatch: cookie-path a prefix of request-path but last character of
        // cookie-path is not /
        // and first character of request-path not included in cookie-path is not /
        is_mismatch(
            "cookie1=value1",
            "http://example.com/fo/",
            Some("http://example.com/foo/bar"),
        );
        is_mismatch(
            "cookie1=value1; Path=/fo",
            "http://example.com/foo/bar",
            None,
        );
        // match: cookie-path a prefix of request-path and first character of
        // request-path
        // not included in the cookie-path is /
        is_match(
            "cookie1=value1",
            "http://example.com/foo/",
            Some("http://example.com/foo/bar"),
        );
        is_match(
            "cookie1=value1; Path=/foo",
            "http://example.com/foo/bar",
            None,
        );
        // match: Path overridden to /, which matches all paths from the domain
        is_match(
            "cookie1=value1; Path=/",
            "http://example.com/foo/bar",
            Some("http://example.com/bus/baz"),
        );
        // mismatch: different domain
        is_mismatch(
            "cookie1=value1",
            "http://example.com/foo/",
            Some("http://notmydomain.com/foo/bar"),
        );
        is_mismatch(
            "cookie1=value1; Domain=example.com",
            "http://foo.example.com/foo/",
            Some("http://notmydomain.com/foo/bar"),
        );
        // match: secure protocol
        is_match(
            "cookie1=value1; Secure",
            "http://example.com/foo/bar",
            Some("https://example.com/foo/bar"),
        );
        // mismatch: non-secure protocol
        is_mismatch(
            "cookie1=value1; Secure",
            "http://example.com/foo/bar",
            Some("http://example.com/foo/bar"),
        );
        // match: no http restriction
        is_match(
            "cookie1=value1",
            "http://example.com/foo/bar",
            Some("ftp://example.com/foo/bar"),
        );
        // match: http protocol
        is_match(
            "cookie1=value1; HttpOnly",
            "http://example.com/foo/bar",
            Some("http://example.com/foo/bar"),
        );
        is_match(
            "cookie1=value1; HttpOnly",
            "http://example.com/foo/bar",
            Some("HTTP://example.com/foo/bar"),
        );
        is_match(
            "cookie1=value1; HttpOnly",
            "http://example.com/foo/bar",
            Some("https://example.com/foo/bar"),
        );
        // mismatch: http requried
        is_mismatch(
            "cookie1=value1; HttpOnly",
            "http://example.com/foo/bar",
            Some("ftp://example.com/foo/bar"),
        );
        is_mismatch(
            "cookie1=value1; HttpOnly",
            "http://example.com/foo/bar",
            Some("data:nonrelativescheme"),
        );
    }
}

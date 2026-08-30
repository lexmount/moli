use cookie::Cookie as RawCookie;
use std::convert::TryFrom;
use url::{Host, Url};

use crate::utils::is_host_name;
use crate::CookieError;

pub fn is_match(domain: &str, request_url: &Url) -> bool {
    if domain.is_empty() {
        return matches!(canonical_request_host(request_url).as_deref(), Some(""));
    }
    CookieDomain::try_from(domain)
        .map(|domain| domain.matches(request_url))
        .unwrap_or(false)
}

fn canonical_request_host(request_url: &Url) -> Option<String> {
    match request_url.host_str() {
        Some(host) => canonicalize_host_text(host),
        None if request_url.scheme() == "file" => Some(String::new()),
        None => None,
    }
}

fn canonicalize_host_text(host: &str) -> Option<String> {
    let host = host.trim();
    if host.contains('%') {
        return None;
    }
    match Host::parse(host).ok()? {
        Host::Domain(domain) => idna::domain_to_ascii(domain.trim()).ok(),
        Host::Ipv4(addr) => Some(addr.to_string()),
        Host::Ipv6(addr) => Some(format!("[{addr}]")),
    }
}

/// The domain of a `Cookie`
#[derive(PartialEq, Eq, Clone, Debug, Hash, PartialOrd, Ord)]
pub enum CookieDomain {
    /// No Domain attribute in Set-Cookie header
    HostOnly(String),
    /// Domain attribute from Set-Cookie header
    Suffix(String),
    /// Domain attribute was not present in the Set-Cookie header
    NotPresent,
    /// Domain attribute-value was empty; technically undefined behavior, but suggested that this
    /// be treated as invalid
    Empty,
}

// 5.1.3.  Domain Matching
// A string domain-matches a given domain string if at least one of the
// following conditions hold:
//
// o  The domain string and the string are identical.  (Note that both
//    the domain string and the string will have been canonicalized to
//    lower case at this point.)
//
// o  All of the following conditions hold:
//
//    *  The domain string is a suffix of the string.
//
//    *  The last character of the string that is not included in the
//       domain string is a %x2E (".") character.
//
//    *  The string is a host name (i.e., not an IP address).
/// The concept of a domain match per [IETF RFC6265 Section
/// 5.1.3](https://datatracker.ietf.org/doc/html/rfc6265#section-5.1.3)
impl CookieDomain {
    /// Get the CookieDomain::HostOnly variant based on `request_url`. This is the effective behavior of
    /// setting the domain-attribute to empty
    pub fn host_only(request_url: &Url) -> Result<CookieDomain, CookieError> {
        match request_url.host_str() {
            Some(host) => canonicalize_host_text(host)
                .map(CookieDomain::HostOnly)
                .ok_or(CookieError::Parse),
            None if request_url.scheme() == "file" => Ok(CookieDomain::HostOnly(String::new())),
            None => Err(CookieError::NonRelativeScheme),
        }
    }

    /// Tests if the given `url::Url` meets the domain-match criteria
    pub fn matches(&self, request_url: &Url) -> bool {
        if let Some(url_host) = canonical_request_host(request_url) {
            match *self {
                CookieDomain::HostOnly(ref host) => host == &url_host,
                CookieDomain::Suffix(ref suffix) => {
                    suffix == &url_host
                        || (is_host_name(&url_host)
                            && url_host.ends_with(suffix)
                            && url_host[(url_host.len() - suffix.len() - 1)..].starts_with('.'))
                }
                CookieDomain::NotPresent | CookieDomain::Empty => false, // nothing can match the Empty case
            }
        } else {
            false // not a matchable scheme
        }
    }

    /// Tests if the given `url::Url` has a request-host identical to the domain attribute
    pub fn host_is_identical(&self, request_url: &Url) -> bool {
        if let Some(url_host) = canonical_request_host(request_url) {
            match *self {
                CookieDomain::HostOnly(ref host) => host == &url_host,
                CookieDomain::Suffix(ref suffix) => suffix == &url_host,
                CookieDomain::NotPresent | CookieDomain::Empty => false, // nothing can match the Empty case
            }
        } else {
            false // not a matchable scheme
        }
    }

    /// Tests if the domain-attribute is a public suffix as indicated by the provided lookup.
    #[cfg(feature = "public_suffix")]
    pub fn is_public_suffix(&self, psl: &dyn crate::CookiePublicSuffixList) -> bool {
        if let Some(domain) = self.as_cow().as_ref().map(|d| d.as_bytes()) {
            psl.is_public_suffix(domain)
        } else {
            false
        }
    }

    /// Get a borrowed string representation of the domain. For `Empty` and `NotPresent` variants,
    /// `None` shall be returned;
    pub fn as_cow(&self) -> Option<std::borrow::Cow<'_, str>> {
        match *self {
            CookieDomain::HostOnly(ref s) | CookieDomain::Suffix(ref s) => {
                Some(std::borrow::Cow::Borrowed(s))
            }
            CookieDomain::Empty | CookieDomain::NotPresent => None,
        }
    }
}

/// Construct a `CookieDomain::Suffix` from a string, stripping a single leading '.' if present.
/// If the source string is empty, returns the `CookieDomain::Empty` variant.
impl TryFrom<&str> for CookieDomain {
    type Error = crate::Error;
    fn try_from(value: &str) -> Result<CookieDomain, Self::Error> {
        idna::domain_to_ascii(value.trim())
            .map_err(super::IdnaErrors::from)
            .map_err(Into::into)
            .map(|domain| {
                if domain.is_empty() || "." == domain {
                    CookieDomain::Empty
                } else if let Some(stripped_suffix) = domain.strip_prefix('.') {
                    CookieDomain::Suffix(String::from(stripped_suffix))
                } else {
                    CookieDomain::Suffix(domain)
                }
            })
    }
}

/// Construct a `CookieDomain::Suffix` from a `cookie::Cookie`, which handles stripping a leading
/// '.' for us. If the cookie.domain is None or an empty string, the `CookieDomain::Empty` variant
/// is returned.
/// __NOTE__: `cookie::Cookie` domain values already have the leading '.' stripped. To avoid
/// performing this step twice, the `From<&cookie::Cookie>` impl should be used,
/// instead of passing `cookie.domain` to the `From<&str>` impl.
impl<'a, 'c> TryFrom<&'a RawCookie<'c>> for CookieDomain {
    type Error = crate::Error;
    fn try_from(cookie: &'a RawCookie<'c>) -> Result<CookieDomain, Self::Error> {
        if let Some(domain) = cookie.domain() {
            idna::domain_to_ascii(domain.trim())
                .map_err(super::IdnaErrors::from)
                .map_err(Into::into)
                .map(|domain| {
                    if domain.is_empty() {
                        CookieDomain::Empty
                    } else {
                        CookieDomain::Suffix(domain)
                    }
                })
        } else {
            Ok(CookieDomain::NotPresent)
        }
    }
}

impl<'a> From<&'a CookieDomain> for String {
    fn from(c: &'a CookieDomain) -> String {
        match *c {
            CookieDomain::HostOnly(ref h) => h.to_owned(),
            CookieDomain::Suffix(ref s) => s.to_owned(),
            CookieDomain::Empty | CookieDomain::NotPresent => "".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use cookie::Cookie as RawCookie;
    use std::convert::TryFrom;
    use url::Url;

    use super::CookieDomain;
    use crate::utils::test::*;

    #[inline]
    fn matches(expected: bool, cookie_domain: &CookieDomain, url: &str) {
        let url = Url::parse(url).unwrap();
        assert!(
            expected == cookie_domain.matches(&url),
            "cookie_domain: {:?} url: {:?}, url.host_str(): {:?}",
            cookie_domain,
            url,
            url.host_str()
        );
    }

    #[inline]
    fn variants(expected: bool, cookie_domain: &CookieDomain, url: &str) {
        matches(expected, cookie_domain, url);
        matches(expected, cookie_domain, &format!("{url}/"));
        matches(expected, cookie_domain, &format!("{url}:8080"));
        matches(expected, cookie_domain, &format!("{url}/foo/bar"));
        matches(expected, cookie_domain, &format!("{url}:8080/foo/bar"));
    }

    #[test]
    fn matches_hostonly() {
        {
            let url = url("http://example.com");
            // HostOnly must be an identical string match, and may be an IP address
            // or a hostname
            let host_name = CookieDomain::host_only(&url).expect("unable to parse domain");
            matches(false, &host_name, "data:nonrelative");
            variants(true, &host_name, "http://example.com");
            variants(false, &host_name, "http://example.org");
            // per RFC6265:
            //    WARNING: Some existing user agents treat an absent Domain
            //      attribute as if the Domain attribute were present and contained
            //      the current host name.  For example, if example.com returns a Set-
            //      Cookie header without a Domain attribute, these user agents will
            //      erroneously send the cookie to www.example.com as well.
            variants(false, &host_name, "http://foo.example.com");
            variants(false, &host_name, "http://127.0.0.1");
            variants(false, &host_name, "http://[::1]");
        }

        {
            let url = url("http://127.0.0.1");
            let ip4 = CookieDomain::host_only(&url).expect("unable to parse Ipv4");
            matches(false, &ip4, "data:nonrelative");
            variants(true, &ip4, "http://127.0.0.1");
            variants(false, &ip4, "http://[::1]");
        }

        {
            let url = url("http://[::1]");
            let ip6 = CookieDomain::host_only(&url).expect("unable to parse Ipv6");
            matches(false, &ip6, "data:nonrelative");
            variants(false, &ip6, "http://127.0.0.1");
            variants(true, &ip6, "http://[::1]");
        }
    }

    #[test]
    fn from_strs() {
        assert_eq!(
            CookieDomain::Empty,
            CookieDomain::try_from("").expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Empty,
            CookieDomain::try_from(".").expect("unable to parse domain")
        );
        // per [IETF RFC6265 Section 5.2.3](https://datatracker.ietf.org/doc/html/rfc6265#section-5.2.3)
        //If the first character of the attribute-value string is %x2E ("."):
        //
        //Let cookie-domain be the attribute-value without the leading %x2E
        //(".") character.
        assert_eq!(
            CookieDomain::Suffix(String::from(".")),
            CookieDomain::try_from("..").expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Suffix(String::from("example.com")),
            CookieDomain::try_from("example.com").expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Suffix(String::from("example.com")),
            CookieDomain::try_from(".example.com").expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Suffix(String::from(".example.com")),
            CookieDomain::try_from("..example.com").expect("unable to parse domain")
        );
    }

    #[test]
    fn from_raw_cookie() {
        fn raw_cookie(s: &str) -> RawCookie<'_> {
            RawCookie::parse(s).unwrap()
        }
        assert_eq!(
            CookieDomain::NotPresent,
            CookieDomain::try_from(&raw_cookie("cookie=value")).expect("unable to parse domain")
        );
        // cookie::Cookie handles this (cookie.domain == None)
        assert_eq!(
            CookieDomain::NotPresent,
            CookieDomain::try_from(&raw_cookie("cookie=value; Domain="))
                .expect("unable to parse domain")
        );
        // cookie::Cookie does not handle this (empty after stripping leading dot)
        assert_eq!(
            CookieDomain::Empty,
            CookieDomain::try_from(&raw_cookie("cookie=value; Domain=."))
                .expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Suffix(String::from("example.com")),
            CookieDomain::try_from(&raw_cookie("cookie=value; Domain=.example.com"))
                .expect("unable to parse domain")
        );
        assert_eq!(
            CookieDomain::Suffix(String::from("example.com")),
            CookieDomain::try_from(&raw_cookie("cookie=value; Domain=example.com"))
                .expect("unable to parse domain")
        );
    }

    #[test]
    fn matches_suffix() {
        {
            let suffix = CookieDomain::try_from("example.com").expect("unable to parse domain");
            variants(true, &suffix, "http://example.com"); //  exact match
            variants(true, &suffix, "http://foo.example.com"); //  suffix match
            variants(false, &suffix, "http://example.org"); //  no match
            variants(false, &suffix, "http://xample.com"); //  request is the suffix, no match
            variants(false, &suffix, "http://fooexample.com"); //  suffix, but no "." b/w foo and example, no match
        }

        {
            // strip leading dot
            let suffix = CookieDomain::try_from(".example.com").expect("unable to parse domain");
            variants(true, &suffix, "http://example.com");
            variants(true, &suffix, "http://foo.example.com");
            variants(false, &suffix, "http://example.org");
            variants(false, &suffix, "http://xample.com");
            variants(false, &suffix, "http://fooexample.com");
        }

        {
            // only first leading dot is stripped
            let suffix = CookieDomain::try_from("..example.com").expect("unable to parse domain");
            variants(true, &suffix, "http://.example.com");
            variants(true, &suffix, "http://foo..example.com");
            variants(false, &suffix, "http://example.com");
            variants(false, &suffix, "http://foo.example.com");
            variants(false, &suffix, "http://example.org");
            variants(false, &suffix, "http://xample.com");
            variants(false, &suffix, "http://fooexample.com");
        }

        {
            // an exact string match, although an IP is specified
            let suffix = CookieDomain::try_from("127.0.0.1").expect("unable to parse Ipv4");
            variants(true, &suffix, "http://127.0.0.1");
        }

        {
            // an exact string match, although an IP is specified
            let suffix = CookieDomain::try_from("[::1]").expect("unable to parse Ipv6");
            variants(true, &suffix, "http://[::1]");
        }

        {
            // non-identical suffix match only works for host names (i.e. not IPs)
            let suffix = CookieDomain::try_from("0.0.1").expect("unable to parse Ipv4");
            variants(false, &suffix, "http://127.0.0.1");
        }
    }
}

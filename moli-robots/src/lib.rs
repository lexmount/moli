//! Robots Exclusion Protocol (RFC 9309) parsing and matching.
//!
//! This crate answers one question: given the text of a `robots.txt` file, a
//! user agent, and a request target, may the fetch proceed? It performs no
//! I/O. Callers own the robots.txt retrieval and hand the outcome to
//! [`RobotsPolicy`], which also models the RFC 9309 §2.3.1 rules for responses
//! that carry no usable rule set.
//!
//! Two deliberate choices are worth naming, because the RFC leaves them to the
//! implementation:
//!
//! * **User-agent matching is a case-insensitive substring test against the
//!   full user-agent string.** Product tokens appear in many positions in real
//!   user agents (`Mozilla/5.0 (compatible; ExampleBot/1.0; ...)`), so
//!   anchoring the comparison to the leading token would silently skip groups
//!   a site wrote for us. A substring test can over-match, which errs toward
//!   obeying more rules rather than fewer — the correct bias for a switch whose
//!   entire purpose is to obey.
//! * **A rule whose path does not begin with `/` or `*` never matches.** This
//!   mirrors Google's reference implementation: patterns are matched from the
//!   start of a request target, and every request target begins with `/`.

mod pattern;
mod robots_txt;

#[cfg(test)]
mod tests;

pub use robots_txt::RobotsTxt;

use url::Url;

/// The rule set that applies to one origin, including the RFC 9309 §2.3.1
/// outcomes for responses that carry no usable rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RobotsPolicy {
    /// A `robots.txt` file was retrieved and parsed.
    Rules(RobotsTxt),
    /// No rules apply, so every request target is permitted. RFC 9309 §2.3.1.3
    /// assigns this to "unavailable" (4xx) responses.
    AllowAll,
    /// The rules could not be retrieved. RFC 9309 §2.3.1.4 asks crawlers to
    /// assume a complete disallow for "unreachable" (5xx) responses, and this
    /// crate extends that to transport failures.
    DisallowAll,
}

impl RobotsPolicy {
    /// Classifies an HTTP response for `robots.txt` per RFC 9309 §2.3.1.
    ///
    /// `status` must be the status of the final response, after any redirects
    /// have been followed.
    pub fn from_http_status(status: u16, body: &str) -> Self {
        match status {
            200..=299 => Self::Rules(RobotsTxt::parse(body)),
            // "Unavailable" — the origin has no rules for us to obey.
            400..=499 => Self::AllowAll,
            // "Unreachable" — the origin may have rules we failed to read.
            500..=599 => Self::DisallowAll,
            // 1xx and 3xx cannot appear as a final response for a followed
            // request; treat anything else as carrying no rules.
            _ => Self::AllowAll,
        }
    }

    /// The policy to apply when `robots.txt` could not be retrieved at all.
    pub fn unreachable() -> Self {
        Self::DisallowAll
    }

    /// Whether `user_agent` may fetch `request_target`.
    ///
    /// `request_target` is a path with an optional query string, as produced by
    /// [`robots_request_target`].
    pub fn allows(&self, user_agent: &str, request_target: &str) -> bool {
        match self {
            Self::Rules(robots) => robots.allows(user_agent, request_target),
            Self::AllowAll => true,
            Self::DisallowAll => false,
        }
    }
}

/// The `robots.txt` URL governing `target`, or `None` when the scheme carries
/// no robots policy.
///
/// Only HTTP(S) URLs have a robots.txt; `file:`, `data:`, and `about:` targets
/// return `None` so callers can skip the check rather than fail it.
pub fn robots_txt_url(target: &Url) -> Option<Url> {
    if !matches!(target.scheme(), "http" | "https") {
        return None;
    }
    let mut robots = target.clone();
    robots.set_path("/robots.txt");
    robots.set_query(None);
    robots.set_fragment(None);
    // Credentials in the target URL are not part of the robots.txt request.
    let _ = robots.set_username("");
    let _ = robots.set_password(None);
    Some(robots)
}

/// The path-and-query string that `robots.txt` rules are matched against.
///
/// RFC 9309 §2.2.2 matches rule patterns against the path and query of the
/// request, excluding the fragment.
pub fn robots_request_target(target: &Url) -> String {
    match target.query() {
        Some(query) => format!("{}?{}", target.path(), query),
        None => target.path().to_owned(),
    }
}

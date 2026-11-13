//! Version-owned, realm-independent Service Worker routing rules.
//!
//! The binding validates URLPattern's native state and rejects patterns with
//! author regexp groups before exporting these immutable component sources.
//! Matching never invokes a worker, an author getter, or an author regexp.

pub(crate) mod race;

use std::sync::{Arc, OnceLock};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use url::Url;

pub(crate) const REQUEST_MODES: &[&str] = &["navigate", "same-origin", "no-cors", "cors"];
// Fetch's RequestDestination enum, including text and excluding the internal
// serviceworker and webidentity destinations that cannot reach a FetchEvent.
pub(crate) const REQUEST_DESTINATIONS: &[&str] = &[
    "",
    "audio",
    "audioworklet",
    "document",
    "embed",
    "font",
    "frame",
    "iframe",
    "image",
    "json",
    "manifest",
    "object",
    "paintworklet",
    "report",
    "script",
    "sharedworker",
    "style",
    "text",
    "track",
    "video",
    "worker",
    "xslt",
];

pub(crate) fn normalize_request_method(value: &str) -> Result<String, String> {
    if http::Method::from_bytes(value.as_bytes()).is_err()
        || ["CONNECT", "TRACE", "TRACK"]
            .iter()
            .any(|method| value.eq_ignore_ascii_case(method))
    {
        return Err("Invalid or forbidden router request method".to_owned());
    }
    let upper = value.to_ascii_uppercase();
    Ok(
        if ["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"].contains(&upper.as_str()) {
            upper
        } else {
            value.to_owned()
        },
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RouterUrlPattern {
    sources: [String; 8],
    ignore_case: [bool; 8],
    #[serde(skip)]
    compiled: Arc<OnceLock<Option<[Regex; 8]>>>,
}

impl PartialEq for RouterUrlPattern {
    fn eq(&self, other: &Self) -> bool {
        self.sources == other.sources && self.ignore_case == other.ignore_case
    }
}

impl Eq for RouterUrlPattern {}

impl RouterUrlPattern {
    pub(crate) fn new(sources: [String; 8], ignore_case: [bool; 8]) -> Result<Self, String> {
        let value = Self {
            sources,
            ignore_case,
            compiled: Arc::default(),
        };
        value
            .compiled()
            .ok_or_else(|| "Invalid static router URLPattern component".to_owned())?;
        Ok(value)
    }

    fn compiled(&self) -> Option<&[Regex; 8]> {
        self.compiled
            .get_or_init(|| {
                self.sources
                    .iter()
                    .zip(self.ignore_case)
                    .map(|(source, ignore_case)| {
                        RegexBuilder::new(source)
                            .case_insensitive(ignore_case)
                            .build()
                            .ok()
                    })
                    .collect::<Option<Vec<_>>>()?
                    .try_into()
                    .ok()
            })
            .as_ref()
    }

    pub(crate) fn matches(&self, url: &Url) -> bool {
        let Ok(Some((input, _))) = urlpattern::quirks::process_match_input(
            urlpattern::quirks::StringOrInit::String(url.as_str().into()),
            None,
        ) else {
            return false;
        };
        let Some(input) = urlpattern::quirks::parse_match_input(input) else {
            return false;
        };
        let Some(compiled) = self.compiled() else {
            return false;
        };
        compiled
            .iter()
            .zip([
                input.protocol,
                input.username,
                input.password,
                input.hostname,
                input.port,
                input.pathname,
                input.search,
                input.hash,
            ])
            .all(|(pattern, value)| pattern.is_match(&value))
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RouterCondition {
    pub(crate) url_pattern: Option<RouterUrlPattern>,
    pub(crate) request_method: Option<String>,
    pub(crate) request_mode: Option<String>,
    pub(crate) request_destination: Option<String>,
    pub(crate) running: Option<bool>,
    pub(crate) or: Option<Vec<Self>>,
    pub(crate) not: Option<Box<Self>>,
}

impl RouterCondition {
    pub(crate) fn verify(&self) -> bool {
        if self
            .url_pattern
            .as_ref()
            .is_some_and(|pattern| pattern.compiled().is_none())
            || self
                .request_method
                .as_ref()
                .is_some_and(|method| normalize_request_method(method).as_ref() != Ok(method))
            || self
                .request_mode
                .as_deref()
                .is_some_and(|mode| !REQUEST_MODES.contains(&mode))
            || self
                .request_destination
                .as_deref()
                .is_some_and(|destination| !REQUEST_DESTINATIONS.contains(&destination))
        {
            return false;
        }
        let ordinary = self.url_pattern.is_some()
            || self.request_method.is_some()
            || self.request_mode.is_some()
            || self.request_destination.is_some()
            || self.running.is_some();
        if let Some(children) = &self.or {
            return !ordinary && self.not.is_none() && children.iter().all(Self::verify);
        }
        if let Some(child) = &self.not {
            return !ordinary && child.verify();
        }
        ordinary
    }

    pub(crate) fn matches(
        &self,
        url: &Url,
        method: &str,
        mode: &str,
        destination: &str,
        running: bool,
    ) -> bool {
        if let Some(children) = &self.or {
            return children
                .iter()
                .any(|child| child.matches(url, method, mode, destination, running));
        }
        if let Some(child) = &self.not {
            return !child.matches(url, method, mode, destination, running);
        }
        self.url_pattern
            .as_ref()
            .is_none_or(|pattern| pattern.matches(url))
            && self
                .request_method
                .as_ref()
                .is_none_or(|expected| expected == method)
            && self
                .request_mode
                .as_ref()
                .is_none_or(|expected| expected == mode)
            && self
                .request_destination
                .as_ref()
                .is_none_or(|expected| expected == destination)
            && self.running.is_none_or(|expected| expected == running)
    }

    fn count_within_limit(&self, remaining: &mut usize, depth: usize) -> bool {
        let Some(next) = remaining.checked_sub(1) else {
            return false;
        };
        *remaining = next;
        if next == 0 || depth == 0 {
            return false;
        }
        if let Some(children) = &self.or {
            return children
                .iter()
                .all(|child| child.count_within_limit(remaining, depth - 1));
        }
        self.not
            .as_ref()
            .is_none_or(|child| child.count_within_limit(remaining, depth - 1))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum RouterSource {
    Network,
    FetchEvent,
    Cache { name: Option<Vec<u16>> },
    RaceNetworkAndFetchHandler,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RouterRule {
    pub(crate) condition: RouterCondition,
    pub(crate) source: RouterSource,
}

pub(crate) fn within_registration_limit(rules: &[RouterRule]) -> bool {
    let mut remaining = 1024;
    rules
        .iter()
        .all(|rule| rule.condition.count_within_limit(&mut remaining, 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(value: &str) -> RouterCondition {
        RouterCondition {
            request_method: Some(value.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn conditions_conjoin_fields_and_preserve_nested_boolean_meaning() {
        let url = Url::parse("https://router.test/path").unwrap();
        let condition = RouterCondition {
            request_method: Some("GET".to_owned()),
            request_mode: Some("cors".to_owned()),
            request_destination: Some("script".to_owned()),
            running: Some(false),
            ..Default::default()
        };
        assert!(condition.verify());
        assert!(condition.matches(&url, "GET", "cors", "script", false));
        assert!(!condition.matches(&url, "GET", "cors", "script", true));
        assert!(!condition.matches(&url, "GET", "cors", "image", false));
        assert!(!condition.matches(&url, "POST", "cors", "script", false));
        let alternatives = RouterCondition {
            or: Some(vec![method("POST"), method("GET")]),
            ..Default::default()
        };
        assert!(alternatives.verify());
        assert!(alternatives.matches(&url, "POST", "cors", "", true));
        let negated = RouterCondition {
            not: Some(Box::new(alternatives)),
            ..Default::default()
        };
        assert!(!negated.matches(&url, "GET", "cors", "", true));
        assert!(negated.matches(&url, "PUT", "cors", "", true));
        assert!(!RouterCondition::default().verify());
        assert!(
            !RouterCondition {
                or: Some(vec![]),
                ..method("GET")
            }
            .verify()
        );
    }

    #[test]
    fn quota_counts_all_nested_conditions_across_all_calls_rules() {
        let rule = RouterRule {
            condition: method("GET"),
            source: RouterSource::Network,
        };
        assert!(within_registration_limit(&vec![rule.clone(); 1023]));
        assert!(!within_registration_limit(&vec![rule.clone(); 1024]));
        let mut nested = method("GET");
        for _ in 0..9 {
            nested = RouterCondition {
                not: Some(Box::new(nested)),
                ..Default::default()
            };
        }
        assert!(within_registration_limit(&[RouterRule {
            condition: nested.clone(),
            ..rule.clone()
        }]));
        assert!(!within_registration_limit(&[RouterRule {
            condition: RouterCondition {
                not: Some(Box::new(nested)),
                ..Default::default()
            },
            ..rule
        }]));
    }

    #[test]
    fn immutable_pattern_survives_storage_and_matches_url_components() {
        let sources = [
            "^https$",
            "^$",
            "^$",
            "^example\\.test$",
            "^$",
            "^/assets/[^/]+\\.js$",
            "^.*$",
            "^$",
        ]
        .map(str::to_owned);
        let pattern = RouterUrlPattern::new(
            sources,
            [false, false, false, false, false, true, false, false],
        )
        .unwrap();
        assert!(pattern.matches(&Url::parse("https://example.test/assets/ENTRY.JS?x=1").unwrap()));
        assert!(!pattern.matches(&Url::parse("https://elsewhere.test/assets/entry.js").unwrap()));
        assert!(
            !pattern.matches(&Url::parse("https://example.test/assets/deep/entry.js").unwrap())
        );
        let restored: RouterUrlPattern =
            serde_json::from_str(&serde_json::to_string(&pattern).unwrap()).unwrap();
        assert_eq!(restored, pattern);
        assert!(restored.matches(&Url::parse("https://example.test/assets/entry.js").unwrap()));
    }
}

use sfv::{BareItem, Dictionary, List, ListEntry, Parser};
use url::Url;
use urlpattern::{UrlPattern, UrlPatternInit, UrlPatternMatchInput};

#[derive(Debug)]
pub(crate) struct RegistrationOptions {
    pattern: UrlPattern,
    match_text: String,
    destinations: Vec<String>,
    id: String,
}

impl RegistrationOptions {
    pub(crate) fn parse(url: &Url, headers: &[(String, Vec<u8>)]) -> Option<Self> {
        let values = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("use-as-dictionary"))
            .map(|(_, value)| std::str::from_utf8(value).ok())
            .collect::<Option<Vec<_>>>()?;
        if values.is_empty() {
            return None;
        }
        let combined = values.join(", ");
        if combined.len() > 16 * 1024 {
            return None;
        }
        let dictionary: Dictionary = Parser::new(&combined).parse().ok()?;
        let ListEntry::Item(item) = dictionary.get("match")? else {
            return None;
        };
        let BareItem::String(value) = &item.bare_item else {
            return None;
        };
        let match_text = value.as_str().to_owned();
        if match_text.len() > 4096 {
            return None;
        }
        let init = UrlPatternInit::parse_constructor_string::<regex::Regex>(
            &match_text,
            Some(url.clone()),
        )
        .ok()?;
        let pattern = UrlPattern::parse(init, Default::default()).ok()?;
        if pattern.has_regexp_groups() {
            return None;
        }
        let id = match dictionary.get("id") {
            None => String::new(),
            Some(ListEntry::Item(item)) => item.bare_item.as_string()?.as_str().to_owned(),
            Some(ListEntry::InnerList(_)) => return None,
        };
        if id.len() > 1024 {
            return None;
        }
        if let Some(kind) = dictionary.get("type") {
            let ListEntry::Item(item) = kind else {
                return None;
            };
            if item.bare_item.as_token()?.as_str() != "raw" {
                return None;
            }
        }
        let destinations = match dictionary.get("match-dest") {
            None => Vec::new(),
            Some(ListEntry::InnerList(list)) => list
                .items
                .iter()
                .map(|item| Some(item.bare_item.as_string()?.as_str().to_owned()))
                .collect::<Option<Vec<_>>>()?,
            Some(ListEntry::Item(_)) => return None,
        };
        Some(Self {
            pattern,
            match_text,
            destinations,
            id,
        })
    }

    pub(crate) fn matches(&self, url: &Url, destination: &str) -> bool {
        (self.destinations.is_empty() || self.destinations.iter().any(|value| value == destination))
            && self
                .pattern
                .test(UrlPatternMatchInput::Url(url.clone()))
                .unwrap_or(false)
    }

    pub(crate) fn match_length(&self) -> usize {
        self.match_text.len()
    }

    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        self.match_text == other.match_text && self.destinations == other.destinations
    }

    pub(crate) fn id_header(&self) -> Option<Vec<u8>> {
        (!self.id.is_empty()).then(|| {
            format!("\"{}\"", self.id.replace('\\', "\\\\").replace('"', "\\\"")).into_bytes()
        })
    }
}

pub(super) fn clears_dictionaries(headers: &[(String, Vec<u8>)]) -> bool {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("clear-site-data"))
        .filter_map(|(_, value)| std::str::from_utf8(value).ok())
        .filter_map(|value| Parser::new(value).parse::<List>().ok())
        .flatten()
        .any(|entry| match entry {
            ListEntry::Item(item) => item
                .bare_item
                .as_string()
                .is_some_and(|value| matches!(value.as_str(), "cache" | "cookies" | "*")),
            ListEntry::InnerList(_) => false,
        })
}

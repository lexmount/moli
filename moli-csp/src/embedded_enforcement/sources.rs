use super::super::{
    CspSchemeMatch, csp_host_part_matches, csp_path_part_matches, csp_scheme_match,
    default_port_for_scheme, parse_host_source,
};
use url::Url;

/// Preserve source-expression syntax for CSP subsumption. In particular,
/// omitted and explicit default ports have distinct scheme-upgrade rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UrlSource {
    scheme: String,
    host: Option<String>,
    port: Port,
    path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Port {
    Any,
    Number(u16),
    Unspecified,
}

impl UrlSource {
    fn scheme(scheme: &str) -> Self {
        Self {
            scheme: scheme.to_ascii_lowercase(),
            host: None,
            port: Port::Any,
            path: String::new(),
        }
    }

    pub(super) fn parse(value: &str, origin: &Url) -> Vec<Self> {
        if value == "*" {
            return ["ftp", "http", "ws", origin.scheme()]
                .into_iter()
                .map(Self::scheme)
                .collect();
        }
        if super::super::csp_keyword_eq(value, "self") {
            if matches!(origin.origin(), url::Origin::Tuple(..))
                && let Some(host) = origin.host_str()
            {
                return vec![Self {
                    scheme: origin.scheme().to_owned(),
                    host: Some(host.to_ascii_lowercase()),
                    port: origin
                        .port_or_known_default()
                        .map_or(Port::Unspecified, Port::Number),
                    path: String::new(),
                }];
            }
            return Vec::new();
        }
        if let Some(scheme) = value.strip_suffix(':') {
            let mut bytes = scheme.bytes();
            if bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
                && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
            {
                return vec![Self::scheme(scheme)];
            }
            return Vec::new();
        }
        let Some(source) = parse_host_source(value) else {
            return Vec::new();
        };
        let scheme = source
            .scheme
            .unwrap_or(origin.scheme())
            .to_ascii_lowercase();
        let port = match source.port {
            Some("*") => Port::Any,
            Some(value) => match value.parse() {
                Ok(port) => Port::Number(port),
                Err(_) => return Vec::new(),
            },
            None => Port::Unspecified,
        };
        vec![Self {
            scheme,
            host: Some(source.host.to_ascii_lowercase()),
            port,
            path: source.path.unwrap_or_default().to_owned(),
        }]
    }

    pub(super) fn subsumes(&self, other: &Self) -> bool {
        csp_scheme_match(&self.scheme, &other.scheme) != CspSchemeMatch::NotMatching
            && host_subsumes(self.host.as_deref(), other.host.as_deref())
            && self.port_subsumes(other)
            && csp_path_part_matches(&self.path, &other.path)
    }

    pub(super) fn intersect(&self, other: &Self) -> Option<Self> {
        let scheme = if csp_scheme_match(&self.scheme, &other.scheme) != CspSchemeMatch::NotMatching
        {
            other.scheme.clone()
        } else if csp_scheme_match(&other.scheme, &self.scheme) != CspSchemeMatch::NotMatching {
            self.scheme.clone()
        } else if csp_scheme_match(&self.scheme, "https") != CspSchemeMatch::NotMatching
            && csp_scheme_match(&other.scheme, "https") != CspSchemeMatch::NotMatching
        {
            // HTTP and WSS both permit HTTPS under CSP's scheme-part rules.
            // Neither source subsumes the other, but their intersection is
            // not empty.
            "https".to_owned()
        } else {
            return None;
        };
        // A scheme-only source adds no host, port or path restriction.
        if self.host.is_none() || other.host.is_none() {
            let mut result = if self.host.is_none() {
                other.clone()
            } else {
                self.clone()
            };
            result.scheme = scheme.clone();
            return Some(result);
        }
        let host = if host_subsumes(self.host.as_deref(), other.host.as_deref()) {
            other.host.clone()
        } else if host_subsumes(other.host.as_deref(), self.host.as_deref()) {
            self.host.clone()
        } else {
            return None;
        };
        let port = if self.port == Port::Any {
            other.port
        } else if other.port == Port::Any {
            self.port
        } else if self.port_subsumes(other)
            && (self.port == Port::Unspecified || other.port != Port::Unspecified)
        {
            other.port
        } else if other.port_subsumes(self) {
            self.port
        } else {
            return None;
        };
        let path = if csp_path_part_matches(&self.path, &other.path) {
            other.path.clone()
        } else if csp_path_part_matches(&other.path, &self.path) {
            self.path.clone()
        } else {
            return None;
        };
        Some(Self {
            scheme: scheme.clone(),
            host,
            port,
            path,
        })
    }

    fn port_subsumes(&self, other: &Self) -> bool {
        if self.port == Port::Any {
            return true;
        }
        if other.port == Port::Any {
            return false;
        }
        if self.port == other.port {
            return true;
        }
        let default = default_port_for_scheme(&other.scheme);
        if (self.port == Port::Unspecified
            && other.port == default.map_or(Port::Unspecified, Port::Number))
            || (other.port == Port::Unspecified
                && self.port == default.map_or(Port::Unspecified, Port::Number))
        {
            return true;
        }
        let source_port = match self.port {
            Port::Number(port) => Some(port),
            _ => default_port_for_scheme(&self.scheme),
        };
        let other_port = match other.port {
            Port::Number(port) => Some(port),
            _ => default,
        };
        source_port == Some(80) && other_port == Some(443)
    }
}

fn host_subsumes(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (None, _) => true,
        (Some(a), Some(b)) => csp_host_part_matches(a, b),
        (Some(_), None) => false,
    }
}

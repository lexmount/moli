use std::borrow::Cow;
use url::{Host, Url};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TupleOrigin {
    ascii_serialization: String,
    unicode_serialization: String,
}

impl TupleOrigin {
    pub fn ascii_serialization(&self) -> &str {
        &self.ascii_serialization
    }

    pub fn unicode_serialization(&self) -> &str {
        &self.unicode_serialization
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebOrigin {
    Tuple(TupleOrigin),
    Opaque,
}

impl WebOrigin {
    pub fn from_url(url: &Url) -> Self {
        let Some(origin_url) = tuple_origin_url(url) else {
            return Self::Opaque;
        };
        let origin = origin_url.origin();
        if origin.is_tuple() {
            Self::Tuple(TupleOrigin {
                ascii_serialization: origin.ascii_serialization(),
                unicode_serialization: origin.unicode_serialization(),
            })
        } else {
            Self::Opaque
        }
    }

    pub fn from_ascii_serialization(serialized: &str) -> Self {
        Url::parse(serialized)
            .ok()
            .map_or(Self::Opaque, |url| Self::from_url(&url))
    }

    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }

    pub fn ascii_serialization(&self) -> &str {
        match self {
            Self::Tuple(origin) => origin.ascii_serialization(),
            Self::Opaque => "null",
        }
    }

    pub fn unicode_serialization(&self) -> &str {
        match self {
            Self::Tuple(origin) => origin.unicode_serialization(),
            Self::Opaque => "null",
        }
    }

    pub fn same_origin(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Tuple(left), Self::Tuple(right)) => left == right,
            _ => false,
        }
    }

    pub fn same_origin_url(&self, url: &Url) -> bool {
        self.same_origin(&Self::from_url(url))
    }
}

pub fn tuple_origin_url(url: &Url) -> Option<Cow<'_, Url>> {
    let origin_url = if url.scheme() == "blob" {
        let inner = Url::parse(url.path()).ok()?;
        // rust-url derives blob origins from any parseable path URL, while the
        // URL Standard limits this fallback to these three schemes.
        if !matches!(inner.scheme(), "http" | "https" | "file") {
            return None;
        }
        Cow::Owned(inner)
    } else {
        Cow::Borrowed(url)
    };
    origin_url.origin().is_tuple().then_some(origin_url)
}

pub fn origin_ascii_serialization(url: &Url) -> String {
    WebOrigin::from_url(url).ascii_serialization().to_owned()
}

pub fn origin_unicode_serialization(url: &Url) -> String {
    WebOrigin::from_url(url).unicode_serialization().to_owned()
}

pub fn is_opaque_origin(url: &Url) -> bool {
    WebOrigin::from_url(url).is_opaque()
}

pub fn same_origin(left: &Url, right: &Url) -> bool {
    WebOrigin::from_url(left).same_origin(&WebOrigin::from_url(right))
}

pub fn parsed_same_origin(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    same_origin(&left, &right)
}

pub fn is_about_blank(url: &Url) -> bool {
    url.scheme() == "about" && url.path().eq_ignore_ascii_case("blank")
}

pub fn is_potentially_trustworthy_url(url: &Url) -> bool {
    if url.scheme() == "blob" {
        return tuple_origin_url(url)
            .is_some_and(|origin_url| is_potentially_trustworthy_url(&origin_url));
    }
    match url.scheme() {
        "https" | "wss" | "file" => true,
        "http" | "ws" => url.host().is_some_and(is_potentially_trustworthy_host),
        _ => false,
    }
}

fn is_potentially_trustworthy_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.');
            domain.eq_ignore_ascii_case("localhost")
                || domain.to_ascii_lowercase().ends_with(".localhost")
        }
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

pub fn origin_ascii_serialization_with_about_blank_inheritance(
    url: &Url,
    inherited_origin: &str,
) -> String {
    if is_about_blank(url) {
        inherited_origin.to_owned()
    } else {
        origin_ascii_serialization(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(input: &str) -> Url {
        Url::parse(input).unwrap()
    }

    #[test]
    fn tuple_origin_serializes_default_and_explicit_ports() {
        assert_eq!(
            origin_ascii_serialization(&url("https://example.test:443/path")),
            "https://example.test"
        );
        assert_eq!(
            origin_ascii_serialization(&url("https://example.test:444/path")),
            "https://example.test:444"
        );
    }

    #[test]
    fn opaque_origins_serialize_to_null_but_are_not_same_origin() {
        let left = url("data:text/html,a");
        let right = url("data:text/html,a");

        assert!(is_opaque_origin(&left));
        assert_eq!(origin_ascii_serialization(&left), "null");
        assert!(!same_origin(&left, &right));
    }

    #[test]
    fn same_origin_requires_tuple_match() {
        assert!(same_origin(
            &url("https://example.test/a"),
            &url("https://example.test/b")
        ));
        assert!(!same_origin(
            &url("https://example.test/a"),
            &url("http://example.test/a")
        ));
        assert!(!same_origin(
            &url("https://example.test/a"),
            &url("https://other.test/a")
        ));
    }

    #[test]
    fn web_origin_compares_directly_with_urls() {
        let origin = WebOrigin::from_url(&url("https://example.test/document"));

        assert!(origin.same_origin_url(&url("https://example.test/resource")));
        assert!(!origin.same_origin_url(&url("https://other.test/resource")));
        assert!(!WebOrigin::Opaque.same_origin_url(&url("https://example.test/resource")));
    }

    #[test]
    fn web_origin_rehydrates_tuple_serializations_and_keeps_null_opaque() {
        assert_eq!(
            WebOrigin::from_ascii_serialization("https://example.test:8443"),
            WebOrigin::from_url(&url("https://example.test:8443/path"))
        );
        assert_eq!(
            WebOrigin::from_ascii_serialization("null"),
            WebOrigin::Opaque
        );
    }

    #[test]
    fn blob_url_path_fallback_only_accepts_http_https_and_file_schemes() {
        let blob = url("blob:https://example.test/object-1");

        assert_eq!(origin_ascii_serialization(&blob), "https://example.test");
        assert!(same_origin(&blob, &url("https://example.test/path")));
        assert!(!same_origin(&blob, &url("https://other.test/path")));
        for input in [
            "blob:null/object-1",
            "blob:blob:https://example.test/object-1",
            "blob:ftp://example.test/object-1",
            "blob:ws://example.test/object-1",
            "blob:wss://example.test/object-1",
        ] {
            let blob = url(input);
            assert!(tuple_origin_url(&blob).is_none(), "{input}");
            assert!(is_opaque_origin(&blob), "{input}");
            assert_eq!(origin_ascii_serialization(&blob), "null", "{input}");
        }
    }

    #[test]
    fn about_blank_can_inherit_a_creator_origin_at_call_sites() {
        assert!(is_about_blank(&url("about:blank")));
        assert!(is_about_blank(&url("about:blank#section")));
        assert!(is_about_blank(&url("about:blank?query")));
        assert!(is_about_blank(&url("about:BLANK")));
        assert!(is_about_blank(&url("about:Blank?query#section")));
        assert!(is_about_blank(&url("ABOUT:bLaNk")));
        assert!(!is_about_blank(&url("about:blank-page")));

        assert_eq!(
            origin_ascii_serialization_with_about_blank_inheritance(
                &url("about:blank"),
                "https://creator.test"
            ),
            "https://creator.test"
        );
        assert_eq!(
            origin_ascii_serialization_with_about_blank_inheritance(
                &url("about:blank#fragment"),
                "https://creator.test"
            ),
            "https://creator.test"
        );
        assert_eq!(
            origin_ascii_serialization_with_about_blank_inheritance(
                &url("about:blank?query"),
                "https://creator.test"
            ),
            "https://creator.test"
        );
        assert_eq!(
            origin_ascii_serialization_with_about_blank_inheritance(
                &url("data:text/html,a"),
                "https://creator.test"
            ),
            "null"
        );
    }

    #[test]
    fn potentially_trustworthy_urls_cover_secure_and_loopback_origins() {
        assert!(is_potentially_trustworthy_url(&url(
            "https://example.test/"
        )));
        assert!(is_potentially_trustworthy_url(&url(
            "wss://example.test/ws"
        )));
        assert!(is_potentially_trustworthy_url(&url("file:///tmp/a.html")));
        assert!(is_potentially_trustworthy_url(&url("http://localhost/")));
        assert!(is_potentially_trustworthy_url(&url(
            "http://app.localhost/"
        )));
        assert!(is_potentially_trustworthy_url(&url("http://127.0.0.1/")));
        assert!(is_potentially_trustworthy_url(&url("http://[::1]/")));
        assert!(is_potentially_trustworthy_url(&url(
            "blob:https://example.test/object-1"
        )));
        assert!(is_potentially_trustworthy_url(&url(
            "blob:http://localhost/object-1"
        )));
        assert!(!is_potentially_trustworthy_url(&url(
            "http://example.test/"
        )));
        assert!(!is_potentially_trustworthy_url(&url(
            "ws://example.test/ws"
        )));
        assert!(!is_potentially_trustworthy_url(&url(
            "data:text/html,hello"
        )));
        assert!(!is_potentially_trustworthy_url(&url(
            "blob:http://example.test/object-1"
        )));
        assert!(!is_potentially_trustworthy_url(&url(
            "blob:data:text/html,hello"
        )));
    }
}

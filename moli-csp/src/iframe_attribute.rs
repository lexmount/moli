//! The iframe CSP attribute crosses an HTTP header boundary. Validate its
//! transport and directive syntax before retaining a navigation's immutable
//! policy value. Source-list interpretation remains the CSP parser's job.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IframeCspAttribute(String);

impl IframeCspAttribute {
    pub fn parse(value: &str) -> Option<Self> {
        // Reject header injection and non-ASCII input before trimming. An
        // invalid directive invalidates the attribute, rather than merely
        // disappearing as it would while parsing a response CSP header.
        // https://w3c.github.io/webappsec-csp/embedded/#csp-attribute
        if value.len() > 4096
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t' | b'\x0c'))
            || value.contains(',')
        {
            return None;
        }
        let value = value.trim_ascii();
        if value.is_empty() {
            return None;
        }
        for directive in value.split(';') {
            let Some(name) = directive.split_ascii_whitespace().next() else {
                continue;
            };
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                || name.eq_ignore_ascii_case("report-uri")
                || name.eq_ignore_ascii_case("report-to")
            {
                return None;
            }
        }
        Some(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iframe_csp_attribute_keeps_forward_compatible_syntax_without_header_injection() {
        for policy in [
            "script-src 'unsafe-inline'",
            "completely wrong csp",
            "invalid-policy-name http://example.com",
            "media-src 'non'",
            "script-src 'unsafe-inline' 127.0.0.1:8000/path?query=string",
            "script-src 'unsafe-inline' 'self' object-src 'self' style-src *",
            "script-src\t'unsafe-inline'",
            "script-src\x0c'unsafe-inline'",
            "a;;b",
            ";a;",
        ] {
            assert_eq!(IframeCspAttribute::parse(policy).unwrap().as_str(), policy);
        }
        for policy in [
            "",
            " \t ",
            "script-src 'self', object-src 'none'",
            "media-src%20127.0.0.1%3A8000",
            "script-src 'unsafe-inline'; report-uri /report",
            "script-src 'unsafe-inline'; REPORT-TO endpoint",
            "report-uri; report-uri /report",
            "script-src\r\nInjected: value",
            "script-src\nInjected: value",
            "script-src\rInjected: value",
            "script-src\0'none'",
            "script-src\x0b'none'",
            "script-src '\x7f'",
            "script-src é",
            "script-src \u{fffd}",
        ] {
            assert!(IframeCspAttribute::parse(policy).is_none(), "{policy:?}");
        }
        assert_eq!(IframeCspAttribute::parse("  a \t").unwrap().as_str(), "a");
        let limit = format!("a {}", "x".repeat(4094));
        assert_eq!(IframeCspAttribute::parse(&limit).unwrap().as_str(), limit);
        assert!(IframeCspAttribute::parse(&(limit + "x")).is_none());
    }
}

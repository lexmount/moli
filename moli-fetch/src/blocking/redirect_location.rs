use anyhow::{Context, Result};
use url::{ParseError, Url};

/// Check network redirect syntax before publishing a response. The boolean
/// indicates a redirect with Location; it never authorizes following a target.
pub(crate) fn validate_network_redirect_location(
    base: &Url,
    status: u16,
    headers: &[(String, Vec<u8>)],
) -> Result<bool> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Ok(false);
    }
    let Some(location) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| crate::encode_redirect_location(value))
    else {
        return Ok(false);
    };
    match base
        .join(&location)
        .or_else(|error| Url::parse(&location).map_err(|_| error))
    {
        Ok(_) => Ok(true),
        // GURL's ASCII host canonicalization is less restrictive than WHATWG
        // IDNA processing. These hosts are observable in manual responses even
        // though they cannot be represented by url::Url. Keep follow policy in
        // the existing strict target resolver, independently of this check.
        Err(ParseError::IdnaError) if chromium_network_authority(&location) => Ok(true),
        Err(error) => Err(error).with_context(|| crate::error::InvalidRedirectLocation {
            location: location.into_owned(),
            base: base.clone(),
        }),
    }
}

fn chromium_network_authority(location: &str) -> bool {
    let cleaned;
    let location = if location.contains(['\t', '\r', '\n']) {
        cleaned = location.replace(['\t', '\r', '\n'], "");
        cleaned.as_str()
    } else {
        location
    };
    let location = location.trim_matches(|c| c <= '\u{20}');
    let slash = |c| matches!(c, '/' | '\\');
    let authority = if location.chars().take(2).count() == 2 && location.chars().take(2).all(slash)
    {
        location.trim_start_matches(slash)
    } else if let Some((scheme, tail)) = location.split_once(':') {
        if !["http", "https", "ftp", "ws", "wss", "file"]
            .iter()
            .any(|known| scheme.eq_ignore_ascii_case(known))
        {
            return false;
        }
        tail.trim_start_matches(slash)
    } else {
        return false;
    };
    let authority = authority
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some((host, port)) = host_port.rsplit_once(':') {
        if !port.is_empty()
            && (!port.bytes().all(|b| b.is_ascii_digit()) || port.parse::<u16>().is_err())
        {
            return false;
        }
        host
    } else {
        host_port
    };
    let decoded = percent_encoding::percent_decode_str(host).collect::<Vec<_>>();
    let Ok(host) = std::str::from_utf8(&decoded) else {
        return false;
    };
    // Initial unescaping must succeed independently of IDNA normalization.
    // In particular, %25 leaves a forbidden literal percent; a later pass
    // must not turn that failure into an accepted space, letter or delimiter.
    if host
        .bytes()
        .any(|b| b.is_ascii() && !allowed_ascii_host_byte(b))
    {
        return false;
    }
    let ascii;
    let host = if host.is_ascii() {
        host
    } else {
        // Chromium escapes spaces/asterisks before IDNA conversion, then
        // validates the resulting ASCII host after decoding escapes again.
        // Keep Unicode validation (including joiners/bidi) enabled; only its
        // ASCII deny list differs from the WHATWG URL parser's.
        let escaped = host.replace(' ', "%20").replace('*', "%2A");
        let Ok(converted) =
            idna::domain_to_ascii_cow(escaped.as_bytes(), idna::AsciiDenyList::EMPTY)
        else {
            return false;
        };
        let Ok(decoded) = percent_encoding::percent_decode_str(&converted).decode_utf8() else {
            return false;
        };
        ascii = decoded.into_owned();
        ascii.as_str()
    };
    // Match Chromium's ASCII host character table. Colon/brackets additionally
    // belong to IP-literal parsing, so they cannot recover an IDNA failure.
    if host.is_empty() || !host.bytes().all(allowed_ascii_host_byte) {
        return false;
    }
    // Ignore one root dot. Any further trailing dot leaves an empty final
    // label, which must not expose a preceding numeric label to IPv4 parsing.
    let last = host
        .strip_suffix('.')
        .unwrap_or(host)
        .rsplit('.')
        .next()
        .unwrap_or_default();
    // Numeric endings enter IPv4 parsing, where a malformed preceding label
    // remains an invalid address rather than an opaque DNS name.
    !(!last.is_empty() && last.bytes().all(|b| b.is_ascii_digit())
        || last
            .strip_prefix("0x")
            .or_else(|| last.strip_prefix("0X"))
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit())))
}

fn allowed_ascii_host_byte(byte: u8) -> bool {
    matches!(byte,
        b' ' | b'!' | b'"' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+'
        | b',' | b'-' | b'.' | b'0'..=b'9' | b';' | b'=' | b'A'..=b'Z'
        | b'_' | b'`' | b'a'..=b'z' | b'{' | b'}' | b'~'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_authorities_preserve_empty_final_labels_before_ipv4_detection() {
        let base = Url::parse("http://source.test/page").unwrap();
        for (location, valid) in [
            ("http://xn--a.123/", false),
            ("http://xn--a.123./", false),
            ("http://xn--a.123../", true),
            ("http://xn--a.123.../", true),
            ("http://xn--a.0x7f./", false),
            ("http://xn--a.0x7f../", true),
            ("http://example%20host.123./", false),
            ("http://example%20host.123../", true),
        ] {
            assert_eq!(
                validate_network_redirect_location(
                    &base,
                    302,
                    &[("location".into(), location.as_bytes().to_vec())],
                )
                .is_ok(),
                valid,
                "{location}",
            );
        }
    }

    #[test]
    fn unicode_network_authorities_do_not_decode_nested_percent_escapes() {
        let base = Url::parse("http://source.test/page").unwrap();
        for (location, valid) in [
            ("http://caf%C3%A9%20domain.test/", true),
            ("http://caf%C3%A9%2520domain.test/", false),
            ("http://%2541.%C3%A9.test/", false),
            ("http://caf%C3%A9%252Adomain.test/", false),
            ("//caf%C3%A9%2520domain.test/", false),
            // Unicode normalization can legitimately introduce an escape.
            // Distinguish that from a forbidden % left by initial unescaping.
            ("http://%EF%BC%8541.%C3%A9.test/", true),
            ("http://%EF%B9%AA41.%C3%A9.test/", true),
        ] {
            assert_eq!(
                validate_network_redirect_location(
                    &base,
                    302,
                    &[("location".into(), location.as_bytes().to_vec())]
                )
                .is_ok(),
                valid,
                "{location}"
            );
        }
    }

    #[test]
    fn network_redirect_validation_preserves_chromium_ascii_hosts() {
        let base = Url::parse("http://source.test/page").unwrap();
        for target in [
            "http://example%20host.test/",
            "http://caf%C3%A9%20domain.test/",
            "//caf%C3%A9%20domain.test/",
            "http://caf%C3%A9%20domain.test:8080/",
            "http://%D7%90%D7%91%20.test/",
            "http://%D8%A7%D8%A8%20.test/",
            "http://caf%C3%A9%2Adomain.test/",
            "http://caf%C3%A9%EF%BC%85%32%30domain.test/",
            "http://xn--a.invalid/",
            "//xn--a.invalid/",
            "//example%20host.test/",
            "https://example%20host.test/",
            "http://xn--a.invalid:80/",
            "http://exa*mple.test/",
        ] {
            assert!(
                validate_network_redirect_location(
                    &base,
                    302,
                    &[("location".into(), target.as_bytes().to_vec())]
                )
                .unwrap(),
                "{target}"
            );
        }
        for target in [
            "http://[",
            "http://[]/",
            "http://[::1",
            "http://%GG/",
            "http://%00/",
            "http://example%2Fhost.test/",
            "http://example%3Ahost.test/",
            "http://%FF/",
            "http://example.test:abc/",
            "http://example.test:65536/",
            "http:///",
            "//",
            "http://xn--a.invalid:abc/",
            "http://example%20host.test:abc/",
            "http://caf%C3%A9%2Fdomain.test/",
            "http://caf%C3%A9%20domain.test:abc/",
            "http://caf%C3%A9%EF%BC%85%30%30domain.test/",
            "http://caf%FF%20domain.test/",
            "http://caf%C3%A9%E2%80%8D%20domain.test/",
            "http://%EF%BC%83caf%C3%A9%20domain.test/",
            "http://caf%C3%A9.xn--a.invalid/",
        ] {
            assert!(
                validate_network_redirect_location(
                    &base,
                    302,
                    &[("location".into(), target.as_bytes().to_vec())]
                )
                .is_err(),
                "{target}"
            );
        }
        assert!(!validate_network_redirect_location(&base, 302, &[]).unwrap());
        assert!(
            !validate_network_redirect_location(
                &base,
                200,
                &[("location".into(), b"http://[".to_vec())]
            )
            .unwrap()
        );
    }
}

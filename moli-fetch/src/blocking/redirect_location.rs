use anyhow::{Context, Result, anyhow};
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
        Err(ParseError::IdnaError) if chromium_ascii_authority(&location) => Ok(true),
        Err(error) => Err(error).with_context(|| {
            anyhow!("failed to resolve redirect location `{location}` from {base}")
        }),
    }
}

fn chromium_ascii_authority(location: &str) -> bool {
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
    let host = percent_encoding::percent_decode_str(host).collect::<Vec<_>>();
    // Match Chromium's ASCII host character table. Colon/brackets additionally
    // belong to IP-literal parsing, so they cannot recover an IDNA failure.
    if host.is_empty()
        || !host.iter().all(|b| {
            matches!(b,
                b' ' | b'!' | b'"' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+'
                | b',' | b'-' | b'.' | b'0'..=b'9' | b';' | b'=' | b'A'..=b'Z'
                | b'_' | b'`' | b'a'..=b'z' | b'{' | b'}' | b'~'
            )
        })
    {
        return false;
    }
    let host = std::str::from_utf8(&host).expect("ASCII checked above");
    let last = host
        .trim_end_matches('.')
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_redirect_validation_preserves_chromium_ascii_hosts() {
        let base = Url::parse("http://source.test/page").unwrap();
        for target in [
            "http://example%20host.test/",
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

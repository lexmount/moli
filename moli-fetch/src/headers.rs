use moli_web_mime::request_header_content_type_essence;

pub fn is_forbidden_request_header_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "accept-charset"
            | "accept-encoding"
            | "access-control-request-headers"
            | "access-control-request-method"
            | "connection"
            | "content-length"
            | "cookie"
            | "cookie2"
            | "date"
            | "dnt"
            | "expect"
            | "host"
            | "keep-alive"
            | "origin"
            | "referer"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "via"
    ) || name.starts_with("proxy-")
        || name.starts_with("sec-")
}

pub fn is_forbidden_response_header_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(name.as_str(), "set-cookie" | "set-cookie2")
}

pub fn is_forbidden_request_header_override_value(name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "x-http-method-override" | "x-http-method" | "x-method-override"
    ) {
        return false;
    }
    // Fetch's "get, decode, and split" keeps quoted strings intact, including
    // their quotes, so commas and method names inside them are ordinary data.
    let mut quoted = false;
    let mut escaped = false;
    value
        .split(|character| {
            if escaped {
                escaped = false;
                false
            } else if quoted && character == '\\' {
                escaped = true;
                false
            } else if character == '"' {
                quoted = !quoted;
                false
            } else {
                character == ',' && !quoted
            }
        })
        .any(|method| {
            matches!(
                method
                    .trim_matches(is_http_whitespace)
                    .to_ascii_uppercase()
                    .as_str(),
                "CONNECT" | "TRACE" | "TRACK"
            )
        })
}

pub fn is_no_cors_safelisted_request_header(name: &str, value: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept" | "accept-language" | "content-language" | "content-type"
    ) && is_cors_safelisted_request_header(name, value)
}

pub fn is_cors_safelisted_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "POST"
    )
}

pub fn cors_unsafe_request_header_names(request_headers: &[(String, String)]) -> Vec<String> {
    let mut unsafe_names = Vec::<String>::new();
    let mut potentially_unsafe_names = Vec::<String>::new();
    let mut safelist_value_size = 0usize;

    for (name, value) in request_headers {
        let lower = name.to_ascii_lowercase();
        if is_cors_safelisted_request_header(&lower, value) {
            safelist_value_size += value.len();
            if !potentially_unsafe_names
                .iter()
                .any(|existing| existing == &lower)
            {
                potentially_unsafe_names.push(lower);
            }
        } else if !unsafe_names.iter().any(|existing| existing == &lower) {
            unsafe_names.push(lower);
        }
    }

    if safelist_value_size > 1024 {
        for name in potentially_unsafe_names {
            if !unsafe_names.iter().any(|existing| existing == &name) {
                unsafe_names.push(name);
            }
        }
    }

    unsafe_names.sort();
    unsafe_names
}

pub fn is_cors_safelisted_request_header(name: &str, value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.to_ascii_lowercase().as_str() {
        "accept" => !value.bytes().any(is_cors_unsafe_request_header_byte),
        "accept-language" | "content-language" => value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b' ' | b'*' | b',' | b'-' | b'.' | b';' | b'=')
        }),
        "content-type" => is_cors_safelisted_request_content_type(value),
        "range" => is_cors_safelisted_request_range(value),
        _ => false,
    }
}

pub fn is_cors_safelisted_request_content_type(value: &str) -> bool {
    if value.bytes().any(is_cors_unsafe_request_header_byte) {
        return false;
    }
    let Some(essence) = request_header_content_type_essence(value) else {
        return false;
    };
    matches!(
        essence.as_str(),
        "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
    )
}

pub fn is_cors_safelisted_request_range(value: &str) -> bool {
    let Some(range) = value.trim().strip_prefix("bytes=") else {
        return false;
    };
    let mut parts = range.split('-');
    let Some(start) = parts.next() else {
        return false;
    };
    let Some(end) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let Ok(start) = start.parse::<u64>() else {
        return false;
    };
    end.is_empty() || end.parse::<u64>().is_ok_and(|end| start <= end)
}

pub fn is_cors_unsafe_request_header_byte(byte: u8) -> bool {
    matches!(
        byte,
        0x00..=0x08
            | 0x0a..=0x1f
            | 0x7f
            | b'"'
            | b'('
            | b')'
            | b':'
            | b'<'
            | b'>'
            | b'?'
            | b'@'
            | b'['
            | b'\\'
            | b']'
            | b'{'
            | b'}'
    )
}

fn is_http_whitespace(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_forbidden_request_header_names_and_method_overrides() {
        assert!(is_forbidden_request_header_name("Cookie"));
        assert!(is_forbidden_request_header_name("Proxy-Authorization"));
        assert!(is_forbidden_request_header_name("Sec-Fetch-Site"));
        assert!(!is_forbidden_request_header_name("X-Test"));

        assert!(is_forbidden_request_header_override_value(
            "x-http-method-override",
            "GET, track "
        ));
        assert!(is_forbidden_request_header_override_value(
            "x-method-override",
            "\tTRACE"
        ));
        assert!(!is_forbidden_request_header_override_value(
            "x-http-method",
            "GETTRACE"
        ));
        assert!(!is_forbidden_request_header_override_value(
            "x-http-method",
            "\",TRACE\","
        ));
    }

    #[test]
    fn method_override_filter_respects_quoted_list_members() {
        for name in [
            "X-HTTP-Method",
            "X-HTTP-Method-Override",
            "X-Method-Override",
        ] {
            for value in [
                r#""GET,TRACE,POST""#,
                r#""GET\",TRACK,POST""#,
                r#"prefix"one,CONNECT,two"suffix"#,
                r#""unterminated,TRACE"#,
                r#""TRACE""#,
            ] {
                assert!(
                    !is_forbidden_request_header_override_value(name, value),
                    "{name}: {value}"
                );
            }
            for value in [
                r#""GET,TRACE", TRACK"#,
                r#""GET\",TRACE", CONNECT"#,
                r#""GET\\", CONNECT"#,
                r#"prefix"CONNECT", TRACE"#,
            ] {
                assert!(
                    is_forbidden_request_header_override_value(name, value),
                    "{name}: {value}"
                );
            }
        }
    }

    #[test]
    fn no_cors_safelist_keeps_fetch_header_subset() {
        assert!(is_no_cors_safelisted_request_header("accept", "text/html"));
        assert!(!is_no_cors_safelisted_request_header("accept", "\""));
        assert!(is_no_cors_safelisted_request_header(
            "accept-language",
            "en-US, zh"
        ));
        assert!(!is_no_cors_safelisted_request_header(
            "accept-language",
            "@"
        ));
        assert!(is_no_cors_safelisted_request_header(
            "content-type",
            "text/plain;charset=UTF-8"
        ));
        assert!(is_no_cors_safelisted_request_header(
            "content-type",
            "Multipart/Form-Data; boundary=abc"
        ));
        assert!(is_no_cors_safelisted_request_header(
            "content-type",
            &format!("text/plain;{}", "s".repeat(116))
        ));
        assert!(!is_no_cors_safelisted_request_header(
            "content-type",
            "text/html"
        ));
        assert!(!is_no_cors_safelisted_request_header(
            "content-type",
            "text/plain;charset=UTF-8, text/plain"
        ));
        assert!(!is_no_cors_safelisted_request_header("range", "bytes=0-1"));
    }

    #[test]
    fn cors_safelist_accepts_single_range_values() {
        assert!(is_cors_safelisted_request_header("range", "bytes=100-200"));
        assert!(is_cors_safelisted_request_header("range", "bytes=200-"));
        assert!(!is_cors_safelisted_request_header("range", "bytes=200-100"));
        assert!(!is_cors_safelisted_request_header("range", "bytes=abc-def"));
        assert!(!is_cors_safelisted_request_header("range", ""));
    }

    #[test]
    fn cors_unsafe_request_header_names_sorts_dedupes_and_applies_safelist_size_limit() {
        let headers = vec![
            ("X-Test".to_owned(), "yes".to_owned()),
            ("x-test".to_owned(), "again".to_owned()),
            ("Range".to_owned(), "bytes=0-1".to_owned()),
            ("Accept-Language".to_owned(), "a".repeat(1025)),
            ("Content-Type".to_owned(), "text/plain".to_owned()),
        ];

        assert_eq!(
            cors_unsafe_request_header_names(&headers),
            vec!["accept-language".to_owned(), "x-test".to_owned()]
        );

        let large_safelisted = (0..9)
            .map(|_| ("Accept".to_owned(), "a".repeat(128)))
            .collect::<Vec<_>>();

        assert_eq!(
            cors_unsafe_request_header_names(&large_safelisted),
            vec!["accept".to_owned()]
        );
    }

    #[test]
    fn response_forbidden_headers_match_fetch_filtering() {
        assert!(is_forbidden_response_header_name("Set-Cookie"));
        assert!(is_forbidden_response_header_name("set-cookie2"));
        assert!(!is_forbidden_response_header_name("content-type"));
    }
}

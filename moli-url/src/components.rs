use std::borrow::Cow;
use url::Url;

/// Apply the Web API host setter, including its partial host/port updates.
pub fn set_host(url: &mut Url, value: &str) {
    let value = strip_tabs_and_newlines(value);
    let mut updated = url.clone();
    if url::quirks::set_host(&mut updated, &value).is_err() {
        return;
    }
    // rust-url's host setter checks usernames but misses password-only
    // credentials when rejecting an empty host. Keep this guard shared by URL
    // and HTML hyperlinks; an empty host cannot retain credentials or a port.
    if url::quirks::hostname(&updated).is_empty()
        && (!updated.username().is_empty()
            || updated
                .password()
                .is_some_and(|password| !password.is_empty())
            || updated.port().is_some())
    {
        return;
    }
    *url = updated;
}

/// Apply the Web API hostname setter without interpreting a port suffix.
pub fn set_hostname(url: &mut Url, value: &str) {
    // In particular, file URLs must treat a tab/newline-only input as an empty
    // host, including rust-url's early check before entering its host parser.
    let _ = url::quirks::set_hostname(url, &strip_tabs_and_newlines(value));
}

/// Apply the Web API port setter, distinguishing an empty value from empty input.
pub fn set_port(url: &mut Url, value: &str) {
    let input = strip_tabs_and_newlines(value);
    // Only the literal empty value clears the port. A nonempty value whose
    // parser input becomes empty leaves the existing port unchanged.
    if !value.is_empty() && input.is_empty() {
        return;
    }
    let _ = url::quirks::set_port(url, &input);
}

/// Apply the Web API pathname setter while preserving opaque paths.
pub fn set_pathname(url: &mut Url, value: &str) {
    if url.cannot_be_a_base() {
        return;
    }
    let value = strip_tabs_and_newlines(value);
    url::quirks::set_pathname(url, &value);
}

fn strip_tabs_and_newlines(value: &str) -> Cow<'_, str> {
    if value
        .bytes()
        .any(|byte| matches!(byte, b'\t' | b'\n' | b'\r'))
    {
        Cow::Owned(value.replace(['\t', '\n', '\r'], ""))
    } else {
        Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_host_setter_rejects_empty_hosts_with_credentials_or_ports() {
        for input in [
            "custom://user@example.test/path",
            "custom://:password@example.test/path",
            "custom://example.test:123/path",
        ] {
            for value in ["", "/discard", "?discard", "#discard", "\t\r\n"] {
                let mut url = Url::parse(input).unwrap();
                set_host(&mut url, value);
                assert_eq!(url.as_str(), input, "host = {value:?}");
            }
        }
    }

    #[test]
    fn web_host_setter_preserves_partial_updates_and_file_host_normalization() {
        for (input, value, expected) in [
            (
                "https://old.test:123/path",
                "new.test:65536",
                "https://new.test:123/path",
            ),
            (
                "https://old.test:123/path",
                "new.test:443tail",
                "https://new.test/path",
            ),
            ("custom://old.test/path", "", "custom:///path"),
            ("file://old.test/path", "loc%41lhost", "file:///path"),
            ("file://old.test/path", "\t\r\n", "file:///path"),
        ] {
            let mut url = Url::parse(input).unwrap();
            set_host(&mut url, value);
            assert_eq!(url.as_str(), expected);
        }
    }

    #[test]
    fn web_component_setters_distinguish_empty_inputs_and_filter_parser_controls() {
        let mut file = Url::parse("file://old.test/path").unwrap();
        set_hostname(&mut file, "\t\r\n");
        assert_eq!(file.as_str(), "file:///path");

        let mut url = Url::parse("https://example.test:123/path").unwrap();
        set_port(&mut url, "\t\r\n");
        assert_eq!(url.port(), Some(123));
        set_port(&mut url, "\t4\n5\r6tail");
        assert_eq!(url.port(), Some(456));
        set_port(&mut url, "");
        assert_eq!(url.port(), None);

        for (input, value, expected) in [
            ("custom:///path", "", "custom://"),
            ("custom:///path", "\t\r\n", "custom://"),
            ("custom:/path", "", "custom:/"),
            (
                "https://example.test/path",
                "\t/next",
                "https://example.test/next",
            ),
            (
                "https://example.test/path",
                "\n\\next",
                "https://example.test/next",
            ),
            ("data:payload", "replacement", "data:payload"),
        ] {
            let mut url = Url::parse(input).unwrap();
            set_pathname(&mut url, value);
            assert_eq!(url.as_str(), expected);
        }
    }
}

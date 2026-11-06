use crate::components::strip_tabs_and_newlines;
use url::{Position, Url};

/// Parse external URL input with Chromium's leading file separator handling.
/// Component setters and stored URL records can still contain empty path segments.
pub fn parse_url(input: &str, base: Option<&Url>) -> Result<Url, url::ParseError> {
    let cleaned = strip_tabs_and_newlines(input);
    let trimmed = cleaned.trim_matches(|character| character <= '\u{20}');
    let path = if trimmed
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
    {
        Some(&trimmed[5..])
    } else if base.is_some_and(|base| base.scheme() == "file") {
        Some(trimmed)
    } else {
        None
    };
    if let Some(path) = path {
        let rest = path.trim_start_matches(['/', '\\']);
        if path.len() - rest.len() > 3 {
            // Normalize an ordinary special path first. Exposing C: or C| to
            // the file parser would create a drive root and change .. handling.
            let normalized = Url::parse(&format!("https://file.invalid/{rest}"))?;
            return Url::parse(&format!(
                "file://{}{}",
                normalized.path().replace('|', "%7C"),
                &normalized[Position::AfterPath..],
            ));
        }
    }
    Url::options().base_url(base).parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_input_collapses_only_the_initial_separator_run() {
        for (input, expected) in [
            ("file:///a", "file:///a"),
            ("file:////a", "file:///a"),
            ("file://///a", "file:///a"),
            (r"file:\\\\a", "file:///a"),
            (" FILE://///a?x=1#f ", "file:///a?x=1#f"),
            ("fi\nle:\t/\r///a", "file:///a"),
            ("file://///", "file:///"),
            ("file:////a//b", "file:///a//b"),
            ("file:////%2F/a", "file:///%2F/a"),
            ("file:////x/..//a", "file:////a"),
            ("file:///.//a", "file:////a"),
            ("file://server///a", "file://server///a"),
            ("https://example.test///a", "https://example.test///a"),
        ] {
            assert_eq!(
                parse_url(input, None).unwrap().as_str(),
                expected,
                "{input:?}"
            );
        }
    }

    #[test]
    fn relative_file_input_uses_the_base_scheme() {
        let file = Url::parse("file://server/base").unwrap();
        let https = Url::parse("https://example.test/base").unwrap();
        for (input, base, expected) in [
            ("/////a", &file, "file:///a"),
            ("/////a:b", &file, "file:///a:b"),
            ("//server///a", &file, "file://server///a"),
            ("/a//b", &file, "file://server/a//b"),
            ("/a//b", &https, "https://example.test/a//b"),
            ("file:////a", &https, "file:///a"),
        ] {
            assert_eq!(
                parse_url(input, Some(base)).unwrap().as_str(),
                expected,
                "{input:?}"
            );
        }
    }

    #[test]
    fn extra_file_separators_do_not_create_drive_roots() {
        for (input, expected) in [
            ("file:////C|/file", "file:///C%7C/file"),
            ("file://///C|/file", "file:///C%7C/file"),
            ("file:////C:/..", "file:///"),
            ("file://///C:/../file", "file:///file"),
            ("file:////./C|/file", "file:///C%7C/file"),
            ("file:////x/../C:/..", "file:///"),
            ("file:////..//a", "file:////a"),
            ("file:////C%7C/file", "file:///C%7C/file"),
            ("file:////C|/a%2Fb/%25", "file:///C%7C/a%2Fb/%25"),
            (
                "file:////C|/file?keep=|#raw|",
                "file:///C%7C/file?keep=|#raw|",
            ),
            (
                r"file:////C|/a\..\b?back=\#frag\",
                r"file:///C%7C/b?back=\#frag\",
            ),
        ] {
            assert_eq!(parse_url(input, None).unwrap().as_str(), expected);
        }

        let base = Url::parse("file://server/base").unwrap();
        for (input, expected) in [
            ("/////C|/file", "file:///C%7C/file"),
            ("/////C:/..", "file:///"),
        ] {
            assert_eq!(parse_url(input, Some(&base)).unwrap().as_str(), expected);
        }
    }
}

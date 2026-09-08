use std::path::Path;

use http::HeaderName;
use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};
use sanitize_filename::Options;
use url::Url;

use super::DownloadBehavior;

pub(super) fn generate_download_guid() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .map_err(|error| format!("failed to generate download GUID: {error}"))?;
    Ok(format_download_guid(bytes))
}

fn format_download_guid(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

pub(super) fn artifact_file_name(
    behavior: DownloadBehavior,
    guid: &str,
    suggested_filename: &str,
) -> String {
    if behavior.names_artifact_by_guid() {
        return guid.to_owned();
    }
    sanitize_filename(suggested_filename)
}

pub(super) fn content_length_from_headers(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(name, _)| header_name_is(name, &HeaderName::from_static("content-length")))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
}

pub(super) fn filename_from_headers(headers: &[(String, String)]) -> Option<String> {
    for (name, value) in headers {
        if !header_name_is(name, &HeaderName::from_static("content-disposition")) {
            continue;
        }
        if let Some(filename) = filename_from_content_disposition(value) {
            return Some(filename);
        }
    }
    None
}

fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    let mut saw_extended = false;

    // A `;` inside a quoted string does not start a new parameter. Splitting on
    // every `;` let text inside a quoted `filename` be read as a parameter of
    // its own, so a site that echoes an attacker-supplied name into the header
    // could smuggle a `filename*` and choose the extension the file is saved
    // under.
    for part in split_outside_quoted_strings(value, ';').into_iter().skip(1) {
        let part = part.trim();
        if let Some(raw) = strip_parameter_name(part, "filename*") {
            saw_extended = true;
            extended = decode_extended_filename(raw);
        } else if let Some(raw) = strip_parameter_name(part, "filename") {
            plain = Some(unquote_parameter_value(raw.trim()).into_owned());
        }
    }

    if extended.is_some() {
        return extended;
    }
    if saw_extended && plain.is_none() {
        return None;
    }

    plain
        .as_deref()
        .and_then(non_empty_filename)
        .map(sanitize_filename)
}

/// Strips a case-insensitive `name=` prefix from one parameter.
fn strip_parameter_name<'a>(part: &'a str, name: &str) -> Option<&'a str> {
    let rest = part
        .get(..name.len())?
        .eq_ignore_ascii_case(name)
        .then(|| &part[name.len()..])?;
    rest.trim_start().strip_prefix('=')
}

fn decode_extended_filename(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_matches('"');
    let mut parts = raw.splitn(3, '\'');
    let charset = parts.next().unwrap_or_default();
    let _language = parts.next();
    let encoded = parts.next().unwrap_or(raw);
    let decoded = percent_decode_bytes(encoded)?;

    let filename = if charset.is_empty() || charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(decoded).ok()?
    } else if charset.eq_ignore_ascii_case("iso-8859-1")
        || charset.eq_ignore_ascii_case("latin1")
        || charset.eq_ignore_ascii_case("latin-1")
    {
        decoded.into_iter().map(char::from).collect()
    } else {
        return None;
    };

    non_empty_filename(&filename).map(sanitize_filename)
}

fn percent_decode_bytes(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return None;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Some(percent_encoding::percent_decode(bytes).collect())
}

pub(super) fn filename_from_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(non_empty_filename)
        .map(sanitize_filename)
}

pub(super) fn non_empty_filename(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn sanitize_filename(value: &str) -> String {
    let component = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(non_empty_filename)
        .unwrap_or("download");
    let sanitized = sanitize_filename::sanitize_with_options(
        component,
        Options {
            windows: true,
            truncate: true,
            replacement: "",
        },
    );
    non_empty_filename(&sanitized)
        .unwrap_or("download")
        .to_owned()
}

fn header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_behavior_helpers_preserve_allow_and_naming_policy() {
        assert!(!DownloadBehavior::Default.allows_download());
        assert!(!DownloadBehavior::Deny.allows_download());
        assert!(DownloadBehavior::Allow.allows_download());
        assert!(DownloadBehavior::AllowAndName.allows_download());

        assert!(!DownloadBehavior::Allow.names_artifact_by_guid());
        assert!(DownloadBehavior::AllowAndName.names_artifact_by_guid());
    }

    #[test]
    fn download_guid_uses_random_uuid_v4_shape() {
        let guid = generate_download_guid().expect("secure random download GUID");
        assert_eq!(guid.len(), 36);
        assert_eq!(
            guid.chars()
                .enumerate()
                .filter_map(|(index, character)| (character == '-').then_some(index))
                .collect::<Vec<_>>(),
            [8, 13, 18, 23]
        );
        assert_eq!(guid.as_bytes()[14], b'4');
        assert!(matches!(guid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert!(
            guid.chars()
                .filter(|character| *character != '-')
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
    }

    #[test]
    fn download_guid_formatter_sets_uuid_version_and_variant_bits() {
        assert_eq!(
            format_download_guid([0; 16]),
            "00000000-0000-4000-8000-000000000000"
        );
        assert_eq!(
            format_download_guid([u8::MAX; 16]),
            "ffffffff-ffff-4fff-bfff-ffffffffffff"
        );
    }

    #[test]
    fn artifact_file_name_uses_guid_only_for_allow_and_name_behavior() {
        assert_eq!(
            artifact_file_name(DownloadBehavior::AllowAndName, "GUID-1", "../report.txt"),
            "GUID-1"
        );
        assert_eq!(
            artifact_file_name(DownloadBehavior::Allow, "GUID-1", "../report.txt"),
            "report.txt"
        );
        assert_eq!(
            artifact_file_name(DownloadBehavior::Default, "GUID-1", "../report.txt"),
            "report.txt"
        );
    }

    #[test]
    fn content_length_from_headers_parses_case_insensitive_header_name() {
        assert_eq!(
            content_length_from_headers(&[("Content-Length".to_owned(), "42".to_owned())]),
            Some(42)
        );
        assert_eq!(
            content_length_from_headers(&[("content-length".to_owned(), "bad".to_owned())]),
            None
        );
    }

    #[test]
    fn content_disposition_ignores_filename_star_inside_a_quoted_filename() {
        // RFC 6266: the whole quoted string is the `filename` value, and there
        // is no `filename*` parameter here at all. Reading the inner text as
        // one let a site that echoes an attacker-supplied name into the header
        // choose the extension the file is saved under.
        let filename = filename_from_content_disposition(
            "attachment; filename=\"a;filename*=UTF-8''evil.exe\"",
        );

        // The saved name is the quoted string itself, with `*` removed by
        // the Windows-safe sanitizer rather than by the parameter scan.
        assert_ne!(filename.as_deref(), Some("evil.exe"));
        assert_eq!(filename.as_deref(), Some("a;filename=UTF-8''evil.exe"));
    }

    #[test]
    fn content_disposition_still_reads_a_real_filename_star_after_a_quoted_filename() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"plain;name.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt",
        );

        assert_eq!(filename.as_deref(), Some("中文.txt"));
    }

    #[test]
    fn content_disposition_prefers_filename_star_when_present() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt",
        );

        assert_eq!(filename.as_deref(), Some("中文.txt"));
    }

    #[test]
    fn content_disposition_falls_back_to_plain_filename_when_extended_decode_fails() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%ZZbroken",
        );

        assert_eq!(filename.as_deref(), Some("fallback.txt"));
    }

    #[test]
    fn content_disposition_rejects_invalid_extended_filename_without_plain_fallback() {
        let filename = filename_from_content_disposition("attachment; filename*=UTF-8''%ZZbroken");

        assert_eq!(filename, None);
    }

    #[test]
    fn sanitize_filename_strips_path_components() {
        assert_eq!(sanitize_filename("../nested/report.txt"), "report.txt");
    }

    #[test]
    fn sanitize_filename_removes_reserved_filename_characters() {
        assert_eq!(sanitize_filename("report?.txt"), "report.txt");
        assert_eq!(sanitize_filename("CON"), "download");
    }
}

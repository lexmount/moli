use super::*;
use moli_web_mime::data_url_body_and_mime_type;

#[derive(Debug)]
pub(crate) enum LocalUrlResponseError {
    BlobUnavailable(url::Url),
    InvalidRequest(String),
}

impl LocalUrlResponseError {
    pub(crate) fn is_unavailable_blob(&self) -> bool {
        matches!(self, Self::BlobUnavailable(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::BlobUnavailable(url) => format!("blob URL `{url}` is unavailable"),
            Self::InvalidRequest(message) => message,
        }
    }
}

/// Fetch's single range parser with HTTP whitespace enabled. Offsets saturate:
/// oversized starts fail the bounds check, while ends and suffix lengths clamp
/// to the available bytes without overflowing or limiting decimal digit counts.
fn blob_byte_range(value: &str, length: usize) -> Option<std::ops::Range<usize>> {
    fn offset(value: &str) -> Option<usize> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(value.bytes().fold(0usize, |number, byte| {
            number
                .saturating_mul(10)
                .saturating_add(usize::from(byte - b'0'))
        }))
    }

    let value = value
        .strip_prefix("bytes")?
        .trim_start_matches(['\t', ' '])
        .strip_prefix('=')?
        .trim_start_matches(['\t', ' ']);
    let (start, end) = value.split_once('-')?;
    let start = start.trim_end_matches(['\t', ' ']);
    let end = end.trim_start_matches(['\t', ' ']);
    if start.is_empty() {
        // A suffix longer than the Blob selects the whole representation.
        // A zero suffix produces an empty slice, as in Fetch's scheme fetch.
        return Some(length.saturating_sub(offset(end)?)..length);
    }
    let start = offset(start)?;
    if start >= length {
        return None;
    }
    let end = if end.is_empty() {
        length
    } else {
        offset(end)?.saturating_add(1).min(length)
    };
    (start < end).then_some(start..end)
}

pub(super) fn blob_response(
    url: &url::Url,
    body_bytes: &[u8],
    mime_type: &str,
    request_headers: &[(String, String)],
) -> Result<Response, LocalUrlResponseError> {
    let mut range_headers = request_headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("range"));
    let range = if let Some((_, value)) = range_headers.next() {
        // Getting a repeated header joins its values with a comma, which
        // cannot be parsed as the single range supported by Blob scheme fetch.
        let range = blob_byte_range(value, body_bytes.len());
        if range_headers.next().is_some() || range.is_none() {
            return Err(LocalUrlResponseError::InvalidRequest(
                "blob URL fetch has an invalid or unsatisfiable Range header".to_owned(),
            ));
        }
        range
    } else {
        None
    };
    let (status, status_text, content_range) = if let Some(range) = &range {
        let last = range
            .end
            .checked_sub(1)
            .map_or_else(|| "-1".to_owned(), |end| end.to_string());
        (
            206,
            "Partial Content",
            Some(format!("bytes {}-{last}/{}", range.start, body_bytes.len())),
        )
    } else {
        (200, "OK", None)
    };
    let body_bytes = &body_bytes[range.unwrap_or(0..body_bytes.len())];
    let mut headers = vec![
        ("Content-Length".to_owned(), body_bytes.len().to_string()),
        ("Content-Type".to_owned(), mime_type.to_owned()),
    ];
    if let Some(content_range) = content_range {
        headers.push(("Content-Range".to_owned(), content_range));
    }
    let headers = moli_fetch::headers_from_byte_strings(&headers)
        .expect("Blob response headers contain ByteStrings");
    Ok(Response::from_head_and_lossy_body_bytes(
        moli_fetch::ResponseHead {
            status_text: Some(status_text.to_owned()),
            final_url: url.clone(),
            status,
            headers,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_bytes.to_vec(),
    ))
}

pub(crate) fn data_url_response(url: &url::Url) -> Option<Response> {
    let (body_bytes, mime_type) = data_url_body_and_mime_type(url.as_str())?;
    Some(Response::from_head_and_lossy_body_bytes(
        moli_fetch::ResponseHead {
            status_text: None,
            final_url: url.clone(),
            status: 200,
            headers: vec![(
                "Content-Type".to_owned(),
                moli_fetch::header_value_from_byte_string(&mime_type)
                    .expect("serialized MIME types contain ByteStrings"),
            )],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_bytes,
    ))
}

/// Reads a local resource for consumers whose requests always use GET.
pub(crate) fn local_url_response(url: &url::Url) -> Option<Response> {
    local_url_response_result(url, "GET", &[]).and_then(Result::ok)
}

/// Resolves renderer-owned URL schemes without falling through to the network
/// transport when the local resource is malformed, revoked, or unavailable.
///
/// `None` means that the URL is not owned by this resolver. `Some(Err(..))`
/// means that it is a local URL and therefore must fail locally instead of
/// being handed to libcurl.
/// `method` is the request's already normalized method.
pub(crate) fn local_url_response_result(
    url: &url::Url,
    method: &str,
    request_headers: &[(String, String)],
) -> Option<Result<Response, String>> {
    local_url_response_with_blob_entry(url, method, request_headers, None)
        .map(|result| result.map_err(LocalUrlResponseError::into_message))
}

pub(crate) fn local_url_response_with_blob_entry(
    url: &url::Url,
    method: &str,
    request_headers: &[(String, String)],
    entry: Option<&CapturedBlobUrl>,
) -> Option<Result<Response, LocalUrlResponseError>> {
    let result = match url.scheme() {
        "blob" if method != "GET" => Some(Err(LocalUrlResponseError::InvalidRequest(format!(
            "blob URL fetch requires GET, got `{method}`"
        )))),
        "blob" => {
            let response = match entry.filter(|entry| entry.matches(url)) {
                Some(entry) => entry.response(url, request_headers),
                None => CapturedBlobUrl::capture(url)
                    .and_then(|entry| entry.response(url, request_headers)),
            };
            Some(
                response
                    .unwrap_or_else(|| Err(LocalUrlResponseError::BlobUnavailable(url.clone()))),
            )
        }
        "data" => Some(data_url_response(url).ok_or_else(|| {
            LocalUrlResponseError::InvalidRequest(format!("data URL `{url}` is invalid"))
        })),
        _ => None,
    }?;
    Some(result.map(|response| {
        if !response_has_null_body(method, response.head().status) {
            return response;
        }
        // Apply the Fetch body filter before recording or delivering a local
        // response, while preserving its status, MIME type and other headers.
        let (head, _) = response.into_body();
        Response::from_head_and_lossy_body_bytes(head, Vec::new())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_byte_ranges_validate_ascii_syntax_and_bound_unlimited_offsets() {
        for (value, expected) in [
            ("bytes=2-5", Some(2..6)),
            ("bytes=4-", Some(4..10)),
            ("bytes=-3", Some(7..10)),
            ("bytes=-12", Some(0..10)),
            ("bytes=-0", Some(10..10)),
            ("bytes=4-10", Some(4..10)),
            ("bytes \t= \t1 \t- \t3", Some(1..4)),
            ("bytes=0001-0003", Some(1..4)),
            ("bytes=10-", None),
            ("bytes=4-2", None),
            ("bytes=-", None),
            ("bytes=+1-3", None),
            ("bytes=1-+3", None),
            ("bytes=1 2-3", None),
            ("bytes=1-3,", None),
            ("bytes=1-3,5-8", None),
            ("bytes=1-3 ", None),
            (" bytes=1-3", None),
            ("BYTES=1-3", None),
            ("bytes=\u{a0}1-3", None),
            ("bytes=\u{c}1-3", None),
            ("bytes=１-３", None),
            ("", None),
        ] {
            assert_eq!(blob_byte_range(value, 10), expected, "{value:?}");
        }
        let huge = "9".repeat(80);
        assert_eq!(blob_byte_range(&format!("bytes=1-{huge}"), 10), Some(1..10));
        assert_eq!(blob_byte_range(&format!("bytes=-{huge}"), 10), Some(0..10));
        assert_eq!(blob_byte_range(&format!("bytes={huge}-"), 10), None);
        assert_eq!(
            blob_byte_range(&format!("bytes=0{}1-3", "0".repeat(80)), 10),
            Some(1..4)
        );
        assert_eq!(blob_byte_range("bytes=0-", 0), None);
        assert_eq!(blob_byte_range("bytes=-1", 0), Some(0..0));
    }

    #[test]
    fn blob_range_response_slices_bytes_and_distinguishes_missing_entries() {
        let url = url::Url::parse("blob:https://example.test/range").unwrap();
        let bytes = [0, 255, 128, 65];
        let response = blob_response(
            &url,
            &bytes,
            "",
            &[("rAnGe".to_owned(), "bytes=1-2".to_owned())],
        )
        .unwrap();
        let head = response.head();
        assert_eq!(head.status, 206);
        assert_eq!(head.status_text.as_deref(), Some("Partial Content"));
        assert_eq!(
            head.headers,
            vec![
                ("Content-Length".to_owned(), b"2".to_vec()),
                ("Content-Type".to_owned(), Vec::new()),
                ("Content-Range".to_owned(), b"bytes 1-2/4".to_vec()),
            ]
        );
        assert_eq!(
            response
                .into_body()
                .1
                .try_into_materialized_bytes()
                .unwrap(),
            [255, 128]
        );
        for headers in [
            vec![("Range".to_owned(), "".to_owned())],
            vec![
                ("Range".to_owned(), "bytes=0-1".to_owned()),
                ("range".to_owned(), "bytes=2-3".to_owned()),
            ],
        ] {
            let error = blob_response(&url, &bytes, "", &headers).unwrap_err();
            assert!(!error.is_unavailable_blob());
            assert!(error.into_message().contains("Range"));
            let missing = local_url_response_with_blob_entry(&url, "GET", &headers, None)
                .unwrap()
                .unwrap_err();
            assert!(missing.is_unavailable_blob());
        }
        let data = url::Url::parse("data:,0123").unwrap();
        let response =
            local_url_response_result(&data, "GET", &[("Range".to_owned(), "invalid".to_owned())])
                .unwrap()
                .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response
                .into_body()
                .1
                .try_into_materialized_bytes()
                .unwrap(),
            b"0123"
        );
    }

    #[test]
    fn data_url_response_decodes_plain_and_base64_payloads() {
        let plain = url::Url::parse("data:,BB-8").unwrap();
        let (_, body) = data_url_response(&plain).unwrap().into_body();
        let (text, _) = body.try_into_lossy_materialized_text().unwrap();
        assert_eq!(text, "BB-8");

        let base64 = url::Url::parse("data:text/plain;base64,Sy0yU08=").unwrap();
        let (_, body) = data_url_response(&base64).unwrap().into_body();
        let (text, _) = body.try_into_lossy_materialized_text().unwrap();
        assert_eq!(text, "K-2SO");
    }

    #[test]
    fn data_url_response_preserves_supplied_charset_parameter() {
        let url = url::Url::parse("data:text/html;charset=iso-2022-jp,hello").unwrap();
        let response = data_url_response(&url).unwrap();

        assert_eq!(
            response
                .head()
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_slice()),
            Some(b"text/html;charset=iso-2022-jp".as_slice())
        );
    }

    #[test]
    fn unavailable_blob_url_is_a_local_failure() {
        let url = url::Url::parse("blob:https://example.test/not-registered").unwrap();

        let error = local_url_response_result(&url, "GET", &[])
            .expect("blob URL must be owned by the local resolver")
            .expect_err("an unregistered blob URL must fail locally");

        assert_eq!(
            error,
            "blob URL `blob:https://example.test/not-registered` is unavailable"
        );
    }
}

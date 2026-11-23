//! Shared download metadata and streaming body transfer.

use std::path::Path;

use anyhow::{Context, Result};
use http::HeaderName;
use moli_fetch::{FetchCancelHandle, Request, StreamingRawResponse};
use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};
use moli_renderer_v8::RendererDownloadBody;
use sanitize_filename::Options;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use url::Url;

pub fn download_request(url: &str, initiator: Option<&Url>) -> Result<Request> {
    let request = Request::get(url)
        .context("invalid download URL")?
        .with_top_level_navigation_cookie_context()
        .with_page_network_policy()
        .with_request_origin(
            initiator.map_or(moli_url::WebOrigin::Opaque, moli_url::WebOrigin::from_url),
        );
    Ok(match initiator {
        Some(url) => request.with_initiator_url(url),
        None => request,
    })
}

#[derive(Debug)]
pub enum DownloadBody {
    Buffered(Vec<u8>),
    Blob(moli_file_api::BlobData),
    Streaming(Box<StreamingRawResponse>),
    Intercepted(crate::network::ResourceInterceptionBody),
}

impl From<RendererDownloadBody> for DownloadBody {
    fn from(body: RendererDownloadBody) -> Self {
        match body {
            RendererDownloadBody::Buffered(bytes) => Self::Buffered(bytes),
            RendererDownloadBody::Blob(blob) => Self::Blob(blob),
            RendererDownloadBody::Network(body) => Self::Intercepted(body),
        }
    }
}

impl DownloadBody {
    pub fn cancellation_handle(&self) -> FetchCancelHandle {
        match self {
            Self::Buffered(_) | Self::Blob(_) => FetchCancelHandle::new(),
            Self::Streaming(response) => response.cancellation_handle(),
            Self::Intercepted(body) => body.cancellation_handle(),
        }
    }

    pub fn known_size(&self) -> Option<u64> {
        match self {
            Self::Buffered(bytes) => Some(bytes.len() as u64),
            Self::Blob(data) => Some(data.len() as u64),
            Self::Streaming(_) | Self::Intercepted(_) => None,
        }
    }

    /// Transfers the body and flushes the writer before reporting success.
    /// Take the live stream directly unless an interception client already read it.
    pub async fn write_to(
        mut self,
        writer: &mut (impl AsyncWrite + Unpin),
        mut progress: impl FnMut(u64),
    ) -> Result<u64> {
        if let Self::Intercepted(body) = &self
            && let Some(response) = body.take_unread_response().await
        {
            self = Self::Streaming(Box::new(response));
        }
        let mut received = 0_u64;
        loop {
            let chunk = match &mut self {
                Self::Buffered(bytes) => {
                    if bytes.is_empty() {
                        break;
                    }
                    std::mem::take(bytes)
                }
                Self::Blob(data) => {
                    let start = usize::try_from(received)?;
                    if start == data.len() {
                        break;
                    }
                    let end = start.saturating_add(64 * 1024).min(data.len());
                    let (send, receive) = tokio::sync::oneshot::channel();
                    data.read(
                        start..end,
                        Box::new(move |result| {
                            let _ = send.send(result);
                        }),
                    );
                    receive.await.context("Blob download read canceled")??
                }
                Self::Streaming(response) => match response.next_chunk().await {
                    Some(chunk) => chunk,
                    None => {
                        response.finish().await?;
                        break;
                    }
                },
                Self::Intercepted(body) => {
                    let (chunk, eof) = body
                        .read_chunk(usize::try_from(received)?, 64 * 1024)
                        .await?;
                    if eof && chunk.is_empty() {
                        break;
                    }
                    chunk
                }
            };
            writer.write_all(&chunk).await?;
            received += chunk.len() as u64;
            progress(received);
        }
        writer.flush().await?;
        Ok(received)
    }
}

pub fn content_length_from_headers(headers: &[(String, Vec<u8>)]) -> Option<u64> {
    headers
        .iter()
        .find(|(name, _)| header_name_is(name, &HeaderName::from_static("content-length")))
        .and_then(|(_, value)| {
            moli_fetch::decode_header_value(value)
                .trim()
                .parse::<u64>()
                .ok()
        })
}

pub fn filename_from_headers(headers: &[(String, Vec<u8>)]) -> Option<String> {
    for (name, value) in headers {
        if !header_name_is(name, &HeaderName::from_static("content-disposition")) {
            continue;
        }
        if let Some(filename) =
            filename_from_content_disposition(&moli_fetch::decode_header_value(value))
        {
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

pub fn filename_from_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(non_empty_filename)
        .map(sanitize_filename)
}

pub fn non_empty_filename(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub fn sanitize_filename(value: &str) -> String {
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
    fn content_length_from_headers_parses_case_insensitive_header_name() {
        assert_eq!(
            content_length_from_headers(&[("Content-Length".to_owned(), b"42".to_vec())]),
            Some(42)
        );
        assert_eq!(
            content_length_from_headers(&[("content-length".to_owned(), b"bad".to_vec())]),
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

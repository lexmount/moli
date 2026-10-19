use crate::{
    determine_nosniff, extract_response_mime_essence,
    response_headers_indicate_attachment_download, sniff_archive_mime_type,
    sniff_audio_video_mime_type, sniff_image_mime_type, sniff_text_or_binary,
};

pub const NAVIGATION_MIME_PREFIX_LIMIT: usize = 1024;
const HTML_PREFIX_LIMIT: usize = 512;
const MAGIC_PREFIX_LIMIT: usize = 42;

/// Resolves a navigation MIME type before a new Document is committed.
///
/// A positive signature can settle the decision before EOF. A provisional
/// text/binary default must not choose a parser while more prefix bytes can
/// still change it. The retained prefix is bounded independently of chunk size.
#[derive(Debug)]
pub struct NavigationMimeSniffer {
    mime_type: Option<String>,
    sniff_scriptable: bool,
    prefix: Vec<u8>,
}

impl NavigationMimeSniffer {
    pub fn new(scheme: &str, headers: &[(String, Vec<u8>)]) -> Self {
        let supplied = extract_response_mime_essence(headers);
        let unknown = supplied
            .as_deref()
            .is_none_or(|mime| matches!(mime, "unknown/unknown" | "application/unknown" | "*/*"));
        let no_sniff = determine_nosniff(headers);
        let needs_prefix = matches!(scheme, "http" | "https" | "file" | "filesystem")
            && !no_sniff
            && !response_headers_indicate_attachment_download(headers)
            && (unknown || supplied.as_deref() == Some("text/plain"));
        Self {
            mime_type: if needs_prefix {
                None
            } else {
                Some(supplied.unwrap_or_else(|| {
                    if no_sniff || scheme == "blob" {
                        "text/plain".to_owned()
                    } else {
                        "text/html".to_owned()
                    }
                }))
            },
            sniff_scriptable: unknown,
            prefix: Vec::new(),
        }
    }

    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.mime_type.is_some() {
            return;
        }
        let count = (NAVIGATION_MIME_PREFIX_LIMIT - self.prefix.len()).min(bytes.len());
        self.prefix.extend_from_slice(&bytes[..count]);
        let header = self.prefix.as_slice();
        if self.sniff_scriptable
            && let Some(mime) =
                sniff_navigation_scriptable(&header[..header.len().min(HTML_PREFIX_LIMIT)])
        {
            self.mime_type = Some(mime.to_owned());
            return;
        }

        let binary = sniff_text_or_binary(header) == "application/octet-stream";
        // A supplied plain-text type cannot be promoted to scriptable content.
        // Its binary check must also precede image signatures such as GIF/BMP,
        // whose shortest identifying prefix contains only printable bytes.
        if self.sniff_scriptable || binary {
            if let Some(mime) = sniff_image_mime_type(header)
                .or_else(|| sniff_audio_video_mime_type(header))
                .or_else(|| sniff_archive_mime_type(header))
            {
                self.mime_type = Some(mime.to_owned());
                return;
            }
            if header.starts_with(b"%PDF-") {
                self.mime_type = Some("application/pdf".to_owned());
                return;
            }
            if header.starts_with(b"%!PS-Adobe-") {
                self.mime_type = Some("application/postscript".to_owned());
                return;
            }
            if header.starts_with(b"MZ") {
                self.mime_type = Some("application/octet-stream".to_owned());
                return;
            }
        }
        let binary_limit = if self.sniff_scriptable {
            HTML_PREFIX_LIMIT
        } else {
            MAGIC_PREFIX_LIMIT
        };
        if header.len() == NAVIGATION_MIME_PREFIX_LIMIT || (binary && header.len() >= binary_limit)
        {
            self.mime_type = Some(sniff_text_or_binary(header).to_owned());
        }
    }

    pub fn finish(&mut self) -> &str {
        self.mime_type
            .get_or_insert_with(|| sniff_text_or_binary(&self.prefix).to_owned())
    }
}

fn sniff_navigation_scriptable(header: &[u8]) -> Option<&'static str> {
    let header = header.trim_ascii_start();
    // Navigation sniffing accepts complete signature prefixes. This includes
    // an HTML comment with no following space, unlike the general MIME parser.
    const HTML_SIGNATURES: &[&[u8]] = &[
        b"<!DOCTYPE HTML",
        b"<HTML",
        b"<HEAD",
        b"<SCRIPT",
        b"<IFRAME",
        b"<H1",
        b"<DIV",
        b"<FONT",
        b"<TABLE",
        b"<A",
        b"<STYLE",
        b"<TITLE",
        b"<B",
        b"<BODY",
        b"<BR",
        b"<P",
        b"<!--",
    ];
    if HTML_SIGNATURES.iter().any(|signature| {
        header
            .get(..signature.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(signature))
    }) {
        Some("text/html")
    } else if header.starts_with(b"<?xml") {
        Some("text/xml")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(mime: &str) -> Vec<(String, Vec<u8>)> {
        vec![("Content-Type".to_owned(), mime.as_bytes().to_vec())]
    }

    #[test]
    fn navigation_signatures_survive_every_chunk_boundary() {
        for (bytes, mime) in [
            (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
            (b"GIF89a", "image/gif"),
            (b"\xff\xd8\xff", "image/jpeg"),
            (b"BM", "image/bmp"),
            (b"RIFF1234WEBPVP", "image/webp"),
            (b"  <!DOCTYPE html", "text/html"),
            (b"<!--without a space", "text/html"),
            (b"<?xml", "text/xml"),
            (b"MZ", "application/octet-stream"),
        ] {
            for supplied in [
                None,
                Some("not a MIME"),
                Some("unknown/unknown"),
                Some("application/unknown"),
                Some("*/*"),
            ] {
                let headers = supplied.map(headers).unwrap_or_default();
                for split in 0..=bytes.len() {
                    let mut sniffer = NavigationMimeSniffer::new("https", &headers);
                    assert_eq!(sniffer.mime_type(), None);
                    sniffer.push(&bytes[..split]);
                    sniffer.push(&bytes[split..]);
                    assert_eq!(
                        sniffer.mime_type(),
                        Some(mime),
                        "{supplied:?}, split {split}"
                    );
                    sniffer.push(b"<script>later bytes cannot replace the decision</script>");
                    assert_eq!(sniffer.finish(), mime);
                }
            }
        }
    }

    #[test]
    fn incomplete_signatures_do_not_commit_a_provisional_default() {
        let mut sniffer = NavigationMimeSniffer::new("https", &[]);
        for byte in b"\x89PNG\r\n\x1a" {
            sniffer.push(&[*byte]);
            assert_eq!(sniffer.mime_type(), None);
        }
        sniffer.push(b"\n");
        assert_eq!(sniffer.mime_type(), Some("image/png"));
        let mut truncated = NavigationMimeSniffer::new("https", &[]);
        truncated.push(b"\x89PNG\r\n\x1a");
        assert_eq!(truncated.finish(), "application/octet-stream");
    }

    #[test]
    fn plain_text_can_become_an_image_but_never_scriptable_html() {
        for mime in [
            "text/plain",
            "text/plain; charset=gbk",
            "TEXT/PLAIN;charset=UTF-8",
        ] {
            let mut image = NavigationMimeSniffer::new("http", &headers(mime));
            assert_eq!(image.mime_type(), None);
            image.push(b"\x89PNG\r\n\x1a\n");
            assert_eq!(image.mime_type(), Some("image/png"));
            let mut text = NavigationMimeSniffer::new("http", &headers(mime));
            text.push(b"<!doctype html><script>mustStayInert()</script>");
            assert_eq!(text.mime_type(), None);
            assert_eq!(text.finish(), "text/plain");
            let mut binary = NavigationMimeSniffer::new("http", &headers(mime));
            binary.push(b"text\0<script>mustNotExecute()</script>");
            assert_eq!(binary.finish(), "application/octet-stream");
            let mut no_sniff = headers(mime);
            no_sniff.push(("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()));
            let mut literal = NavigationMimeSniffer::new("http", &no_sniff);
            literal.push(b"text\0<script>mustNotExecute()</script>");
            assert_eq!(literal.finish(), "text/plain");
        }
        let mut gif = NavigationMimeSniffer::new("http", &headers("text/plain"));
        gif.push(b"GIF89a");
        assert_eq!(gif.mime_type(), None);
        gif.push(b"\x01\x00");
        assert_eq!(gif.mime_type(), Some("image/gif"));
    }

    #[test]
    fn authoritative_headers_do_not_wait_for_response_bytes() {
        for supplied in [None, Some("not a MIME"), Some("text/plain")] {
            let headers = supplied.map(headers).unwrap_or_default();
            let mut sniffer = NavigationMimeSniffer::new("blob", &headers);
            assert_eq!(sniffer.mime_type(), Some("text/plain"));
            sniffer.push(b"\x89PNG\r\n\x1a\n<script>mustStayInert()</script>");
            assert_eq!(sniffer.finish(), "text/plain");
        }
        for mime in [
            "text/html",
            "application/xml",
            "image/svg+xml",
            "image/png",
            "application/octet-stream",
            "application/json",
        ] {
            let mut sniffer = NavigationMimeSniffer::new("https", &headers(mime));
            assert_eq!(sniffer.mime_type(), Some(mime));
            sniffer.push(b"GIF89a");
            assert_eq!(sniffer.finish(), mime);
        }
        for supplied in [
            None,
            Some("not a MIME"),
            Some("text/plain; charset=gbk"),
            Some("unknown/unknown"),
        ] {
            let mut headers = supplied.map(headers).unwrap_or_default();
            headers.push(("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()));
            let expected = if supplied == Some("unknown/unknown") {
                "unknown/unknown"
            } else {
                "text/plain"
            };
            let mut sniffer = NavigationMimeSniffer::new("https", &headers);
            assert_eq!(sniffer.mime_type(), Some(expected));
            sniffer.push(b"\x89PNG\r\n\x1a\n");
            assert_eq!(sniffer.finish(), expected);
        }
    }

    #[test]
    fn provisional_text_is_bounded_and_cannot_execute_later_markup() {
        let mut sniffer = NavigationMimeSniffer::new("https", &[]);
        sniffer.push(&vec![b'a'; NAVIGATION_MIME_PREFIX_LIMIT - 1]);
        assert_eq!(sniffer.mime_type(), None);
        sniffer.push(b"a<script>mustNotChangeTheParser()</script>");
        assert_eq!(sniffer.mime_type(), Some("text/plain"));
        assert_eq!(sniffer.prefix.len(), NAVIGATION_MIME_PREFIX_LIMIT);
        let mut short = NavigationMimeSniffer::new("https", &[]);
        short.push(b"short text");
        assert_eq!(short.mime_type(), None);
        assert_eq!(short.finish(), "text/plain");
        assert_eq!(
            NavigationMimeSniffer::new("https", &[]).finish(),
            "text/plain"
        );
    }
}

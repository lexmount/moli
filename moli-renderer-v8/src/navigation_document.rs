use std::{borrow::Cow, fmt, sync::Arc};

use url::Url;

/// Only this browser-owned shell is parsed for a raster image navigation.
/// The response bytes are retained separately and never become parser input.
pub(crate) const IMAGE_DOCUMENT_SHELL: &str =
    "<!DOCTYPE html><html><head></head><body><img></body></html>";

pub(crate) fn is_image_document_mime(mime: &str) -> bool {
    moli_web_mime::is_image_mime(mime) && !moli_web_mime::is_xml_document_mime(mime)
}

/// Uses the same MIME decision for complete child responses and main streams.
/// The URL scheme controls sniffing; a path extension does not select a parser.
pub(crate) fn navigation_response_content_type(
    final_url: &Url,
    headers: &[(String, Vec<u8>)],
    bytes: &[u8],
) -> String {
    let mut sniffer = moli_web_mime::NavigationMimeSniffer::new(final_url.scheme(), headers);
    sniffer.push(bytes);
    sniffer.finish().to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NavigationDocumentContent {
    Text(String),
    Image(NavigationImageDocument),
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NavigationImageDocument {
    headers: Arc<[(String, Vec<u8>)]>,
    body: Arc<[u8]>,
}

impl fmt::Debug for NavigationImageDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationImageDocument")
            .field("encoded_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl NavigationImageDocument {
    pub(crate) fn from_response(
        bytes: &[u8],
        headers: &[(String, Vec<u8>)],
        content_type: Option<&str>,
    ) -> Option<Self> {
        content_type
            .is_some_and(is_image_document_mime)
            .then(|| Self {
                headers: headers.into(),
                body: bytes.into(),
            })
    }

    pub(crate) fn headers(&self) -> &[(String, Vec<u8>)] {
        &self.headers
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.body
    }
}

impl NavigationDocumentContent {
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(source) => Some(source),
            Self::Image(_) => None,
        }
    }

    pub(crate) fn parser_source(&self) -> &str {
        match self {
            Self::Text(source) => source,
            Self::Image(_) => IMAGE_DOCUMENT_SHELL,
        }
    }

    pub(crate) fn image(&self) -> Option<&NavigationImageDocument> {
        match self {
            Self::Text(_) => None,
            Self::Image(image) => Some(image),
        }
    }

    /// Protocol and detached projections use ordinary serialized markup. Live
    /// navigation instead binds the exact native host to its accepted response.
    pub(crate) fn snapshot_markup(&self, document_url: &Url) -> Cow<'_, str> {
        match self {
            Self::Text(source) => Cow::Borrowed(source),
            Self::Image(_) => {
                let source = document_url
                    .as_str()
                    .replace('&', "&amp;")
                    .replace('"', "&quot;")
                    .replace('<', "&lt;");
                Cow::Owned(format!(
                    "<!DOCTYPE html><html><head></head><body><img src=\"{source}\"></body></html>"
                ))
            }
        }
    }
}

impl From<String> for NavigationDocumentContent {
    fn from(source: String) -> Self {
        Self::Text(source)
    }
}

pub(crate) fn decode_navigation_document_response(
    bytes: &[u8],
    headers: &[(String, Vec<u8>)],
    content_type: Option<&str>,
    final_url: &Url,
    inherited_encoding: Option<&str>,
) -> (NavigationDocumentContent, &'static str) {
    if let Some(image) = NavigationImageDocument::from_response(bytes, headers, content_type) {
        return (NavigationDocumentContent::Image(image), "UTF-8");
    }
    let (source, encoding) = crate::document_response_decoder::decode_document_response(
        bytes,
        headers,
        content_type,
        final_url,
        inherited_encoding,
    );
    (NavigationDocumentContent::Text(source), encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_unsupplied_http_mime_from_bytes() {
        let url = Url::parse("https://example.test/actually-html.png").unwrap();
        let html = b"<!DOCTYPE html><script>ordinaryHTML()</script>";
        let gif = b"GIF89a\x01\x00\x01\x00";
        let supplied = vec![("Content-Type".to_owned(), b"image/png".to_vec())];
        for (headers, bytes, expected, image) in [
            (&[][..], html.as_slice(), "text/html", false),
            (&[][..], gif.as_slice(), "image/gif", true),
            (supplied.as_slice(), html.as_slice(), "image/png", true),
        ] {
            let mime = navigation_response_content_type(&url, headers, bytes);
            assert_eq!(mime, expected);
            let (content, _) =
                decode_navigation_document_response(bytes, headers, Some(&mime), &url, None);
            assert_eq!(content.image().is_some(), image);
            if !image {
                assert_eq!(content.parser_source().as_bytes(), html);
            }
        }
    }

    #[test]
    fn unsupplied_navigation_mime_with_nosniff_never_becomes_an_image_document() {
        let headers = vec![("X-Content-Type-Options".to_owned(), b"nosniff".to_vec())];
        let bytes = b"GIF89a\x01\x00\x01\x00";
        let mime = navigation_response_content_type(
            &Url::parse("https://example.test/picture.gif").unwrap(),
            &headers,
            bytes,
        );
        assert_eq!(mime, "text/plain");
        let (content, _) = decode_navigation_document_response(
            bytes,
            &headers,
            Some(&mime),
            &Url::parse("https://example.test/picture.gif").unwrap(),
            None,
        );
        assert!(content.image().is_none());
        assert_eq!(content.parser_source().as_bytes(), bytes);
    }

    #[test]
    fn image_navigation_never_decodes_response_bytes_as_html() {
        let payload = b"<script>parent.executed = true</script>";
        let url = Url::parse("https://example.test/image?one=1&two=2").unwrap();
        for mime in [
            "image/png",
            "Image/GIF; charset=gbk",
            "image/jpeg",
            "image/bmp",
        ] {
            let headers = vec![("Content-Type".into(), mime.as_bytes().to_vec())];
            let (content, encoding) = decode_navigation_document_response(
                payload,
                &headers,
                Some(mime),
                &url,
                Some("GBK"),
            );
            assert_eq!(encoding, "UTF-8");
            assert_eq!(content.parser_source(), IMAGE_DOCUMENT_SHELL);
            assert_eq!(content.image().unwrap().bytes(), payload);
            assert_eq!(content.image().unwrap().headers(), headers);
            assert!(!content.snapshot_markup(&url).contains("<script>"));
            assert!(content.snapshot_markup(&url).contains("one=1&amp;two=2"));
        }
    }

    #[test]
    fn svg_and_custom_xml_image_types_keep_the_xml_source() {
        let source = "<svg xmlns='http://www.w3.org/2000/svg'><title>café</title></svg>";
        let url = Url::parse("https://example.test/image.svg").unwrap();
        for mime in ["image/svg+xml", "image/example+xml"] {
            let headers = vec![("Content-Type".into(), mime.as_bytes().to_vec())];
            let (content, encoding) = decode_navigation_document_response(
                source.as_bytes(),
                &headers,
                Some(mime),
                &url,
                Some("GBK"),
            );
            assert_eq!(encoding, "UTF-8");
            assert_eq!(content.parser_source(), source);
            assert!(content.image().is_none());
        }
    }
}

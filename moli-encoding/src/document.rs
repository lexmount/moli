use encoding_rs::{CoderResult, Decoder, Encoding};
use moli_charset_parser::{HtmlMetaCharsetParser, HtmlMetaCharsetScanResult};

use crate::{encoding_for_label, encoding_from_response_headers};

const DEFAULT_HTML_DOCUMENT_ENCODING: &str = "windows-1252";
const UTF16LE_XML_PREFIX: &[u8; 6] = b"<\0?\0x\0";
const UTF16BE_XML_PREFIX: &[u8; 6] = b"\0<\0?\0x";

/// Stateless hook for optional, heuristic detection of legacy document
/// encodings. Deterministic BOM, transport and in-document declarations are
/// resolved before this hook is called.
pub type LegacyEncodingDetector =
    fn(bytes: &[u8], url_hint: Option<&str>) -> Option<&'static Encoding>;

pub fn decode_html_document(bytes: &[u8], headers: &[(String, String)]) -> (String, &'static str) {
    decode_html_document_with_fallback(bytes, headers, None)
}

pub fn decode_html_document_with_fallback(
    bytes: &[u8],
    headers: &[(String, String)],
    fallback_encoding: Option<&str>,
) -> (String, &'static str) {
    let mut decoder = HtmlDocumentStreamingDecoder::new_with_fallback(headers, fallback_encoding);
    let mut output = String::new();
    for chunk in decoder.push(bytes) {
        output.push_str(&chunk);
    }
    if let Some(chunk) = decoder.finish() {
        output.push_str(&chunk);
    }
    let encoding = decoder
        .selected_encoding_name()
        .unwrap_or(DEFAULT_HTML_DOCUMENT_ENCODING);
    (output, encoding)
}

pub struct HtmlDocumentStreamingDecoder {
    transport_encoding: Option<&'static Encoding>,
    fallback_encoding: &'static Encoding,
    sniff_buffer: Vec<u8>,
    emitted_sniff_len: usize,
    meta_prescan_fed_len: usize,
    meta_charset_parser: HtmlMetaCharsetParser,
    legacy_encoding_detector: Option<LegacyEncodingDetector>,
    url_hint: Option<String>,
    decoder: Option<Decoder>,
    selected_encoding: Option<&'static Encoding>,
}

impl HtmlDocumentStreamingDecoder {
    pub fn new(headers: &[(String, String)]) -> Self {
        Self::new_with_options(headers, None, None, None)
    }

    pub fn new_with_legacy_encoding_detector(
        headers: &[(String, String)],
        url_hint: &str,
        detector: LegacyEncodingDetector,
    ) -> Self {
        Self::new_with_options(headers, None, Some(url_hint), Some(detector))
    }

    pub fn new_with_fallback(
        headers: &[(String, String)],
        fallback_encoding: Option<&str>,
    ) -> Self {
        Self::new_with_options(headers, fallback_encoding, None, None)
    }

    fn new_with_options(
        headers: &[(String, String)],
        fallback_encoding: Option<&str>,
        url_hint: Option<&str>,
        legacy_encoding_detector: Option<LegacyEncodingDetector>,
    ) -> Self {
        Self {
            transport_encoding: encoding_from_response_headers(headers),
            fallback_encoding: fallback_encoding
                .and_then(encoding_for_label)
                .unwrap_or(encoding_rs::WINDOWS_1252),
            sniff_buffer: Vec::new(),
            emitted_sniff_len: 0,
            meta_prescan_fed_len: 0,
            meta_charset_parser: HtmlMetaCharsetParser::new(),
            legacy_encoding_detector,
            url_hint: url_hint.map(str::to_owned),
            decoder: None,
            selected_encoding: None,
        }
    }

    pub fn selected_encoding_name(&self) -> Option<&'static str> {
        self.selected_encoding.map(Encoding::name)
    }

    pub fn document_encoding_name(&self) -> &'static str {
        self.selected_encoding_name()
            .unwrap_or(self.fallback_encoding.name())
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<String> {
        if data.is_empty() {
            return Vec::new();
        }
        if self.decoder.is_some() {
            let decoded = self.decode(data, false);
            return non_empty_chunk(decoded);
        }

        self.sniff_buffer.extend_from_slice(data);
        if let Some(encoding) = self.encoding_ready(false) {
            self.start_decoder(encoding, self.emitted_sniff_len > 0);
            let buffered = std::mem::take(&mut self.sniff_buffer);
            let decode_start = self.emitted_sniff_len.min(buffered.len());
            self.emitted_sniff_len = 0;
            let decoded = self.decode(&buffered[decode_start..], false);
            return non_empty_chunk(decoded);
        }

        non_empty_chunk(self.take_safe_ascii_sniff_prefix())
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.decoder.is_none() {
            let encoding = self
                .encoding_ready(true)
                .unwrap_or(self.transport_encoding.unwrap_or(self.fallback_encoding));
            self.start_decoder(encoding, self.emitted_sniff_len > 0);
            let buffered = std::mem::take(&mut self.sniff_buffer);
            let decode_start = self.emitted_sniff_len.min(buffered.len());
            self.emitted_sniff_len = 0;
            let decoded = self.decode(&buffered[decode_start..], true);
            return (!decoded.is_empty()).then_some(decoded);
        }

        let decoded = self.decode(&[], true);
        (!decoded.is_empty()).then_some(decoded)
    }

    fn encoding_ready(&mut self, finishing: bool) -> Option<&'static Encoding> {
        if let Some(encoding) = encoding_for_document_bom(&self.sniff_buffer) {
            return Some(encoding);
        }
        if !finishing && bytes_could_still_be_bom_prefix(&self.sniff_buffer) {
            return None;
        }
        if let Some(encoding) = self.transport_encoding {
            return Some(encoding);
        }
        if let Some(encoding) = encoding_for_document_utf16_xml_prefix(&self.sniff_buffer) {
            return Some(encoding);
        }
        let meta_scan = self.feed_meta_charset_prescan(finishing);
        if let HtmlMetaCharsetScanResult::Found(encoding) = meta_scan {
            return Some(encoding);
        }
        match meta_scan {
            HtmlMetaCharsetScanResult::NotFound => Some(self.encoding_after_prescan()),
            HtmlMetaCharsetScanResult::Pending if finishing => Some(self.encoding_after_prescan()),
            HtmlMetaCharsetScanResult::Pending | HtmlMetaCharsetScanResult::Found(_) => None,
        }
    }

    fn encoding_after_prescan(&self) -> &'static Encoding {
        encoding_for_document_xml_declaration(&self.sniff_buffer)
            .or_else(|| self.detected_legacy_content_encoding())
            .unwrap_or(self.fallback_encoding)
    }

    fn detected_legacy_content_encoding(&self) -> Option<&'static Encoding> {
        self.legacy_encoding_detector
            .and_then(|detector| detector(&self.sniff_buffer, self.url_hint.as_deref()))
    }

    fn feed_meta_charset_prescan(&mut self, finishing: bool) -> HtmlMetaCharsetScanResult {
        let scan = if self.meta_prescan_fed_len < self.sniff_buffer.len() {
            let scan = self
                .meta_charset_parser
                .feed(&self.sniff_buffer[self.meta_prescan_fed_len..]);
            self.meta_prescan_fed_len = self.sniff_buffer.len();
            scan
        } else {
            self.meta_charset_parser.status()
        };
        if finishing && matches!(scan, HtmlMetaCharsetScanResult::Pending) {
            self.meta_charset_parser.finish()
        } else {
            scan
        }
    }

    fn start_decoder(&mut self, encoding: &'static Encoding, stream_prefix_already_emitted: bool) {
        self.selected_encoding = Some(encoding);
        self.decoder = Some(if stream_prefix_already_emitted {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        });
    }

    fn decode(&mut self, bytes: &[u8], last: bool) -> String {
        let mut output = String::new();
        let mut total_read = 0usize;
        loop {
            let input = &bytes[total_read..];
            let reserve = self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.max_utf8_buffer_length(input.len()))
                .unwrap_or_else(|| input.len().saturating_mul(3).saturating_add(16));
            output.reserve(reserve);
            let (result, read, _) = self
                .decoder
                .as_mut()
                .expect("document decoder should be initialized before decode")
                .decode_to_string(input, &mut output, last);
            total_read += read;
            match result {
                CoderResult::InputEmpty => return output,
                CoderResult::OutputFull => continue,
            }
        }
    }

    fn take_safe_ascii_sniff_prefix(&mut self) -> String {
        if self.emitted_sniff_len >= self.sniff_buffer.len()
            || bytes_could_still_be_bom_prefix(&self.sniff_buffer)
            || bytes_could_still_be_utf16_xml_prefix(&self.sniff_buffer)
        {
            return String::new();
        }

        let start = self.emitted_sniff_len;
        let end = self.sniff_buffer[start..]
            .iter()
            .position(|byte| !byte.is_ascii())
            .map(|offset| start + offset)
            .unwrap_or(self.sniff_buffer.len());
        if end == start {
            return String::new();
        }
        self.emitted_sniff_len = end;
        std::str::from_utf8(&self.sniff_buffer[start..end])
            .expect("ASCII sniff prefix must be valid UTF-8")
            .to_owned()
    }
}

fn non_empty_chunk(chunk: String) -> Vec<String> {
    if chunk.is_empty() {
        Vec::new()
    } else {
        vec![chunk]
    }
}

fn bytes_could_still_be_bom_prefix(bytes: &[u8]) -> bool {
    matches!(bytes, [] | [0xEF] | [0xEF, 0xBB] | [0xFF] | [0xFE])
}

fn bytes_could_still_be_utf16_xml_prefix(bytes: &[u8]) -> bool {
    bytes.len() < UTF16LE_XML_PREFIX.len()
        && (UTF16LE_XML_PREFIX.starts_with(bytes) || UTF16BE_XML_PREFIX.starts_with(bytes))
}

fn encoding_for_document_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some(encoding_rs::UTF_8);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Some(encoding_rs::UTF_16BE);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Some(encoding_rs::UTF_16LE);
    }
    None
}

/// Detect BOM-less UTF-16LE/BE documents that begin with an XML declaration.
///
/// The HTML Standard requires the following case-sensitive six-byte patterns:
///   UTF-16LE: 3C 00 3F 00 78 00  ("\<?x")
///   UTF-16BE: 00 3C 00 3F 00 78  ("\<?x")
///
/// These match the start of `<?xml` without requiring a BOM or transport charset.
fn encoding_for_document_utf16_xml_prefix(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.starts_with(UTF16LE_XML_PREFIX) {
        return Some(encoding_rs::UTF_16LE);
    }
    if bytes.starts_with(UTF16BE_XML_PREFIX) {
        return Some(encoding_rs::UTF_16BE);
    }
    None
}

/// Applies the HTML prescan's compatibility parser for an ASCII-compatible
/// XML encoding declaration at the very start of a `text/html` byte stream.
///
/// This deliberately implements the permissive "get an XML encoding"
/// algorithm rather than XML syntax. In particular, only `<?xml` and the
/// lowercase `encoding` keyword are fixed syntax, control bytes are accepted
/// around `=`, and the declaration ends at the first `>` byte.
///
/// <https://html.spec.whatwg.org/multipage/parsing.html#concept-get-xml-encoding>
fn encoding_for_document_xml_declaration(bytes: &[u8]) -> Option<&'static Encoding> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }

    let declaration_end = bytes.iter().position(|byte| *byte == b'>')?;
    let declaration = &bytes[..declaration_end];
    let keyword_offset = declaration
        .windows(b"encoding".len())
        .position(|window| window == b"encoding")?;
    let mut position = keyword_offset + b"encoding".len();

    while declaration.get(position).is_some_and(|byte| *byte <= 0x20) {
        position += 1;
    }
    if declaration.get(position) != Some(&b'=') {
        return None;
    }
    position += 1;
    while declaration.get(position).is_some_and(|byte| *byte <= 0x20) {
        position += 1;
    }

    let quote = *declaration.get(position)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    position += 1;
    let label_end = declaration[position..]
        .iter()
        .position(|byte| *byte == quote)
        .map(|offset| position + offset)?;
    let label = &declaration[position..label_end];
    if label.iter().any(|byte| *byte <= 0x20) {
        return None;
    }

    let encoding = Encoding::for_label(label)?;
    Some(
        if encoding == encoding_rs::UTF_16BE || encoding == encoding_rs::UTF_16LE {
            encoding_rs::UTF_8
        } else {
            encoding
        },
    )
}

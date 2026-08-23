use encoding_rs::{CoderResult, Decoder, Encoding};
use moli_charset_parser::{HtmlMetaCharsetParser, HtmlMetaCharsetScanResult};

use crate::{charset_from_headers, encoding_for_label};

const DEFAULT_HTML_DOCUMENT_ENCODING: &str = "windows-1252";

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
    decoder: Option<Decoder>,
    selected_encoding: Option<&'static Encoding>,
}

impl HtmlDocumentStreamingDecoder {
    pub fn new(headers: &[(String, String)]) -> Self {
        Self::new_with_fallback(headers, None)
    }

    pub fn new_with_fallback(
        headers: &[(String, String)],
        fallback_encoding: Option<&str>,
    ) -> Self {
        Self {
            transport_encoding: charset_from_headers(headers)
                .as_deref()
                .and_then(encoding_for_label),
            fallback_encoding: fallback_encoding
                .and_then(encoding_for_label)
                .unwrap_or(encoding_rs::WINDOWS_1252),
            sniff_buffer: Vec::new(),
            emitted_sniff_len: 0,
            meta_prescan_fed_len: 0,
            meta_charset_parser: HtmlMetaCharsetParser::new(),
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
        if let Some(encoding) = encoding_for_document_xml_declaration(&self.sniff_buffer) {
            return Some(encoding);
        }
        if let Some(encoding) = self.transport_encoding {
            return Some(encoding);
        }
        let meta_scan = self.feed_meta_charset_prescan(finishing);
        if let HtmlMetaCharsetScanResult::Found(encoding) = meta_scan {
            return Some(encoding);
        }
        match meta_scan {
            HtmlMetaCharsetScanResult::NotFound => Some(self.fallback_encoding),
            HtmlMetaCharsetScanResult::Pending if finishing => Some(self.fallback_encoding),
            HtmlMetaCharsetScanResult::Pending | HtmlMetaCharsetScanResult::Found(_) => None,
        }
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
fn encoding_for_document_xml_declaration(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.len() >= 6
        && bytes[0] == 0x3C
        && bytes[1] == 0x00
        && bytes[2] == 0x3F
        && bytes[3] == 0x00
        && bytes[4] == 0x78
        && bytes[5] == 0x00
    {
        return Some(encoding_rs::UTF_16LE);
    }
    if bytes.len() >= 6
        && bytes[0] == 0x00
        && bytes[1] == 0x3C
        && bytes[2] == 0x00
        && bytes[3] == 0x3F
        && bytes[4] == 0x00
        && bytes[5] == 0x78
    {
        return Some(encoding_rs::UTF_16BE);
    }
    None
}

use crate::document::HtmlDocumentStreamingDecoder;
use encoding_rs::{CoderResult, Decoder, Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// In-band encoding declarations are only meaningful for legacy XML text
/// responses and document responses. Explicit `responseType = "text"` does
/// not inspect markup.
#[derive(Clone, Copy)]
pub enum XhrResponseTextKind {
    Text,
    Xml,
    Html,
}

pub struct XhrResponseDecoder {
    kind: XhrResponseTextKind,
    transport_encoding: Option<&'static Encoding>,
    prefix: Vec<u8>,
    html: Option<HtmlDocumentStreamingDecoder>,
    decoder: Option<Decoder>,
    selected_encoding: Option<&'static Encoding>,
}

impl XhrResponseDecoder {
    pub fn new(kind: XhrResponseTextKind, transport_encoding: Option<&'static Encoding>) -> Self {
        let html = matches!(kind, XhrResponseTextKind::Html).then(|| {
            let headers = transport_encoding
                .map(|encoding| {
                    vec![(
                        "content-type".to_owned(),
                        format!("text/html;charset={}", encoding.name()),
                    )]
                })
                .unwrap_or_default();
            HtmlDocumentStreamingDecoder::new_with_fallback(&headers, Some("UTF-8"))
        });
        Self {
            kind,
            transport_encoding,
            prefix: Vec::new(),
            html,
            decoder: None,
            selected_encoding: None,
        }
    }

    pub fn encoding_name(&self) -> &'static str {
        self.html
            .as_ref()
            .map(HtmlDocumentStreamingDecoder::document_encoding_name)
            .unwrap_or_else(|| self.selected_encoding.unwrap_or(UTF_8).name())
    }

    pub fn push(&mut self, bytes: &[u8]) -> String {
        if let Some(html) = self.html.as_mut() {
            return html.push(bytes).concat();
        }
        self.decode(bytes, false)
    }

    pub fn finish(&mut self) -> String {
        if let Some(html) = self.html.as_mut() {
            return html.finish().unwrap_or_default();
        }
        self.decode(&[], true)
    }

    fn decode(&mut self, bytes: &[u8], last: bool) -> String {
        if self.decoder.is_some() {
            return self.decode_selected(bytes, last);
        }
        self.prefix.extend_from_slice(bytes);
        let Some(encoding) = self.select_encoding(last) else {
            return String::new();
        };
        self.selected_encoding = Some(encoding);
        self.decoder = Some(encoding.new_decoder_with_bom_removal());
        let prefix = std::mem::take(&mut self.prefix);
        self.decode_selected(&prefix, last)
    }

    fn select_encoding(&mut self, last: bool) -> Option<&'static Encoding> {
        if let Some((encoding, _)) = Encoding::for_bom(&self.prefix) {
            return Some(encoding);
        }
        if !last
            && matches!(
                self.prefix.as_slice(),
                [] | [0xef] | [0xef, 0xbb] | [0xff] | [0xfe]
            )
        {
            return None;
        }
        if let Some(encoding) = self.transport_encoding {
            return Some(encoding);
        }
        match self.kind {
            XhrResponseTextKind::Text | XhrResponseTextKind::Html => Some(UTF_8),
            XhrResponseTextKind::Xml => {
                for (signature, encoding) in
                    [(b"<\0?\0".as_slice(), UTF_16LE), (b"\0<\0?", UTF_16BE)]
                {
                    if self.prefix.starts_with(signature) {
                        return Some(encoding);
                    }
                    if !last && signature.starts_with(&self.prefix) {
                        return None;
                    }
                }
                if !last && b"<?xml".starts_with(&self.prefix) {
                    return None;
                }
                if self.prefix.starts_with(b"<?xml") {
                    if let Some(end) = self.prefix.iter().position(|byte| *byte == b'>') {
                        if let Some(label) = xml_declaration_encoding(&self.prefix[..end]) {
                            let encoding = Encoding::for_label(label);
                            return Some(encoding.unwrap_or(UTF_8));
                        }
                    } else if !last {
                        return None;
                    }
                }
                Some(UTF_8)
            }
        }
    }

    fn decode_selected(&mut self, bytes: &[u8], last: bool) -> String {
        let decoder = self.decoder.as_mut().expect("XHR encoding was selected");
        let mut output = String::new();
        let mut read = 0;
        loop {
            let input = &bytes[read..];
            output.reserve(
                decoder
                    .max_utf8_buffer_length(input.len())
                    .unwrap_or(input.len().saturating_mul(3).saturating_add(16)),
            );
            let (result, consumed, _) = decoder.decode_to_string(input, &mut output, last);
            read += consumed;
            if result == CoderResult::InputEmpty {
                return output;
            }
        }
    }
}

fn xml_declaration_encoding(declaration: &[u8]) -> Option<&[u8]> {
    let mut rest = declaration.strip_prefix(b"<?xml")?;
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = rest.trim_ascii_start();
        let end = rest
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || *byte == b'=')?;
        let name = &rest[..end];
        rest = rest[end..]
            .trim_ascii_start()
            .strip_prefix(b"=")?
            .trim_ascii_start();
        let quote = *rest.first()?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        rest = &rest[1..];
        let end = rest.iter().position(|byte| *byte == quote)?;
        if name == b"encoding" {
            return Some(&rest[..end]);
        }
        rest = &rest[end + 1..];
    }
    None
}

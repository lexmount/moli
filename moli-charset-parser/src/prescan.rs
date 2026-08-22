use std::cell::RefCell;

use encoding_rs::Encoding;
use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
        states::{Rawtext, Rcdata, ScriptData},
    },
};

pub const HTML_META_CHARSET_PRESCAN_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlMetaCharsetScanResult {
    Pending,
    Found(&'static Encoding),
    NotFound,
}

pub struct HtmlMetaCharsetParser {
    tokenizer: Tokenizer<MetaCharsetTokenSink>,
    bytes_fed_to_tokenizer: usize,
}

impl Default for HtmlMetaCharsetParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlMetaCharsetParser {
    pub fn new() -> Self {
        Self {
            tokenizer: Tokenizer::new(MetaCharsetTokenSink::default(), TokenizerOpts::default()),
            bytes_fed_to_tokenizer: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> HtmlMetaCharsetScanResult {
        if bytes.is_empty() || self.is_done() {
            return self.status();
        }

        if self.bytes_fed_to_tokenizer < HTML_META_CHARSET_PRESCAN_LIMIT {
            let prefix_len =
                (HTML_META_CHARSET_PRESCAN_LIMIT - self.bytes_fed_to_tokenizer).min(bytes.len());
            self.feed_bytes(&bytes[..prefix_len]);
            if self.is_done() {
                return self.status();
            }
            if self.bytes_fed_to_tokenizer >= HTML_META_CHARSET_PRESCAN_LIMIT {
                self.tokenizer.sink.finish_without_charset();
                return self.status();
            }
        }
        self.status()
    }

    pub fn finish(&mut self) -> HtmlMetaCharsetScanResult {
        if !self.is_done() {
            self.tokenizer.end();
            if !self.is_done() {
                self.tokenizer.sink.finish_without_charset();
            }
        }
        self.status()
    }

    pub fn status(&self) -> HtmlMetaCharsetScanResult {
        self.tokenizer.sink.status()
    }

    fn is_done(&self) -> bool {
        !matches!(self.status(), HtmlMetaCharsetScanResult::Pending)
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let input = BufferQueue::default();
        input.push_back(StrTendril::from(byte_prescan_input_as_latin1(bytes)));
        let _ = self.tokenizer.feed(&input);
        self.bytes_fed_to_tokenizer += bytes.len();
    }
}

pub fn sniff_html_meta_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let mut parser = HtmlMetaCharsetParser::new();
    match parser.feed(bytes) {
        HtmlMetaCharsetScanResult::Found(encoding) => Some(encoding),
        HtmlMetaCharsetScanResult::Pending => match parser.finish() {
            HtmlMetaCharsetScanResult::Found(encoding) => Some(encoding),
            HtmlMetaCharsetScanResult::Pending | HtmlMetaCharsetScanResult::NotFound => None,
        },
        HtmlMetaCharsetScanResult::NotFound => None,
    }
}

struct MetaCharsetTokenSink {
    result: RefCell<HtmlMetaCharsetScanResult>,
}

impl Default for MetaCharsetTokenSink {
    fn default() -> Self {
        Self {
            result: RefCell::new(HtmlMetaCharsetScanResult::Pending),
        }
    }
}

impl MetaCharsetTokenSink {
    fn status(&self) -> HtmlMetaCharsetScanResult {
        *self.result.borrow()
    }

    fn finish_without_charset(&self) {
        if !self.is_done() {
            *self.result.borrow_mut() = HtmlMetaCharsetScanResult::NotFound;
        }
    }

    fn process_start_tag(&self, tag: &Tag) {
        if self.is_done() {
            return;
        }
        if tag.name.as_ref() == "meta"
            && let Some(encoding) = meta_charset_from_token(tag)
        {
            *self.result.borrow_mut() = HtmlMetaCharsetScanResult::Found(encoding);
        }
    }

    fn is_done(&self) -> bool {
        !matches!(*self.result.borrow(), HtmlMetaCharsetScanResult::Pending)
    }
}

impl TokenSink for MetaCharsetTokenSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };

        if tag.kind == TagKind::StartTag {
            self.process_start_tag(&tag);
        }

        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }
        match tag.name.as_ref() {
            "script" => TokenSinkResult::RawData(ScriptData),
            "noscript" | "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                TokenSinkResult::RawData(Rawtext)
            }
            "title" | "textarea" => TokenSinkResult::RawData(Rcdata),
            "plaintext" => TokenSinkResult::Plaintext,
            _ => TokenSinkResult::Continue,
        }
    }
}

fn meta_charset_from_token(tag: &Tag) -> Option<&'static Encoding> {
    if let Some(encoding) = html_token_attr_value(tag, "charset").and_then(encoding_for_label) {
        return Some(encoding);
    }

    let has_content_type_pragma = html_token_attr_value(tag, "http-equiv")
        .is_some_and(|value| value.eq_ignore_ascii_case("content-type"));
    has_content_type_pragma
        .then(|| html_token_attr_value(tag, "content"))
        .flatten()
        .and_then(|content| charset_from_content_type(&content))
        .and_then(encoding_for_label)
}

fn html_token_attr_value(tag: &Tag, name: &str) -> Option<String> {
    tag.attrs
        .iter()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|attr| attr.value.to_string())
}

fn encoding_for_label(label: String) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes()).map(prescan_charset_override)
}

/// Applies the two charset rewrites the prescan performs before returning a
/// label it found on a `meta` element.
///
/// A prescan only ever reaches a `meta` element through ASCII-compatible
/// bytes, so a document that declares a UTF-16 encoding there has necessarily
/// mislabeled itself, and the standard decodes it as UTF-8 instead.
/// `x-user-defined` is likewise rewritten to windows-1252, leaving its
/// private-use mapping to apply only when the transport layer asked for it.
///
/// <https://html.spec.whatwg.org/multipage/parsing.html#prescan-a-byte-stream-to-determine-its-encoding>
fn prescan_charset_override(encoding: &'static Encoding) -> &'static Encoding {
    if encoding == encoding_rs::UTF_16BE || encoding == encoding_rs::UTF_16LE {
        encoding_rs::UTF_8
    } else if encoding == encoding_rs::X_USER_DEFINED {
        encoding_rs::WINDOWS_1252
    } else {
        encoding
    }
}

const CHARSET_KEYWORD: &[u8] = b"charset";

/// Extracts a character encoding label from a `meta` element's `content`
/// attribute.
///
/// This is the HTML Standard's own algorithm rather than a MIME parameter
/// parse: it searches for the literal `charset` anywhere in the value, so a
/// `content` attribute carrying no media type still yields its label, and it
/// terminates an unquoted label at the first ASCII whitespace or `;`. A label
/// opened with a quote must be closed by the same quote, otherwise the
/// attribute contributes nothing.
///
/// <https://html.spec.whatwg.org/multipage/parsing.html#algorithm-for-extracting-a-character-encoding-from-a-meta-element>
fn charset_from_content_type(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut position = 0;

    loop {
        let offset = bytes[position..]
            .windows(CHARSET_KEYWORD.len())
            .position(|window| window.eq_ignore_ascii_case(CHARSET_KEYWORD))?;
        let keyword_end = position + offset + CHARSET_KEYWORD.len();
        let separator = keyword_end + leading_ascii_whitespace(&bytes[keyword_end..]);

        if bytes.get(separator) != Some(&b'=') {
            // The keyword was not a label assignment, so resume the search
            // just past it rather than rescanning the same match forever.
            position = separator.max(keyword_end);
            continue;
        }

        let label_start = separator + 1;
        let label_start = label_start + leading_ascii_whitespace(&bytes[label_start..]);
        return charset_label_at(value, label_start).map(str::to_owned);
    }
}

fn charset_label_at(value: &str, start: usize) -> Option<&str> {
    let bytes = value.as_bytes();
    match bytes.get(start) {
        Some(&quote) if quote == b'"' || quote == b'\'' => {
            let quoted_start = start + 1;
            let quoted_len = bytes[quoted_start..]
                .iter()
                .position(|byte| *byte == quote)?;
            Some(&value[quoted_start..quoted_start + quoted_len])
        }
        _ => {
            let end = bytes[start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b';')
                .map_or(bytes.len(), |offset| start + offset);
            Some(&value[start..end])
        }
    }
}

fn leading_ascii_whitespace(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count()
}

fn byte_prescan_input_as_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| *byte as char).collect()
}

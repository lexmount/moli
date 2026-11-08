use std::borrow::Cow;

use encoding_rs::Encoding;

use crate::{encoding_for_label, percent::append_percent_encoded_byte};

pub fn form_submission_encoding(
    accept_charset: Option<&str>,
    document_character_set: &str,
) -> &'static Encoding {
    accept_charset
        .into_iter()
        .flat_map(|value| value.split(|ch: char| ch.is_ascii_whitespace() || ch == ','))
        .find_map(form_output_encoding_for_label)
        .or_else(|| form_output_encoding_for_label(document_character_set))
        .unwrap_or(encoding_rs::UTF_8)
}

pub fn form_output_encoding_for_label(label: &str) -> Option<&'static Encoding> {
    encoding_for_label(label).map(form_output_encoding)
}

/// Selects the output encoding for form and URL serialization. HTML meta
/// charset rewrites are handled separately by the document prescan.
pub fn form_output_encoding(encoding: &'static Encoding) -> &'static Encoding {
    encoding.output_encoding()
}

pub fn encode_text_for_legacy_web<'a>(
    input: &'a str,
    encoding: &'static Encoding,
) -> Cow<'a, [u8]> {
    // encoding_rs emits HTML decimal numeric character references for unmappable
    // scalar values here, which is the legacy form/query output behavior.
    form_output_encoding(encoding).encode(input).0
}

pub fn is_charset_sentinel_name(name: &str) -> bool {
    // Browser form serialization treats ASCII-case variants of `_charset_` as
    // the encoding sentinel, but must not fold non-ASCII lookalikes.
    name.eq_ignore_ascii_case("_charset_")
}

pub fn form_urlencoded_serialize_pairs<I, N, V>(pairs: I, encoding: &'static Encoding) -> String
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<str>,
    V: AsRef<str>,
{
    let mut output = String::new();
    for (index, (name, value)) in pairs.into_iter().enumerate() {
        if index > 0 {
            output.push('&');
        }
        append_form_urlencoded_component(&mut output, name.as_ref(), encoding);
        output.push('=');
        append_form_urlencoded_component(&mut output, value.as_ref(), encoding);
    }
    output
}

fn append_form_urlencoded_component(output: &mut String, input: &str, encoding: &'static Encoding) {
    for byte in encode_text_for_legacy_web(input, encoding).as_ref() {
        match *byte {
            b'*' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'_' | b'a'..=b'z' => {
                output.push(*byte as char);
            }
            b' ' => output.push('+'),
            byte => append_percent_encoded_byte(output, byte),
        }
    }
}

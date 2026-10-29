use std::borrow::Cow;

use encoding_rs::{EncoderResult, Encoding};

use crate::{
    form_output_encoding,
    percent::{append_percent_encoded_byte, append_percent_encoded_ncr},
};

pub fn encode_url_query_for_legacy_web<'a>(
    input: &'a str,
    encoding: &'static Encoding,
) -> Cow<'a, str> {
    let encoding = form_output_encoding(encoding);
    if encoding == encoding_rs::UTF_8 {
        return Cow::Borrowed(input);
    }
    let Some(query_start) = query_delimiter_before_fragment(input) else {
        return Cow::Borrowed(input);
    };
    let query_value_start = query_start + 1;
    let fragment_start = input[query_value_start..]
        .find('#')
        .map(|offset| query_value_start + offset)
        .unwrap_or(input.len());
    let query = &input[query_value_start..fragment_start];
    if query.is_empty() {
        return Cow::Borrowed(input);
    }

    let mut output = String::with_capacity(input.len());
    output.push_str(&input[..query_value_start]);
    append_url_query(&mut output, query, encoding);
    output.push_str(&input[fragment_start..]);
    Cow::Owned(output)
}

/// Encodes the query supplied by a URL parser's encoding override callback.
///
/// The parser selects eligible schemes and removes URL whitespace before
/// calling this. Unmappable characters need percent-encoded numeric character
/// references so their `&`, `#`, and `;` cannot become query delimiters.
pub fn encode_url_query_for_url_parser<'a>(
    input: &'a str,
    encoding: &'static Encoding,
) -> Cow<'a, [u8]> {
    let encoding = form_output_encoding(encoding);
    if encoding == encoding_rs::UTF_8 || input.is_empty() {
        return Cow::Borrowed(input.as_bytes());
    }
    let mut output = String::with_capacity(input.len());
    append_url_query(&mut output, input, encoding);
    Cow::Owned(output.into_bytes())
}

fn query_delimiter_before_fragment(input: &str) -> Option<usize> {
    input
        .find('?')
        .filter(|query_start| input[..*query_start].find('#').is_none())
}

fn append_url_query(output: &mut String, query: &str, encoding: &'static Encoding) {
    let mut encoder = encoding.new_encoder();
    let mut input = query;
    let mut encoded = Vec::new();
    loop {
        let last = true;
        let capacity = encoder
            .max_buffer_length_from_utf8_without_replacement(input.len())
            .unwrap_or(input.len())
            .max(1);
        if encoded.capacity() < capacity {
            encoded.reserve(capacity - encoded.capacity());
        }
        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(input, &mut encoded, last);
        append_url_query_bytes(output, &encoded);
        encoded.clear();
        input = &input[read..];
        match result {
            EncoderResult::InputEmpty => break,
            EncoderResult::OutputFull => encoded.reserve(encoded.capacity().max(1)),
            EncoderResult::Unmappable(ch) => append_percent_encoded_ncr(output, ch),
        }
    }
}

fn append_url_query_bytes(output: &mut String, input: &[u8]) {
    for byte in input {
        match *byte {
            0x00..=0x1F | 0x7F | b' ' | b'"' | b'#' | b'\'' | b'<' | b'>' | 0x80..=0xFF => {
                append_percent_encoded_byte(output, *byte);
            }
            byte => output.push(byte as char),
        }
    }
}

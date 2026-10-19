use moli_header_field::{HeaderFieldTokenMode, HeaderFieldTokenizer};

fn supported_content_codings() -> impl Iterator<Item = &'static str> {
    // The bounded native decoder supplies the same coding set even when the
    // linked libcurl lacks Zstandard or external-dictionary support.
    ["deflate", "gzip", "br", "zstd", "dcb", "dcz"].into_iter()
}

/// Choose the browser's header before cache lookup and transport setup so
/// negotiation and Vary matching use the same actual value.
pub(crate) fn default_accept_encoding() -> String {
    let value = supported_content_codings()
        .filter(|coding| !matches!(*coding, "dcb" | "dcz"))
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        "identity".to_owned()
    } else {
        value
    }
}

pub(crate) fn dictionary_accept_encoding() -> String {
    supported_content_codings().collect::<Vec<_>>().join(", ")
}

/// Resource Timing exposes registered, supported content codings rather than
/// reflecting an arbitrary response header into the timing entry.
pub fn resource_timing_content_encoding(headers: &[(String, Vec<u8>)]) -> &'static str {
    let mut codings = Vec::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
    {
        let value = crate::decode_header_value(value);
        for part in crate::split_http_header_list(&value) {
            if part.is_empty() {
                codings.push(String::new());
                continue;
            }
            let mut tokenizer = HeaderFieldTokenizer::new(part);
            let Some(coding) = tokenizer.consume_token(HeaderFieldTokenMode::Normal) else {
                return "";
            };
            if !tokenizer.is_consumed() {
                return "";
            }
            codings.push(coding.to_ascii_lowercase());
        }
    }
    let [coding] = codings.as_slice() else {
        return if codings.is_empty() { "" } else { "multiple" };
    };
    if coding.is_empty() {
        return "";
    }
    supported_content_codings()
        .find(|supported| *supported == coding)
        .unwrap_or("@unknown")
}

fn supported_content_codings() -> impl Iterator<Item = &'static str> {
    let version = curl::Version::get();
    [
        ("deflate", version.feature_libz()),
        ("gzip", version.feature_libz()),
        ("br", version.feature_brotli()),
        ("zstd", version.feature_zstd()),
    ]
    .into_iter()
    .filter_map(|(coding, supported)| supported.then_some(coding))
}

/// Choose the browser's header before cache lookup and transport setup so
/// negotiation and Vary matching use the same actual value.
pub(crate) fn default_accept_encoding() -> String {
    let value = supported_content_codings().collect::<Vec<_>>().join(", ");
    if value.is_empty() {
        "identity".to_owned()
    } else {
        value
    }
}

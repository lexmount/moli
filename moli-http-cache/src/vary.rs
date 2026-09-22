use rustc_hash::FxHashSet;

/// Parses all Vary response header fields into normalized field names.
///
/// Returns `None` for `Vary: *`, which cannot be safely matched by this cache.
pub fn response_vary_header_names(headers: &[(String, Vec<u8>)]) -> Option<Vec<String>> {
    let headers = moli_header_field::headers_to_byte_strings(headers);
    let mut seen = FxHashSet::default();
    let mut out = Vec::new();
    for raw_name in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("vary"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if raw_name == "*" {
            return None;
        }
        let normalized_name = raw_name.to_ascii_lowercase();
        if seen.insert(normalized_name.clone()) {
            out.push(normalized_name);
        }
    }
    Some(out)
}

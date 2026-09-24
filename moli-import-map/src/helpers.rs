use serde_json::Value;
use url::Url;

use crate::types::ResolvedModuleRecord;

pub(crate) fn import_map_entry_value(value: Option<&Url>) -> Value {
    value
        .map(|url| Value::String(url.as_str().to_owned()))
        .unwrap_or(Value::Null)
}

pub(crate) fn import_map_error_message(error: ::import_map::ImportMapError) -> String {
    match error.0.as_ref() {
        ::import_map::ImportMapErrorKind::UnmappedBareSpecifier(specifier, _) => {
            format!("failed to resolve bare module specifier `{specifier}` without import map")
        }
        _ => error.to_string(),
    }
}

pub(crate) fn scope_matches_record(scope_prefix: &Url, record: &ResolvedModuleRecord) -> bool {
    let prefix = scope_prefix.as_str();
    prefix == record.base_url || (prefix.ends_with('/') && record.base_url.starts_with(prefix))
}

pub(crate) fn resolved_module_matches_rule(
    specifier: &str,
    is_package_prefix: bool,
    record: &ResolvedModuleRecord,
) -> bool {
    if !is_package_prefix {
        return record.normalized_specifier == specifier;
    }
    record.normalized_specifier.starts_with(specifier)
        && record.specifier_url.as_ref().is_none_or(is_special_scheme)
}

pub(crate) fn is_special_scheme(url: &Url) -> bool {
    matches!(
        url.scheme(),
        "ftp" | "file" | "http" | "https" | "ws" | "wss"
    )
}

/// Resolve a URL-like module specifier without consulting import maps.
/// Bare specifiers return `None`; only `/`, `./`, and `../` prefixes use the base URL.
pub fn resolve_url_like_module_specifier(specifier: &str, base_url: &Url) -> Option<Url> {
    if specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../") {
        return base_url.join(specifier).ok();
    }
    Url::parse(specifier).ok()
}

pub(crate) fn normalize_module_specifier(specifier: &str, base_url: &Url) -> (String, Option<Url>) {
    let as_url = resolve_url_like_module_specifier(specifier, base_url);
    let normalized_specifier = as_url
        .as_ref()
        .map(|url| url.as_str().to_owned())
        .unwrap_or_else(|| specifier.to_owned());
    (normalized_specifier, as_url)
}

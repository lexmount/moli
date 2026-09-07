/// Installed page-level request policy. Frontends resolve their contributions
/// before supplying this value; context header defaults remain context-owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkRequestPolicy {
    pub cache_disabled: bool,
    pub bypass_service_worker: bool,
    pub blocked_url_patterns: Vec<String>,
    pub extra_headers: moli_fetch::RequestHeaders,
}
pub fn merge_extra_header_layers(
    layers: &[&moli_fetch::RequestHeaders],
) -> moli_fetch::RequestHeaders {
    let mut headers = moli_fetch::RequestHeaders::default();
    for layer in layers {
        for (name, value) in *layer {
            headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            headers.push((name.clone(), value.clone()));
        }
    }
    headers
}

/// Installed page-level request policy. Frontends resolve their contributions
/// before supplying this value; context header defaults remain context-owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkRequestPolicy {
    pub cache_disabled: bool,
    pub bypass_service_worker: bool,
    pub blocked_url_patterns: Vec<String>,
    pub extra_headers: Vec<(String, String)>,
}
pub fn merge_extra_header_layers(layers: &[&[(String, String)]]) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for layer in layers {
        for (name, value) in *layer {
            headers.retain(|(existing, _)| existing != name);
            headers.push((name.clone(), value.clone()));
        }
    }
    headers
}

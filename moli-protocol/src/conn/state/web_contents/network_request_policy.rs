/// Installed page-level request policy. Frontends resolve their contributions
/// before supplying this value; context header defaults remain context-owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::conn) struct NetworkRequestPolicy {
    pub(in crate::conn) cache_disabled: bool,
    pub(in crate::conn) bypass_service_worker: bool,
    pub(in crate::conn) blocked_url_patterns: Vec<String>,
    pub(in crate::conn) extra_headers: Vec<(String, String)>,
}

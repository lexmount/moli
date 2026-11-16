use url::Url;

pub(super) fn service_worker_scope_matches_url(scope_url: &Url, candidate_url: &Url) -> bool {
    // Scope matching uses serialized URL prefixes, including partial path
    // segments and query strings, rather than directory boundaries.
    candidate_url.as_str().starts_with(scope_url.as_str())
}

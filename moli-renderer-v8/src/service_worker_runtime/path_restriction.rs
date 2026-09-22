use url::Url;

const SERVICE_WORKER_ALLOWED_HEADER: &str = "Service-Worker-Allowed";

pub(super) fn service_worker_allowed_header_value(headers: &[(String, Vec<u8>)]) -> Option<String> {
    moli_web_mime::response_header_value(headers, SERVICE_WORKER_ALLOWED_HEADER)
        .map(|value| value.trim().to_owned())
}

pub(super) fn verify_service_worker_script_path_restriction(
    scope_url: &Url,
    script_response_url: &Url,
    service_worker_allowed_header_value: Option<&str>,
) -> Result<(), String> {
    if scope_or_script_contains_disallowed_escape(scope_url, script_response_url) {
        return Err(format!(
            "The provided scope ('{}') or scriptURL ('{}') includes a disallowed escape character.",
            scope_url, script_response_url
        ));
    }

    let (max_scope_path, from_allowed_header) = match service_worker_allowed_header_value {
        Some(value) => {
            let max_scope = script_response_url.join(value).map_err(|_| {
                format!(
                    "An invalid Service-Worker-Allowed header value ('{value}') was received when fetching the script."
                )
            })?;
            if !moli_url::same_origin(&max_scope, script_response_url) {
                return Err(format!(
                    "A cross-origin Service-Worker-Allowed header value ('{value}') was received when fetching the script."
                ));
            }
            (max_scope.path().to_owned(), true)
        }
        None => (script_directory_path(script_response_url), false),
    };

    let scope_path = scope_url.path();
    if scope_path.starts_with(&max_scope_path) {
        return Ok(());
    }

    let mut message = format!(
        "The path of the provided scope ('{scope_path}') is not under the max scope allowed ("
    );
    if from_allowed_header {
        message.push_str("set by Service-Worker-Allowed: ");
    }
    message.push('\'');
    message.push_str(&max_scope_path);
    message.push_str(
        "'). Adjust the scope, move the Service Worker script, or use the Service-Worker-Allowed HTTP header to allow the scope.",
    );
    Err(message)
}

fn scope_or_script_contains_disallowed_escape(scope_url: &Url, script_url: &Url) -> bool {
    path_contains_disallowed_escape(scope_url.path())
        || path_contains_disallowed_escape(script_url.path())
}

fn path_contains_disallowed_escape(path: &str) -> bool {
    path.contains("%2f") || path.contains("%2F") || path.contains("%5c") || path.contains("%5C")
}

fn script_directory_path(script_url: &Url) -> String {
    let path = script_url.path();
    path.rfind('/')
        .map(|index| path[..=index].to_owned())
        .unwrap_or_else(|| "/".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        service_worker_allowed_header_value, verify_service_worker_script_path_restriction,
    };

    fn url(value: &str) -> url::Url {
        url::Url::parse(value).unwrap()
    }

    fn assert_allowed(scope: &str, script: &str, allowed_header: Option<&str>) {
        verify_service_worker_script_path_restriction(&url(scope), &url(script), allowed_header)
            .unwrap_or_else(|error| panic!("{scope} should be allowed for {script}: {error}"));
    }

    fn assert_rejected(scope: &str, script: &str, allowed_header: Option<&str>) -> String {
        verify_service_worker_script_path_restriction(&url(scope), &url(script), allowed_header)
            .expect_err("path restriction should reject")
    }

    #[test]
    fn default_path_restriction_uses_script_directory() {
        assert_allowed("https://example.test/", "https://example.test/sw.js", None);
        assert_allowed(
            "https://example.test/app/",
            "https://example.test/app/sw.js",
            None,
        );
        assert_allowed(
            "https://example.test/app/page",
            "https://example.test/app/sw.js",
            None,
        );
        assert_rejected(
            "https://example.test/",
            "https://example.test/app/sw.js",
            None,
        );
        assert_rejected(
            "https://example.test/other/",
            "https://example.test/app/sw.js",
            None,
        );
        assert_rejected(
            "https://example.test/app",
            "https://example.test/app/sw.js",
            None,
        );
    }

    #[test]
    fn path_restriction_ignores_url_queries() {
        assert_allowed(
            "https://example.test/app/?scope-query",
            "https://example.test/app/sw.js?script-query",
            None,
        );
        assert_rejected(
            "https://example.test/?scope-query",
            "https://example.test/app/sw.js?script-query",
            None,
        );
    }

    #[test]
    fn service_worker_allowed_header_can_widen_or_restrict_scope() {
        assert_allowed(
            "https://example.test/",
            "https://example.test/app/sw.js",
            Some("/"),
        );
        assert_allowed(
            "https://example.test/",
            "https://example.test/app/sw.js",
            Some(".."),
        );
        assert_allowed(
            "https://example.test/bar/",
            "https://example.test/app/sw.js",
            Some("../b"),
        );
        assert_rejected(
            "https://example.test/bar/",
            "https://example.test/app/sw.js",
            Some("../c"),
        );
        assert_rejected(
            "https://example.test/",
            "https://example.test/sw.js",
            Some("app"),
        );
        assert_allowed(
            "https://example.test/app/",
            "https://example.test/sw.js",
            Some("app"),
        );
    }

    #[test]
    fn service_worker_allowed_header_resolves_against_script_response_url() {
        assert_allowed(
            "https://example.test/final/",
            "https://example.test/redirected/sw.js",
            Some("../final/"),
        );
        assert_rejected(
            "https://example.test/request/",
            "https://example.test/redirected/sw.js",
            Some("../final/"),
        );
    }

    #[test]
    fn empty_service_worker_allowed_header_resolves_to_script_url_path() {
        assert_rejected(
            "https://example.test/",
            "https://example.test/sw.js",
            Some(""),
        );
        assert_allowed(
            "https://example.test/sw.js/child",
            "https://example.test/sw.js",
            Some(""),
        );
    }

    #[test]
    fn service_worker_allowed_header_rejects_cross_origin_values() {
        let error = assert_rejected(
            "https://example.test/app/",
            "https://example.test/app/sw.js",
            Some("https://other.test/app/"),
        );
        assert!(error.contains("cross-origin Service-Worker-Allowed header value"));
    }

    #[test]
    fn service_worker_allowed_header_rejects_invalid_values() {
        let error = assert_rejected(
            "https://example.test/app/",
            "https://example.test/app/sw.js",
            Some("https://[invalid/"),
        );
        assert!(error.contains("invalid Service-Worker-Allowed header value"));
    }

    #[test]
    fn path_restriction_rejects_escaped_slash_or_backslash_in_paths() {
        let slash_scope = assert_rejected(
            "https://example.test/app%2f/",
            "https://example.test/app/sw.js",
            None,
        );
        assert!(slash_scope.contains("disallowed escape character"));

        let slash_script = assert_rejected(
            "https://example.test/app/",
            "https://example.test/app%2Fsw.js",
            None,
        );
        assert!(slash_script.contains("disallowed escape character"));

        let backslash_scope = assert_rejected(
            "https://example.test/app%5c/",
            "https://example.test/app/sw.js",
            None,
        );
        assert!(backslash_scope.contains("disallowed escape character"));

        let backslash_script = assert_rejected(
            "https://example.test/app/",
            "https://example.test/app%5Csw.js",
            None,
        );
        assert!(backslash_script.contains("disallowed escape character"));
    }

    #[test]
    fn escaped_slash_or_backslash_in_query_does_not_reject() {
        assert_allowed(
            "https://example.test/app/?q=%2f",
            "https://example.test/app/sw.js?script=%5c",
            None,
        );
    }

    #[test]
    fn allowed_header_value_is_case_insensitive_and_trimmed() {
        let headers: Vec<(String, Vec<u8>)> =
            vec![("service-worker-allowed".to_owned(), b" /app/ ".to_vec())];
        assert_eq!(
            service_worker_allowed_header_value(&headers).as_deref(),
            Some("/app/")
        );
    }
}

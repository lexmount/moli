use moli_web_mime::{
    FetchDestination, ScriptResponseMimeError, check_script_response_mime, response_header_values,
};
use url::Url;

pub(crate) fn ensure_worker_script_mime_acceptable(
    script_url: &Url,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(), String> {
    check_script_response_mime(headers, body, FetchDestination::Worker, true)
        .map_err(|error| worker_script_mime_error_message(script_url, error))
}

pub(crate) fn worker_response_content_type(headers: &[(String, String)]) -> Option<String> {
    response_header_values(headers, "content-type")
        .into_iter()
        .next_back()
}

pub(crate) fn worker_response_has_webassembly_mime(headers: &[(String, String)]) -> bool {
    worker_response_content_type(headers)
        .as_deref()
        .is_some_and(moli_web_mime::is_webassembly_mime)
}

fn worker_script_mime_error_message(script_url: &Url, error: ScriptResponseMimeError) -> String {
    match error {
        ScriptResponseMimeError::Nosniff => format!(
            "Failed to load worker script `{script_url}`: blocked by X-Content-Type-Options nosniff."
        ),
        ScriptResponseMimeError::Unsupported(mime_type) => format!(
            "Failed to load worker script `{script_url}`: unsupported script MIME type `{mime_type}`."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_script_mime_accepts_javascript_content_types() {
        let url = Url::parse("https://example.test/worker.js").expect("valid url");
        let headers = vec![(
            "Content-Type".to_owned(),
            "Text/JavaScript; charset=utf-8".to_owned(),
        )];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_ok());
    }

    #[test]
    fn worker_script_mime_rejects_http_non_javascript_content_types() {
        let url = Url::parse("https://example.test/worker.py").expect("valid url");
        let headers = vec![("content-type".to_owned(), "text/html".to_owned())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_err());
    }

    #[test]
    fn worker_script_mime_rejects_missing_content_type() {
        let blob_url = Url::parse("blob:https://example.test/id").expect("valid url");
        let http_url = Url::parse("https://example.test/worker").expect("valid url");

        // Classic blob/data main scripts are materialized without this check;
        // imported scripts and modules still require a JavaScript MIME type.
        assert!(ensure_worker_script_mime_acceptable(&blob_url, &[], b"").is_err());
        assert!(ensure_worker_script_mime_acceptable(&http_url, &[], b"").is_err());
    }

    #[test]
    fn worker_script_mime_rejects_invalid_content_type() {
        let url = Url::parse("https://example.test/worker").expect("valid url");
        let headers = vec![("content-type".to_owned(), "not a mime type".to_owned())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_err());
    }

    #[test]
    fn worker_script_mime_rejects_nosniff_missing_content_type() {
        let url = Url::parse("https://example.test/worker").expect("valid url");
        let headers = vec![("x-content-type-options".to_owned(), "nosniff".to_owned())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_err());
    }
}

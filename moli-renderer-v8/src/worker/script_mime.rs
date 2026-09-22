use moli_web_mime::{
    FetchDestination, ScriptResponseMimeError, check_script_response_mime, response_header_values,
};
use url::Url;

pub(crate) fn ensure_worker_script_mime_acceptable(
    script_url: &Url,
    headers: &[(String, Vec<u8>)],
    body: &[u8],
) -> Result<(), String> {
    check_script_response_mime(headers, body, FetchDestination::Worker, true)
        .map_err(|error| worker_script_mime_error_message(script_url, error))
}

pub(crate) fn worker_response_content_type(headers: &[(String, Vec<u8>)]) -> Option<String> {
    response_header_values(headers, "content-type")
        .into_iter()
        .next_back()
}

pub(crate) fn worker_response_has_webassembly_mime(headers: &[(String, Vec<u8>)]) -> bool {
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
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "Content-Type".to_owned(),
            b"Text/JavaScript; charset=utf-8".to_vec(),
        )];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_ok());
    }

    #[test]
    fn worker_script_mime_rejects_http_non_javascript_content_types() {
        let url = Url::parse("https://example.test/worker.py").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> =
            vec![("content-type".to_owned(), b"text/html".to_vec())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_err());
    }

    #[test]
    fn worker_script_mime_does_not_reject_blob_or_missing_content_type() {
        let blob_url = Url::parse("blob:https://example.test/id").expect("valid url");
        let http_url = Url::parse("https://example.test/worker").expect("valid url");

        assert!(ensure_worker_script_mime_acceptable(&blob_url, &[], b"").is_ok());
        assert!(ensure_worker_script_mime_acceptable(&http_url, &[], b"").is_ok());
    }

    #[test]
    fn worker_script_mime_allows_invalid_content_type_through_script_context_default() {
        let url = Url::parse("https://example.test/worker").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> =
            vec![("content-type".to_owned(), b"not a mime type".to_vec())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_ok());
    }

    #[test]
    fn worker_script_mime_rejects_nosniff_missing_content_type() {
        let url = Url::parse("https://example.test/worker").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> =
            vec![("x-content-type-options".to_owned(), b"nosniff".to_vec())];

        assert!(ensure_worker_script_mime_acceptable(&url, &headers, b"").is_err());
    }
}

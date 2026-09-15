use super::*;
use moli_web_mime::data_url_body_and_mime_type;

pub(crate) fn blob_url_response(url: &url::Url) -> Option<Response> {
    let (body_bytes, mime_type) = blob::object_url_bytes_and_type(url.as_str())?;
    Some(blob_response(url, body_bytes, mime_type))
}

pub(super) fn blob_response(url: &url::Url, body_bytes: Vec<u8>, mime_type: String) -> Response {
    let mut headers = Vec::new();
    if !mime_type.is_empty() {
        headers.push(("Content-Type".to_owned(), mime_type));
    }
    Response::from_head_and_lossy_body_bytes(
        moli_fetch::ResponseHead {
            status_text: None,
            final_url: url.clone(),
            status: 200,
            headers,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_bytes,
    )
}

pub(crate) fn data_url_response(url: &url::Url) -> Option<Response> {
    let (body_bytes, mime_type) = data_url_body_and_mime_type(url.as_str())?;
    Some(Response::from_head_and_lossy_body_bytes(
        moli_fetch::ResponseHead {
            status_text: None,
            final_url: url.clone(),
            status: 200,
            headers: vec![("Content-Type".to_owned(), mime_type)],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_bytes,
    ))
}

/// Reads a local resource for consumers whose requests always use GET.
pub(crate) fn local_url_response(url: &url::Url) -> Option<Response> {
    local_url_response_result(url, "GET").and_then(Result::ok)
}

/// Resolves renderer-owned URL schemes without falling through to the network
/// transport when the local resource is malformed, revoked, or unavailable.
///
/// `None` means that the URL is not owned by this resolver. `Some(Err(..))`
/// means that it is a local URL and therefore must fail locally instead of
/// being handed to libcurl.
/// `method` is the request's already normalized method.
pub(crate) fn local_url_response_result(
    url: &url::Url,
    method: &str,
) -> Option<Result<Response, String>> {
    local_url_response_with_blob_entry(url, method, None)
}

pub(crate) fn local_url_response_with_blob_entry(
    url: &url::Url,
    method: &str,
    entry: Option<&CapturedBlobUrl>,
) -> Option<Result<Response, String>> {
    let result = match url.scheme() {
        "blob" if method != "GET" => {
            Some(Err(format!("blob URL fetch requires GET, got `{method}`")))
        }
        "blob" => {
            let response = match entry.filter(|entry| entry.matches(url)) {
                Some(entry) => entry.response(url),
                None => blob_url_response(url),
            };
            Some(response.ok_or_else(|| format!("blob URL `{url}` is unavailable")))
        }
        "data" => {
            Some(data_url_response(url).ok_or_else(|| format!("data URL `{url}` is invalid")))
        }
        _ => None,
    }?;
    Some(result.map(|response| {
        if !response_has_null_body(method, response.head().status) {
            return response;
        }
        // Apply the Fetch body filter before recording or delivering a local
        // response, while preserving its status, MIME type and other headers.
        let (head, _) = response.into_body();
        Response::from_head_and_lossy_body_bytes(head, Vec::new())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_response_decodes_plain_and_base64_payloads() {
        let plain = url::Url::parse("data:,BB-8").unwrap();
        let (_, body) = data_url_response(&plain).unwrap().into_body();
        let (text, _) = body.try_into_lossy_materialized_text().unwrap();
        assert_eq!(text, "BB-8");

        let base64 = url::Url::parse("data:text/plain;base64,Sy0yU08=").unwrap();
        let (_, body) = data_url_response(&base64).unwrap().into_body();
        let (text, _) = body.try_into_lossy_materialized_text().unwrap();
        assert_eq!(text, "K-2SO");
    }

    #[test]
    fn data_url_response_preserves_supplied_charset_parameter() {
        let url = url::Url::parse("data:text/html;charset=iso-2022-jp,hello").unwrap();
        let response = data_url_response(&url).unwrap();

        assert_eq!(
            response
                .head()
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str()),
            Some("text/html;charset=iso-2022-jp")
        );
    }

    #[test]
    fn unavailable_blob_url_is_a_local_failure() {
        let url = url::Url::parse("blob:https://example.test/not-registered").unwrap();

        let error = local_url_response_result(&url, "GET")
            .expect("blob URL must be owned by the local resolver")
            .expect_err("an unregistered blob URL must fail locally");

        assert_eq!(
            error,
            "blob URL `blob:https://example.test/not-registered` is unavailable"
        );
    }
}

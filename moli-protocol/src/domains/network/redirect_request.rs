/// Project the request metadata for successive followed redirect hops.
///
/// This shares FetchRequest::apply_redirect_status rules: the emitted CDP request
/// must describe the new hop, not repeat the original POST after it became GET.
pub(super) struct RedirectRequest<'a> {
    pub(super) method: &'a str,
    pub(super) body: Option<&'a str>,
    pub(super) headers: Vec<(String, String)>,
}

impl<'a> RedirectRequest<'a> {
    pub(super) fn new(
        method: &'a str,
        body: Option<&'a str>,
        headers: &[(String, String)],
    ) -> Self {
        Self {
            method,
            body,
            headers: headers.to_vec(),
        }
    }

    pub(super) fn follow(&mut self, status: u16) {
        if moli_fetch::redirect_status_rewrites_to_get(status, self.method) {
            self.method = "GET";
            self.body = None;
            self.headers
                .retain(|(name, _)| !moli_fetch::is_request_body_header_name(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RedirectRequest;

    #[test]
    fn redirect_status_method_matrix_matches_fetch_semantics() {
        // Explicit HTTP expectations, independent of the implementation's
        // classification formula. None means the redirect discards the body.
        for (status, method, expected_method, expected_body) in [
            (301, "GET", "GET", Some("value")),
            (301, "HEAD", "HEAD", Some("value")),
            (301, "POST", "GET", None),
            (301, "PUT", "PUT", Some("value")),
            (301, "DELETE", "DELETE", Some("value")),
            (301, "post", "GET", None),
            (302, "GET", "GET", Some("value")),
            (302, "HEAD", "HEAD", Some("value")),
            (302, "POST", "GET", None),
            (302, "PUT", "PUT", Some("value")),
            (302, "DELETE", "DELETE", Some("value")),
            (302, "post", "GET", None),
            (303, "GET", "GET", Some("value")),
            (303, "HEAD", "HEAD", Some("value")),
            (303, "POST", "GET", None),
            (303, "PUT", "GET", None),
            (303, "DELETE", "GET", None),
            (303, "post", "GET", None),
            (307, "GET", "GET", Some("value")),
            (307, "HEAD", "HEAD", Some("value")),
            (307, "POST", "POST", Some("value")),
            (307, "PUT", "PUT", Some("value")),
            (307, "DELETE", "DELETE", Some("value")),
            (307, "post", "post", Some("value")),
            (308, "GET", "GET", Some("value")),
            (308, "HEAD", "HEAD", Some("value")),
            (308, "POST", "POST", Some("value")),
            (308, "PUT", "PUT", Some("value")),
            (308, "DELETE", "DELETE", Some("value")),
            (308, "post", "post", Some("value")),
        ] {
            let headers = vec![
                ("Content-Type".into(), "text/plain".into()),
                ("CONTENT-LENGTH".into(), "5".into()),
                ("Accept".into(), "*/*".into()),
            ];
            let mut request = RedirectRequest::new(method, Some("value"), &headers);
            request.follow(status);
            assert_eq!(request.method, expected_method, "{status} {method}");
            assert_eq!(request.body, expected_body, "{status} {method}");
            let expected_headers = if expected_body.is_none() {
                vec![("Accept".into(), "*/*".into())]
            } else {
                headers.clone()
            };
            assert_eq!(request.headers, expected_headers, "{status} {method}");
            assert_eq!(headers.len(), 3, "original metadata stays unchanged");
        }
    }

    #[test]
    fn redirected_get_does_not_recover_original_post_on_later_307() {
        let mut request = RedirectRequest::new("POST", Some("value"), &[]);
        request.follow(302);
        request.follow(307);
        assert_eq!(request.method, "GET");
        assert_eq!(request.body, None);
    }
}

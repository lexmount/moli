/// Project the request metadata for successive followed redirect hops.
///
/// This mirrors FetchRequest::apply_redirect_status: the emitted CDP request
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
        let becomes_get = matches!(status, 301 | 302) && self.method.eq_ignore_ascii_case("POST")
            || status == 303
                && !self.method.eq_ignore_ascii_case("GET")
                && !self.method.eq_ignore_ascii_case("HEAD");
        if becomes_get {
            self.method = "GET";
            self.body = None;
            self.headers.retain(|(name, _)| {
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-encoding"
                        | "content-language"
                        | "content-length"
                        | "content-location"
                        | "content-type"
                )
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RedirectRequest;

    #[test]
    fn redirect_status_method_matrix_matches_fetch_semantics() {
        for status in [301, 302, 303, 307, 308] {
            for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "post"] {
                let headers = vec![
                    ("Content-Type".into(), "text/plain".into()),
                    ("CONTENT-LENGTH".into(), "5".into()),
                    ("Accept".into(), "*/*".into()),
                ];
                let mut request = RedirectRequest::new(method, Some("value"), &headers);
                request.follow(status);
                let changed = matches!(status, 301 | 302) && method.eq_ignore_ascii_case("POST")
                    || status == 303 && !matches!(method, "GET" | "HEAD");
                assert_eq!(request.method, if changed { "GET" } else { method });
                assert_eq!(request.body, if changed { None } else { Some("value") });
                assert_eq!(request.headers.len(), if changed { 1 } else { 3 });
                assert_eq!(headers.len(), 3, "original metadata stays unchanged");
            }
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

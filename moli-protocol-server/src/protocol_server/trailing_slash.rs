use std::task::{Context, Poll};

use axum::http::{Request, Uri};
use tower::{Layer, Service};

/// Accept a single trailing slash without registering a second route tree.
/// Canonical-only paths retain their existing unmatched trailing-slash form.
#[derive(Clone, Copy)]
pub(super) struct TrailingSlashLayer {
    canonical_only: &'static [&'static str],
}

impl TrailingSlashLayer {
    pub(super) const fn new(canonical_only: &'static [&'static str]) -> Self {
        Self { canonical_only }
    }
}

impl<S> Layer<S> for TrailingSlashLayer {
    type Service = TrailingSlash<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TrailingSlash {
            inner,
            canonical_only: self.canonical_only,
        }
    }
}

#[derive(Clone)]
pub(super) struct TrailingSlash<S> {
    inner: S,
    canonical_only: &'static [&'static str],
}

impl<S, B> Service<Request<B>> for TrailingSlash<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let path = request.uri().path();
        if let Some(canonical) = path.strip_suffix('/')
            && !canonical.is_empty()
            && !canonical.ends_with('/')
            && !path.starts_with("//")
            && !self.canonical_only.contains(&canonical)
        {
            trim_one_slash(request.uri_mut());
        }
        self.inner.call(request)
    }
}

#[cold]
fn trim_one_slash(uri: &mut Uri) {
    let path_end = uri.path().len();
    let original = uri
        .path_and_query()
        .expect("a URI with a trailing path slash has a path")
        .as_str();
    let mut normalized = String::with_capacity(original.len() - 1);
    normalized.push_str(&original[..path_end - 1]);
    normalized.push_str(&original[path_end..]);
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(
        normalized
            .try_into()
            .expect("removing one path slash preserves URI validity"),
    );
    *uri = Uri::from_parts(parts).expect("scheme and authority are unchanged");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use tower::{ServiceExt, service_fn};

    #[tokio::test]
    async fn single_alias_preserves_query_authority_method_body_and_extensions() {
        let inner =
            service_fn(|request: Request<String>| async move { Ok::<_, Infallible>(request) });
        let app = TrailingSlashLayer::new(&["/json"]).layer(inner);
        let mut request = Request::builder()
            .method("PUT")
            .uri("https://example.test/json/new/?url=https://example.test/a//&x=%2F")
            .header("x-probe", "kept")
            .body(String::from("body/unchanged"))
            .unwrap();
        request.extensions_mut().insert(42_u32);
        let result = app.oneshot(request).await.unwrap();
        assert_eq!(
            result.uri().to_string(),
            "https://example.test/json/new?url=https://example.test/a//&x=%2F"
        );
        assert_eq!(result.method(), "PUT");
        assert_eq!(result.headers()["x-probe"], "kept");
        assert_eq!(result.extensions().get::<u32>(), Some(&42));
        assert_eq!(result.body(), "body/unchanged");
    }

    #[tokio::test]
    async fn canonical_root_and_multiple_slash_paths_remain_exact() {
        for original in [
            "/",
            "//",
            "/json",
            "/json/",
            "/json/?q=%2F",
            "/json/version",
            "/json/version//",
            "//json/version/",
            "///json/version///",
            "/session//",
            "/session/a/url?x=/",
            "*",
        ] {
            let inner = service_fn(|request: Request<()>| async move {
                Ok::<_, Infallible>(request.uri().clone())
            });
            let app = TrailingSlashLayer::new(&["/json"]).layer(inner);
            let request = Request::builder().uri(original).body(()).unwrap();
            assert_eq!(app.oneshot(request).await.unwrap().to_string(), original);
        }
    }

    #[tokio::test]
    async fn encoded_path_and_empty_query_survive_single_slash_removal() {
        for (original, expected) in [
            ("/session/x/cookie/name%2F/", "/session/x/cookie/name%2F"),
            ("/json/version/?", "/json/version?"),
            ("/unknown/", "/unknown"),
        ] {
            let inner = service_fn(|request: Request<()>| async move {
                Ok::<_, Infallible>(request.uri().clone())
            });
            let app = TrailingSlashLayer::new(&["/json"]).layer(inner);
            let request = Request::builder().uri(original).body(()).unwrap();
            assert_eq!(app.oneshot(request).await.unwrap().to_string(), expected);
        }
    }

    #[tokio::test]
    async fn inner_service_errors_propagate() {
        let inner = service_fn(|_: Request<()>| async { Err::<(), _>("expected error") });
        let app = TrailingSlashLayer::new(&[]).layer(inner);
        let request = Request::builder().uri("/json/version/").body(()).unwrap();
        assert_eq!(app.oneshot(request).await, Err("expected error"));
    }
}

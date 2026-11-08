use super::*;

#[derive(Clone, Copy, Debug)]
enum CacheTransport {
    Html,
    Raw,
}

impl CacheTransport {
    async fn fetch(self, config: &FetchConfig, request: Request) -> Result<Response> {
        // Reopen the client on every request to exercise the shared disk cache.
        let client = FetchClient::new(config, new_shared_browser_cookie_store());
        let response = match self {
            Self::Html => {
                client
                    .fetch_html_stream(request)
                    .await?
                    .into_materialized_text_response()
                    .await
            }
            Self::Raw => client.fetch(request).await,
        };
        assert!(client.shutdown().is_clean());
        response
    }
}

fn cacheable_response(body: &str) -> ScriptedResponse {
    ScriptedResponse::ok(body).with_header("Cache-Control", "max-age=600")
}

#[tokio::test]
async fn literal_cookie_cache_ignores_unbound_and_out_of_scope_configuration() -> Result<()> {
    for transport in [CacheTransport::Html, CacheTransport::Raw] {
        for bound in [false, true] {
            let cache_dir = unique_test_cache_dir();
            let server = ScriptedHttpServer::spawn(vec![cacheable_response("public"); 2]);
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            config.set_additional_cookies(Some("session=literal".to_owned()));
            if bound {
                config.bind_additional_cookie_scope(&Request::get("https://app.test/start")?);
            }
            let request = Request::new("GET", &server.url(), None, Vec::new())?
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::Include);

            for from_cache in [false, true] {
                let response = transport.fetch(&config, request.clone()).await?;
                assert_eq!(
                    response.body_text(),
                    "public",
                    "{transport:?}, bound={bound}"
                );
                assert_eq!(
                    response.from_cache, from_cache,
                    "{transport:?}, bound={bound}"
                );
            }
            assert_eq!(server.hits(), 1, "{transport:?}, bound={bound}");
            assert!(
                !server.requests()[0]
                    .to_ascii_lowercase()
                    .contains("\r\ncookie:")
            );
            server.shutdown();
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn literal_cookie_cache_obeys_browser_credentials() -> Result<()> {
    for transport in [CacheTransport::Html, CacheTransport::Raw] {
        for in_config in [false, true] {
            for credentials in [
                RequestCredentialsMode::Omit,
                RequestCredentialsMode::SameOrigin,
            ] {
                let cache_dir = unique_test_cache_dir();
                let server = ScriptedHttpServer::spawn(vec![cacheable_response("public"); 2]);
                let mut config = FetchConfig::default();
                config.set_http_cache_dir(Some(cache_dir.display().to_string()));
                let mut request = Request::new_browser(
                    "GET",
                    Url::parse(&server.url())?,
                    None,
                    Vec::new(),
                    moli_url::WebOrigin::from_url(&Url::parse("https://other.test/page")?),
                )
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(credentials);
                if in_config {
                    config.set_additional_cookies(Some("session=literal".to_owned()));
                    config.bind_additional_cookie_scope(&Request::get(&server.url())?);
                } else {
                    request = request.with_additional_cookies(Some("session=literal".to_owned()));
                }

                for from_cache in [false, true] {
                    let response = transport.fetch(&config, request.clone()).await?;
                    assert_eq!(response.body_text(), "public");
                    assert_eq!(
                        response.from_cache, from_cache,
                        "{transport:?}, in_config={in_config}, {credentials:?}"
                    );
                }
                assert_eq!(server.hits(), 1);
                assert!(
                    !server.requests()[0]
                        .to_ascii_lowercase()
                        .contains("\r\ncookie:")
                );
                server.shutdown();
                fs::remove_dir_all(cache_dir)?;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn literal_cookie_cache_keeps_credentialed_responses_private() -> Result<()> {
    for transport in [CacheTransport::Html, CacheTransport::Raw] {
        for in_config in [false, true] {
            let cache_dir = unique_test_cache_dir();
            let server = ScriptedHttpServer::spawn(vec![
                cacheable_response("public"),
                cacheable_response("private one"),
                cacheable_response("private two"),
            ]);
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let public = Request::new("GET", &server.url(), None, Vec::new())?
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::Include);
            let response = transport.fetch(&config, public.clone()).await?;
            assert_eq!(response.body_text(), "public");
            assert!(!response.from_cache);

            let mut private = public.clone();
            if in_config {
                config.set_additional_cookies(Some("session=literal".to_owned()));
                config.bind_additional_cookie_scope(&Request::get(&server.url())?);
            } else {
                private = private.with_additional_cookies(Some("session=literal".to_owned()));
            }
            for body in ["private one", "private two"] {
                let response = transport.fetch(&config, private.clone()).await?;
                assert_eq!(
                    response.body_text(),
                    body,
                    "{transport:?}, in_config={in_config}"
                );
                assert!(!response.from_cache);
            }

            config.set_additional_cookies(None);
            let response = transport.fetch(&config, public).await?;
            assert_eq!(response.body_text(), "public");
            assert!(response.from_cache);
            assert_eq!(server.hits(), 3);
            for (index, request) in server.requests().iter().enumerate() {
                assert_eq!(
                    request
                        .to_ascii_lowercase()
                        .contains("\r\ncookie: session=literal\r\n"),
                    index != 0,
                    "{transport:?}, in_config={in_config}: {request}"
                );
            }
            server.shutdown();
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[test]
fn request_cache_control_headers_revalidate_fresh_cached_responses() {
    for (name, value) in [
        ("Cache-Control", "max-age=0"),
        ("cache-control", "no-cache"),
        ("Pragma", "no-cache"),
    ] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("cached body")
                .with_header("Cache-Control", "max-age=60")
                .with_header("ETag", "\"version-1\""),
            ScriptedResponse::status(304, "Not Modified")
                .with_header("Cache-Control", "max-age=600")
                .with_header("ETag", "\"version-1\""),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let first =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(first.body_text(), "cached body");
        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert!(cached.from_cache);
        assert_eq!(server.hits(), 1);

        let mut request = Request::get(&server.url()).unwrap();
        request.request_headers = vec![(name.to_owned(), value.to_owned())].into();
        let validated = fetch_with_config_for_test(&config, request).unwrap();
        assert_eq!(
            validated.status, 200,
            "{name}: {value} must merge the cached 200 response with its 304 validation"
        );
        assert_eq!(validated.body_text(), "cached body");
        assert!(validated.from_cache);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("if-none-match: \"version-1\""),
            "cached validators missing from {}",
            requests[1]
        );
        assert!(requests[1].to_ascii_lowercase().contains(&format!(
            "{}: {}",
            name.to_ascii_lowercase(),
            value
        )));

        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(cached.body_text(), "cached body");
        assert!(cached.from_cache);
        assert_eq!(server.hits(), 2);
        server.shutdown();
        fs::remove_dir_all(cache_dir).unwrap();
    }
}

#[test]
fn request_no_store_does_not_reuse_or_replace_an_existing_cached_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("original").with_header("Cache-Control", "max-age=600"),
        ScriptedResponse::ok("private one").with_header("Cache-Control", "max-age=600"),
        ScriptedResponse::ok("private two").with_header("Cache-Control", "max-age=600"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let original =
        fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    assert_eq!(original.body_text(), "original");
    for body in ["private one", "private two"] {
        let mut request = Request::get(&server.url()).unwrap();
        request.request_headers =
            vec![("Cache-Control".to_owned(), "max-age=0, No-Store".to_owned())].into();
        let response = fetch_with_config_for_test(&config, request).unwrap();
        assert_eq!(response.body_text(), body);
        assert!(!response.from_cache);
        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(cached.body_text(), "original");
        assert!(cached.from_cache);
    }
    assert_eq!(server.hits(), 3);
    server.shutdown();
    fs::remove_dir_all(cache_dir).unwrap();
}

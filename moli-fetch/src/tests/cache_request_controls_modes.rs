use super::*;

#[derive(Clone, Copy, Debug)]
enum Transport {
    Raw,
    Html,
}

impl Transport {
    async fn fetch(self, config: &FetchConfig, request: Request) -> Result<Response> {
        let client = FetchClient::new(config, new_shared_browser_cookie_store());
        let result = match self {
            Self::Raw => client.fetch(request).await,
            Self::Html => match client.fetch_html_stream(request).await {
                Ok(response) => response.into_materialized_text_response().await,
                Err(error) => Err(error),
            },
        };
        assert!(client.shutdown().is_clean());
        result
    }
}

fn cached(body: &str, control: &str) -> ScriptedResponse {
    ScriptedResponse::ok(body)
        .with_header("Cache-Control", control)
        .with_header("ETag", "\"version\"")
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(field, value)| field.eq_ignore_ascii_case(name).then_some(value.trim()))
}

#[tokio::test]
async fn cache_modes_emit_wire_controls_and_preserve_author_fields() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        for (label, control, pragma) in [
            ("default", None, None),
            ("no-cache", Some("max-age=0"), None),
            ("reload", Some("no-cache"), Some("no-cache")),
            ("no-store", Some("no-cache"), Some("no-cache")),
        ] {
            for explicit in [None, Some("author"), Some("")] {
                let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("wire")]);
                let mut request = Request::get(&server.url())?.with_cache_mode(label.parse()?);
                if let Some(value) = explicit {
                    request.request_headers = vec![
                        ("cAcHe-CoNtRoL".to_owned(), value.to_owned()),
                        ("pRaGmA".to_owned(), value.to_owned()),
                    ]
                    .into();
                }
                let response = transport.fetch(&FetchConfig::default(), request).await?;
                assert_eq!(response.body_bytes(), b"wire");
                let requests = server.requests();
                assert_eq!(requests.len(), 1);
                assert_eq!(header(&requests[0], "cache-control"), explicit.or(control));
                assert_eq!(header(&requests[0], "pragma"), explicit.or(pragma));
                server.shutdown();
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn conditional_fetch_headers_bypass_cache_without_replacing_stored_response() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        for field in [
            "If-Modified-Since",
            "If-None-Match",
            "If-Unmodified-Since",
            "If-Match",
            "If-Range",
        ] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let server = ScriptedHttpServer::spawn(vec![
                cached("stored", "max-age=600"),
                cached("conditional", "max-age=600"),
            ]);
            let request = Request::get(&server.url())?;
            transport.fetch(&config, request.clone()).await?;
            let mut conditional = request.clone();
            conditional.request_headers = vec![(field.to_owned(), "author".to_owned())].into();
            let mode = RequestCacheMode::Default.for_fetch_headers(&conditional.request_headers);
            let response = transport
                .fetch(&config, conditional.with_cache_mode(mode))
                .await?;
            assert!(!response.from_cache, "{transport:?}, {field}");
            assert_eq!(response.body_bytes(), b"conditional");
            let retained = transport.fetch(&config, request).await?;
            assert!(retained.from_cache);
            assert_eq!(retained.body_bytes(), b"stored");
            let requests = server.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(header(&requests[1], field), Some("author"));
            assert_eq!(header(&requests[1], "cache-control"), Some("no-cache"));
            assert_eq!(header(&requests[1], "pragma"), Some("no-cache"));
            server.shutdown();
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn reload_replaces_cached_data_while_no_store_preserves_it() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let server = ScriptedHttpServer::spawn(vec![
            cached("first", "max-age=600"),
            cached("reloaded", "max-age=600"),
            cached("private", "max-age=600"),
        ]);
        let request = Request::get(&server.url())?;
        transport.fetch(&config, request.clone()).await?;
        let reloaded = transport
            .fetch(
                &config,
                request.clone().with_cache_mode(RequestCacheMode::Bypass),
            )
            .await?;
        assert!(!reloaded.from_cache);
        assert_eq!(reloaded.body_bytes(), b"reloaded");
        let hit = transport.fetch(&config, request.clone()).await?;
        assert!(hit.from_cache);
        assert_eq!(hit.body_bytes(), b"reloaded");
        let private = transport
            .fetch(
                &config,
                request.clone().with_cache_mode(RequestCacheMode::NoStore),
            )
            .await?;
        assert!(!private.from_cache);
        assert_eq!(private.body_bytes(), b"private");
        let retained = transport.fetch(&config, request).await?;
        assert!(retained.from_cache);
        assert_eq!(retained.body_bytes(), b"reloaded");
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        for wire in &requests[1..] {
            assert_eq!(header(wire, "if-none-match"), None);
            assert_eq!(header(wire, "if-modified-since"), None);
        }
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

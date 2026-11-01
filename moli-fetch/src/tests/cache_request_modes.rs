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
            ("force-cache", None, None),
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
async fn force_cache_and_cache_only_reuse_stale_entries_without_validation() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        for control in ["max-age=0, must-revalidate", "no-cache"] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let server = ScriptedHttpServer::spawn(vec![
                cached("stored", control),
                cached("must not be requested", "max-age=600"),
            ]);
            let request = Request::get(&server.url())?;
            let response = transport.fetch(&config, request.clone()).await?;
            assert!(!response.from_cache);
            for mode in [RequestCacheMode::ForceCache, RequestCacheMode::OnlyIfCached] {
                let response = transport
                    .fetch(&config, request.clone().with_cache_mode(mode))
                    .await?;
                assert!(response.from_cache, "{transport:?}: {mode:?}, {control}");
                assert_eq!(response.body_bytes(), b"stored");
            }
            assert_eq!(server.hits(), 1);
            server.shutdown();
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn cache_only_mode_misses_reject_without_network_on_every_transport() -> Result<()> {
    for enabled in [false, true] {
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        if enabled {
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        }
        let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("forbidden")]);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        for path in 0..4 {
            let request =
                Request::get(&server.url())?.with_cache_mode(RequestCacheMode::OnlyIfCached);
            let error = match path {
                0 => client.fetch_raw(request).await.unwrap_err(),
                1 => client.fetch(request.with_single_hop()).await.unwrap_err(),
                2 => client.fetch_html_stream(request).await.err().unwrap(),
                _ => client
                    .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                    .await
                    .err()
                    .unwrap(),
            };
            assert!(
                error.is::<crate::FetchCacheMiss>(),
                "{path}, enabled={enabled}: {error:#}"
            );
            assert_eq!(
                crate::error::browser_network_error_text(&error),
                "net::ERR_CACHE_MISS"
            );
        }
        assert!(client.shutdown().is_clean());
        assert_eq!(server.hits(), 0);
        assert!(server.requests().is_empty());
        server.shutdown();
        if cache_dir.exists() {
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

#[tokio::test]
async fn cache_only_mode_keeps_vary_partition_and_cancellation_checks() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let server = ScriptedHttpServer::spawn(vec![
            cached("english", "max-age=600").with_header("Vary", "Accept-Language"),
        ]);
        let mut request = Request::get(&server.url())?;
        request.request_headers = vec![("Accept-Language".to_owned(), "en".to_owned())].into();
        transport.fetch(&config, request.clone()).await?;
        let cache_only = request.with_cache_mode(RequestCacheMode::OnlyIfCached);
        let mut varied = cache_only.clone();
        varied.request_headers = vec![("Accept-Language".to_owned(), "fr".to_owned())].into();
        let error = transport.fetch(&config, varied).await.unwrap_err();
        assert!(error.is::<crate::FetchCacheMiss>());
        let error = transport
            .fetch(
                &config,
                cache_only
                    .clone()
                    .with_network_partition_key(Some("other".to_owned())),
            )
            .await
            .unwrap_err();
        assert!(error.is::<crate::FetchCacheMiss>());
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let cancel = FetchCancelHandle::new();
        cancel.cancel();
        let error = client
            .fetch_raw_with_cancel(cache_only, cancel)
            .await
            .unwrap_err();
        assert!(error.is::<crate::FetchCancelled>(), "{error:#}");
        assert!(client.shutdown().is_clean());
        assert_eq!(server.hits(), 1);
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn cache_only_mode_follows_cached_redirects_with_same_origin_checks() -> Result<()> {
    for transport in [Transport::Raw, Transport::Html] {
        for cross_origin in [false, true] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let target = ScriptedHttpServer::spawn(vec![cached("target", "max-age=600")]);
            let location = if cross_origin {
                target.url()
            } else {
                "/final".to_owned()
            };
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &location)
                    .with_header("Cache-Control", "max-age=600"),
                cached("target", "max-age=600"),
            ]);
            let url = Url::parse(&source.url())?;
            let request = Request::new_browser(
                "GET",
                url.clone(),
                None,
                Vec::new(),
                moli_url::WebOrigin::from_url(&url),
            )
            .with_request_mode(RequestMode::NoCors)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
            let seed = transport.fetch(&config, request.clone()).await?;
            assert_eq!(seed.body_bytes(), b"target");
            let hits = (source.hits(), target.hits());
            let result = transport
                .fetch(
                    &config,
                    request
                        .with_request_mode(RequestMode::SameOrigin)
                        .with_cache_mode(RequestCacheMode::OnlyIfCached),
                )
                .await;
            if cross_origin {
                assert!(
                    result.is_err(),
                    "cached redirects must retain same-origin checks"
                );
            } else {
                let response = result?;
                assert!(response.from_cache);
                assert!(response.redirected);
                assert_eq!(response.body_bytes(), b"target");
            }
            assert_eq!((source.hits(), target.hits()), hits);
            source.shutdown();
            target.shutdown();
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

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

use super::*;

fn controlled(url: &str, control: &str) -> Result<Request> {
    let mut request = request(url)?;
    request.request_headers = vec![("Cache-Control".to_owned(), control.to_owned())].into();
    Ok(request)
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_max_stale_uses_age_and_cache_only_misses_keep_the_body() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[("Cache-Control", "max-age=1500"), ("Age", "2000")],
            b"stale body",
        ),
        reply(200, &[("Cache-Control", "max-age=600")], b"fresh body"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, true).await?;
    for control in [
        "only-if-cached",
        "only-if-cached, max-stale=499",
        "only-if-cached, max-stale, no-store",
    ] {
        let (miss, journal, cancel) = fetch(&client, controlled(&url, control)?, true).await?;
        assert_eq!(miss.status, 504, "{control}");
        assert!(miss.body_bytes().is_empty());
        assert_eq!(miss.cache_state, ResponseCacheState::Local);
        assert!(journal.is_empty());
        assert!(cancel.response_timing().request_started_at().is_none());
        assert!(miss.negotiated_http_version.is_none());
    }
    for control in ["only-if-cached, max-stale=1000", "max-stale"] {
        let (hit, journal, _) = fetch(&client, controlled(&url, control)?, true).await?;
        assert_eq!(hit.status, 200);
        assert_eq!(hit.body_bytes(), b"stale body");
        assert_eq!(hit.cache_state, ResponseCacheState::Local);
        assert!(journal.is_empty());
    }
    assert_eq!(
        fetch(&client, request(&url)?, false).await?.0.body_bytes(),
        b"fresh body"
    );
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_only_misses_preserve_all_matching_vary_variants() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=1500"),
                ("Age", "2000"),
                ("Vary", "Foo"),
            ],
            b"one",
        ),
        reply(
            200,
            &[
                ("Cache-Control", "max-age=1500"),
                ("Age", "2000"),
                ("Vary", "Foo"),
            ],
            b"two",
        ),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let variant = |value: &str, control: &str| -> Result<Request> {
        let mut request = controlled(&url, control)?;
        request
            .request_headers
            .push(("Foo".into(), value.as_bytes().to_vec()));
        Ok(request)
    };
    for value in ["one", "two"] {
        assert_eq!(
            fetch(&client, variant(value, "")?, false)
                .await?
                .0
                .body_bytes(),
            value.as_bytes()
        );
    }
    for value in ["one", "two"] {
        assert_eq!(
            fetch(&client, variant(value, "only-if-cached")?, true)
                .await?
                .0
                .status,
            504
        );
    }
    for value in ["two", "one"] {
        let (hit, journal, _) = fetch(
            &client,
            variant(value, "max-stale=1000, only-if-cached")?,
            true,
        )
        .await?;
        assert_eq!(hit.body_bytes(), value.as_bytes());
        assert_eq!(hit.cache_state, ResponseCacheState::Local);
        assert!(journal.is_empty());
    }
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_max_stale_obeys_response_validation_requirements() -> Result<()> {
    for policy in ["no-cache, max-age=600", "must-revalidate, max-age=0"] {
        let (url, server) = server(vec![
            reply(
                200,
                &[("Cache-Control", policy), ("ETag", "\"version\"")],
                b"original",
            ),
            reply(304, &[("Cache-Control", "max-age=600")], b""),
        ])
        .await?;
        let client = ResourceRequestClient::new(&FetchConfig::default())?;
        fetch(&client, request(&url)?, false).await?;
        let (miss, journal, _) = fetch(
            &client,
            controlled(&url, "only-if-cached, max-stale")?,
            true,
        )
        .await?;
        assert_eq!(miss.status, 504, "{policy}");
        assert!(journal.is_empty());
        let (validated, journal, _) = fetch(&client, controlled(&url, "max-stale")?, true).await?;
        assert_eq!(validated.body_bytes(), b"original");
        assert_eq!(validated.cache_state, ResponseCacheState::Validated);
        assert_eq!(journal.final_response_observation().unwrap().status(), 304);
        let requests = server.await??;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            header(&requests[1], "if-none-match"),
            Some(b"\"version\"".as_slice())
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_request_constraints_validate_before_reusing_fresh_immutable_entries()
-> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600, immutable"),
                ("Age", "200"),
                ("ETag", "\"original\""),
            ],
            b"original",
        ),
        reply(
            304,
            &[("Cache-Control", "max-age=600, immutable"), ("Age", "0")],
            b"",
        ),
        reply(
            304,
            &[("Cache-Control", "max-age=600, immutable"), ("Age", "0")],
            b"",
        ),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, false).await?;
    assert_eq!(
        fetch(&client, controlled(&url, "max-age=0")?, false)
            .await?
            .0
            .cache_state,
        ResponseCacheState::Local
    );
    for request in [
        controlled(&url, "only-if-cached, no-cache")?,
        controlled(&url, "only-if-cached")?.with_cache_mode(RequestCacheMode::Validate),
        controlled(&url, "only-if-cached")?.with_cache_mode(RequestCacheMode::Bypass),
    ] {
        let (miss, journal, _) = fetch(&client, request, true).await?;
        assert_eq!(miss.status, 504);
        assert!(journal.is_empty());
    }
    for control in ["max-age=100", "min-fresh=1000"] {
        let validated = fetch(&client, controlled(&url, control)?, true).await?.0;
        assert_eq!(validated.body_bytes(), b"original");
        assert_eq!(validated.cache_state, ResponseCacheState::Validated);
    }
    assert_eq!(
        fetch(&client, controlled(&url, "only-if-cached")?, false)
            .await?
            .0
            .cache_state,
        ResponseCacheState::Local
    );
    let requests = server.await??;
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1..]
            .iter()
            .all(|request| header(request, "if-none-match") == Some(b"\"original\"".as_slice()))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_only_redirects_stop_at_uncached_targets() -> Result<()> {
    let (url, server) = server(vec![reply(
        302,
        &[("Location", "/missing"), ("Cache-Control", "max-age=600")],
        b"",
    )])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let origin = Url::parse(&url)?;
    let request = || {
        Request::new_browser("GET", origin.clone(), None, Vec::new(), (&origin).into())
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
            .with_page_network_policy()
    };
    // Prime the complete redirect representation independently. The browser
    // redirect loop may drop an unread body before its cache tee completes.
    assert_eq!(
        fetch(&client, request().with_single_hop(), false)
            .await?
            .0
            .status,
        302
    );
    let mut request = request();
    request.request_headers =
        vec![("Cache-Control".to_owned(), "only-if-cached".to_owned())].into();
    let observed = crate::network_host::fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(&client, request, None, Vec::new()).await.map_err(anyhow::Error::msg)?;
    let (stream, journal) = observed.into_parts_with_observation_journal();
    let response = stream.into_materialized_raw_response().await?;
    assert_eq!(response.status, 504);
    assert_eq!(response.final_url.path(), "/missing");
    assert_eq!(response.redirect_chain.len(), 1);
    assert!(response.redirected);
    assert!(journal.is_empty());
    assert_eq!(server.await??.len(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_only_miss_can_still_use_the_native_disk_cache() -> Result<()> {
    let (url, server) = server(vec![reply(
        200,
        &[("Cache-Control", "max-age=600")],
        b"disk body",
    )])
    .await?;
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-renderer-cache-only-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let native =
        moli_fetch::FetchClient::new(&config, moli_cookie_jar::new_shared_browser_cookie_store());
    assert_eq!(
        native.fetch_raw(request(&url)?).await?.body_bytes(),
        b"disk body"
    );
    native.shutdown();
    let client = ResourceRequestClient::new(&config)?;
    for _ in 0..2 {
        let (hit, journal, cancel) =
            fetch(&client, controlled(&url, "only-if-cached")?, true).await?;
        assert_eq!(hit.status, 200);
        assert_eq!(hit.body_bytes(), b"disk body");
        assert_eq!(hit.cache_state, ResponseCacheState::Local);
        assert!(journal.is_empty());
        assert!(cancel.response_timing().request_started_at().is_none());
    }
    assert_eq!(server.await??.len(), 1);
    drop(client);
    std::fs::remove_dir_all(cache_dir)?;
    Ok(())
}

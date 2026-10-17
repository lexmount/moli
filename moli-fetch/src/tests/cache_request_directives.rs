use super::*;
use crate::ResponseCacheState;

fn controlled(url: &str, value: &str) -> Result<Request> {
    let mut request = Request::get(url)?;
    request.request_headers = vec![("Cache-Control".to_owned(), value.to_owned())].into();
    Ok(request)
}

#[tokio::test]
async fn cache_only_requests_preserve_cancellation_before_local_completion() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("must not be requested")]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for follow_redirects in [true, false] {
        let cancel = FetchCancelHandle::new();
        cancel.cancel();
        let request =
            controlled(&server.url(), "only-if-cached")?.with_follow_redirects(follow_redirects);
        let error = client
            .fetch_with_cancel(request, cancel)
            .await
            .expect_err("an already cancelled cache-only request must not complete with 504");
        assert!(error.is::<crate::FetchCancelled>(), "{error:#}");
    }
    client.shutdown();
    assert_eq!(server.hits(), 0);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn cache_only_misses_never_contact_origin_on_raw_text_html_or_buffered_paths() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("must not be requested")]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for path in 0..4 {
        let cancel = FetchCancelHandle::new();
        let request = controlled(&server.url(), "only-if-cached")?;
        let head = match path {
            0 => {
                let observed = client
                    .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel.clone())
                    .await?;
                let (stream, journal) = observed.into_parts_with_observation_journal();
                assert!(journal.is_empty());
                assert!(stream.network_request_extra_info().is_none());
                let response = stream.into_materialized_raw_response().await?;
                assert!(response.body_bytes().is_empty());
                assert!(cancel.response_timing().request_started_at().is_none());
                assert!(cancel.response_timing().response_started_at().is_none());
                assert!(cancel.response_timing().response_end_at().is_some());
                response.head()
            }
            1 | 2 => {
                let request = request.with_follow_redirects(path == 1);
                let observed = client
                    .fetch_with_cancel_and_network_metadata(request, cancel.clone())
                    .await?;
                let (response, journal) = observed.into_parts_with_observation_journal();
                assert!(journal.is_empty());
                assert!(response.body_bytes().is_empty());
                assert!(cancel.response_timing().request_started_at().is_none());
                response.head()
            }
            _ => {
                let response = client
                    .fetch_html_stream(request)
                    .await?
                    .into_materialized_text_response()
                    .await?;
                assert!(response.body_bytes().is_empty());
                response.head()
            }
        };
        assert_eq!(head.status, 504, "path {path}");
        assert_eq!(head.status_text.as_deref(), Some("Gateway Timeout"));
        assert!(!head.from_cache);
        assert_eq!(head.cache_state, ResponseCacheState::Local);
        assert!(head.negotiated_http_version.is_none());
    }
    client.shutdown();
    assert_eq!(server.hits(), 0);
    assert!(server.requests().is_empty());
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn disk_cache_staleness_bounds_and_cache_only_misses_preserve_stored_entries() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("stale")
            .with_header("Cache-Control", "max-age=1500")
            .with_header("Age", "2000"),
        ScriptedResponse::ok("fresh").with_header("Cache-Control", "max-age=600"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    assert_eq!(
        client
            .fetch_raw(Request::get(&server.url())?)
            .await?
            .body_bytes(),
        b"stale"
    );
    for control in [
        "only-if-cached",
        "only-if-cached, max-stale=499",
        "only-if-cached, max-stale=1000, no-store",
    ] {
        let response = client
            .fetch_raw(controlled(&server.url(), control)?)
            .await?;
        assert_eq!(response.status, 504, "{control}");
        assert!(response.body_bytes().is_empty());
    }
    for control in ["max-stale=1000, only-if-cached", "max-stale"] {
        let response = client
            .fetch_raw(controlled(&server.url(), control)?)
            .await?;
        assert_eq!(response.status, 200);
        assert_eq!(response.body_bytes(), b"stale");
        assert_eq!(response.cache_state, ResponseCacheState::Local);
    }
    assert_eq!(server.hits(), 1);
    assert_eq!(
        client
            .fetch_raw(Request::get(&server.url())?)
            .await?
            .body_bytes(),
        b"fresh"
    );
    let hit = client
        .fetch_raw(controlled(&server.url(), "only-if-cached")?)
        .await?;
    assert_eq!(hit.body_bytes(), b"fresh");
    assert_eq!(hit.cache_state, ResponseCacheState::Local);
    client.shutdown();
    assert_eq!(server.hits(), 2);
    server.shutdown();
    fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn disk_cache_request_age_and_freshness_constraints_validate_with_original_body() -> Result<()>
{
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("original")
            .with_header("Cache-Control", "max-age=600")
            .with_header("Age", "200")
            .with_header("ETag", "\"original\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=600")
            .with_header("Age", "0"),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=600")
            .with_header("Age", "0"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    client.fetch_raw(Request::get(&server.url())?).await?;
    for control in ["max-age=100", "min-fresh=1000"] {
        let response = client
            .fetch_raw(controlled(&server.url(), control)?)
            .await?;
        assert_eq!(response.body_bytes(), b"original");
        assert_eq!(response.cache_state, ResponseCacheState::Validated);
    }
    let hit = client
        .fetch_raw(controlled(
            &server.url(),
            "only-if-cached, max-age=600, min-fresh=500",
        )?)
        .await?;
    assert_eq!(hit.body_bytes(), b"original");
    assert_eq!(hit.cache_state, ResponseCacheState::Local);
    client.shutdown();
    assert_eq!(server.hits(), 3);
    assert!(server.requests()[1..].iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains("if-none-match: \"original\"")
    }));
    server.shutdown();
    fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn disk_cache_max_stale_cannot_override_response_validation_requirements() -> Result<()> {
    for control in ["no-cache, max-age=600", "must-revalidate, max-age=0"] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("original")
                .with_header("Cache-Control", control)
                .with_header("ETag", "\"original\""),
            ScriptedResponse::status(304, "Not Modified")
                .with_header("Cache-Control", "max-age=600"),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        client.fetch_raw(Request::get(&server.url())?).await?;
        let miss = client
            .fetch_raw(controlled(&server.url(), "only-if-cached, max-stale")?)
            .await?;
        assert_eq!(miss.status, 504, "{control}");
        assert_eq!(server.hits(), 1);
        let validated = client
            .fetch_raw(controlled(&server.url(), "max-stale")?)
            .await?;
        assert_eq!(validated.body_bytes(), b"original");
        assert_eq!(validated.cache_state, ResponseCacheState::Validated);
        client.shutdown();
        assert_eq!(server.hits(), 2);
        assert!(
            server.requests()[1]
                .to_ascii_lowercase()
                .contains("if-none-match: \"original\"")
        );
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn disk_cache_only_redirects_stop_at_uncached_hops_without_network() -> Result<()> {
    for html in [false, true] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found")
                .with_header("Location", "/missing")
                .with_header("Cache-Control", "max-age=600"),
            ScriptedResponse::ok("must not be requested"),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let prime = Request::get(&server.url())?.with_follow_redirects(false);
        assert_eq!(client.fetch_raw(prime).await?.status, 302);
        let request = controlled(&server.url(), "only-if-cached")?;
        let head = if html {
            client
                .fetch_html_stream(request)
                .await?
                .into_materialized_text_response()
                .await?
                .head()
        } else {
            let observed = client.fetch_raw_with_network_metadata(request).await?;
            let (response, journal) = observed.into_parts_with_observation_journal();
            assert!(journal.is_empty());
            response.head()
        };
        assert_eq!(head.status, 504);
        assert_eq!(head.final_url.path(), "/missing");
        assert!(head.redirected);
        assert_eq!(head.redirect_chain.len(), 1);
        assert!(head.redirect_chain[0].from_cache);
        client.shutdown();
        assert_eq!(server.hits(), 1);
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

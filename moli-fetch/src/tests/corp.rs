use super::request_security::Transport;
use super::*;

fn browser_request(url: &str) -> Result<Request> {
    Ok(Request::get(url)?
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
            "https://initiator.test/",
        )?))
        .with_request_mode(RequestMode::NoCors))
}

#[tokio::test]
async fn forbidden_corp_redirect_never_requests_its_target_in_any_transport() -> Result<()> {
    for transport in Transport::ALL {
        for handling in [
            RequestRedirectHandling::Follow,
            RequestRedirectHandling::Manual,
            RequestRedirectHandling::Error,
            RequestRedirectHandling::FollowByCaller,
        ] {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &target.url())
                    .with_header("Cross-Origin-Resource-Policy", "same-origin"),
            ]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let error = transport
                .fetch(
                    &client,
                    browser_request(&source.url())?.with_redirect_handling(handling),
                )
                .await
                .expect_err("a forbidden redirect must be rejected before its target is fetched");
            assert!(
                error.to_string().contains("Cross-Origin-Resource-Policy"),
                "{transport:?}, {handling:?}: {error:#}"
            );
            assert_eq!(source.hits(), 1, "{transport:?}, {handling:?}");
            assert_eq!(target.hits(), 0, "{transport:?}, {handling:?}");
            source.shutdown();
            target.shutdown();
        }
    }
    Ok(())
}

#[tokio::test]
async fn raw_stream_rejects_corp_before_exposing_final_or_manual_redirect_headers() -> Result<()> {
    for status in [200, 302, 304] {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(status, "Response")
                .with_header("Location", "/never")
                .with_header("Cross-Origin-Resource-Policy", "same-origin"),
        ]);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let error = client
            .fetch_raw_stream_with_cancel(
                browser_request(&server.url())?.with_redirect_mode(RequestRedirectMode::Manual),
                FetchCancelHandle::new(),
            )
            .await
            .err()
            .context("CORP must fail before returning a response head")?;
        assert!(error.to_string().contains("Cross-Origin-Resource-Policy"));
        assert_eq!(server.hits(), 1);
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn corp_is_checked_again_when_reading_cached_responses_and_redirects() -> Result<()> {
    // The buffered auth/compatibility transport does not use the HTTP cache.
    for transport in [Transport::Html, Transport::Raw] {
        for redirect in [false, true] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let response = if redirect {
                ScriptedResponse::status(302, "Found").with_header("Location", "/target")
            } else {
                ScriptedResponse::ok("cached")
            }
            .with_header("Cross-Origin-Resource-Policy", "same-origin")
            .with_header("Cache-Control", "max-age=3600");
            let server = ScriptedHttpServer::spawn(vec![response, ScriptedResponse::ok("target")]);
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            // Standalone HTTP clients have no browser initiator and may cache
            // the response. A browser must still apply its own CORP check.
            let mut warm = client
                .fetch_raw_stream_with_cancel(
                    Request::get(&server.url())?.with_redirect_mode(RequestRedirectMode::Manual),
                    FetchCancelHandle::new(),
                )
                .await?;
            while warm.next_chunk().await.is_some() {}
            warm.finish().await?;
            let error = transport
                .fetch(&client, browser_request(&server.url())?)
                .await
                .expect_err("cached responses retain their CORP policy");
            assert!(
                error.to_string().contains("Cross-Origin-Resource-Policy"),
                "{transport:?}, redirect={redirect}: {error:#}"
            );
            assert_eq!(server.hits(), 1, "{transport:?}, redirect={redirect}");
            server.shutdown();
            drop(client);
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn corp_does_not_block_cors_navigation_or_standalone_http_requests() -> Result<()> {
    for transport in Transport::ALL {
        for mode in [
            RequestMode::Cors,
            RequestMode::Navigate,
            RequestMode::NoCors,
        ] {
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::ok("allowed")
                    .with_header("Cross-Origin-Resource-Policy", "same-origin")
                    .with_header("Access-Control-Allow-Origin", "*"),
            ]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let request = if mode == RequestMode::NoCors {
                Request::get(&server.url())?.with_request_mode(mode)
            } else {
                browser_request(&server.url())?
                    .with_request_mode(mode)
                    .with_credentials_mode(RequestCredentialsMode::Omit)
            };
            transport.fetch(&client, request).await?;
            assert_eq!(server.hits(), 1);
            server.shutdown();
        }
    }
    Ok(())
}

#[tokio::test]
async fn raw_stream_revalidation_checks_merged_corp_before_exposing_cached_body() -> Result<()> {
    for (cached_policy, updated_policy, allowed) in [
        ("same-origin", None, false),
        ("cross-origin", Some("same-origin"), false),
        ("same-origin", Some("cross-origin"), true),
    ] {
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let cached = ScriptedResponse::ok("cached")
            .with_header("Cross-Origin-Resource-Policy", cached_policy)
            .with_header("Cache-Control", "max-age=3600")
            .with_header("ETag", "\"corp\"");
        let mut validation = ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=3600");
        if let Some(policy) = updated_policy {
            validation = validation.with_header("Cross-Origin-Resource-Policy", policy);
        }
        let server = ScriptedHttpServer::spawn(vec![cached, validation]);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let mut warm = client
            .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
            .await?;
        while warm.next_chunk().await.is_some() {}
        warm.finish().await?;

        let result = client
            .fetch_raw_stream_with_cancel(
                browser_request(&server.url())?.with_cache_mode(RequestCacheMode::Validate),
                FetchCancelHandle::new(),
            )
            .await;
        assert_eq!(
            result.is_ok(),
            allowed,
            "cached={cached_policy}, updated={updated_policy:?}"
        );
        match result {
            Ok(mut response) => {
                assert!(response.from_cache);
                let mut body = Vec::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.extend_from_slice(&chunk);
                }
                response.finish().await?;
                assert_eq!(body, b"cached");
            }
            Err(error) => assert!(error.to_string().contains("Cross-Origin-Resource-Policy")),
        }
        assert_eq!(server.hits(), 2);
        assert!(
            server.requests()[1]
                .to_ascii_lowercase()
                .contains("if-none-match: \"corp\"")
        );
        server.shutdown();
        drop(client);
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

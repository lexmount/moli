use super::request_security::Transport;
use super::*;

async fn response_head(
    client: &FetchClientHandle,
    transport: Transport,
    request: Request,
) -> Result<ResponseHead> {
    match transport {
        Transport::Buffered => {
            let (tx, rx) = oneshot::channel();
            client.fetch_with_cancel_callback(request, FetchCancelHandle::new(), move |value| {
                let _ = tx.send(value);
            })?;
            let response = rx.await??;
            let head = response.clone().into_body().0;
            assert_eq!(
                head.request_include_credentials,
                response.request_include_credentials
            );
            Ok(head)
        }
        Transport::Html => {
            let response = client.fetch_html_stream(request).await?;
            let head = response.head();
            let response = response.into_materialized_text_response().await?;
            assert_eq!(
                response.request_include_credentials,
                head.request_include_credentials
            );
            Ok(head)
        }
        Transport::Raw => {
            let response = client
                .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                .await?;
            let head = response.head();
            let response = response.into_materialized_raw_response().await?;
            assert_eq!(
                response.clone().into_body().0.request_include_credentials,
                head.request_include_credentials
            );
            Ok(head)
        }
    }
}

#[tokio::test]
async fn response_credentials_flag_does_not_depend_on_cookie_presence_in_any_transport()
-> Result<()> {
    for transport in Transport::ALL {
        let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("body"); 6]);
        let url = Url::parse(&server.url())?;
        let other = Url::parse("http://other.test/")?;
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        for (origin, mode, expected) in [
            (&url, RequestCredentialsMode::Include, true),
            (&url, RequestCredentialsMode::SameOrigin, true),
            (&url, RequestCredentialsMode::Omit, false),
            (&other, RequestCredentialsMode::Include, true),
            (&other, RequestCredentialsMode::SameOrigin, false),
            (&other, RequestCredentialsMode::Omit, false),
        ] {
            let request = Request::new_browser(
                "GET",
                url.clone(),
                None,
                Vec::new(),
                moli_url::WebOrigin::from_url(origin),
            )
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(mode);
            let head = response_head(&client, transport, request).await?;
            assert!(head.request_cookie_report.is_none());
            assert_eq!(
                head.request_include_credentials, expected,
                "{transport:?}, {mode:?}, {origin}"
            );
        }
        assert!(
            server
                .requests()
                .iter()
                .all(|request| !request.to_ascii_lowercase().contains("\r\ncookie:"))
        );
        assert!(client.shutdown().is_clean());
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn response_credentials_flag_survives_partitioned_http_cache_hits() -> Result<()> {
    for transport in [Transport::Html, Transport::Raw] {
        let cache_dir = unique_test_cache_dir();
        let server =
            ScriptedHttpServer::spawn(vec![
                ScriptedResponse::ok("public")
                    .with_header("Cache-Control", "max-age=600");
                8
            ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let url = Url::parse(&server.url())?;
        let other = Url::parse("http://other.test/")?;
        // Both mode and actual credential admission partition the HTTP cache.
        // Reopening clients must restore each original response flag.
        for (origin, mode, expected, from_cache) in [
            (&other, RequestCredentialsMode::Include, true, false),
            (&other, RequestCredentialsMode::Omit, false, false),
            (&url, RequestCredentialsMode::SameOrigin, true, false),
            (&other, RequestCredentialsMode::SameOrigin, false, false),
            (&url, RequestCredentialsMode::SameOrigin, true, true),
            (&other, RequestCredentialsMode::Include, true, true),
            (&other, RequestCredentialsMode::Omit, false, true),
        ] {
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let request = Request::new_browser("GET", url.clone(), None, Vec::new(), origin.into())
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(mode);
            let head = response_head(&client, transport, request).await?;
            assert_eq!(head.from_cache, from_cache, "{transport:?}, {mode:?}");
            assert_eq!(
                head.request_include_credentials, expected,
                "{transport:?}, {mode:?}"
            );
            assert!(client.shutdown().is_clean());
        }
        assert_eq!(server.hits(), 4, "{transport:?}");
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn credentialless_images_omit_cookies_and_ignore_set_cookie_in_every_transport() -> Result<()>
{
    for transport in Transport::ALL {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("image").with_header("Set-Cookie", "session=original; Path=/"),
            ScriptedResponse::ok("image"),
            ScriptedResponse::ok("image").with_header("Set-Cookie", "session=replaced; Path=/"),
            ScriptedResponse::ok("image"),
        ]);
        let url = Url::parse(&server.url())?;
        let mut other = url.clone();
        other
            .set_port(Some(if url.port() == Some(1) { 2 } else { 1 }))
            .unwrap();
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        response_head(&client, transport, Request::get(url.as_str())?).await?;
        for (origin, credentialless, expected) in [
            (&other, false, true),
            (&other, true, false),
            (&url, true, true),
        ] {
            let request = Request::new_browser("GET", url.clone(), None, Vec::new(), origin.into())
                .with_initiator_url(origin)
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::Include)
                .with_browser_request_metadata(BrowserRequestMetadata::Image)
                .with_credentialless(credentialless);
            let head = response_head(&client, transport, request).await?;
            assert_eq!(head.request_include_credentials, expected, "{transport:?}");
        }
        let requests = server.requests();
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("\r\ncookie: session=original\r\n")
        );
        assert!(!requests[2].to_ascii_lowercase().contains("\r\ncookie:"));
        assert!(
            requests[3]
                .to_ascii_lowercase()
                .contains("\r\ncookie: session=original\r\n"),
            "credentialless Set-Cookie must be ignored"
        );
        assert!(client.shutdown().is_clean());
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn credentialless_http_cache_entries_preserve_flags_without_cross_policy_reuse() -> Result<()>
{
    for transport in [Transport::Html, Transport::Raw] {
        let cache_dir = unique_test_cache_dir();
        let server =
            ScriptedHttpServer::spawn(vec![
                ScriptedResponse::ok("image")
                    .with_header("Cache-Control", "max-age=600");
                4
            ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let url = Url::parse(&server.url())?;
        for (credentialless, cached) in [(false, false), (true, false), (true, true), (false, true)]
        {
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let request = Request::new_browser(
                "GET",
                url.clone(),
                None,
                Vec::new(),
                moli_url::WebOrigin::from_url(&Url::parse("http://other.test/")?),
            )
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
            .with_credentialless(credentialless);
            let head = response_head(&client, transport, request).await?;
            assert_eq!(head.request_include_credentials, !credentialless);
            assert_eq!(
                head.from_cache, cached,
                "{transport:?}, credentialless={credentialless}"
            );
            assert!(client.shutdown().is_clean());
        }
        assert_eq!(server.hits(), 2);
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

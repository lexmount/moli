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
async fn response_credentials_flag_is_recomputed_for_http_cache_hits() -> Result<()> {
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
        // Credentials modes partition the HTTP cache. Changing the request
        // origin within the SameOrigin partition must still recompute the
        // response flag, including after reopening the Fetch client.
        for (origin, mode, expected, from_cache) in [
            (&other, RequestCredentialsMode::Include, true, false),
            (&other, RequestCredentialsMode::Omit, false, false),
            (&url, RequestCredentialsMode::SameOrigin, true, false),
            (&other, RequestCredentialsMode::SameOrigin, false, true),
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
        assert_eq!(server.hits(), 3, "{transport:?}");
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

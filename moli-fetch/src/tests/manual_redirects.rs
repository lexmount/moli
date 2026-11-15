use super::request_security::Transport;
use super::*;

#[tokio::test]
async fn manual_network_redirect_rejects_invalid_location_in_every_transport() -> Result<()> {
    for transport in Transport::ALL {
        for location in ["http://[", "http://%GG/", "http://example.test:abc/"] {
            for status in [301, 302, 303, 307, 308] {
                let server = ScriptedHttpServer::spawn(vec![
                    ScriptedResponse::status(status, "Redirect")
                        .with_header("Location", location)
                        .with_body("invalid redirect body"),
                ]);
                let client =
                    FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
                let request =
                    Request::get(&server.url())?.with_redirect_mode(RequestRedirectMode::Manual);
                let result = transport.fetch(&client, request).await;
                let hits = server.hits();
                server.shutdown();
                let Err(error) = result else {
                    panic!("{transport:?}/{status}/{location}: accepted an invalid redirect");
                };
                assert!(
                    format!("{error:#}").contains("failed to resolve redirect location"),
                    "{transport:?}/{status}/{location}: {error:#}"
                );
                assert_eq!(hits, 1);
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn manual_network_redirect_preserves_valid_locations_and_non_redirect_responses() -> Result<()>
{
    // Fetch API bodies use these transports. The HTML navigation stream has
    // its own automatic-follow contract for valid redirect responses.
    for transport in [Transport::Buffered, Transport::Raw] {
        for (status, location) in [
            (302, None),
            (302, Some("")),
            (302, Some("/not-followed")),
            (302, Some("http://user:pass@example.test/not-followed")),
            (302, Some("http://127.0.0.1:9/not-followed")),
            (200, Some("http://[")),
        ] {
            let mut response = ScriptedResponse::status(status, "Response");
            if let Some(location) = location {
                response = response.with_header("Location", location);
            }
            let server = ScriptedHttpServer::spawn(vec![response]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let request =
                Request::get(&server.url())?.with_redirect_mode(RequestRedirectMode::Manual);
            let result = transport.fetch(&client, request).await;
            let hits = server.hits();
            server.shutdown();
            result.with_context(|| format!("{transport:?}/{status}/{location:?}"))?;
            assert_eq!(hits, 1, "manual mode must never follow Location");
        }
    }
    Ok(())
}

#[tokio::test]
async fn manual_network_redirect_preserves_chromium_network_hosts() -> Result<()> {
    for transport in [Transport::Buffered, Transport::Raw] {
        for location in [
            "http://example%20host.test/",
            "http://xn--a.invalid/",
            "//example%20host.test/",
            "//xn--a.invalid/",
        ] {
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found").with_header("Location", location),
            ]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let request =
                Request::get(&server.url())?.with_redirect_mode(RequestRedirectMode::Manual);
            let result = transport.fetch(&client, request).await;
            let hits = server.hits();
            server.shutdown();
            result.with_context(|| {
                format!("Chromium accepts {location} in {transport:?} manual mode")
            })?;
            assert_eq!(hits, 1);
        }
    }
    Ok(())
}

#[tokio::test]
async fn manual_network_redirect_rejects_invalid_location_before_body() -> Result<()> {
    for transport in Transport::ALL {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/redirect", listener.local_addr()?);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                anyhow::ensure!(stream.read(&mut byte).await? != 0, "incomplete request");
                request.push(byte[0]);
            }
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 302 Found\r\n",
                        "Location: http://[\r\n",
                        "Content-Length: 100\r\n",
                        "Connection: close\r\n\r\n",
                    )
                    .as_bytes(),
                )
                .await?;
            // Hold the body until the client rejects the headers and closes.
            assert_eq!(stream.read(&mut byte).await?, 0);
            Ok::<_, anyhow::Error>(())
        });
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let request = Request::get(&url)?.with_redirect_mode(RequestRedirectMode::Manual);
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            match transport {
                Transport::Raw => client
                    .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                    .await
                    .expect_err("invalid redirect must not publish a raw response head"),
                Transport::Html => client
                    .fetch_html_stream(request)
                    .await
                    .expect_err("invalid redirect must not publish an HTML response head"),
                Transport::Buffered => {
                    let (send, receive) = oneshot::channel();
                    client
                        .fetch_with_cancel_callback(
                            request,
                            FetchCancelHandle::new(),
                            move |result| {
                                let _ = send.send(result);
                            },
                        )
                        .expect("start buffered request");
                    receive
                        .await
                        .expect("buffered result")
                        .expect_err("buffered redirect must fail without reading the body")
                }
            }
        })
        .await?;
        assert!(format!("{error:#}").contains("failed to resolve redirect location"));
        tokio::time::timeout(Duration::from_secs(5), server).await???;
    }
    Ok(())
}

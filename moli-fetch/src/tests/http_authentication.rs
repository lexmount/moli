use super::*;

fn browser_request(url: &str) -> Result<Request> {
    let origin: moli_url::WebOrigin = (&Url::parse(url)?).into();
    Ok(Request::get(url)?.with_request_origin(origin))
}

fn credentials(url: &str) -> String {
    let mut url = Url::parse(url).unwrap();
    url.set_username("us%65r").unwrap();
    url.set_password(Some("p%2Bss%3Aword")).unwrap();
    url.into()
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

fn challenge() -> ScriptedResponse {
    ScriptedResponse::status(401, "Unauthorized")
        .with_header("WWW-Authenticate", "Basic realm=\"session\"")
        .with_header("Cache-Control", "no-store")
}

async fn body(
    client: &FetchClientHandle,
    request: Request,
    transport: &str,
) -> Result<(u16, Vec<u8>)> {
    let response = match transport {
        "buffered" => {
            let response = client.fetch(request.with_single_hop()).await?;
            return Ok((response.status, response.body_text().as_bytes().to_vec()));
        }
        "html" => {
            let response = client
                .fetch_html_stream(request)
                .await?
                .into_materialized_text_response()
                .await?;
            return Ok((response.status, response.body_text().as_bytes().to_vec()));
        }
        "raw" => client.fetch_raw(request).await?,
        _ => unreachable!(),
    };
    Ok((response.status, response.body_bytes().to_vec()))
}

#[tokio::test]
async fn url_basic_authentication_challenges_then_reuses_session_on_all_transports() -> Result<()> {
    use base64::Engine;
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("user:p+ss:word")
    );
    for transport in ["buffered", "html", "raw"] {
        // The private challenge body exceeds the final response limit; it is
        // retired at headers without entering the response collector.
        let server = ScriptedHttpServer::spawn(vec![
            challenge()
                .with_body(&"x".repeat(1024))
                .with_header("Set-Cookie", "challenge=seen; Path=/"),
            ScriptedResponse::ok("accepted"),
            ScriptedResponse::ok("reused"),
            ScriptedResponse::ok("explicit"),
        ]);
        let mut config = FetchConfig::default();
        config.set_connection_limits(None, None, Some(64));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let first = browser_request(&credentials(&server.url()))?;
        assert_eq!(
            body(&client, first, transport).await?,
            (200, b"accepted".to_vec())
        );
        assert_eq!(
            body(&client, browser_request(&server.url())?, transport).await?,
            (200, b"reused".to_vec())
        );
        let mut explicit = browser_request(&credentials(&server.url()))?;
        explicit.credentials_mode = RequestCredentialsMode::Omit;
        explicit
            .request_headers
            .push(("Authorization".into(), b"Bearer author".to_vec()));
        assert_eq!(
            body(&client, explicit, transport).await?,
            (200, b"explicit".to_vec())
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 4, "{transport}");
        assert_eq!(header(&requests[0], "Authorization"), None);
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some(expected.as_str())
        );
        assert_eq!(header(&requests[1], "Cookie"), Some("challenge=seen"));
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some(expected.as_str())
        );
        assert_eq!(header(&requests[3], "Authorization"), Some("Bearer author"));
        assert_eq!(header(&requests[3], "Cookie"), None);
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn browser_url_credentials_respect_cors_and_credentials_mode() -> Result<()> {
    for (mode, credentials_mode, cross_origin) in [
        (RequestMode::Cors, RequestCredentialsMode::Include, true),
        (RequestMode::Cors, RequestCredentialsMode::Omit, false),
        (
            RequestMode::NoCors,
            RequestCredentialsMode::SameOrigin,
            true,
        ),
    ] {
        let server = ScriptedHttpServer::spawn(vec![challenge().with_body("final 401")]);
        let mut request = browser_request(&credentials(&server.url()))?.with_request_mode(mode);
        request.credentials_mode = credentials_mode;
        if cross_origin {
            request = request.with_request_origin((&Url::parse("https://other.test/")?).into());
        }
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let response = client.fetch_raw(request).await?;
        assert_eq!(response.status, 401);
        assert_eq!(response.body_bytes(), b"final 401");
        assert_eq!(server.hits(), 1);
        assert_eq!(header(&server.requests()[0], "Authorization"), None);
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn basic_authentication_cache_is_partitioned_and_owned_by_the_client() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        challenge(),
        ScriptedResponse::ok("accepted"),
        ScriptedResponse::ok("cached"),
        challenge(),
        challenge(),
        challenge(),
    ]);
    let cookies = new_shared_browser_cookie_store();
    let client = FetchClient::new(&FetchConfig::default(), Arc::clone(&cookies));
    let request = |url: &str, partition: &str| -> Result<Request> {
        Ok(browser_request(url)?.with_network_partition_key(Some(partition.into())))
    };
    assert_eq!(
        client
            .fetch_raw(request(&credentials(&server.url()), "partition-a")?)
            .await?
            .status,
        200
    );
    assert_eq!(
        client
            .fetch_raw(request(&server.url(), "partition-a")?)
            .await?
            .status,
        200
    );
    assert_eq!(
        client
            .fetch_raw(request(&server.url(), "partition-b")?)
            .await?
            .status,
        401
    );
    let mut omitted = request(&server.url(), "partition-a")?;
    omitted.credentials_mode = RequestCredentialsMode::Omit;
    assert_eq!(client.fetch_raw(omitted).await?.status, 401);
    let other = FetchClient::new(&FetchConfig::default(), cookies);
    assert_eq!(
        other
            .fetch_raw(request(&server.url(), "partition-a")?)
            .await?
            .status,
        401
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    for request in &requests[3..] {
        assert_eq!(header(request, "Authorization"), None);
    }
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn rejected_basic_credentials_stop_after_one_retry_and_evict_cached_identity() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        challenge(),
        challenge().with_body("rejected"),
        challenge(),
        challenge(),
        ScriptedResponse::ok("accepted"),
        challenge().with_body("expired"),
        challenge(),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    assert_eq!(
        client
            .fetch_raw(browser_request(&credentials(&server.url()))?)
            .await?
            .body_bytes(),
        b"rejected"
    );
    assert_eq!(
        client
            .fetch_raw(browser_request(&server.url())?)
            .await?
            .status,
        401
    );
    assert_eq!(
        client
            .fetch_raw(browser_request(&credentials(&server.url()))?)
            .await?
            .status,
        200
    );
    assert_eq!(
        client
            .fetch_raw(browser_request(&server.url())?)
            .await?
            .body_bytes(),
        b"expired"
    );
    assert_eq!(
        client
            .fetch_raw(browser_request(&server.url())?)
            .await?
            .status,
        401
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 7);
    for index in [0, 2, 3, 6] {
        assert_eq!(header(&requests[index], "Authorization"), None);
    }
    for index in [1, 4, 5] {
        assert!(header(&requests[index], "Authorization").is_some());
    }
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn unsupported_or_malformed_authentication_challenges_preserve_final_response() -> Result<()>
{
    for challenge in [
        "Digest realm=\"other\", nonce=\"n\"",
        "Basic realm=\"unterminated",
        "Basic realm=",
    ] {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(401, "Unauthorized")
                .with_header("WWW-Authenticate", challenge)
                .with_header("X-Final", "retained")
                .with_body("final body"),
        ]);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let response = client
            .fetch_raw(browser_request(&credentials(&server.url()))?)
            .await?;
        assert_eq!(response.status, 401);
        assert_eq!(response.body_bytes(), b"final body");
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "x-final" && value == b"retained")
        );
        assert_eq!(server.hits(), 1);
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn cached_and_configured_basic_credentials_do_not_cross_redirect_origins() -> Result<()> {
    for configured in [false, true] {
        let target = ScriptedHttpServer::spawn(vec![challenge().with_body("target challenge")]);
        let redirect =
            ScriptedResponse::status(302, "Found").with_header("Location", &target.url());
        let responses = if configured {
            vec![redirect]
        } else {
            vec![challenge(), ScriptedResponse::ok("accepted"), redirect]
        };
        let source = ScriptedHttpServer::spawn(responses);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        if !configured {
            assert_eq!(
                client
                    .fetch_raw(browser_request(&credentials(&source.url()))?)
                    .await?
                    .status,
                200
            );
        }
        let mut request = browser_request(&source.url())?.with_request_mode(RequestMode::NoCors);
        if configured {
            request = request.with_auth(RequestAuth {
                target: RequestAuthTarget::Server,
                scheme: RequestAuthScheme::Basic,
                username: "explicit".into(),
                password: "secret".into(),
            });
        }
        let response = client.fetch_raw(request).await?;
        assert_eq!(response.status, 401);
        assert_eq!(response.body_bytes(), b"target challenge");
        assert!(response.redirected);
        assert_eq!(response.redirect_chain.len(), 1);
        assert_eq!(target.hits(), 1);
        assert_eq!(header(&target.requests()[0], "Authorization"), None);
        assert!(header(source.requests().last().unwrap(), "Authorization").is_some());
        source.shutdown();
        target.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn basic_authentication_retry_retains_original_timeout_budget() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        challenge().with_delay_ms(150),
        ScriptedResponse::ok("late").with_delay_ms(250),
    ]);
    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(300);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let error = client
        .fetch_raw(browser_request(&credentials(&server.url()))?)
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<curl::Error>()
            .is_some_and(curl::Error::is_operation_timedout),
        "{error:#}"
    );
    assert_eq!(server.hits(), 2);
    server.shutdown();
    Ok(())
}

async fn read_upload(stream: &mut tokio::net::TcpStream) -> Result<String> {
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(
            request.len() < 65536 && stream.read(&mut byte).await? == 1,
            "incomplete request"
        );
        request.push(byte[0]);
    }
    let length = header(std::str::from_utf8(&request)?, "Content-Length")
        .unwrap_or("0")
        .parse::<usize>()?;
    let start = request.len();
    request.resize(start + length, 0);
    stream.read_exact(&mut request[start..]).await?;
    Ok(String::from_utf8(request)?)
}

#[tokio::test]
async fn basic_authentication_replays_upload_and_keeps_final_body_streaming_and_cancellable()
-> Result<()> {
    for cancel_final_body in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/upload", listener.local_addr()?);
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let first = read_upload(&mut stream).await?;
            stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n").await?;
            drop(stream);
            let (mut stream, _) = listener.accept().await?;
            let second = read_upload(&mut stream).await?;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\none")
                .await?;
            release_rx.await?;
            let _ = stream.write_all(b"two").await;
            Ok::<_, anyhow::Error>((first, second))
        });
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let mut request = browser_request(&credentials(&url))?;
        request.method = "POST".into();
        request.body = Some(bytes::Bytes::from_static(b"payload"));
        let cancel = FetchCancelHandle::new();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            client.fetch_raw_stream_with_cancel_and_network_metadata(request, cancel.clone()),
        )
        .await??;
        let (mut response, journal) = result.into_parts_with_observation_journal();
        assert_eq!(response.status, 200);
        assert!(!response.redirected);
        assert!(response.redirect_chain.is_empty());
        assert_eq!(
            journal
                .exchanges()
                .iter()
                .map(|exchange| exchange.response().unwrap().status())
                .collect::<Vec<_>>(),
            [401, 200]
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), response.next_chunk()).await?,
            Some(b"one".to_vec())
        );
        assert!(!cancel.response_completion_is_committed());
        if cancel_final_body {
            assert!(cancel.cancel_if_response_pending());
        }
        release_tx.send(()).unwrap();
        if cancel_final_body {
            let error = response.finish().await.unwrap_err();
            assert!(crate::is_fetch_cancelled(&error), "{error:#}");
        } else {
            assert_eq!(response.next_chunk().await, Some(b"two".to_vec()));
            response.finish().await?;
        }
        let (first, second) = tokio::time::timeout(Duration::from_secs(5), server).await???;
        for request in [&first, &second] {
            assert!(request.starts_with("POST /upload HTTP/1.1\r\n"));
            assert!(request.ends_with("\r\n\r\npayload"));
        }
        assert_eq!(header(&first, "Authorization"), None);
        assert!(header(&second, "Authorization").is_some());
    }
    Ok(())
}

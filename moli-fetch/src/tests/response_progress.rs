use super::*;

async fn read_request_head(stream: &mut tokio::net::TcpStream) -> Result<String> {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        anyhow::ensure!(stream.read(&mut byte).await? != 0, "incomplete request");
        request.push(byte[0]);
    }
    Ok(String::from_utf8(request)?)
}

#[tokio::test]
async fn buffered_auth_finishes_null_body_probe_before_resending_upload() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(204, "No Content"),
        ScriptedResponse::status(204, "No Content"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let response = client
        .fetch(
            Request::new(
                "POST",
                &server.url(),
                Some("payload".to_owned()),
                Vec::new(),
            )?
            .with_auth(RequestAuth {
                target: RequestAuthTarget::Server,
                scheme: RequestAuthScheme::Digest,
                username: "user".to_owned(),
                password: "pass".to_owned(),
            }),
        )
        .await?;
    assert_eq!(response.status, 204);
    assert_eq!(
        server.hits(),
        2,
        "the empty auth probe must precede the upload"
    );
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn redirect_transports_retire_body_at_headers() -> Result<()> {
    for (authenticate, use_callback) in [(false, false), (false, true), (true, false), (true, true)]
    {
        for handling in [
            RequestRedirectHandling::Follow,
            RequestRedirectHandling::FollowByCaller,
            RequestRedirectHandling::Manual,
            RequestRedirectHandling::Error,
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/start", listener.local_addr()?);
            let server = tokio::spawn(async move {
                if authenticate {
                    let (mut challenge, _) = listener.accept().await?;
                    assert!(
                        read_request_head(&mut challenge)
                            .await?
                            .starts_with("GET /start ")
                    );
                    challenge
                        .write_all(
                            concat!(
                                "HTTP/1.1 401 Unauthorized\r\n",
                                "WWW-Authenticate: Digest realm=\"test\", nonce=\"nonce\", qop=\"auth\"\r\n",
                                "Content-Length: 0\r\n",
                                "Connection: close\r\n\r\n",
                            )
                            .as_bytes(),
                        )
                        .await?;
                    drop(challenge);
                }
                let (mut redirect, _) = listener.accept().await?;
                let authenticated = read_request_head(&mut redirect).await?;
                assert!(authenticated.starts_with("GET /start "));
                assert_eq!(
                    authenticated
                        .to_ascii_lowercase()
                        .contains("authorization: digest "),
                    authenticate
                );
                redirect
                    .write_all(
                        concat!(
                            "HTTP/1.1 302 Found\r\n",
                            "Location: /final\r\n",
                            "Content-Length: 100\r\n",
                            "Connection: close\r\n\r\n",
                        )
                        .as_bytes(),
                    )
                    .await?;
                // The redirect body is never sent, even after authentication.
                assert_eq!(redirect.read(&mut [0_u8; 1]).await?, 0);
                if handling == RequestRedirectHandling::Follow {
                    let (mut final_response, _) = listener.accept().await?;
                    assert!(
                        read_request_head(&mut final_response)
                            .await?
                            .starts_with("GET /final ")
                    );
                    final_response
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone")
                        .await?;
                }
                Ok::<_, anyhow::Error>(())
            });
            let mut config = FetchConfig::default();
            // The discarded redirect advertises a body larger than this limit.
            config.set_connection_limits(None, None, Some(4));
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let mut request = Request::get(&url)?.with_redirect_handling(handling);
            if authenticate {
                request = request.with_auth(RequestAuth {
                    target: RequestAuthTarget::Server,
                    scheme: RequestAuthScheme::Digest,
                    username: "user".to_owned(),
                    password: "pass".to_owned(),
                });
            }
            let response = tokio::time::timeout(Duration::from_secs(5), async {
                if use_callback {
                    let (tx, rx) = oneshot::channel();
                    let observer = child.clone();
                    client.fetch_with_cancel_callback(request, child.clone(), move |result| {
                        assert!(observer.response_completion_is_committed());
                        let _ = tx.send(result);
                    })?;
                    rx.await?
                } else {
                    client.fetch_with_cancel(request, child.clone()).await
                }
            })
            .await?
            .with_context(|| {
                format!("{handling:?}, auth={authenticate}, callback={use_callback}")
            })?;
            parent.adopt_response_progress(&child);
            if handling == RequestRedirectHandling::Follow {
                assert_eq!(response.status, 200);
                assert_eq!(response.body_text(), "done");
                assert_eq!(response.final_url.path(), "/final");
                assert_eq!(response.redirect_chain.len(), 1);
            } else {
                assert_eq!(response.status, 302);
                assert!(response.body_text().is_empty());
                assert_eq!(response.final_url.path(), "/start");
                assert!(response.redirect_chain.is_empty());
            }
            assert!(parent.response_completion_is_committed());
            assert!(!parent.cancel_if_response_pending());
            assert!(!parent.is_cancelled());
            tokio::time::timeout(Duration::from_secs(5), server).await???;
        }
    }
    Ok(())
}

#[tokio::test]
async fn buffered_response_commits_completion_before_delivery() -> Result<()> {
    let responses = [
        ScriptedResponse::ok("body"),
        ScriptedResponse::status(204, "No Content"),
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "/final")
            .with_body("redirect body"),
    ];
    let server = ScriptedHttpServer::spawn(responses.iter().chain(&responses).cloned().collect());
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for use_callback in [false, true] {
        for (status, body) in [(200, "body"), (204, ""), (302, "")] {
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let request = Request::get(&server.url())?.with_single_hop();
            let response = if use_callback {
                let (tx, rx) = oneshot::channel();
                let observer = child.clone();
                client.fetch_with_cancel_callback(request, child.clone(), move |result| {
                    // The token must be committed before invoking consumers.
                    assert!(observer.response_completion_is_committed());
                    let _ = tx.send(result);
                })?;
                rx.await??
            } else {
                client.fetch_with_cancel(request, child.clone()).await?
            };
            parent.adopt_response_progress(&child);
            assert_eq!(response.status, status);
            assert_eq!(response.body_text(), body);
            assert!(parent.response_completion_is_committed());
            assert!(!parent.cancel_if_response_pending());
            assert!(!parent.is_cancelled());
        }
    }
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn auth_challenge_body_wait_is_cancellable_without_committing_completion() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/auth", listener.local_addr()?);
    let (headers_tx, headers_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut byte = [0_u8; 1];
        assert!(
            read_request_head(&mut stream)
                .await?
                .starts_with("GET /auth ")
        );
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 401 Unauthorized\r\n",
                    "WWW-Authenticate: Digest realm=\"test\", nonce=\"nonce\", qop=\"auth\"\r\n",
                    "Content-Length: 100\r\n",
                    "Connection: keep-alive\r\n\r\n",
                )
                .as_bytes(),
            )
            .await?;
        let _ = headers_tx.send(());
        assert_eq!(stream.read(&mut byte).await?, 0);
        Ok::<_, anyhow::Error>(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let (tx, mut rx) = oneshot::channel();
    client.fetch_with_cancel_callback(
        Request::get(&url)?.with_auth(RequestAuth {
            target: RequestAuthTarget::Server,
            scheme: RequestAuthScheme::Digest,
            username: "user".to_owned(),
            password: "pass".to_owned(),
        }),
        cancel.clone(),
        move |result| {
            let _ = tx.send(result);
        },
    )?;
    tokio::time::timeout(Duration::from_secs(5), headers_rx).await??;
    // Libcurl must finish HTTP/1 framing to reuse this authentication connection.
    // No challenge body is sent, so cancellation must interrupt the negotiation.
    assert!(matches!(
        rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(!cancel.response_completion_is_committed());
    assert!(cancel.cancel_if_response_pending());
    let error = tokio::time::timeout(Duration::from_secs(5), rx)
        .await??
        .unwrap_err();
    assert!(error.is::<crate::FetchCancelled>());
    assert!(cancel.is_cancelled());
    assert!(!cancel.response_completion_is_committed());
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

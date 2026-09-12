use super::*;
use anyhow::Result;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn request_head(socket: &mut tokio::net::TcpStream) -> Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(
            bytes.len() < 8192 && socket.read(&mut byte).await? == 1,
            "incomplete request head"
        );
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8(bytes)?)
}

#[test]
fn cors_redirect_referrer_never_recovers_stripped_information() -> Result<()> {
    let source = url::Url::parse("https://origin.test/private?token=secret#fragment")?;
    for (initial_policy, first_referrer) in [
        ("no-referrer", None),
        ("origin", Some("https://origin.test/")),
        (
            "unsafe-url",
            Some("https://origin.test/private?token=secret"),
        ),
    ] {
        let request = Request::new("GET", "https://origin.test/start", None, Vec::new())?
            .with_initiator_url(&source)
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
            .with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
                referrer_policy: Some(initial_policy.to_owned()),
                ..Default::default()
            });
        let mut redirects = ManualCorsRedirectState::new(request, Vec::new());
        for (index, (location, policy)) in [
            ("https://remote.test/second", "unsafe-url"),
            ("https://another.test/third", "origin"),
            ("https://origin.test/final", "unsafe-url"),
        ]
        .into_iter()
        .enumerate()
        {
            let head = ResponseHead {
                final_url: redirects.request().url.clone(),
                status: 302,
                status_text: None,
                headers: vec![
                    ("Location".to_owned(), location.to_owned()),
                    ("Referrer-Policy".to_owned(), policy.to_owned()),
                    ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
                ],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            };
            assert_eq!(
                redirects.advance(head, false, None),
                Ok(ManualCorsRedirectTransition::FollowedRedirect)
            );
            let request = redirects.request();
            let expected = if index == 0 {
                first_referrer
            } else {
                first_referrer.map(|_| "https://origin.test/")
            };
            assert_eq!(
                request.referrer_header_value(&request.url).as_deref(),
                expected,
                "initial policy={initial_policy}, hop={index}"
            );
            assert_eq!(request.cookie_context.initiator_url.as_ref(), Some(&source));
        }
    }
    Ok(())
}

#[test]
fn cors_redirect_modes_precede_location_parsing() -> Result<()> {
    let url = url::Url::parse("http://origin.test/start")?;
    for status in [301, 302, 303, 307, 308] {
        for location in [
            None,
            Some("http://[invalid"),
            Some("data:text/plain,redirect"),
            Some("http://user:password@remote.test/"),
        ] {
            for mode in [RequestRedirectMode::Manual, RequestRedirectMode::Error] {
                let request = Request::new("GET", url.as_str(), None, Vec::new())?
                    .with_initiator_url(&url)
                    .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
                    .with_redirect_mode(mode);
                let mut redirects = ManualCorsRedirectState::new(request, Vec::new());
                let head = ResponseHead {
                    final_url: url.clone(),
                    status,
                    status_text: None,
                    headers: location
                        .map(|value| vec![("Location".to_owned(), value.to_owned())])
                        .unwrap_or_default(),
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                };
                let result = redirects.advance(head, false, None);
                if mode == RequestRedirectMode::Manual {
                    assert_eq!(
                        result,
                        Ok(ManualCorsRedirectTransition::ManualResponse {
                            response_url: url.clone()
                        }),
                        "status={status}, location={location:?}"
                    );
                } else {
                    assert_eq!(
                        result,
                        Err(redirect_mode_error_message(&url)),
                        "status={status}, location={location:?}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_discards_intermediate_bodies_and_tracks_final_completion() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
    let (retired_tx, retired_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut discarded = Vec::new();
        let mut final_socket = None;
        for (method, path, origin) in [
            ("OPTIONS", "/start", "http://origin.test"),
            ("PUT", "/start", "http://origin.test"),
            ("OPTIONS", "/final", "null"),
            ("PUT", "/final", "null"),
        ] {
            let (mut socket, _) = listener.accept().await?;
            let head = request_head(&mut socket).await?;
            assert!(
                head.starts_with(&format!("{method} {path} HTTP/1.1")),
                "{head}"
            );
            assert!(
                head.to_ascii_lowercase()
                    .contains(&format!("\r\norigin: {origin}\r\n")),
                "{head}"
            );
            let final_response = method == "PUT" && path == "/final";
            let mut response = format!(
                "HTTP/1.1 {} Response\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: PUT\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n",
                if method == "PUT" && path == "/start" {
                    307
                } else {
                    200
                },
                if final_response { 5 } else { 100 }
            );
            if method == "PUT" && path == "/start" {
                response.push_str(&format!(
                    "Location: http://localhost:{}/final\r\n",
                    address.port()
                ));
            }
            response.push_str("\r\n");
            socket.write_all(response.as_bytes()).await?;
            if final_response {
                socket.write_all(b"he").await?;
                final_socket = Some(socket);
            } else {
                // Deliberately never send the declared preflight/redirect body.
                discarded.push(socket);
            }
        }
        for mut socket in discarded {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), socket.read(&mut [0; 1])).await??,
                0,
                "discarded hop must release its connection"
            );
        }
        let _ = retired_tx.send(());
        tail_rx.await?;
        final_socket.unwrap().write_all(b"llo").await?;
        Ok::<_, anyhow::Error>(())
    });
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let owner = ResourceRequestClient::new(&config)?;
    let loader = owner.handle();
    let cancel = FetchCancelHandle::new();
    let request = Request::new("PUT", &format!("http://{address}/start"), None, Vec::new())?
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_initiator_url(&url::Url::parse("http://origin.test/page")?)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
    let observed = tokio::time::timeout(
        Duration::from_secs(3),
        fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
            &loader,
            request,
            Some(cancel.clone()),
            Vec::new(),
        ),
    )
    .await
    .expect("final headers must not wait for discarded response bodies")
    .map_err(anyhow::Error::msg)?;
    let mut response = observed.into_response();
    assert_eq!(response.status, 200);
    assert_eq!(response.redirect_chain.len(), 1);
    tokio::time::timeout(Duration::from_secs(3), retired_rx).await??;
    assert!(
        !cancel.is_cancelled(),
        "retiring a hop must not cancel the logical request"
    );
    assert!(
        !cancel.response_completion_is_committed(),
        "discarded hops must not commit the unfinished final response"
    );
    tail_tx.send(()).unwrap();
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend(chunk);
    }
    assert_eq!(body, b"hello");
    assert!(
        cancel.response_completion_is_committed(),
        "final transport progress must reach the logical handle"
    );
    cancel.cancel();
    response.finish().await?;
    server.await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_parent_abort_reaches_the_current_hop() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut redirect, _) = listener.accept().await?;
        assert!(
            request_head(&mut redirect)
                .await?
                .starts_with("GET /start HTTP/1.1")
        );
        redirect.write_all(b"HTTP/1.1 302 Found\r\nAccess-Control-Allow-Origin: *\r\nLocation: /final\r\nContent-Length: 100\r\nConnection: close\r\n\r\n").await?;
        let (mut final_socket, _) = listener.accept().await?;
        assert!(
            request_head(&mut final_socket)
                .await?
                .starts_with("GET /final HTTP/1.1")
        );
        let _ = started_tx.send(());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), final_socket.read(&mut [0; 1])).await??,
            0,
            "parent abort must close the active transport"
        );
        Ok::<_, anyhow::Error>(())
    });
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let owner = ResourceRequestClient::new(&config)?;
    let loader = owner.handle();
    let cancel = FetchCancelHandle::new();
    let task_cancel = cancel.clone();
    let request = Request::new("GET", &format!("http://{address}/start"), None, Vec::new())?
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_initiator_url(&url::Url::parse("http://origin.test/page")?)
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
    let fetch = tokio::spawn(async move {
        fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
            &loader,
            request,
            Some(task_cancel),
            Vec::new(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), started_rx).await??;
    assert!(!cancel.response_completion_is_committed());
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), fetch)
            .await??
            .is_err()
    );
    server.await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_rejects_preflight_redirects_and_opaque_origin_denials_before_contact()
-> Result<()> {
    for preflight in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            loop {
                let mut socket = tokio::select! {
                    accepted = listener.accept() => accepted?.0,
                    _ = &mut stop_rx => break,
                };
                let head = request_head(&mut socket).await?;
                let line = head.lines().next().unwrap();
                let first = line.contains(" /start ");
                let response = format!(
                    "HTTP/1.1 {} Response\r\nLocation: /forbidden\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: PUT\r\nContent-Length: 2\r\nConnection: close\r\nCache-Control: no-store\r\n\r\nok",
                    if first { 302 } else { 200 },
                    if first && !preflight {
                        "http://wrong.test"
                    } else {
                        "*"
                    }
                );
                requests.push(line.to_owned());
                socket.write_all(response.as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(requests)
        });
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let owner = ResourceRequestClient::new(&config)?;
        let request = Request::new(
            if preflight { "PUT" } else { "GET" },
            &format!("http://{address}/start"),
            None,
            Vec::new(),
        )?
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let request = if preflight {
            request.with_initiator_url(&url::Url::parse("http://origin.test/page")?)
        } else {
            // An explicit opaque origin can exist without an initiator URL.
            request.with_request_origin(moli_url::WebOrigin::Opaque)
        };
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                &owner.handle(),
                request,
                None,
                Vec::new(),
            ),
        )
        .await?;
        let _ = stop_tx.send(());
        let requests = server.await??;
        assert!(result.is_err(), "preflight={preflight}");
        assert_eq!(
            requests,
            [format!(
                "{} /start HTTP/1.1",
                if preflight { "OPTIONS" } else { "GET" }
            )]
        );
    }
    Ok(())
}

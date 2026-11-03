use super::*;
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn stream_source_redirects_do_not_replay_on_materialized_or_raw_fetches() -> Result<()> {
    for raw in [false, true] {
        for mode in [RequestMode::Cors, RequestMode::SameOrigin] {
            for method in ["POST", "PUT"] {
                for status in [301, 302, 303, 307, 308] {
                    let listener = TcpListener::bind("127.0.0.1:0").await?;
                    let url = url::Url::parse(&format!("http://{}/start", listener.local_addr()?))?;
                    let (stop, mut stopped) = tokio::sync::oneshot::channel();
                    let server = tokio::spawn(async move {
                        let mut requests = Vec::new();
                        loop {
                            let mut socket = tokio::select! {
                                result = listener.accept() => result?.0,
                                _ = &mut stopped => break,
                            };
                            let head = super::redirect_tests::request_head(&mut socket).await?;
                            let line = head.lines().next().context("request line")?.to_owned();
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            let mut body = vec![0; length];
                            socket.read_exact(&mut body).await?;
                            let response = if line.contains(" /start ") {
                                format!(
                                    "HTTP/1.1 {status} Redirect\r\nLocation: /final\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                )
                            } else {
                                "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                            };
                            requests.push((line, body));
                            socket.write_all(response.as_bytes()).await?;
                        }
                        Ok::<_, anyhow::Error>(requests)
                    });
                    let mut config = moli_fetch::FetchConfig::default();
                    config.set_http_no_proxy(Some("*".to_owned()));
                    let owner = ResourceRequestClient::new(&config)?;
                    // A byte snapshot must not erase an author stream's source
                    // identity. Test the replay policy independently of H2 framing.
                    let request = Request::new_browser(
                        method,
                        url.clone(),
                        Some(bytes::Bytes::from_static(b"Test")),
                        Vec::new(),
                        (&url).into(),
                    )
                    .with_body_source_is_null(true)
                    .with_request_mode(mode)
                    .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
                    let fetch = async {
                        if raw {
                            match fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(
                                &owner.handle(), request, None, Vec::new(),
                            ).await {
                                Ok(observed) => observed.into_response().into_lossy_materialized_text_response().await.map_err(format_network_error),
                                Err(error) => Err(error),
                            }
                        } else {
                            fetch_browser_subresource_with_preflight(owner.handle(), request, None)
                                .await
                        }
                    };
                    let result = tokio::time::timeout(Duration::from_secs(5), fetch).await?;
                    let _ = stop.send(());
                    let requests = server.await??;
                    assert_eq!(
                        requests[0],
                        (format!("{method} /start HTTP/1.1"), b"Test".to_vec())
                    );
                    if status == 303 {
                        let response = result.map_err(anyhow::Error::msg)?;
                        assert_eq!(response.status, 200);
                        assert_eq!(requests.len(), 2);
                        assert_eq!(requests[1], ("GET /final HTTP/1.1".to_owned(), Vec::new()));
                    } else {
                        let error = result.expect_err("stream source cannot be replayed");
                        assert!(error.contains("streaming request body"), "{error}");
                        assert_eq!(
                            requests.len(),
                            1,
                            "raw={raw}, mode={mode:?}, method={method}, status={status}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn response_head(url: &url::Url, status: u16, location: Option<&str>) -> ResponseHead {
    ResponseHead {
        final_url: url.clone(),
        status,
        status_text: None,
        headers: location
            .map(|value| vec![("Location".to_owned(), value.as_bytes().to_vec())])
            .unwrap_or_default(),
        request_cookie_report: None,
        request_include_credentials: false,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        cache_state: Default::default(),
        preload_state: Default::default(),
        negotiated_http_version: None,
    }
}

#[test]
fn stream_source_redirect_modes_and_missing_location_keep_the_response() -> Result<()> {
    let url = url::Url::parse("https://origin.test/start")?;
    for status in [301, 302, 303, 307, 308] {
        for location in [None, Some("/final")] {
            for mode in [
                RequestRedirectMode::Follow,
                RequestRedirectMode::Manual,
                RequestRedirectMode::Error,
            ] {
                let request = Request::new_browser(
                    "POST",
                    url.clone(),
                    Some(bytes::Bytes::from_static(b"Test")),
                    Vec::new(),
                    (&url).into(),
                )
                .with_body_source_is_null(true)
                .with_redirect_mode(mode)
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
                let mut state = ManualCorsRedirectState::new(request, Vec::new());
                let result = state.advance(response_head(&url, status, location), false, None);
                match mode {
                    RequestRedirectMode::Manual => assert_eq!(
                        result,
                        Ok(ManualCorsRedirectTransition::ManualResponse {
                            response_url: url.clone()
                        })
                    ),
                    RequestRedirectMode::Error => {
                        assert_eq!(result, Err(redirect_mode_error_message(&url)))
                    }
                    RequestRedirectMode::Follow if location.is_none() => {
                        assert_eq!(result, Ok(ManualCorsRedirectTransition::FinalResponse))
                    }
                    RequestRedirectMode::Follow if status == 303 => {
                        assert_eq!(result, Ok(ManualCorsRedirectTransition::FollowedRedirect));
                        assert_eq!(state.request().method, "GET");
                        assert!(state.request().body.is_none());
                        assert!(!state.request().body_source_is_null());
                    }
                    RequestRedirectMode::Follow => assert!(result.is_err()),
                }
            }
        }
    }
    Ok(())
}

#[test]
fn byte_backed_request_bodies_remain_replayable_across_redirects() -> Result<()> {
    let url = url::Url::parse("https://origin.test/start")?;
    for status in [301, 302, 307, 308] {
        let request = Request::new_browser(
            "POST",
            url.clone(),
            Some(bytes::Bytes::from_static(b"Test")),
            Vec::new(),
            (&url).into(),
        )
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let mut state = ManualCorsRedirectState::new(request, Vec::new());
        assert_eq!(
            state.advance(response_head(&url, status, Some("/final")), false, None),
            Ok(ManualCorsRedirectTransition::FollowedRedirect)
        );
        if matches!(status, 301 | 302) {
            assert_eq!(state.request().method, "GET");
            assert!(state.request().body.is_none());
        } else {
            assert_eq!(state.request().method, "POST");
            assert_eq!(state.request().body.as_deref(), Some(b"Test".as_slice()));
        }
    }
    Ok(())
}

use super::*;
mod vary;
use moli_fetch::{NetworkObservationJournal, RequestCacheMode, ResponseCacheState};

fn reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

type ServerTask = tokio::task::JoinHandle<Result<Vec<Vec<u8>>>>;

async fn serve(listener: TcpListener, responses: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
    let mut requests = Vec::new();
    for response in responses {
        let (mut socket, _) = timeout(Duration::from_secs(5), listener.accept()).await??;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(timeout(Duration::from_secs(5), socket.read_u8()).await??);
        }
        requests.push(head);
        socket.write_all(&response).await?;
    }
    Ok(requests)
}

async fn server(responses: Vec<Vec<u8>>) -> Result<(String, ServerTask)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/resource", listener.local_addr()?);
    let task = tokio::spawn(serve(listener, responses));
    Ok((url, task))
}

fn request(url: &str) -> Result<Request> {
    Ok(browser_navigation_request(url)?
        .with_page_network_policy()
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch))
}

async fn fetch(
    client: &ResourceRequestClient,
    request: Request,
    observe: bool,
) -> Result<(RawResponse, NetworkObservationJournal, FetchCancelHandle)> {
    let cancel = FetchCancelHandle::new();
    let result = if observe {
        client
            .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel.clone())
            .await?
    } else {
        moli_fetch::NetworkFetchResult::without_request_observation(
            client
                .fetch_raw_stream_with_cancel(request, cancel.clone())
                .await?,
        )
    };
    let (stream, journal) = result.into_parts_with_observation_journal();
    Ok((
        stream.into_materialized_raw_response().await?,
        journal,
        cancel,
    ))
}

fn header<'a>(request: &'a [u8], name: &str) -> Option<&'a [u8]> {
    request.split(|byte| *byte == b'\n').find_map(|line| {
        let colon = line.iter().position(|byte| *byte == b':')?;
        line[..colon]
            .eq_ignore_ascii_case(name.as_bytes())
            .then(|| line[colon + 1..].trim_ascii_start().trim_ascii_end())
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_revalidates_request_controls_and_preserves_wire_observations()
-> Result<()> {
    for observe in [false, true] {
        for control in [
            Some(("Cache-Control", "max-age=0")),
            Some(("Cache-Control", "no-cache")),
            Some(("Pragma", "no-cache")),
            None,
        ] {
            let (url, server) = server(vec![
                reply(
                    200,
                    &[
                        ("Cache-Control", "max-age=60"),
                        ("ETag", "\"version-1\""),
                        ("X-Metadata", "old"),
                    ],
                    b"cached body",
                ),
                reply(
                    304,
                    &[
                        ("Cache-Control", "max-age=600"),
                        ("ETag", "\"version-1\""),
                        ("X-Metadata", "new"),
                    ],
                    b"",
                ),
            ])
            .await?;
            let client = ResourceRequestClient::new(&FetchConfig::default())?;
            let first = fetch(&client, request(&url)?, observe).await?.0;
            assert_eq!(first.cache_state, ResponseCacheState::None);
            let local = fetch(&client, request(&url)?, observe).await?;
            assert_eq!(local.0.cache_state, ResponseCacheState::Local);
            assert!(local.1.is_empty());
            let mut validation = request(&url)?;
            if let Some((name, value)) = control {
                validation.request_headers = vec![(name.to_owned(), value.to_owned())].into();
            } else {
                validation = validation.with_cache_mode(RequestCacheMode::Validate);
            }
            let (validated, journal, cancel) = fetch(&client, validation, observe).await?;
            assert_eq!(validated.status, 200);
            assert_eq!(validated.body_bytes(), b"cached body");
            assert_eq!(validated.cache_state, ResponseCacheState::Validated);
            assert!(validated.from_cache);
            assert!(
                validated
                    .headers
                    .contains(&("x-metadata".into(), b"new".to_vec()))
            );
            assert!(
                validated
                    .headers
                    .contains(&("content-length".into(), b"11".to_vec()))
            );
            assert!(cancel.response_timing().request_started_at().is_some());
            assert!(cancel.response_timing().response_started_at().is_some());
            assert!(cancel.response_timing().response_end_at().is_some());
            assert!(!cancel.response_timing().body_bytes_received());
            if observe {
                assert_eq!(journal.exchanges().len(), 1);
                assert_eq!(journal.final_response_observation().unwrap().status(), 304);
                assert!(
                    journal
                        .final_request_observation()
                        .unwrap()
                        .headers()
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("if-none-match")
                            && value == "\"version-1\"")
                );
            } else {
                assert!(journal.is_empty());
            }
            let refreshed = fetch(&client, request(&url)?, observe).await?.0;
            assert_eq!(refreshed.cache_state, ResponseCacheState::Local);
            assert_eq!(refreshed.body_bytes(), b"cached body");
            let requests = server.await??;
            assert_eq!(requests.len(), 2);
            assert_eq!(header(&requests[0], "if-none-match"), None);
            assert_eq!(
                header(&requests[1], "if-none-match"),
                Some(b"\"version-1\"".as_slice())
            );
            if let Some((name, value)) = control {
                assert_eq!(header(&requests[1], name), Some(value.as_bytes()));
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_retains_stale_and_no_cache_validators() -> Result<()> {
    for policy in ["max-age=0", "no-cache"] {
        let modified = "Wed, 21 Oct 2015 07:28:00 GMT";
        let (url, server) = server(vec![
            reply(
                200,
                &[("Cache-Control", policy), ("Last-Modified", modified)],
                b"last modified body",
            ),
            reply(
                304,
                &[
                    ("Cache-Control", "max-age=600"),
                    ("Last-Modified", modified),
                ],
                b"",
            ),
        ])
        .await?;
        let client = ResourceRequestClient::new(&FetchConfig::default())?;
        fetch(&client, request(&url)?, false).await?;
        let validated = fetch(&client, request(&url)?, false).await?.0;
        assert_eq!(validated.cache_state, ResponseCacheState::Validated);
        assert_eq!(validated.body_bytes(), b"last modified body");
        let requests = server.await??;
        assert_eq!(
            header(&requests[1], "if-modified-since"),
            Some(modified.as_bytes())
        );
        assert_eq!(header(&requests[1], "if-none-match"), None);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_does_not_reuse_or_store_no_store_and_author_conditionals() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[("Cache-Control", "max-age=600"), ("ETag", "\"original\"")],
            b"original",
        ),
        reply(200, &[("Cache-Control", "max-age=600")], b"private one"),
        reply(200, &[("Cache-Control", "max-age=600")], b"private two"),
        reply(304, &[], b""),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, false).await?;
    let no_store = request(&url)?.with_cache_mode(RequestCacheMode::NoStore);
    assert_eq!(
        fetch(&client, no_store, false).await?.0.body_bytes(),
        b"private one"
    );
    let mut no_store_header = request(&url)?;
    no_store_header.request_headers =
        vec![("Cache-Control".into(), "max-age=0, No-Store".into())].into();
    assert_eq!(
        fetch(&client, no_store_header, false).await?.0.body_bytes(),
        b"private two"
    );
    let mut conditional = request(&url)?;
    conditional.request_headers = vec![("If-None-Match".into(), "\"author\"".into())].into();
    let author_response = fetch(&client, conditional, true).await?.0;
    assert_eq!(author_response.status, 304);
    assert_eq!(author_response.cache_state, ResponseCacheState::None);
    assert!(author_response.body_bytes().is_empty());
    let retained = fetch(&client, request(&url)?, false).await?.0;
    assert_eq!(retained.body_bytes(), b"original");
    assert_eq!(retained.cache_state, ResponseCacheState::Local);
    let requests = server.await??;
    for request in &requests[..3] {
        assert_eq!(header(request, "if-none-match"), None);
    }
    assert_eq!(
        header(&requests[3], "if-none-match"),
        Some(b"\"author\"".as_slice())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_revalidation_drops_non_storable_replacements() -> Result<()> {
    for (status, control) in [
        (304, ("Cache-Control", "no-store")),
        (304, ("Vary", "*")),
        (304, ("Vary", "Cookie")),
        (200, ("Cache-Control", "no-store")),
    ] {
        let (url, server) = server(vec![
            reply(
                200,
                &[("Cache-Control", "max-age=600"), ("ETag", "\"original\"")],
                b"original",
            ),
            reply(
                status,
                &[control],
                if status == 304 { b"" } else { b"replacement" },
            ),
            reply(200, &[("Cache-Control", "max-age=600")], b"new body"),
        ])
        .await?;
        let client = ResourceRequestClient::new(&FetchConfig::default())?;
        fetch(&client, request(&url)?, false).await?;
        let validated = fetch(
            &client,
            request(&url)?.with_cache_mode(RequestCacheMode::Validate),
            false,
        )
        .await?
        .0;
        assert_eq!(validated.status, 200);
        assert_eq!(
            validated.body_bytes(),
            if status == 304 {
                b"original".as_slice()
            } else {
                b"replacement".as_slice()
            }
        );
        let fresh = fetch(&client, request(&url)?, false).await?.0;
        assert_eq!(fresh.cache_state, ResponseCacheState::None);
        assert_eq!(fresh.body_bytes(), b"new body");
        assert_eq!(
            fetch(&client, request(&url)?, false).await?.0.cache_state,
            ResponseCacheState::Local
        );
        let requests = server.await??;
        assert_eq!(header(&requests[2], "if-none-match"), None);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_keeps_vary_variants_separate_during_validation() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Accept-Language"),
                ("ETag", "\"fr\""),
            ],
            b"bonjour",
        ),
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Accept-Language"),
                ("ETag", "\"en\""),
            ],
            b"hello",
        ),
        reply(304, &[("Cache-Control", "max-age=600")], b""),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let variant = |language: &str| -> Result<Request> {
        let mut request = request(&url)?;
        request.request_headers = vec![("Accept-Language".into(), language.into())].into();
        Ok(request)
    };
    assert_eq!(
        fetch(&client, variant("fr")?, false).await?.0.body_bytes(),
        b"bonjour"
    );
    assert_eq!(
        fetch(&client, variant("en")?, false).await?.0.body_bytes(),
        b"hello"
    );
    let validated = fetch(
        &client,
        variant("fr")?.with_cache_mode(RequestCacheMode::Validate),
        false,
    )
    .await?
    .0;
    assert_eq!(validated.cache_state, ResponseCacheState::Validated);
    assert_eq!(validated.body_bytes(), b"bonjour");
    let english = fetch(&client, variant("en")?, false).await?.0;
    assert_eq!(english.cache_state, ResponseCacheState::Local);
    assert_eq!(english.body_bytes(), b"hello");
    let requests = server.await??;
    assert_eq!(
        header(&requests[2], "if-none-match"),
        Some(b"\"fr\"".as_slice())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_validation_headers_do_not_follow_redirects() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[("Cache-Control", "max-age=600"), ("ETag", "\"source\"")],
            b"source body",
        ),
        reply(302, &[("Location", "/target")], b""),
        reply(200, &[("Cache-Control", "max-age=600")], b"target body"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, true).await?;
    let (redirected, journal, _) = fetch(
        &client,
        request(&url)?.with_cache_mode(RequestCacheMode::Validate),
        true,
    )
    .await?;
    assert_eq!(redirected.status, 200);
    assert!(redirected.redirected);
    assert_eq!(redirected.body_bytes(), b"target body");
    assert_eq!(redirected.cache_state, ResponseCacheState::None);
    assert_eq!(journal.exchanges().len(), 2);
    let requests = server.await??;
    assert_eq!(
        header(&requests[1], "if-none-match"),
        Some(b"\"source\"".as_slice())
    );
    assert_eq!(header(&requests[2], "if-none-match"), None);
    assert!(requests[2].starts_with(b"GET /target "));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_validation_preserves_opaque_etag_bytes() -> Result<()> {
    let initial = [
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=600\r\nETag: \"\xff\xe9\"\r\n".as_slice(),
        b"Content-Length: 4\r\nConnection: close\r\n\r\nbody",
    ]
    .concat();
    let (url, server) = server(vec![
        initial,
        reply(304, &[("Cache-Control", "max-age=600")], b""),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, false).await?;
    let validated = fetch(
        &client,
        request(&url)?.with_cache_mode(RequestCacheMode::Validate),
        true,
    )
    .await?
    .0;
    assert_eq!(validated.status, 200);
    assert_eq!(validated.body_bytes(), b"body");
    assert_eq!(validated.cache_state, ResponseCacheState::Validated);
    let requests = server.await??;
    assert_eq!(
        header(&requests[1], "if-none-match"),
        Some(b"\"\xff\xe9\"".as_slice())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_revalidates_cors_redirect_hops_without_reusing_tainted_responses()
-> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let origin = Url::parse(&format!("http://127.0.0.1:{port}/"))?;
    let start = Url::parse(&format!("http://localhost:{port}/start"))?;
    let target = origin.join("target")?;
    let redirect = reply(
        302,
        &[
            (
                "Access-Control-Allow-Origin",
                origin.origin().ascii_serialization().as_str(),
            ),
            ("Location", target.as_str()),
        ],
        b"",
    );
    let original = reply(
        200,
        &[
            ("Access-Control-Allow-Origin", "null"),
            ("Cache-Control", "max-age=600"),
            ("ETag", "\"redirected\""),
            ("Vary", "Origin"),
        ],
        b"cors body",
    );
    let validation = reply(
        304,
        &[
            ("Access-Control-Allow-Origin", "null"),
            ("Cache-Control", "max-age=600"),
        ],
        b"",
    );
    let direct = reply(200, &[("Cache-Control", "max-age=600")], b"direct body");
    let server = tokio::spawn(serve(
        listener,
        vec![
            redirect.clone(),
            original,
            redirect.clone(),
            redirect,
            validation,
            direct,
        ],
    ));
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let request = |url: Url, mode| {
        Request::new_browser("GET", url, None, Vec::new(), (&origin).into())
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
            .with_page_network_policy()
            .with_cache_mode(mode)
    };
    for (mode, expected) in [
        (RequestCacheMode::Default, ResponseCacheState::None),
        (RequestCacheMode::Default, ResponseCacheState::Local),
        (RequestCacheMode::Validate, ResponseCacheState::Validated),
    ] {
        let response = crate::network_host::fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata(&client, request(start.clone(), mode), None, Vec::new()).await.map_err(anyhow::Error::msg)?;
        let (stream, journal) = response.into_parts_with_observation_journal();
        let raw = stream.into_materialized_raw_response().await?;
        assert_eq!(raw.status, 200);
        assert_eq!(raw.body_bytes(), b"cors body");
        assert_eq!(raw.cache_state, expected);
        assert!(raw.redirected);
        assert_eq!(raw.redirect_chain.len(), 1);
        assert_eq!(raw.redirect_chain[0].from_url, start);
        if expected == ResponseCacheState::Validated {
            assert_eq!(journal.final_response_observation().unwrap().status(), 304);
        }
    }
    let direct = fetch(&client, request(target, RequestCacheMode::Default), false)
        .await?
        .0;
    assert_eq!(direct.body_bytes(), b"direct body");
    assert_eq!(direct.cache_state, ResponseCacheState::None);
    let requests = server.await??;
    assert_eq!(requests.len(), 6);
    assert_eq!(header(&requests[4], "origin"), Some(b"null".as_slice()));
    assert_eq!(
        header(&requests[4], "if-none-match"),
        Some(b"\"redirected\"".as_slice())
    );
    assert_eq!(header(&requests[5], "if-none-match"), None);
    assert!(
        requests.iter().all(|head| !head.starts_with(b"OPTIONS ")),
        "cache-owned validators must not trigger author preflight"
    );
    Ok(())
}

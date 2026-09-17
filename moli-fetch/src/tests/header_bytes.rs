use super::*;
use crate::network_fetch_result::NetworkObservationRecorder;

async fn serve_responses(listener: TcpListener, responses: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
    let mut requests = Vec::new();
    for response in responses {
        let (mut stream, _) =
            tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
        let mut request = Vec::new();
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let count =
                tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await??;
            anyhow::ensure!(
                count != 0 && request.len() < 64 * 1024,
                "incomplete request"
            );
            request.extend_from_slice(&chunk[..count]);
        }
        requests.push(request);
        stream.write_all(&response).await?;
        stream.shutdown().await?;
    }
    Ok(requests)
}

fn response(headers: &[u8]) -> Vec<u8> {
    [
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n".as_slice(),
        headers,
        b"\r\n\x00\xff\x80",
    ]
    .concat()
}

async fn fetch_in_mode(
    client: &FetchClientHandle,
    request: Request,
    mode: &str,
) -> Result<(ResponseHead, Vec<u8>)> {
    let (head, body) = match mode {
        "buffered" => client
            .fetch(request.with_follow_redirects(false))
            .await?
            .into_body(),
        "html" => client.fetch_html_stream(request).await?.into_body(),
        "raw" => client
            .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
            .await?
            .into_body(),
        _ => unreachable!(),
    };
    Ok((head, body.into_materialized_bytes().await?))
}

#[tokio::test]
async fn request_content_type_distinguishes_absent_empty_and_nonempty_in_all_transports()
-> Result<()> {
    let cases = [
        ("GET", None),
        ("POST", None),
        ("POST", Some("")),
        ("POST", Some("body")),
        ("PUT", None),
        ("PUT", Some("body")),
        ("PATCH", Some("body")),
        ("DELETE", Some("body")),
    ];
    for mode in ["buffered", "html", "raw"] {
        for content_type in [Some(""), None, Some("application/custom")] {
            let server = ScriptedHttpServer::spawn(
                cases.iter().map(|_| ScriptedResponse::ok("ok")).collect(),
            );
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            for (method, body) in cases {
                let mut headers = vec![("X-Test".to_owned(), String::new())];
                if let Some(value) = content_type {
                    headers.push(("cOnTeNt-TyPe".to_owned(), value.to_owned()));
                }
                let recorder = NetworkObservationRecorder::default();
                let request =
                    Request::new(method, &server.url(), body.map(str::to_owned), headers)?
                        .with_network_observation_recorder(recorder.clone());
                fetch_in_mode(&client, request, mode).await?;
                let journal = recorder.snapshot();
                let observed = journal.final_request_observation().unwrap().headers();
                assert_eq!(
                    observed
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                        .map(|(_, value)| value.as_str()),
                    content_type,
                    "{mode}, {method}, body={body:?}: {observed:?}"
                );
            }
            let requests = server.requests();
            server.shutdown();
            assert!(client.shutdown().is_clean());
            assert_eq!(requests.len(), cases.len());
            for (request, (method, body)) in requests.iter().zip(cases) {
                assert_eq!(
                    request_head_header_value(request, "content-type"),
                    content_type,
                    "{mode}, {method}, body={body:?}: {request}"
                );
                assert_eq!(request_head_header_value(request, "x-test"), Some(""));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn http_header_bytes_survive_all_transports_and_observation() -> Result<()> {
    let bytes: Vec<u8> = (0x80..=0xff).collect();
    let value: String = bytes.iter().copied().map(char::from).collect();
    let headers = [
        b"X-Bytes: \t".as_slice(),
        bytes.as_slice(),
        b" \t\r\n",
        b"X-Edges: \t\xa0value\x85 \t\r\n",
        b"X-Utf8: \xc3\xbf\xe4\xb8\xad\r\n",
        b"X-Empty: \t \r\nX-Repeat: first\r\nX-Repeat: second\r\n",
    ]
    .concat();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/bytes", listener.local_addr()?);
    let server = tokio::spawn(serve_responses(listener, vec![response(&headers); 3]));
    let mut config = FetchConfig::default();
    config.set_default_request_headers(vec![("X-Config".into(), "é中".into())]);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    for mode in ["buffered", "html", "raw"] {
        let recorder = NetworkObservationRecorder::default();
        let request = Request::new(
            "GET",
            &url,
            None,
            crate::RequestHeaders::from_bytes(vec![("X-Bytes".into(), bytes.clone())]),
        )?
        .with_network_observation_recorder(recorder.clone());
        let (head, body) = fetch_in_mode(&client, request, mode).await?;
        // The HTML stream intentionally decodes the body as text, independently
        // of header ByteStrings. The raw and buffered paths preserve its bytes.
        if mode != "html" {
            assert_eq!(body, [0, 255, 128], "{mode}");
        }
        let expected = [
            ("x-bytes".to_owned(), bytes.clone()),
            ("x-edges".to_owned(), b"\xa0value\x85".to_vec()),
            ("x-utf8".to_owned(), b"\xc3\xbf\xe4\xb8\xad".to_vec()),
            ("x-empty".to_owned(), Vec::new()),
            ("x-repeat".to_owned(), b"first".to_vec()),
            ("x-repeat".to_owned(), b"second".to_vec()),
        ];
        assert_eq!(
            head.headers
                .iter()
                .filter(|(n, _)| n.starts_with("x-"))
                .cloned()
                .collect::<Vec<_>>(),
            expected,
            "{mode}"
        );
        let journal = recorder.snapshot();
        let received = journal.final_response_observation().unwrap().headers();
        assert_eq!(
            received
                .iter()
                .filter(|(n, _)| n.starts_with("X-"))
                .map(|(n, v)| (n.to_ascii_lowercase(), v.clone()))
                .collect::<Vec<_>>(),
            expected,
            "{mode}"
        );
        let sent = journal.final_request_observation().unwrap().headers();
        assert!(
            sent.contains(&("X-Bytes".into(), value.clone())),
            "{mode}: {sent:?}"
        );
    }
    let expected_line = [b"X-Bytes: ".as_slice(), bytes.as_slice(), b"\r\n"].concat();
    for request in server.await?? {
        assert!(
            request
                .windows("X-Config: é中\r\n".len())
                .any(|part| part == "X-Config: é中\r\n".as_bytes())
        );
        assert!(
            request
                .windows(expected_line.len())
                .any(|part| part == expected_line)
        );
    }
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn ordinary_request_header_strings_keep_utf8_encoding() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/utf8", listener.local_addr()?);
    let server = tokio::spawn(serve_responses(listener, vec![response(b""); 3]));
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for mode in ["buffered", "html", "raw"] {
        let request = Request::new("GET", &url, None, vec![("X-Utf8".into(), "é中".into())])?;
        fetch_in_mode(&client, request, mode).await?;
    }
    for request in server.await?? {
        assert!(
            request
                .windows("X-Utf8: é中\r\n".len())
                .any(|part| part == "X-Utf8: é中\r\n".as_bytes())
        );
    }
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn http_header_bytes_survive_redirect_handle_reuse() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/start", listener.local_addr()?);
    let redirect = b"HTTP/1.1 307 Redirect\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let server = tokio::spawn(serve_responses(
        listener,
        vec![redirect, response(b"X-Bytes: \xff\r\n")],
    ));
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut headers = crate::RequestHeaders::from(vec![("X-Utf8".into(), "é中".into())]);
    headers.push(("X-Bytes".into(), vec![0xff]));
    let request = Request::new("GET", &url, None, headers)?;
    let (head, body) = fetch_in_mode(&client, request, "raw").await?;
    assert!(head.redirected);
    assert_eq!(head.final_url.path(), "/final");
    assert_eq!(body, [0, 255, 128]);
    assert!(head.headers.contains(&("x-bytes".into(), b"\xff".to_vec())));
    let requests = server.await??;
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert!(
            request
                .windows("X-Utf8: é中\r\n".len())
                .any(|part| part == "X-Utf8: é中\r\n".as_bytes())
        );
        assert!(
            request
                .windows(b"X-Bytes: \xff\r\n".len())
                .any(|part| part == b"X-Bytes: \xff\r\n")
        );
    }
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn http_header_bytes_survive_disk_cache_reopen() -> Result<()> {
    for first_mode in ["html", "raw"] {
        let cache_dir = unique_test_cache_dir();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/cache", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(
            listener,
            vec![response(
                b"Cache-Control: max-age=60\r\nX-Bytes: \xa0\xc3\xbf\x85\r\n",
            )],
        ));
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        for (index, mode) in [first_mode, "html", "raw"].into_iter().enumerate() {
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let (head, _) = fetch_in_mode(&client, Request::get(&url)?, mode).await?;
            assert_eq!(head.from_cache, index != 0, "{first_mode} to {mode}");
            assert!(
                head.headers
                    .contains(&("x-bytes".into(), b"\xa0\xc3\xbf\x85".to_vec()))
            );
            assert!(client.shutdown().is_clean());
        }
        assert_eq!(server.await??.len(), 1);
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn current_hop_headers_expire_before_redirect_method_rewrite_in_all_transports() -> Result<()>
{
    for status in [302, 307] {
        for mode in ["buffered", "html", "raw"] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/start", listener.local_addr()?);
            let redirect = format!(
                "HTTP/1.1 {status} Redirect\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ).into_bytes();
            let server = tokio::spawn(serve_responses(listener, vec![redirect, response(b"")]));
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let original = crate::RequestHeaders::from_bytes(vec![
                ("X-Raw".into(), vec![0xe9, 0xff]),
                ("Content-Type".into(), b"text/plain".to_vec()),
            ]);
            let request = Request::new(
                "POST",
                &url,
                Some("body".into()),
                vec![
                    ("X-Raw".into(), "ÿ".into()),
                    ("Content-Type".into(), "application/json".into()),
                ],
            )?
            .with_redirect_headers(Some(original));
            let (head, body) = match mode {
                "buffered" => client.fetch(request).await?.into_body(),
                "html" => client.fetch_html_stream(request).await?.into_body(),
                "raw" => client
                    .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                    .await?
                    .into_body(),
                _ => unreachable!(),
            };
            body.into_materialized_bytes().await?;
            assert!(head.redirected, "{mode} {status}");
            let requests = server.await??;
            assert_eq!(requests.len(), 2, "{mode} {status}");
            assert!(
                requests[0]
                    .windows(b"X-Raw: \xc3\xbf\r\n".len())
                    .any(|value| value == b"X-Raw: \xc3\xbf\r\n"),
                "{mode} {status}"
            );
            assert!(
                requests[1]
                    .windows(b"X-Raw: \xe9\xff\r\n".len())
                    .any(|value| value == b"X-Raw: \xe9\xff\r\n"),
                "{mode} {status}"
            );
            if status == 302 {
                assert!(requests[1].starts_with(b"GET /final "), "{mode}");
                assert!(
                    !requests[1]
                        .windows(b"Content-Type:".len())
                        .any(|value| value.eq_ignore_ascii_case(b"Content-Type:")),
                    "{mode}"
                );
            } else {
                assert!(requests[1].starts_with(b"POST /final "), "{mode}");
                assert!(
                    requests[1]
                        .windows(b"Content-Type: text/plain\r\n".len())
                        .any(|value| value.eq_ignore_ascii_case(b"Content-Type: text/plain\r\n")),
                    "{mode}"
                );
            }
            assert!(client.shutdown().is_clean());
        }
    }
    Ok(())
}

fn accept_encoding_values(request: &[u8]) -> Vec<&str> {
    std::str::from_utf8(request)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("accept-encoding")
                .then_some(value.trim())
        })
        .collect()
}

#[tokio::test]
async fn range_requests_select_identity_in_every_transport() -> Result<()> {
    // Presence, including an empty or malformed value, selects identity.
    // Ordinary requests before and after must still negotiate compression.
    let ranges = [
        None,
        Some("bytes=0-10"),
        Some("foo=0-10"),
        Some("foo"),
        Some(""),
        Some("bytes=-3"),
        Some("bytes=0-1,4-5"),
        None,
    ];
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/range", listener.local_addr()?);
    let server = tokio::spawn(serve_responses(
        listener,
        vec![response(b""); ranges.len() * 3],
    ));
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for mode in ["buffered", "html", "raw"] {
        for range in ranges {
            let headers = range
                .map(|value| vec![("rAnGe".into(), value.into())])
                .unwrap_or_default();
            let recorder = NetworkObservationRecorder::default();
            let request = Request::new("GET", &url, None, headers)?
                .with_network_observation_recorder(recorder.clone());
            fetch_in_mode(&client, request.clone(), mode).await?;
            assert!(
                request
                    .request_headers
                    .iter()
                    .all(|(name, _)| !name.eq_ignore_ascii_case("accept-encoding"))
            );
            if range.is_some() {
                let journal = recorder.snapshot();
                assert!(
                    journal
                        .final_request_observation()
                        .unwrap()
                        .headers()
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("accept-encoding")
                            && value == "identity"),
                    "{mode}: {range:?}"
                );
            }
        }
    }
    let requests = server.await??;
    assert_eq!(requests.len(), ranges.len() * 3);
    for (request, range) in requests.iter().zip(ranges.into_iter().cycle()) {
        let values = accept_encoding_values(request);
        if range.is_some() {
            assert_eq!(values, ["identity"], "{range:?}");
        } else {
            assert_eq!(values.len(), 1);
            assert!(values[0].contains("gzip"), "{values:?}");
        }
    }
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn range_requests_respect_native_header_configuration() -> Result<()> {
    for (defaults, headers, expected) in [
        (vec![("Range", "bytes=0-10")], vec![], "identity"),
        (
            vec![("aCcEpT-EnCoDiNg", "gzip")],
            vec![("Range", "bytes=0-10")],
            "gzip",
        ),
        (
            vec![("Range", "bytes=0-10")],
            vec![("Accept-Encoding", "br")],
            "br",
        ),
        (
            vec![("Range", "bytes=0-10")],
            vec![("Accept-Encoding", "")],
            "",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/configured", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(listener, vec![response(b""); 3]));
        let mut config = FetchConfig::default();
        for (name, value) in defaults {
            config.push_default_request_header(name, value);
        }
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        for mode in ["buffered", "html", "raw"] {
            let request = Request::new(
                "GET",
                &url,
                None,
                headers
                    .iter()
                    .map(|(n, v)| (n.to_string(), v.to_string()))
                    .collect(),
            )?;
            fetch_in_mode(&client, request, mode).await?;
        }
        for request in server.await?? {
            assert_eq!(accept_encoding_values(&request), [expected]);
        }
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn range_redirects_keep_identity_and_response_decompression() -> Result<()> {
    // A native HTTP caller still receives decoded bytes if a server ignores
    // the requested encoding and sends a complete gzip response.
    let gzip = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff\x4b\x49\x4d\xce\x4f\x49\x4d\x01\x00\xf6\x9a\xf0\x1a\x07\x00\x00\x00";
    let compressed = [
        format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", gzip.len()).as_bytes(),
        gzip,
    ].concat();
    let redirect = b"HTTP/1.1 307 Redirect\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    for mode in ["html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/start", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(
            listener,
            vec![redirect.clone(), compressed.clone(), compressed.clone()],
        ));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let request = Request::new(
            "GET",
            &url,
            None,
            vec![("Range".into(), "bytes=0-10".into())],
        )?;
        let (head, body) = fetch_in_mode(&client, request, mode).await?;
        assert!(head.redirected);
        assert_eq!(head.final_url.path(), "/final");
        assert_eq!(body, b"decoded");
        let (_, body) = fetch_in_mode(&client, Request::get(&url)?, mode).await?;
        assert_eq!(body, b"decoded");
        let requests = server.await??;
        assert_eq!(requests.len(), 3);
        for request in &requests[..2] {
            assert_eq!(accept_encoding_values(request), ["identity"]);
        }
        assert!(accept_encoding_values(&requests[2])[0].contains("gzip"));
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

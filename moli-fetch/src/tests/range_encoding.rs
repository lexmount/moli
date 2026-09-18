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
                    .collect::<Vec<_>>(),
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

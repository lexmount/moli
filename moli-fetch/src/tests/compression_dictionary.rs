use super::*;

const DICTIONARY: &[u8] = b"This is a test dictionary.\n";
const PAYLOAD: &[u8] = b"This is compressed test data using a test dictionary";
const HASH: &str = ":U5abz16WDg7b8KS93msLPpOB4Vbef1uRzoORYkJw9BY=:";
const DCB: &[u8] = include_bytes!("../../tests/fixtures/compression-dictionary/payload.dcb");
const DCZ: &[u8] = include_bytes!("../../tests/fixtures/compression-dictionary/payload.dcz");

fn reply(headers: &[(&str, &str)], body: &[u8], chunked: bool) -> Vec<u8> {
    let mut head =
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\n".to_owned();
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if chunked {
        head.push_str("Transfer-Encoding: chunked\r\n\r\n");
    } else {
        head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    }
    let mut response = head.into_bytes();
    if chunked {
        for byte in body {
            response.extend_from_slice(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n']);
        }
        response.extend_from_slice(b"0\r\nX-Trailer: retained\r\n\r\n");
    } else {
        response.extend_from_slice(body);
    }
    response
}

async fn server(
    responses: Vec<Vec<u8>>,
) -> Result<(String, tokio::task::JoinHandle<Result<Vec<Vec<u8>>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/data", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 4096];
                let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
                    .await??;
                anyhow::ensure!(count != 0 && request.len() < 65536, "incomplete request");
                request.extend_from_slice(&buffer[..count]);
            }
            requests.push(request);
            // Error responses may make the client close the socket early.
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        }
        Ok(requests)
    });
    Ok((url, task))
}

fn header<'a>(request: &'a [u8], name: &str) -> Option<&'a str> {
    std::str::from_utf8(request)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

async fn fetch(client: &FetchClientHandle, url: &str, mode: &str) -> Result<Vec<u8>> {
    let request = Request::new("GET", url, None, crate::RequestHeaders::default())?;
    let (_, body) = match mode {
        "buffered" => client
            .fetch(request.with_auth(crate::RequestAuth {
                target: crate::RequestAuthTarget::Server,
                scheme: crate::RequestAuthScheme::Digest,
                username: "user".to_owned(),
                password: "password".to_owned(),
            }))
            .await?
            .into_body(),
        "html" => client.fetch_html_stream(request).await?.into_body(),
        "raw" => client
            .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
            .await?
            .into_body(),
        _ => unreachable!(),
    };
    body.into_materialized_bytes().await
}

#[tokio::test]
async fn compression_dictionary_registration_and_decoding_cover_every_transport() -> Result<()> {
    for mode in ["raw", "html", "buffered"] {
        for chunked in [false, true] {
            let coding = [
                ("dcb", DCB, PAYLOAD),
                ("dcz", DCZ, PAYLOAD),
                (
                    "gzip",
                    include_bytes!("../../tests/fixtures/compression-dictionary/dictionary.gzip")
                        .as_slice(),
                    DICTIONARY,
                ),
                (
                    "deflate",
                    include_bytes!(
                        "../../tests/fixtures/compression-dictionary/dictionary.deflate"
                    )
                    .as_slice(),
                    DICTIONARY,
                ),
                (
                    "deflate",
                    include_bytes!(
                        "../../tests/fixtures/compression-dictionary/dictionary.raw-deflate"
                    )
                    .as_slice(),
                    DICTIONARY,
                ),
                (
                    "br",
                    include_bytes!("../../tests/fixtures/compression-dictionary/dictionary.br")
                        .as_slice(),
                    DICTIONARY,
                ),
                (
                    "zstd",
                    include_bytes!("../../tests/fixtures/compression-dictionary/dictionary.zstd")
                        .as_slice(),
                    DICTIONARY,
                ),
            ];
            let mut responses = vec![reply(
                &[
                    ("Cache-Control", "max-age=600"),
                    ("Use-As-Dictionary", "match=\"/*\", id=\"first\""),
                ],
                DICTIONARY,
                chunked,
            )];
            responses.extend(
                coding
                    .iter()
                    .map(|(name, body, _)| reply(&[("Content-Encoding", name)], body, chunked)),
            );
            let (url, task) = server(responses).await?;
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            assert_eq!(
                fetch(&client, &url, mode).await?,
                DICTIONARY,
                "{mode}/{chunked}"
            );
            for (name, _, expected) in coding {
                assert_eq!(
                    fetch(&client, &url, mode).await?,
                    expected,
                    "{mode}/{chunked}/{name}"
                );
            }
            let requests = task.await??;
            assert_eq!(header(&requests[0], "available-dictionary"), None);
            assert!(
                !header(&requests[0], "accept-encoding")
                    .unwrap()
                    .contains("dcb")
            );
            for request in &requests[1..] {
                assert_eq!(header(request, "available-dictionary"), Some(HASH));
                assert_eq!(header(request, "dictionary-id"), Some("\"first\""));
                let encoding = header(request, "accept-encoding").unwrap();
                assert!(encoding.contains("dcb") && encoding.contains("dcz"));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_clear_site_data_observes_cache_and_cookie_directives() -> Result<()>
{
    let registered = reply(
        &[
            ("Cache-Control", "max-age=600"),
            ("Use-As-Dictionary", "match=\"/*\""),
        ],
        DICTIONARY,
        false,
    );
    for directive in ["storage", "cache", "cookies", "*"] {
        let field = format!("\"{directive}\"");
        let (url, task) = server(vec![
            registered.clone(),
            reply(&[("Clear-Site-Data", &field)], b"OK", false),
            reply(&[], b"OK", false),
        ])
        .await?;
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        for _ in 0..3 {
            fetch(&client, &url, "raw").await?;
        }
        let requests = task.await??;
        assert_eq!(header(&requests[1], "available-dictionary"), Some(HASH));
        assert_eq!(
            header(&requests[2], "available-dictionary"),
            if directive == "storage" {
                Some(HASH)
            } else {
                None
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_bad_hash_and_truncated_streams_do_not_register() -> Result<()> {
    let registered = reply(
        &[
            ("Cache-Control", "max-age=600"),
            ("Use-As-Dictionary", "match=\"/*\", id=\"good\""),
        ],
        DICTIONARY,
        false,
    );
    for (coding, original) in [("dcb", DCB), ("dcz", DCZ)] {
        let mut bad_hash = original.to_vec();
        bad_hash[12] ^= 1;
        for body in [bad_hash.as_slice(), &original[..original.len() - 1]] {
            let invalid = reply(
                &[
                    ("Content-Encoding", coding),
                    ("Cache-Control", "max-age=600"),
                    ("Use-As-Dictionary", "match=\"/*\", id=\"bad\""),
                ],
                body,
                true,
            );
            let (url, task) =
                server(vec![registered.clone(), invalid, reply(&[], b"OK", false)]).await?;
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            fetch(&client, &url, "raw").await?;
            assert!(fetch(&client, &url, "raw").await.is_err(), "{coding}");
            fetch(&client, &url, "raw").await?;
            assert_eq!(header(&task.await??[2], "dictionary-id"), Some("\"good\""));
        }
    }
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_request_snapshot_survives_later_registration() -> Result<()> {
    let (url, task) = server(vec![
        reply(
            &[
                ("Cache-Control", "max-age=600"),
                ("Use-As-Dictionary", "match=\"/*\""),
            ],
            DICTIONARY,
            false,
        ),
        reply(&[], b"OK", false),
        reply(&[], b"OK", false),
        reply(&[], b"OK", false),
    ])
    .await?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let early = client.prepare_request(Request::get(&url)?);
    fetch(&client, &url, "raw").await?;
    assert!(
        !client
            .cache_request_headers(&early)
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("available-dictionary"))
    );
    client.fetch_raw(early.clone()).await?;
    fetch(&client, &url, "raw").await?;
    client
        .fetch_raw(early.with_network_partition_key(Some("https://other.test".to_owned())))
        .await?;
    let requests = task.await??;
    assert_eq!(header(&requests[1], "available-dictionary"), None);
    assert_eq!(header(&requests[2], "available-dictionary"), Some(HASH));
    assert_eq!(header(&requests[3], "available-dictionary"), None);
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_cached_body_registration_keeps_original_freshness() -> Result<()> {
    let (url, task) = server(vec![
        reply(
            &[
                ("Cache-Control", "max-age=600"),
                ("Use-As-Dictionary", "match=\"/*\""),
            ],
            DICTIONARY,
            false,
        ),
        reply(&[("Clear-Site-Data", "\"cache\"")], b"OK", false),
        reply(&[], b"OK", false),
        reply(&[("Clear-Site-Data", "\"cache\"")], b"OK", false),
        reply(&[], b"OK", false),
    ])
    .await?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let request = Request::get(&url)?;
    let cached = client.fetch_raw(request.clone()).await?;
    fetch(&client, &url, "raw").await?;
    client.observe_cached_response(&request, &cached, moli_http_cache::unix_now_ms() + 600_000);
    fetch(&client, &url, "raw").await?;
    fetch(&client, &url, "raw").await?;
    client.observe_cached_response(
        &request,
        &cached,
        moli_http_cache::unix_now_ms().saturating_sub(1),
    );
    fetch(&client, &url, "raw").await?;
    let requests = task.await??;
    assert_eq!(header(&requests[2], "available-dictionary"), Some(HASH));
    assert_eq!(header(&requests[4], "available-dictionary"), None);
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_disk_cache_hit_registers_decoded_dictionary() -> Result<()> {
    let (url, task) = server(vec![
        reply(
            &[
                ("Cache-Control", "max-age=600"),
                ("Use-As-Dictionary", "match=\"/*\""),
            ],
            DICTIONARY,
            false,
        ),
        reply(&[], b"probe", false),
    ])
    .await?;
    let cache_dir = unique_test_cache_dir();
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let request = Request::get(&url)?;
    {
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        assert_eq!(
            client.fetch_raw(request.clone()).await?.body_bytes(),
            DICTIONARY
        );
    }
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let hit = client.fetch_raw(request).await?;
    assert_eq!(hit.cache_state, crate::ResponseCacheState::Local);
    let probe = format!("{url}?probe");
    client.fetch_raw(Request::get(&probe)?).await?;
    let requests = task.await??;
    assert_eq!(requests.len(), 2);
    assert_eq!(header(&requests[1], "available-dictionary"), Some(HASH));
    fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn compression_dictionary_registration_requires_readable_and_fresh_responses() -> Result<()> {
    let cases = [
        ("match=\"/*\"", "max-age=600", "0", true),
        ("match=42", "max-age=600", "0", false),
        ("match=\"/:id(d.*)\"", "max-age=600", "0", false),
        ("match=\"/*\", type=other", "max-age=600", "0", false),
        (
            "match=\"/*\", match-dest=\"script\"",
            "max-age=600",
            "0",
            false,
        ),
        ("match=\"/*\"", "max-age=600", "601", false),
        ("match=\"/*\"", "no-store, max-age=600", "0", false),
    ];
    for (options, cache, age, expected) in cases {
        let (url, task) = server(vec![
            reply(
                &[
                    ("Use-As-Dictionary", options),
                    ("Cache-Control", cache),
                    ("Age", age),
                ],
                DICTIONARY,
                false,
            ),
            reply(&[], b"OK", false),
        ])
        .await?;
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        fetch(&client, &url, "raw").await?;
        fetch(&client, &url, "raw").await?;
        assert_eq!(
            header(&task.await??[1], "available-dictionary"),
            expected.then_some(HASH),
            "{options}/{cache}/{age}"
        );
    }
    for (mode, cors, expected) in [
        (crate::RequestMode::NoCors, true, false),
        (crate::RequestMode::Cors, false, false),
        (crate::RequestMode::Cors, true, true),
    ] {
        let mut headers = vec![
            ("Use-As-Dictionary", "match=\"/*\""),
            ("Cache-Control", "max-age=600"),
        ];
        if cors {
            headers.push(("Access-Control-Allow-Origin", "*"));
        }
        let (url, task) = server(vec![
            reply(&headers, DICTIONARY, false),
            reply(&[("Access-Control-Allow-Origin", "*")], b"OK", false),
        ])
        .await?;
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let mut request = Request::get(&url)?
            .with_request_origin(moli_url::WebOrigin::from_serialized("https://frame.test"));
        request.request_mode = mode;
        request.credentials_mode = crate::RequestCredentialsMode::Omit;
        let _ = client.fetch_raw(request.clone()).await;
        request.request_mode = crate::RequestMode::Cors;
        client.fetch_raw(request).await?;
        assert_eq!(
            header(&task.await??[1], "available-dictionary"),
            expected.then_some(HASH),
            "{mode:?}/{cors}"
        );
    }
    Ok(())
}

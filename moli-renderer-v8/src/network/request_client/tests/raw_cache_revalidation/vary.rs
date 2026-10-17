use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_selects_overlapping_variants_by_response_date() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=2147483648"),
                ("Vary", "Foo"),
                ("Date", "Wed, 21 Oct 2015 07:28:00 GMT"),
            ],
            b"newer generation",
        ),
        reply(
            200,
            &[
                ("Cache-Control", "max-age=2147483648"),
                ("Vary", "Bar"),
                ("Date", "Tue, 20 Oct 2015 07:28:00 GMT"),
            ],
            b"older generation received later",
        ),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(
        &client,
        variant_request(&url, &[("Foo", b"1"), ("Bar", b"1")])?,
        false,
    )
    .await?;
    fetch(
        &client,
        variant_request(&url, &[("Foo", b"2"), ("Bar", b"2")])?,
        false,
    )
    .await?;
    let selected = fetch(
        &client,
        variant_request(&url, &[("Foo", b"1"), ("Bar", b"2")])?,
        false,
    )
    .await?
    .0;
    assert_eq!(selected.body_bytes(), b"newer generation");
    assert_eq!(selected.cache_state, ResponseCacheState::Local);
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_validation_does_not_resurrect_overlapping_variants() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Foo"),
                ("ETag", "\"one\""),
            ],
            b"one",
        ),
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Bar"),
                ("ETag", "\"two\""),
            ],
            b"two",
        ),
        reply(304, &[("Cache-Control", "no-store")], b""),
        reply(200, &[("Cache-Control", "max-age=600")], b"new body"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(
        &client,
        variant_request(&url, &[("Foo", b"1"), ("Bar", b"1")])?,
        false,
    )
    .await?;
    fetch(
        &client,
        variant_request(&url, &[("Foo", b"2"), ("Bar", b"2")])?,
        false,
    )
    .await?;
    let overlap = || variant_request(&url, &[("Foo", b"1"), ("Bar", b"2")]);
    let validated = fetch(
        &client,
        overlap()?.with_cache_mode(RequestCacheMode::Validate),
        false,
    )
    .await?
    .0;
    assert_eq!(validated.body_bytes(), b"two");
    assert_eq!(validated.cache_state, ResponseCacheState::Validated);
    let replaced = fetch(&client, overlap()?, false).await?.0;
    assert_eq!(replaced.body_bytes(), b"new body");
    assert_eq!(replaced.cache_state, ResponseCacheState::None);
    let requests = server.await??;
    assert_eq!(
        header(&requests[2], "if-none-match"),
        Some(b"\"two\"".as_slice())
    );
    assert_eq!(header(&requests[3], "if-none-match"), None);
    Ok(())
}

fn variant_request(url: &str, fields: &[(&str, &[u8])]) -> Result<Request> {
    let mut request = request(url)?;
    request.request_headers = moli_fetch::RequestHeaders::from_bytes(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.to_vec()))
            .collect(),
    );
    Ok(request)
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_ignores_unvaried_author_headers() -> Result<()> {
    let (url, server) = server(vec![reply(
        200,
        &[("Cache-Control", "max-age=600")],
        b"shared representation",
    )])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(
        &client,
        variant_request(&url, &[("X-Client", b"one")])?,
        false,
    )
    .await?;
    let cached = fetch(
        &client,
        variant_request(&url, &[("X-Client", b"two"), ("Other", b"new")])?,
        false,
    )
    .await?
    .0;
    assert_eq!(cached.cache_state, ResponseCacheState::Local);
    assert_eq!(cached.body_bytes(), b"shared representation");
    assert_eq!(server.await??.len(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_matches_only_named_vary_fields() -> Result<()> {
    let headers = [("Cache-Control", "max-age=600"), ("Vary", "Foo")];
    let (url, server) = server(vec![
        reply(200, &headers, b"one"),
        reply(200, &headers, b"two"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    for (foo, other, body, cache) in [
        ("1", "old", b"one".as_slice(), ResponseCacheState::None),
        ("1", "new", b"one".as_slice(), ResponseCacheState::Local),
        ("2", "another", b"two".as_slice(), ResponseCacheState::None),
        ("1", "last", b"one".as_slice(), ResponseCacheState::Local),
    ] {
        let response = fetch(
            &client,
            variant_request(
                &url,
                &[("fOo", foo.as_bytes()), ("Other", other.as_bytes())],
            )?,
            true,
        )
        .await?
        .0;
        assert_eq!(response.body_bytes(), body);
        assert_eq!(response.cache_state, cache);
    }
    let requests = server.await??;
    assert_eq!(requests.len(), 2);
    assert_eq!(header(&requests[0], "foo"), Some(b"1".as_slice()));
    assert_eq!(header(&requests[1], "foo"), Some(b"2".as_slice()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_vary_preserves_absence_empty_duplicates_and_byte_values() -> Result<()> {
    let headers = [("Cache-Control", "max-age=600"), ("Vary", "X-Mode")];
    let (url, server) = server(vec![
        reply(200, &headers, b"missing"),
        reply(200, &headers, b"empty"),
        reply(200, &headers, b"multiple"),
        reply(200, &headers, b"bytes"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let cases: &[(&[(&str, &[u8])], &[u8], ResponseCacheState)] = &[
        (&[], b"missing", ResponseCacheState::None),
        (&[("X-Mode", b"")], b"empty", ResponseCacheState::None),
        (
            &[("X-Mode", b"alpha"), ("x-mode", b"beta")],
            b"multiple",
            ResponseCacheState::None,
        ),
        (
            &[("x-MODE", b"alpha, beta")],
            b"multiple",
            ResponseCacheState::Local,
        ),
        (
            &[("X-Mode", b"\xff\xe9")],
            b"bytes",
            ResponseCacheState::None,
        ),
        (&[], b"missing", ResponseCacheState::Local),
        (&[("x-mode", b"")], b"empty", ResponseCacheState::Local),
        (
            &[("x-MoDe", b"\xff\xe9")],
            b"bytes",
            ResponseCacheState::Local,
        ),
    ];
    for (fields, body, state) in cases {
        let response = fetch(&client, variant_request(&url, fields)?, false)
            .await?
            .0;
        assert_eq!(response.body_bytes(), *body);
        assert_eq!(response.cache_state, *state);
    }
    let requests = server.await??;
    assert_eq!(header(&requests[0], "x-mode"), None);
    assert_eq!(header(&requests[1], "x-mode"), Some(b"".as_slice()));
    assert_eq!(header(&requests[3], "x-mode"), Some(b"\xff\xe9".as_slice()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_revalidation_updates_vary_without_losing_other_variants() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Foo"),
                ("ETag", "\"one\""),
            ],
            b"one",
        ),
        reply(
            200,
            &[
                ("Cache-Control", "max-age=600"),
                ("Vary", "Foo"),
                ("ETag", "\"two\""),
            ],
            b"two",
        ),
        reply(
            304,
            &[("Cache-Control", "max-age=600"), ("Vary", "Foo, Bar")],
            b"",
        ),
        reply(
            200,
            &[("Cache-Control", "max-age=600"), ("Vary", "Foo, Bar")],
            b"one-b",
        ),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let one_a = || variant_request(&url, &[("Foo", b"1"), ("Bar", b"a")]);
    let two = || variant_request(&url, &[("Foo", b"2")]);
    fetch(&client, one_a()?, true).await?;
    fetch(&client, two()?, true).await?;
    let validated = fetch(
        &client,
        one_a()?.with_cache_mode(RequestCacheMode::Validate),
        true,
    )
    .await?;
    assert_eq!(validated.0.cache_state, ResponseCacheState::Validated);
    assert_eq!(validated.0.body_bytes(), b"one");
    assert_eq!(
        validated.1.final_response_observation().unwrap().status(),
        304
    );
    let one_b = fetch(
        &client,
        variant_request(&url, &[("Foo", b"1"), ("Bar", b"b")])?,
        true,
    )
    .await?
    .0;
    assert_eq!(one_b.body_bytes(), b"one-b");
    assert_eq!(one_b.cache_state, ResponseCacheState::None);
    let retained_two = fetch(&client, two()?, false).await?.0;
    assert_eq!(retained_two.body_bytes(), b"two");
    assert_eq!(retained_two.cache_state, ResponseCacheState::Local);
    let retained_one = fetch(&client, one_a()?, false).await?.0;
    assert_eq!(retained_one.body_bytes(), b"one");
    assert_eq!(retained_one.cache_state, ResponseCacheState::Local);
    let requests = server.await??;
    assert_eq!(
        header(&requests[2], "if-none-match"),
        Some(b"\"one\"".as_slice())
    );
    assert_eq!(header(&requests[3], "if-none-match"), None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_non_storable_validation_preserves_unrelated_variants() -> Result<()> {
    for control in [
        ("Cache-Control", "no-store"),
        ("Vary", "*"),
        ("Vary", "Cookie"),
    ] {
        let headers = [
            ("Cache-Control", "max-age=600"),
            ("Vary", "Foo"),
            ("ETag", "\"one\""),
        ];
        let (url, server) = server(vec![
            reply(200, &headers, b"one"),
            reply(200, &headers, b"two"),
            reply(304, &[control], b""),
            reply(200, &headers, b"new one"),
        ])
        .await?;
        let client = ResourceRequestClient::new(&FetchConfig::default())?;
        let variant = |value| variant_request(&url, &[("Foo", value)]);
        fetch(&client, variant(b"1")?, false).await?;
        fetch(&client, variant(b"2")?, false).await?;
        let validated = fetch(
            &client,
            variant(b"1")?.with_cache_mode(RequestCacheMode::Validate),
            false,
        )
        .await?
        .0;
        assert_eq!(validated.body_bytes(), b"one");
        assert_eq!(validated.cache_state, ResponseCacheState::Validated);
        let other = fetch(&client, variant(b"2")?, false).await?.0;
        assert_eq!(other.body_bytes(), b"two");
        assert_eq!(other.cache_state, ResponseCacheState::Local);
        let replaced = fetch(&client, variant(b"1")?, false).await?.0;
        assert_eq!(replaced.body_bytes(), b"new one");
        assert_eq!(replaced.cache_state, ResponseCacheState::None);
        assert_eq!(server.await??.len(), 4);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_author_sensitive_headers_bypass_without_replacing_public_entry()
-> Result<()> {
    let (url, server) = server(vec![
        reply(200, &[("Cache-Control", "max-age=600")], b"public"),
        reply(200, &[("Cache-Control", "max-age=600")], b"authorized"),
        reply(200, &[("Cache-Control", "max-age=600")], b"cookie"),
        reply(206, &[("Cache-Control", "max-age=600")], b"partial"),
        reply(412, &[], b"precondition"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    fetch(&client, request(&url)?, false).await?;
    for (name, value, body, status) in [
        (
            "Authorization",
            "Bearer private",
            b"authorized".as_slice(),
            200,
        ),
        ("Cookie", "private=value", b"cookie".as_slice(), 200),
        ("Range", "bytes=0-6", b"partial".as_slice(), 206),
        ("If-Match", "\"author\"", b"precondition".as_slice(), 412),
    ] {
        let response = fetch(
            &client,
            variant_request(&url, &[(name, value.as_bytes()), ("X-Client", b"custom")])?,
            true,
        )
        .await?
        .0;
        assert_eq!(response.status, status);
        assert_eq!(response.body_bytes(), body);
        assert_eq!(response.cache_state, ResponseCacheState::None);
    }
    let public = fetch(&client, request(&url)?, false).await?.0;
    assert_eq!(public.body_bytes(), b"public");
    assert_eq!(public.cache_state, ResponseCacheState::Local);
    assert_eq!(server.await??.len(), 5);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_vary_uses_configured_and_author_header_bytes() -> Result<()> {
    let headers = [("Cache-Control", "max-age=600"), ("Vary", "X-Profile")];
    let (url, server) = server(vec![
        reply(200, &headers, b"default"),
        reply(200, &headers, b"default plus author"),
    ])
    .await?;
    let mut config = FetchConfig::default();
    config.set_default_request_headers(vec![("X-Profile".into(), "configured".into())]);
    let client = ResourceRequestClient::new(&config)?;
    let plain = || variant_request(&url, &[("Other", b"ignored")]);
    let author = || variant_request(&url, &[("X-Profile", b"author")]);
    fetch(&client, plain()?, false).await?;
    fetch(&client, author()?, false).await?;
    let cached = fetch(&client, plain()?, false).await?.0;
    assert_eq!(cached.body_bytes(), b"default");
    assert_eq!(cached.cache_state, ResponseCacheState::Local);
    let cached = fetch(&client, author()?, false).await?.0;
    assert_eq!(cached.body_bytes(), b"default plus author");
    assert_eq!(cached.cache_state, ResponseCacheState::Local);
    let requests = server.await??;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        header(&requests[0], "x-profile"),
        Some(b"configured".as_slice())
    );
    assert_eq!(
        requests[1]
            .split(|byte| *byte == b'\n')
            .filter(|line| line.len() >= 10 && line[..10].eq_ignore_ascii_case(b"X-Profile:"))
            .count(),
        2
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_configured_sensitive_headers_remain_ineligible() -> Result<()> {
    for name in ["Authorization", "Cookie"] {
        let (url, server) = server(vec![
            reply(200, &[("Cache-Control", "max-age=600")], b"private one"),
            reply(200, &[("Cache-Control", "max-age=600")], b"private two"),
        ])
        .await?;
        let mut config = FetchConfig::default();
        config.set_default_request_headers(vec![(name.to_owned(), "private".into())]);
        let client = ResourceRequestClient::new(&config)?;
        let first = fetch(&client, request(&url)?, false).await?.0;
        let second = fetch(&client, request(&url)?, false).await?.0;
        assert_eq!(first.body_bytes(), b"private one");
        assert_eq!(second.body_bytes(), b"private two");
        assert_eq!(second.cache_state, ResponseCacheState::None);
        let requests = server.await??;
        assert!(
            requests
                .iter()
                .all(|head| header(head, name) == Some(b"private".as_slice()))
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_memory_cache_does_not_store_unsolicited_partial_representations() -> Result<()> {
    let (url, server) = server(vec![
        reply(
            206,
            &[
                ("Cache-Control", "max-age=600"),
                ("Content-Range", "bytes 0-2/10"),
            ],
            b"abc",
        ),
        reply(200, &[("Cache-Control", "max-age=600")], b"whole body"),
    ])
    .await?;
    let client = ResourceRequestClient::new(&FetchConfig::default())?;
    let first = fetch(
        &client,
        variant_request(&url, &[("X-Client", b"custom")])?,
        false,
    )
    .await?
    .0;
    assert_eq!(first.status, 206);
    let second = fetch(
        &client,
        variant_request(&url, &[("X-Client", b"custom")])?,
        false,
    )
    .await?
    .0;
    assert_eq!(second.status, 200);
    assert_eq!(second.body_bytes(), b"whole body");
    assert_eq!(second.cache_state, ResponseCacheState::None);
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

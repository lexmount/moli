use super::*;

fn fetch_variant(
    config: &FetchConfig,
    request: Request,
    raw: bool,
) -> Result<(String, crate::ResponseCacheState)> {
    let client = FetchClient::new(config, new_shared_browser_cookie_store());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            if raw {
                let response = client.fetch_raw(request).await?;
                Ok((
                    String::from_utf8(response.clone_body_bytes())?,
                    response.cache_state,
                ))
            } else {
                let response = client.fetch(request).await?;
                Ok((response.body_text().to_owned(), response.cache_state))
            }
        })
}

fn metadata(cache_dir: &std::path::Path) -> Result<(std::path::PathBuf, HttpCacheEntryMetadata)> {
    let entry = fs::read_dir(cache_dir)?
        .next()
        .context("cache entry must exist")??;
    let path = entry.path().join("meta.json");
    let metadata = serde_json::from_slice(&fs::read(&path)?)?;
    Ok((path, metadata))
}

fn encoding_from_wire(request: &str) -> &str {
    let values = request
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("accept-encoding")
                .then_some(value.trim())
        })
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 1, "one actual Accept-Encoding header");
    values[0]
}

#[test]
fn cache_vary_encoding_records_wire_value_and_reuses_equivalent_explicit_request() -> Result<()> {
    for raw in [false, true] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("automatic")
                .with_header("Cache-Control", "max-age=600")
                .with_header("Vary", "Accept-Encoding"),
            ScriptedResponse::ok("identity")
                .with_header("Cache-Control", "max-age=600")
                .with_header("Vary", "Accept-Encoding"),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let first = fetch_variant(
            &config,
            Request::new("GET", &server.url(), None, crate::RequestHeaders::default())?,
            raw,
        )?;
        assert_eq!(first.0, "automatic");
        assert_eq!(first.1, crate::ResponseCacheState::None);
        let requests = server.requests();
        let value = encoding_from_wire(&requests[0]);
        let (_, stored) = metadata(&cache_dir)?;
        assert_eq!(stored.vary_headers.len(), 1);
        assert_eq!(stored.vary_headers[0].name, "accept-encoding");
        assert_eq!(stored.vary_headers[0].value.as_deref(), Some(value));

        let equivalent = fetch_variant(
            &config,
            Request::new(
                "GET",
                &server.url(),
                None,
                vec![("Accept-Encoding".to_owned(), value.to_owned())],
            )?,
            raw,
        )?;
        assert_eq!(equivalent.0, "automatic");
        assert_eq!(equivalent.1, crate::ResponseCacheState::Local);
        assert_eq!(server.hits(), 1);
        for (index, state) in [
            crate::ResponseCacheState::None,
            crate::ResponseCacheState::Local,
        ]
        .into_iter()
        .enumerate()
        {
            let changed = fetch_variant(
                &config,
                Request::new(
                    "GET",
                    &server.url(),
                    None,
                    vec![("Accept-Encoding".to_owned(), "identity".to_owned())],
                )?,
                raw,
            )?;
            assert_eq!(changed.0, "identity", "raw={raw}/index={index}");
            assert_eq!(changed.1, state);
        }
        assert_eq!(server.hits(), 2);
        let requests = server.requests();
        assert_eq!(encoding_from_wire(&requests[1]), "identity");
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[test]
fn cache_vary_encoding_refetches_legacy_automatic_marker_before_reusing_new_value() -> Result<()> {
    for raw in [false, true] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("old representation")
                .with_header("Cache-Control", "max-age=600")
                .with_header("Vary", "Accept-Encoding"),
            ScriptedResponse::ok("current representation")
                .with_header("Cache-Control", "max-age=600")
                .with_header("Vary", "Accept-Encoding"),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        assert_eq!(
            fetch_variant(&config, Request::get(&server.url())?, raw)?.0,
            "old representation"
        );
        let (path, mut stored) = metadata(&cache_dir)?;
        assert_eq!(stored.vary_headers[0].name, "accept-encoding");
        // This is the genuine metadata value used before the native zstd
        // capability change, not a reconstructible request header value.
        stored.vary_headers[0].value = Some("libcurl-auto".to_owned());
        fs::write(path, serde_json::to_vec(&stored)?)?;
        for state in [
            crate::ResponseCacheState::None,
            crate::ResponseCacheState::Local,
        ] {
            let response = fetch_variant(&config, Request::get(&server.url())?, raw)?;
            assert_eq!(response.0, "current representation");
            assert_eq!(response.1, state);
        }
        assert_eq!(server.hits(), 2);
        let requests = server.requests();
        let value = encoding_from_wire(&requests[1]);
        let (_, stored) = metadata(&cache_dir)?;
        assert_eq!(stored.vary_headers[0].value.as_deref(), Some(value));
        server.shutdown();
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

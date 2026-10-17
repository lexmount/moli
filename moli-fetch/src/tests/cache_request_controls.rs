use super::*;

#[test]
fn request_cache_control_headers_revalidate_fresh_cached_responses() {
    for (name, value) in [
        ("Cache-Control", "max-age=0"),
        ("cache-control", "no-cache"),
        ("Pragma", "no-cache"),
    ] {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("cached body")
                .with_header("Cache-Control", "max-age=60")
                .with_header("ETag", "\"version-1\""),
            ScriptedResponse::status(304, "Not Modified")
                .with_header("Cache-Control", "max-age=600")
                .with_header("ETag", "\"version-1\""),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let first =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(first.body_text(), "cached body");
        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.cache_state, crate::ResponseCacheState::Local);
        assert_eq!(server.hits(), 1);

        let mut request = Request::get(&server.url()).unwrap();
        request.request_headers = vec![(name.to_owned(), value.to_owned())].into();
        let validated = fetch_with_config_for_test(&config, request).unwrap();
        assert_eq!(
            validated.status, 200,
            "{name}: {value} must merge the cached 200 response with its 304 validation"
        );
        assert_eq!(validated.body_text(), "cached body");
        assert!(validated.from_cache);
        assert_eq!(validated.cache_state, crate::ResponseCacheState::Validated);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("if-none-match: \"version-1\""),
            "cached validators missing from {}",
            requests[1]
        );
        assert!(requests[1].to_ascii_lowercase().contains(&format!(
            "{}: {}",
            name.to_ascii_lowercase(),
            value
        )));

        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(cached.body_text(), "cached body");
        assert!(cached.from_cache);
        assert_eq!(cached.cache_state, crate::ResponseCacheState::Local);
        assert_eq!(server.hits(), 2);
        server.shutdown();
        fs::remove_dir_all(cache_dir).unwrap();
    }
}

#[test]
fn request_no_store_does_not_reuse_or_replace_an_existing_cached_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("original").with_header("Cache-Control", "max-age=600"),
        ScriptedResponse::ok("private one").with_header("Cache-Control", "max-age=600"),
        ScriptedResponse::ok("private two").with_header("Cache-Control", "max-age=600"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let original =
        fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    assert_eq!(original.body_text(), "original");
    for body in ["private one", "private two"] {
        let mut request = Request::get(&server.url()).unwrap();
        request.request_headers =
            vec![("Cache-Control".to_owned(), "max-age=0, No-Store".to_owned())].into();
        let response = fetch_with_config_for_test(&config, request).unwrap();
        assert_eq!(response.body_text(), body);
        assert!(!response.from_cache);
        let cached =
            fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
        assert_eq!(cached.body_text(), "original");
        assert!(cached.from_cache);
        assert_eq!(response.cache_state, crate::ResponseCacheState::None);
        assert_eq!(cached.cache_state, crate::ResponseCacheState::Local);
    }
    assert_eq!(server.hits(), 3);
    server.shutdown();
    fs::remove_dir_all(cache_dir).unwrap();
}

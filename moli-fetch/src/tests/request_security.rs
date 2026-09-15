use super::*;

mod intercepted_redirects;
use crate::{CookieHeaderPolicy, RedirectInfo, RedirectSource};

#[derive(Clone, Copy, Debug)]
pub(super) enum Transport {
    Buffered,
    Html,
    Raw,
}

impl Transport {
    pub(super) const ALL: [Self; 3] = [Self::Buffered, Self::Html, Self::Raw];

    pub(super) async fn fetch(self, client: &FetchClientHandle, request: Request) -> Result<()> {
        match self {
            Self::Buffered => {
                let (tx, rx) = oneshot::channel();
                client.fetch_with_cancel_callback(
                    request,
                    FetchCancelHandle::new(),
                    move |result| {
                        let _ = tx.send(result);
                    },
                )?;
                rx.await??;
            }
            Self::Html => {
                let mut response = client.fetch_html_stream(request).await?;
                while response.next_chunk().await.is_some() {}
                response.finish().await?;
            }
            Self::Raw => {
                client.fetch(request).await?;
            }
        }
        Ok(())
    }
}

fn synthetic_redirect(from: &Url, to: &Url) -> RedirectInfo {
    RedirectInfo {
        source: RedirectSource::ServiceWorker,
        from_url: from.clone(),
        to_url: to.clone(),
        status: 302,
        headers: vec![("Location".to_owned(), to.as_str().as_bytes().to_vec())],
        network_extra_info_available: false,
        request_extra_info: None,
        dispatched_request: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }
}

#[tokio::test]
async fn curl_explicit_cookies_do_not_cross_origins_in_any_transport() -> Result<()> {
    for transport in Transport::ALL {
        let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
        let source = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found").with_header("Location", &target.url()),
        ]);
        let mut config = FetchConfig::default();
        config.push_default_request_header("Cookie", "config=secret");
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let request = Request::new(
            "GET",
            &source.url(),
            None,
            vec![("cOoKiE".to_owned(), "request=secret".to_owned())],
        )?
        .with_cookie_header_policy(CookieHeaderPolicy::PreserveExplicit);

        transport.fetch(&client, request).await?;
        assert_eq!(source.hits(), 1, "{transport:?}");
        let initial = source.requests()[0].to_ascii_lowercase();
        assert!(
            initial.contains("\r\ncookie: config=secret\r\n"),
            "{initial}"
        );
        assert!(
            initial.contains("\r\ncookie: request=secret\r\n"),
            "{initial}"
        );
        assert_eq!(target.hits(), 1, "{transport:?}");
        let redirected = target.requests()[0].to_ascii_lowercase();
        assert!(
            !redirected.contains("\r\ncookie:"),
            "{transport:?}: {redirected}"
        );
        source.shutdown();
        target.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn literal_cookies_obey_browser_credentials_in_every_transport() -> Result<()> {
    for transport in Transport::ALL {
        let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("ok"); 6]);
        let target_url = Url::parse(&server.url())?;
        let other_origin = Url::parse("http://other.test/")?;
        let mut config = FetchConfig::default();
        config.set_additional_cookies(Some("session=literal".to_owned()));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        transport
            .fetch(&client, Request::get(target_url.as_str())?)
            .await?;
        for (origin, credentials, expected) in [
            (&target_url, RequestCredentialsMode::Omit, false),
            (&other_origin, RequestCredentialsMode::Omit, false),
            (&target_url, RequestCredentialsMode::SameOrigin, true),
            (&other_origin, RequestCredentialsMode::SameOrigin, false),
            (&other_origin, RequestCredentialsMode::Include, true),
        ] {
            let request = Request::new_browser(
                "GET",
                target_url.clone(),
                None,
                Vec::new(),
                moli_url::WebOrigin::from_url(origin),
            )
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(credentials);
            transport.fetch(&client, request).await?;
            let requests = server.requests();
            let actual = requests.last().unwrap().to_ascii_lowercase();
            assert_eq!(
                actual.contains("\r\ncookie: session=literal\r\n"),
                expected,
                "{transport:?}, {credentials:?}, origin={origin}: {actual}"
            );
        }
        assert_eq!(server.hits(), 6, "{transport:?}");
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn configured_cookie_scope_keeps_navigation_redirects_and_excludes_foreign_requests()
-> Result<()> {
    for transport in Transport::ALL {
        let foreign = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("foreign"); 4]);
        let mut foreign_url = Url::parse(&foreign.url())?;
        foreign_url.set_host(Some("other.test"))?;
        let home = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found").with_header("Location", foreign_url.as_str()),
            ScriptedResponse::ok("home asset"),
            ScriptedResponse::ok("home return"),
        ]);
        let mut config = FetchConfig::default();
        config.set_additional_cookies(Some("session=literal".to_owned()));
        config.set_http_no_proxy(Some("*".to_owned()));
        config.set_http_host_resolve(vec![format!(
            "other.test:{}:127.0.0.1",
            foreign_url.port().unwrap()
        )]);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        // A separate runtime created from the same config must share the
        // eventual binding, including when it was cloned before navigation.
        let secondary = FetchClient::new(&config, new_shared_browser_cookie_store());
        let automatic = |target: &str| {
            Request::new("GET", target, None, Vec::new())
                .unwrap()
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::Include)
        };

        transport
            .fetch(&secondary, automatic(foreign_url.as_str()))
            .await?;
        transport.fetch(&client, Request::get(&home.url())?).await?;
        transport.fetch(&secondary, automatic(&home.url())).await?;
        transport
            .fetch(&secondary, automatic(foreign_url.as_str()))
            .await?;
        transport
            .fetch(&secondary, Request::get(foreign_url.as_str())?)
            .await?;
        transport.fetch(&client, Request::get(&home.url())?).await?;
        assert!(client.matches_config(&config));
        assert!(secondary.matches_config(&config));

        assert_eq!(home.hits(), 3, "{transport:?}");
        for request in home.requests() {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("\r\ncookie: session=literal\r\n"),
                "{request}"
            );
        }
        assert_eq!(foreign.hits(), 4, "{transport:?}");
        for (index, request) in foreign.requests().iter().enumerate() {
            assert_eq!(
                request
                    .to_ascii_lowercase()
                    .contains("\r\ncookie: session=literal\r\n"),
                index == 1,
                "{transport:?}, foreign request {index}: {request}"
            );
        }
        home.shutdown();
        foreign.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn redirect_checks_block_before_contact_in_every_transport() -> Result<()> {
    let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("forbidden")]);
    let source = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", &target.url());
        3
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let target_url = Url::parse(&target.url())?;
    let seen = calls.clone();
    let check = crate::RequestRedirectCheck::new(move |next_url| {
        assert_eq!(next_url, &target_url);
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err("redirect denied by embedder".to_owned())
    });
    for transport in Transport::ALL {
        let error = transport
            .fetch(
                &client,
                Request::get(&source.url())?.with_redirect_check(check.clone()),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("redirect denied by embedder"),
            "{transport:?}: {error:#}"
        );
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(source.hits(), 3);
    assert_eq!(target.hits(), 0);
    source.shutdown();
    target.shutdown();
    Ok(())
}

#[tokio::test]
async fn cached_redirects_run_the_current_request_redirect_check() -> Result<()> {
    for transport in [Transport::Html, Transport::Raw] {
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("allowed once")]);
        let source = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found")
                .with_header("Location", &target.url())
                .with_header("Cache-Control", "max-age=600"),
        ]);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let request = Request::get(&source.url())?;
        transport.fetch(&client, request.clone()).await?;
        let error = transport
            .fetch(
                &client,
                request.with_redirect_check(crate::RequestRedirectCheck::new(|_| {
                    Err("cached redirect denied".to_owned())
                })),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("cached redirect denied"),
            "{transport:?}: {error:#}"
        );
        assert_eq!(source.hits(), 1, "the redirect response should be cached");
        assert_eq!(
            target.hits(),
            1,
            "the newly rejected request must not reach the target"
        );
        source.shutdown();
        target.shutdown();
        drop(client);
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn plain_http_requests_do_not_infer_browser_origin_from_referrer() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("http"); 3]);
    let url = Url::parse(&server.url())?;
    let referrer = Url::parse("http://other.test/page")?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for transport in Transport::ALL {
        let request = Request::new_bytes("GET", url.as_str(), None, Vec::new())?
            .with_initiator_url(&referrer);
        assert!(request.request_origin().is_none());
        transport.fetch(&client, request).await?;
    }
    assert_eq!(server.hits(), 3);
    for request in server.requests() {
        assert!(!request.to_ascii_lowercase().contains("\r\norigin:"));
    }
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn same_origin_mode_blocks_cross_origin_before_every_transport() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("forbidden").with_header("Access-Control-Allow-Origin", "*"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for transport in Transport::ALL {
        for origin in [Some(Url::parse("http://other.test/")?), None] {
            let mut request =
                Request::get(&server.url())?.with_request_mode(RequestMode::SameOrigin);
            if let Some(origin) = origin {
                request = request
                    .with_initiator_url(&origin)
                    .with_request_origin(moli_url::WebOrigin::from_url(&origin));
            }
            let error = transport.fetch(&client, request).await.unwrap_err();
            assert!(
                error.to_string().contains("same-origin request mode"),
                "{transport:?}: {error:#}"
            );
        }
    }
    assert_eq!(
        server.hits(),
        0,
        "rejected requests must not reach the server"
    );
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn same_origin_mode_checks_redirects_in_every_transport() -> Result<()> {
    let target = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("forbidden").with_header("Access-Control-Allow-Origin", "*"),
    ]);
    for transport in Transport::ALL {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found").with_header("Location", "/final"),
            ScriptedResponse::ok("ok"),
            ScriptedResponse::status(302, "Found").with_header("Location", &target.url()),
        ]);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let request = Request::get(&server.url())?
            .with_initiator_url(&Url::parse(&server.origin())?)
            .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(
                &server.origin(),
            )?))
            .with_request_mode(RequestMode::SameOrigin);
        transport.fetch(&client, request.clone()).await?;
        let error = transport.fetch(&client, request).await.unwrap_err();
        assert!(
            error.to_string().contains("same-origin request mode"),
            "{transport:?}: {error:#}"
        );
        assert_eq!(server.hits(), 3);
        server.shutdown();
    }
    assert_eq!(
        target.hits(),
        0,
        "cross-origin redirect must not be followed"
    );
    target.shutdown();
    Ok(())
}

#[tokio::test]
async fn same_origin_mode_rejects_inherited_cross_origin_history() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("forbidden")]);
    let url = Url::parse(&server.url())?;
    let request = Request::get_with_url(url.clone())
        .with_initiator_url(&url)
        .with_request_origin(moli_url::WebOrigin::from_url(&url))
        .with_request_mode(RequestMode::SameOrigin)
        .with_redirect_chain(vec![synthetic_redirect(
            &Url::parse("http://other.test/start")?,
            &url,
        )]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for transport in Transport::ALL {
        let error = transport.fetch(&client, request.clone()).await.unwrap_err();
        assert!(
            error.to_string().contains("same-origin request mode"),
            "{transport:?}: {error:#}"
        );
    }
    assert_eq!(server.hits(), 0);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn cors_redirect_rejection_never_contacts_next_hop() -> Result<()> {
    let origin = Url::parse("http://origin.test/")?;
    let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("forbidden")]);
    for transport in Transport::ALL {
        for (allow_origin, allow_credentials, credentials) in [
            (None, false, RequestCredentialsMode::SameOrigin),
            (None, false, RequestCredentialsMode::Include),
            (
                Some("http://wrong.test"),
                false,
                RequestCredentialsMode::SameOrigin,
            ),
            (Some("*"), true, RequestCredentialsMode::Include),
            (
                Some("http://origin.test"),
                false,
                RequestCredentialsMode::Include,
            ),
        ] {
            let mut redirect =
                ScriptedResponse::status(302, "Found").with_header("Location", &target.url());
            if let Some(value) = allow_origin {
                redirect = redirect.with_header("Access-Control-Allow-Origin", value);
            }
            if allow_credentials {
                redirect = redirect.with_header("Access-Control-Allow-Credentials", "true");
            }
            let server = ScriptedHttpServer::spawn(vec![redirect]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let request = Request::get(&server.url())?
                .with_initiator_url(&origin)
                .with_request_origin(moli_url::WebOrigin::from_url(&origin))
                .with_request_mode(RequestMode::Cors)
                .with_credentials_mode(credentials);
            let error = transport.fetch(&client, request).await.unwrap_err();
            assert!(
                error.to_string().contains("CORS check failed:"),
                "{transport:?}, {credentials:?}: {error:#}"
            );
            assert_eq!(server.hits(), 1);
            assert_eq!(target.hits(), 0, "CORS must fail before the next request");
            server.shutdown();
        }
    }
    target.shutdown();
    Ok(())
}

#[tokio::test]
async fn cors_redirect_validates_the_origin_before_recording_the_hop() -> Result<()> {
    for transport in Transport::ALL {
        for credentials in [
            RequestCredentialsMode::SameOrigin,
            RequestCredentialsMode::Include,
        ] {
            let origin = Url::parse("http://origin.test/")?;
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("ok")]);
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &target.url())
                    .with_header("Access-Control-Allow-Origin", "http://origin.test")
                    .with_header("Access-Control-Allow-Credentials", "true"),
            ]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let request = Request::get(&server.url())?
                .with_initiator_url(&origin)
                .with_request_origin(moli_url::WebOrigin::from_url(&origin))
                .with_request_mode(RequestMode::Cors)
                .with_credentials_mode(credentials);
            transport.fetch(&client, request).await?;
            assert_eq!(server.hits(), 1);
            assert_eq!(target.hits(), 1);
            assert!(
                server.requests()[0]
                    .to_ascii_lowercase()
                    .contains("\r\norigin: http://origin.test\r\n")
            );
            assert!(
                target.requests()[0]
                    .to_ascii_lowercase()
                    .contains("\r\norigin: null\r\n")
            );
            server.shutdown();
            target.shutdown();
        }
    }
    Ok(())
}

#[tokio::test]
async fn cors_redirect_inherits_synthetic_redirect_taint_without_checking_synthetic_headers()
-> Result<()> {
    for transport in Transport::ALL {
        for (allow_origin, allowed) in [("null", true), ("http://old.test", false)] {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("ok")]);
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &target.url())
                    .with_header("Access-Control-Allow-Origin", allow_origin),
            ]);
            let url = Url::parse(&server.url())?;
            let request = Request::get_with_url(url.clone())
                .with_initiator_url(&url)
                .with_request_origin(moli_url::WebOrigin::from_url(&url))
                .with_request_mode(RequestMode::Cors)
                .with_credentials_mode(RequestCredentialsMode::SameOrigin)
                .with_redirect_chain(vec![synthetic_redirect(
                    &Url::parse("http://cross.test/sw")?,
                    &url,
                )]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let result = transport.fetch(&client, request).await;
            if allowed {
                result?;
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("CORS check failed:")
                );
            }
            assert_eq!(server.hits(), 1);
            assert_eq!(target.hits(), usize::from(allowed));
            server.shutdown();
            target.shutdown();
        }
    }
    Ok(())
}

#[tokio::test]
async fn cached_redirects_enforce_cors_and_same_origin_before_follow() -> Result<()> {
    for transport in [Transport::Html, Transport::Raw] {
        for mode in [RequestMode::Cors, RequestMode::SameOrigin] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("ok")]);
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &target.url())
                    .with_header("Cache-Control", "max-age=600"),
            ]);
            let origin = if mode == RequestMode::SameOrigin {
                Url::parse(&server.origin())?
            } else {
                Url::parse("http://other.test/")?
            };
            let request = Request::get(&server.url())?
                .with_initiator_url(&origin)
                .with_request_origin(moli_url::WebOrigin::from_url(&origin))
                .with_request_mode(RequestMode::NoCors);
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            transport.fetch(&client, request.clone()).await?;
            let error = transport
                .fetch(&client, request.with_request_mode(mode))
                .await
                .unwrap_err();
            let expected = if mode == RequestMode::SameOrigin {
                "same-origin request mode"
            } else {
                "CORS check failed:"
            };
            assert!(
                error.to_string().contains(expected),
                "{transport:?}, {mode:?}: {error:#}"
            );
            assert_eq!(server.hits(), 1, "the second redirect must come from cache");
            assert_eq!(
                target.hits(),
                1,
                "the rejected cached redirect must not contact its target"
            );
            server.shutdown();
            target.shutdown();
            drop(client);
            fs::remove_dir_all(cache_dir)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn explicit_request_origin_controls_every_transport_independently_of_initiator_url()
-> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("allowed"); 3]);
    let url = Url::parse(&server.url())?;
    let unrelated = Url::parse("http://unrelated.test/base/")?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for transport in Transport::ALL {
        let opaque = Request::get_with_url(url.clone())
            .with_initiator_url(&url)
            .with_request_origin(moli_url::WebOrigin::Opaque)
            .with_request_mode(RequestMode::SameOrigin)
            .with_credentials_mode(RequestCredentialsMode::SameOrigin);
        assert!(!opaque.allows_credentials_for_url(&url));
        assert_eq!(opaque.serialized_origin(), "null");
        let error = transport.fetch(&client, opaque).await.unwrap_err();
        assert!(
            error.to_string().contains("same-origin request mode"),
            "{error:#}"
        );
        let allowed = Request::get_with_url(url.clone())
            .with_initiator_url(&unrelated)
            .with_request_origin((&url).into())
            .with_request_mode(RequestMode::SameOrigin);
        transport.fetch(&client, allowed).await?;
    }
    assert_eq!(
        server.hits(),
        3,
        "only requests with the matching origin are dispatched"
    );
    server.shutdown();
    Ok(())
}

#[test]
fn accumulated_interception_edits_preserve_upload_and_header_scope() -> Result<()> {
    for persistent in [false, true] {
        let mut request = Request::new(
            "POST",
            "https://example.test/source",
            Some("old".into()),
            vec![("X-Original".to_owned(), "original".to_owned())],
        )?;
        let headers =
            crate::RequestHeaders::from(vec![("X-Edited".to_owned(), "edited".to_owned())]);
        crate::RequestOverrides {
            url: Some(Url::parse("https://example.test/rewritten/source")?),
            method: Some("PUT".into()),
            body: Some(Some(bytes::Bytes::from_static(b"new\xff"))),
            headers: Some(if persistent {
                headers.into()
            } else {
                crate::RequestHeaderOverride::current_request(headers, None)
            }),
        }
        .apply(&mut request);
        assert_eq!(request.method, "PUT");
        assert_eq!(request.body.as_deref(), Some(b"new\xff".as_slice()));
        let mut redirect = synthetic_redirect(
            &request.url,
            &Url::parse("https://example.test/rewritten/next")?,
        );
        redirect.status = 307;
        request.follow_redirect(redirect)?;
        assert_eq!(request.method, "PUT");
        assert_eq!(request.body.as_deref(), Some(b"new\xff".as_slice()));
        assert_eq!(
            request
                .request_headers
                .iter()
                .any(|(name, _)| name == "X-Edited"),
            persistent
        );
        assert_eq!(
            request
                .request_headers
                .iter()
                .any(|(name, _)| name == "X-Original"),
            !persistent
        );
        crate::RequestOverrides {
            body: Some(None),
            ..Default::default()
        }
        .apply(&mut request);
        assert!(request.body.is_none());
        assert!(request.body_text().is_none());
    }
    Ok(())
}

#[test]
fn interception_origin_matches_transport_without_becoming_an_author_header() -> Result<()> {
    let source = Url::parse("https://source.test/page")?;
    let request = Request::new_browser(
        "GET",
        Url::parse("https://other.test/echo")?,
        None,
        Vec::new(),
        (&source).into(),
    )
    .with_request_mode(RequestMode::Cors);
    let projected = request.interception_request_headers().to_byte_strings();
    assert_eq!(
        projected,
        [("Origin".to_owned(), "https://source.test".to_owned())]
    );
    assert!(request.request_headers.is_empty());
    let wire = crate::blocking::outgoing_request_headers(&FetchConfig::default(), &request, None);
    assert!(wire.contains(&projected[0]));
    let same = Request::new_browser("GET", source.clone(), None, Vec::new(), (&source).into())
        .with_request_mode(RequestMode::Cors);
    assert!(same.interception_request_headers().is_empty());
    let opaque = Request::new_browser(
        "POST",
        source,
        None,
        Vec::new(),
        moli_url::WebOrigin::Opaque,
    );
    assert_eq!(
        opaque.interception_request_headers().to_byte_strings(),
        [("Origin".to_owned(), "null".to_owned())]
    );
    Ok(())
}

#[test]
fn interception_header_replay_does_not_restore_redirected_upload_headers() -> Result<()> {
    for status in [301, 302, 303, 307, 308] {
        let original = crate::RequestHeaders::from(vec![
            ("Content-Type".into(), "text/plain".into()),
            ("Content-Length".into(), "3".into()),
            ("X-Custom".into(), "value".into()),
        ]);
        let mut request = Request::new(
            "POST",
            "https://example.test/start",
            Some("old".into()),
            original.clone(),
        )?;
        let mut redirect =
            synthetic_redirect(&request.url, &Url::parse("https://example.test/final")?);
        redirect.status = status;
        request.follow_redirect(redirect)?;
        crate::RequestOverrides {
            headers: Some(crate::RequestHeaderOverride::current_request(
                original, None,
            )),
            ..Default::default()
        }
        .apply(&mut request);
        let names: Vec<_> = request
            .request_headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        let retains_upload = matches!(status, 307 | 308);
        assert_eq!(
            names.contains(&"content-type".into()),
            retains_upload,
            "{status}"
        );
        assert_eq!(
            names.contains(&"content-length".into()),
            retains_upload,
            "{status}"
        );
        assert!(names.contains(&"x-custom".into()));
        assert_eq!(request.body.is_some(), retains_upload);
    }
    Ok(())
}

#[test]
fn interception_authorization_is_scoped_to_the_explicitly_overridden_hop() -> Result<()> {
    let mut config = FetchConfig::default();
    config.push_default_request_header("Authorization", "config-secret");
    let mut request = Request::new(
        "GET",
        "https://source.test/start",
        None,
        vec![("Authorization".into(), "author-secret".into())],
    )?;
    request.follow_redirect(synthetic_redirect(
        &request.url,
        &Url::parse("https://target.test/next")?,
    ))?;
    let has_auth = |request: &Request, url: &Url| {
        crate::blocking::outgoing_request_headers_for_url(&config, request, url, None)
            .into_iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .collect::<Vec<_>>()
    };
    assert!(has_auth(&request, &request.url).is_empty());
    crate::RequestOverrides {
        headers: Some(crate::RequestHeaderOverride::current_request(
            vec![("Authorization".into(), "interceptor-secret".into())].into(),
            None,
        )),
        ..Default::default()
    }
    .apply(&mut request);
    assert_eq!(
        has_auth(&request, &request.url),
        [("Authorization".to_owned(), "interceptor-secret".to_owned())]
    );
    assert!(has_auth(&request, &Url::parse("https://third.test/escaped")?).is_empty());
    request.follow_redirect(synthetic_redirect(
        &request.url,
        &Url::parse("https://third.test/final")?,
    ))?;
    assert!(has_auth(&request, &request.url).is_empty());
    Ok(())
}

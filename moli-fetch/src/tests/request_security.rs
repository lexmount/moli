use super::*;
use crate::{RedirectInfo, RedirectSource};

#[derive(Clone, Copy, Debug)]
enum Transport {
    Buffered,
    Html,
    Raw,
}

impl Transport {
    const ALL: [Self; 3] = [Self::Buffered, Self::Html, Self::Raw];

    async fn fetch(self, client: &FetchClientHandle, request: Request) -> Result<()> {
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
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }
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

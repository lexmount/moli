use super::*;

#[derive(Clone, Copy, Debug)]
enum RequestKind {
    Navigation,
    Iframe,
    Fetch,
}

impl RequestKind {
    fn request(self, target: &Url) -> Result<Request> {
        let request = Request::get(target.as_str())?;
        Ok(match self {
            Self::Navigation => request.with_top_level_navigation_cookie_context(),
            Self::Iframe => request
                .with_initiator_url(&target.join("/creator")?)
                .with_request_origin(moli_url::WebOrigin::from_url(target))
                .with_subframe_navigation_cookie_context(),
            Self::Fetch => request
                .with_initiator_url(&target.join("/creator")?)
                .with_request_origin(moli_url::WebOrigin::from_url(target))
                .with_request_mode(RequestMode::NoCors)
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch),
        })
    }

    fn mode(self) -> &'static str {
        match self {
            Self::Navigation | Self::Iframe => "navigate",
            Self::Fetch => "no-cors",
        }
    }

    fn destination(self) -> &'static str {
        match self {
            Self::Navigation => "document",
            Self::Iframe => "iframe",
            Self::Fetch => "empty",
        }
    }
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[test]
fn browser_metadata_requires_a_potentially_trustworthy_current_destination() -> Result<()> {
    let config = FetchConfig::default();
    for (target, trustworthy) in [
        ("https://example.test/resource", true),
        ("http://example.test/resource", false),
        ("http://localhost/resource", true),
        ("http://sub.localhost/resource", true),
        ("http://LOCALHOST./resource", true),
        ("http://127.0.0.1/resource", true),
        ("http://127.0.0.2/resource", true),
        ("http://[::1]/resource", true),
        ("http://192.168.1.1/resource", false),
        ("http://localhost.example.test/resource", false),
        ("http://127.0.0.1.example.test/resource", false),
    ] {
        for kind in [
            RequestKind::Navigation,
            RequestKind::Iframe,
            RequestKind::Fetch,
        ] {
            let request = kind.request(&Url::parse(target)?)?;
            let headers = crate::outgoing_request_headers(&config, &request, None);
            let value = |name: &str| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
            };
            let case = format!("{kind:?}: {target}");
            assert!(value("Accept").is_some(), "{case}");
            assert!(value("Accept-Language").is_some(), "{case}");
            if trustworthy {
                assert_eq!(value("Sec-Fetch-Mode"), Some(kind.mode()), "{case}");
                assert_eq!(value("Sec-Fetch-Dest"), Some(kind.destination()), "{case}");
                assert_eq!(
                    value("Sec-Fetch-Site"),
                    Some(if matches!(kind, RequestKind::Navigation) {
                        "none"
                    } else {
                        "same-origin"
                    }),
                    "{case}"
                );
                assert_eq!(
                    value("Sec-Fetch-User"),
                    matches!(kind, RequestKind::Navigation).then_some("?1"),
                    "{case}"
                );
            } else {
                assert!(
                    !headers
                        .iter()
                        .any(|(name, _)| name.to_ascii_lowercase().starts_with("sec-fetch-")),
                    "{case}: {headers:?}"
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Transport {
    Buffered,
    Html,
    Raw,
}

fn destination_url(server: &ScriptedHttpServer, trustworthy: bool) -> Result<Url> {
    let mut url = Url::parse(&server.url())?;
    if !trustworthy {
        url.set_host(Some("untrusted.test"))?;
    }
    Ok(url)
}

async fn check_redirects(transport: Transport) -> Result<()> {
    for kind in [
        RequestKind::Navigation,
        RequestKind::Iframe,
        RequestKind::Fetch,
    ] {
        for trustworthy in [
            [true, false, true],
            [false, true, false],
            [false, false, false],
        ] {
            let last = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("done")]);
            let last_url = destination_url(&last, trustworthy[2])?;
            let middle = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found").with_header("Location", last_url.as_str()),
            ]);
            let middle_url = destination_url(&middle, trustworthy[1])?;
            let first = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found").with_header("Location", middle_url.as_str()),
            ]);
            let first_url = destination_url(&first, trustworthy[0])?;
            let mut config = FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            config.set_http_host_resolve(
                [&first_url, &middle_url, &last_url]
                    .map(|url| format!("untrusted.test:{}:127.0.0.1", url.port().unwrap()))
                    .to_vec(),
            );
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let request = kind.request(&first_url)?;
            let result = match transport {
                Transport::Buffered => client.fetch(request).await,
                Transport::Html => {
                    client
                        .fetch_html_stream(request)
                        .await?
                        .into_materialized_text_response()
                        .await
                }
                Transport::Raw => {
                    client
                        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                        .await?
                        .into_lossy_materialized_text_response()
                        .await
                }
            };
            let requests = [first.requests(), middle.requests(), last.requests()];
            first.shutdown();
            middle.shutdown();
            last.shutdown();
            let response = result?;
            let case = format!("{transport:?}, {kind:?}, trustworthy={trustworthy:?}");
            assert_eq!(response.status, 200, "{case}");
            assert_eq!(response.body_text(), "done", "{case}");
            assert_eq!(response.redirect_chain.len(), 2, "{case}");
            for (hop, requests) in requests.iter().enumerate() {
                assert_eq!(requests.len(), 1, "{case}: hop {hop}");
                let request = &requests[0];
                assert!(header(request, "Accept").is_some(), "{case}: hop {hop}");
                assert!(
                    header(request, "Accept-Language").is_some(),
                    "{case}: hop {hop}"
                );
                if trustworthy[hop] {
                    assert_eq!(
                        header(request, "Sec-Fetch-Mode"),
                        Some(kind.mode()),
                        "{case}: hop {hop}"
                    );
                    assert_eq!(
                        header(request, "Sec-Fetch-Dest"),
                        Some(kind.destination()),
                        "{case}: hop {hop}"
                    );
                    assert!(
                        header(request, "Sec-Fetch-Site").is_some(),
                        "{case}: hop {hop}"
                    );
                    assert_eq!(
                        header(request, "Sec-Fetch-User"),
                        matches!(kind, RequestKind::Navigation).then_some("?1"),
                        "{case}: hop {hop}"
                    );
                } else {
                    assert!(
                        !request
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .any(|(name, _)| name.to_ascii_lowercase().starts_with("sec-fetch-")),
                        "{case}: hop {hop}: {request}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn buffered_metadata_is_filtered_for_each_redirect_destination() -> Result<()> {
    check_redirects(Transport::Buffered).await
}

#[tokio::test]
async fn html_stream_metadata_is_filtered_for_each_redirect_destination() -> Result<()> {
    check_redirects(Transport::Html).await
}

#[tokio::test]
async fn raw_stream_metadata_is_filtered_for_each_redirect_destination() -> Result<()> {
    check_redirects(Transport::Raw).await
}

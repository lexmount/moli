use super::*;

#[derive(Clone, Copy, Debug)]
enum Transport {
    Buffered,
    Html,
    Raw,
}

impl Transport {
    async fn fetch(self, client: &FetchClientHandle, request: Request) -> Result<Response> {
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
                rx.await?
            }
            Self::Html => {
                client
                    .fetch_html_stream(request)
                    .await?
                    .into_materialized_text_response()
                    .await
            }
            Self::Raw => {
                client
                    .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                    .await?
                    .into_lossy_materialized_text_response()
                    .await
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Referrer {
    Omitted,
    Origin,
    Full,
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

async fn check_redirect_referrers(
    transport: Transport,
    policy: Option<&str>,
    document_policy: bool,
    response_policies: &[&str],
    expected: [Referrer; 3],
    cached: bool,
) -> Result<()> {
    let runs = if cached { 2 } else { 1 };
    // The first hop is cross-origin; both subsequent hops return to the
    // document origin. Relaxing the policy on the middle hop must not recover
    // anything removed by either of the earlier requests.
    let target = ScriptedHttpServer::spawn(
        (0..runs)
            .flat_map(|_| {
                [
                    ScriptedResponse::status(302, "Found")
                        .with_header("Location", "/final")
                        .with_header("Referrer-Policy", "unsafe-url")
                        .with_header("Cache-Control", "no-store"),
                    ScriptedResponse::ok("final").with_header("Cache-Control", "no-store"),
                ]
            })
            .collect(),
    );
    let mut redirect = ScriptedResponse::status(302, "Found")
        .with_header("Location", &target.url_path("/middle"))
        .with_header(
            "Cache-Control",
            if cached { "max-age=600" } else { "no-store" },
        );
    for value in response_policies {
        redirect = redirect.with_header("rEfErReR-pOlIcY", value);
    }
    let source = ScriptedHttpServer::spawn(vec![redirect]);
    let document = Url::parse(&target.url_path("/private/document?source=1#fragment"))?;
    let full_referrer = target.url_path("/private/document?source=1");
    let origin_referrer = target.url_path("/");
    let metadata = if document_policy {
        SubresourceRequestMetadata {
            document_referrer_policy: policy.map(str::to_owned),
            ..Default::default()
        }
    } else {
        SubresourceRequestMetadata {
            referrer_policy: policy.map(str::to_owned),
            ..Default::default()
        }
    };
    let request = Request::get(&source.url())?
        .with_initiator_url(&document)
        .with_subresource_request_metadata(metadata);
    let mut config = FetchConfig::default();
    let cache_dir = cached.then(unique_test_cache_dir);
    if let Some(cache_dir) = &cache_dir {
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    }
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut responses = Vec::new();
    for _ in 0..runs {
        responses.push(transport.fetch(&client, request.clone()).await?);
    }
    let source_requests = source.requests();
    let target_requests = target.requests();
    source.shutdown();
    target.shutdown();
    drop(client);
    if let Some(cache_dir) = cache_dir {
        fs::remove_dir_all(cache_dir)?;
    }

    let case = format!(
        "{transport:?}, {policy:?}, document={document_policy}, headers={response_policies:?}, cached={cached}"
    );
    assert_eq!(source_requests.len(), 1, "{case}: cacheable first redirect");
    assert_eq!(
        target_requests.len(),
        runs * 2,
        "{case}: middle and final hops"
    );
    let expected = expected.map(|value| match value {
        Referrer::Omitted => None,
        Referrer::Origin => Some(origin_referrer.as_str()),
        Referrer::Full => Some(full_referrer.as_str()),
    });
    assert_eq!(
        header(&source_requests[0], "Referer"),
        expected[0],
        "{case}: first hop"
    );
    for (run, response) in responses.iter().enumerate() {
        assert_eq!(response.status, 200, "{case}");
        assert_eq!(response.body_text(), "final", "{case}");
        assert_eq!(response.final_url.path(), "/final", "{case}");
        assert!(response.redirected, "{case}");
        assert!(!response.from_cache, "{case}");
        assert_eq!(response.redirect_chain.len(), 2, "{case}");
        assert_eq!(response.redirect_chain[0].from_cache, run == 1, "{case}");
        assert!(!response.redirect_chain[1].from_cache, "{case}");
        for hop in 0..2 {
            let received = &target_requests[run * 2 + hop];
            assert_eq!(
                header(received, "Referer"),
                expected[hop + 1],
                "{case}: run {run}, hop {hop}"
            );
        }
    }
    Ok(())
}

async fn check_transport(transport: Transport) -> Result<()> {
    use Referrer::{Full, Omitted, Origin};
    let cache_modes: &[bool] = match transport {
        Transport::Buffered => &[false],
        Transport::Html | Transport::Raw => &[false, true],
    };
    for &cached in cache_modes {
        for document_policy in [false, true] {
            for (policy, expected) in [
                (None, Origin),
                (Some("no-referrer"), Omitted),
                (Some("no-referrer-when-downgrade"), Full),
                (Some("origin"), Origin),
                (Some("origin-when-cross-origin"), Origin),
                (Some("same-origin"), Omitted),
                (Some("strict-origin"), Origin),
                (Some("strict-origin-when-cross-origin"), Origin),
                (Some("unsafe-url"), Full),
            ] {
                check_redirect_referrers(
                    transport,
                    policy,
                    document_policy,
                    &[],
                    [expected; 3],
                    cached,
                )
                .await?;
            }
            for (policy, headers, expected) in [
                ("unsafe-url", &["no-referrer"][..], [Full, Omitted, Omitted]),
                ("unsafe-url", &["origin"][..], [Full, Origin, Origin]),
                ("origin", &["unsafe-url"][..], [Origin; 3]),
                ("same-origin", &["unsafe-url"][..], [Omitted; 3]),
                ("no-referrer", &["unsafe-url"][..], [Omitted; 3]),
                ("unsafe-url", &["", "future-policy"][..], [Full; 3]),
                (
                    "unsafe-url",
                    &["no-referrer, origin", "future-policy"][..],
                    [Full, Origin, Origin],
                ),
                (
                    "unsafe-url",
                    &["unsafe-url", "origin, no-referrer, future-policy"][..],
                    [Full, Omitted, Omitted],
                ),
            ] {
                check_redirect_referrers(
                    transport,
                    Some(policy),
                    document_policy,
                    headers,
                    expected,
                    cached,
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn buffered_redirects_preserve_referrer_state() -> Result<()> {
    check_transport(Transport::Buffered).await
}

#[tokio::test]
async fn html_redirects_preserve_referrer_state() -> Result<()> {
    check_transport(Transport::Html).await
}

#[tokio::test]
async fn raw_redirects_preserve_referrer_state() -> Result<()> {
    check_transport(Transport::Raw).await
}

#[tokio::test]
async fn manual_redirects_keep_the_current_response_and_referrer() -> Result<()> {
    let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("unreachable")]);
    let source = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", &target.url())
            .with_header("Referrer-Policy", "no-referrer")
            .with_body("redirect");
        2
    ]);
    let initiator = Url::parse("http://document.test/private?query=1#fragment")?;
    let request = Request::get(&source.url())?
        .with_initiator_url(&initiator)
        .with_subresource_request_metadata(SubresourceRequestMetadata {
            referrer_policy: Some("unsafe-url".to_owned()),
            ..Default::default()
        })
        .with_follow_redirects(false);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    // Manual responses use the buffered or raw API. Raw streaming intentionally
    // returns only the redirect head so its owner need not wait for the body.
    for (transport, body) in [(Transport::Buffered, "redirect"), (Transport::Raw, "")] {
        let response = transport.fetch(&client, request.clone()).await?;
        assert_eq!(response.status, 302, "{transport:?}");
        assert_eq!(response.body_text(), body, "{transport:?}");
        assert_eq!(response.final_url, request.url, "{transport:?}");
        assert!(!response.redirected, "{transport:?}");
        assert!(response.redirect_chain.is_empty(), "{transport:?}");
    }
    let received = source.requests();
    let target_hits = target.hits();
    source.shutdown();
    target.shutdown();
    assert_eq!(target_hits, 0);
    assert_eq!(received.len(), 2);
    for request in received {
        assert_eq!(
            header(&request, "Referer"),
            Some("http://document.test/private?query=1")
        );
    }
    Ok(())
}

#[test]
fn redirect_referrer_updates_preserve_request_identity_and_metadata() -> Result<()> {
    let initiator = Url::parse("https://document.test/private?query=1#fragment")?;
    let first_hop = Url::parse("https://other.test/redirect")?;
    for (policy, expected) in [
        ("same-origin", None),
        ("origin", Some("https://document.test/")),
    ] {
        let mut request = Request::new_browser(
            "POST",
            first_hop.clone(),
            None,
            Vec::new(),
            (&initiator).into(),
        )
        .with_initiator_url(&initiator)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
        .with_network_partition_key(Some("original-partition".to_owned()))
        .with_subresource_request_metadata(SubresourceRequestMetadata {
            referrer_policy: Some(policy.to_owned()),
            document_referrer_policy: Some("strict-origin".to_owned()),
            integrity: Some("original-integrity".to_owned()),
        });
        request.update_referrer_for_redirect(
            &first_hop,
            &[("Referrer-Policy".to_owned(), b"unsafe-url".to_vec())],
        );
        request.url = initiator.join("/returned")?;
        let headers = crate::outgoing_request_headers(&FetchConfig::default(), &request, None);
        let value = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("Referer"), expected);
        assert_eq!(value("Origin"), Some("https://document.test"));
        assert_eq!(
            request.cookie_context.initiator_url.as_ref(),
            Some(&initiator)
        );
        assert_eq!(request.network_partition_key(), Some("original-partition"));
        let metadata = request.subresource_request_metadata().unwrap();
        assert_eq!(metadata.integrity.as_deref(), Some("original-integrity"));
        assert_eq!(
            metadata.document_referrer_policy.as_deref(),
            Some("strict-origin")
        );
    }
    Ok(())
}

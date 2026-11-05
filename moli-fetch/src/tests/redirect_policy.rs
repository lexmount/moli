use super::*;

async fn fetch(client: &FetchClient, request: Request, transport: &str) -> Result<()> {
    match transport {
        "buffered" => {
            client.fetch(request).await?;
        }
        "html" => {
            client
                .fetch_html_stream(request)
                .await?
                .into_materialized_text_response()
                .await?;
        }
        "raw" => {
            client
                .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
                .await?
                .into_materialized_raw_response()
                .await?;
        }
        "callback" => {
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
        _ => unreachable!(),
    }
    Ok(())
}

#[tokio::test]
async fn redirect_policy_blocks_before_dispatch_in_every_transport_and_cached_redirect()
-> Result<()> {
    for transport in ["buffered", "html", "raw", "callback"] {
        for cached in [false, true] {
            let cache_dir = unique_test_cache_dir();
            let mut config = FetchConfig::default();
            config.set_http_cache_dir(Some(cache_dir.display().to_string()));
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let origin = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found")
                    .with_header("Location", &target.url())
                    .with_header("Cache-Control", "max-age=600"),
            ]);
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let mut request = Request::get(&origin.url())?;
            if cached {
                // The callback transport reads the shared HTTP cache but does
                // not populate it. Seed through the ordinary streaming path.
                client.fetch(request.clone()).await?;
                request = request.with_cache_mode(RequestCacheMode::OnlyIfCached);
            }
            let target_url = Url::parse(&target.url())?;
            let guard = crate::RequestRedirectPolicy::new(move |url, _ordinal| {
                anyhow::ensure!(url != &target_url, "redirect denied before dispatch");
                Ok(())
            });
            let error = fetch(&client, request.with_redirect_policy(guard), transport)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("redirect denied before dispatch"),
                "{transport}, cached={cached}: {error}"
            );
            assert_eq!(origin.hits(), 1, "{transport}, cached={cached}");
            assert_eq!(
                target.hits(),
                usize::from(cached),
                "blocked destination received a request: {transport}, cached={cached}"
            );
            assert!(client.shutdown().is_clean());
            origin.shutdown();
            target.shutdown();
            if cache_dir.exists() {
                fs::remove_dir_all(cache_dir)?;
            }
        }
    }
    Ok(())
}

#[test]
fn caller_owned_redirect_cannot_bypass_request_policy() -> Result<()> {
    let mut request = Request::get("https://allowed.test/a")?
        .with_single_hop()
        .with_redirect_policy(crate::RequestRedirectPolicy::new(|_, _ordinal| {
            anyhow::bail!("denied")
        }));
    let redirect = crate::RedirectInfo {
        from_url: request.url.clone(),
        to_url: Url::parse("https://blocked.test/b")?,
        status: 302,
        source: crate::RedirectSource::ServiceWorker,
        headers: Vec::new(),
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    assert!(request.follow_redirect(redirect).is_err());
    assert_eq!(request.url.as_str(), "https://allowed.test/a");
    assert!(request.redirect_chain().is_empty());
    Ok(())
}

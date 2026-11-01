use super::request_security::Transport;
use super::*;

#[tokio::test]
async fn redirected_navigation_keeps_cookie_method_allowance_in_every_transport() -> Result<()> {
    let cases: [(&str, bool, bool, &[&str]); 4] = [
        ("GET", true, false, &["lax=1", "unspecified=1"]),
        ("POST", true, false, &["unspecified=1"]),
        ("GET", false, false, &[]),
        ("POST", true, true, &[]),
    ];
    for transport in Transport::ALL {
        for (method, navigation, force_cross_site, expected) in cases {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let mut target_url = Url::parse(&target.url())?;
            target_url.set_host(Some("other.test"))?;
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(307, "Temporary Redirect")
                    .with_header("Location", target_url.as_str()),
            ]);
            let source_url = Url::parse(&source.url())?;
            let mut config = FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            config.set_http_host_resolve(vec![format!(
                "other.test:{}:127.0.0.1",
                target_url.port().unwrap()
            )]);
            let cookies = new_shared_browser_cookie_store();
            cookies
                .lock()
                .expect("cookie store is available")
                .store_response_headers(
                    &target_url,
                    &[
                        (
                            "set-cookie".to_owned(),
                            b"strict=1; SameSite=Strict; Path=/".to_vec(),
                        ),
                        (
                            "set-cookie".to_owned(),
                            b"lax=1; SameSite=Lax; Path=/".to_vec(),
                        ),
                        ("set-cookie".to_owned(), b"unspecified=1; Path=/".to_vec()),
                    ],
                );
            let client = FetchClient::new(&config, cookies);
            let mut request = Request::new(method, source_url.as_str(), None, vec![])?;
            if navigation {
                request = request.with_top_level_navigation_cookie_context();
            }
            request = request
                .with_initiator_url(&source_url)
                .with_site_for_cookies_url(&source_url)
                .with_top_frame_origin_url(&source_url);
            if force_cross_site {
                request = request.with_cross_site_cookie_context();
            }
            transport.fetch(&client, request).await?;
            let source_requests = source.requests();
            let target_requests = target.requests();
            source.shutdown();
            target.shutdown();
            assert_eq!(source_requests.len(), 1);
            assert_eq!(target_requests.len(), 1);
            assert!(
                target_requests[0].starts_with(&format!("{method} {} ", target_url.path())),
                "unexpected redirected request: {}",
                target_requests[0],
            );
            let mut actual: Vec<_> = target_requests[0]
                .lines()
                .filter_map(|line| line.split_once(':'))
                .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                .flat_map(|(_, value)| value.split(';').map(str::trim))
                .collect();
            actual.sort_unstable();
            let mut expected = expected.to_vec();
            expected.sort_unstable();
            assert_eq!(
                actual, expected,
                "{transport:?}, {method}, navigation={navigation}, force_cross_site={force_cross_site}: {}",
                target_requests[0],
            );
        }
    }
    Ok(())
}

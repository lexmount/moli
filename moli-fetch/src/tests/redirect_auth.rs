use super::request_security::Transport;
use super::*;
use crate::{RedirectInfo, RedirectSource, blocking::outgoing_request_headers_for_url};

#[derive(Clone, Copy, Debug)]
enum Credentials {
    Headers,
    Basic,
}

impl Credentials {
    const ALL: [Self; 2] = [Self::Headers, Self::Basic];

    fn request(self, config: &mut FetchConfig, url: &str) -> Result<Request> {
        Ok(match self {
            Self::Headers => {
                config.push_default_request_header("AuThOrIzAtIoN", "Bearer config-secret");
                Request::new(
                    "GET",
                    url,
                    None,
                    vec![
                        (
                            "authorization".to_owned(),
                            "Bearer request-secret".to_owned(),
                        ),
                        ("X-Public".to_owned(), "retained".to_owned()),
                    ],
                )?
            }
            Self::Basic => Request::get(url)?.with_auth(server_auth(RequestAuthScheme::Basic)),
        })
    }

    fn expected(self) -> Vec<&'static str> {
        match self {
            Self::Headers => vec!["Bearer config-secret", "Bearer request-secret"],
            Self::Basic => vec!["Basic dXNlcjpwYXNz"],
        }
    }
}

fn server_auth(scheme: RequestAuthScheme) -> RequestAuth {
    RequestAuth {
        target: RequestAuthTarget::Server,
        scheme,
        username: "user".to_owned(),
        password: "pass".to_owned(),
    }
}

fn authorization_headers(request: &str) -> Vec<&str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.trim())
        .collect()
}

fn redirect(from: &Url, to: &Url) -> RedirectInfo {
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
async fn redirect_authorization_does_not_reach_foreign_origin_in_any_transport() -> Result<()> {
    for transport in Transport::ALL {
        for credentials in Credentials::ALL {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found").with_header("Location", &target.url()),
            ]);
            let mut config = FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            let request = credentials.request(&mut config, &source.url())?;
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());

            transport.fetch(&client, request).await?;

            let source_requests = source.requests();
            let target_requests = target.requests();
            source.shutdown();
            target.shutdown();
            assert_eq!(source_requests.len(), 1, "{transport:?}, {credentials:?}");
            assert_eq!(target_requests.len(), 1, "{transport:?}, {credentials:?}");
            assert_eq!(
                authorization_headers(&source_requests[0]),
                credentials.expected(),
                "{transport:?}, {credentials:?}"
            );
            assert!(
                authorization_headers(&target_requests[0]).is_empty(),
                "{transport:?}, {credentials:?}: {}",
                target_requests[0]
            );
            if matches!(credentials, Credentials::Headers) {
                assert!(target_requests[0].contains("X-Public: retained\r\n"));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn redirect_authorization_survives_same_origin_in_any_transport() -> Result<()> {
    for transport in Transport::ALL {
        for credentials in Credentials::ALL {
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(302, "Found").with_header("Location", "/final"),
                ScriptedResponse::ok("target"),
            ]);
            let mut config = FetchConfig::default();
            let request = credentials.request(&mut config, &source.url())?;
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());

            transport.fetch(&client, request).await?;

            let requests = source.requests();
            source.shutdown();
            assert_eq!(requests.len(), 2, "{transport:?}, {credentials:?}");
            for request in requests {
                assert_eq!(
                    authorization_headers(&request),
                    credentials.expected(),
                    "{transport:?}, {credentials:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn redirect_authorization_keeps_initial_scope_when_callers_update_request_url() -> Result<()> {
    let initial = Url::parse("https://initial.test/start")?;
    for credentials in Credentials::ALL {
        for target in [
            "https://other.test/final",
            "https://initial.test:444/final",
            "http://initial.test/final",
        ] {
            let mut config = FetchConfig::default();
            let mut request = credentials.request(&mut config, initial.as_str())?;
            let target = Url::parse(target)?;
            request.follow_redirect(redirect(&initial, &target))?;
            let headers = outgoing_request_headers_for_url(&config, &request, &target, None);
            assert!(
                !headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
                "{credentials:?}, {target}: {headers:?}"
            );

            request.follow_redirect(redirect(&target, &initial))?;
            let headers = outgoing_request_headers_for_url(&config, &request, &initial, None);
            let actual: Vec<_> = headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_str())
                .collect();
            assert_eq!(actual, credentials.expected(), "{credentials:?}, {target}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn redirect_authorization_digest_cannot_answer_foreign_challenge() -> Result<()> {
    let target = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(401, "Unauthorized").with_header(
            "WWW-Authenticate",
            "Digest realm=\"foreign\", nonce=\"test-nonce\", qop=\"auth\"",
        ),
        ScriptedResponse::ok("must not authenticate"),
    ]);
    let source = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found").with_header("Location", &target.url()),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    Transport::Buffered
        .fetch(
            &client,
            Request::get(&source.url())?.with_auth(server_auth(RequestAuthScheme::Digest)),
        )
        .await?;
    let requests = target.requests();
    source.shutdown();
    target.shutdown();
    assert_eq!(
        requests.len(),
        1,
        "foreign challenge was answered: {requests:?}"
    );
    assert!(authorization_headers(&requests[0]).is_empty());
    Ok(())
}

#[test]
fn redirect_authorization_proxy_credentials_keep_their_independent_scope() -> Result<()> {
    let initial = Url::parse("http://initial.test/start")?;
    let foreign = Url::parse("http://foreign.test/final")?;
    for target in [RequestAuthTarget::Proxy, RequestAuthTarget::ProxyHeader] {
        let request = Request::get(initial.as_str())?.with_auth(RequestAuth {
            target,
            ..server_auth(RequestAuthScheme::Basic)
        });
        assert_eq!(
            request.transport_auth_for_url(&foreign).unwrap().target,
            target
        );
        assert!(
            request
                .preemptive_server_basic_auth_for_url(&foreign)
                .is_none()
        );
    }
    Ok(())
}

#[tokio::test]
async fn redirect_authorization_new_credentials_bind_only_to_the_challenged_origin() -> Result<()> {
    let initial = Url::parse("http://initial.test/start")?;
    for transport in Transport::ALL {
        let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("authorized")]);
        let challenged = Url::parse(&target.url())?;
        let mut request =
            Request::get(initial.as_str())?.with_auth(server_auth(RequestAuthScheme::Basic));
        request.follow_redirect(redirect(&initial, &challenged))?;
        assert!(
            request
                .preemptive_server_basic_auth_for_url(&challenged)
                .is_none()
        );

        // A later explicit auth response is a new grant for B, not permission
        // to reuse the credentials originally supplied for A.
        request.set_auth(Some(server_auth(RequestAuthScheme::Basic)));
        assert!(
            request
                .preemptive_server_basic_auth_for_url(&initial)
                .is_none()
        );
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        transport.fetch(&client, request.clone()).await?;
        let requests = target.requests();
        target.shutdown();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            authorization_headers(&requests[0]),
            Credentials::Basic.expected()
        );
        request.set_auth(None);
        assert!(
            request
                .preemptive_server_basic_auth_for_url(&challenged)
                .is_none()
        );
    }
    Ok(())
}

#[tokio::test]
async fn redirect_authorization_digest_still_answers_same_origin_challenge() -> Result<()> {
    let source = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(401, "Unauthorized").with_header(
            "WWW-Authenticate",
            "Digest realm=\"home\", nonce=\"test-nonce\", qop=\"auth\"",
        ),
        ScriptedResponse::ok("authorized"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    Transport::Buffered
        .fetch(
            &client,
            Request::get(&source.url())?.with_auth(server_auth(RequestAuthScheme::Digest)),
        )
        .await?;
    let requests = source.requests();
    source.shutdown();
    assert_eq!(requests.len(), 2);
    assert!(authorization_headers(&requests[0]).is_empty());
    assert!(authorization_headers(&requests[1])[0].starts_with("Digest "));
    Ok(())
}

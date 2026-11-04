use super::*;
use parking_lot::Mutex;

struct ScriptServer {
    url: Url,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl ScriptServer {
    async fn new(status: u16, headers: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/script.mjs",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let body = "export const value = 1;";
        let extra_headers = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/javascript\r\nCache-Control: max-age=3600\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.len()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_http_request_text(&mut stream).await.unwrap();
                captured.lock().push(request);
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().len()
    }

    fn last_origin(&self) -> Option<String> {
        self.last_header("origin")
    }

    fn last_header(&self, header: &str) -> Option<String> {
        self.requests.lock().last().and_then(|request| {
            request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(header)
                    .then(|| value.trim().to_owned())
            })
        })
    }
}

impl Drop for ScriptServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn cors_request(
    url: &Url,
    origin: &str,
    mode: moli_fetch::RequestMode,
    credentials: RequestCredentialsMode,
) -> Request {
    Request::new("GET", url.as_str(), None, Vec::new())
        .unwrap()
        .with_initiator_url(url)
        .with_request_origin(moli_url::WebOrigin::from_url(&Url::parse(origin).unwrap()))
        .with_request_mode(mode)
        .with_credentials_mode(credentials)
        .with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata::default())
}

#[tokio::test]
async fn cached_async_scripts_require_explicit_origin_and_credential_authorization() {
    use moli_fetch::RequestMode;
    for (headers, credentials, allowed) in [
        (vec![], RequestCredentialsMode::SameOrigin, false),
        (
            vec![("Access-Control-Allow-Origin", "https://wrong.test")],
            RequestCredentialsMode::SameOrigin,
            false,
        ),
        (
            vec![("Access-Control-Allow-Origin", "https://page.test")],
            RequestCredentialsMode::SameOrigin,
            true,
        ),
        (
            vec![("Access-Control-Allow-Origin", "*")],
            RequestCredentialsMode::SameOrigin,
            true,
        ),
        (
            vec![("Access-Control-Allow-Origin", "*")],
            RequestCredentialsMode::Include,
            false,
        ),
        (
            vec![("Access-Control-Allow-Origin", "https://page.test")],
            RequestCredentialsMode::Include,
            false,
        ),
        (
            vec![
                ("Access-Control-Allow-Origin", "https://page.test"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
            RequestCredentialsMode::Include,
            true,
        ),
    ] {
        let server = ScriptServer::new(200, &headers).await;
        let owner = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let client = owner.handle();
        let request = cors_request(
            &server.url,
            "https://page.test",
            RequestMode::Cors,
            credentials,
        );
        let result = timeout(
            Duration::from_secs(5),
            client.fetch_cacheable_script_text_stream(request.clone()),
        )
        .await
        .unwrap();
        assert_eq!(
            result.is_ok(),
            allowed,
            "{headers:?}, {credentials:?}: {result:?}"
        );
        assert_eq!(server.last_origin().as_deref(), Some("https://page.test"));
        if allowed {
            client
                .fetch_cacheable_script_text_stream(request)
                .await
                .unwrap();
            assert_eq!(
                server.request_count(),
                1,
                "authorized responses retain cache reuse"
            );
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("CORS check failed")
            );
        }
    }
}

#[tokio::test]
async fn script_cache_keeps_request_modes_and_client_origins_separate() {
    use moli_fetch::RequestMode;
    let server = ScriptServer::new(
        200,
        &[("Access-Control-Allow-Origin", "https://first.test")],
    )
    .await;
    let owner = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let client = owner.handle();
    let first = cors_request(
        &server.url,
        "https://first.test",
        RequestMode::Cors,
        RequestCredentialsMode::SameOrigin,
    );
    client
        .fetch_cacheable_script_text_stream(first.clone())
        .await
        .unwrap();
    client
        .fetch_cacheable_script_text_stream(first)
        .await
        .unwrap();
    assert_eq!(server.request_count(), 1);
    let second = cors_request(
        &server.url,
        "https://second.test",
        RequestMode::Cors,
        RequestCredentialsMode::SameOrigin,
    );
    assert!(
        client
            .fetch_cacheable_script_text_stream(second)
            .await
            .is_err()
    );
    let server = ScriptServer::new(200, &[]).await;
    let request = cors_request(
        &server.url,
        "https://first.test",
        RequestMode::NoCors,
        RequestCredentialsMode::Include,
    );
    client
        .fetch_cacheable_script_text_stream(request)
        .await
        .unwrap();
    let request = cors_request(
        &server.url,
        "https://first.test",
        RequestMode::Cors,
        RequestCredentialsMode::Include,
    );
    assert!(
        client
            .fetch_cacheable_script_text_stream(request)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn script_cors_rejection_does_not_wait_for_body_eof_or_contact_redirect_targets() {
    use moli_fetch::RequestMode;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/never-ending.js",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_http_request_text(&mut socket).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 999999\r\n\r\n").await.unwrap();
        std::future::pending::<()>().await;
        drop(socket);
    });
    let owner = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let client = owner.handle();
    let request = cors_request(
        &url,
        "https://page.test",
        RequestMode::Cors,
        RequestCredentialsMode::SameOrigin,
    );
    let result = timeout(
        Duration::from_secs(3),
        client.fetch_cacheable_script_text_stream(request),
    )
    .await;
    task.abort();
    let error = result
        .expect("CORS rejection precedes body completion")
        .unwrap_err();
    assert!(error.to_string().contains("CORS check failed"), "{error}");

    let target = ScriptServer::new(200, &[("Access-Control-Allow-Origin", "*")]).await;
    let redirect = ScriptServer::new(302, &[("Location", target.url.as_str())]).await;
    let request = cors_request(
        &redirect.url,
        "https://page.test",
        RequestMode::Cors,
        RequestCredentialsMode::SameOrigin,
    );
    assert!(
        client
            .fetch_cacheable_script_text_stream(request)
            .await
            .is_err()
    );
    assert_eq!(target.request_count(), 0);
}

#[tokio::test]
async fn script_cors_redirect_taint_keeps_browser_headers_out_of_preflight() {
    use moli_fetch::RequestMode;
    let target = ScriptServer::new(200, &[("Access-Control-Allow-Origin", "null")]).await;
    let redirect = ScriptServer::new(
        302,
        &[
            ("Location", target.url.as_str()),
            ("Access-Control-Allow-Origin", "https://page.test"),
        ],
    )
    .await;
    let owner = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let client = owner.handle();
    let mut request = cors_request(
        &redirect.url,
        "https://page.test",
        RequestMode::Cors,
        RequestCredentialsMode::SameOrigin,
    );
    request.request_headers =
        vec![("X-Embedder".to_owned(), "browser-generated".to_owned())].into();
    client
        .fetch_cacheable_script_text_stream(request)
        .await
        .unwrap();
    assert_eq!(
        target.request_count(),
        1,
        "browser headers must not introduce OPTIONS after redirects"
    );
    assert_eq!(target.last_origin().as_deref(), Some("null"));
    assert_eq!(
        target.last_header("x-embedder").as_deref(),
        Some("browser-generated")
    );
}

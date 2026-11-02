use super::*;

fn assert_initial_worker_auth_network_headers(headers: Option<&[(String, String)]>) {
    let headers = headers.expect("worker auth transport request headers");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("host") && !value.is_empty()),
        "worker auth transport headers should contain Host: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("authorization")),
        "the browser-visible auth request observation must remain the initial unauthenticated exchange: {headers:?}"
    );
}

mod fetch_network;
mod file_and_xhr;
mod header_names;
mod opfs_storage;
mod websockets;
mod worker_globals;
mod worker_storage_security;

#[tokio::test]
async fn worker_fetch_referrer_inputs_reach_the_network() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..14 {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_http_request_head(&mut socket).await.unwrap());
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
        requests
    });
    let source = format!(
        "{}\nfetchReferrerInputs('{base}').then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error)); close(); }});",
        include_str!("../../../../tests/fixtures/fetch-referrer-inputs.js"),
    );
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let mut handle = spawn_worker_with_request_client(
        source,
        format!("{base}/context/worker.js?base=1#fragment"),
        ResourceRequestClient::new(&config).unwrap(),
    );
    assert_eq!(recv_post_json(&mut handle).await, "\"pass\"");
    let requests = timeout(TIMEOUT, server).await.unwrap().unwrap();
    for (index, (request, expected)) in requests
        .iter()
        .zip([
            Some("/selected?q=1"),
            Some("/request"),
            Some("/clone"),
            Some("/override"),
            None,
            None,
            Some("/context/worker.js?base=1"),
            Some("/context/worker.js?base=1"),
            Some("/context/relative?q=1"),
            Some("/"),
            Some("/"),
            None,
            Some("/context/worker.js?base=1"),
            Some("/context/worker.js?base=1"),
        ])
        .enumerate()
    {
        assert!(request.starts_with(&format!("GET /echo?case={index} HTTP/1.1\r\n")));
        let actual = request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
            .map(|(_, value)| value.trim());
        assert_eq!(
            actual,
            expected.map(|path| format!("{base}{path}")).as_deref(),
            "case {index}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("\r\nsec-fetch-site: same-origin\r\n")
        );
    }
}

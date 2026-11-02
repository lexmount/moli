use super::*;

#[tokio::test]
async fn worker_fetch_header_name_inputs_preserve_native_wire_case() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..12 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut socket).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                request.len(),
                request
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let source = format!(
        "{}\nfetchHeaderNameInputs('{base}').then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error)); close(); }});",
        include_str!("../../../../../tests/fixtures/fetch-header-name-inputs.js"),
    );
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let mut handle = spawn_worker_with_request_client(
        source,
        format!("{base}/worker.js"),
        ResourceRequestClient::new(&config).unwrap(),
    );
    assert_eq!(recv_post_json(&mut handle).await, "\"pass\"");
    timeout(TIMEOUT, server).await.unwrap().unwrap();
}

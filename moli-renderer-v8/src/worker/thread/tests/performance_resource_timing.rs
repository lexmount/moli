use super::*;

async fn import_resource_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut stream).await.unwrap();
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let path = target.split('?').next().unwrap();
            let response = if path == "/timing-redirect.js" {
                "HTTP/1.1 302 Found\r\nLocation: /timing-import.js?redirected=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
            } else {
                let mut body = if path == "/timing-noop.js" {
                    "/* empty */".to_owned()
                } else {
                    "self.__importTimingDuring.push(performance.getEntriesByType('resource').map(e=>e.name));".to_owned()
                };
                if target.contains("throws=1") {
                    body.push_str("throw new Error('script execution failure');");
                }
                let status = if path == "/timing-missing.js" {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let mime = if path == "/timing-mime.js" {
                    "text/plain"
                } else {
                    "text/javascript"
                };
                let tao = if target.contains("tao=1") {
                    "Timing-Allow-Origin: *\r\n"
                } else {
                    ""
                };
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nCross-Origin-Resource-Policy: cross-origin\r\n{tao}Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (base, server)
}

fn import_resource_client() -> crate::network::ResourceRequestClientOwner {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    ResourceRequestClient::new(&config).unwrap()
}

#[tokio::test]
async fn worker_import_scripts_publish_native_resources_and_process_buffer_overflow() {
    ensure_v8();
    let (base, server) = import_resource_server().await;
    let source = format!(
        r#"const probe = {};
            probe({}).then(checks => {{
                postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
                close();
            }}, error => {{ postMessage({{error: String(error.stack || error)}}); close(); }});"#,
        include_str!("../../../../tests/fixtures/worker-import-resource-timing.js"),
        serde_json::to_string(&base).unwrap(),
    );
    let mut worker = spawn_worker_with_request_client(
        source,
        format!("{base}/worker.js"),
        import_resource_client(),
    );
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["total"], 74, "{result:#}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
    assert!(timeout(TIMEOUT, worker.recv()).await.unwrap().is_none());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn worker_close_retires_pending_resource_buffer_full_tasks() {
    ensure_v8();
    let (base, server) = import_resource_server().await;
    let mut worker = spawn_worker_with_request_client(
        r#"globalThis.__importTimingDuring = [];
            performance.setResourceTimingBufferSize(0);
            performance.onresourcetimingbufferfull = () => postMessage('unexpected event');
            importScripts('./timing-import.js?close=1');
            postMessage({observed: performance.getEntriesByType('resource').length});
            close();"#
            .to_owned(),
        format!("{base}/worker.js"),
        import_resource_client(),
    );
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#"{"observed":0}"#);
    assert!(timeout(TIMEOUT, worker.recv()).await.unwrap().is_none());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

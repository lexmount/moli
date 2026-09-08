use super::*;
use parking_lot::Mutex;

struct CorsServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl CorsServer {
    async fn new(redirect_to: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/data", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_http_request_head(&mut stream).await.unwrap();
                let preflight = request.starts_with("OPTIONS ");
                captured.lock().push(request);
                let origin = if redirect_to.is_some() {
                    "http://worker.test"
                } else {
                    "null"
                };
                let (status, location, body) = if preflight {
                    ("204 No Content", String::new(), "")
                } else if let Some(target) = &redirect_to {
                    (
                        "307 Temporary Redirect",
                        format!("Location: {target}\r\n"),
                        "",
                    )
                } else {
                    ("200 OK", String::new(), "allowed")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-test\r\nContent-Type: text/plain\r\nX-Hidden: secret\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}

impl Drop for CorsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn worker_cors_redirects_validate_null_origin_in_fetch_and_xhr() {
    ensure_v8();
    for api in ["fetch", "xhr", "sync-xhr"] {
        let target = CorsServer::new(None).await;
        let source = CorsServer::new(Some(target.url.clone())).await;
        let loader = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut worker = spawn_worker_with_request_client(
            format!(
                r#"
                (async () => {{
                    try {{
                        const url = {url:?};
                        const api = {api:?};
                        if (api === "fetch") {{
                            const response = await fetch(url, {{ method: "PUT", headers: {{ "X-Test": "1" }} }});
                            postMessage([response.status, await response.text(), response.headers.get("X-Hidden")]);
                        }} else {{
                            const xhr = new XMLHttpRequest();
                            const done = new Promise(resolve => xhr.onloadend = resolve);
                            xhr.open(api === "sync-xhr" ? "GET" : "PUT", url, api !== "sync-xhr");
                            if (api !== "sync-xhr") xhr.setRequestHeader("X-Test", "1");
                            xhr.send();
                            if (api !== "sync-xhr") await done;
                            postMessage([xhr.status, xhr.responseText, xhr.getResponseHeader("X-Hidden")]);
                        }}
                    }} catch (error) {{
                        postMessage(String(error));
                    }} finally {{ close(); }}
                }})();
                "#,
                url = source.url,
            ),
            "http://worker.test/main.js".to_owned(),
            loader,
        );
        let posted = timeout(TIMEOUT, async {
            loop {
                match worker.recv().await.expect("worker channel") {
                    WorkerToParentMessage::Post(payload) => break stringify_payload(&payload),
                    WorkerToParentMessage::SubresourceNetwork(_) => {}
                    message => panic!("unexpected worker message: {message:?}"),
                }
            }
        })
        .await
        .expect("worker CORS redirect result");
        assert_eq!(posted, r#"[200,"allowed",null]"#, "{api}");
        let requests = target.requests.lock();
        let expected_requests = if api == "sync-xhr" { 1 } else { 2 };
        assert_eq!(requests.len(), expected_requests, "{api}");
        assert!(
            requests
                .last()
                .unwrap()
                .starts_with(if api == "sync-xhr" { "GET " } else { "PUT " }),
            "{api}: actual request must reach the target"
        );
        for request in requests.iter() {
            assert!(
                request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("origin: null")),
                "{api}: {request}"
            );
        }
    }
}

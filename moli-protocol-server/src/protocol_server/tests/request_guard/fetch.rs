use super::*;
use moli_core::runtime::{Browser, BrowserConfig};

const HEADER_PROBES: &str = r#"
async function probeRequestHeaders(options) {
    const results = [];
    for (const mode of [undefined, 'cors', 'same-origin', 'no-cors']) {
        for (const method of options.methods) {
            for (const input of ['url', 'request', 'override']) {
                const url = method === 'POST' ? '/session' : '/json/version';
                const headers = input === 'url' ? options.headers
                    : input === 'request' ? new Headers(options.headers)
                    : Object.entries(options.headers);
                const init = {method, mode, headers};
                if (method === 'POST')
                    init.body = JSON.stringify({capabilities: {alwaysMatch: {}}});
                const response = input === 'url' ? await fetch(url, init)
                    : input === 'request' ? await fetch(new Request(url, init))
                    : await fetch(new Request(url, {method, mode}), init);
                await response.text();
                results.push({mode: mode ?? 'default', method, input, status: response.status});
            }
        }
    }
    return results;
}
"#;

struct HeaderProbeServer {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HeaderProbeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HeaderProbeServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = protocol_server_test_state(
            addr,
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
        );
        // The fixture serves the author's page, then routes its same-origin
        // protocol requests through the actual server guard on the same port.
        let app = Router::new()
            .route(
                "/author-page",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/html")],
                        "<!doctype html><title>Header guard fixture</title>",
                    )
                }),
            )
            .route(
                "/header-worker.js",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/javascript")],
                        format!(
                            "{HEADER_PROBES}\nonmessage = async (event) => {{
                                try {{ postMessage({{results: await probeRequestHeaders(event.data)}}); }}
                                catch (error) {{ postMessage({{error: String(error)}}); }}
                            }};"
                        ),
                    )
                }),
            )
            .fallback_service(build_router(state));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, task }
    }
}

async fn assert_author_fetch_headers_are_guarded(trusted_headers: bool) {
    let server = HeaderProbeServer::spawn().await;
    let mut config = BrowserConfig::default();
    config.fetch_mut().set_http_no_proxy(Some("*".into()));
    config.fetch_mut().set_http_host_resolve(vec![format!(
        "header-guard.test:{}:127.0.0.1",
        server.addr.port()
    )]);
    let browser = Browser::new(config).unwrap();
    let mut page = browser
        .fetch(&format!(
            "http://header-guard.test:{}/author-page",
            server.addr.port()
        ))
        .await
        .unwrap();
    let options = if trusted_headers {
        page.set_extra_http_headers_async(
            &vec![
                ("Host".to_owned(), server.addr.to_string()),
                ("Origin".to_owned(), format!("http://{}", server.addr)),
            ]
            .into(),
        )
        .await
        .unwrap();
        json!({
            "methods": ["GET"],
            "headers": {"hOsT": "script.invalid", "oRiGiN": "http://script.invalid"}
        })
    } else {
        json!({
            "methods": ["GET", "POST"],
            "headers": {"hOsT": server.addr.to_string(), "oRiGiN": format!("http://{}", server.addr)}
        })
    };
    let expression = format!(
        r#"(async () => {{
            {HEADER_PROBES}
            const options = {options};
            const windowResults = await probeRequestHeaders(options);
            const worker = new Worker('/header-worker.js');
            try {{
                const workerResults = await new Promise((resolve, reject) => {{
                    worker.onmessage = (event) => event.data.error
                        ? reject(new Error(event.data.error)) : resolve(event.data.results);
                    worker.onerror = (event) => reject(new Error(event.message));
                    worker.postMessage(options);
                }});
                return JSON.stringify({{window: windowResults, worker: workerResults}});
            }} finally {{ worker.terminate(); }}
        }})()"#
    );
    let value = timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await
    .expect("header probes must finish")
    .unwrap();
    let results: serde_json::Value =
        serde_json::from_str(value["value"].as_str().unwrap()).unwrap();
    let expected = if trusted_headers { 200 } else { 403 };
    for realm in ["window", "worker"] {
        let probes = results[realm].as_array().unwrap();
        assert_eq!(probes.len(), if trusted_headers { 12 } else { 24 });
        for probe in probes {
            assert_eq!(probe["status"], expected, "{realm}: {probe}");
        }
    }
    let (status, body) = fetch_server_response(server.addr, "GET", "/status").await;
    assert_eq!(status, 200);
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["value"]["ready"], true, "no session may be created");
    assert_eq!(
        fetch_server_response(server.addr, "GET", "/json/version")
            .await
            .0,
        200,
        "native clients must remain usable"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn author_fetch_cannot_spoof_protocol_headers_through_host_resolution() {
    assert_author_fetch_headers_are_guarded(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_extra_headers_survive_author_fetch_header_filtering() {
    assert_author_fetch_headers_are_guarded(true).await;
}

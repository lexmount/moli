use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

struct ConnectionTimingServer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for ConnectionTimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ConnectionTimingServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut socket, _)) = accepted else { break };
                        let observed = observed.clone();
                        connections.spawn(async move {
                            let mut request = Vec::new();
                            while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                                let Ok(byte) = socket.read_u8().await else { return };
                                request.push(byte);
                            }
                            let request = String::from_utf8_lossy(&request);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            observed.lock().push(path.to_owned());
                            let (mime, body) = if path.starts_with("/page") {
                                ("text/html", "<!doctype html><body>connection timing</body>")
                            } else if path == "/sw.js" {
                                ("text/javascript", r#"
                                    self.addEventListener('install', event => event.waitUntil(skipWaiting()));
                                    self.addEventListener('activate', event => event.waitUntil(clients.claim()));
                                    self.addEventListener('fetch', event => {
                                        const path = new URL(event.request.url).pathname;
                                        if (path === '/synthetic')
                                            event.respondWith(new Response('synthetic'));
                                        else if (path === '/forward')
                                            event.respondWith(fetch('/network'));
                                    });
                                "#)
                            } else {
                                ("text/plain", "network")
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}", body.len()
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            origin,
            requests,
            task,
        })
    }
}

async fn check_service_worker_connection_timing(iframe: bool) -> Result<()> {
    let server = ConnectionTimingServer::spawn().await?;
    let remote = ConnectionTimingServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let expression = format!(
        r#"(async () => {{
            const registration = await navigator.serviceWorker.register('/sw.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller)
                await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once:true}}));
            let realm = window;
            if ({iframe}) {{
                const frame = document.createElement('iframe');
                frame.src = '/page?child';
                await new Promise(resolve => {{ frame.onload = resolve; document.body.appendChild(frame); }});
                realm = frame.contentWindow;
            }}
            await new Promise(resolve => setTimeout(resolve, 20));
            const results = [];
            for (const path of ['/synthetic', '/forward', '/network', {remote:?} + '/network']) {{
                const url = new URL(path, location.href).href;
                const body = await (await realm.fetch(url)).text();
                const entries = realm.performance.getEntriesByName(url, 'resource').map(e => ({{
                    fetchStart:e.fetchStart, domainLookupStart:e.domainLookupStart,
                    domainLookupEnd:e.domainLookupEnd, connectStart:e.connectStart, connectEnd:e.connectEnd,
                    secureConnectionStart:e.secureConnectionStart, responseStart:e.responseStart,
                    responseEnd:e.responseEnd, nextHopProtocol:e.nextHopProtocol, transferSize:e.transferSize
                }}));
                results.push({{path, body, entries, topCount:performance.getEntriesByName(url, 'resource').length}});
            }}
            await registration.unregister();
            return JSON.stringify(results);
        }})()"#,
        remote = remote.origin,
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    assert!(result.get("exceptionDetails").is_none(), "{result}");
    let results: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("missing timing results: {result}")),
    )?;
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), 4);
    for (index, result) in results.iter().enumerate() {
        assert_eq!(
            result["body"],
            if index == 0 { "synthetic" } else { "network" }
        );
        assert_eq!(result["topCount"], if iframe { 0 } else { 1 });
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(
            entries.len(),
            1,
            "one timing in the requesting Window: {result}"
        );
        let entry = &entries[0];
        let fetch_start = entry["fetchStart"].as_f64().unwrap();
        assert!(fetch_start > 0.0, "{result}");
        assert!(
            entry["responseEnd"].as_f64().unwrap() >= fetch_start,
            "{result}"
        );
        if index < 2 {
            for field in [
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
            ] {
                assert_eq!(entry[field], entry["fetchStart"], "{field}: {result}");
            }
            assert_eq!(
                entry["secureConnectionStart"], 0,
                "no client TLS connection: {result}"
            );
            assert_eq!(
                entry["nextHopProtocol"], "",
                "no client HTTP connection: {result}"
            );
        } else if index == 2 {
            assert!(
                entry["connectStart"].as_f64().unwrap() >= fetch_start,
                "{result}"
            );
            assert!(
                entry["connectEnd"].as_f64().unwrap() >= entry["connectStart"].as_f64().unwrap(),
                "{result}"
            );
            assert_eq!(entry["nextHopProtocol"], "http/1.1", "{result}");
            assert!(entry["transferSize"].as_f64().unwrap() > 300.0, "{result}");
        } else {
            for field in [
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
                "secureConnectionStart",
                "responseStart",
                "transferSize",
            ] {
                assert_eq!(
                    entry[field], 0,
                    "cross-origin response without TAO: {field}: {result}"
                );
            }
            assert_eq!(entry["nextHopProtocol"], "", "{result}");
        }
    }
    let requests = server.requests.lock();
    assert!(
        !requests
            .iter()
            .any(|path| path == "/synthetic" || path == "/forward")
    );
    assert_eq!(
        requests.iter().filter(|path| *path == "/network").count(),
        2
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_response_without_connection_uses_window_fetch_start() -> Result<()> {
    check_service_worker_connection_timing(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_response_without_connection_uses_child_fetch_start() -> Result<()> {
    check_service_worker_connection_timing(true).await
}

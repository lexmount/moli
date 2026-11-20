use super::*;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};

struct TimingServer {
    origin: String,
    finish_stream: Arc<Semaphore>,
    task: JoinHandle<()>,
}

impl Drop for TimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TimingServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let finish_stream = Arc::new(Semaphore::new(0));
        let finish = finish_stream.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let finish = finish.clone();
                connections.spawn(async move {
                    let mut request = Vec::new();
                    while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                        let mut buf = [0; 2048];
                        let count = socket.read(&mut buf).await.unwrap();
                        if count == 0 { return; }
                        request.extend_from_slice(&buf[..count]);
                    }
                    let text = String::from_utf8(request).unwrap();
                    let path = text.split_whitespace().nth(1).unwrap();
                    if path == "/disconnect" {
                        return;
                    }
                    if path.starts_with("/redirect") {
                        socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /events\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                        return;
                    }
                    let (status, mime, body): (&str, &str, &[u8]) = if path == "/page" {
                        ("200 OK", "text/html", b"<!doctype html><body>EventSource timing</body>")
                    } else if path == "/sw.js" {
                        ("200 OK", "text/javascript", br#"
                            self.addEventListener('install', event => event.waitUntil(skipWaiting()));
                            self.addEventListener('activate', event => event.waitUntil(clients.claim()));
                            self.addEventListener('fetch', event => {
                                if (new URL(event.request.url).pathname === '/synthetic')
                                    event.respondWith(new Response('retry: 60000\ndata: synthetic\n\n', {headers:{'Content-Type':'text/event-stream'}}));
                            });
                        "#)
                    } else if path == "/no-content" {
                        ("204 No Content", "text/event-stream", b"")
                    } else if path == "/wrong-mime" {
                        ("200 OK", "text/plain", b"not an event stream")
                    } else {
                        ("200 OK", "text/event-stream", b"retry: 10\ndata: hello\n\n")
                    };
                    let cors = if path.contains("deny") { "" } else { "Access-Control-Allow-Origin: *\r\n" };
                    let tao = if path.contains("tao") { "Timing-Allow-Origin: *\r\n" } else { "" };
                    let length = body.len() + usize::from(path.starts_with("/stream"));
                    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nCache-Control: no-store\r\n{cors}{tao}Connection: close\r\n\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    if path.starts_with("/stream") {
                        finish.acquire().await.unwrap().forget();
                        let _ = socket.write_all(b"\n").await;
                    }
                });
                while let Some(result) = connections.try_join_next() {
                    result.unwrap();
                }
            }
        });
        Self {
            origin,
            finish_stream,
            task,
        }
    }
}

const SNAPSHOT: &str = r#"entry => ({name:entry.name, initiatorType:entry.initiatorType,
    responseStatus:entry.responseStatus, startTime:entry.startTime,
    responseStart:entry.responseStart, responseEnd:entry.responseEnd,
    duration:entry.duration, encodedBodySize:entry.encodedBodySize,
    decodedBodySize:entry.decodedBodySize, transferSize:entry.transferSize,
    redirectStart:entry.redirectStart, redirectEnd:entry.redirectEnd})"#;

async fn evaluate_timing(
    page: &mut moli_core::page::Page,
    expression: &str,
) -> Result<serde_json::Value> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(expression, true),
    )
    .await??;
    assert!(result.get("exceptionDetails").is_none(), "{result}");
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("timing probe did not fulfill: {result}")),
    )?;
    assert!(value.get("error").is_none(), "{value}");
    Ok(value)
}

fn timing_probe(url: &str, count: usize, close_on_message: bool) -> String {
    format!(
        r#"new Promise((resolve, reject) => {{
        const url = {url:?}; let messages = 0;
        const source = new EventSource(url);
        const timer = setTimeout(() => {{ source.close(); observer.disconnect(); resolve(JSON.stringify({{error:'missing EventSource timing',messages,timingCount:performance.getEntriesByName(url).length}})); }}, 5000);
        const observer = new PerformanceObserver(() => {{
            const entries = performance.getEntriesByName(url);
            if (entries.length < {count}) return;
            source.close(); observer.disconnect(); clearTimeout(timer);
            resolve(JSON.stringify({{messages, entries:entries.map({SNAPSHOT})}}));
        }});
        observer.observe({{type:'resource'}});
        source.onmessage = () => {{ messages++; if ({close_on_message}) source.close(); }};
    }})"#
    )
}

fn assert_entry(entry: &serde_json::Value, url: &str, status: u64) {
    assert_eq!(entry["name"], url, "{entry}");
    assert_eq!(entry["initiatorType"], "other", "{entry}");
    assert_eq!(entry["responseStatus"], status, "{entry}");
    assert!(
        entry["responseEnd"].as_f64().unwrap() >= entry["startTime"].as_f64().unwrap(),
        "{entry}"
    );
    assert!(entry["duration"].as_f64().unwrap() >= 0.0, "{entry}");
}

#[tokio::test(flavor = "multi_thread")]
async fn window_event_source_records_each_reconnection_once() -> Result<()> {
    let server = TimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let url = format!("{}/events", server.origin);
    let result = evaluate_timing(&mut page, &timing_probe(&url, 2, false)).await?;
    assert_eq!(result["messages"], 2, "{result}");
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert_entry(entry, &url, 200);
        assert_eq!(
            entry["encodedBodySize"],
            b"retry: 10\ndata: hello\n\n".len()
        );
        assert_eq!(entry["decodedBodySize"], entry["encodedBodySize"]);
    }
    assert!(
        entries[1]["startTime"].as_f64().unwrap() >= entries[0]["responseEnd"].as_f64().unwrap()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn window_event_source_timing_waits_for_the_body_terminal() -> Result<()> {
    let server = TimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let url = format!("{}/stream", server.origin);
    let expression = format!(
        r#"new Promise((resolve, reject) => {{
        globalThis.source = new EventSource({url:?});
        source.onerror = () => reject(new Error('stream failed'));
        source.onmessage = () => resolve(JSON.stringify(performance.getEntriesByName({url:?}).length));
        globalThis.completedTiming = new Promise(done => {{
            const observer = new PerformanceObserver(() => {{
                const entries = performance.getEntriesByName({url:?});
                if (!entries.length) return;
                source.close(); observer.disconnect(); done(JSON.stringify(entries.map({SNAPSHOT})));
            }});
            observer.observe({{type:'resource'}});
        }});
    }})"#
    );
    assert_eq!(evaluate_timing(&mut page, &expression).await?, 0);
    server.finish_stream.add_permits(1);
    let entries = evaluate_timing(&mut page, "completedTiming").await?;
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_entry(&entries[0], &url, 200);
    assert_eq!(
        entries[0]["decodedBodySize"],
        b"retry: 10\ndata: hello\n\n\n".len()
    );
    Ok(())
}

use super::*;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct TimingServerState {
    seen: HashSet<String>,
    gates: HashMap<String, Arc<tokio::sync::Notify>>,
}

struct TimingServer {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn timing_server() -> TimingServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(TimingServerState::default()));
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let state = state.clone();
            connections.spawn(handle_timing_request(socket, state));
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    TimingServer { origin, task }
}

async fn handle_timing_request(
    mut socket: tokio::net::TcpStream,
    state: Arc<Mutex<TimingServerState>>,
) {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 2048];
        let count = socket.read(&mut buffer).await.unwrap();
        if count == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8(request).unwrap();
    let target = request.split_whitespace().nth(1).unwrap();
    let url = url::Url::parse(&format!("http://fixture.test{target}")).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let get = |name: &str| query.get(name).map(String::as_str).unwrap_or("");
    let key = get("key").to_owned();
    let gate = {
        let mut state = state.lock();
        state.seen.insert(key.clone());
        state.gates.entry(key).or_default().clone()
    };
    if url.path() == "/fail" {
        return;
    }
    use base64::Engine;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jL1kAAAAASUVORK5CYII=";
    let mut body = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    let mut mime = "image/png";
    if url.path() == "/release" {
        gate.notify_one();
        body = b"ok".to_vec();
        mime = "text/plain";
    } else if url.path() == "/seen" {
        body = state
            .lock()
            .seen
            .contains(get("target"))
            .to_string()
            .into_bytes();
        mime = "application/json";
    }
    if get("gate") == "before" {
        gate.notified().await;
    }
    let redirect = url.path() == "/redirect";
    if redirect {
        body.clear();
    }
    let status = get("status")
        .parse::<u16>()
        .unwrap_or(if redirect { 302 } else { 200 });
    if status == 204 {
        body.clear();
    }
    let mut headers = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if redirect {
        headers.push_str(&format!("Location: {}\r\n", get("to")));
    }
    for (query_name, header_name) in [
        ("cors", "Access-Control-Allow-Origin"),
        ("tao", "Timing-Allow-Origin"),
        ("tao2", "Timing-Allow-Origin"),
    ] {
        if !get(query_name).is_empty() {
            headers.push_str(&format!("{header_name}: {}\r\n", get(query_name)));
        }
    }
    headers.push_str("\r\n");
    if socket.write_all(headers.as_bytes()).await.is_err() {
        return;
    }
    if get("truncate") == "1" {
        body.truncate(4);
    }
    if get("gate") == "body" {
        if socket.write_all(&body[..4]).await.is_err() {
            return;
        }
        gate.notified().await;
        body.drain(..4);
    }
    let _ = socket.write_all(&body).await;
}

#[tokio::test(flavor = "current_thread")]
async fn window_fetch_reports_native_resource_timing_at_body_terminal() {
    let first = timing_server().await;
    let second = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", first.origin), &loader);
    page.eval(&format!(
        "globalThis.__fetchOrigins = {}; globalThis.__fetchTimingResult = null;",
        serde_json::to_string(&[&first.origin, &second.origin]).unwrap(),
    ))
    .unwrap();
    let fixture = include_str!("../../../tests/fixtures/fetch-resource-timing.js");
    page.eval(&format!(
        "({}).then(value => __fetchTimingResult = value, error => __fetchTimingResult = String(error));",
        fixture.trim().trim_end_matches(';'),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(__fetchTimingResult !== null)",
        "true",
        "fetch Resource Timing",
    )
    .await;
    let result = page.eval("__fetchTimingResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result)
        .unwrap_or_else(|error| panic!("fetch timing probe returned {result:?}: {error}"));
    assert_eq!(result["total"], 36);
    assert_eq!(result["failures"], serde_json::json!([]));
}

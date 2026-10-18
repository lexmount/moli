use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct BodyNetworkServerState {
    seen: HashSet<String>,
    gates: HashMap<String, Arc<tokio::sync::Notify>>,
}

pub(super) struct BodyNetworkServer {
    pub(super) origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for BodyNetworkServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn body_network_server() -> BodyNetworkServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(BodyNetworkServerState::default()));
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let state = state.clone();
            connections.spawn(handle_body_network_request(socket, state));
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    BodyNetworkServer { origin, task }
}

async fn handle_body_network_request(
    mut socket: tokio::net::TcpStream,
    state: Arc<Mutex<BodyNetworkServerState>>,
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
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if url.path() == "/mime" {
        for (_, value) in url.query_pairs().filter(|(name, _)| name == "value") {
            headers.push_str(&format!("Content-Type: {value}\r\n"));
        }
    } else {
        headers.push_str(&format!("Content-Type: {mime}\r\n"));
    }
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

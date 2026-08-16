use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{WebSocketUpgrade, ws::Message},
    http::{Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_util::{SinkExt, StreamExt};
use moli_browser_profile::{BrowserProfileLock, BrowserProfilePaths};
use moli_cookie_jar::{StoredCookieSameSite, StoredCookieSourceScheme};
use moli_protocol_webdriver_classic::{
    CLASSIC_ELEMENT_REFERENCE_KEY, CLASSIC_FRAME_REFERENCE_KEY, CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
    CLASSIC_WINDOW_REFERENCE_KEY,
};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::util::ServiceExt;

struct DedicatedFixtureServer {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for DedicatedFixtureServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_dedicated_fixture_server(
    app: Router,
    name: &'static str,
) -> (std::net::SocketAddr, DedicatedFixtureServer) {
    let app = app.route("/__moli_fixture_ready", get(|| async { "ok" }));
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| panic!("bind {name} fixture listener: {error}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|error| panic!("set {name} fixture listener nonblocking: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("read {name} fixture listener addr: {error}"));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::Builder::new()
        .name(format!("moli-{name}-fixture"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|error| panic!("build {name} fixture runtime: {error}"));
            runtime.block_on(async move {
                let listener = TcpListener::from_std(listener)
                    .unwrap_or_else(|error| panic!("adopt {name} fixture listener: {error}"));
                let _ = ready_tx.send(());
                let server = axum::serve(listener, app);
                tokio::select! {
                    result = server => {
                        result.unwrap_or_else(|error| panic!("{name} fixture server failed: {error}"));
                    }
                    _ = shutdown_rx => {}
                }
            });
        })
        .unwrap_or_else(|error| panic!("spawn {name} fixture thread: {error}"));
    ready_rx
        .recv()
        .unwrap_or_else(|error| panic!("wait for {name} fixture listener: {error}"));
    wait_for_dedicated_fixture_ready(addr, name);
    (
        addr,
        DedicatedFixtureServer {
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
        },
    )
}

fn spawn_shared_worker_fixture_server(
    name: &'static str,
) -> (std::net::SocketAddr, DedicatedFixtureServer) {
    let app = Router::new()
        .route(
            "/",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/html")],
                    r#"<!doctype html><html><body>shared worker
<script>
globalThis.__sharedWorkerProbe = value => new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error('shared worker timeout')), 1000);
  const worker = new SharedWorker('/shared-worker.js', 'webdriver-shared-worker-smoke');
  globalThis.__sharedWorkerSmoke = worker;
  worker.port.onmessage = event => {
    if (event.data && event.data.kind === 'probe-result') {
      clearTimeout(timer);
      resolve(event.data);
    }
  };
  worker.port.start();
  worker.port.postMessage({ kind: 'probe', value });
});
</script></body></html>"#,
                )
            }),
        )
        .route(
            "/shared-worker.js",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "globalThis.__sharedWorkerConnectCount = 0;\
                     self.onconnect = event => {\
                     globalThis.__sharedWorkerConnectCount += 1;\
                     const port = event.ports[0];\
                     port.onmessage = message => {\
                     const data = message.data;\
                     if (data && data.kind === 'probe') {\
                     port.postMessage({\
                     kind: 'probe-result',\
                     echoed: data.value,\
                     name,\
                     pathname: self.location.pathname,\
                     isSharedWorker: typeof SharedWorkerGlobalScope !== 'undefined' && self instanceof SharedWorkerGlobalScope,\
                     selfEqualsGlobal: self === globalThis,\
                     connectCount: globalThis.__sharedWorkerConnectCount\
                     });\
                     }\
                     };\
                     port.start();\
                     };",
                )
            }),
        );
    spawn_dedicated_fixture_server(app, name)
}

fn wait_for_dedicated_fixture_ready(addr: std::net::SocketAddr, name: &'static str) {
    let mut stream = std::net::TcpStream::connect(addr)
        .unwrap_or_else(|error| panic!("connect to {name} fixture readiness route: {error}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap_or_else(|error| panic!("set {name} fixture readiness read timeout: {error}"));
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap_or_else(|error| panic!("set {name} fixture readiness write timeout: {error}"));
    stream
        .write_all(
            b"GET /__moli_fixture_ready HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .unwrap_or_else(|error| panic!("write {name} fixture readiness request: {error}"));

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .unwrap_or_else(|error| panic!("read {name} fixture readiness response: {error}"));
    assert!(
        response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"),
        "{name} fixture readiness route returned unexpected response: {response:?}"
    );
}

struct TempDir {
    path: PathBuf,
}

struct TempPath {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-protocol-{name}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        Self { path }
    }
}

impl TempPath {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("moli-cdp-{name}-{}-{nonce}", std::process::id()));
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stored_cookie(name: &str, value: &str) -> StoredCookie {
    StoredCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        host_only: false,
        path: "/".to_owned(),
        secure: false,
        http_only: false,
        expires: None,
        same_site: StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: StoredCookieSourceScheme::NonSecure,
        source_port: -1,
        creation_index: 0,
        last_access_index: 0,
    }
}

fn test_state() -> AppState {
    protocol_server_test_state(
        "127.0.0.1:9222".parse().expect("test addr"),
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
    )
}

fn protocol_server_test_fetch_config(mut fetch_config: FetchConfig) -> FetchConfig {
    if fetch_config.http_no_proxy().is_none() {
        fetch_config.set_http_no_proxy(Some("*".to_owned()));
    }
    fetch_config
}

fn protocol_server_test_state(
    addr: std::net::SocketAddr,
    fetch_config: FetchConfig,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
) -> AppState {
    let storage_partition = Arc::new(
        StoragePartitionState::open(None).expect("test storage partition should initialize"),
    );
    AppState::new_with_storage_partition_and_runtime_config(
        addr,
        storage_partition,
        protocol_server_test_runtime_config(
            protocol_server_test_fetch_config(fetch_config),
            optional_resource_fetch_mask,
        ),
    )
    .expect("test app state should initialize")
}

async fn request_json(path: &str) -> serde_json::Value {
    request_json_with_method(Method::GET, path).await
}

async fn request_json_with_method(method: Method, path: &str) -> serde_json::Value {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::OK, "path {path}");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&body).expect("json response")
}

async fn request_status(path: &str) -> StatusCode {
    request_status_with_method(Method::GET, path).await
}

async fn request_status_with_method(method: Method, path: &str) -> StatusCode {
    request_status_and_text_with_method(method, path).await.0
}

async fn request_status_and_text_with_method(method: Method, path: &str) -> (StatusCode, String) {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        String::from_utf8(body.to_vec()).expect("utf-8 body"),
    )
}

async fn spawn_test_protocol_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_protocol_server_with_fetch_config_and_optional_resource_fetch_mask(
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
    )
    .await
}

async fn spawn_test_protocol_server_with_owner_registry() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    SharedCdpOwnerRegistry,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test protocol server");
    let addr = listener.local_addr().expect("test protocol server addr");
    let state = protocol_server_test_state(
        addr,
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
    );
    let owner_registry = state.cdp_owner_registry.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state))
            .await
            .expect("test protocol server should serve");
    });
    (addr, server, owner_registry)
}

async fn spawn_test_protocol_server_with_fetch_config(
    fetch_config: FetchConfig,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_protocol_server_with_fetch_config_and_optional_resource_fetch_mask(
        fetch_config,
        OptionalResourceFetchMask::NONE,
    )
    .await
}

async fn spawn_test_protocol_server_with_image_fetch_enabled(
    image_fetch_enabled: bool,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_protocol_server_with_fetch_config_and_optional_resource_fetch_mask(
        FetchConfig::default(),
        if image_fetch_enabled {
            OptionalResourceFetchMask::IMAGE
        } else {
            OptionalResourceFetchMask::NONE
        },
    )
    .await
}

async fn spawn_test_protocol_server_with_fetch_config_and_optional_resource_fetch_mask(
    fetch_config: FetchConfig,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let fetch_config = protocol_server_test_fetch_config(fetch_config);
    spawn_test_protocol_server_with_runtime_config(protocol_server_test_runtime_config(
        fetch_config,
        optional_resource_fetch_mask,
    ))
    .await
}

fn protocol_server_test_runtime_config(
    fetch_config: FetchConfig,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
) -> NavigationRuntimeConfig {
    NavigationRuntimeConfig::new(
        fetch_config,
        optional_resource_fetch_mask,
        true,
        LayoutPolicy::OnDemand,
    )
}

async fn spawn_test_protocol_server_with_layout_policy(
    layout_policy: LayoutPolicy,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_protocol_server_with_runtime_config(NavigationRuntimeConfig::new(
        protocol_server_test_fetch_config(FetchConfig::default()),
        OptionalResourceFetchMask::NONE,
        true,
        layout_policy,
    ))
    .await
}

async fn spawn_test_protocol_server_with_runtime_config(
    navigation_runtime_config: NavigationRuntimeConfig,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test protocol server");
    let addr = listener.local_addr().expect("test protocol server addr");
    let storage_partition = Arc::new(
        StoragePartitionState::open(None).expect("test storage partition should initialize"),
    );
    let state = AppState::new_with_storage_partition_and_runtime_config(
        addr,
        storage_partition,
        navigation_runtime_config,
    )
    .expect("test app state should initialize");
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state))
            .await
            .expect("test protocol server should serve");
    });
    (addr, server)
}

async fn spawn_delayed_download_fixture_server(
    body: &'static str,
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed download fixture server");
    let addr = listener
        .local_addr()
        .expect("delayed download fixture server addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><a id=\"dl\" href=\"/download\" download=\"saved.txt\" style=\"display:inline-block;width:200px;height:200px\">download</a></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                "attachment; filename=\"saved.txt\"",
                            ),
                        ],
                        body,
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("delayed download fixture server should serve");
    });
    (addr, server)
}

async fn spawn_delayed_plain_attachment_fixture_server(
    body: &'static str,
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed plain attachment fixture server");
    let addr = listener
        .local_addr()
        .expect("delayed plain attachment fixture server addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><a id=\"dl\" href=\"/download\" style=\"display:inline-block;width:200px;height:200px\">download</a></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                "attachment; filename=\"saved.txt\"",
                            ),
                        ],
                        body,
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("delayed plain attachment fixture server should serve");
    });
    (addr, server)
}

async fn spawn_post_parse_location_download_fixture_server(
    body: &'static str,
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind post-parse location download fixture server");
    let addr = listener
        .local_addr()
        .expect("post-parse location download fixture server addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id=\"source\">source</main><script>window.addEventListener('load', () => setTimeout(() => location.assign('/download'), 0));</script></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                "attachment; filename=\"saved.txt\"",
                            ),
                        ],
                        body,
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("post-parse location download fixture server should serve");
    });
    (addr, server)
}

async fn spawn_delayed_content_disposition_download_fixture_server(
    body: &'static str,
    delay: Duration,
    content_disposition: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed content disposition fixture server");
    let addr = listener
        .local_addr()
        .expect("delayed content disposition fixture server addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><a id=\"dl\" href=\"/download\" download style=\"display:inline-block;width:200px;height:200px\">download</a></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                content_disposition,
                            ),
                        ],
                        body,
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("delayed content disposition fixture server should serve");
    });
    (addr, server)
}

async fn spawn_local_storage_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind localStorage fixture server");
    let addr = listener
        .local_addr()
        .expect("localStorage fixture server addr");
    let server = tokio::spawn(async move {
        let app = Router::new().route(
            "/page",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body><main>localstorage fixture</main></body></html>",
                )
            }),
        );
        axum::serve(listener, app)
            .await
            .expect("localStorage fixture server should serve");
    });
    (addr, server)
}

async fn spawn_response_stage_streaming_document_fixture_server(
    release_tail: Arc<tokio::sync::Notify>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_response_stage_streaming_document_fixture_server_with_head_signal(release_tail, None)
        .await
}

async fn spawn_response_stage_streaming_document_fixture_server_with_head_signal(
    release_tail: Arc<tokio::sync::Notify>,
    response_head_sent: Option<Arc<tokio::sync::Notify>>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind response-stage streaming fixture server");
    let addr = listener
        .local_addr()
        .expect("response-stage streaming fixture addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept response-stage streaming request");
        let mut request = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buf)
                .await
                .expect("read response-stage streaming request");
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buf[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        let head = b"<!doctype html><html><body><main id=\"head\">head";
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response-stage streaming response head");
        stream
            .write_all(format!("{:x}\r\n", head.len()).as_bytes())
            .await
            .expect("write response-stage streaming head chunk size");
        stream
            .write_all(head)
            .await
            .expect("write response-stage streaming head chunk");
        stream
            .write_all(b"\r\n")
            .await
            .expect("write response-stage streaming head chunk terminator");
        if let Some(response_head_sent) = response_head_sent {
            response_head_sent.notify_one();
        }

        release_tail.notified().await;
        let tail = b" tail</main></body></html>";
        stream
            .write_all(format!("{:x}\r\n", tail.len()).as_bytes())
            .await
            .expect("write response-stage streaming tail chunk size");
        stream
            .write_all(tail)
            .await
            .expect("write response-stage streaming tail chunk");
        stream
            .write_all(b"\r\n0\r\n\r\n")
            .await
            .expect("finish response-stage streaming body");
        let _ = stream.shutdown().await;
    });
    (addr, server)
}

async fn spawn_profiled_test_protocol_server(
    profile_dir: PathBuf,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_profiled_test_protocol_server_with_cookie_profile(profile_dir, Vec::new()).await
}

async fn spawn_profiled_test_protocol_server_with_cookie_profile(
    profile_dir: PathBuf,
    initial_cookies: Vec<StoredCookie>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let fetch_config = protocol_server_test_fetch_config(FetchConfig::default());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind profiled test protocol server");
    let addr = listener
        .local_addr()
        .expect("profiled test protocol server addr");
    let storage_partition = Arc::new(
        StoragePartitionState::open(Some(&profile_dir)).expect("profiled partition should open"),
    );
    storage_partition
        .import_cookies(initial_cookies)
        .expect("profiled initial cookies should import");
    let state = AppState::new_with_storage_partition(
        addr,
        storage_partition,
        fetch_config,
        OptionalResourceFetchMask::NONE,
        true,
    )
    .expect("profiled app state should initialize");
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state))
            .await
            .expect("profiled test protocol server should serve");
    });
    (addr, server)
}

async fn abort_test_cdp_server(server: tokio::task::JoinHandle<()>) {
    server.abort();
    let _ = server.await;
}

async fn recv_ws_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> serde_json::Value {
    loop {
        match socket.next().await.expect("websocket message") {
            Ok(WsMessage::Text(text)) => {
                return serde_json::from_str(&text).expect("json websocket message");
            }
            Ok(WsMessage::Binary(bytes)) => {
                return serde_json::from_slice(&bytes).expect("json websocket message");
            }
            Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => continue,
            Ok(other) => panic!("unexpected websocket message: {other:?}"),
            Err(error) => panic!("websocket recv failed: {error:?}"),
        }
    }
}

async fn recv_until_id(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected_id: u64,
) -> Vec<serde_json::Value> {
    // Test websocket helpers should fail with the messages seen so far instead
    // of letting a missing response hang the whole nextest run.
    const RECV_UNTIL_ID_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + RECV_UNTIL_ID_TIMEOUT;
    loop {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "recv_until_id timed out after {:?} waiting for id {}; \
                     received messages so far: {messages:#?}",
                    RECV_UNTIL_ID_TIMEOUT, expected_id
                )
            });
        let done = message["id"] == json!(expected_id);
        messages.push(message);
        if done {
            return messages;
        }
    }
}

async fn recv_until_match(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut predicate: impl FnMut(&serde_json::Value) -> bool,
) -> Vec<serde_json::Value> {
    // CDP websocket tests run alongside many in-process HTTP servers, V8
    // runtimes, and renderer tasks under full nextest parallelism. This is a
    // test harness budget, not a protocol timing expectation; the predicates
    // below still assert the exact event or response contract.
    const RECV_UNTIL_MATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + RECV_UNTIL_MATCH_TIMEOUT;
    loop {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "recv_until_match timed out after {:?} waiting for predicate match; \
                     received messages so far: {messages:#?}",
                    RECV_UNTIL_MATCH_TIMEOUT
                )
            });
        let done = predicate(&message);
        messages.push(message);
        if done {
            return messages;
        }
    }
}

async fn recv_cdp_messages_for(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    duration: std::time::Duration,
) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + duration;
    while let Ok(message) = tokio::time::timeout_at(deadline, recv_ws_json(socket)).await {
        messages.push(message);
    }
    messages
}

async fn send_cdp_command_without_wait(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    session_id: Option<&str>,
    params: serde_json::Value,
) {
    let mut command = json!({
        "id": id,
        "method": method,
        "params": params,
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    socket
        .send(WsMessage::Text(command.to_string().into()))
        .await
        .unwrap_or_else(|error| panic!("send {method}: {error}"));
}

async fn send_cdp_command(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    session_id: Option<&str>,
    params: serde_json::Value,
) -> Vec<serde_json::Value> {
    send_cdp_command_without_wait(socket, id, method, session_id, params).await;
    recv_until_id(socket, id).await
}

async fn cdp_create_browser_context(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
) -> String {
    send_cdp_command(socket, id, "Target.createBrowserContext", None, json!({}))
        .await
        .iter()
        .find(|message| message["id"] == json!(id))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned()
}

struct TestCdpTargetSession {
    target_id: String,
    session_id: String,
}

async fn cdp_create_attached_target(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id_base: u64,
    browser_context_id: &str,
) -> TestCdpTargetSession {
    let create_target = send_cdp_command(
        socket,
        id_base,
        "Target.createTarget",
        None,
        json!({
            "browserContextId": browser_context_id,
            "url": "about:blank",
        }),
    )
    .await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(id_base))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    let attach = send_cdp_command(
        socket,
        id_base + 1,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(id_base + 1))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    TestCdpTargetSession {
        target_id,
        session_id,
    }
}

async fn cdp_navigate_and_wait_for_load(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    session_id: &str,
    url: &str,
) -> Vec<serde_json::Value> {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let mut messages = Vec::new();
    let mut saw_navigate_response = false;
    let mut saw_load_event = false;
    while !(saw_navigate_response && saw_load_event) {
        let message = recv_ws_json(socket).await;
        if message["id"] == json!(id) {
            saw_navigate_response = true;
        }
        if message["sessionId"].as_str() == Some(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_load_event = true;
        }
        messages.push(message);
    }
    messages
}

async fn cdp_create_session_and_navigate(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    url: &str,
) -> String {
    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Runtime.enable",
                "sessionId": session_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let mut saw_navigate_response = false;
    let mut saw_load_event = false;
    while !(saw_navigate_response && saw_load_event) {
        let message = recv_ws_json(socket).await;
        if message["id"] == json!(5_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_load_event = true;
        }
    }

    session_id
}

async fn cdp_create_default_session_and_navigate(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    url: &str,
) -> String {
    let create_target = send_cdp_command(
        socket,
        1,
        "Target.createTarget",
        None,
        json!({
            "browserContextId": "BID-default",
            "url": "about:blank",
        }),
    )
    .await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("default targetId")
        .to_owned();

    let attach = send_cdp_command(
        socket,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("default sessionId")
        .to_owned();

    let _ = send_cdp_command(socket, 3, "Runtime.enable", Some(&session_id), json!({})).await;
    cdp_navigate_and_wait_for_load(socket, 5, &session_id, url).await;

    session_id
}

async fn cdp_runtime_evaluate_string(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session_id: &str,
    id: u64,
    expression: &str,
) -> String {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": { "expression": expression }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let messages = recv_until_id(socket, id).await;
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("Runtime.evaluate string result")
        .to_owned()
}

async fn wait_for_cdp_runtime_string(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session_id: &str,
    mut id: u64,
    expression: &str,
    expected: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let value = cdp_runtime_evaluate_string(socket, session_id, id, expression).await;
        if value == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for Runtime.evaluate `{expression}` to become `{expected}`, last value `{value}`"
        );
        id += 1;
        sleep(Duration::from_millis(25)).await;
    }
}

async fn rejected_websocket_status(url: String) -> u16 {
    match connect_async(url)
        .await
        .expect_err("websocket should be rejected")
    {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
        error => panic!("expected websocket HTTP rejection, got {error:?}"),
    }
}

mod bidi;
mod cdp_dynamic_page;
mod classic;
mod download;
mod misc;
mod p6_output_handoff;
mod screenshot;
mod websocket;

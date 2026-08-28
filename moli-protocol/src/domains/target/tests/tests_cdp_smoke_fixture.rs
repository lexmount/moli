use super::*;
use axum::{
    body::Bytes,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderName, HeaderValue, Method, Uri,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH},
    },
    response::Response,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WORKER_SCRIPT: &str = r#"
self.onmessage = async event => {
  const data = event.data;
  try {
    if (data && data.kind === 'fetch') {
      const response = await fetch(data.url);
      self.postMessage({ ok: response.ok, status: response.status, text: await response.text() });
      return;
    }
    if (data && data.kind === 'xhr') {
      const result = await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        xhr.open('GET', data.url, true);
        xhr.onload = () => resolve({ ok: true, status: xhr.status, text: xhr.responseText });
        xhr.onerror = () => resolve({ ok: false, status: xhr.status, error: 'xhr error' });
        xhr.send();
      });
      self.postMessage(result);
      return;
    }
    self.postMessage({
      echoed: data,
      pathname: self.location.pathname,
      selfEqualsGlobal: self === globalThis,
    });
  } catch (error) {
    self.postMessage({
      ok: false,
      error: `${error && error.constructor && error.constructor.name || 'Error'}:${error && error.message || String(error)}`,
    });
  }
};
"#;

const DEDICATED_WORKER_SMOKE_SCRIPT: &str = r#"
globalThis.__dedicatedWorkerSmoke = {
  name,
  pathname: self.location.pathname,
  selfEqualsGlobal: self === globalThis,
  isDedicatedWorker:
    typeof DedicatedWorkerGlobalScope !== "undefined" &&
    self instanceof DedicatedWorkerGlobalScope,
};
console.log("dedicated worker smoke boot:" + name);
self.onmessage = event => self.postMessage({ echoed: event.data });
"#;

const SHARED_WORKER_SCRIPT: &str = r#"
console.log("shared worker smoke boot:" + name);
globalThis.__sharedWorkerSmoke = {
  name,
  pathname: self.location.pathname,
  selfEqualsGlobal: self === globalThis,
  isSharedWorker:
    typeof SharedWorkerGlobalScope !== "undefined" &&
    self instanceof SharedWorkerGlobalScope,
  connectCount: 0,
  lastMessages: [],
};
self.onconnect = event => {
  globalThis.__sharedWorkerSmoke.connectCount += 1;
  const port = event.ports[0];
  port.onmessage = message => {
    const data = message.data;
    globalThis.__sharedWorkerSmoke.lastMessages.push(data);
    if (data && data.kind === "probe") {
      port.postMessage({
        kind: "probe-result",
        echoed: data.value,
        name,
        pathname: self.location.pathname,
        selfEqualsGlobal: self === globalThis,
        isSharedWorker: globalThis.__sharedWorkerSmoke.isSharedWorker,
        connectCount: globalThis.__sharedWorkerSmoke.connectCount,
      });
      return;
    }
    port.postMessage({ kind: "echo", data });
  };
  port.start();
  port.postMessage({
    kind: "connected",
    name,
    pathname: self.location.pathname,
    isSharedWorker: globalThis.__sharedWorkerSmoke.isSharedWorker,
    connectCount: globalThis.__sharedWorkerSmoke.connectCount,
  });
};
"#;

const TRANSPARENT_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2, 0,
    239, 191, 167, 219, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const RESOURCE_MEDIA_BYTES: &[u8] = b"\x00\xffmoli-media";
const RESOURCE_XHR_BYTES: &[u8] = b"\x00\xffmoli-xhr";

#[derive(Default)]
struct SmokeFixtureState {
    profile_requests: Mutex<HashMap<String, Value>>,
    coop_blocked_redirect_target_requests: AtomicUsize,
}

pub(super) struct SmokeFixtureServer {
    pub(super) addr: SocketAddr,
    state: Arc<SmokeFixtureState>,
    server: tokio::task::JoinHandle<()>,
}

impl SmokeFixtureServer {
    pub(super) async fn start() -> Self {
        let state = Arc::new(SmokeFixtureState::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/ws-echo", get(ws_echo))
                    .fallback(any(fixture_handler))
                    .with_state(server_state),
            )
            .await
            .unwrap();
        });
        Self {
            addr,
            state,
            server,
        }
    }

    pub(super) fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub(super) fn coop_blocked_redirect_target_requests(&self) -> usize {
        self.state
            .coop_blocked_redirect_target_requests
            .load(Ordering::SeqCst)
    }
}

impl Drop for SmokeFixtureServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(super) struct RawFixtureResponse {
    pub(super) status: u16,
    pub(super) headers: HashMap<String, String>,
    pub(super) body: Vec<u8>,
}

impl RawFixtureResponse {
    pub(super) fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub(super) async fn fixture_get(fixture: &SmokeFixtureServer, path: &str) -> RawFixtureResponse {
    raw_http_request(fixture.addr, "GET", path, None, &[]).await
}

pub(super) async fn fixture_get_with_headers(
    fixture: &SmokeFixtureServer,
    path: &str,
    headers: &[(&str, &str)],
) -> RawFixtureResponse {
    raw_http_request(fixture.addr, "GET", path, None, headers).await
}

pub(super) async fn fixture_post(
    fixture: &SmokeFixtureServer,
    path: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) -> RawFixtureResponse {
    raw_http_request(fixture.addr, "POST", path, Some(body), headers).await
}

async fn raw_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    headers: &[(&str, &str)],
) -> RawFixtureResponse {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = body.unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    if !body.is_empty() {
        stream.write_all(body).await.unwrap();
    }

    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(8), stream.read_to_end(&mut bytes))
        .await
        .expect("fixture response timed out")
        .unwrap();
    parse_raw_response(bytes)
}

fn parse_raw_response(bytes: Vec<u8>) -> RawFixtureResponse {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response should contain header terminator");
    let head = String::from_utf8_lossy(&bytes[..split]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .expect("response status");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    RawFixtureResponse {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}

async fn ws_echo(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.protocols(["smoke"])
        .on_upgrade(|socket| async move { handle_ws_echo(socket).await })
}

async fn handle_ws_echo(mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Text(text) => {
                let _ = socket
                    .send(Message::Text(format!("echo:{text}").into()))
                    .await;
            }
            Message::Ping(payload) => {
                let _ = socket.send(Message::Pong(payload)).await;
            }
            Message::Close(_) => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            _ => {}
        }
    }
}

async fn fixture_handler(
    State(state): State<Arc<SmokeFixtureState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path();
    let query = query_map(uri.query().unwrap_or_default());
    match path {
        "/favicon.ico" => response(
            StatusCode::NO_CONTENT,
            "text/plain; charset=utf-8",
            Vec::new(),
            &[],
        ),
        "/plain" => html("<!doctype html><main>plain ok</main>"),
        "/iframe" => html(r#"<!doctype html><main>parent</main><iframe src="/child"></iframe>"#),
        "/child" => html("<!doctype html><body>child body text</body>"),
        "/wait-for-function" => html(
            "<!doctype html><body><script>setTimeout(() => { globalThis.__ready = true; }, 50);</script></body>",
        ),
        "/lifecycle-load-state" => html(
            "<!doctype html><body data-dcl='0' data-load='0'>\
             <main>lifecycle load state</main>\
             <img id='delayed' src='/delayed-image.png?delay=0.2' alt='delayed'>\
             <script>\
             document.addEventListener('DOMContentLoaded', () => { document.body.dataset.dcl = '1'; });\
             window.addEventListener('load', () => { document.body.dataset.load = '1'; });\
             </script></body>",
        ),
        "/chromium-cdp-lifecycle-page" => html(
            "<!doctype html><body><main>chromium lifecycle page</main>\
             <script>document.body.dataset.scriptRan = '1';</script></body>",
        ),
        "/chromium-cdp-dom-page" => html(
            r#"<!doctype html><body><p class="class1" attr1="attr1">Paragraph Text</p></body>"#,
        ),
        "/chromium-cdp-dom-query-page" => html(
            "<!doctype html><body>\
             <div class=\"testClass\" id=\"firstDiv\"></div>\
             <div class=\"testClass\" id=\"secondDiv\"></div>\
             <div class=\"testClass\"></div><div class=\"testClass\"></div><div class=\"testClass\"></div>\
             <div id=\"depth-1\"><div id=\"depth-2\"><div id=\"targetDiv\"></div></div>\
             <div id=\"targetUncle\"><div id=\"targetCousin\"></div></div></div></body>",
        ),
        "/chromium-cdp-hit-test-page" => html(
            "<!doctype html><body>\
             <div id='hit-target' style='position:absolute;top:0;left:0;width:100px;height:100px'></div>\
             <div style='position:absolute;top:0;left:0;width:200px;height:200px;pointer-events:none'></div>\
             </body>",
        ),
        "/chromium-cdp-layout-page" => html(
            "<!doctype html><body><div style='height:10000px;width:10000px'>content</div></body>",
        ),
        "/shared-worker-smoke" => html(
            "<!doctype html><body><main>shared worker smoke</main><script>\
             globalThis.__sharedWorkerSmokeMessages = [];\
             globalThis.__sharedWorkerSmokeReady = new Promise(resolve => {\
             const worker = new SharedWorker('/shared-worker-smoke.js', 'shared-worker-smoke');\
             globalThis.__sharedWorkerSmokeWorker = worker;\
             worker.port.onmessage = event => {\
             globalThis.__sharedWorkerSmokeMessages.push(event.data);\
             if (event.data && event.data.kind === 'probe-result') resolve(event.data);\
             };\
             worker.port.start();\
             worker.port.postMessage({ kind: 'probe', value: 'page-probe' });\
             });\
             </script></body>",
        ),
        "/playwright-route-times" => html("<!doctype html><main>server fallback</main>"),
        "/playwright-fallback-chain" => html("<!doctype html><main>fallback chain</main>"),
        "/delayed-image.png" => {
            if let Some(delay) = query
                .get("delay")
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| value.clamp(0.0, 2.0))
                .filter(|value| *value > 0.0)
            {
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }
            png(TRANSPARENT_PNG)
        }
        "/dialog" => html(
            r#"<!doctype html><button id='alert' onclick="alert('fixture alert')">alert</button>"#,
        ),
        "/set-cookie" => html_with_headers(
            "<!doctype html><main>set cookie</main>",
            &[("set-cookie", "serverCookie=server; Path=/")],
        ),
        "/echo-cookie" => html(&format!(
            "<!doctype html><body>{}</body>",
            escape_html(
                headers
                    .get("cookie")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
            )
        )),
        "/profile-headers" => {
            let token = query.get("token").cloned().unwrap_or_default();
            state.profile_requests.lock().insert(
                token,
                json!({
                    "userAgent": header_string(&headers, "user-agent"),
                    "acceptLanguage": header_string(&headers, "accept-language"),
                    "extraHeader": header_string(&headers, "x-moli-profile-smoke"),
                    "devtoolsTest": header_string(&headers, "x-devtools-test"),
                    "referer": header_string(&headers, "referer"),
                }),
            );
            html("<!doctype html><main>profile headers</main>")
        }
        "/profile-result" => {
            let token = query.get("token").cloned().unwrap_or_default();
            json_response(
                state
                    .profile_requests
                    .lock()
                    .get(&token)
                    .cloned()
                    .unwrap_or(Value::Null),
                &[],
            )
        }
        "/redirect-start" => redirect("/redirect-final", &[("cache-control", "no-store")]),
        "/redirect-final" => html("<!doctype html><main>redirect final</main>"),
        "/coop-redirect-start" => redirect(
            "/coop-redirect-final",
            &[("cross-origin-opener-policy", "same-origin")],
        ),
        "/coop-redirect-final" => {
            html("<!doctype html><main id='coop-marker'>COOP redirect committed popup</main>")
        }
        "/coop-redirect-to" => redirect(
            query.get("url").map(String::as_str).unwrap_or("/plain"),
            &[("cross-origin-opener-policy", "same-origin")],
        ),
        "/coop-same-origin" => html_with_headers(
            "<!doctype html><main id='coop-marker'>COOP committed popup</main>",
            &[("cross-origin-opener-policy", "same-origin")],
        ),
        "/coop-csp-sandbox" => html_with_headers(
            "<!doctype html><main id='must-not-commit'>blocked COOP sandbox body</main><script>globalThis.__blockedCoopBodyRan = true</script>",
            &[
                ("cross-origin-opener-policy", "same-origin"),
                (
                    "content-security-policy",
                    "sandbox allow-popups allow-scripts allow-same-origin",
                ),
            ],
        ),
        "/coop-sandbox-blocked-redirect" => redirect(
            "/coop-sandbox-blocked-redirect-target",
            &[
                ("cross-origin-opener-policy", "same-origin"),
                (
                    "content-security-policy",
                    "sandbox allow-popups allow-scripts allow-same-origin",
                ),
            ],
        ),
        "/coop-sandbox-blocked-redirect-target" => {
            state
                .coop_blocked_redirect_target_requests
                .fetch_add(1, Ordering::SeqCst);
            html("<!doctype html><main>blocked redirect target must not load</main>")
        }
        "/csp-sandbox-navigate-to-coop" => html_with_headers(
            "<!doctype html><script>location.replace('/coop-same-origin')</script>",
            &[(
                "content-security-policy",
                "sandbox allow-popups allow-scripts allow-same-origin",
            )],
        ),
        "/no-content" => response(
            StatusCode::NO_CONTENT,
            "text/plain; charset=utf-8",
            Vec::new(),
            &[("x-smoke-navigation-terminal", "no-content")],
        ),
        "/reset-content" => response(
            StatusCode::RESET_CONTENT,
            "text/plain; charset=utf-8",
            Vec::new(),
            &[("x-smoke-navigation-terminal", "reset-content")],
        ),
        "/history-a" => html("<!doctype html><main>history a</main>"),
        "/history-b" => html("<!doctype html><main>history b</main>"),
        "/document-continue" => {
            let marker = header_string(&headers, "x-smoke-nav-route")
                .unwrap_or_else(|| "missing-document-route-header".to_owned());
            html(&format!("<!doctype html><main>{marker}</main>"))
        }
        "/document-response-stage" => html_with_headers(
            "<!doctype html><main>document response-stage body</main>",
            &[("x-smoke-document-stage", "paused")],
        ),
        "/api" => text("fixture api body", StatusCode::OK, &[]),
        "/api-continue" | "/worker-route-continue" => json_response(
            json!({
                "method": method.as_str(),
                "routeHeader": header_string(&headers, "x-smoke-route")
                    .or_else(|| header_string(&headers, "x-smoke-worker-route")),
            }),
            &[],
        ),
        "/api-abort" => text("api abort fallback", StatusCode::OK, &[]),
        "/api-echo" => json_response(
            json!({
                "method": method.as_str(),
                "body": String::from_utf8_lossy(&body),
                "contentType": header_string(&headers, "content-type"),
                "customHeader": header_string(&headers, "x-smoke-post"),
            }),
            &[],
        ),
        "/api-response-headers" => json_response(
            json!({"ok": true, "kind": header_string(&headers, "x-smoke-response-kind")}),
            &[
                ("x-smoke-response", "header-visible"),
                (
                    "x-smoke-request-kind",
                    header_string(&headers, "x-smoke-response-kind")
                        .as_deref()
                        .unwrap_or("missing"),
                ),
            ],
        ),
        "/api-response-stage" => {
            if let Some(delay) = query
                .get("delay")
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| value.clamp(0.0, 2.0))
                .filter(|value| *value > 0.0)
            {
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }
            text(
                "response-stage body",
                StatusCode::OK,
                &[("x-smoke-response-stage", "paused")],
            )
        }
        "/api-binary" => response(
            StatusCode::OK,
            "application/octet-stream",
            b"\x00\xffa".to_vec(),
            &[("x-smoke-binary", "ok")],
        ),
        "/api-auth" => {
            if header_string(&headers, "authorization").as_deref() == Some("Basic dXNlcjpwYXNz") {
                text(
                    "authenticated fetch",
                    StatusCode::OK,
                    &[("x-smoke-auth-stage", "ok")],
                )
            } else {
                let realm = query
                    .get("realm")
                    .map(String::as_str)
                    .unwrap_or("smoke-auth");
                let escaped = realm.replace('\\', "\\\\").replace('"', "\\\"");
                text(
                    "auth required",
                    StatusCode::UNAUTHORIZED,
                    &[("www-authenticate", &format!("Basic realm=\"{escaped}\""))],
                )
            }
        }
        "/api-redirect-start" => redirect(
            "/api-redirect-final",
            &[("x-smoke-redirect", "start"), ("cache-control", "no-store")],
        ),
        "/api-redirect-final" => {
            json_response(json!({"redirected": true, "method": method.as_str()}), &[])
        }
        "/parser-script-page" => html(
            r#"<!doctype html><script src="/parser-script.js"></script><main>parser script page</main>"#,
        ),
        "/parser-script.js" => {
            js(r#"globalThis.__smokeParserScriptValue = "parser script loaded";"#)
        }
        "/stylesheet-resource-page" => html(
            "<!doctype html><head><link rel=\"stylesheet\" href=\"/resource-link.css\">\
             <style>@import url('/resource-import.css'); main { border-top-width: 1px; }</style>\
             <script src=\"/resource-after-style.js\"></script></head>\
             <body><main id='styled'>stylesheet resource page</main></body>",
        ),
        "/stylesheet-resource-no-script-page" => html(
            "<!doctype html><head><link rel=\"stylesheet\" href=\"/resource-link.css\">\
             <style>@import url('/resource-import.css'); main { border-top-width: 1px; }</style></head>\
             <body><main id='styled'>stylesheet resource page</main></body>",
        ),
        "/resource-link.css" => css("main { color: rgb(12, 34, 56); }"),
        "/resource-import.css" => css("main { background-color: rgb(210, 220, 230); }"),
        "/resource-after-style.js" => js("globalThis.__smokeAfterStylesheet = true;"),
        "/chromium-resource-type-page" => html(
            "<!doctype html><head><link rel=\"stylesheet\" href=\"/chromium-resource-style.css\">\
             <script src=\"/chromium-resource-script.js\"></script></head><body>\
             <img id=\"resource-image\" src=\"/chromium-resource-image.png\" alt=\"resource\">\
             <audio id=\"resource-audio\" src=\"/chromium-resource-audio.wav\"></audio>\
             <video id='resource-video'><source src=\"/chromium-resource-video.ogv\" type=\"video/ogg\">\
             <track default kind=\"captions\" src=\"/chromium-resource-captions.vtt\"></video>\
             <script>globalThis.__smokeResourceXhrDone = new Promise(resolve => {\
             const xhr = new XMLHttpRequest(); xhr.open('GET', '/chromium-resource-xhr.bin', true);\
             xhr.responseType = 'arraybuffer'; xhr.onload = () => resolve({ status: xhr.status, length: xhr.response.byteLength });\
             xhr.onerror = () => resolve({ status: xhr.status, error: 'xhr error' }); xhr.send();});</script>\
             <main>chromium resource type page</main></body>",
        ),
        "/chromium-resource-style.css" => css("main { color: rgb(31, 41, 59); }"),
        "/chromium-resource-script.js" => js("globalThis.__smokeChromiumResourceScript = true;"),
        "/chromium-resource-image.png" => png(TRANSPARENT_PNG),
        "/chromium-resource-audio.wav" => response(
            StatusCode::OK,
            "audio/wav",
            RESOURCE_MEDIA_BYTES.to_vec(),
            &[],
        ),
        "/chromium-resource-video.ogv" => response(
            StatusCode::OK,
            "video/ogg",
            RESOURCE_MEDIA_BYTES.to_vec(),
            &[],
        ),
        "/chromium-resource-captions.vtt" => response(
            StatusCode::OK,
            "text/vtt; charset=utf-8",
            b"WEBVTT\n\n00:00.000 --> 00:01.000\ncaption\n".to_vec(),
            &[],
        ),
        "/chromium-resource-xhr.bin" => response(
            StatusCode::OK,
            "application/octet-stream",
            RESOURCE_XHR_BYTES.to_vec(),
            &[],
        ),
        "/worker.js" => js(WORKER_SCRIPT),
        "/shared-worker-smoke.js" => js(SHARED_WORKER_SCRIPT),
        "/dedicated-worker-smoke.js" => js(DEDICATED_WORKER_SMOKE_SCRIPT),
        "/worker-route-fulfill" => text("worker route fulfill fallback", StatusCode::OK, &[]),
        "/worker-route-abort" => text("worker route abort fallback", StatusCode::OK, &[]),
        "/download-page" => html(
            r#"<!doctype html><a id="download" href="/download" download>download</a><a id="slow-download" href="/slow-download" download>slow</a>"#,
        ),
        "/download" => response(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            b"download contents".to_vec(),
            &[(
                "content-disposition",
                "attachment; filename=\"smoke-download.txt\"",
            )],
        ),
        "/slow-download" => {
            tokio::time::sleep(Duration::from_millis(50)).await;
            response(
                StatusCode::OK,
                "text/plain; charset=utf-8",
                b"slow download contents".to_vec(),
                &[(
                    "content-disposition",
                    "attachment; filename=\"slow-smoke-download.txt\"",
                )],
            )
        }
        _ => text(&format!("not found: {path}"), StatusCode::NOT_FOUND, &[]),
    }
}

fn query_map(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn html(body: &str) -> Response {
    html_with_headers(body, &[])
}

fn html_with_headers(body: &str, headers: &[(&str, &str)]) -> Response {
    response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        body.as_bytes().to_vec(),
        headers,
    )
}

fn js(body: &str) -> Response {
    response(
        StatusCode::OK,
        "application/javascript; charset=utf-8",
        body.as_bytes().to_vec(),
        &[],
    )
}

fn css(body: &str) -> Response {
    response(
        StatusCode::OK,
        "text/css; charset=utf-8",
        body.as_bytes().to_vec(),
        &[],
    )
}

fn png(body: &[u8]) -> Response {
    response(StatusCode::OK, "image/png", body.to_vec(), &[])
}

fn text(body: &str, status: StatusCode, headers: &[(&str, &str)]) -> Response {
    response(
        status,
        "text/plain; charset=utf-8",
        body.as_bytes().to_vec(),
        headers,
    )
}

fn json_response(value: Value, headers: &[(&str, &str)]) -> Response {
    response(
        StatusCode::OK,
        "application/json; charset=utf-8",
        serde_json::to_vec(&value).unwrap(),
        headers,
    )
}

fn redirect(location: &str, headers: &[(&str, &str)]) -> Response {
    response(
        StatusCode::FOUND,
        "text/plain; charset=utf-8",
        Vec::new(),
        headers,
    )
    .with_header("location", location)
}

trait ResponseHeaderExt {
    fn with_header(self, name: &str, value: &str) -> Self;
}

impl ResponseHeaderExt for Response {
    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
        self
    }
}

fn response(
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
    headers: &[(&str, &str)],
) -> Response {
    let body_len = body.len();
    let mut response = (status, body).into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&body_len.to_string()).unwrap(),
    );
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).unwrap();
        let header_value = HeaderValue::from_str(value).unwrap();
        if header_name == CONTENT_DISPOSITION {
            response_headers.insert(CONTENT_DISPOSITION, header_value);
        } else {
            response_headers.insert(header_name, header_value);
        }
    }
    response
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_smoke_fixture_serves_all_document_and_control_routes() {
    let fixture = SmokeFixtureServer::start().await;
    for path in [
        "/plain",
        "/iframe",
        "/child",
        "/wait-for-function",
        "/lifecycle-load-state",
        "/chromium-cdp-lifecycle-page",
        "/chromium-cdp-dom-page",
        "/chromium-cdp-dom-query-page",
        "/chromium-cdp-hit-test-page",
        "/chromium-cdp-layout-page",
        "/playwright-route-times",
        "/playwright-fallback-chain",
        "/dialog",
        "/set-cookie",
        "/redirect-final",
        "/coop-redirect-final",
        "/coop-same-origin",
        "/coop-csp-sandbox",
        "/coop-sandbox-blocked-redirect-target",
        "/csp-sandbox-navigate-to-coop",
        "/history-a",
        "/history-b",
        "/document-continue",
        "/parser-script-page",
        "/stylesheet-resource-page",
        "/stylesheet-resource-no-script-page",
        "/chromium-resource-type-page",
        "/download-page",
    ] {
        let response = fixture_get(&fixture, path).await;
        assert_eq!(response.status, 200, "route {path}");
        assert!(
            response
                .header("content-type")
                .is_some_and(|value| value.starts_with("text/html")),
            "route {path} should be html: {:?}",
            response.headers
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_smoke_fixture_serves_navigation_no_commit_routes() {
    let fixture = SmokeFixtureServer::start().await;
    for (path, expected_status) in [("/no-content", 204), ("/reset-content", 205)] {
        let response = fixture_get(&fixture, path).await;
        assert_eq!(response.status, expected_status, "route {path}");
        assert!(
            response.body.is_empty(),
            "route {path} must not carry a body"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_smoke_fixture_serves_all_subresource_routes() {
    let fixture = SmokeFixtureServer::start().await;
    for (path, content_type) in [
        ("/delayed-image.png?delay=0.01", "image/png"),
        ("/parser-script.js", "application/javascript"),
        ("/resource-link.css", "text/css"),
        ("/resource-import.css", "text/css"),
        ("/resource-after-style.js", "application/javascript"),
        ("/chromium-resource-style.css", "text/css"),
        ("/chromium-resource-script.js", "application/javascript"),
        ("/chromium-resource-image.png", "image/png"),
        ("/chromium-resource-audio.wav", "audio/wav"),
        ("/chromium-resource-video.ogv", "video/ogg"),
        ("/chromium-resource-captions.vtt", "text/vtt"),
        ("/chromium-resource-xhr.bin", "application/octet-stream"),
        ("/worker.js", "application/javascript"),
    ] {
        let response = fixture_get(&fixture, path).await;
        assert_eq!(response.status, 200, "route {path}");
        assert!(
            response
                .header("content-type")
                .is_some_and(|value| value.starts_with(content_type)),
            "route {path} should be {content_type}: {:?}",
            response.headers
        );
        assert!(!response.body.is_empty(), "route {path} body");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_smoke_fixture_serves_all_api_download_and_profile_routes() {
    let fixture = SmokeFixtureServer::start().await;

    assert_eq!(
        fixture_get(&fixture, "/api").await.body_text(),
        "fixture api body"
    );
    let api_continue =
        fixture_get_with_headers(&fixture, "/api-continue", &[("x-smoke-route", "continued")])
            .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&api_continue.body).unwrap()["routeHeader"],
        "continued"
    );

    let echo = fixture_post(
        &fixture,
        "/api-echo",
        b"posted",
        &[("content-type", "text/plain"), ("x-smoke-post", "custom")],
    )
    .await;
    let echo_json = serde_json::from_slice::<Value>(&echo.body).unwrap();
    assert_eq!(echo_json["method"], "POST");
    assert_eq!(echo_json["body"], "posted");
    assert_eq!(echo_json["customHeader"], "custom");

    let headers = fixture_get_with_headers(
        &fixture,
        "/api-response-headers",
        &[("x-smoke-response-kind", "sample")],
    )
    .await;
    assert_eq!(headers.header("x-smoke-response"), Some("header-visible"));
    assert_eq!(headers.header("x-smoke-request-kind"), Some("sample"));

    assert_eq!(fixture_get(&fixture, "/api-auth").await.status, 401);
    assert_eq!(
        fixture_get_with_headers(
            &fixture,
            "/api-auth",
            &[("authorization", "Basic dXNlcjpwYXNz")]
        )
        .await
        .body_text(),
        "authenticated fetch"
    );

    let profile = fixture_get_with_headers(
        &fixture,
        "/profile-headers?token=abc",
        &[
            ("user-agent", "fixture-agent"),
            ("accept-language", "en-US"),
            ("x-moli-profile-smoke", "extra"),
            ("referer", "https://example.test/"),
        ],
    )
    .await;
    assert_eq!(profile.status, 200);
    let profile_result = fixture_get(&fixture, "/profile-result?token=abc").await;
    let profile_json = serde_json::from_slice::<Value>(&profile_result.body).unwrap();
    assert_eq!(profile_json["userAgent"], "fixture-agent");
    assert_eq!(profile_json["extraHeader"], "extra");

    let binary = fixture_get(&fixture, "/api-binary").await;
    assert_eq!(binary.body, b"\x00\xffa");
    assert_eq!(binary.header("x-smoke-binary"), Some("ok"));

    let download = fixture_get(&fixture, "/download").await;
    assert_eq!(download.body_text(), "download contents");
    assert_eq!(
        download.header("content-disposition"),
        Some("attachment; filename=\"smoke-download.txt\"")
    );
    assert_eq!(
        fixture_get(&fixture, "/slow-download").await.body_text(),
        "slow download contents"
    );
}

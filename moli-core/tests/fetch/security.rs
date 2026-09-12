use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
use url::Url;

#[derive(Debug)]
struct IncomingRequest {
    method: String,
    path: String,
    origin: Option<String>,
    cookie: Option<String>,
}

struct SecurityServers {
    origin: String,
    cross: String,
    requests: Arc<Mutex<Vec<IncomingRequest>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for SecurityServers {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl SecurityServers {
    async fn spawn() -> Result<Self> {
        let main = TcpListener::bind("127.0.0.1:0").await?;
        let other = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", main.local_addr()?);
        let cross = format!("http://{}", other.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let tasks = [main, other].into_iter().map(|listener| {
            let requests = Arc::clone(&requests);
            tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let Ok((mut stream, _)) = accepted else { break };
                            let requests = Arc::clone(&requests);
                            connections.spawn(async move {
                                let mut bytes = Vec::new();
                                while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
                                    let Ok(byte) = stream.read_u8().await else { return };
                                    bytes.push(byte);
                                }
                                let text = String::from_utf8_lossy(&bytes);
                                let mut parts = text.split_whitespace();
                                let method = parts.next().unwrap_or("").to_owned();
                                let path = parts.next().unwrap_or("/").to_owned();
                                let origin = text.lines().filter_map(|line| line.split_once(':'))
                                    .find(|(key, _)| key.eq_ignore_ascii_case("Origin"))
                                    .map(|(_, value)| value.trim().to_owned());
                                let cookie = text.lines().filter_map(|line| line.split_once(':'))
                                    .find(|(key, _)| key.eq_ignore_ascii_case("Cookie"))
                                    .map(|(_, value)| value.trim().to_owned());
                                let request = IncomingRequest { method, path, origin, cookie };
                                let (status, headers, body) = fixture_response(&request);
                                requests.lock().push(request);
                                let response = format!("HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                                let _ = stream.write_all(response.as_bytes()).await;
                            });
                        }
                        _ = connections.join_next(), if !connections.is_empty() => {}
                    }
                }
            })
        }).collect();
        Ok(Self {
            origin,
            cross,
            requests,
            tasks,
        })
    }
}

fn fixture_response(request: &IncomingRequest) -> (u16, String, String) {
    let url = Url::parse(&format!("http://fixture.test{}", request.path)).unwrap();
    let cors = format!(
        "Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-probe\r\n",
        request.origin.as_deref().unwrap_or("*")
    );
    let cache = if url.path().starts_with("/cached-") {
        "Cache-Control: max-age=600\r\n"
    } else {
        "Cache-Control: no-store\r\n"
    };
    if url.path() == "/page" {
        return (
            200,
            "Content-Type: text/html\r\nSet-Cookie: originSession=present; Path=/; SameSite=Lax\r\n".to_owned(),
            "<!doctype html><body>fetch security</body>".to_owned(),
        );
    }
    if url.path() == "/blob-creator" {
        return (
            200,
            "Content-Type: text/html\r\n".to_owned(),
            r#"<!doctype html><script>
            const blob = URL.createObjectURL(new Blob(['foreign']));
            fetch(blob, {mode: 'same-origin'}).then(response => response.text()).then(text =>
                parent.postMessage({blob, text}, '*'));
        </script>"#
                .to_owned(),
        );
    }
    if url.path() == "/sw.js" {
        return (
            200,
            "Content-Type: text/javascript\r\nCache-Control: no-store\r\n".to_owned(),
            r#"
            const seen = [];
            self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
            self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
            self.addEventListener('fetch', event => {
                const url = new URL(event.request.url);
                if (url.pathname === '/sw-log') {
                    event.respondWith(new Response(JSON.stringify(seen)));
                } else if (url.pathname.startsWith('/sw-')) {
                    seen.push(url.pathname);
                    event.respondWith(url.pathname === '/sw-redirect'
                        ? Response.redirect(url.searchParams.get('to'))
                        : new Response('worker'));
                }
            });
        "#
            .to_owned(),
        );
    }
    if request.method == "OPTIONS" {
        if let Some(status) = url.path().strip_prefix("/preflight-redirect/") {
            return (
                status.parse().unwrap(),
                format!("{cors}Location: /preflight-target/{status}\r\n"),
                String::new(),
            );
        }
        return (204, cors, String::new());
    }
    if matches!(
        url.path(),
        "/redirect-allowed" | "/redirect-denied" | "/cached-redirect"
    ) {
        let target = url.query_pairs().find(|(name, _)| name == "to").unwrap().1;
        let cors = if url.path() == "/redirect-denied" {
            String::new()
        } else {
            cors
        };
        return (
            302,
            format!("{cors}{cache}Location: {target}\r\n"),
            String::new(),
        );
    }
    if url.path() == "/track.vtt" {
        return (
            200,
            format!("{cors}{cache}Content-Type: text/vtt\r\n"),
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhello\n".to_owned(),
        );
    }
    (
        200,
        format!("{cors}{cache}Content-Type: text/plain\r\n"),
        "ok".to_owned(),
    )
}

fn redirect(origin: &str, path: &str, target: &str) -> String {
    let mut url = Url::parse(&format!("{origin}/{path}")).unwrap();
    url.query_pairs_mut().append_pair("to", target);
    url.into()
}

fn fetch_cases(cases: &Value) -> String {
    format!(
        r#"(async () => {{
        const results = {{}};
        for (const test of {cases}) {{
            try {{
                const response = await fetch(test.url, test);
                results[test.name] = response.type === 'opaqueredirect'
                    ? response.type : await response.text();
            }} catch (error) {{ results[test.name] = error.name; }}
        }}
        return JSON.stringify(results);
    }})()"#
    )
}

fn results(value: Value) -> Result<Value> {
    Ok(serde_json::from_str(
        value["value"].as_str().expect("JSON result"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_fetch_rejects_before_network_or_preflight() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let cross_target = format!("{}/forbidden-target", server.cross);
    let cross_redirect = redirect(&server.origin, "redirect-allowed", &cross_target);
    let cases = json!([
        {"name":"same", "url":"/resource", "mode":"same-origin"},
        {"name":"same-put", "url":"/resource-put", "mode":"same-origin", "method":"PUT", "headers":{"X-Probe":"yes"}},
        {"name":"cross", "url":format!("{}/forbidden-get", server.cross), "mode":"same-origin"},
        {"name":"cross-put", "url":format!("{}/forbidden-put", server.cross), "mode":"same-origin", "method":"PUT", "headers":{"X-Probe":"yes"}},
        {"name":"cors", "url":format!("{}/cors-control", server.cross), "mode":"cors"},
        {"name":"data", "url":"data:text/plain,ok", "mode":"same-origin"},
        {"name":"same-redirect", "url":redirect(&server.origin, "redirect-allowed", &format!("{}/local-final", server.origin)), "mode":"same-origin"},
        {"name":"cross-redirect", "url":cross_redirect, "mode":"same-origin"},
        {"name":"manual", "url":cross_redirect, "mode":"same-origin", "redirect":"manual"},
        {"name":"error", "url":cross_redirect, "mode":"same-origin", "redirect":"error"}
    ]);
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&fetch_cases(&cases), true)
            .await?,
    )?;
    assert_eq!(
        observed,
        json!({"same":"ok", "same-put":"ok", "cross":"TypeError", "cross-put":"TypeError", "cors":"ok", "data":"ok", "same-redirect":"ok", "cross-redirect":"TypeError", "manual":"opaqueredirect", "error":"TypeError"})
    );
    let requests = server.requests.lock();
    assert!(
        !requests
            .iter()
            .any(|request| request.path.starts_with("/forbidden-") || request.method == "OPTIONS"),
        "{requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path == "/resource-put" && request.method == "PUT")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cors_and_preflight_rejection_never_issue_redirect_target_requests() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let forbidden = format!("{}/forbidden-cors-target", server.origin);
    let mut cases = json!([
        {"name":"denied", "url":redirect(&server.cross, "redirect-denied", &forbidden), "mode":"cors"},
        {"name":"denied-include", "url":redirect(&server.cross, "redirect-denied", &forbidden), "mode":"cors", "credentials":"include"},
        {"name":"allowed", "url":redirect(&server.cross, "redirect-allowed", &format!("{}/allowed-final", server.origin)), "mode":"cors", "credentials":"include"},
        {"name":"preflight-ok", "url":format!("{}/preflight-ok", server.cross), "method":"PUT", "headers":{"X-Probe":"yes"}}
    ]);
    for status in [301, 302, 303, 307, 308] {
        cases.as_array_mut().unwrap().push(json!({"name":format!("preflight-{status}"), "url":format!("{}/preflight-redirect/{status}", server.cross), "method":"PUT", "headers":{"X-Probe":"yes"}}));
    }
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&fetch_cases(&cases), true)
            .await?,
    )?;
    for test in cases.as_array().unwrap() {
        let name = test["name"].as_str().unwrap();
        assert_eq!(
            observed[name],
            if matches!(name, "allowed" | "preflight-ok") {
                "ok"
            } else {
                "TypeError"
            },
            "{name}: {observed}"
        );
    }
    let requests = server.requests.lock();
    assert!(
        !requests
            .iter()
            .any(|request| request.path.starts_with("/forbidden-")
                || request.path.starts_with("/preflight-target/")),
        "{requests:?}"
    );
    let preflights: Vec<_> = requests
        .iter()
        .filter(|request| request.path.starts_with("/preflight-redirect/"))
        .collect();
    assert_eq!(preflights.len(), 5, "{requests:?}");
    assert!(preflights.iter().all(|request| request.method == "OPTIONS"));
    assert_eq!(
        requests
            .iter()
            .find(|request| request.path == "/allowed-final")
            .unwrap()
            .origin
            .as_deref(),
        Some("null")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path == "/preflight-ok" && request.method == "PUT")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_fetch_cannot_reuse_a_cross_origin_memory_cache_entry() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let url = format!("{}/cached-resource", server.cross);
    let cases = json!([
        {"name":"warm", "url":url, "mode":"cors"},
        {"name":"cached", "url":url, "mode":"cors"},
        {"name":"same-origin", "url":url, "mode":"same-origin"}
    ]);
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&fetch_cases(&cases), true)
            .await?,
    )?;
    assert_eq!(
        observed,
        json!({"warm":"ok", "cached":"ok", "same-origin":"TypeError"})
    );
    let requests = server.requests.lock();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path.starts_with("/cached-"))
            .count(),
        1,
        "{requests:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_mode_rejects_before_service_worker_dispatch_and_redispatch() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    page.evaluate_runtime_expression_with_await_async(r#"(async () => {
        await navigator.serviceWorker.register('/sw.js');
        await navigator.serviceWorker.ready;
        if (!navigator.serviceWorker.controller) {
            await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
        }
        return true;
    })()"#, true).await?;
    let cases = json!([
        {"name":"safe", "url":"/sw-safe", "mode":"same-origin"},
        {"name":"cross", "url":format!("{}/sw-cross", server.cross), "mode":"same-origin"},
        {"name":"redirect", "url":redirect(&server.origin, "sw-redirect", &format!("{}/sw-forbidden-target", server.cross)), "mode":"same-origin"},
        {"name":"cors", "url":redirect(&server.cross, "sw-redirect", &format!("{}/sw-cors-target", server.origin)), "mode":"cors"},
        {"name":"fallback", "url":redirect(&server.origin, "sw-redirect", &format!("{}/resource", server.origin)), "mode":"same-origin"},
        {"name":"preflight", "url":redirect(&server.origin, "sw-redirect", &format!("{}/preflight-redirect/307", server.cross)), "method":"PUT", "headers":{"X-Probe":"yes"}}
    ]);
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&fetch_cases(&cases), true)
            .await?,
    )?;
    assert_eq!(
        observed,
        json!({"safe":"worker", "cross":"TypeError", "redirect":"TypeError", "cors":"worker", "fallback":"ok", "preflight":"TypeError"})
    );
    let seen = results(
        page.evaluate_runtime_expression_with_await_async(
            "fetch('/sw-log').then(response => response.text())",
            true,
        )
        .await?,
    )?;
    assert_eq!(
        seen,
        json!([
            "/sw-safe",
            "/sw-redirect",
            "/sw-redirect",
            "/sw-cors-target",
            "/sw-redirect",
            "/sw-redirect"
        ])
    );
    let requests = server.requests.lock();
    assert!(
        !requests
            .iter()
            .any(|request| request.path.starts_with("/sw-cross")
                || request.path.starts_with("/sw-forbidden-target")
                || request.path.starts_with("/preflight-target/")),
        "{requests:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn text_track_without_crossorigin_blocks_cross_origin_before_network() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let mut config = BrowserConfig::default();
    config.set_optional_resource_fetch_enabled(
        moli_page_types::SubresourceResourceType::TextTrack,
        true,
    );
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let cases = json!([
        {"name":"same", "url":"/track.vtt?same"},
        {"name":"cross", "url":format!("{}/track.vtt?cross", server.cross)},
        {"name":"cors", "url":format!("{}/track.vtt?cors", server.cross), "crossOrigin":"anonymous"},
        {"name":"redirect", "url":redirect(&server.origin, "redirect-allowed", &format!("{}/track.vtt?forbidden-redirect", server.cross))}
    ]);
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(
            &format!(
                r#"(async () => {{
        const results = {{}};
        for (const test of {cases}) {{
            results[test.name] = await new Promise(resolve => {{
                const media = document.createElement('video');
                if ('crossOrigin' in test) media.crossOrigin = test.crossOrigin;
                const track = document.createElement('track');
                track.onload = () => resolve('load');
                track.onerror = () => resolve('error');
                track.src = test.url;
                media.append(track);
                document.body.append(media);
                track.track.mode = 'hidden';
            }});
        }}
        return JSON.stringify(results);
    }})()"#
            ),
            true,
        )
        .await?,
    )?;
    assert_eq!(
        observed,
        json!({"same":"load", "cross":"error", "cors":"load", "redirect":"error"})
    );
    let requests = server.requests.lock();
    assert!(
        !requests.iter().any(|request| matches!(
            request.path.as_str(),
            "/track.vtt?cross" | "/track.vtt?forbidden-redirect"
        )),
        "{requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path == "/track.vtt?cors")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn window_and_worker_same_origin_mode_checks_blob_before_local_resolution() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let expression = format!(
        "({})({})",
        include_str!("fixtures/blob-origin.js"),
        json!(server.cross)
    );
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&expression, true)
            .await?,
    )?;
    assert_eq!(
        observed,
        json!({
            "creator": "foreign",
            "window-foreign": "TypeError", "window-local": "local", "window-data": "data",
            "worker-foreign": "TypeError", "worker-local": "local", "worker-data": "data"
        })
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn srcdoc_fetch_uses_inherited_or_opaque_origin_independently_of_base_url() -> Result<()> {
    let server = SecurityServers::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let expression = format!(
        "({})({})",
        include_str!("fixtures/srcdoc-origin.js"),
        json!(server.cross)
    );
    let observed = results(
        page.evaluate_runtime_expression_with_await_async(&expression, true)
            .await?,
    )?;
    assert_eq!(
        observed,
        json!({
            "inherited": {"relative":"TypeError", "home":"ok", "redirect":"TypeError", "blob":"local", "data":"data", "cors":"ok", "preflight":"ok"},
            "opaque": {"relative":"TypeError", "home":"TypeError", "redirect":"TypeError", "blob":"TypeError", "data":"data", "cors":"ok", "preflight":"ok"},
            "topBase":"TypeError"
        })
    );
    let requests = server.requests.lock();
    assert!(
        !requests
            .iter()
            .any(|request| request.path.starts_with("/forbidden-")),
        "{requests:?}"
    );
    let home = requests
        .iter()
        .find(|request| request.path == "/allowed-home")
        .expect("same-origin control");
    assert_eq!(
        home.cookie.as_deref(),
        Some("originSession=present"),
        "{home:?}"
    );
    for (kind, expected_origin) in [("inherited", server.origin.as_str()), ("opaque", "null")] {
        for (method, path) in [
            ("GET", format!("/cors-{kind}")),
            ("OPTIONS", format!("/preflight-{kind}")),
            ("PUT", format!("/preflight-{kind}")),
        ] {
            let request = requests
                .iter()
                .find(|request| request.method == method && request.path == path)
                .expect("expected allowed request");
            assert_eq!(
                request.origin.as_deref(),
                Some(expected_origin),
                "{request:?}"
            );
        }
    }
    Ok(())
}

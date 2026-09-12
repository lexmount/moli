use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const SCRIPT: &str = "globalThis.sriExecutions = (globalThis.sriExecutions || 0) + 1;";
const INTEGRITY: &str = "sha384-T7tuz8k7Hz0eBaWUPKiAEECRmaKHLJ1eRz7NF4VdK1fN++IaKD3hEk0SETOP+8aJ";

struct IntegrityServers {
    origin: String,
    cross_origin: String,
    requests: Arc<Mutex<Vec<IntegrityRequest>>>,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Debug)]
struct IntegrityRequest {
    method: String,
    path: String,
    origin: Option<String>,
    cookie: Option<String>,
    sec_fetch_dest: Option<String>,
    sec_fetch_mode: Option<String>,
    sec_fetch_site: Option<String>,
}

impl Drop for IntegrityServers {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl IntegrityServers {
    async fn spawn() -> Result<Self> {
        let main = TcpListener::bind("127.0.0.1:0").await?;
        let cross = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", main.local_addr()?);
        let cross_origin = format!("http://{}", cross.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let tasks = [main, cross]
            .into_iter()
            .map(|listener| {
                let origin = origin.clone();
                let cross_origin = cross_origin.clone();
                let requests = Arc::clone(&requests);
                tokio::spawn(async move {
                    let mut connections = JoinSet::new();
                    loop {
                        tokio::select! {
                            accepted = listener.accept() => {
                                let Ok((mut stream, _)) = accepted else { break };
                                let origin = origin.clone();
                                let cross_origin = cross_origin.clone();
                                let requests = Arc::clone(&requests);
                                connections.spawn(async move {
                                    let mut request = Vec::new();
                                    while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                                        let Ok(byte) = stream.read_u8().await else { return };
                                        request.push(byte);
                                    }
                                    let request = String::from_utf8_lossy(&request);
                                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                                    let header = |name: &str| request.lines().filter_map(|line| line.split_once(':'))
                                        .find(|(key, _)| key.eq_ignore_ascii_case(name))
                                        .map(|(_, value)| value.trim().to_owned());
                                    let incoming = IntegrityRequest {
                                        method: request.split_whitespace().next().unwrap_or("GET").to_owned(),
                                        path: path.to_owned(), origin: header("Origin"), cookie: header("Cookie"),
                                        sec_fetch_dest: header("Sec-Fetch-Dest"),
                                        sec_fetch_mode: header("Sec-Fetch-Mode"),
                                        sec_fetch_site: header("Sec-Fetch-Site"),
                                    };
                                    let (status, headers, body) = fixture_response(&incoming, &origin, &cross_origin);
                                    requests.lock().push(incoming);
                                    let response = format!(
                                        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                        body.len(),
                                    );
                                    let _ = stream.write_all(response.as_bytes()).await;
                                });
                            }
                            _ = connections.join_next(), if !connections.is_empty() => {}
                        }
                    }
                })
            })
            .collect();
        Ok(Self {
            origin,
            cross_origin,
            requests,
            tasks,
        })
    }
}

fn network_cases(origin: &str, cross: &str) -> serde_json::Value {
    serde_json::json!([
        {"name": "same-origin", "src": format!("{origin}/script.js"), "integrity": INTEGRITY, "expected": "load"},
        {"name": "cross-no-cors", "src": format!("{cross}/script.js"), "integrity": INTEGRITY, "expected": "error"},
        {"name": "cross-anonymous", "src": format!("{cross}/cors.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "load"},
        {"name": "cross-no-acao", "src": format!("{cross}/script.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "error"},
        {"name": "cors-without-integrity", "src": format!("{cross}/script.js"), "crossOrigin": "anonymous", "expected": "error"},
        {"name": "cors-empty-integrity", "src": format!("{cross}/script.js"), "integrity": "", "crossOrigin": "anonymous", "expected": "error"},
        {"name": "cors-unsupported-integrity", "src": format!("{cross}/script.js"), "integrity": "sha1-ignored", "crossOrigin": "anonymous", "expected": "error"},
        {"name": "redirect-cross", "src": format!("{origin}/redirect.js"), "integrity": INTEGRITY, "expected": "error"},
        {"name": "redirect-home", "src": format!("{origin}/roundtrip.js"), "integrity": INTEGRITY, "expected": "error"},
        {"name": "cors-redirect", "src": format!("{origin}/cors-redirect.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "load"},
        {"name": "cors-unapproved-hop", "src": format!("{cross}/unapproved-redirect.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "error"},
        {"name": "cors-null-origin", "src": format!("{cross}/cors-redirect-null.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "load"},
        {"name": "cors-old-origin", "src": format!("{cross}/cors-redirect-origin.js"), "integrity": INTEGRITY, "crossOrigin": "anonymous", "expected": "error"},
        {"name": "absent-integrity", "src": format!("{cross}/script.js"), "expected": "load"},
        {"name": "empty-integrity", "src": format!("{cross}/script.js"), "integrity": "", "expected": "load"},
        {"name": "unsupported-integrity", "src": format!("{cross}/script.js"), "integrity": "sha1-ignored", "expected": "load"},
        {"name": "mismatch", "src": format!("{origin}/script.js"), "integrity": "sha384-foobar", "expected": "error"}
    ])
}

fn fixture_response(
    request: &IntegrityRequest,
    origin: &str,
    cross: &str,
) -> (&'static str, String, String) {
    let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
    let javascript = "Content-Type: text/javascript\r\n";
    match path {
        "/echo-origin.js"
        | "/echo-origin-redirect.js"
        | "/echo-origin-home.js"
        | "/echo-origin-roundtrip.js"
        | "/module-graph.js" => {
            // Echo the actual request Origin: a missing header must not silently
            // receive ACAO: null and hide a broken redirect handoff.
            let mut headers = format!(
                "{javascript}Cache-Control: no-store\r\nVary: Origin\r\nX-Sri-Private: yes\r\n"
            );
            if let Some(origin) = &request.origin {
                headers.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\n"));
            }
            if request.method == "OPTIONS" {
                headers.push_str("Access-Control-Allow-Methods: GET, PUT\r\nAccess-Control-Allow-Headers: x-sri-test\r\n");
                return ("204 No Content", headers, String::new());
            }
            let target = match path {
                "/echo-origin-redirect.js" => Some(format!("{cross}/echo-origin.js?{query}")),
                "/echo-origin-home.js" => Some(format!("{origin}/echo-origin.js?{query}")),
                "/echo-origin-roundtrip.js" => Some(format!("{cross}/echo-origin-home.js?{query}")),
                _ => None,
            };
            if let Some(target) = target {
                headers.push_str(&format!("Location: {target}\r\n"));
                ("302 Found", headers, String::new())
            } else if path == "/module-graph.js" {
                (
                    "200 OK",
                    headers,
                    format!(
                        "import '{cross}/{}';",
                        if query == "graph-blocked" {
                            "script.js?blocked-static"
                        } else {
                            "echo-origin.js?static-dependency"
                        }
                    ),
                )
            } else {
                ("200 OK", headers, SCRIPT.to_owned())
            }
        }
        "/style.css"
        | "/style-roundtrip.css"
        | "/style-home.css"
        | "/app.webmanifest"
        | "/manifest-roundtrip.webmanifest"
        | "/manifest-home.webmanifest" => {
            let mut headers = "Cache-Control: no-store\r\nVary: Origin\r\n".to_owned();
            if let Some(incoming_origin) = &request.origin
                && !(query == "bad-hop" && path.contains("-home."))
            {
                let allowed_origin = if query == "bad-final" && !path.contains('-') {
                    origin
                } else {
                    incoming_origin
                };
                headers.push_str(&format!(
                    "Access-Control-Allow-Origin: {allowed_origin}\r\n"
                ));
            }
            let target = match path {
                "/style-roundtrip.css" => Some(format!("{cross}/style-home.css?{query}")),
                "/style-home.css" => Some(format!("{origin}/style.css?{query}")),
                "/manifest-roundtrip.webmanifest" => {
                    Some(format!("{cross}/manifest-home.webmanifest?{query}"))
                }
                "/manifest-home.webmanifest" => Some(format!("{origin}/app.webmanifest?{query}")),
                _ => None,
            };
            if let Some(target) = target {
                headers.push_str(&format!("Location: {target}\r\n"));
                ("302 Found", headers, String::new())
            } else if path.ends_with(".css") {
                headers.push_str("Content-Type: text/css\r\n");
                ("200 OK", headers, "body { color: green; }".to_owned())
            } else {
                headers.push_str("Content-Type: application/manifest+json\r\n");
                (
                    "200 OK",
                    headers,
                    r#"{"name":"Redirected manifest"}"#.to_owned(),
                )
            }
        }
        "/manifest-page.html" => (
            "200 OK",
            "Content-Type: text/html\r\n".to_owned(),
            format!(
                "<!doctype html><link rel=manifest href='/{}?{query}'>",
                if query == "same-origin" {
                    "app.webmanifest"
                } else {
                    "manifest-roundtrip.webmanifest"
                }
            ),
        ),
        "/script.js" => ("200 OK", javascript.to_owned(), SCRIPT.to_owned()),
        "/cacheable.js" => (
            "200 OK",
            format!("{javascript}Cache-Control: max-age=60\r\n"),
            SCRIPT.to_owned(),
        ),
        "/cors.js" | "/cors-null.js" | "/cors-origin.js" => (
            "200 OK",
            format!(
                "{javascript}Access-Control-Allow-Origin: {}\r\n",
                match path {
                    "/cors-null.js" => "null",
                    "/cors-origin.js" => origin,
                    _ => "*",
                }
            ),
            SCRIPT.to_owned(),
        ),
        "/redirect.js"
        | "/roundtrip.js"
        | "/redirect-home.js"
        | "/cors-redirect.js"
        | "/unapproved-redirect.js"
        | "/cors-redirect-null.js"
        | "/cors-redirect-origin.js" => {
            let target = match path {
                "/redirect.js" => format!("{cross}/script.js"),
                "/roundtrip.js" => format!("{cross}/redirect-home.js"),
                "/cors-redirect.js" => format!("{cross}/cors.js"),
                "/unapproved-redirect.js" => format!("{origin}/cors.js"),
                "/cors-redirect-null.js" => format!("{origin}/cors-null.js"),
                "/cors-redirect-origin.js" => format!("{origin}/cors-origin.js"),
                _ => format!("{origin}/script.js"),
            };
            let acao = if path.starts_with("/cors-redirect") {
                "Access-Control-Allow-Origin: *\r\n"
            } else {
                ""
            };
            (
                "302 Found",
                format!("Location: {target}\r\n{acao}"),
                String::new(),
            )
        }
        "/worker.js" => {
            let body = format!(
                r#"
                self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
                self.addEventListener('activate', event => event.waitUntil(clients.claim()));
                self.addEventListener('fetch', event => {{
                    const path = new URL(event.request.url).pathname;
                    if (path === '/sw-opaque.js')
                        event.respondWith(fetch('{cross}/script.js', {{mode: 'no-cors'}}));
                    else if (path === '/sw-basic.js')
                        event.respondWith(fetch('{origin}/script.js'));
                    else if (path === '/sw-cors.js')
                        event.respondWith(fetch('{cross}/cors.js'));
                    else if (path === '/sw-default.js')
                        event.respondWith(new Response({script}, {{headers: {{'Content-Type': 'text/javascript'}}}}));
                    else if (path === '/sw-redirect.js')
                        event.respondWith(Response.redirect('{cross}/script.js'));
                    else if (path === '/sw-roundtrip.js')
                        event.respondWith(Response.redirect('{cross}/redirect-home.js'));
                    else if (path === '/sw-cors-redirect.js')
                        event.respondWith(Response.redirect('{origin}/cors.js'));
                    else if (path === '/sw-cors-redirect-null.js')
                        event.respondWith(Response.redirect('{origin}/cors-null.js'));
                    else if (path === '/sw-cors-redirect-origin.js')
                        event.respondWith(Response.redirect('{origin}/cors-origin.js'));
                    else if (path === '/sw-cors-redirect-no-acao.js')
                        event.respondWith(Response.redirect('{origin}/script.js'));
                    else if (path === '/sw-cors-redirect-unapproved-hop.js')
                        event.respondWith(Response.redirect('{cross}/unapproved-redirect.js'));
                    else if (path === '/sw-request-state.js')
                        event.respondWith(Response.redirect('{origin}/echo-origin.js' + new URL(event.request.url).search));
                    else if (path === '/sw-request-state-redirect.js')
                        event.respondWith(Response.redirect('{origin}/echo-origin-redirect.js' + new URL(event.request.url).search));
                    else if (path === '/sw-cacheable.js')
                        event.respondWith(Response.redirect('{origin}/cacheable.js'));
                    else if (path.startsWith('/sw-return-'))
                        event.respondWith(Response.redirect('{origin}' + path.replace('/sw-return-', '/sw-final-')));
                    else if (path === '/sw-final-basic.js')
                        event.respondWith(fetch('{origin}/echo-origin.js?worker-basic'));
                    else if (path === '/sw-final-basic-buffered.js')
                        event.respondWith(fetch('{origin}/echo-origin.js?worker-basic-buffered').then(async response => {{
                            await response.clone().text();
                            return response;
                        }}));
                    else if (path === '/sw-final-cors.js')
                        event.respondWith(fetch('{cross}/echo-origin.js?worker-cors'));
                    else if (path === '/sw-final-default.js')
                        event.respondWith(new Response({script}, {{headers: {{'Content-Type': 'text/javascript', 'X-Sri-Private': 'yes'}}}}));
                }});
            "#,
                script = serde_json::to_string(SCRIPT).unwrap()
            );
            ("200 OK", javascript.to_owned(), body)
        }
        "/page.html" | "/parser.html" | "/cookie-page.html" | "/csp-page.html" => {
            let mut html = "<!doctype html><body><script>globalThis.sriEvents = {}; globalThis.sriExecutions = 0;</script>".to_owned();
            if path == "/csp-page.html" {
                html.insert_str(15, "<meta http-equiv='Content-Security-Policy' content=\"script-src 'self' 'unsafe-inline'\">");
            }
            if path == "/parser.html" {
                for case in network_cases(origin, cross).as_array().unwrap() {
                    let name = case["name"].as_str().unwrap();
                    html.push_str(&format!("<script src=\"{}\" onload=\"sriEvents['{name}']='load'\" onerror=\"sriEvents['{name}']='error'\"", case["src"].as_str().unwrap()));
                    for attribute in ["integrity", "crossOrigin"] {
                        if let Some(value) = case[attribute].as_str() {
                            html.push_str(&format!(" {attribute}=\"{value}\""));
                        }
                    }
                    html.push_str("></script>");
                }
            }
            let mut headers = "Content-Type: text/html\r\n".to_owned();
            if path == "/cookie-page.html" {
                headers.push_str("Set-Cookie: sriSession=present; Path=/; SameSite=Lax\r\n");
            }
            ("200 OK", headers, html)
        }
        _ => (
            "404 Not Found",
            "Content-Type: text/plain\r\n".to_owned(),
            "not found".to_owned(),
        ),
    }
}

fn assert_integrity_results(result: serde_json::Value, cases: &serde_json::Value) {
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("integrity result JSON")).unwrap();
    let mut executions = 0;
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        assert_eq!(result["events"][name], case["expected"], "{name}: {result}");
        executions += usize::from(case["expected"] == "load");
    }
    assert_eq!(
        result["executions"], executions,
        "rejected scripts must not execute"
    );
}

fn dynamic_probe(cases: &serde_json::Value, service_worker: bool) -> String {
    format!(
        r#"(async () => {{
        if ({service_worker}) {{
            const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
                new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once: true}}));
            await navigator.serviceWorker.register('/worker.js', {{scope: '/'}});
            await navigator.serviceWorker.ready;
            await controlled;
        }}
        for (const test of {cases}) {{
            sriEvents[test.name] = await new Promise(resolve => {{
                const script = document.createElement('script');
                script.src = test.src;
                if ('type' in test) script.type = test.type;
                if ('integrity' in test) script.integrity = test.integrity;
                if ('crossOrigin' in test) script.crossOrigin = test.crossOrigin;
                script.onload = () => resolve('load');
                script.onerror = () => resolve('error');
                document.body.append(script);
            }});
        }}
        return JSON.stringify({{events: sriEvents, executions: sriExecutions}});
    }})()"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_integrity_requires_readable_response() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/parser.html", servers.origin))
        .await?;
    let result = page
        .evaluate_runtime_expression_async(
            "JSON.stringify({events: sriEvents, executions: sriExecutions})",
        )
        .await?;
    assert_integrity_results(
        result,
        &network_cases(&servers.origin, &servers.cross_origin),
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_integrity_requires_readable_response() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let cases = network_cases(&servers.origin, &servers.cross_origin);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, false), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_integrity_requires_readable_response() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let cases = serde_json::json!([
        {"name": "same-origin", "src": "/script.js", "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "cors", "src": format!("{}/cors.js", servers.cross_origin), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "no-acao", "src": format!("{}/script.js", servers.cross_origin), "type": "module", "integrity": INTEGRITY, "expected": "error"},
        {"name": "without-integrity", "src": format!("{}/script.js?absent", servers.cross_origin), "type": "module", "expected": "error"},
        {"name": "empty-integrity", "src": format!("{}/script.js?empty", servers.cross_origin), "type": "module", "integrity": "", "expected": "error"},
        {"name": "unsupported-integrity", "src": format!("{}/script.js?unsupported", servers.cross_origin), "type": "module", "integrity": "sha1-ignored", "expected": "error"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, false), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_scripts_send_origin_and_fetch_metadata() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    let origin = &servers.origin;
    let cross = &servers.cross_origin;
    let cases = serde_json::json!([
        {"name": "control", "src": "/echo-origin.js?control", "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "classic", "src": format!("{cross}/echo-origin.js?classic"), "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module", "src": format!("{cross}/echo-origin.js?module"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "include", "src": format!("{cross}/echo-origin.js?include"), "type": "module", "crossOrigin": "use-credentials", "integrity": INTEGRITY, "expected": "load"},
        {"name": "home", "src": format!("{cross}/echo-origin-home.js?home"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "roundtrip", "src": "/echo-origin-roundtrip.js?roundtrip", "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "no-cors", "src": format!("{cross}/echo-origin.js?no-cors"), "expected": "load"},
        {"name": "graph", "src": "/module-graph.js?graph", "type": "module", "expected": "load"},
        {"name": "graph-blocked", "src": "/module-graph.js?graph-blocked", "type": "module", "expected": "error"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, false), true)
        .await?;
    assert_integrity_results(result, &cases);
    let imported = page
        .evaluate_runtime_expression_with_await_async(
            &format!("import('{cross}/echo-origin.js?dynamic-import').then(() => 'loaded')"),
            true,
        )
        .await?;
    assert_eq!(imported["value"], "loaded");
    let rejected = page
        .evaluate_runtime_expression_with_await_async(
            &format!(
                "import('{cross}/script.js?blocked-dynamic').then(() => 'loaded', () => 'rejected')"
            ),
            true,
        )
        .await?;
    assert_eq!(rejected["value"], "rejected");
    let executions = page
        .evaluate_runtime_expression_async("sriExecutions")
        .await?;
    assert_eq!(
        executions["value"], 9,
        "CORS-rejected dependencies must not execute"
    );

    let requests = servers.requests.lock();
    for name in [
        "control",
        "classic",
        "module",
        "include",
        "home",
        "roundtrip",
        "no-cors",
        "graph",
        "graph-blocked",
        "static-dependency",
        "dynamic-import",
        "blocked-static",
        "blocked-dynamic",
    ] {
        let actual: Vec<_> = requests
            .iter()
            .filter(|request| request.path.ends_with(&format!("?{name}")))
            .collect();
        let expected_origins = match name {
            "home" => vec![Some(origin.as_str()), Some("null")],
            "roundtrip" => vec![None, Some(origin.as_str()), Some("null")],
            "control" | "graph" | "graph-blocked" | "no-cors" => vec![None],
            _ => vec![Some(origin.as_str())],
        };
        assert_eq!(actual.len(), expected_origins.len(), "{name}: {actual:?}");
        for (index, (request, expected_origin)) in actual.iter().zip(expected_origins).enumerate() {
            assert_eq!(
                request.origin.as_deref(),
                expected_origin,
                "{name}: {request:?}"
            );
            assert_eq!(
                request.sec_fetch_dest.as_deref(),
                Some("script"),
                "{name}: {request:?}"
            );
            assert_eq!(
                request.sec_fetch_mode.as_deref(),
                Some(if name == "no-cors" { "no-cors" } else { "cors" }),
                "{name}: {request:?}"
            );
            let same_origin = matches!(name, "control" | "graph" | "graph-blocked")
                || (name == "roundtrip" && index == 0);
            assert_eq!(
                request.sec_fetch_site.as_deref(),
                Some(if same_origin {
                    "same-origin"
                } else {
                    "same-site"
                }),
                "{name}: {request:?}"
            );
            assert_eq!(
                request.cookie.as_deref(),
                if same_origin || matches!(name, "include" | "no-cors") {
                    Some("sriSession=present")
                } else {
                    None
                },
                "{name}: {request:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn srcdoc_module_requests_use_window_origin_instead_of_base_url() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    let result = page.evaluate_runtime_expression_with_await_async(&format!(r#"(async () => {{
        const results = [];
        for (const kind of ['module', 'classic', 'preload', 'dynamic']) {{
        for (const originKind of ['same-origin', 'inherited', 'sandboxed']) {{
            const name = originKind + '-' + kind;
            const frame = document.createElement('iframe');
            if (originKind === 'sandboxed') frame.setAttribute('sandbox', 'allow-scripts');
            const terminal = new Promise(resolve => {{
                const handler = event => {{
                    if (event.source !== frame.contentWindow || !String(event.data).startsWith(name + ':')) return;
                    removeEventListener('message', handler);
                    resolve(event.data);
                }};
                addEventListener('message', handler);
            }});
            const terminalAttributes = `onload="parent.postMessage('${{name}}:load', '*')" onerror="parent.postMessage('${{name}}:error', '*')"`;
            const source = './echo-origin.js?' + name;
            const resource = kind === 'preload'
                ? `<link rel='modulepreload' href='${{source}}' integrity='{INTEGRITY}' ${{terminalAttributes}}>`
                : kind === 'dynamic'
                    ? `<script>import('${{source}}').then(() => parent.postMessage('${{name}}:load', '*'), () => parent.postMessage('${{name}}:error', '*'));<\/script>`
                    : `<script type='${{kind === 'module' ? 'module' : 'text/javascript'}}' crossorigin='anonymous' src='${{source}}' integrity='{INTEGRITY}' ${{terminalAttributes}}><\/script>`;
            frame.srcdoc = `<base href='${{originKind === 'same-origin' ? location.origin : '{cross}'}}/'>` + resource;
            document.body.append(frame);
            results.push(await terminal);
            frame.remove();
        }}
        }}
        return JSON.stringify(results);
    }})()"#, cross = servers.cross_origin), true).await?;
    let result: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    let mut expected = Vec::new();
    let requests = servers.requests.lock();
    for kind in ["module", "classic", "preload", "dynamic"] {
        for (origin_kind, origin) in [
            ("same-origin", None),
            ("inherited", Some(servers.origin.as_str())),
            ("sandboxed", Some("null")),
        ] {
            let name = format!("{origin_kind}-{kind}");
            expected.push(format!("{name}:load"));
            let request = requests
                .iter()
                .find(|request| request.path == format!("/echo-origin.js?{name}"))
                .expect("child script request");
            assert_eq!(request.origin.as_deref(), origin, "{request:?}");
            assert_eq!(request.sec_fetch_dest.as_deref(), Some("script"));
            assert_eq!(request.sec_fetch_mode.as_deref(), Some("cors"));
            assert_eq!(
                request.cookie.as_deref(),
                if origin_kind == "same-origin" {
                    Some("sriSession=present")
                } else {
                    None
                },
                "{request:?}"
            );
        }
    }
    assert_eq!(result, serde_json::json!(expected));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_blocks_dynamic_scripts_before_cors_fetch_and_reports_violation() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/csp-page.html", servers.origin))
        .await?;
    let result = page.evaluate_runtime_expression_with_await_async(&format!(r#"(async () => {{
        const results = [];
        for (const type of ['', 'module']) {{
            const script = document.createElement('script');
            script.type = type;
            script.crossOrigin = 'anonymous';
            script.src = '{cross}/script.js?csp-blocked-' + type;
            const violation = new Promise(resolve => {{
                document.addEventListener('securitypolicyviolation', event => resolve(event.effectiveDirective), {{once: true}});
            }});
            const terminal = new Promise(resolve => {{
                script.onload = () => resolve('load');
                script.onerror = () => resolve('error');
            }});
            document.body.append(script);
            results.push(await Promise.all([terminal, violation]));
        }}
        return JSON.stringify(results);
    }})()"#, cross = servers.cross_origin), true).await?;
    let result: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    assert_eq!(
        result,
        serde_json::json!([["error", "script-src-elem"], ["error", "script-src-elem"]])
    );
    assert!(
        !servers
            .requests
            .lock()
            .iter()
            .any(|request| request.path.contains("csp-blocked")),
        "CSP must reject before sending a request"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_roundtrip_preserves_cors_validation_and_cssom_origin_clean() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(async () => {
        const results = {};
        for (const name of ['same-origin', 'no-cors', 'allow', 'bad-hop', 'bad-final']) {
            const link = document.createElement('link');
            link.rel = 'stylesheet';
            link.href = (name === 'same-origin' ? '/style.css?' : '/style-roundtrip.css?') + name;
            if (!['same-origin', 'no-cors'].includes(name)) link.crossOrigin = 'anonymous';
            const event = await new Promise(resolve => {
                link.onload = () => resolve('load');
                link.onerror = () => resolve('error');
                document.head.append(link);
            });
            let rules;
            try { rules = link.sheet ? link.sheet.cssRules.length : null; }
            catch (error) { rules = error.name; }
            results[name] = {event, rules};
            link.remove();
        }
        return JSON.stringify(results);
    })()"#,
            true,
        )
        .await?;
    let result: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    assert_eq!(
        result,
        serde_json::json!({
            "same-origin": {"event": "load", "rules": 1},
            "no-cors": {"event": "load", "rules": "SecurityError"},
            "allow": {"event": "load", "rules": 1},
            // Chromium keeps an unreadable empty sheet after a failed load.
            "bad-hop": {"event": "error", "rules": "SecurityError"},
            "bad-final": {"event": "error", "rules": "SecurityError"}
        })
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn manifest_roundtrip_checks_intermediate_and_final_cors_responses() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for name in ["same-origin", "allow", "bad-hop", "bad-final"] {
        let mut page = browser
            .fetch(&format!("{}/manifest-page.html?{name}", servers.origin))
            .await?;
        let completion = page.start_prepare_app_manifest_load()?.wait().await?;
        let moli_renderer_v8::RendererAppManifestLoadPreparation::Ready(load) =
            page.finish_prepare_app_manifest_load(completion)?
        else {
            anyhow::bail!("{name}: expected a network manifest load");
        };
        let (result, publication) = load.execute().await.into_parts();
        let completion = page
            .start_publish_app_manifest_load(publication)?
            .wait()
            .await?;
        page.finish_publish_app_manifest_load(completion)?;
        let allowed = matches!(name, "same-origin" | "allow");
        assert_eq!(
            result.data.as_deref(),
            Some(if allowed {
                r#"{"name":"Redirected manifest"}"#
            } else {
                ""
            }),
            "{name}: {result:?}",
        );
        assert_eq!(
            result.manifest.name.as_deref(),
            allowed.then_some("Redirected manifest"),
            "{name}: {result:?}"
        );
    }
    let requests = servers.requests.lock();
    let roundtrip: Vec<_> = requests
        .iter()
        .filter(|request| request.path.ends_with(".webmanifest?allow"))
        .collect();
    assert_eq!(roundtrip.len(), 3, "{roundtrip:?}");
    for (request, expected_origin) in
        roundtrip
            .iter()
            .zip([None, Some(servers.origin.as_str()), Some("null")])
    {
        assert_eq!(request.origin.as_deref(), expected_origin, "{request:?}");
        assert_eq!(request.sec_fetch_dest.as_deref(), Some("manifest"));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_script_integrity_preserves_response_filter() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let cases = serde_json::json!([
        {"name": "opaque", "src": "/sw-opaque.js", "integrity": INTEGRITY, "expected": "error"},
        {"name": "basic", "src": "/sw-basic.js", "integrity": INTEGRITY, "expected": "load"},
        {"name": "cors", "src": "/sw-cors.js", "integrity": INTEGRITY, "expected": "load"},
        {"name": "default", "src": "/sw-default.js", "integrity": INTEGRITY, "expected": "load"},
        {"name": "redirect-cross", "src": "/sw-redirect.js", "integrity": INTEGRITY, "expected": "error"},
        {"name": "redirect-home", "src": "/sw-roundtrip.js", "integrity": INTEGRITY, "expected": "error"},
        {"name": "opaque-without-integrity", "src": "/sw-opaque.js", "expected": "load"},
        {"name": "opaque-empty-integrity", "src": "/sw-opaque.js", "integrity": "", "expected": "load"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, true), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_cross_origin_redirect_integrity_checks_network_responses() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let cross = &servers.cross_origin;
    // These requests start cross-origin, so the worker's synthetic response
    // must be distinguished from a cross-origin HTTP redirect without ACAO.
    let cases = serde_json::json!([
        {"name": "classic-cors", "src": format!("{cross}/sw-cors-redirect.js"), "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module-cors", "src": format!("{cross}/sw-cors-redirect.js"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module-null-origin", "src": format!("{cross}/sw-cors-redirect-null.js"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module-old-origin", "src": format!("{cross}/sw-cors-redirect-origin.js"), "type": "module", "integrity": INTEGRITY, "expected": "error"},
        {"name": "module-no-acao", "src": format!("{cross}/sw-cors-redirect-no-acao.js"), "type": "module", "integrity": INTEGRITY, "expected": "error"},
        {"name": "module-unapproved-hop", "src": format!("{cross}/sw-cors-redirect-unapproved-hop.js"), "type": "module", "integrity": INTEGRITY, "expected": "error"},
        {"name": "classic-opaque", "src": format!("{cross}/sw-cors-redirect.js"), "integrity": INTEGRITY, "expected": "error"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, true), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_script_redirect_preserves_origin_and_credentials() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    let cross = &servers.cross_origin;
    let cases = serde_json::json!([
        {"name": "control", "src": "/echo-origin.js?control", "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module", "src": format!("{cross}/sw-request-state.js?module"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "classic", "src": format!("{cross}/sw-request-state.js?classic"), "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module-include", "src": format!("{cross}/sw-request-state.js?module-include"), "type": "module", "crossOrigin": "use-credentials", "integrity": INTEGRITY, "expected": "load"},
        {"name": "classic-include", "src": format!("{cross}/sw-request-state.js?classic-include"), "crossOrigin": "use-credentials", "integrity": INTEGRITY, "expected": "load"},
        {"name": "module-chain", "src": format!("{cross}/sw-request-state-redirect.js?module-chain"), "type": "module", "integrity": INTEGRITY, "expected": "load"},
        {"name": "classic-chain", "src": format!("{cross}/sw-request-state-redirect.js?classic-chain"), "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "load"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, true), true)
        .await?;
    {
        let requests = servers.requests.lock();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let requests: Vec<_> = requests
                .iter()
                .filter(|request| request.path.ends_with(&format!("?{name}")))
                .collect();
            assert_eq!(
                requests.len(),
                if name.ends_with("-chain") { 2 } else { 1 },
                "{name}: {requests:?}"
            );
            for request in requests {
                assert_eq!(
                    request.sec_fetch_dest.as_deref(),
                    Some("script"),
                    "{name}: {request:?}"
                );
                assert_eq!(
                    request.sec_fetch_mode.as_deref(),
                    Some("cors"),
                    "{name}: {request:?}"
                );
                assert_eq!(
                    request.origin.as_deref(),
                    if name == "control" {
                        None
                    } else {
                        Some("null")
                    },
                    "{name}: {request:?}"
                );
                assert_eq!(
                    request.cookie.as_deref(),
                    if name == "control" || name.ends_with("-include") {
                        Some("sriSession=present")
                    } else {
                        None
                    },
                    "{name}: {request:?}"
                );
            }
        }
    }
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_fetch_redirect_preserves_origin_and_credentials() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    page.evaluate_runtime_expression_with_await_async(
        &dynamic_probe(&serde_json::json!([]), true),
        true,
    )
    .await?;
    let cross = &servers.cross_origin;
    let cases = serde_json::json!([
        {"name": "control", "src": "/echo-origin.js?control", "credentials": "same-origin"},
        {"name": "same-origin", "src": format!("{cross}/sw-request-state.js?same-origin"), "credentials": "same-origin"},
        {"name": "include", "src": format!("{cross}/sw-request-state.js?include"), "credentials": "include"},
        {"name": "omit", "src": format!("{cross}/sw-request-state.js?omit"), "credentials": "omit"},
        {"name": "preflight", "src": format!("{cross}/sw-request-state.js?preflight"), "credentials": "same-origin", "method": "PUT", "headers": {"X-Sri-Test": "yes"}},
        {"name": "chain", "src": format!("{cross}/sw-request-state-redirect.js?chain"), "credentials": "same-origin"},
        {"name": "preflight-chain", "src": format!("{cross}/sw-request-state-redirect.js?preflight-chain"), "credentials": "same-origin", "method": "PUT", "headers": {"X-Sri-Test": "yes"}}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(
            &format!(
                r#"(async () => {{
        const results = {{}};
        for (const test of {cases}) {{
            try {{
                const response = await fetch(test.src, test);
                results[test.name] = {{body: await response.text(), type: response.type,
                    privateHeader: response.headers.get('X-Sri-Private')}};
            }}
            catch (error) {{ results[test.name] = String(error); }}
        }}
        return JSON.stringify(results);
    }})()"#
            ),
            true,
        )
        .await?;
    let results: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    let requests = servers.requests.lock();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let requests: Vec<_> = requests
            .iter()
            .filter(|request| request.path.ends_with(&format!("?{name}")))
            .collect();
        assert_eq!(
            requests.len(),
            match name {
                "preflight-chain" => 4,
                "preflight" | "chain" => 2,
                _ => 1,
            },
            "{name}: {requests:?}"
        );
        for request in requests {
            assert_eq!(
                request.origin.as_deref(),
                if name == "control" {
                    None
                } else {
                    Some("null")
                },
                "{name}: {request:?}"
            );
            assert_eq!(
                request.cookie.as_deref(),
                if name == "control" || name == "include" {
                    Some("sriSession=present")
                } else {
                    None
                },
                "{name}: {request:?}"
            );
        }
        assert_eq!(results[name]["body"], SCRIPT, "{name}: {results}");
        assert_eq!(
            results[name]["type"],
            if name == "control" { "basic" } else { "cors" },
            "{name}: {results}"
        );
        assert_eq!(
            results[name]["privateHeader"],
            if name == "control" {
                serde_json::json!("yes")
            } else {
                serde_json::Value::Null
            },
            "{name}: {results}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_script_redirect_does_not_reuse_another_response_url_list() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    // Prime the ordinary Fetch memory cache, and prove the entry is reusable
    // before introducing a different URL list through the worker.
    let result = page.evaluate_runtime_expression_with_await_async(
        "(async () => { await (await fetch('/cacheable.js')).text(); return await (await fetch('/cacheable.js')).text(); })()",
        true,
    ).await?;
    assert_eq!(result["value"], SCRIPT);
    assert_eq!(
        servers
            .requests
            .lock()
            .iter()
            .filter(|request| request.path == "/cacheable.js")
            .count(),
        1
    );
    let cases = serde_json::json!([
        {"name": "same-origin", "src": "/sw-cacheable.js", "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "load"},
        {"name": "cross-origin", "src": format!("{}/sw-cacheable.js", servers.cross_origin), "crossOrigin": "anonymous", "integrity": INTEGRITY, "expected": "error"}
    ]);
    // Both fetches end at the same cacheable URL. Its body can be reused, but
    // the first response's same-origin URL list must not authorize the second.
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, true), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_redirect_preserves_worker_response_filter() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    page.evaluate_runtime_expression_with_await_async(
        &dynamic_probe(&serde_json::json!([]), true),
        true,
    )
    .await?;
    let result = page.evaluate_runtime_expression_with_await_async(
        &format!(r#"(async () => {{
            const results = {{}};
            for (const kind of ['basic', 'basic-buffered', 'cors', 'default']) {{
                try {{
                    const response = await fetch('{cross}/sw-return-' + kind + '.js');
                    results[kind] = {{type: response.type, privateHeader: response.headers.get('X-Sri-Private'), body: await response.text()}};
                }} catch (error) {{ results[kind] = String(error); }}
            }}
            return JSON.stringify(results);
        }})()"#, cross = servers.cross_origin), true,
    ).await?;
    let results: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    for kind in ["basic", "basic-buffered", "cors", "default"] {
        assert_eq!(results[kind]["body"], SCRIPT, "{kind}: {results}");
        assert_eq!(
            results[kind]["type"],
            if kind.starts_with("basic") {
                "basic"
            } else {
                "cors"
            },
            "{kind}: {results}"
        );
        assert_eq!(
            results[kind]["privateHeader"],
            if kind.starts_with("basic") {
                serde_json::json!("yes")
            } else {
                serde_json::Value::Null
            },
            "{kind}: {results}"
        );
    }
    let cases = serde_json::Value::Array(
        ["basic", "basic-buffered", "cors", "default"]
            .into_iter()
            .map(|kind| {
                serde_json::json!({
                    "name": kind, "src": format!("{}/sw-return-{kind}.js", servers.cross_origin),
                    "type": "module", "integrity": INTEGRITY, "expected": "load"
                })
            })
            .collect(),
    );
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, false), true)
        .await?;
    assert_integrity_results(result, &cases);
    Ok(())
}

use super::*;
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
    tasks: Vec<JoinHandle<()>>,
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
        let tasks = [main, cross]
            .into_iter()
            .map(|listener| {
                let origin = origin.clone();
                let cross_origin = cross_origin.clone();
                tokio::spawn(async move {
                    let mut connections = JoinSet::new();
                    loop {
                        tokio::select! {
                            accepted = listener.accept() => {
                                let Ok((mut stream, _)) = accepted else { break };
                                let origin = origin.clone();
                                let cross_origin = cross_origin.clone();
                                connections.spawn(async move {
                                    let mut request = Vec::new();
                                    while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                                        let Ok(byte) = stream.read_u8().await else { return };
                                        request.push(byte);
                                    }
                                    let request = String::from_utf8_lossy(&request);
                                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                                    let path = path.split('?').next().unwrap_or(path);
                                    let (status, headers, body) = fixture_response(path, &origin, &cross_origin);
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

fn fixture_response(path: &str, origin: &str, cross: &str) -> (&'static str, String, String) {
    let javascript = "Content-Type: text/javascript\r\n";
    match path {
        "/script.js" => ("200 OK", javascript.to_owned(), SCRIPT.to_owned()),
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
                }});
            "#,
                script = serde_json::to_string(SCRIPT).unwrap()
            );
            ("200 OK", javascript.to_owned(), body)
        }
        "/page.html" | "/parser.html" => {
            let mut html = "<!doctype html><body><script>globalThis.sriEvents = {}; globalThis.sriExecutions = 0;</script>".to_owned();
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
            ("200 OK", "Content-Type: text/html\r\n".to_owned(), html)
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
        {"name": "no-acao", "src": format!("{}/script.js", servers.cross_origin), "type": "module", "integrity": INTEGRITY, "expected": "error"}
    ]);
    let result = page
        .evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, false), true)
        .await?;
    assert_integrity_results(result, &cases);
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

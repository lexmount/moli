use anyhow::{Result, ensure};
use axum::{Router, http::header::CONTENT_TYPE, response::Html, routing::get};
use moli_core::LayoutPolicy;
use moli_core::page::{RendererCaptureScreenshotReply, RendererPrintToPdfReply};
use moli_core::runtime::{Browser, BrowserConfig, RenderedDomWaitUntil};
use std::time::Duration;
use tokio::net::TcpListener;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Lazy))?;
    let script_attributes = if std::env::args().any(|arg| arg == "script") {
        "defer"
    } else {
        r#"type="module""#
    };
    let source = r#"
const b = document.createElement('button');
b.textContent = 'Moli runtime';
document.body.appendChild(b);
let n = 0; b.onclick = () => n++; b.click();
if (n !== 1) throw new Error('DOM activation failed');
const range = document.createRange(); range.selectNodeContents(b);
getSelection().addRange(range);
if (getSelection().isCollapsed) throw new Error('DOM selection failed');
getSelection().removeAllRanges();
const input = document.createElement('input'); document.body.appendChild(input);
input.value = 'abc'; input.setSelectionRange(1, 2); input.setRangeText('Z');
if (input.value !== 'aZc') throw new Error('text selection failed');
globalThis.__probeDomOK = true;
"#;
    let router = Router::new()
        .route(
            "/",
            get(move || async move {
                Html(format!(
                    r#"<!doctype html><script {script_attributes} src="/probe.js"></script><body></body>"#
                ))
            }),
        )
        .route(
            "/probe.js",
            get(move || async move { ([(CONTENT_TYPE, "text/javascript")], source) }),
        )
        .route(
            "/probe-sw.js",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/javascript")],
                    "self.addEventListener('install', e => e.waitUntil(self.skipWaiting())); self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));",
                )
            }),
        )
        .route(
            "/probe-shared-worker.js",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/javascript")],
                    r#"self.onconnect = ({ports: [port]}) => {
                        port.onmessage = async () => {
                            port.postMessage(await (await fetch('/probe-worker-resource')).text());
                        };
                    };"#,
                )
            }),
        )
        .route("/probe-worker-resource", get(|| async { "native-worker-network" }));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, router).await });
    let mut page = browser
        .fetch_with_wait_until(
            &base_url,
            RenderedDomWaitUntil::Idle,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_selector(&mut page, "button", Duration::from_secs(2))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.querySelector('button') !== null",
            Duration::from_secs(2),
        )
        .await?;
    let evaluation = page
        .evaluate_runtime_expression_async("globalThis.__probeDomOK === true ? 6 * 7 : 0")
        .await?;
    let value = ["/result/result/value", "/result/value", "/value"]
        .into_iter()
        .find_map(|path| evaluation.pointer(path))
        .and_then(|value| value.as_i64());
    ensure!(value == Some(42), "unexpected evaluation: {evaluation}");
    let promise = page
        .evaluate_runtime_expression_with_await_async("Promise.resolve(42)", true)
        .await?;
    let value = ["/result/result/value", "/result/value", "/value"]
        .into_iter()
        .find_map(|path| promise.pointer(path))
        .and_then(|value| value.as_i64());
    ensure!(
        value == Some(42),
        "unexpected promise evaluation: {promise}"
    );
    let page_and_shared_worker_fetch = page
        .evaluate_runtime_expression_with_await_async(
            r#"(() => {
                    const worker = new SharedWorker('/probe-shared-worker.js');
                    globalThis.__probeSharedWorker = worker;
                    globalThis.__probeSharedWorkerFetch = () => new Promise((resolve, reject) => {
                        worker.onerror = reject;
                        worker.port.onmessage = event => resolve(event.data);
                        worker.port.postMessage('network');
                    });
                    return Promise.all([
                        fetch('/probe-worker-resource').then(response => response.text()),
                        __probeSharedWorkerFetch()
                    ]).then(results => results.join('|'));
                })()"#,
            true,
        )
        .await?;
    ensure!(
        ["/result/result/value", "/result/value", "/value"]
            .into_iter()
            .find_map(|path| page_and_shared_worker_fetch.pointer(path))
            .and_then(|value| value.as_str())
            == Some("native-worker-network|native-worker-network"),
        "unexpected Page or SharedWorker fetch result: {page_and_shared_worker_fetch}"
    );
    let environment = moli_core::ProcessEnvironmentOwner::default();
    page.evaluate_runtime_expression_with_await_async(
        r#"new Promise(resolve => {
            const source = `
                self.onmessage = async event => {
                    if (event.data === 'network') {
                        const url = ${JSON.stringify(location.origin + '/probe-worker-resource')};
                        const text = await (await fetch(url)).text();
                        const xhrText = await new Promise((done, fail) => {
                            const xhr = new XMLHttpRequest(); xhr.open('GET', url);
                            xhr.onload = () => done(xhr.responseText); xhr.onerror = fail; xhr.send();
                        });
                        self.postMessage(text + '|' + xhrText);
                    } else {
                        self.postMessage(Intl.DateTimeFormat().resolvedOptions().timeZone);
                    }
                };
                self.postMessage('ready');
            `;
            const worker = new Worker(URL.createObjectURL(new Blob([source], {type: 'text/javascript'})));
            globalThis.__probeWorker = worker;
            globalThis.__probeWorkerRequest = (kind = 'timezone') => new Promise(done => {
                worker.onmessage = event => done(event.data); worker.postMessage(kind);
            });
            worker.onmessage = () => resolve(true);
        })"#,
        true,
    ).await?;
    let network = page
        .evaluate_runtime_expression_with_await_async("__probeWorkerRequest('network')", true)
        .await?;
    let value = ["/result/result/value", "/result/value", "/value"]
        .into_iter()
        .find_map(|path| network.pointer(path))
        .and_then(|value| value.as_str());
    ensure!(
        value == Some("native-worker-network|native-worker-network"),
        "unexpected Worker fetch/XHR result: {network}"
    );
    for timezone in ["Asia/Tokyo", "Europe/Paris"] {
        environment.set_timezone(Some(timezone))?;
        let result = page
            .evaluate_runtime_expression_async("Intl.DateTimeFormat().resolvedOptions().timeZone")
            .await?;
        let value = ["/result/result/value", "/result/value", "/value"]
            .into_iter()
            .find_map(|path| result.pointer(path))
            .and_then(|value| value.as_str());
        ensure!(
            value == Some(timezone),
            "timezone notification failed: {result}"
        );
        let result = page
            .evaluate_runtime_expression_with_await_async("__probeWorkerRequest()", true)
            .await?;
        let value = ["/result/result/value", "/result/value", "/value"]
            .into_iter()
            .find_map(|path| result.pointer(path))
            .and_then(|value| value.as_str());
        ensure!(
            value == Some(timezone),
            "worker timezone notification failed: {result}"
        );
    }
    page.evaluate_runtime_expression_async("__probeWorker.terminate()")
        .await?;
    let registration = page.evaluate_runtime_expression_with_await_async(
        "navigator.serviceWorker.register('/probe-sw.js').then(() => navigator.serviceWorker.ready).then(r => Boolean(r.active))",
        true,
    ).await?;
    ensure!(
        ["/result/result/value", "/result/value", "/value"]
            .into_iter()
            .find_map(|path| registration.pointer(path))
            .and_then(|value| value.as_bool())
            == Some(true),
        "service worker registration failed: {registration}"
    );
    browser
        .wait_for_script_truthy(
            &mut page,
            "navigator.serviceWorker.controller !== null",
            Duration::from_secs(5),
        )
        .await?;
    let controlled_worker = page
        .evaluate_runtime_expression_with_await_async(
            "__probeSharedWorkerFetch().finally(() => __probeSharedWorker.port.close())",
            true,
        )
        .await?;
    ensure!(
        ["/result/result/value", "/result/value", "/value"]
            .into_iter()
            .find_map(|path| controlled_worker.pointer(path))
            .and_then(|value| value.as_str())
            == Some("native-worker-network"),
        "SharedWorker fetch failed after ServiceWorker claim: {controlled_worker}"
    );
    let screenshot = page.start_capture_screenshot()?.wait().await?;
    let screenshot = page.finish_capture_screenshot(screenshot)?;
    match screenshot {
        RendererCaptureScreenshotReply::Captured(image) => {
            ensure!(image.bytes.starts_with(b"\x89PNG"), "invalid screenshot");
            if let Some(directory) = std::env::var_os("MOLI_PROBE_ARTIFACT_DIR") {
                std::fs::create_dir_all(&directory)?;
                std::fs::write(
                    std::path::Path::new(&directory).join("probe.png"),
                    &image.bytes,
                )?;
            }
        }
        other => anyhow::bail!("screenshot failed: {other:?}"),
    }
    let pdf = page.start_print_to_pdf(Default::default())?.wait().await?;
    match page.finish_print_to_pdf(pdf)? {
        RendererPrintToPdfReply::Printed(bytes) => {
            ensure!(bytes.starts_with(b"%PDF-"), "invalid PDF");
            if let Some(directory) = std::env::var_os("MOLI_PROBE_ARTIFACT_DIR") {
                std::fs::create_dir_all(&directory)?;
                std::fs::write(std::path::Path::new(&directory).join("probe.pdf"), &bytes)?;
            }
        }
        other => anyhow::bail!("PDF failed: {other:?}"),
    }
    #[cfg(feature = "devtools")]
    {
        page.dispatch_runtime_protocol_message_async(r#"{"id":1,"method":"Runtime.enable"}"#)
            .await?;
    }
    println!(
        "probe-ok devtools={} host_eval=true promise=true wait=true environment=true page_fetch=true dedicated_worker=true shared_worker=true service_worker=true png=true pdf=true",
        cfg!(feature = "devtools")
    );
    Ok(())
}

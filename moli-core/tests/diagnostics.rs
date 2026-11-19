use std::time::Duration;

use anyhow::{Context, Result};
use moli_core::{
    DiagnosticEvent, DiagnosticSource, DiagnosticsSubscription, ExceptionOrigin,
    page::Page,
    runtime::{Browser, BrowserConfig},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinHandle,
};

#[tokio::test]
async fn browser_diagnostics_are_isolated_and_do_not_read_author_stack_properties() -> Result<()> {
    let browser = Browser::new(BrowserConfig::default())?;
    let other = Browser::new(BrowserConfig::default())?;
    let mut page = browser.fetch("about:blank").await?;
    page.evaluate_runtime_expression_async("reportError(new Error('before subscribing'))")
        .await?;
    let mut diagnostics = browser.subscribe_diagnostics();
    let mut other_diagnostics = other.subscribe_diagnostics();
    assert_empty(&mut diagnostics);

    let result = page
        .evaluate_runtime_expression_async(
            r#"(() => {
                let getterCalls = 0;
                let prepareCalls = 0;
                const error = new Error("with custom stack");
                Object.defineProperty(error, "stack", {
                    get() { getterCalls++; return "author stack"; }
                });
                Error.prepareStackTrace = () => { prepareCalls++; return "prepared by author"; };
                try {
                    reportError(error);
                    reportError(new Error("with lazy stack"));
                    const element = document.createElement("button");
                    element.setAttribute("onclick", "return )");
                    void element.onclick;
                    const frame = document.body.appendChild(document.createElement("iframe")).contentWindow;
                    frame.reportError(new frame.Error("child frame exception"));
                    return JSON.stringify([getterCalls, prepareCalls]);
                } finally {
                    delete Error.prepareStackTrace;
                }
            })()"#,
        )
        .await?;
    assert_eq!(result["value"], "[0,0]");

    for marker in [
        "with custom stack",
        "with lazy stack",
        "SyntaxError",
        "child frame exception",
    ] {
        let event = diagnostics.try_recv()?;
        assert!(
            matches!(
                event.as_ref(),
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    origin: ExceptionOrigin::UncaughtException,
                    message,
                    default_prevented: false,
                    ..
                } if *page_id == page.page_id() && message.contains(marker)
            ),
            "{event:?}"
        );
    }
    assert_empty(&mut diagnostics);
    assert_empty(&mut other_diagnostics);
    Ok(())
}

#[tokio::test]
async fn parser_script_failures_emit_one_diagnostic_and_one_dom_event_each() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser.subscribe_diagnostics();
    let mut page = browser.fetch(&server.url("/parser")).await?;
    let result = page
        .evaluate_runtime_expression_async("scriptEvents.slice().sort().join(',')")
        .await?;
    assert_eq!(result["value"], "async,blocking,defer,written");

    let mut urls = Vec::new();
    for _ in 0..4 {
        let event = diagnostics.try_recv()?;
        let DiagnosticEvent::ScriptLoadFailure {
            source: DiagnosticSource::Page { page_id },
            entry_url,
            resource_url,
            message,
        } = event.as_ref()
        else {
            panic!("expected a Page script load failure, got {event:?}");
        };
        assert_eq!(*page_id, page.page_id());
        assert!(message.contains("404"), "{message}");
        assert_eq!(entry_url, resource_url);
        urls.push(entry_url.clone());
    }
    urls.sort();
    assert_eq!(
        urls,
        ["/async.js", "/blocking.js", "/defer.js", "/written.js"].map(|path| server.url(path))
    );
    append_missing_script(&mut page, "/dynamic.js", "text/javascript").await?;
    assert_page_load_failure(
        diagnostics.try_recv()?.as_ref(),
        page.page_id(),
        &server.url("/dynamic.js"),
        &server.url("/dynamic.js"),
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn module_script_diagnostics_keep_the_failed_dependency_across_module_map_reuse() -> Result<()>
{
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser.subscribe_diagnostics();
    let mut page = browser.fetch(&server.url("/module")).await?;
    let result = page
        .evaluate_runtime_expression_async("moduleError")
        .await?;
    assert_eq!(result["value"], "root");
    assert_page_load_failure(
        diagnostics
            .try_recv()
            .context("parser-owned module failure diagnostic")?
            .as_ref(),
        page.page_id(),
        &server.url("/root.js"),
        &server.url("/missing-dependency.js"),
    );
    assert_empty(&mut diagnostics);

    append_missing_script(&mut page, "/second-root.js", "module").await?;
    assert_page_load_failure(
        diagnostics
            .try_recv()
            .context("cached module failure diagnostic")?
            .as_ref(),
        page.page_id(),
        &server.url("/second-root.js"),
        &server.url("/missing-dependency.js"),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
                window.addEventListener("error", event => resolve(event.message), { once: true });
                const script = document.createElement("script");
                script.type = "module";
                script.src = "/throws.js";
                document.head.append(script);
            })"#,
            true,
        ),
    )
    .await??;
    assert!(
        result["value"]
            .as_str()
            .is_some_and(|message| message.contains("module exception"))
    );
    let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv()).await??;
    assert!(
        matches!(
            event.as_ref(),
            DiagnosticEvent::RuntimeException { origin: ExceptionOrigin::UncaughtException, message, .. }
                if message.contains("module exception")
        ),
        "{event:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn page_exception_diagnostics_distinguish_rejection_and_default_cancellation() -> Result<()> {
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser.subscribe_diagnostics();
    let mut page = browser.fetch("about:blank").await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
                window.addEventListener("unhandledrejection", event => {
                    event.preventDefault();
                    resolve("prevented");
                }, { once: true });
                const script = document.createElement("script");
                script.textContent = 'throw new Error("same message")';
                document.head.append(script);
                Promise.reject(new Error("same message"));
            })"#,
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], "prevented", "{result}");

    for (expected_origin, prevented) in [
        (ExceptionOrigin::UncaughtException, false),
        (ExceptionOrigin::UnhandledRejection, true),
    ] {
        let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv()).await??;
        assert!(
            matches!(
                event.as_ref(),
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    origin,
                    message,
                    default_prevented,
                    ..
                } if *page_id == page.page_id() && *origin == expected_origin
                    && *default_prevented == prevented && message.contains("same message")
            ),
            "{event:?}",
        );
    }
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn shared_worker_diagnostics_identify_the_instance_across_clients() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser.subscribe_diagnostics();
    let mut instance = None;

    let mut first = browser.fetch(&server.url("/page")).await?;
    let mut second = browser.fetch(&server.url("/page")).await?;
    for page in [&mut first, &mut second] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                r#"new Promise((resolve, reject) => {
                    globalThis.worker = new SharedWorker("/shared.js", "diagnostics");
                    worker.onerror = () => reject(Error("could not start SharedWorker"));
                    worker.port.onmessage = event => resolve(event.data);
                })"#,
                true,
            ),
        )
        .await??;
        assert_eq!(result["value"], "ready", "{result}");
    }

    for (page, marker, expected_origin, prevented) in [
        (
            &mut first,
            "first",
            ExceptionOrigin::UncaughtException,
            false,
        ),
        (
            &mut second,
            "second",
            ExceptionOrigin::UnhandledRejection,
            true,
        ),
    ] {
        page.evaluate_runtime_expression_async(&format!("worker.port.postMessage({marker:?})"))
            .await?;
        let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv()).await??;
        let DiagnosticEvent::RuntimeException {
            source: DiagnosticSource::SharedWorker { instance_id },
            origin,
            message,
            filename,
            default_prevented,
            ..
        } = event.as_ref()
        else {
            panic!("expected a SharedWorker exception, got {event:?}");
        };
        assert!(message.contains(marker), "{message}");
        assert_eq!(filename, &server.url("/shared.js"));
        assert_eq!(*origin, expected_origin);
        assert_eq!(*default_prevented, prevented);
        assert_eq!(*instance.get_or_insert(*instance_id), *instance_id);
    }
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        first.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
                globalThis.missingWorker = new SharedWorker("/missing.js");
                missingWorker.onerror = () => resolve("error");
            })"#,
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], "error", "{result}");
    let failure = diagnostics.try_recv()?;
    assert!(
        matches!(
            failure.as_ref(),
            DiagnosticEvent::ScriptLoadFailure {
                source: DiagnosticSource::SharedWorker { instance_id },
                entry_url,
                resource_url,
                message,
            } if Some(*instance_id) != instance && entry_url == &server.url("/missing.js")
                && entry_url == resource_url && message.contains("404")
        ),
        "{failure:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

async fn append_missing_script(page: &mut Page, path: &str, script_type: &str) -> Result<()> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            &format!(
                r#"new Promise(resolve => {{
                const script = document.createElement("script");
                script.type = {script_type:?};
                script.src = {path:?};
                script.onerror = () => resolve("error");
                script.onload = () => resolve("load");
                document.head.append(script);
            }})"#
            ),
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], "error", "{result}");
    Ok(())
}

fn assert_page_load_failure(
    event: &DiagnosticEvent,
    expected_page: u64,
    expected_entry: &str,
    expected_resource: &str,
) {
    assert!(
        matches!(
            event,
            DiagnosticEvent::ScriptLoadFailure {
                source: DiagnosticSource::Page { page_id },
                entry_url,
                resource_url,
                message,
            } if *page_id == expected_page && entry_url == expected_entry
                && resource_url == expected_resource && message.contains("404")
        ),
        "{event:?}",
    );
}

fn assert_empty(diagnostics: &mut DiagnosticsSubscription) {
    assert_eq!(
        diagnostics.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty),
    );
}

struct DiagnosticFixtureServer {
    origin: String,
    task: JoinHandle<()>,
}

impl DiagnosticFixtureServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let (status, content_type, body) = fixture_response(line.split_whitespace().nth(1));
                loop {
                    line.clear();
                    if stream.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
                stream.get_mut().shutdown().await.unwrap();
            }
        });
        Ok(Self { origin, task })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}

impl Drop for DiagnosticFixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn fixture_response(path: Option<&str>) -> (&'static str, &'static str, &'static str) {
    match path {
        Some("/parser") => (
            "200 OK",
            "text/html",
            r#"<!doctype html>
            <script>globalThis.scriptEvents = [];</script>
            <script src="/blocking.js" onerror="scriptEvents.push('blocking')"></script>
            <script defer src="/defer.js" onerror="scriptEvents.push('defer')"></script>
            <script async src="/async.js" onerror="scriptEvents.push('async')"></script>
            <script>
                document.write(`<script src="/written.js" onerror="scriptEvents.push('written')"><\/script>`);
            </script>"#,
        ),
        Some("/module") => (
            "200 OK",
            "text/html",
            r#"<!doctype html>
            <script>globalThis.moduleError = "";</script>
            <script type="module" src="/root.js" onerror="moduleError = 'root'"></script>"#,
        ),
        Some("/root.js" | "/second-root.js") => (
            "200 OK",
            "text/javascript",
            "import './missing-dependency.js';",
        ),
        Some("/throws.js") => (
            "200 OK",
            "text/javascript",
            "throw new Error('module exception');",
        ),
        Some("/page") => ("200 OK", "text/html", "<!doctype html>"),
        Some("/shared.js") => (
            "200 OK",
            "text/javascript",
            "addEventListener('unhandledrejection', event => event.preventDefault());
            onconnect = event => {
                const port = event.ports[0];
                port.onmessage = event => {
                    const error = new Error(event.data);
                    if (event.data === 'second') Promise.reject(error);
                    else throw error;
                };
                port.postMessage('ready');
            };",
        ),
        _ => ("404 Not Found", "text/plain", "fixture script not found"),
    }
}

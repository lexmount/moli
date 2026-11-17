use std::time::Duration;

use anyhow::Result;
use moli_core::{
    DiagnosticEvent, DiagnosticSource, DiagnosticsSubscription,
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
                    return JSON.stringify([getterCalls, prepareCalls]);
                } finally {
                    delete Error.prepareStackTrace;
                }
            })()"#,
        )
        .await?;
    assert_eq!(result["value"], "[0,0]");

    for marker in ["with custom stack", "with lazy stack", "SyntaxError"] {
        let event = diagnostics.try_recv()?;
        assert!(
            matches!(
                event.as_ref(),
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    message,
                    handled: false,
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
    assert_eq!(result["value"], "blocking,defer,written");

    let mut urls = Vec::new();
    for _ in 0..3 {
        let event = diagnostics.try_recv()?;
        let DiagnosticEvent::ScriptLoadFailure {
            source: DiagnosticSource::Page { page_id },
            url,
            message,
        } = event.as_ref()
        else {
            panic!("expected a Page script load failure, got {event:?}");
        };
        assert_eq!(*page_id, page.page_id());
        assert!(message.contains("404"), "{message}");
        urls.push(url.clone());
    }
    urls.sort();
    assert_eq!(
        urls,
        ["/blocking.js", "/defer.js", "/written.js"].map(|path| server.url(path))
    );
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

    for (page, marker) in [(&mut first, "first"), (&mut second, "second")] {
        page.evaluate_runtime_expression_async(&format!("worker.port.postMessage({marker:?})"))
            .await?;
        let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv()).await??;
        let DiagnosticEvent::RuntimeException {
            source: DiagnosticSource::SharedWorker { instance_id },
            message,
            filename,
            ..
        } = event.as_ref()
        else {
            panic!("expected a SharedWorker exception, got {event:?}");
        };
        assert!(message.contains(marker), "{message}");
        assert_eq!(filename, &server.url("/shared.js"));
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
                url,
                message,
            } if Some(*instance_id) != instance && url == &server.url("/missing.js") && message.contains("404")
        ),
        "{failure:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
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
            <script>
                document.write(`<script src="/written.js" onerror="scriptEvents.push('written')"><\/script>`);
            </script>"#,
        ),
        Some("/page") => ("200 OK", "text/html", "<!doctype html>"),
        Some("/shared.js") => (
            "200 OK",
            "text/javascript",
            "onconnect = event => {
                const port = event.ports[0];
                port.onmessage = event => { throw new Error(event.data); };
                port.postMessage('ready');
            };",
        ),
        _ => ("404 Not Found", "text/plain", "fixture script not found"),
    }
}

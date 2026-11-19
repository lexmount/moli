use std::time::Duration;

use anyhow::{Context, Result};
use moli_core::{
    DiagnosticEvent, DiagnosticSource, DiagnosticsSubscription, ErrorReportOutcome,
    ExceptionOrigin,
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
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut other_diagnostics = other
        .subscribe_diagnostics()
        .context("first subscription")?;
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
                &event,
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    origin: ExceptionOrigin::UncaughtException,
                    message,
                    outcome: ErrorReportOutcome::Uncanceled,
                    ..
                } if *page_id == page.page_id() && message.contains(marker)
            ),
            "{event:?}"
        );
    }
    assert_empty(&mut diagnostics);
    assert_empty(&mut other_diagnostics);
    diagnostics.close();
    assert!(browser.subscribe_diagnostics().is_none());
    drop(other_diagnostics);
    assert!(other.subscribe_diagnostics().is_none());
    Ok(())
}

#[tokio::test]
async fn parser_script_failures_emit_one_diagnostic_and_one_dom_event_each() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
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
        } = &event
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
        &diagnostics.try_recv()?,
        page.page_id(),
        &server.url("/dynamic.js"),
        &server.url("/dynamic.js"),
        "404",
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn module_script_diagnostics_keep_the_failed_dependency_across_module_map_reuse() -> Result<()>
{
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch(&server.url("/module")).await?;
    let result = page
        .evaluate_runtime_expression_async("moduleError")
        .await?;
    assert_eq!(result["value"], "root");
    assert_page_load_failure(
        &diagnostics
            .try_recv()
            .context("parser-owned module failure diagnostic")?,
        page.page_id(),
        &server.url("/root.js"),
        &server.url("/missing-dependency.js"),
        "404",
    );
    assert_empty(&mut diagnostics);

    append_missing_script(&mut page, "/second-root.js", "module").await?;
    assert_page_load_failure(
        &diagnostics
            .try_recv()
            .context("cached module failure diagnostic")?,
        page.page_id(),
        &server.url("/second-root.js"),
        &server.url("/missing-dependency.js"),
        "404",
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
    let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv())
        .await?
        .context("diagnostics subscription closed")?;
    assert!(
        matches!(
            &event,
            DiagnosticEvent::RuntimeException { origin: ExceptionOrigin::UncaughtException, message, .. }
                if message.contains("module exception")
        ),
        "{event:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

const CSP_BLOCKED_MODULE_URL: &str = "http://127.0.0.1:1/blocked-dependency.js";

#[tokio::test]
async fn module_csp_diagnostics_report_blocked_dependency_even_when_cached() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    for subscribe_before_fetch in [true, false] {
        let browser = Browser::new(BrowserConfig::default())?;
        let diagnostics = subscribe_before_fetch
            .then(|| {
                browser
                    .subscribe_diagnostics()
                    .context("early subscription")
            })
            .transpose()?;
        let mut page = browser.fetch(&server.url("/module-csp")).await?;
        let result = page
            .evaluate_runtime_expression_async("moduleError")
            .await?;
        assert_eq!(result["value"], "blocked");

        let mut diagnostics = if let Some(mut diagnostics) = diagnostics {
            assert_page_load_failure(
                &diagnostics
                    .try_recv()
                    .context("CSP-blocked module diagnostic")?,
                page.page_id(),
                &server.url("/csp-root.js"),
                CSP_BLOCKED_MODULE_URL,
                "Content Security Policy",
            );
            assert_empty(&mut diagnostics);
            diagnostics
        } else {
            browser
                .subscribe_diagnostics()
                .context("late subscription")?
        };
        append_missing_script(&mut page, "/csp-second-root.js", "module").await?;
        assert_page_load_failure(
            &diagnostics
                .try_recv()
                .context("cached CSP failure diagnostic")?,
            page.page_id(),
            &server.url("/csp-second-root.js"),
            CSP_BLOCKED_MODULE_URL,
            "Content Security Policy",
        );
        assert_empty(&mut diagnostics);
    }
    Ok(())
}

#[tokio::test]
async fn page_exception_diagnostics_distinguish_rejection_and_default_cancellation() -> Result<()> {
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
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

    for (expected_origin, expected_outcome) in [
        (
            ExceptionOrigin::UncaughtException,
            ErrorReportOutcome::Uncanceled,
        ),
        (
            ExceptionOrigin::UnhandledRejection,
            ErrorReportOutcome::Canceled,
        ),
    ] {
        let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv())
            .await?
            .context("diagnostics subscription closed")?;
        assert!(
            matches!(
                &event,
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    origin,
                    message,
                    outcome,
                    ..
                } if *page_id == page.page_id() && *origin == expected_origin
                    && *outcome == expected_outcome && message.contains("same message")
            ),
            "{event:?}",
        );
    }
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn iframe_callback_exceptions_report_after_dom_error_dispatch() -> Result<()> {
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch("about:blank").await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
                const frame = document.body.appendChild(document.createElement("iframe")).contentWindow;
                const errors = [];
                frame.addEventListener("error", event => {
                    if (event.message.includes("frame listener")) {
                        event.preventDefault();
                        errors.push("listener");
                    } else if (event.message.includes("frame timer")) {
                        errors.push("timer");
                        resolve(errors.join(","));
                    } else {
                        errors.push(event.message);
                    }
                });
                frame.eval(`
                    addEventListener("probe", () => { throw new Error("frame listener"); });
                    dispatchEvent(new Event("probe"));
                    dispatchEvent(new ErrorEvent("error", {message: "synthetic error"}));
                    setTimeout(() => { throw new Error("frame timer"); }, 0);
                `);
            })"#,
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], "listener,synthetic error,timer");

    for (marker, expected_outcome) in [
        ("frame listener", ErrorReportOutcome::Canceled),
        ("frame timer", ErrorReportOutcome::Uncanceled),
    ] {
        let event = diagnostics
            .try_recv()
            .with_context(|| format!("{marker} diagnostic"))?;
        assert!(
            matches!(
                &event,
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::Page { page_id },
                    origin: ExceptionOrigin::UncaughtException,
                    message,
                    outcome,
                    ..
                } if *page_id == page.page_id() && *outcome == expected_outcome
                    && message.contains(marker)
            ),
            "{event:?}"
        );
    }
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn dedicated_worker_diagnostics_reflect_cancellation_across_owners() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch(&server.url("/page")).await?;
    for (mode, expected_outcome, expected_trace) in [
        ("source", ErrorReportOutcome::Canceled, "continued"),
        ("wrapper", ErrorReportOutcome::Canceled, "worker,continued"),
        (
            "window",
            ErrorReportOutcome::Canceled,
            "worker,window,continued",
        ),
        (
            "uncanceled",
            ErrorReportOutcome::Uncanceled,
            "worker,window,continued",
        ),
    ] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                &format!(
                    r#"new Promise(resolve => {{
                        const mode = {mode:?};
                        const events = [];
                        const worker = new Worker("/dedicated.js");
                        function finish() {{
                            worker.terminate();
                            setTimeout(() => {{
                                window.onerror = null;
                                events.push("continued");
                                resolve(events.join(","));
                            }}, 0);
                        }}
                        worker.onerror = event => {{
                            events.push("worker");
                            if (mode === "wrapper") {{
                                event.preventDefault();
                                finish();
                            }}
                        }};
                        window.onerror = () => {{
                            events.push("window");
                            finish();
                            return mode === "window";
                        }};
                        worker.onmessage = finish;
                        worker.postMessage(mode);
                    }})"#
                ),
                true,
            ),
        )
        .await??;
        assert_eq!(result["value"], expected_trace, "{mode}: {result}");

        let diagnostic = diagnostics.try_recv()?;
        assert!(
            matches!(
                &diagnostic,
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::DedicatedWorker { .. },
                    origin: ExceptionOrigin::UncaughtException,
                    message,
                    outcome,
                    ..
                } if message.contains("dedicated worker failure") && *outcome == expected_outcome
            ),
            "{mode}: {diagnostic:?}"
        );
        assert_empty(&mut diagnostics);
    }
    Ok(())
}

#[tokio::test]
async fn dedicated_worker_report_respects_window_retirement_during_error_dispatch() -> Result<()> {
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch("about:blank").await?;
    for (instance_id, cancel, expected_outcome) in [
        (1, false, ErrorReportOutcome::Interrupted),
        (2, true, ErrorReportOutcome::Canceled),
    ] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                &format!(
                    r#"new Promise(resolve => {{
                        const frame = document.body.appendChild(document.createElement("iframe"));
                        const events = [];
                        const record = kind => {{
                            events.push(kind);
                            if (kind === "worker") {{
                                setTimeout(() => resolve(events.join(",") + "|" + frame.isConnected), 0);
                            }}
                        }};
                        frame.contentWindow.eval(`record => {{
                            window.onerror = () => {{
                                record("window");
                                return true;
                            }};
                            const worker = new Worker("data:text/javascript," +
                                encodeURIComponent('throw new Error("retiring iframe worker")'));
                            worker.onerror = event => {{
                                if ({cancel}) event.preventDefault();
                                record("worker");
                                frameElement.remove();
                            }};
                        }}`)(record);
                    }})"#
                ),
                true,
            ),
        )
        .await??;
        assert_eq!(result["value"], "worker|false", "{result}");

        let diagnostic = diagnostics
            .try_recv()
            .context("retired Window worker diagnostic")?;
        assert!(
            matches!(
                &diagnostic,
                DiagnosticEvent::RuntimeException {
                    source: DiagnosticSource::DedicatedWorker { instance_id: source_id },
                    origin: ExceptionOrigin::UncaughtException,
                    message,
                    outcome,
                    ..
                } if *source_id == instance_id && message.contains("retiring iframe worker")
                    && *outcome == expected_outcome
            ),
            "expected {expected_outcome:?} for Worker {instance_id}, got {diagnostic:?}"
        );
        assert_empty(&mut diagnostics);
    }
    Ok(())
}

#[tokio::test]
async fn nested_worker_report_reaches_window_after_its_intermediate_owner_terminates() -> Result<()>
{
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch(&server.url("/page")).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
                const channel = new BroadcastChannel("nested-report");
                const worker = new Worker("/nested-report-owner.js", {type: "module"});
                channel.onmessage = () => {
                    worker.terminate();
                    channel.close();
                };
                window.onerror = (message, filename, lineno, colno) => {
                    setTimeout(() => resolve(JSON.stringify({message, filename, lineno, colno})), 0);
                    return true;
                };
            })"#,
            true,
        ),
    )
    .await??;
    let filename = server.url("/nested-report-child.js");
    let error_event: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("nested Worker Window error event")?,
    )?;
    assert_eq!(error_event["filename"], filename, "{result}");
    assert!(
        error_event["message"]
            .as_str()
            .is_some_and(|message| message.contains("nested report")),
        "{result}"
    );

    let diagnostic = diagnostics.try_recv()?;
    assert!(
        matches!(
            &diagnostic,
            DiagnosticEvent::RuntimeException {
                source: DiagnosticSource::DedicatedWorker { instance_id: 2 },
                message,
                filename: source_url,
                lineno,
                colno,
                stack: Some(stack),
                outcome: ErrorReportOutcome::Canceled,
                ..
            } if error_event["message"] == *message && source_url == &filename
                && *lineno > 0 && error_event["lineno"] == *lineno
                && *colno > 0 && error_event["colno"] == *colno
                && stack.contains("at throwNestedReport")
        ),
        "{diagnostic:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn module_worker_diagnostics_distinguish_fetch_failure_from_exception() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch(&server.url("/page")).await?;
    for path in ["/worker-module.js", "/throws.js"] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                &format!(
                    r#"new Promise(resolve => {{
                        const worker = new Worker({path:?}, {{type: "module"}});
                        worker.onerror = event => {{
                            event.preventDefault();
                            worker.terminate();
                            resolve(event.type);
                        }};
                    }})"#
                ),
                true,
            ),
        )
        .await??;
        assert_eq!(result["value"], "error");

        let event = diagnostics.try_recv().context("module worker diagnostic")?;
        match &event {
            DiagnosticEvent::ScriptLoadFailure {
                source: DiagnosticSource::DedicatedWorker { .. },
                entry_url,
                resource_url,
                message,
            } if path == "/worker-module.js" => {
                assert_eq!(entry_url, &server.url(path));
                assert_eq!(resource_url, &server.url("/missing-dependency.js"));
                assert!(message.contains("404"), "{message}");
            }
            DiagnosticEvent::RuntimeException {
                source: DiagnosticSource::DedicatedWorker { .. },
                origin: ExceptionOrigin::UncaughtException,
                message,
                ..
            } if path == "/throws.js" => assert!(message.contains("module exception"), "{message}"),
            _ => panic!("unexpected diagnostic for {path}: {event:?}"),
        }
        assert_empty(&mut diagnostics);
    }
    Ok(())
}

#[tokio::test]
async fn service_worker_entry_load_failure_reaches_diagnostics() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
    let mut page = browser.fetch(&server.url("/page")).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"navigator.serviceWorker.register("/missing-sw.js").then(
                () => "registered",
                error => error.message
            )"#,
            true,
        ),
    )
    .await??;
    assert!(
        result["value"]
            .as_str()
            .is_some_and(|message| message.contains("404")),
        "{result}"
    );

    let expected_url = server.url("/missing-sw.js");
    let event = diagnostics
        .try_recv()
        .context("service worker load diagnostic")?;
    assert!(
        matches!(
            &event,
            DiagnosticEvent::ScriptLoadFailure {
                source: DiagnosticSource::ServiceWorker { version_id },
                entry_url,
                resource_url,
                message,
            } if *version_id != 0 && entry_url == &expected_url
                && resource_url == &expected_url && message.contains("404")
        ),
        "{event:?}"
    );
    assert_empty(&mut diagnostics);
    Ok(())
}

#[tokio::test]
async fn shared_worker_diagnostics_identify_the_instance_across_clients() -> Result<()> {
    let server = DiagnosticFixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut diagnostics = browser
        .subscribe_diagnostics()
        .context("first subscription")?;
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

    for (page, marker, expected_origin, expected_outcome) in [
        (
            &mut first,
            "first",
            ExceptionOrigin::UncaughtException,
            ErrorReportOutcome::Uncanceled,
        ),
        (
            &mut second,
            "second",
            ExceptionOrigin::UnhandledRejection,
            ErrorReportOutcome::Canceled,
        ),
    ] {
        page.evaluate_runtime_expression_async(&format!("worker.port.postMessage({marker:?})"))
            .await?;
        let event = tokio::time::timeout(Duration::from_secs(15), diagnostics.recv())
            .await?
            .context("diagnostics subscription closed")?;
        let DiagnosticEvent::RuntimeException {
            source: DiagnosticSource::SharedWorker { instance_id },
            origin,
            message,
            filename,
            outcome,
            ..
        } = &event
        else {
            panic!("expected a SharedWorker exception, got {event:?}");
        };
        assert!(message.contains(marker), "{message}");
        assert_eq!(filename, &server.url("/shared.js"));
        assert_eq!(*origin, expected_origin);
        assert_eq!(*outcome, expected_outcome);
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
            &failure,
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
    expected_message: &str,
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
                && resource_url == expected_resource && message.contains(expected_message)
        ),
        "{event:?}",
    );
}

fn assert_empty(diagnostics: &mut DiagnosticsSubscription) {
    assert_eq!(
        diagnostics.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty),
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
        Some("/module-csp") => (
            "200 OK",
            "text/html",
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src 'self'; script-src-attr 'unsafe-inline'">
            <script type="module" src="/csp-root.js" onerror="globalThis.moduleError = 'blocked'"></script>"#,
        ),
        Some("/csp-root.js" | "/csp-second-root.js") => (
            "200 OK",
            "text/javascript",
            concat!("import '", "http://127.0.0.1:1/blocked-dependency.js", "';"),
        ),
        Some("/throws.js") => (
            "200 OK",
            "text/javascript",
            "throw new Error('module exception');",
        ),
        Some("/page") => ("200 OK", "text/html", "<!doctype html>"),
        Some("/worker-module.js") => (
            "200 OK",
            "text/javascript",
            "import './missing-dependency.js';",
        ),
        Some("/dedicated.js") => (
            "200 OK",
            "text/javascript",
            r#"onmessage = ({data}) => {
                if (data === "source") {
                    onerror = () => {
                        setTimeout(() => postMessage("continued"), 0);
                        return true;
                    };
                }
                throw new Error("dedicated worker failure");
            };"#,
        ),
        Some("/nested-report-owner.js") => (
            "200 OK",
            "text/javascript",
            "new Worker('/nested-report-child.js'); await new Promise(() => {});",
        ),
        Some("/nested-report-child.js") => (
            "200 OK",
            "text/javascript",
            r#"const channel = new BroadcastChannel("nested-report");
               setTimeout(function throwNestedReport() { throw new Error("nested report"); }, 0);
               setTimeout(() => channel.postMessage("reported"), 0);"#,
        ),
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

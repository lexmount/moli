use super::{Output, clean_output, run_fetch_cli_with_args};
use anyhow::Result;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const STYLESHEET_PATHS: [&str; 7] = [
    "/main.css",
    "/noscript.css",
    "/imported.css",
    "/child.css",
    "/grandchild.css",
    "/srcdoc.css",
    "/srcdoc-noscript.css",
];
const SCRIPT_PATHS: [&str; 16] = [
    "/initial.js",
    "/preload.js",
    "/module-preload.js",
    "/classic.js",
    "/defer.js",
    "/async.js",
    "/module.js",
    "/child-preload.js",
    "/child.js",
    "/grandchild-preload.js",
    "/grandchild.js",
    "/srcdoc-preload.js",
    "/srcdoc.js",
    "/dynamic.js",
    "/child-dynamic.js",
    "/document-write.js",
];

const DISABLED_READY_SCRIPT: &str = r#"(() => {
    const fallback = document.getElementById('fallback');
    const styled = document.getElementById('styled');
    const child = document.getElementById('child')?.contentDocument;
    const childFallback = child?.getElementById('child-fallback');
    const grandchild = child?.getElementById('grandchild')?.contentDocument;
    const grandchildFallback = grandchild?.getElementById('grandchild-fallback');
    const srcdoc = document.getElementById('srcdoc-child')?.contentDocument;
    const srcdocFallback = srcdoc?.getElementById('srcdoc-fallback');
    const ready = Boolean(
        fallback &&
        styled &&
        childFallback &&
        grandchildFallback &&
        srcdocFallback &&
        getComputedStyle(fallback).color === 'rgb(1, 2, 3)' &&
        getComputedStyle(fallback).backgroundColor === 'rgb(4, 5, 6)' &&
        getComputedStyle(styled).color === 'rgb(7, 8, 9)' &&
        child.defaultView.getComputedStyle(childFallback).color === 'rgb(10, 11, 12)' &&
        grandchild.defaultView.getComputedStyle(grandchildFallback).color === 'rgb(13, 14, 15)' &&
        srcdoc.defaultView.getComputedStyle(srcdocFallback).color === 'rgb(16, 17, 18)' &&
        srcdoc.defaultView.getComputedStyle(srcdocFallback).backgroundColor === 'rgb(19, 20, 21)' &&
        !document.documentElement.hasAttribute('data-inline-script') &&
        !document.documentElement.hasAttribute('data-external-script') &&
        !child.documentElement.hasAttribute('data-child-inline-script') &&
        !child.documentElement.hasAttribute('data-external-script') &&
        !grandchild.documentElement.hasAttribute('data-grandchild-inline-script') &&
        !grandchild.documentElement.hasAttribute('data-external-script') &&
        !srcdoc.documentElement.hasAttribute('data-srcdoc-inline-script') &&
        !srcdoc.documentElement.hasAttribute('data-external-script')
    );
    if (ready) {
        document.documentElement.setAttribute('data-disable-js-probe', 'ready');
    }
    return ready;
})()"#;

const ENABLED_READY_SCRIPT: &str = r#"(() => {
    const child = document.getElementById('child')?.contentDocument;
    const grandchild = child?.getElementById('grandchild')?.contentDocument;
    const srcdoc = document.getElementById('srcdoc-child')?.contentDocument;
    const mainScripts = document.documentElement.getAttribute('data-external-script') || '';
    const childScripts = child?.documentElement.getAttribute('data-external-script') || '';
    const grandchildScripts = grandchild?.documentElement.getAttribute('data-external-script') || '';
    const srcdocScripts = srcdoc?.documentElement.getAttribute('data-external-script') || '';
    const ready = Boolean(
        document.documentElement.getAttribute('data-inline-script') === 'ran' &&
        child?.documentElement.getAttribute('data-child-inline-script') === 'ran' &&
        grandchild?.documentElement.getAttribute('data-grandchild-inline-script') === 'ran' &&
        srcdoc?.documentElement.getAttribute('data-srcdoc-inline-script') === 'ran' &&
        mainScripts.includes('/classic.js,') &&
        mainScripts.includes('/defer.js,') &&
        mainScripts.includes('/async.js,') &&
        mainScripts.includes('/module.js,') &&
        childScripts.includes('/child.js,') &&
        grandchildScripts.includes('/grandchild.js,') &&
        srcdocScripts.includes('/srcdoc.js,') &&
        !document.getElementById('fallback') &&
        !child?.getElementById('child-fallback') &&
        !grandchild?.getElementById('grandchild-fallback') &&
        !srcdoc?.getElementById('srcdoc-fallback')
    );
    if (ready) {
        document.documentElement.setAttribute('data-default-js-probe', 'ready');
    }
    return ready;
})()"#;

const MAIN_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <link rel="stylesheet" href="/main.css">
    <noscript><link rel="stylesheet" href="/noscript.css"></noscript>
    <link rel="preload" as="script" href="/preload.js">
    <link rel="modulepreload" href="/module-preload.js">
    <script src="/classic.js"></script>
    <script defer src="/defer.js"></script>
    <script async src="/async.js"></script>
    <script type="importmap">{"imports":{"disabled-alias":"/module.js"}}</script>
    <script type="module" src="/module.js"></script>
    <script>document.documentElement.setAttribute("data-inline-script", "ran");</script>
  </head>
  <body>
    <noscript><main id="fallback">main fallback</main></noscript>
    <div id="styled">imported stylesheet target</div>
    <iframe id="child" src="/child.html"></iframe>
    <iframe id="srcdoc-child" srcdoc="<!doctype html><html><head><link rel='stylesheet' href='/srcdoc.css'><noscript><link rel='stylesheet' href='/srcdoc-noscript.css'></noscript><link rel='modulepreload' href='/srcdoc-preload.js'><script src='/srcdoc.js'></script><script>document.documentElement.setAttribute('data-srcdoc-inline-script', 'ran');</script></head><body><noscript><main id='srcdoc-fallback'>srcdoc fallback</main></noscript></body></html>"></iframe>
  </body>
</html>"#;

const CHILD_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <link rel="stylesheet" href="/child.css">
    <link rel="modulepreload" href="/child-preload.js">
    <script src="/child.js"></script>
    <script>document.documentElement.setAttribute("data-child-inline-script", "ran");</script>
  </head>
  <body>
    <noscript><main id="child-fallback">child fallback</main></noscript>
    <iframe id="grandchild" src="/grandchild.html"></iframe>
  </body>
</html>"#;

const GRANDCHILD_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <link rel="stylesheet" href="/grandchild.css">
    <link rel="preload" as="script" href="/grandchild-preload.js">
    <script type="module" src="/grandchild.js"></script>
    <script>document.documentElement.setAttribute("data-grandchild-inline-script", "ran");</script>
  </head>
  <body><noscript><main id="grandchild-fallback">grandchild fallback</main></noscript></body>
</html>"#;

const JAVASCRIPT_URL_PAGE: &str = r#"<!doctype html>
<iframe
  id="javascript-url-child"
  src="javascript:document.documentElement.setAttribute('data-javascript-url-ran', 'yes')"
></iframe>"#;

struct DisableJsFixtureServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl DisableJsFixtureServer {
    async fn spawn() -> Result<Self> {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/page.html", get(|| async { Html(MAIN_PAGE) }))
            .route("/child.html", get(|| async { Html(CHILD_PAGE) }))
            .route("/grandchild.html", get(|| async { Html(GRANDCHILD_PAGE) }))
            .route(
                "/javascript-url.html",
                get(|| async { Html(JAVASCRIPT_URL_PAGE) }),
            )
            .route("/redirect.html", get(redirect_to_page))
            .route("/meta-refresh.html", get(meta_refresh_page))
            .route("/document-write.html", get(document_write_page))
            .fallback(resource)
            .with_state(Arc::clone(&requests));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("disable-js fixture server should serve");
        });
        Ok(Self {
            base_url: format!("http://{addr}"),
            requests,
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn request_count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .iter()
            .filter(|request| request.as_str() == path)
            .count()
    }
}

impl Drop for DisableJsFixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn redirect_to_page() -> Response {
    (
        StatusCode::FOUND,
        [("location", "/page.html"), ("cache-control", "no-store")],
        "",
    )
        .into_response()
}

async fn meta_refresh_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<meta http-equiv="refresh" content="0; url=/page.html">
<script src="/initial.js"></script>
<script>document.documentElement.setAttribute("data-initial-inline-script", "ran");</script>
<main id="meta-refresh-source">meta refresh source</main>"#,
    )
}

async fn document_write_page() -> Html<&'static str> {
    Html(r#"<!doctype html><main id="document-write-source">rewrite source</main>"#)
}

async fn resource(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
    let path = uri.path().to_owned();
    requests.lock().push(path.clone());
    if path.ends_with(".js") {
        let body = format!(
            "(() => {{ const old = document.documentElement.getAttribute('data-external-script') || ''; document.documentElement.setAttribute('data-external-script', old + '{path},'); }})();"
        );
        return (
            StatusCode::OK,
            [
                ("content-type", "application/javascript; charset=utf-8"),
                ("cache-control", "no-store"),
            ],
            body,
        )
            .into_response();
    }

    let Some(body) = stylesheet_body(&path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    (
        StatusCode::OK,
        [
            ("content-type", "text/css; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        body,
    )
        .into_response()
}

fn stylesheet_body(path: &str) -> Option<&'static str> {
    match path {
        "/main.css" => Some("@import url('/imported.css'); #fallback { color: rgb(1, 2, 3); }"),
        "/noscript.css" => Some("#fallback { background-color: rgb(4, 5, 6); }"),
        "/imported.css" => Some("#styled { color: rgb(7, 8, 9); }"),
        "/child.css" => Some("#child-fallback { color: rgb(10, 11, 12); }"),
        "/grandchild.css" => Some("#grandchild-fallback { color: rgb(13, 14, 15); }"),
        "/srcdoc.css" => Some("#srcdoc-fallback { color: rgb(16, 17, 18); }"),
        "/srcdoc-noscript.css" => Some("#srcdoc-fallback { background-color: rgb(19, 20, 21); }"),
        "/document-write.css" => Some("#document-write-fallback { color: rgb(41, 42, 43); }"),
        _ => None,
    }
}

fn fetch_disabled(
    server: &DisableJsFixtureServer,
    page_path: &str,
    extra_args: &[&str],
) -> Result<Output> {
    fetch_disabled_until(server, page_path, DISABLED_READY_SCRIPT, extra_args)
}

fn fetch_disabled_until(
    server: &DisableJsFixtureServer,
    page_path: &str,
    ready_script: &str,
    extra_args: &[&str],
) -> Result<Output> {
    let mut args = vec![
        "--disable-js",
        "--with-frames",
        "--timeout",
        "5000",
        "--wait-script",
        ready_script,
    ];
    args.extend_from_slice(extra_args);
    run_fetch_cli_with_args(&server.url(page_path), &args)
}

fn assert_disabled_result(
    output: &Output,
    server: &DisableJsFixtureServer,
    scenario: &str,
) -> String {
    assert!(
        output.status.success(),
        "{scenario} failed: stdout={}\nstderr={}\nrequests={:?}",
        clean_output(&output.stdout),
        clean_output(&output.stderr),
        server.requests.lock().as_slice()
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-disable-js-probe=\"ready\""),
        "{scenario} must leave automation expressions available: {stdout}"
    );
    for marker in [
        "data-inline-script=\"ran\"",
        "data-external-script=\"",
        "data-child-inline-script=\"ran\"",
        "data-grandchild-inline-script=\"ran\"",
        "data-srcdoc-inline-script=\"ran\"",
    ] {
        assert!(
            !stdout.contains(marker),
            "{scenario} unexpectedly contains executed-script marker {marker}: {stdout}"
        );
    }
    for path in STYLESHEET_PATHS {
        assert_eq!(
            server.request_count(path),
            1,
            "{scenario} must fetch required stylesheet {path} exactly once"
        );
    }
    for path in SCRIPT_PATHS {
        assert_eq!(
            server.request_count(path),
            0,
            "{scenario} unexpectedly requested script resource {path}"
        );
    }
    stdout
}

#[test]
fn keeps_css_noscript_and_nested_frames_without_fetching_scripts() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled(&server, "/page.html", &[])?;
    let stdout = assert_disabled_result(&output, &server, "direct HTTP navigation");

    for marker in [
        "<main id=\"fallback\">main fallback</main>",
        "child-fallback",
        "srcdoc-fallback",
    ] {
        assert!(
            stdout.contains(marker),
            "direct HTTP navigation must serialize fallback marker {marker}: {stdout}"
        );
    }
    assert!(
        stdout.contains("<script src=\"/classic.js\"></script>"),
        "disable-js must preserve inert script nodes in the DOM: {stdout}"
    );
    Ok(())
}

#[test]
fn survives_http_redirect() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled(&server, "/redirect.html", &[])?;
    assert_disabled_result(&output, &server, "HTTP redirect navigation");
    Ok(())
}

#[test]
fn survives_meta_refresh_document_replacement() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled(&server, "/meta-refresh.html", &[])?;
    let stdout = assert_disabled_result(&output, &server, "meta refresh document replacement");
    assert!(
        !stdout.contains("data-initial-inline-script=\"ran\""),
        "the source Document must not execute before navigation: {stdout}"
    );
    Ok(())
}

#[test]
fn blocks_dynamic_scripts_and_event_handlers() -> Result<()> {
    let ready_script = r#"(() => {
        if (!__BASE_READY__) {
            return false;
        }
        const child = document.getElementById('child').contentDocument;
        if (!globalThis.__disableJsDynamicProbeStarted) {
            globalThis.__disableJsDynamicProbeStarted = Date.now();
            const install = (targetDocument, label, src) => {
                const inline = targetDocument.createElement('script');
                inline.textContent = `document.documentElement.setAttribute('data-${label}-dynamic-inline', 'ran')`;
                targetDocument.body.appendChild(inline);

                const external = targetDocument.createElement('script');
                external.src = src;
                targetDocument.body.appendChild(external);

                const button = targetDocument.createElement('button');
                button.setAttribute(
                    'onclick',
                    `document.documentElement.setAttribute('data-${label}-event-handler', 'ran')`
                );
                targetDocument.body.appendChild(button);
                button.click();
            };
            install(document, 'main', '/dynamic.js');
            install(child, 'child', '/child-dynamic.js');
            return false;
        }
        const quiet = Date.now() - globalThis.__disableJsDynamicProbeStarted >= 150;
        const blocked = [document, child].every((targetDocument) =>
            !targetDocument.documentElement.hasAttribute('data-external-script') &&
            !targetDocument.documentElement.hasAttribute(
                targetDocument === document ? 'data-main-dynamic-inline' : 'data-child-dynamic-inline'
            ) &&
            !targetDocument.documentElement.hasAttribute(
                targetDocument === document ? 'data-main-event-handler' : 'data-child-event-handler'
            )
        );
        if (quiet && blocked) {
            document.documentElement.setAttribute('data-dynamic-disable-js-probe', 'ready');
            return true;
        }
        return false;
    })()"#
        .replace("__BASE_READY__", DISABLED_READY_SCRIPT);
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled_until(&server, "/page.html", &ready_script, &[])?;
    let stdout = assert_disabled_result(
        &output,
        &server,
        "dynamic scripts and inline event handlers",
    );
    assert!(
        stdout.contains("data-dynamic-disable-js-probe=\"ready\""),
        "dynamic page-owned code was not observed long enough: {stdout}"
    );
    Ok(())
}

#[test]
fn blocks_child_javascript_url_execution() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        const child = document.getElementById('javascript-url-child')?.contentDocument;
        if (!child?.documentElement) {
            return false;
        }
        if (!globalThis.__disableJsJavascriptUrlProbeStarted) {
            globalThis.__disableJsJavascriptUrlProbeStarted = Date.now();
            return false;
        }
        if (Date.now() - globalThis.__disableJsJavascriptUrlProbeStarted < 150) {
            return false;
        }
        document.documentElement.setAttribute('data-javascript-url-disable-js-probe', 'ready');
        return true;
    })()"#;

    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled_until(&server, "/javascript-url.html", READY_SCRIPT, &[])?;

    assert!(
        output.status.success(),
        "script-disabled javascript URL fixture failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-javascript-url-disable-js-probe=\"ready\""),
        "automation probe did not observe the child long enough: {stdout}"
    );
    assert!(
        !stdout.contains("data-javascript-url-ran=&quot;yes&quot;"),
        "a child javascript: URL executed despite --disable-js: {stdout}"
    );
    Ok(())
}

#[test]
fn child_javascript_url_fixture_executes_by_default() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        const child = document.getElementById('javascript-url-child')?.contentDocument;
        return child?.documentElement?.getAttribute('data-javascript-url-ran') === 'yes';
    })()"#;

    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/javascript-url.html"),
        &[
            "--with-frames",
            "--timeout",
            "5000",
            "--wait-script",
            READY_SCRIPT,
        ],
    )?;

    assert!(
        output.status.success(),
        "default javascript URL control failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-javascript-url-ran=&quot;yes&quot;"),
        "default control did not execute the child javascript: URL: {stdout}"
    );
    Ok(())
}

#[test]
fn fragment_parsers_use_the_disabled_document_policy() -> Result<()> {
    let ready_script = r#"(() => {
        if (!__BASE_READY__) {
            return false;
        }
        if (!globalThis.__disableJsFragmentProbeStarted) {
            globalThis.__disableJsFragmentProbeStarted = true;
            const child = document.getElementById('child').contentDocument;

            const inner = document.createElement('div');
            inner.innerHTML = '<noscript><span id="main-inner-fallback"></span></noscript>';
            document.body.appendChild(inner);

            const unsafe = child.createElement('div');
            unsafe.setHTMLUnsafe(
                '<noscript><span id="child-unsafe-fallback"></span></noscript>'
            );
            child.body.appendChild(unsafe);
        }

        const child = document.getElementById('child').contentDocument;
        const ready = Boolean(
            document.getElementById('main-inner-fallback') &&
            child.getElementById('child-unsafe-fallback')
        );
        if (ready) {
            document.documentElement.setAttribute('data-fragment-disable-js-probe', 'ready');
        }
        return ready;
    })()"#
        .replace("__BASE_READY__", DISABLED_READY_SCRIPT);
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled_until(&server, "/page.html", &ready_script, &[])?;
    let stdout = assert_disabled_result(&output, &server, "document-owned fragment parsing");

    assert!(
        stdout.contains("data-fragment-disable-js-probe=\"ready\""),
        "fragment parsers did not inherit the disabled Document policy: {stdout}"
    );
    Ok(())
}

#[test]
fn applies_to_document_open_replacement_parser() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        if (window.name !== '__moli_disable_js_document_write__') {
            window.name = '__moli_disable_js_document_write__';
            document.open();
            document.write(`<!doctype html><html><head>
                <link rel="stylesheet" href="/document-write.css">
                <script src="/document-write.js"></script>
                <script>document.documentElement.setAttribute('data-document-write-inline', 'ran');</script>
                </head><body><noscript><main id="document-write-fallback">document.write fallback</main></noscript></body></html>`);
            document.close();
            return false;
        }
        const fallback = document.getElementById('document-write-fallback');
        const ready = Boolean(
            fallback &&
            getComputedStyle(fallback).color === 'rgb(41, 42, 43)' &&
            !document.documentElement.hasAttribute('data-document-write-inline') &&
            !document.documentElement.hasAttribute('data-external-script')
        );
        if (ready) {
            document.documentElement.setAttribute('data-document-write-probe', 'ready');
        }
        return ready;
    })()"#;
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled_until(&server, "/document-write.html", READY_SCRIPT, &[])?;

    assert!(
        output.status.success(),
        "script-disabled document.open replacement failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-document-write-probe=\"ready\""),
        "document.open replacement did not preserve the disable-js state: {stdout}"
    );
    assert!(
        stdout.contains("<main id=\"document-write-fallback\">document.write fallback</main>"),
        "document.open replacement must parse noscript markup: {stdout}"
    );
    assert!(!stdout.contains("data-document-write-inline=\"ran\""));
    assert!(!stdout.contains("data-external-script=\""));
    assert_eq!(server.request_count("/document-write.css"), 1);
    assert_eq!(server.request_count("/document-write.js"), 0);
    Ok(())
}

#[test]
fn composes_with_script_stripping() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = fetch_disabled(&server, "/page.html", &["--strip-mode", "js"])?;
    let stdout = assert_disabled_result(&output, &server, "script-disabled stripped output");
    assert!(
        !stdout.contains("<script src=\"/classic.js\"></script>"),
        "strip-mode=js must remove the inert script node from output: {stdout}"
    );
    Ok(())
}

#[test]
fn handles_non_streaming_data_document() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        const fallback = document.getElementById('data-fallback');
        const ready = Boolean(
            fallback &&
            getComputedStyle(fallback).color === 'rgb(31, 32, 33)' &&
            !document.documentElement.hasAttribute('data-data-inline') &&
            !document.documentElement.hasAttribute('data-data-module')
        );
        if (ready) {
            document.documentElement.setAttribute('data-data-probe', 'ready');
        }
        return ready;
    })()"#;
    let html = r#"<!doctype html>
<style>#data-fallback { color: rgb(31, 32, 33); }</style>
<script>document.documentElement.setAttribute('data-data-inline', 'ran');</script>
<script type="module">document.documentElement.setAttribute('data-data-module', 'ran');</script>
<noscript><main id="data-fallback">data fallback</main></noscript>"#;
    let encoded = url::form_urlencoded::byte_serialize(html.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    let url = format!("data:text/html,{encoded}");
    let output = run_fetch_cli_with_args(
        &url,
        &[
            "--disable-js",
            "--timeout",
            "5000",
            "--wait-script",
            READY_SCRIPT,
        ],
    )?;

    assert!(
        output.status.success(),
        "script-disabled data URL failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(stdout.contains("data-data-probe=\"ready\""), "{stdout}");
    assert!(
        stdout.contains("<main id=\"data-fallback\">data fallback</main>"),
        "data URL noscript fallback was not parsed: {stdout}"
    );
    assert!(
        stdout.contains(
            "<script>document.documentElement.setAttribute('data-data-inline', 'ran');</script>"
        ),
        "data URL script nodes must remain inert DOM content: {stdout}"
    );
    assert!(!stdout.contains("data-data-inline=\"ran\""));
    assert!(!stdout.contains("data-data-module=\"ran\""));
    Ok(())
}

#[test]
fn keeps_script_execution_enabled_by_default() -> Result<()> {
    const EXECUTED_SCRIPT_PATHS: [&str; 7] = [
        "/classic.js",
        "/defer.js",
        "/async.js",
        "/module.js",
        "/child.js",
        "/grandchild.js",
        "/srcdoc.js",
    ];
    const ACTIVE_STYLESHEET_PATHS: [&str; 5] = [
        "/main.css",
        "/imported.css",
        "/child.css",
        "/grandchild.css",
        "/srcdoc.css",
    ];

    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableJsFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/page.html"),
        &[
            "--with-frames",
            "--timeout",
            "5000",
            "--wait-script",
            ENABLED_READY_SCRIPT,
        ],
    )?;

    assert!(
        output.status.success(),
        "default script-enabled control failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-default-js-probe=\"ready\""),
        "default behavior must remain script-enabled: {stdout}"
    );
    for path in EXECUTED_SCRIPT_PATHS {
        assert_eq!(
            server.request_count(path),
            1,
            "default script-enabled control must request {path} exactly once"
        );
    }
    for path in ACTIVE_STYLESHEET_PATHS {
        assert_eq!(
            server.request_count(path),
            1,
            "default script-enabled control must request {path} exactly once"
        );
    }
    for path in ["/noscript.css", "/srcdoc-noscript.css"] {
        assert_eq!(
            server.request_count(path),
            0,
            "default script-enabled control must keep noscript stylesheet {path} inert"
        );
    }
    Ok(())
}

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

const CSS_PATHS: [&str; 12] = [
    "/external.css",
    "/external-import.css",
    "/preload.css",
    "/inline-import.css",
    "/dynamic.css",
    "/dynamic-import.css",
    "/shadow-import.css",
    "/child.css",
    "/grandchild.css",
    "/srcdoc.css",
    "/rewrite.css",
    "/rewrite-import.css",
];

const MAIN_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <link id="external-link" rel="stylesheet" href="/external.css">
    <link id="style-preload" rel="preload" as="style" href="/preload.css">
    <style id="inline-sheet">
      @import url('/inline-import.css');
      #main-target { color: rgb(1, 2, 3); display: none; margin-left: 41px; }
    </style>
    <script>
      document.documentElement.setAttribute('data-script-ran', 'yes');

      const constructed = new CSSStyleSheet();
      constructed.replaceSync('#adopted-target { color: rgb(7, 8, 9); display: none; }');
      document.adoptedStyleSheets = [constructed];

      const dynamicStyle = document.createElement('style');
      dynamicStyle.textContent = "@import url('/dynamic-import.css'); #dynamic-target { display: none; }";
      document.head.appendChild(dynamicStyle);
      const dynamicLink = document.createElement('link');
      dynamicLink.rel = 'stylesheet';
      dynamicLink.href = '/dynamic.css';
      document.head.appendChild(dynamicLink);
    </script>
  </head>
  <body>
    <div id="main-target" class="external" style="color: rgb(4, 5, 6); display: none">main</div>
    <div id="adopted-target">adopted</div>
    <div id="dynamic-target">dynamic</div>
    <div id="shadow-host"></div>
    <iframe id="child" src="/child.html"></iframe>
    <iframe id="srcdoc-child" srcdoc="<!doctype html><style>@import url('/srcdoc.css'); #srcdoc-target { display:none; color:rgb(31,32,33) }</style><div id='srcdoc-target' style='display:none'>srcdoc</div><script>document.documentElement.setAttribute('data-script-ran','yes')</script>"></iframe>
    <script>
      const root = document.getElementById('shadow-host').attachShadow({ mode: 'open' });
      root.innerHTML = `<style>@import url('/shadow-import.css'); #shadow-target { display: none; color: rgb(21, 22, 23); }</style><span id="shadow-target" style="display:none">shadow</span>`;
      const shadowSheet = new CSSStyleSheet();
      shadowSheet.replaceSync('#shadow-target { margin-left: 57px; }');
      root.adoptedStyleSheets = [shadowSheet];
    </script>
  </body>
</html>"#;

const CHILD_PAGE: &str = r#"<!doctype html>
<link rel="stylesheet" href="/child.css">
<style>#child-target { display:none; color:rgb(11,12,13) }</style>
<div id="child-target" style="display:none">child</div>
<iframe id="grandchild" src="/grandchild.html"></iframe>
<script>document.documentElement.setAttribute('data-script-ran', 'yes')</script>"#;

const GRANDCHILD_PAGE: &str = r#"<!doctype html>
<link rel="stylesheet" href="/grandchild.css">
<div id="grandchild-target" style="display:none; color:rgb(14,15,16)">grandchild</div>
<script>document.documentElement.setAttribute('data-script-ran', 'yes')</script>"#;

const DISABLED_READY_SCRIPT: &str = r#"(() => {
    const child = document.getElementById('child')?.contentDocument;
    const grandchild = child?.getElementById('grandchild')?.contentDocument;
    const srcdoc = document.getElementById('srcdoc-child')?.contentDocument;
    const shadow = document.getElementById('shadow-host')?.shadowRoot;
    const targets = [
        [window, document.getElementById('main-target'), 'block'],
        [window, document.getElementById('adopted-target'), 'block'],
        [window, document.getElementById('dynamic-target'), 'block'],
        [window, shadow?.getElementById('shadow-target'), 'inline'],
        [child?.defaultView, child?.getElementById('child-target'), 'block'],
        [grandchild?.defaultView, grandchild?.getElementById('grandchild-target'), 'block'],
        [srcdoc?.defaultView, srcdoc?.getElementById('srcdoc-target'), 'block'],
    ];
    if (!targets.every(([view, target]) => view && target)) {
        return false;
    }
    const authorStylesAreAbsent = targets.every(([view, target, display]) => {
        const style = view.getComputedStyle(target);
        return style.display === display && style.color === 'rgb(0, 0, 0)';
    });
    const scriptsRan = [document, child, grandchild, srcdoc].every(
        targetDocument => targetDocument?.documentElement?.getAttribute('data-script-ran') === 'yes'
    );
    if (!authorStylesAreAbsent || !scriptsRan) {
        return false;
    }
    document.documentElement.setAttribute('data-disable-css-probe', 'ready');
    return true;
})()"#;

struct DisableCssFixtureServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl DisableCssFixtureServer {
    async fn spawn() -> Result<Self> {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/page.html", get(|| async { Html(MAIN_PAGE) }))
            .route("/child.html", get(|| async { Html(CHILD_PAGE) }))
            .route("/grandchild.html", get(|| async { Html(GRANDCHILD_PAGE) }))
            .route("/baseline.html", get(baseline_page))
            .route("/redirect.html", get(redirect_to_page))
            .route("/document-open.html", get(document_open_page))
            .fallback(css_resource)
            .with_state(Arc::clone(&requests));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("disable-css fixture server should serve");
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

impl Drop for DisableCssFixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn baseline_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
        <link rel="stylesheet" href="/external.css">
        <div id="main-target" class="external">baseline</div>"#,
    )
}

async fn redirect_to_page() -> Response {
    (
        StatusCode::FOUND,
        [("location", "/page.html"), ("cache-control", "no-store")],
        "",
    )
        .into_response()
}

async fn document_open_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
        <script>
          document.open();
          document.write(`<!doctype html>
            <link id="rewrite-link" rel="stylesheet" href="/rewrite.css">
            <style>@import url('/rewrite-import.css'); #rewrite-target { display:none; color:rgb(91,92,93) }</style>
            <div id="rewrite-target" style="display:none; color:rgb(94,95,96)">rewrite</div>`);
          document.close();
          document.documentElement.setAttribute('data-rewrite-script-ran', 'yes');
        </script>"#,
    )
}

async fn css_resource(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
    let path = uri.path().to_owned();
    requests.lock().push(path.clone());
    let body = match path.as_str() {
        "/external.css" => {
            "@import url('/external-import.css'); .external { color: rgb(51, 52, 53); }"
        }
        "/external-import.css" => "#main-target { display: none; }",
        "/child.css" => "#child-target { color: rgb(61, 62, 63); }",
        "/grandchild.css" => "#grandchild-target { color: rgb(71, 72, 73); }",
        _ if path.ends_with(".css") => "body { color: rgb(81, 82, 83); }",
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
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

fn assert_success(output: &Output, requests: &[String], scenario: &str) -> String {
    assert!(
        output.status.success(),
        "{scenario} failed: stdout={}\nstderr={}\nrequests={requests:?}",
        clean_output(&output.stdout),
        clean_output(&output.stderr),
    );
    clean_output(&output.stdout)
}

fn assert_no_css_requests(server: &DisableCssFixtureServer, scenario: &str) {
    for path in CSS_PATHS {
        assert_eq!(
            server.request_count(path),
            0,
            "{scenario} unexpectedly requested {path}; requests={:?}",
            server.requests.lock().as_slice()
        );
    }
}

#[test]
fn disables_all_author_style_surfaces_without_disabling_scripts() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/page.html"),
        &[
            "--disable-css",
            "--with-frames",
            "--timeout",
            "5000",
            "--wait-script",
            DISABLED_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(
        &output,
        server.requests.lock().as_slice(),
        "disabled author styles",
    );

    assert!(
        stdout.contains("data-disable-css-probe=\"ready\""),
        "automation must observe UA-only computed styles: {stdout}"
    );
    assert!(
        stdout.contains("data-script-ran=\"yes\""),
        "--disable-css must not disable page JavaScript: {stdout}"
    );
    for preserved in [
        "id=\"external-link\"",
        "id=\"inline-sheet\"",
        "style=\"color: rgb(4, 5, 6); display: none\"",
    ] {
        assert!(
            stdout.contains(preserved),
            "disabled CSS must remain represented in the DOM ({preserved}): {stdout}"
        );
    }
    assert_no_css_requests(&server, "--disable-css");
    Ok(())
}

#[test]
fn survives_http_redirect_without_losing_the_page_policy() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/redirect.html"),
        &[
            "--disable-css",
            "--with-frames",
            "--timeout",
            "5000",
            "--wait-script",
            DISABLED_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(
        &output,
        server.requests.lock().as_slice(),
        "redirected disabled author styles",
    );

    assert!(stdout.contains("data-disable-css-probe=\"ready\""));
    assert_no_css_requests(&server, "redirected --disable-css");
    Ok(())
}

#[test]
fn survives_document_open_replacement() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        const target = document.getElementById('rewrite-target');
        if (!target || document.documentElement.getAttribute('data-rewrite-script-ran') !== 'yes') {
            return false;
        }
        const style = getComputedStyle(target);
        const ready = style.display === 'block' && style.color === 'rgb(0, 0, 0)';
        if (ready) document.documentElement.setAttribute('data-rewrite-css-probe', 'ready');
        return ready;
    })()"#;

    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/document-open.html"),
        &[
            "--disable-css",
            "--timeout",
            "5000",
            "--wait-script",
            READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(
        &output,
        server.requests.lock().as_slice(),
        "document.open disabled author styles",
    );

    assert!(stdout.contains("data-rewrite-css-probe=\"ready\""));
    assert!(stdout.contains("id=\"rewrite-link\""));
    assert_no_css_requests(&server, "document.open --disable-css");
    Ok(())
}

#[test]
fn author_styles_remain_enabled_by_default() -> Result<()> {
    const READY_SCRIPT: &str = r#"(() => {
        const target = document.getElementById('main-target');
        if (!target) return false;
        const ready = getComputedStyle(target).color === 'rgb(51, 52, 53)';
        if (ready) document.documentElement.setAttribute('data-css-probe', 'ready');
        return ready;
    })()"#;

    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssFixtureServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/baseline.html"),
        &["--timeout", "5000", "--wait-script", READY_SCRIPT],
    )?;
    let stdout = assert_success(
        &output,
        server.requests.lock().as_slice(),
        "default author styles",
    );

    assert!(stdout.contains("data-css-probe=\"ready\""));
    assert_eq!(server.request_count("/external.css"), 1);
    assert_eq!(server.request_count("/external-import.css"), 1);
    Ok(())
}

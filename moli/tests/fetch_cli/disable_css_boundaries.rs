use super::{Output, clean_output, run_fetch_cli_with_args};
use anyhow::Result;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const RESOURCE_PAGE: &str = r#"<!doctype html>
<style>
  @import url('/disabled-import.css');
  @font-face {
    font-family: DisabledRemoteFont;
    src: url('/disabled-font.woff2') format('woff2');
  }
  #css-resource-target {
    background-image: url('/disabled-background.svg');
    cursor: url('/disabled-cursor.svg'), pointer;
    font-family: DisabledRemoteFont;
    list-style-image: url('/disabled-list-marker.svg');
  }
  #css-resource-target::before { content: url('/disabled-content.svg'); }
</style>
<div id="css-resource-target"
     style="background-image: url('/disabled-inline-background.svg')">target</div>
<div id="resource-shadow-host"></div>
<img id="direct-image" src="/direct-image.svg"
     onload="document.documentElement.dataset.directImage = 'load'"
     onerror="document.documentElement.dataset.directImage = 'error'">
<script>
  const root = document.getElementById('resource-shadow-host')
    .attachShadow({mode: 'open'});
  root.innerHTML = `<style>
    #shadow-target { background-image: url('/disabled-shadow-background.svg'); }
  </style><span id="shadow-target">shadow</span>`;
</script>"#;

const RESOURCE_READY_SCRIPT: &str = r#"(() => {
  if (document.documentElement.dataset.directImage !== 'load') return false;
  const target = document.getElementById('css-resource-target');
  const shadowTarget = document.getElementById('resource-shadow-host')
    ?.shadowRoot?.getElementById('shadow-target');
  if (!target || !shadowTarget) return false;
  const targetStyle = getComputedStyle(target);
  const shadowStyle = getComputedStyle(shadowTarget);
  const inactive =
    targetStyle.backgroundImage === 'none' &&
    targetStyle.cursor === 'auto' &&
    targetStyle.listStyleImage === 'none' &&
    shadowStyle.backgroundImage === 'none';
  if (inactive) document.documentElement.dataset.resourceBoundary = 'ready';
  return inactive;
})()"#;

const CSS_MODULE_PAGE: &str = r#"<!doctype html>
<div id="module-target">module target</div>
<script>
  globalThis.cssModuleResult = 'pending';
  globalThis.explicitCssFetchResult = 'pending';
  import('/blocked-module.css', {with: {type: 'css'}}).then(module => {
    document.adoptedStyleSheets = [module.default];
    globalThis.cssModuleResult = 'loaded';
  }, error => {
    globalThis.cssModuleResult = 'rejected:' + error.name;
  });
  fetch('/explicit-fetch.css').then(response => response.text()).then(text => {
    globalThis.explicitCssFetchResult = text;
  }, error => {
    globalThis.explicitCssFetchResult = 'rejected:' + error.name;
  });
</script>"#;

const CSS_MODULE_READY_SCRIPT: &str = r#"(() => {
  if (!String(globalThis.cssModuleResult).startsWith('rejected:')) return false;
  if (globalThis.explicitCssFetchResult !== 'explicit-css-fetch-ok') return false;
  const target = document.getElementById('module-target');
  const inactive = target &&
    getComputedStyle(target).display === 'block' &&
    getComputedStyle(target).color === 'rgb(0, 0, 0)' &&
    document.adoptedStyleSheets.length === 0;
  if (!inactive) return false;
  document.documentElement.dataset.cssModuleBoundary = [
    globalThis.cssModuleResult,
    globalThis.explicitCssFetchResult,
  ].join('|');
  return true;
})()"#;

const CACHE_WARMED_CSS_MODULE_PAGE: &str = r#"<!doctype html>
<div id="module-target">cache-warmed module target</div>
<script>
  globalThis.cacheWarmedCssModuleResult = 'pending';
  (async () => {
    const response = await fetch('/cache-warmed-module.css');
    const source = await response.text();
    let moduleResult = 'loaded';
    try {
      const module = await import('/cache-warmed-module.css', {with: {type: 'css'}});
      document.adoptedStyleSheets = [module.default];
    } catch (error) {
      moduleResult = 'rejected:' + error.name;
    }
    globalThis.cacheWarmedCssModuleResult = [source, moduleResult].join('|');
  })().catch(error => {
    globalThis.cacheWarmedCssModuleResult = 'failed:' + error.name;
  });
</script>"#;

const CACHE_WARMED_CSS_MODULE_READY_SCRIPT: &str = r#"(() => {
  const result = String(globalThis.cacheWarmedCssModuleResult);
  if (!result.startsWith('cache-warm-ok|rejected:')) return false;
  const target = document.getElementById('module-target');
  if (!target || getComputedStyle(target).display !== 'block' ||
      document.adoptedStyleSheets.length !== 0) {
    return false;
  }
  document.documentElement.dataset.cacheWarmedCssModuleBoundary = result;
  return true;
})()"#;

const SERVICE_WORKER_PAGE: &str = r#"<!doctype html>
<div id="sw-target">service worker target</div>
<script>
  globalThis.serviceWorkerCssBoundary = 'pending';
  (async () => {
    const controllerChanged = navigator.serviceWorker.controller
      ? Promise.resolve()
      : new Promise(resolve => navigator.serviceWorker.addEventListener(
          'controllerchange', resolve, {once: true}));
    const registration = await navigator.serviceWorker.register(
      '/disable-css-worker.js', {scope: '/'});
    const worker = registration.installing || registration.waiting || registration.active;
    if (worker && worker.state !== 'activated') {
      await new Promise(resolve => worker.addEventListener('statechange', () => {
        if (worker.state === 'activated') resolve();
      }));
    }
    await navigator.serviceWorker.ready;
    if (!navigator.serviceWorker.controller) await controllerChanged;

    const probe = await fetch('/sw-probe.txt').then(response => response.text());
    const linkResult = await new Promise(resolve => {
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.href = '/sw-style.css';
      link.onload = () => resolve('load');
      link.onerror = () => resolve('error');
      document.head.appendChild(link);
    });
    const stylesheetFetchCount = await fetch('/sw-report.txt')
      .then(response => response.text());
    globalThis.serviceWorkerCssBoundary = [
      probe,
      linkResult,
      stylesheetFetchCount,
      String(Boolean(navigator.serviceWorker.controller)),
    ].join('|');
  })().catch(error => {
    globalThis.serviceWorkerCssBoundary = 'rejected:' + error.name + ':' + error.message;
  });
</script>"#;

const SERVICE_WORKER_READY_SCRIPT: &str = r#"(() => {
  if (globalThis.serviceWorkerCssBoundary !== 'worker-probe|error|0|true') return false;
  const target = document.getElementById('sw-target');
  if (!target || getComputedStyle(target).color !== 'rgb(0, 0, 0)') return false;
  document.documentElement.dataset.serviceWorkerCssBoundary =
    globalThis.serviceWorkerCssBoundary;
  return true;
})()"#;

const SERVICE_WORKER_SCRIPT: &str = r#"
let stylesheetFetchCount = 0;
self.addEventListener('install', event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener('activate', event => {
  event.waitUntil(self.clients.claim());
});
self.addEventListener('fetch', event => {
  const path = new URL(event.request.url).pathname;
  if (path === '/sw-style.css') {
    stylesheetFetchCount += 1;
    event.respondWith(new Response('#sw-target { color: red; }', {
      headers: {'content-type': 'text/css'}
    }));
    return;
  }
  if (path === '/sw-probe.txt') {
    event.respondWith(new Response('worker-probe'));
    return;
  }
  if (path === '/sw-report.txt') {
    event.respondWith(new Response(String(stylesheetFetchCount)));
  }
});"#;

const CSS_DERIVED_RESOURCE_PATHS: [&str; 8] = [
    "/disabled-import.css",
    "/disabled-font.woff2",
    "/disabled-background.svg",
    "/disabled-inline-background.svg",
    "/disabled-cursor.svg",
    "/disabled-list-marker.svg",
    "/disabled-content.svg",
    "/disabled-shadow-background.svg",
];

struct DisableCssBoundaryServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl DisableCssBoundaryServer {
    async fn spawn() -> Result<Self> {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route(
                "/resource-boundary.html",
                get(|| async { Html(RESOURCE_PAGE) }),
            )
            .route(
                "/css-module-boundary.html",
                get(|| async { Html(CSS_MODULE_PAGE) }),
            )
            .route(
                "/cache-warmed-css-module-boundary.html",
                get(|| async { Html(CACHE_WARMED_CSS_MODULE_PAGE) }),
            )
            .route(
                "/service-worker-boundary.html",
                get(|| async { Html(SERVICE_WORKER_PAGE) }),
            )
            .route(
                "/disable-css-worker.js",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                        SERVICE_WORKER_SCRIPT,
                    )
                }),
            )
            .fallback(boundary_resource)
            .with_state(Arc::clone(&requests));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("disable-css boundary server should serve");
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

impl Drop for DisableCssBoundaryServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn boundary_resource(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
    let path = uri.path().to_owned();
    requests.lock().push(path.clone());
    match path.as_str() {
        "/direct-image.svg" => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/svg+xml")],
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"></svg>"#,
        )
            .into_response(),
        "/blocked-module.css" => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/css")],
            "#module-target { display: none; color: red; }",
        )
            .into_response(),
        "/explicit-fetch.css" => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/css")],
            "explicit-css-fetch-ok",
        )
            .into_response(),
        "/cache-warmed-module.css" => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/css"),
                (header::CACHE_CONTROL, "max-age=3600"),
            ],
            "cache-warm-ok",
        )
            .into_response(),
        path if path.ends_with(".css") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/css")],
            "body { background-image: url('/unexpected-nested.svg'); }",
        )
            .into_response(),
        path if path.ends_with(".svg") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/svg+xml")],
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
        )
            .into_response(),
        path if path.ends_with(".woff2") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "font/woff2")],
            "not-a-real-font",
        )
            .into_response(),
        _ => (StatusCode::NOT_FOUND, "network-fallback").into_response(),
    }
}

fn assert_success(output: &Output, server: &DisableCssBoundaryServer, scenario: &str) -> String {
    assert!(
        output.status.success(),
        "{scenario} failed: stdout={}\nstderr={}\nrequests={:?}",
        clean_output(&output.stdout),
        clean_output(&output.stderr),
        server.requests.lock().as_slice(),
    );
    clean_output(&output.stdout)
}

#[test]
fn resource_mode_fetches_dom_resources_without_fetching_css_derived_resources() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssBoundaryServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/resource-boundary.html"),
        &[
            "--disable-css",
            "--resource",
            "--timeout",
            "5000",
            "--wait-script",
            RESOURCE_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(&output, &server, "--resource disable-css boundary");

    assert!(stdout.contains("data-resource-boundary=\"ready\""));
    assert_eq!(server.request_count("/direct-image.svg"), 1);
    for path in CSS_DERIVED_RESOURCE_PATHS {
        assert_eq!(
            server.request_count(path),
            0,
            "disabled CSS unexpectedly fetched {path}; requests={:?}",
            server.requests.lock().as_slice()
        );
    }
    Ok(())
}

#[test]
fn css_module_fetch_is_blocked_but_explicit_fetch_of_css_remains_available() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssBoundaryServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/css-module-boundary.html"),
        &[
            "--disable-css",
            "--timeout",
            "5000",
            "--wait-script",
            CSS_MODULE_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(&output, &server, "CSS module disable-css boundary");

    assert!(stdout.contains("data-css-module-boundary=\"rejected:"));
    assert!(stdout.contains("|explicit-css-fetch-ok\""));
    assert_eq!(server.request_count("/blocked-module.css"), 0);
    assert_eq!(server.request_count("/explicit-fetch.css"), 1);
    Ok(())
}

#[test]
fn cached_fetch_response_cannot_bypass_css_module_blocking() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssBoundaryServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/cache-warmed-css-module-boundary.html"),
        &[
            "--disable-css",
            "--timeout",
            "5000",
            "--wait-script",
            CACHE_WARMED_CSS_MODULE_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(&output, &server, "cache-warmed CSS module boundary");

    assert!(stdout.contains("data-cache-warmed-css-module-boundary=\"cache-warm-ok|rejected:"));
    assert_eq!(
        server.request_count("/cache-warmed-module.css"),
        1,
        "only the explicit fetch may reach the server"
    );
    Ok(())
}

#[test]
fn service_worker_cannot_intercept_a_policy_blocked_stylesheet() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(DisableCssBoundaryServer::spawn())?;
    let output = run_fetch_cli_with_args(
        &server.url("/service-worker-boundary.html"),
        &[
            "--disable-css",
            "--timeout",
            "10000",
            "--wait-script",
            SERVICE_WORKER_READY_SCRIPT,
        ],
    )?;
    let stdout = assert_success(&output, &server, "Service Worker disable-css boundary");

    assert!(stdout.contains("data-service-worker-css-boundary=\"worker-probe|error|0|true\""));
    for path in ["/sw-style.css", "/sw-probe.txt", "/sw-report.txt"] {
        assert_eq!(
            server.request_count(path),
            0,
            "Service Worker boundary unexpectedly reached network for {path}; requests={:?}",
            server.requests.lock().as_slice()
        );
    }
    Ok(())
}

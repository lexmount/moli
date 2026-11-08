use super::clean_output;
use anyhow::Result;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

// Reduced from the Anubis 1.27 challenge template. Upstream changed the
// challenge module from `async` to `defer` so it cannot observe DOM that the
// stylesheet-blocked parser has not created yet.
const CHALLENGE_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <title>local anubis challenge</title>
    <script>globalThis.__anubisOrder = ["bootstrap"];</script>
    <link rel="stylesheet" href="/anubis/blocked.css">
  </head>
  <body>
    <main id="challenge">
      <p id="status">calculating</p>
      <script defer type="module" src="/anubis/main.mjs"></script>
      <script src="/anubis/parser-continuation.js"></script>
      <div id="progress"><div class="bar-inner"></div></div>
      <script>document.documentElement.dataset.executionOrder = __anubisOrder.join(',');</script>
    </main>
  </body>
</html>"#;

const MAIN_MODULE: &str = r#"
const locale = await (await fetch('/anubis/locale.json')).json();
globalThis.__anubisOrder.push('module-continuation');
const progress = document.getElementById('progress');
document.documentElement.dataset.progressAtModuleContinuation = progress ? 'present' : 'missing';
if (progress) progress.style.display = 'block';
document.getElementById('status').textContent = locale.ready;
location.replace(
  '/anubis/passed?order=' + encodeURIComponent(__anubisOrder.join(',')) +
  '&style=' + encodeURIComponent(document.documentElement.dataset.styleAtParserContinuation || 'missing') +
  '&progress=' + encodeURIComponent(document.documentElement.dataset.progressAtModuleContinuation)
);
"#;

const PARSER_CONTINUATION: &str = r#"
globalThis.__anubisOrder.push('parser-continuation');
document.documentElement.dataset.parserContinuation = 'ran';
document.documentElement.dataset.styleAtParserContinuation =
  getComputedStyle(document.getElementById('status')).color;
"#;

#[derive(Default)]
struct Event {
    signaled: AtomicBool,
    notify: Notify,
}

impl Event {
    fn signal(&self) {
        if !self.signaled.swap(true, Ordering::Release) {
            self.notify.notify_waiters();
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.signaled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Clone, Default)]
struct FixtureState {
    stylesheet_requested: Arc<Event>,
    parser_continuation_requested: Arc<Event>,
}

struct AnubisDeferredModuleFixture {
    base_url: String,
    task: JoinHandle<()>,
}

impl AnubisDeferredModuleFixture {
    async fn spawn() -> Result<Self> {
        let state = FixtureState::default();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/anubis/challenge", get(|| async { Html(CHALLENGE_PAGE) }))
            .route("/anubis/blocked.css", get(blocked_stylesheet))
            .route(
                "/anubis/main.mjs",
                get(|| async { javascript_response(MAIN_MODULE) }),
            )
            .route("/anubis/parser-continuation.js", get(parser_continuation))
            .route("/anubis/locale.json", get(locale))
            .route("/anubis/passed", get(passed))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("Anubis deferred-module fixture should serve");
        });
        Ok(Self {
            base_url: format!("http://{addr}"),
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

async fn passed(uri: Uri) -> Html<String> {
    Html(format!(
        "<!doctype html><title>passed</title><main id='passed' data-query='{}'>passed</main>",
        uri.query().unwrap_or_default()
    ))
}

impl Drop for AnubisDeferredModuleFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blocked_stylesheet(State(state): State<FixtureState>) -> Response {
    state.stylesheet_requested.signal();
    // A parser-blocking external classic script starts its fetch only after
    // the HTML parser has reached that script boundary. Waiting for this
    // request proves the parser is suspended behind this stylesheet before
    // the stylesheet terminal is released; the test does not depend on a
    // sleep being long enough for the parser to get there.
    state.parser_continuation_requested.wait().await;
    (
        StatusCode::OK,
        [
            ("content-type", "text/css; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        "#status { color: rgb(1, 2, 3); } #progress { display: none; }",
    )
        .into_response()
}

async fn parser_continuation(State(state): State<FixtureState>) -> Response {
    state.parser_continuation_requested.signal();
    javascript_response(PARSER_CONTINUATION)
}

async fn locale(State(state): State<FixtureState>) -> Response {
    state.stylesheet_requested.wait().await;
    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", "no-store"),
        ],
        r#"{"ready":"worker-ready"}"#,
    )
        .into_response()
}

fn javascript_response(source: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        source,
    )
        .into_response()
}

#[test]
fn cli_deferred_module_observes_stylesheet_unblocked_parser_dom() -> Result<()> {
    let runtime = super::fixture_runtime()?;
    let server = runtime.block_on(AnubisDeferredModuleFixture::spawn())?;
    let url = server.url("/anubis/challenge");
    let output = Command::new(env!("CARGO_BIN_EXE_moli"))
        .args([
            "fetch",
            "--http-no-proxy",
            "*",
            "--wait-until",
            "done",
            "--wait-selector",
            "#passed",
            "--timeout",
            "3000",
            "--dump",
            "html",
            &url,
        ])
        .output()?;
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);

    assert!(
        output.status.success(),
        "local Anubis fetch failed: stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("id=\"passed\""),
        "deferred module did not finish the local challenge: stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("order=bootstrap%2Cparser-continuation%2Cmodule-continuation"),
        "deferred module ran before parsing completed: stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("style=rgb(1%2C%202%2C%203)"),
        "the parser resumed before the completed stylesheet affected computed style: stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("progress=present"),
        "deferred module did not observe parser-created DOM: stdout={stdout}\nstderr={stderr}"
    );
    Ok(())
}

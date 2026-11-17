use super::*;
use axum::body::Body;
use futures_util::{StreamExt as _, stream};
use std::convert::Infallible;
use std::time::Duration;

pub(super) const TIMEOUT_MS: &str = "30000";

// A 10s main response leaves at most 20s for the 24s activity. Resetting the
// readiness budget would let this exact page become quiet and return normally.
// Both sides have seconds of scheduling slack instead of a sub-500ms window.
pub(super) const BECOMES_QUIET: &str = r#"<!doctype html><body data-activity="pending">
<p>slow-main=ready</p>
<script>
let tick = 0;
const interval = setInterval(() => { document.body.dataset.tick = String(++tick); }, 100);
fetch('/activity').then(() => {
    clearInterval(interval);
    document.body.dataset.activity = 'complete';
}).catch(() => {});
</script></body>"#;

// NetworkIdle exhausts the shared 30s deadline before the selector appears.
// A fresh selector budget would observe it and incorrectly succeed, so this
// regression uses the CLI result rather than a wall-clock upper bound.
pub(super) const LATE_SELECTOR: &str = r#"<!doctype html><body>
<p>post-readiness-budget</p>
<script>
fetch('/pending').catch(() => {});
setTimeout(() => {
    const ready = document.createElement('div');
    ready.id = 'late-ready';
    document.body.appendChild(ready);
}, 35000);
</script></body>"#;

// Streaming spends 10s of the original 30s budget before Load. The selector
// appears 25s after Load: beyond the shared deadline but within a fresh one.
const STREAMING_ERROR_HEAD: &str = r#"<!doctype html><html><body>
<script>
window.addEventListener('load', () => {
    setTimeout(() => {
        const ready = document.createElement('main');
        ready.id = 'slow-http-error-post-load-after-total-deadline';
        document.body.appendChild(ready);
    }, 25000);
});
</script>"#;

pub(super) struct TimingFixture {
    base_url: String,
    task: JoinHandle<()>,
}

impl TimingFixture {
    pub(super) async fn spawn(page: &'static str) -> Result<Self> {
        let app = Router::new()
            .route(
                "/page",
                get(move || async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Html(page)
                }),
            )
            .route(
                "/activity",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(24)).await;
                    "complete"
                }),
            )
            .route("/pending", get(std::future::pending::<&'static str>));
        Self::serve(app).await
    }

    pub(super) async fn spawn_streaming_error() -> Result<Self> {
        let app = Router::new().route(
            "/page",
            get(|| async {
                let head = stream::once(async { Ok::<_, Infallible>(STREAMING_ERROR_HEAD) });
                let tail = stream::once(async {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok::<_, Infallible>("<main id='streaming-tail'></main></body></html>")
                });
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/html; charset=utf-8")
                    .body(Body::from_stream(head.chain(tail)))
                    .unwrap()
            }),
        );
        Self::serve(app).await
    }

    async fn serve(app: Router) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self { base_url, task })
    }

    pub(super) fn url(&self) -> String {
        format!("{}/page", self.base_url)
    }
}

impl Drop for TimingFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

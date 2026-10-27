use super::{clean_output, run_fetch_cli_with_args};
use anyhow::Result;
use axum::{Router, body::Body, extract::Query, response::Html, routing::get};
use std::{collections::HashMap, convert::Infallible, time::Duration};

const PAGE: &str = include_str!("xhr_timeout.html");

async fn response(Query(delays): Query<HashMap<String, u64>>) -> Body {
    tokio::time::sleep(Duration::from_millis(*delays.get("head").unwrap_or(&0))).await;
    Body::from_stream(futures_util::stream::once(async move {
        tokio::time::sleep(Duration::from_millis(*delays.get("body").unwrap_or(&0))).await;
        Ok::<_, Infallible>("network-body")
    }))
}

#[test]
fn xhr_timeouts_preserve_completed_responses_after_long_javascript_tasks() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(32)
        .enable_all()
        .build()?;
    let (url, server) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/page", listener.local_addr()?);
        let app = Router::new()
            .route("/page", get(|| async { Html(PAGE) }))
            .route("/response", get(response));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Ok::<_, anyhow::Error>((url, server))
    })?;
    let result = run_fetch_cli_with_args(
        &url,
        &[
            "--timeout",
            "45000",
            "--wait-script",
            "document.body.dataset.xhrTimeouts !== undefined",
        ],
    );
    server.abort();
    assert!(runtime.block_on(server).unwrap_err().is_cancelled());
    let output = result?;
    let stdout = clean_output(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout}\nstderr={}",
        clean_output(&output.stderr)
    );
    assert!(
        stdout.contains("window:10/10|worker:10/10"),
        "completed responses, pending timeouts, timeout changes, abort and request reuse: {stdout}"
    );
    Ok(())
}

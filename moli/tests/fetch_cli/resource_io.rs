use super::{clean_output, run_fetch_cli_with_args};
use anyhow::Result;
use axum::{
    Router,
    extract::{Path, State},
    response::Html,
    routing::get,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const PAGE: &str = include_str!("resource_io.html");

async fn body(State(received): State<Arc<AtomicUsize>>, Path(scope): Path<String>) -> &'static str {
    received.fetch_or(scope_bit(&scope), Ordering::SeqCst);
    "network-body"
}

async fn observed(
    State(received): State<Arc<AtomicUsize>>,
    Path(scope): Path<String>,
) -> &'static str {
    if received.load(Ordering::SeqCst) & scope_bit(&scope) != 0 {
        "yes"
    } else {
        "no"
    }
}

fn scope_bit(scope: &str) -> usize {
    match scope {
        "window" => 1,
        "worker" => 2,
        _ => panic!("unexpected probe scope: {scope}"),
    }
}

#[test]
fn resource_io_runs_while_window_and_worker_javascript_are_busy() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(32)
        .enable_all()
        .build()?;
    let received = Arc::new(AtomicUsize::new(0));
    let (url, server) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/page", listener.local_addr()?);
        let app = Router::new()
            .route("/page", get(|| async { Html(PAGE) }))
            .route("/body/{scope}", get(body))
            .route("/observed/{scope}", get(observed))
            .with_state(Arc::clone(&received));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok::<_, anyhow::Error>((url, server))
    })?;
    let output = run_fetch_cli_with_args(
        &url,
        &[
            "--timeout",
            "10000",
            "--wait-script",
            "document.body.dataset.resourceIo !== undefined",
        ],
    )?;
    server.abort();
    assert!(runtime.block_on(server).unwrap_err().is_cancelled());

    let stdout = clean_output(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout}\nstderr={}",
        clean_output(&output.stderr)
    );
    assert!(
        stdout.contains("window:yes:200:network-body:true|worker:yes:200:network-body:true"),
        "requests must start during the JS task, with events dispatched after it: {stdout}"
    );
    assert_eq!(received.load(Ordering::SeqCst), 3);
    Ok(())
}

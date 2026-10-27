use super::{clean_output, run_fetch_cli_with_args};
use anyhow::Result;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderName, StatusCode, header},
    response::Html,
    routing::get,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const PAGE: &str = include_str!("redirect_owner.html");

fn bit(id: usize, phase: usize) -> u64 {
    assert!(id < 20);
    1 << (id * 3 + phase)
}

async fn redirect(
    State(observed): State<Arc<AtomicU64>>,
    Path(id): Path<usize>,
) -> (StatusCode, [(HeaderName, String); 1], &'static str) {
    observed.fetch_or(bit(id, 0), Ordering::SeqCst);
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("/target/{id}"))],
        "",
    )
}

async fn target(State(observed): State<Arc<AtomicU64>>, Path(id): Path<usize>) -> &'static str {
    observed.fetch_or(bit(id, 1), Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    "body"
}

async fn direct(State(observed): State<Arc<AtomicU64>>, Path(id): Path<usize>) -> &'static str {
    observed.fetch_or(bit(id, 2), Ordering::SeqCst);
    "direct"
}

async fn snapshot(State(observed): State<Arc<AtomicU64>>, Path(id): Path<usize>) -> String {
    let value = observed.load(Ordering::SeqCst);
    (0..3)
        .map(|phase| {
            if value & bit(id, phase) != 0 {
                "yes"
            } else {
                "no"
            }
        })
        .collect::<Vec<_>>()
        .join(":")
}

#[test]
fn redirects_wait_for_script_owners_without_blocking_other_resource_io() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(32)
        .enable_all()
        .build()?;
    let observed = Arc::new(AtomicU64::new(0));
    let (url, server) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/page", listener.local_addr()?);
        let app = Router::new()
            .route("/page", get(|| async { Html(PAGE) }))
            .route("/redirect/{id}", get(redirect))
            .route("/target/{id}", get(target))
            .route("/direct/{id}", get(direct))
            .route("/observed/{id}", get(snapshot))
            .with_state(observed);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Ok::<_, anyhow::Error>((url, server))
    })?;
    let result = run_fetch_cli_with_args(
        &url,
        &[
            "--timeout",
            "30000",
            "--wait-script",
            "document.body.dataset.redirectOwners !== undefined",
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
        stdout.contains("window:9/9|worker:9/9"),
        "redirect owners, cancellation, keepalive and independent I/O: {stdout}"
    );
    Ok(())
}

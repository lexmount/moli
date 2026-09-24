use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const PROBE: &str = include_str!("frame_attribute_ancestors.js");

async fn ancestor_page() -> (SameDocumentPage, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let loading = gate.clone();
    let started = gate.clone();
    let routes = axum::Router::new()
        .route(
            "/frame-parent.html",
            axum::routing::get(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                async { axum::response::Html("<!doctype html><body>parent</body>") }
            }),
        )
        .route(
            "/frame-child.html",
            axum::routing::get(move |uri: axum::http::Uri| {
                let gate = loading.clone();
                async move {
                    if uri.query() == Some("pending") {
                        gate.requests.fetch_add(1, Ordering::SeqCst);
                        gate.release.acquire().await.unwrap().forget();
                    }
                    axum::response::Html("<!doctype html><body>child</body>")
                }
            }),
        )
        .route(
            "/pending-started",
            axum::routing::get(move || {
                let gate = started.clone();
                async move {
                    ResponseGate::wait_for(&gate.requests, 1).await;
                    "started"
                }
            }),
        )
        .route(
            "/release-pending",
            axum::routing::get(move || {
                gate.release.add_permits(1);
                async { "released" }
            }),
        );
    let mut page = SameDocumentPage::with_routes(routes).await;
    let url = page.base_url.replace("/history.html", "/frame-parent.html");
    page.command("Page.navigate", json!({"url": url})).await;
    wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
    (page, requests)
}

async fn probe(
    page: &mut SameDocumentPage,
    mode: &str,
    method: &str,
    depth: usize,
) -> serde_json::Value {
    page.evaluate(&format!("({PROBE})({mode:?}, {method:?}, {depth})"))
        .await
}

fn assert_unchanged(result: &serde_json::Value, url: &str) {
    assert_eq!(result["sameDocument"], true, "{result}");
    assert_eq!(result["url"], url, "{result}");
    assert_eq!(result["reflected"], true, "{result}");
    assert_eq!(result["events"], json!([]), "{result}");
    assert_eq!(result["loads"], 0, "{result}");
    assert_eq!(result["historyDelta"], 0, "{result}");
    assert_eq!(result["readyState"], "complete", "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_src_matching_any_ancestor_is_reflected_without_navigation() {
    for method in ["src", "attribute", "namespace", "value", "node"] {
        for depth in [0, 2] {
            let (mut page, requests) = ancestor_page().await;
            let result = probe(&mut page, "blocked", method, depth).await;
            assert_unchanged(&result, "/frame-child.html?initial");
            assert_eq!(requests.load(Ordering::SeqCst), 1, "{method}/{depth}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_initial_insertion_and_srcdoc_removal_reject_ancestor_urls() {
    for (mode, expected) in [
        ("insert", "about:blank"),
        ("frame", "/frame-child.html?initial"),
        ("srcdoc", "about:srcdoc"),
        ("remove-srcdoc", "about:srcdoc"),
    ] {
        let (mut page, requests) = ancestor_page().await;
        let result = probe(&mut page, mode, "src", 0).await;
        assert_unchanged(&result, expected);
        if mode == "insert" {
            assert_eq!(result["initialLoads"], 0);
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1, "{mode}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_frame_src_preserves_pending_navigation_and_only_retries_on_mutation() {
    for method in ["src", "attribute", "namespace", "value", "node"] {
        let (mut page, requests) = ancestor_page().await;
        let pending = probe(&mut page, "pending", method, 0).await;
        assert_eq!(pending["url"], "/frame-child.html?pending", "{pending}");
        assert_eq!(
            pending["events"],
            json!(["beforeunload", "pagehide", "unload"])
        );
        assert_eq!(pending["loads"], 1);
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        let retried = probe(&mut page, "ancestor-change", method, 0).await;
        assert_unchanged(&retried, "/frame-child.html?initial");
        assert_unchanged(&retried["afterAncestorChange"], "/frame-child.html?initial");
        assert_eq!(
            retried["afterRetry"]["url"],
            "/frame-parent.html#ignored-fragment"
        );
        assert_eq!(retried["afterRetry"]["sameDocument"], false);
        assert_eq!(retried["afterRetry"]["loads"], 1);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_url_attribute_rule_preserves_query_and_location_navigation_boundaries() {
    for (mode, url) in [
        ("location", "/frame-parent.html#ignored-fragment"),
        ("query", "/frame-parent.html?different"),
    ] {
        let (mut page, requests) = ancestor_page().await;
        let result = probe(&mut page, mode, "src", 0).await;
        assert_eq!(result["url"], url, "{result}");
        assert_eq!(result["sameDocument"], false);
        assert_eq!(result["loads"], 1);
        assert_eq!(
            result["events"],
            json!(["beforeunload", "pagehide", "unload"])
        );
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }
}

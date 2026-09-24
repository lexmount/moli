use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const PROBE: &str = include_str!("child_beforeunload.js");
const LOADING_PARENT: &str = r#"<!doctype html><body><script>
const trace = [['start', document.readyState]];
addEventListener('beforeunload', () => trace.push('parent'));
const frame = document.body.appendChild(document.createElement('iframe'));
frame.contentWindow.addEventListener('beforeunload', () => {
  trace.push('child');
  location.href = '/history.html?parent-destination';
});
frame.contentWindow.location.href = '/history.html?child-destination';
trace.push('after');
top.loadingBeforeunloadTrace = trace;
</script>"#;

async fn beforeunload_page(response: &'static str) -> (SameDocumentPage, Arc<ResponseGate>) {
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let observed = gate.clone();
    let routes = axum::Router::new().route(
        "/beforeunload.html",
        axum::routing::get(move || {
            let gate = observed.clone();
            async move {
                use axum::response::IntoResponse;
                let visit = gate.requests.fetch_add(1, Ordering::SeqCst) + 1;
                if visit == 2 {
                    gate.release.acquire().await.unwrap().forget();
                }
                gate.responses.fetch_add(1, Ordering::SeqCst);
                if visit == 2 {
                    match response {
                        "204" => return axum::http::StatusCode::NO_CONTENT.into_response(),
                        "205" => return axum::http::StatusCode::RESET_CONTENT.into_response(),
                        _ => {}
                    }
                }
                let mut result = (
                    [
                        (axum::http::header::CONTENT_TYPE, "text/html"),
                        (axum::http::header::CACHE_CONTROL, "no-store"),
                    ],
                    "<!doctype html><body>child</body>",
                )
                    .into_response();
                if visit == 2 && response == "attachment" {
                    result.headers_mut().insert(
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_static("attachment; filename=child.txt"),
                    );
                }
                result
            }
        }),
    );
    (SameDocumentPage::with_routes(routes).await, gate)
}

async fn setup(page: &mut SameDocumentPage, method: &str, action: &str) {
    page.evaluate(&format!(
        "({PROBE})({method:?}, {action:?}).then(probe => {{globalThis.beforeunloadProbe = probe}})"
    ))
    .await;
}

fn assert_pending(snapshot: &serde_json::Value) {
    assert_eq!(
        snapshot["events"],
        json!(["beforeunload", "child:beforeunload"]),
        "{snapshot}"
    );
    assert_eq!(
        snapshot["details"],
        json!([[true, true, true, false], [true, true, true, false]])
    );
    assert_eq!(snapshot["sameDocument"], true);
    assert_eq!(snapshot["nestedAlive"], true);
    assert_eq!(snapshot["oldHidden"], false);
    assert_eq!(snapshot["loads"], 0);
    assert_eq!(snapshot["url"], "?initial");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_beforeunload_precedes_response_without_retiring_document_or_repeating_at_commit() {
    for method in [
        "href",
        "assign",
        "replace",
        "reload",
        "navigation",
        "src",
        "anchor",
        "form",
    ] {
        let (mut page, gate) = beforeunload_page("html").await;
        setup(&mut page, method, "self").await;
        let immediate = page.evaluate("beforeunloadProbe.start()").await;
        if matches!(
            method,
            "href" | "assign" | "replace" | "reload" | "navigation"
        ) {
            assert_pending(&immediate);
        }
        ResponseGate::wait_for(&gate.requests, 2).await;
        assert_pending(&page.evaluate("beforeunloadProbe.snapshot()").await);
        gate.release.add_permits(1);
        let committed = page.evaluate("beforeunloadProbe.waitForLoad()").await;
        assert_eq!(committed["loads"], 1, "{method}: {committed}");
        assert_eq!(committed["sameDocument"], false);
        assert_eq!(committed["nestedAlive"], false);
        assert_eq!(committed["oldHidden"], true);
        assert_eq!(
            committed["events"],
            json!([
                "beforeunload",
                "child:beforeunload",
                "pagehide",
                "unload",
                "child:pagehide",
                "child:unload"
            ]),
            "{method}: {committed}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_beforeunload_ignored_response_allows_a_fresh_navigation_check() {
    for response in ["204", "205", "attachment"] {
        for method in ["href", "navigation", "src", "form"] {
            let (mut page, gate) = beforeunload_page(response).await;
            page.allow_downloads().await;
            setup(&mut page, method, "none").await;
            page.evaluate("beforeunloadProbe.start()").await;
            ResponseGate::wait_for(&gate.requests, 2).await;
            let pending = page.evaluate("beforeunloadProbe.snapshot()").await;
            assert_pending(&pending);
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 2).await;
            let ignored = page
                .evaluate("new Promise(r => setTimeout(r, 100)).then(beforeunloadProbe.snapshot)")
                .await;
            assert_eq!(ignored, pending, "{response}/{method}");
            page.evaluate("beforeunloadProbe.start()").await;
            let committed = page.evaluate("beforeunloadProbe.waitForLoad()").await;
            assert_eq!(committed["loads"], 1, "{response}/{method}: {committed}");
            assert_eq!(committed["sameDocument"], false);
            assert_eq!(
                committed["events"],
                json!([
                    "beforeunload",
                    "child:beforeunload",
                    "beforeunload",
                    "child:beforeunload",
                    "pagehide",
                    "unload",
                    "child:pagehide",
                    "child:unload"
                ]),
                "{response}/{method}: {committed}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_beforeunload_reentrant_stop_and_ancestor_navigation_preserve_the_current_navigation()
{
    for method in ["href", "navigation", "src"] {
        for action in ["stop", "supersede", "forged-flag"] {
            let (mut page, gate) = beforeunload_page("html").await;
            setup(&mut page, method, action).await;
            page.evaluate("beforeunloadProbe.start()").await;
            ResponseGate::wait_for(&gate.requests, 2).await;
            assert_pending(&page.evaluate("beforeunloadProbe.snapshot()").await);
            gate.release.add_permits(1);
            let committed = page.evaluate("beforeunloadProbe.waitForLoad()").await;
            assert_eq!(committed["loads"], 1, "{method}/{action}: {committed}");
            assert_eq!(committed["url"], "?next");
            assert_eq!(
                committed["events"],
                json!([
                    "beforeunload",
                    "child:beforeunload",
                    "pagehide",
                    "unload",
                    "child:pagehide",
                    "child:unload"
                ]),
                "{method}/{action}: {committed}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_beforeunload_removal_cancels_fetch_without_checking_retired_descendants() {
    for method in ["href", "navigation", "src"] {
        let (mut page, gate) = beforeunload_page("html").await;
        setup(&mut page, method, "remove").await;
        page.evaluate("beforeunloadProbe.start()").await;
        let removed = page
            .evaluate("new Promise(r => setTimeout(r, 100)).then(beforeunloadProbe.snapshot)")
            .await;
        assert_eq!(gate.requests.load(Ordering::SeqCst), 1, "{method}");
        assert_eq!(removed["loads"], 0);
        assert_eq!(removed["sameDocument"], false);
        assert_eq!(removed["nestedAlive"], false);
        assert_eq!(
            removed["events"],
            json!([
                "beforeunload",
                "pagehide",
                "unload",
                "child:pagehide",
                "child:unload"
            ]),
            "{method}: {removed}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_beforeunload_can_navigate_a_loading_parent_without_reentering_itself() {
    let routes = axum::Router::new().route(
        "/loading-parent.html",
        axum::routing::get(|| async { axum::response::Html(LOADING_PARENT) }),
    );
    let mut page = SameDocumentPage::with_routes(routes).await;
    let trace = page
        .evaluate(
            r#"(async () => {
        globalThis.loadingBeforeunloadTrace = null;
        const frame = document.createElement('iframe');
        frame.src = '/loading-parent.html';
        document.body.append(frame);
        for (let i = 0; i < 500 && loadingBeforeunloadTrace === null; i++)
          await new Promise(resolve => setTimeout(resolve, 10));
        const trace = loadingBeforeunloadTrace && loadingBeforeunloadTrace.slice();
        frame.remove();
        return trace;
    })()"#,
        )
        .await;
    assert_eq!(
        trace,
        json!([["start", "loading"], "child", "parent", "after"])
    );
}

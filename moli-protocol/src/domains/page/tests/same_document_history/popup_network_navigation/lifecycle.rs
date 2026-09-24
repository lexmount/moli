use super::*;

const PROBE: &str = include_str!("lifecycle.js");

async fn lifecycle_page(response: &'static str) -> (SameDocumentPage, Arc<ResponseGate>) {
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let observed = gate.clone();
    let routes = axum::Router::new().route(
        "/popup-unload.html",
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
                    "<!doctype html><body>popup</body>",
                )
                    .into_response();
                if visit == 2 && response == "attachment" {
                    result.headers_mut().insert(
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_static("attachment; filename=popup.txt"),
                    );
                }
                result
            }
        }),
    );
    (SameDocumentPage::with_routes(routes).await, gate)
}

async fn setup(page: &mut SameDocumentPage, method: &str, action: &str, destination: &str) {
    page.evaluate(&format!(
        "({PROBE})({method:?}, {action:?}, {destination:?}).then(probe => {{globalThis.popupLifecycle = probe}})"
    ))
    .await;
}

fn beforeunload_events() -> serde_json::Value {
    json!([
        "beforeunload",
        "child:beforeunload",
        "grandchild:beforeunload"
    ])
}

fn completed_events() -> serde_json::Value {
    json!([
        "beforeunload",
        "child:beforeunload",
        "grandchild:beforeunload",
        "pagehide",
        "unload",
        "child:pagehide",
        "child:unload",
        "grandchild:pagehide",
        "grandchild:unload"
    ])
}

fn assert_pending(result: &serde_json::Value) {
    assert_eq!(result["events"], beforeunload_events(), "{result}");
    assert_eq!(
        result["details"],
        json!(vec![vec![true, true, true, false]; 3])
    );
    assert_eq!(result["sameDocument"], true, "{result}");
    assert_eq!(result["nestedAlive"], true, "{result}");
    assert_eq!(result["oldHidden"], false, "{result}");
    assert_eq!(result["url"], "/popup-unload.html?initial", "{result}");
}

fn assert_committed(result: &serde_json::Value) {
    assert_eq!(result["events"], completed_events(), "{result}");
    assert_eq!(result["sameDocument"], false, "{result}");
    assert_eq!(result["nestedAlive"], false, "{result}");
    assert_eq!(result["oldHidden"], true, "{result}");
    assert_eq!(result["closed"], false, "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_navigation_checks_descendants_before_fetch_and_unloads_only_at_commit() {
    for method in [
        "href",
        "assign",
        "replace",
        "reload",
        "navigation",
        "named",
        "anchor",
        "form",
    ] {
        let (mut page, gate) = lifecycle_page("html").await;
        setup(&mut page, method, "none", "network").await;
        page.evaluate("popupLifecycle.start()").await;
        ResponseGate::wait_for(&gate.requests, 2).await;
        assert_pending(&page.evaluate("popupLifecycle.snapshot()").await);
        gate.release.add_permits(1);
        let committed = page.evaluate("popupLifecycle.waitForCommit()").await;
        assert_committed(&committed);
        assert_eq!(
            committed["url"],
            if method == "reload" {
                "/popup-unload.html?initial"
            } else {
                "/popup-unload.html?step=next"
            },
            "{method}: {committed}"
        );
        assert_eq!(gate.requests.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_ignored_responses_keep_documents_live_and_repeat_beforeunload_on_retry() {
    for response in ["204", "205", "attachment"] {
        for method in ["href", "navigation", "named", "form"] {
            let (mut page, gate) = lifecycle_page(response).await;
            page.allow_downloads().await;
            setup(&mut page, method, "none", "network").await;
            page.evaluate("popupLifecycle.start()").await;
            ResponseGate::wait_for(&gate.requests, 2).await;
            let pending = page.evaluate("popupLifecycle.snapshot()").await;
            assert_pending(&pending);
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 2).await;
            let ignored = page
                .evaluate("new Promise(r => setTimeout(r, 100)).then(popupLifecycle.snapshot)")
                .await;
            assert_eq!(ignored, pending, "{response}/{method}");
            page.evaluate("popupLifecycle.start()").await;
            let committed = page.evaluate("popupLifecycle.waitForCommit()").await;
            let mut events = beforeunload_events().as_array().unwrap().clone();
            events.extend(completed_events().as_array().unwrap().iter().cloned());
            assert_eq!(
                committed["events"],
                json!(events),
                "{response}/{method}: {committed}"
            );
            assert_eq!(committed["sameDocument"], false);
            assert_eq!(committed["oldHidden"], true);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_unload_guards_cover_descendant_writes_and_navigation_reentry() {
    for action in [
        "writes",
        "self",
        "ancestor",
        "named-ancestor",
        "named-anchor",
        "named-form",
        "stop",
    ] {
        let (mut page, gate) = lifecycle_page("html").await;
        setup(&mut page, "href", action, "network").await;
        page.evaluate("popupLifecycle.start()").await;
        ResponseGate::wait_for(&gate.requests, 2).await;
        assert_pending(&page.evaluate("popupLifecycle.snapshot()").await);
        gate.release.add_permits(1);
        let result = page.evaluate("popupLifecycle.waitForCommit()").await;
        assert_committed(&result);
        assert_eq!(
            result["url"], "/popup-unload.html?step=next",
            "{action}: {result}"
        );
        if action == "writes" {
            let writes = result["writes"].as_array().unwrap();
            assert_eq!(writes.len(), 9, "{result}");
            assert!(
                writes
                    .iter()
                    .all(|entry| entry.as_array().unwrap()[1..].iter().all(|ok| *ok == true)),
                "{result}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_about_blank_navigation_and_close_unload_each_document_once() {
    for method in ["href", "navigation", "named", "close"] {
        let (mut page, gate) = lifecycle_page("html").await;
        setup(&mut page, method, "writes", "blank").await;
        page.evaluate("popupLifecycle.start()").await;
        let result = page.evaluate("popupLifecycle.waitForCommit()").await;
        assert_eq!(result["events"], completed_events(), "{method}: {result}");
        assert_eq!(result["closed"], method == "close");
        assert_eq!(gate.requests.load(Ordering::SeqCst), 1);
        if method != "close" {
            assert_eq!(result["url"], "about:blank");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_history_traversal_checks_descendants_without_duplicate_beforeunload() {
    for method in ["back", "navigation-back"] {
        let (mut page, gate) = lifecycle_page("html").await;
        setup(&mut page, method, "writes", "history").await;
        page.evaluate("popupLifecycle.start()").await;
        let result = page.evaluate("popupLifecycle.waitForCommit()").await;
        assert_committed(&result);
        assert_eq!(result["url"], "/history.html?previous");
        assert_eq!(gate.requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_close_from_unload_cancels_replacement_and_keeps_task_microtasks() {
    for action in ["close-before", "close-pagehide"] {
        let (mut page, gate) = lifecycle_page("html").await;
        setup(&mut page, "href", action, "network").await;
        page.evaluate("popupLifecycle.start()").await;
        if action == "close-pagehide" {
            ResponseGate::wait_for(&gate.requests, 2).await;
            gate.release.add_permits(1);
        }
        let result = page.evaluate("popupLifecycle.waitForCommit()").await;
        assert_eq!(result["closed"], true, "{action}: {result}");
        assert_eq!(result["microtasks"], json!(["close"]), "{result}");
        let expected = if action == "close-before" {
            json!([
                "beforeunload",
                "beforeunload",
                "child:beforeunload",
                "grandchild:beforeunload",
                "pagehide",
                "unload",
                "child:pagehide",
                "child:unload",
                "grandchild:pagehide",
                "grandchild:unload"
            ])
        } else {
            json!([
                "beforeunload",
                "child:beforeunload",
                "grandchild:beforeunload",
                "pagehide",
                "beforeunload",
                "child:beforeunload",
                "grandchild:beforeunload",
                "unload",
                "child:pagehide",
                "child:unload",
                "grandchild:pagehide",
                "grandchild:unload"
            ])
        };
        assert_eq!(result["events"], expected, "{action}: {result}");
        assert_eq!(
            gate.requests.load(Ordering::SeqCst),
            if action == "close-before" { 1 } else { 2 }
        );
    }
}

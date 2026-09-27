use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn response_gate() -> Arc<ResponseGate> {
    Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    })
}

fn gated_route(gate: &Arc<ResponseGate>, status: u16) -> axum::routing::MethodRouter {
    let gate = gate.clone();
    axum::routing::get(move || {
        let gate = gate.clone();
        async move {
            gate.requests.fetch_add(1, Ordering::SeqCst);
            gate.release.acquire().await.unwrap().forget();
            gate.responses.fetch_add(1, Ordering::SeqCst);
            (
                axum::http::StatusCode::from_u16(status).unwrap(),
                [
                    (axum::http::header::CONTENT_TYPE, "text/html"),
                    (axum::http::header::CACHE_CONTROL, "no-store"),
                ],
                if status == 200 { "<!doctype html>" } else { "" },
            )
        }
    })
}

fn is_console(message: &serde_json::Value) -> bool {
    message["sessionId"] == SESSION
        && message["method"] == "Runtime.consoleAPICalled"
        && message["params"]["args"][0]["value"] == "pending-navigation-console"
}

fn is_exception(message: &serde_json::Value) -> bool {
    message["sessionId"] == SESSION
        && message["method"] == "Runtime.exceptionThrown"
        && message["params"]["exceptionDetails"]["exception"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("pending-navigation-exception"))
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_navigation_delivers_current_document_inspector_notifications() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for status in [200, 204, 205] {
                for network_enabled in [false, true] {
                    let signal = response_gate();
                    let destination = response_gate();
                    let routes = axum::Router::new()
                        .route("/signal", gated_route(&signal, 200))
                        .route("/destination", gated_route(&destination, status));
                    let mut page = SameDocumentPage::with_routes(routes).await;
                    page.ctx.enable_background_navigation_scheduler_for_test();
                    page.command("Runtime.enable", json!({})).await;
                    if network_enabled {
                        page.command("Network.enable", json!({})).await;
                    }
                    assert_eq!(
                        page.evaluate(
                            "fetch('/signal').then(r => r.text()).then(() => {\
                                console.log('pending-navigation-console');\
                                setTimeout(() => { throw new Error('pending-navigation-exception'); }, 0);\
                            }); 'armed'",
                        )
                        .await,
                        "armed"
                    );
                    ResponseGate::wait_for(&signal.requests, 1).await;
                    page.ctx.sent.clear();
                    page.ctx
                        .process_async(json!({
                            "id": 9001,
                            "sessionId": SESSION,
                            "method": "Page.navigate",
                            "params": {"url": page.base_url.replace("history.html", "destination")},
                        }))
                        .await;
                    ResponseGate::wait_for(&destination.requests, 1).await;
                    // No Inspector command follows this release: its response
                    // could otherwise flush the suspended notification prefix
                    // and hide the missing asynchronous delivery.
                    signal.release.add_permits(1);
                    wait_until_scheduler_message(
                        &mut page.ctx,
                        "current document console while navigation awaits its response",
                        is_console,
                    )
                    .await;
                    wait_until_scheduler_message(
                        &mut page.ctx,
                        "current document exception while navigation awaits its response",
                        is_exception,
                    )
                    .await;
                    assert_eq!(destination.responses.load(Ordering::SeqCst), 0);
                    assert!(!page.ctx.sent.iter().any(|message| message["id"] == 9001));
                    assert!(!page.ctx.sent.iter().any(|message| matches!(
                        message["method"].as_str(),
                        Some("Page.frameNavigated" | "Runtime.executionContextDestroyed" | "Runtime.executionContextsCleared")
                    )));
                    destination.release.add_permits(1);
                    wait_until_message(&mut page.ctx, SESSION, "navigation result", |message| {
                        message["id"] == 9001
                    })
                    .await;
                    let response = take_response_by_id(&mut page.ctx, 9001);
                    assert!(response["error"].is_null(), "{response}");
                    if status == 200 {
                        assert!(response["result"]["errorText"].is_null(), "{response}");
                    } else {
                        assert_eq!(response["result"]["errorText"], "net::ERR_ABORTED");
                    }
                    wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
                    page.evaluate("42").await;
                    assert_eq!(page.ctx.sent.iter().filter(|message| is_console(message)).count(), 1);
                    assert_eq!(page.ctx.sent.iter().filter(|message| is_exception(message)).count(), 1);
                }
            }
        })
        .await;
}

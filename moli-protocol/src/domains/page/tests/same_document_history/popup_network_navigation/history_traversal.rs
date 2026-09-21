use super::*;

async fn traversal_page(response: &'static str) -> (SameDocumentPage, Arc<ResponseGate>) {
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let observed = gate.clone();
    let routes = axum::Router::new().route(
        "/traversal.html",
        axum::routing::get(move || {
            let gate = observed.clone();
            async move {
                use axum::response::IntoResponse;
                let visit = gate.requests.fetch_add(1, Ordering::SeqCst) + 1;
                if visit > 1 {
                    gate.release.acquire().await.unwrap().forget();
                }
                gate.responses.fetch_add(1, Ordering::SeqCst);
                if visit > 1 {
                    match response {
                        "204" => return axum::http::StatusCode::NO_CONTENT.into_response(),
                        "205" => return axum::http::StatusCode::RESET_CONTENT.into_response(),
                        _ => {}
                    }
                }
                let mut result = (
                    [(axum::http::header::CONTENT_TYPE, "text/html")],
                    r#"<!doctype html><title>Traversal target</title><script>
                    onpageshow = event => opener.traversalPageShown(event.persisted);
                    </script>"#,
                )
                    .into_response();
                if visit > 1 && response == "attachment" {
                    result.headers_mut().insert(
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_static("attachment; filename=traversal.txt"),
                    );
                }
                result
            }
        }),
    );
    (SameDocumentPage::with_routes(routes).await, gate)
}

const SETUP_TRAVERSAL: &str = r#"(async () => {
    globalThis.shown = [];
    globalThis.traversalPageShown = persisted => shown.push(persisted);
    globalThis.popup = open('traversal.html', 'history-traversal');
    await new Promise(resolve => popup.addEventListener('load', resolve, {once:true}));
    await new Promise(resolve => setTimeout(resolve, 0));
    if (shown.length !== 1 || shown[0] !== false) throw new Error('missing initial pageshow');
    popup.navigation.updateCurrentEntry({state: {original: true}});
    const entry = popup.navigation.currentEntry;
    globalThis.destination = {url: entry.url, key: entry.key, id: entry.id};
    const firstDocument = popup.document;
    popup.addEventListener('unload', () => {});
    popup.location.href = 'history.html?second-popup';
    for (let i = 0; popup.document === firstDocument && i < 500; ++i)
        await new Promise(resolve => setTimeout(resolve, 10));
    if (popup.document === firstDocument) throw new Error('second Document never committed');
    await new Promise(resolve => setTimeout(resolve, 0));
    globalThis.popupDocument = popup.document;
    globalThis.popupNavigation = popup.navigation;
    globalThis.PopupDOMException = popup.DOMException;
    globalThis.events = [];
    globalThis.signals = [];
    globalThis.cancellationFlags = [];
    globalThis.reasons = [];
    globalThis.states = {};
    globalThis.details = [];
    globalThis.info = {token: 42};
    for (const type of ['beforeunload', 'pagehide', 'unload'])
        popup.addEventListener(type, () => {
            events.push(type);
            if (type === 'pagehide' && ACTION === 'close-pagehide') popup.close();
        });
    popupNavigation.addEventListener('navigate', event => {
        events.push('navigate');
        signals.push(event.signal);
        event.signal.addEventListener('abort', () => {
            events.push('abort');
            cancellationFlags.push(event.defaultPrevented);
        });
        const detail = {
            type: event.navigationType, cancelable: event.cancelable,
            canIntercept: event.canIntercept, userInitiated: event.userInitiated,
            hashChange: event.hashChange, sameDocument: event.destination.sameDocument,
            urlMatches: event.destination.url === destination.url,
            keyMatches: event.destination.key === destination.key,
            idMatches: event.destination.id === destination.id,
            index: event.destination.index, state: event.destination.getState(),
            infoMatches: event.info === (METHOD === 'history-back' ? undefined : info),
            formData: event.formData, downloadRequest: event.downloadRequest,
            sourceElement: event.sourceElement,
        };
        details.push(detail);
        if (ACTION === 'prevent') {
            event.preventDefault();
            detail.prevented = event.defaultPrevented;
        }
        if (ACTION === 'intercept') {
            try { event.intercept(); detail.intercept = 'allowed'; }
            catch (error) { detail.intercept = error.name; }
        }
        if (ACTION === 'stop-dispatch') popup.stop();
        if (ACTION === 'close-dispatch') popup.close();
    });
    popupNavigation.addEventListener('navigateerror', event => {
        events.push('error:' + event.error.name); reasons.push(event.error);
    });
    popupNavigation.addEventListener('navigatesuccess', () => events.push('success'));
    globalThis.traversalSnapshot = () => ({
        closed: popup.closed, sameDocument: popup.document === popupDocument,
        href: popup.location.href, events: [...events], details: [...details],
        index: popup.closed ? null : popup.navigation.currentEntry.index,
        length: popup.closed ? null : popup.history.length,
        states: {...states}, aborted: signals.map(signal => signal.aborted),
        cancellationFlags: [...cancellationFlags],
        errorIdentity: reasons.every(error => error === signals[0].reason && error instanceof PopupDOMException),
        reasons: reasons.length, shown: [...shown],
    });
    globalThis.beginTraversal = () => {
        let result;
        if (METHOD === 'history-back') popup.history.back();
        else if (METHOD === 'back') result = popupNavigation.back({info});
        else result = popupNavigation.traverseTo(destination.key, {info});
        if (result) for (const key of ['committed', 'finished']) result[key].then(
            () => states[key] = 'fulfilled',
            error => { states[key] = error.name; reasons.push(error); }
        );
        return traversalSnapshot();
    };
    return traversalSnapshot();
})()"#;

async fn setup_traversal(
    page: &mut SameDocumentPage,
    method: &str,
    action: &str,
) -> serde_json::Value {
    page.evaluate(
        &SETUP_TRAVERSAL
            .replace("METHOD", &json!(method).to_string())
            .replace("ACTION", &json!(action).to_string()),
    )
    .await
}

fn assert_traverse_event(snapshot: &serde_json::Value, action: &str) {
    let mut expected = json!({
        "type": "traverse", "cancelable": false, "canIntercept": false,
        "userInitiated": false, "hashChange": false, "sameDocument": false,
        "urlMatches": true, "keyMatches": true, "idMatches": true,
        "index": 0, "state": {"original": true}, "infoMatches": true,
        "formData": null, "downloadRequest": null, "sourceElement": null,
    });
    if action == "prevent" {
        expected["prevented"] = json!(false);
    }
    if action == "intercept" {
        expected["intercept"] = json!("SecurityError");
    }
    assert_eq!(snapshot["details"], json!([expected]), "{snapshot}");
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_cross_document_history_traversal_events_and_cancellation() {
    for method in ["back", "traverseTo", "history-back"] {
        for action in [
            "none",
            "prevent",
            "intercept",
            "stop-dispatch",
            "stop-pending",
            "close-pending",
        ] {
            let (mut page, gate) = traversal_page("html").await;
            let initial = setup_traversal(&mut page, method, action).await;
            let started = page.evaluate("beginTraversal()").await;
            assert_eq!(started["events"], json!([]), "{method}/{action}: {started}");
            assert_eq!(started["index"], 1);
            ResponseGate::wait_for(&gate.requests, 2).await;
            let pending = page.evaluate("traversalSnapshot()").await;
            assert_traverse_event(&pending, action);
            assert_eq!(pending["sameDocument"], true, "{pending}");
            assert_eq!(pending["href"], initial["href"]);
            assert_eq!(pending["index"], 1);
            assert_eq!(pending["length"], 2);
            if action != "stop-dispatch" {
                assert_eq!(pending["events"], json!(["beforeunload", "navigate"]));
                assert_eq!(pending["states"], json!({}));
                assert_eq!(pending["aborted"], json!([false]));
            }
            match action {
                "stop-pending" => {
                    page.evaluate("popup.stop(); void 0").await;
                }
                "close-pending" => {
                    page.evaluate("popup.close(); void 0").await;
                }
                _ => {}
            }
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 2).await;
            let snapshot = if action == "close-pending" {
                page.evaluate(
                    "new Promise(r => setTimeout(r, 100)).then(() => traversalSnapshot())",
                )
                .await
            } else {
                page.evaluate(
                    r#"(async () => {
                    for (let i = 0; shown.length < 2 && i < 500; ++i)
                        await new Promise(resolve => setTimeout(resolve, 10));
                    return traversalSnapshot();
                })()"#,
                )
                .await
            };
            let aborted = action.starts_with("stop") || action.starts_with("close");
            let mut events = vec!["beforeunload", "navigate"];
            if action == "close-pending" {
                events.push("beforeunload");
            }
            if aborted {
                events.extend(["abort", "error:AbortError"]);
            }
            events.extend(["pagehide", "unload"]);
            assert_eq!(
                snapshot["events"],
                json!(events),
                "{method}/{action}: {snapshot}"
            );
            assert_eq!(snapshot["aborted"], json!([aborted]), "{snapshot}");
            assert_eq!(
                snapshot["cancellationFlags"],
                if aborted {
                    json!([action == "stop-dispatch"])
                } else {
                    json!([])
                },
                "{method}/{action}: {snapshot}"
            );
            assert_eq!(snapshot["errorIdentity"], true, "{snapshot}");
            let states = if aborted && method != "history-back" {
                json!({"committed": "AbortError", "finished": "AbortError"})
            } else {
                json!({})
            };
            assert_eq!(snapshot["states"], states, "{snapshot}");
            assert_eq!(
                snapshot["reasons"],
                if aborted {
                    if method == "history-back" { 1 } else { 3 }
                } else {
                    0
                }
            );
            if action == "close-pending" {
                assert_eq!(snapshot["closed"], true);
                assert_eq!(snapshot["shown"], json!([false]), "{snapshot}");
            } else {
                assert_eq!(snapshot["closed"], false);
                assert_eq!(snapshot["sameDocument"], false, "{snapshot}");
                assert_eq!(snapshot["index"], 0);
                assert_eq!(snapshot["length"], 2);
                assert_eq!(snapshot["shown"], json!([false, false]));
                let entry = page.evaluate("({key: popup.navigation.currentEntry.key === destination.key, state: popup.navigation.currentEntry.getState()})").await;
                assert_eq!(entry, json!({"key": true, "state": {"original": true}}));
                page.evaluate("popup.close(); void 0").await;
            }
            page.assert_history(&[""], 0).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_cross_document_history_traversal_ignored_responses_remain_pending() {
    for response in ["204", "205", "attachment"] {
        let (mut page, gate) = traversal_page(response).await;
        page.allow_downloads().await;
        let initial = setup_traversal(&mut page, "back", "none").await;
        page.evaluate("beginTraversal()").await;
        ResponseGate::wait_for(&gate.requests, 2).await;
        gate.release.add_permits(1);
        ResponseGate::wait_for(&gate.responses, 2).await;
        let pending = page
            .evaluate("new Promise(r => setTimeout(r, 100)).then(() => traversalSnapshot())")
            .await;
        assert_traverse_event(&pending, "none");
        assert_eq!(
            pending["events"],
            json!(["beforeunload", "navigate"]),
            "{response}: {pending}"
        );
        assert_eq!(pending["states"], json!({}));
        assert_eq!(pending["aborted"], json!([false]));
        assert_eq!(pending["sameDocument"], true);
        assert_eq!(pending["href"], initial["href"]);
        assert_eq!(pending["index"], 1);
        assert_eq!(pending["shown"], json!([false]));
        let stopped = page
            .evaluate("popup.stop(); Promise.resolve().then(() => traversalSnapshot())")
            .await;
        assert_eq!(
            stopped["states"],
            json!({"committed": "AbortError", "finished": "AbortError"}),
            "{stopped}"
        );
        assert_eq!(stopped["aborted"], json!([true]));
        assert_eq!(stopped["errorIdentity"], true);
        page.evaluate("popup.close(); void 0").await;
        page.assert_history(&[""], 0).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_cross_document_history_traversal_close_during_callbacks() {
    for action in ["close-dispatch", "close-pagehide"] {
        let (mut page, gate) = traversal_page("html").await;
        setup_traversal(&mut page, "back", action).await;
        page.evaluate("beginTraversal()").await;
        if action == "close-pagehide" {
            ResponseGate::wait_for(&gate.requests, 2).await;
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 2).await;
        }
        let snapshot = page
            .evaluate("new Promise(r => setTimeout(r, 100)).then(() => traversalSnapshot())")
            .await;
        assert_eq!(snapshot["closed"], true, "{action}: {snapshot}");
        assert_traverse_event(&snapshot, action);
        if action == "close-dispatch" {
            assert_eq!(
                snapshot["events"],
                json!([
                    "beforeunload",
                    "navigate",
                    "beforeunload",
                    "abort",
                    "error:AbortError",
                    "pagehide",
                    "unload"
                ]),
                "{snapshot}"
            );
            assert_eq!(snapshot["aborted"], json!([true]));
            assert_eq!(
                snapshot["states"],
                json!({"committed": "AbortError", "finished": "AbortError"})
            );
            assert_eq!(snapshot["errorIdentity"], true);
            assert_eq!(snapshot["reasons"], 3);
            assert_eq!(gate.requests.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(
                snapshot["events"],
                json!([
                    "beforeunload",
                    "navigate",
                    "pagehide",
                    "beforeunload",
                    "unload"
                ]),
                "{snapshot}"
            );
            assert_eq!(snapshot["aborted"], json!([false]));
            assert_eq!(snapshot["states"], json!({}));
            assert_eq!(snapshot["reasons"], 0);
        }
        page.assert_history(&[""], 0).await;
    }
}

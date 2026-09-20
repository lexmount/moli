use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

mod history_traversal;

async fn gated_page() -> (SameDocumentPage, Arc<ResponseGate>) {
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let observed = gate.clone();
    let routes = axum::Router::new().route(
        "/popup-response.html",
        axum::routing::get(move |uri: axum::http::Uri| {
            let gate = observed.clone();
            async move {
                use axum::response::IntoResponse;
                let visit = gate.requests.fetch_add(1, Ordering::SeqCst) + 1;
                let query = uri.query().unwrap_or("");
                // A reload first commits this URL, then waits at the same gate.
                if !(query.contains("reload") && visit == 1) {
                    gate.release.acquire().await.unwrap().forget();
                }
                gate.responses.fetch_add(1, Ordering::SeqCst);
                if query.contains("redirect") {
                    return axum::response::Redirect::to("/history.html?redirected")
                        .into_response();
                }
                if query.contains("204") {
                    return axum::http::StatusCode::NO_CONTENT.into_response();
                }
                if query.contains("205") {
                    return axum::http::StatusCode::RESET_CONTENT.into_response();
                }
                let mut response = (
                    [(axum::http::header::CONTENT_TYPE, "text/html")],
                    format!("<!doctype html><title>Popup response {visit}</title><p>Committed</p>"),
                )
                    .into_response();
                if query.contains("attachment") {
                    response.headers_mut().insert(
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_static("attachment; filename=popup.txt"),
                    );
                }
                response
            }
        }),
    );
    (SameDocumentPage::with_routes(routes).await, gate)
}

const SETUP: &str = r#"(async () => {
    globalThis.popup = open(INITIAL_URL, 'network-popup');
    await new Promise(resolve => popup.addEventListener('load', resolve, {once:true}));
    await new Promise(resolve => setTimeout(resolve, 0));
    globalThis.popupDocument = popup.document;
    globalThis.popupNavigation = popup.navigation;
    globalThis.popupEntry = popupNavigation.currentEntry;
    globalThis.popupEvents = [];
    globalThis.popupStates = {};
    globalThis.popupReasons = [];
    globalThis.popupSignals = [];
    globalThis.PopupDOMException = popup.DOMException;
    globalThis.popupInitial = {href: popup.location.href, length: popup.history.length, index: popupEntry.index};
    globalThis.observePopupResult = result => {
        for (const key of ['committed', 'finished']) result[key].then(
            () => popupStates[key] = 'fulfilled',
            error => { popupStates[key] = error.name; popupReasons.push(error); }
        );
    };
    popupNavigation.addEventListener('navigate', event => {
        popupEvents.push('navigate:' + event.navigationType);
        popupSignals.push(event.signal);
        event.signal.addEventListener('abort', () => popupEvents.push('abort'));
    });
    popupNavigation.addEventListener('navigateerror', event => {
        popupEvents.push('error:' + event.error.name); popupReasons.push(event.error);
    });
    popupNavigation.addEventListener('navigatesuccess', () => popupEvents.push('success'));
    globalThis.popupSnapshot = () => ({
        closed: popup.closed,
        href: popup.location.href,
        documentURL: popup.document.URL,
        sameDocument: popup.document === popupDocument,
        sameEntry: popupNavigation.currentEntry === popupEntry,
        currentURL: popup.navigation.currentEntry?.url,
        length: popup.closed ? null : popup.history.length,
        index: popup.navigation.currentEntry?.index,
        title: popup.document.title,
        events: [...popupEvents], states: {...popupStates},
        aborted: popupSignals.map(signal => signal.aborted),
        sameError: popupReasons.every(error => error === popupSignals[0].reason && error instanceof PopupDOMException),
    });
    return popupInitial;
})()"#;

async fn start_popup(page: &mut SameDocumentPage, reload: bool) -> serde_json::Value {
    page.evaluate(&SETUP.replace(
        "INITIAL_URL",
        if reload {
            "'popup-response.html?reload'"
        } else {
            "'history.html?initial-popup'"
        },
    ))
    .await
}

fn assert_uncommitted(result: &serde_json::Value, initial: &serde_json::Value) {
    assert_eq!(result["sameDocument"], true, "{result}");
    assert_eq!(result["sameEntry"], true, "{result}");
    for key in ["href", "documentURL", "currentURL"] {
        assert_eq!(result[key], initial["href"], "{key}: {result}");
    }
    for key in ["length", "index"] {
        assert_eq!(result[key], initial[key], "{key}: {result}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_network_navigation_commits_only_after_response_and_can_be_stopped() {
    for operation in [
        "navigate-push",
        "navigate-replace",
        "location-assign",
        "location-replace",
        "reload",
    ] {
        for action in ["commit", "stop", "borrow-stop", "close"] {
            let (mut page, gate) = gated_page().await;
            let reload = operation == "reload";
            let initial = start_popup(&mut page, reload).await;
            let command = match operation {
                "navigate-push" => {
                    "observePopupResult(popupNavigation.navigate('popup-response.html', {history:'push'}))"
                }
                "navigate-replace" => {
                    "observePopupResult(popupNavigation.navigate('popup-response.html', {history:'replace'}))"
                }
                "location-assign" => "popup.location.assign('popup-response.html')",
                "location-replace" => "popup.location.replace('popup-response.html')",
                "reload" => "observePopupResult(popupNavigation.reload())",
                _ => unreachable!(),
            };
            let started = page.evaluate(&format!("{command}; popupSnapshot()")).await;
            assert_uncommitted(&started, &initial);
            let kind = if reload {
                "reload"
            } else if operation.ends_with("replace") {
                "replace"
            } else {
                "push"
            };
            assert_eq!(
                started["events"],
                json!([format!("navigate:{kind}")]),
                "{operation}/{action}: {started}"
            );
            assert_eq!(started["aborted"], json!([false]));
            assert_eq!(started["states"], json!({}));
            let requests = if reload { 2 } else { 1 };
            ResponseGate::wait_for(&gate.requests, requests).await;
            let while_pending = page.evaluate("popupSnapshot()").await;
            assert_uncommitted(&while_pending, &initial);
            match action {
                "stop" => {
                    page.evaluate("popup.stop(); void 0").await;
                }
                "borrow-stop" => {
                    page.evaluate("window.stop.call(popup); void 0").await;
                }
                "close" => {
                    page.evaluate("popup.close(); void 0").await;
                }
                _ => {}
            }
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, requests).await;
            let result = page.evaluate(if action == "commit" {
                "(async () => { for(let n=0;n<500 && popup.document===popupDocument;n++)await new Promise(r=>setTimeout(r,5));return popupSnapshot(); })()"
            } else {
                "new Promise(r=>setTimeout(r,100)).then(()=>popupSnapshot())"
            }).await;
            if action == "commit" {
                assert_eq!(result["sameDocument"], false, "{operation}: {result}");
                let expected_length =
                    initial["length"].as_u64().unwrap() + u64::from(kind == "push");
                assert_eq!(result["length"], expected_length, "{operation}: {result}");
                assert_eq!(result["currentURL"], result["href"]);
                assert_eq!(result["documentURL"], result["href"]);
                assert_eq!(result["title"], format!("Popup response {requests}"));
                assert_eq!(result["aborted"], json!([false]));
                // The old realm's method promises intentionally remain pending.
                assert_eq!(result["states"], json!({}));
            } else {
                if action == "close" {
                    assert_eq!(result["closed"], true, "{operation}: {result}");
                    assert_eq!(result["sameDocument"], true);
                } else {
                    assert_uncommitted(&result, &initial);
                }
                assert_eq!(
                    result["aborted"],
                    json!([true]),
                    "{operation}/{action}: {result}"
                );
                assert_eq!(
                    result["events"],
                    json!([
                        format!("navigate:{kind}"),
                        "abort".to_owned(),
                        "error:AbortError".to_owned()
                    ])
                );
                assert_eq!(
                    result["states"],
                    if operation.starts_with("navigate") || reload {
                        json!({"committed":"AbortError","finished":"AbortError"})
                    } else {
                        json!({})
                    }
                );
                assert_eq!(result["sameError"], true, "{operation}/{action}: {result}");
            }
            page.evaluate("popup.close(); void 0").await;
            page.assert_history(&[""], 0).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_ignored_navigation_keeps_history_and_promises_until_superseded() {
    for response in ["204", "205", "attachment"] {
        for api in [true, false] {
            let (mut page, gate) = gated_page().await;
            page.allow_downloads().await;
            let initial = start_popup(&mut page, false).await;
            let target = json!(format!("popup-response.html?{response}"));
            page.evaluate(&if api {
                format!("observePopupResult(popupNavigation.navigate({target}));void 0")
            } else {
                format!("popup.location.href={target};void 0")
            })
            .await;
            ResponseGate::wait_for(&gate.requests, 1).await;
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 1).await;
            let ignored = page
                .evaluate("new Promise(r=>setTimeout(r,100)).then(()=>popupSnapshot())")
                .await;
            assert_uncommitted(&ignored, &initial);
            assert_eq!(ignored["states"], json!({}), "{response}/{api}: {ignored}");
            assert_eq!(ignored["events"], json!(["navigate:push"]));
            assert_eq!(ignored["aborted"], json!([false]));
            let next = page.evaluate("(async () => {await popupNavigation.navigate('#next').finished;return popupSnapshot();})()").await;
            assert_eq!(
                next["states"],
                if api {
                    json!({"committed":"AbortError","finished":"AbortError"})
                } else {
                    json!({})
                }
            );
            assert_eq!(next["aborted"], json!([true, false]));
            assert_eq!(next["sameError"], true);
            assert_eq!(next["sameDocument"], true);
            assert_eq!(
                next["events"],
                json!([
                    "navigate:push",
                    "abort",
                    "error:AbortError",
                    "navigate:push",
                    "success"
                ])
            );
            page.evaluate("popup.close(); void 0").await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_network_navigation_ignores_new_state_and_commits_redirect_url() {
    for operation in ["push", "replace", "reload", "redirect"] {
        let (mut page, gate) = gated_page().await;
        let initial = start_popup(&mut page, operation == "reload").await;
        let started = page.evaluate(&r#"(() => {
            popupNavigation.updateCurrentEntry({state:{previous:true}});
            popupNavigation.addEventListener('navigate',event=>globalThis.popupDestinationState=event.destination.getState(),{once:true});
            const state={map:new Map([['answer',42]]),bytes:new Uint8Array([7,8])};state.self=state;
            globalThis.suppliedState=state;
            const operation=OPERATION;
            observePopupResult(operation==='reload' ? popupNavigation.reload({state}) : popupNavigation.navigate('popup-response.html'+(operation==='redirect'?'?redirect':''),{state,history:operation==='replace'?'replace':'push'}));
            state.map.set('answer',99);state.bytes[0]=0;
            return popupSnapshot();
        })()"#.replace("OPERATION", &json!(operation).to_string())).await;
        assert_uncommitted(&started, &initial);
        let requests = if operation == "reload" { 2 } else { 1 };
        ResponseGate::wait_for(&gate.requests, requests).await;
        gate.release.add_permits(1);
        let result = page.evaluate(r#"(async () => {
            for(let n=0;n<500 && popup.document===popupDocument;n++) await new Promise(r=>setTimeout(r,5));
            const state=popup.navigation.currentEntry.getState(),destination=popupDestinationState;
            return {snapshot:popupSnapshot(),state,checks:{destination:!!destination,cycle:destination?.self===destination,map:destination?.map?.get('answer')===42,bytes:destination?.bytes?.[0]===7,clone:destination!==suppliedState,newStateIgnored:!state?.self}};
        })()"#).await;
        assert_eq!(
            result["snapshot"]["sameDocument"], false,
            "{operation}: {result}"
        );
        assert!(
            result["checks"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == true),
            "{operation}: {result}"
        );
        if operation == "reload" {
            assert_eq!(result["state"], json!({"previous":true}));
        } else {
            assert!(result["state"].is_null(), "{operation}: {result}");
        }
        assert_eq!(result["snapshot"]["href"], result["snapshot"]["currentURL"]);
        if operation == "redirect" {
            assert!(
                result["snapshot"]["href"]
                    .as_str()
                    .unwrap()
                    .ends_with("/history.html?redirected")
            );
        }
        page.evaluate("popup.close(); void 0").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_network_stop_rejects_reentrant_navigation_and_discards_late_response() {
    for listener in ["abort", "navigateerror"] {
        for operation in ["navigate", "fragment", "location", "reload"] {
            let (mut page, gate) = gated_page().await;
            let initial = start_popup(&mut page, false).await;
            page.evaluate(
                "observePopupResult(popupNavigation.navigate('popup-response.html'));void 0",
            )
            .await;
            ResponseGate::wait_for(&gate.requests, 1).await;
            let stopped = page.evaluate(&r#"(async () => {
                const reenter=()=>{
                    const operation=OPERATION;
                    if(operation==='location'){popup.location.href='history.html?reentrant';return;}
                    const result=operation==='reload'?popupNavigation.reload():popupNavigation.navigate(operation==='fragment'?'#reentrant':'history.html?reentrant');
                    for(const key of ['committed','finished'])result[key].then(()=>popupStates['reentrant'+key]='fulfilled',e=>popupStates['reentrant'+key]=e.name);
                };
                if(LISTENER==='abort')popupSignals[0].addEventListener('abort',reenter,{once:true});
                else popupNavigation.addEventListener('navigateerror',reenter,{once:true});
                popup.stop();await new Promise(r=>setTimeout(r,0));return popupSnapshot();
            })()"#.replace("OPERATION", &json!(operation).to_string()).replace("LISTENER", &json!(listener).to_string())).await;
            assert_uncommitted(&stopped, &initial);
            assert_eq!(
                stopped["events"],
                json!(["navigate:push", "abort", "error:AbortError"])
            );
            assert_eq!(stopped["aborted"], json!([true]));
            assert_eq!(stopped["sameError"], true);
            let mut expected = json!({"committed":"AbortError","finished":"AbortError"});
            if operation != "location" {
                expected["reentrantcommitted"] = json!("AbortError");
                expected["reentrantfinished"] = json!("AbortError");
            }
            assert_eq!(
                stopped["states"], expected,
                "{listener}/{operation}: {stopped}"
            );
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 1).await;
            let final_state = page
                .evaluate("new Promise(r=>setTimeout(r,100)).then(()=>popupSnapshot())")
                .await;
            assert_eq!(
                final_state, stopped,
                "late response after {listener}/{operation}"
            );
            // The stop guard must not suppress a later ordinary navigation.
            page.evaluate("popup.location.href='history.html?after-stop';void 0")
                .await;
            let recovered = page.evaluate("(async()=>{for(let n=0;n<500&&popup.document===popupDocument;n++)await new Promise(r=>setTimeout(r,5));return popupSnapshot();})()").await;
            assert_eq!(recovered["sameDocument"], false, "{recovered}");
            assert!(recovered["href"].as_str().unwrap().ends_with("?after-stop"));
            page.evaluate("popup.close();void 0").await;
        }
    }
}

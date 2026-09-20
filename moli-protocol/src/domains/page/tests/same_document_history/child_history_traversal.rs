use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

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
                    "<!doctype html><title>Traversal target</title><body>target</body>",
                )
                    .into_response();
                if visit == 2 && response == "attachment" {
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

const SETUP: &str = r#"(async () => {
 const id = "protocol", kind = "html", method = METHOD, nested = NESTED;
 const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
 const frame = document.createElement('iframe');
 frame.src = `traversal.html?id=${id}&kind=${kind}`;
 let loads = 0;
 frame.addEventListener('load', () => loads++);
 const loaded = new Promise(resolve => frame.onload = resolve);
 document.body.appendChild(frame);
 await loaded;
 await sleep(0);
 frame.contentWindow.navigation.updateCurrentEntry({state: {original:true}});
 const entry = frame.contentWindow.navigation.currentEntry;
 const destination = {url:entry.url, key:entry.key, id:entry.id};
 frame.contentWindow.onunload = () => {};
 const second = new Promise(resolve => frame.onload = resolve);
 frame.contentWindow.location.href = 'history.html?second-child';
 await second;
 await sleep(0);
 const win = frame.contentWindow, doc = frame.contentDocument, nav = win.navigation;
 let child, childDoc;
 if (nested) {
   child = doc.createElement('iframe');
   child.src = 'history.html?grandchild';
   const childLoaded = new Promise(resolve => child.onload = resolve);
   doc.body.appendChild(child);
   await childLoaded;
   childDoc = child.contentDocument;
 }
 const events = [], states = {}, signals = [], reasons = [], details = [];
 const Exception = win.DOMException;
 for (const type of ['beforeunload','pagehide','unload']) {
   win.addEventListener(type, () => events.push(type));
   if (child) child.contentWindow.addEventListener(type, () => events.push('child:' + type));
 }
 nav.addEventListener('navigate', event => {
   events.push('navigate');
   details.push({type:event.navigationType, sameDocument:event.destination.sameDocument,
    cancelable:event.cancelable, canIntercept:event.canIntercept,
    key:event.destination.key===destination.key, url:event.destination.url===destination.url});
   signals.push(event.signal);
   event.signal.addEventListener('abort', () => events.push('abort'));
 });
 nav.addEventListener('navigateerror', event => {events.push('error:'+event.error.name); reasons.push(event.error)});
 nav.addEventListener('navigatesuccess', () => events.push('success'));
 function snapshot() {
   return {events:[...events],states:{...states},aborted:signals.map(signal=>signal.aborted),
    sameDocument:frame.contentDocument===doc, nestedAlive:child?child.contentDocument===childDoc:null,
    index:frame.isConnected?frame.contentWindow.navigation.currentEntry.index:null,
    length:frame.isConnected?frame.contentWindow.history.length:null,
    url:frame.isConnected?frame.contentWindow.location.pathname:null,
    errorIdentity:reasons.every(error=>error===signals[0].reason && error instanceof Exception),
    reasons:reasons.length,loads,details:[...details]};
 }
 const initial=snapshot();
 function begin() {
   let result;
   if (method==='back') result=nav.back();
   else if (method==='traverseTo') result=nav.traverseTo(destination.key);
   else win.history.back();
   if(result) for(const key of ['committed','finished']) result[key].then(
    ()=>states[key]='fulfilled', error=>{states[key]=error.name;reasons.push(error)});
 }
 Object.assign(globalThis, {
   childFrame:frame, childWindow:win, childSnapshot:snapshot, beginChildTraversal:begin,
   waitForChildTraversal:async () => {
     for(let i=0;loads<3&&i<500;++i) await sleep(10);
     if(loads<3) throw new Error('traversal did not commit: '+JSON.stringify(snapshot()));
     return snapshot();
   }
 });
 return initial;
})()"#;

async fn setup(page: &mut SameDocumentPage, method: &str, nested: bool) -> serde_json::Value {
    page.evaluate(
        &SETUP
            .replace("METHOD", &json!(method).to_string())
            .replace("NESTED", &nested.to_string()),
    )
    .await
}

fn before_events(nested: bool) -> Vec<&'static str> {
    if nested {
        vec!["beforeunload", "child:beforeunload"]
    } else {
        vec!["beforeunload"]
    }
}

fn unload_events(nested: bool) -> Vec<&'static str> {
    if nested {
        vec!["pagehide", "unload", "child:pagehide", "child:unload"]
    } else {
        vec!["pagehide", "unload"]
    }
}

fn assert_pending(snapshot: &serde_json::Value, initial: &serde_json::Value, nested: bool) {
    let mut events = before_events(nested);
    events.push("navigate");
    assert_eq!(snapshot["events"], json!(events), "{snapshot}");
    assert_eq!(snapshot["sameDocument"], true);
    assert_eq!(
        snapshot["nestedAlive"],
        if nested { json!(true) } else { json!(null) }
    );
    assert_eq!(snapshot["index"], 1);
    assert_eq!(snapshot["url"], initial["url"]);
    assert_eq!(snapshot["length"], initial["length"]);
    assert_eq!(snapshot["states"], json!({}));
    assert_eq!(snapshot["aborted"], json!([false]));
    assert_eq!(snapshot["loads"], 2);
    assert_eq!(
        snapshot["details"],
        json!([{
            "type":"traverse", "sameDocument":false, "cancelable":false,
            "canIntercept":false, "key":true, "url":true,
        }])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_history_traversal_ignored_responses_preserve_documents_and_allow_retry() {
    for response in ["204", "205", "attachment"] {
        for method in ["back", "traverseTo", "history-back"] {
            for nested in [false, true] {
                let (mut page, gate) = traversal_page(response).await;
                page.allow_downloads().await;
                let initial = setup(&mut page, method, nested).await;
                page.evaluate("beginChildTraversal(); void 0").await;
                ResponseGate::wait_for(&gate.requests, 2).await;
                let pending = page.evaluate("childSnapshot()").await;
                assert_pending(&pending, &initial, nested);
                gate.release.add_permits(1);
                ResponseGate::wait_for(&gate.responses, 2).await;
                let ignored = page
                    .evaluate("new Promise(r => setTimeout(r, 100)).then(childSnapshot)")
                    .await;
                assert_eq!(ignored, pending, "{response}/{method}/{nested}");

                page.evaluate("beginChildTraversal(); void 0").await;
                let retried = page.evaluate("waitForChildTraversal()").await;
                let mut events = before_events(nested);
                events.push("navigate");
                events.extend(before_events(nested));
                events.extend(["abort", "error:AbortError", "navigate"]);
                events.extend(unload_events(nested));
                assert_eq!(
                    retried["events"],
                    json!(events),
                    "{response}/{method}/{nested}: {retried}"
                );
                assert_eq!(retried["sameDocument"], false);
                assert_eq!(
                    retried["nestedAlive"],
                    if nested { json!(false) } else { json!(null) }
                );
                assert_eq!(retried["index"], 0);
                assert_eq!(retried["length"], initial["length"]);
                assert_eq!(retried["loads"], 3);
                assert_eq!(retried["aborted"], json!([true, false]));
                assert_eq!(retried["errorIdentity"], true);
                assert_eq!(
                    retried["states"],
                    if method == "history-back" {
                        json!({})
                    } else {
                        json!({"committed":"AbortError", "finished":"AbortError"})
                    }
                );
                assert_eq!(
                    retried["reasons"],
                    if method == "history-back" { 1 } else { 3 }
                );
                page.evaluate("childFrame.remove(); void 0").await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_history_traversal_waits_for_response_and_survives_stop() {
    for method in ["back", "traverseTo", "history-back"] {
        for stop in [false, true] {
            let (mut page, gate) = traversal_page("html").await;
            let initial = setup(&mut page, method, true).await;
            page.evaluate("beginChildTraversal(); void 0").await;
            ResponseGate::wait_for(&gate.requests, 2).await;
            let pending = page.evaluate("childSnapshot()").await;
            assert_pending(&pending, &initial, true);
            if stop {
                page.evaluate("childWindow.stop(); void 0").await;
            }
            gate.release.add_permits(1);
            ResponseGate::wait_for(&gate.responses, 2).await;
            let committed = page.evaluate("waitForChildTraversal()").await;
            let mut events = before_events(true);
            events.push("navigate");
            if stop {
                events.extend(["abort", "error:AbortError"]);
            }
            events.extend(unload_events(true));
            assert_eq!(
                committed["events"],
                json!(events),
                "{method}/{stop}: {committed}"
            );
            assert_eq!(committed["sameDocument"], false);
            assert_eq!(committed["nestedAlive"], false);
            assert_eq!(committed["index"], 0);
            assert_eq!(committed["length"], initial["length"]);
            assert_eq!(committed["loads"], 3);
            assert_eq!(committed["aborted"], json!([stop]));
            assert_eq!(committed["errorIdentity"], true);
            assert_eq!(
                committed["states"],
                if stop && method != "history-back" {
                    json!({"committed":"AbortError", "finished":"AbortError"})
                } else {
                    json!({})
                }
            );
            page.evaluate("childFrame.remove(); void 0").await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_history_traversal_detachment_discards_the_late_response() {
    for method in ["back", "traverseTo", "history-back"] {
        let (mut page, gate) = traversal_page("html").await;
        let initial = setup(&mut page, method, true).await;
        page.evaluate("beginChildTraversal(); void 0").await;
        ResponseGate::wait_for(&gate.requests, 2).await;
        let pending = page.evaluate("childSnapshot()").await;
        assert_pending(&pending, &initial, true);
        page.evaluate("childFrame.remove(); void 0").await;
        gate.release.add_permits(1);
        ResponseGate::wait_for(&gate.responses, 2).await;
        let removed = page
            .evaluate("new Promise(r => setTimeout(r, 100)).then(childSnapshot)")
            .await;
        let mut events = before_events(true);
        events.extend(["navigate", "abort", "error:AbortError"]);
        events.extend(unload_events(true));
        assert_eq!(removed["events"], json!(events), "{method}: {removed}");
        assert_eq!(removed["sameDocument"], false);
        assert_eq!(removed["nestedAlive"], false);
        assert_eq!(removed["loads"], 2);
        assert_eq!(removed["index"], json!(null));
        assert_eq!(removed["aborted"], json!([true]));
        assert_eq!(removed["errorIdentity"], true);
        assert_eq!(
            removed["states"],
            if method == "history-back" {
                json!({})
            } else {
                json!({"committed":"AbortError", "finished":"AbortError"})
            }
        );
    }
}

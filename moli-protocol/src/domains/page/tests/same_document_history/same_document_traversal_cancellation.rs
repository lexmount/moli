use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn gated_page() -> (SameDocumentPage, Arc<ResponseGate>) {
    let gate = Arc::new(ResponseGate {
        requests: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let observed = gate.clone();
    let observed_start = gate.clone();
    let routes = axum::Router::new()
        .route(
            "/navigation-started",
            axum::routing::get(move || {
                let gate = observed_start.clone();
                async move {
                    ResponseGate::wait_for(&gate.requests, 1).await;
                    "started"
                }
            }),
        )
        .route(
            "/opaque.html",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/html")],
                    OPAQUE_PAGE,
                )
            }),
        )
        .route(
            "/pending.html",
            axum::routing::get(move || {
                let gate = observed.clone();
                // Keep the late response observable even if cancellation closes
                // the connection and drops the HTTP handler waiting for it.
                let response = tokio::spawn(async move {
                    gate.requests.fetch_add(1, Ordering::SeqCst);
                    gate.release.acquire().await.unwrap().forget();
                    gate.responses.fetch_add(1, Ordering::SeqCst);
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/html")],
                        "<!doctype html><title>Late navigation</title>",
                    )
                });
                async move { response.await.unwrap() }
            }),
        );
    (SameDocumentPage::with_routes(routes).await, gate)
}

const SETUP: &str = r#"(async () => {
    let target = window;
    if (OWNER === 'child') {
        const frame = document.createElement('iframe');
        const loaded = new Promise(resolve => frame.onload = resolve);
        frame.src = 'history.html?child';
        document.body.appendChild(frame);
        await loaded;
        target = frame.contentWindow;
    } else if (OWNER === 'popup') {
        target = open('history.html?popup', 'traversal-cancellation');
        await new Promise(resolve => target.onload = resolve);
    }
    await new Promise(resolve => setTimeout(resolve, 0));
    target.history.pushState(null, '', '#1');
    target.history.pushState(null, '', '#2');
    globalThis.testWin = target;
    const doc = target.document, nav = target.navigation;
    const initial = nav.entries().map(entry => ({url:entry.url,key:entry.key,id:entry.id}));
    const events = [], signals = [], states = {}, reasons = [];
    const Exception = target.DOMException;
    globalThis.observeResult = (result, prefix) => {
        for (const key of ['committed','finished']) result[key].then(
            () => states[prefix + key] = 'fulfilled',
            error => { states[prefix + key] = error.name; reasons.push(error); }
        );
    };
    nav.addEventListener('navigate', event => {
        events.push('navigate:' + event.navigationType);
        signals.push(event.signal);
        event.signal.addEventListener('abort', () => events.push('abort'));
    });
    globalThis.onOldAbort = callback => signals[0].addEventListener('abort', callback, {once:true});
    nav.addEventListener('navigateerror', event => {
        events.push('error:' + event.error.name);
        reasons.push(event.error);
    });
    for (const type of ['beforeunload','pagehide','unload','load','popstate','hashchange'])
        target.addEventListener(type, () => events.push(type));
    globalThis.snapshot = () => ({
        sameDocument: target.document === doc,
        href: target.location.href, documentURL: target.document.URL,
        entries: target.navigation.entries().map(entry => ({url:entry.url,key:entry.key,id:entry.id})),
        index: target.navigation.currentEntry.index,
        events: [...events], aborted: signals.map(signal => signal.aborted),
        states: {...states},
        errorIdentity: reasons.length > 0 && reasons.every(error =>
            error === signals[0].reason && error instanceof Exception),
    });
    return {entries:initial, index:nav.currentEntry.index, href:target.location.href};
})()"#;

async fn check_cancellation(owner: &str) {
    for operation in ["location", "navigate"] {
        for traversal in ["history", "back", "traverseTo"] {
            let (mut page, gate) = gated_page().await;
            let initial = page
                .evaluate(&SETUP.replace("OWNER", &format!("'{owner}'")))
                .await;
            page.ctx.sent.clear();
            let traversal = match traversal {
                "history" => "testWin.history.back()",
                "back" => "observeResult(testWin.navigation.back(), 'back:')",
                "traverseTo" => {
                    "observeResult(testWin.navigation.traverseTo(testWin.navigation.entries().at(-2).key), 'back:')"
                }
                _ => unreachable!(),
            };
            let begin = if operation == "location" {
                "testWin.location.href = 'pending.html'"
            } else {
                "observeResult(testWin.navigation.navigate('pending.html'), 'old:')"
            };
            let (pending, sync) = if owner == "top" {
                // CDP refuses a new Runtime.evaluate while the top Document
                // has a pending navigation. Let its own task perform the
                // traversal after the HTTP server has observed that request.
                page.evaluate(&format!(
                    r#"
                    setTimeout(async () => {{
                        {begin};
                        globalThis.pendingSnapshot = snapshot();
                        await fetch('navigation-started');
                        {traversal};
                        globalThis.syncSnapshot = snapshot();
                    }}, 0); void 0
                "#
                ))
                .await;
                ResponseGate::wait_for_with_context(&mut page.ctx, &gate.requests, 1).await;
                let expected = initial["entries"][initial["index"].as_u64().unwrap() as usize - 1]
                    ["url"]
                    .clone();
                wait_until_message(
                    &mut page.ctx,
                    SESSION,
                    "top same-document traversal",
                    |message| {
                        message["method"] == "Page.navigatedWithinDocument"
                            && message["params"]["url"] == expected
                    },
                )
                .await;
                let snapshots = page
                    .evaluate("({pending:pendingSnapshot,sync:syncSnapshot})")
                    .await;
                (snapshots["pending"].clone(), snapshots["sync"].clone())
            } else {
                page.evaluate(&format!("{begin}; void 0")).await;
                ResponseGate::wait_for_with_context(&mut page.ctx, &gate.requests, 1).await;
                let pending = page.evaluate("snapshot()").await;
                let sync = page.evaluate(&format!("{traversal}; snapshot()")).await;
                (pending, sync)
            };
            assert_eq!(pending["sameDocument"], true, "{owner}: {pending}");
            assert_eq!(pending["href"], initial["href"]);
            assert_eq!(sync["href"], initial["href"], "traversal must be queued");
            let traversed = page
                .evaluate(
                    r#"(async () => {
                for (let n = 0; n < 100 && testWin.location.hash !== '#1'; ++n)
                    await new Promise(resolve => setTimeout(resolve, 5));
                return snapshot();
            })()"#,
                )
                .await;
            let index = initial["index"].as_u64().unwrap() - 1;
            assert_eq!(traversed["index"], index, "{owner}: {traversed}");
            assert_eq!(traversed["href"], initial["entries"][index as usize]["url"]);
            gate.release.add_permits(1);
            ResponseGate::wait_for_with_context(&mut page.ctx, &gate.responses, 1).await;
            let settled = page
                .evaluate("new Promise(resolve => setTimeout(resolve, 100)).then(() => snapshot())")
                .await;
            assert_eq!(
                settled["sameDocument"], true,
                "{owner}/{operation}: {settled}"
            );
            assert_eq!(settled["href"], traversed["href"], "{owner}: {settled}");
            assert_eq!(settled["documentURL"], settled["href"]);
            assert_eq!(settled["entries"], initial["entries"]);
            assert_eq!(settled["index"], index);
            assert_eq!(
                settled["aborted"],
                json!([true, false]),
                "{owner}: {settled}"
            );
            assert_eq!(settled["errorIdentity"], true, "{owner}: {settled}");
            if operation == "navigate" {
                for key in ["old:committed", "old:finished"] {
                    assert_eq!(settled["states"][key], "AbortError", "{owner}: {settled}");
                }
            }
            if traversal.contains("observeResult") {
                for key in ["back:committed", "back:finished"] {
                    assert_eq!(settled["states"][key], "fulfilled", "{owner}: {settled}");
                }
            }
            let events = settled["events"].as_array().unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|value| *value == "beforeunload")
                    .count(),
                pending["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|value| *value == "beforeunload")
                    .count(),
                "same-document traversal must not dispatch beforeunload: {settled}"
            );
            for event in ["pagehide", "unload", "load"] {
                assert!(!events.contains(&json!(event)), "{owner}: {settled}");
            }
            if owner == "popup" {
                page.evaluate("testWin.close(); void 0").await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_cancels_top_network_navigation() {
    run_history_test(check_cancellation("top")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_cancels_child_network_navigation() {
    run_history_test(check_cancellation("child")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_cancels_popup_network_navigation() {
    run_history_test(check_cancellation("popup")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_cancels_only_the_older_network_navigation() {
    run_history_test(async {
        for owner in ["child", "popup"] {
            let (mut page, gate) = gated_page().await;
            page.evaluate(&SETUP.replace("OWNER", &format!("'{owner}'")))
                .await;
            page.evaluate(
                "observeResult(testWin.navigation.navigate('pending.html'), 'old:'); void 0",
            )
            .await;
            ResponseGate::wait_for_with_context(&mut page.ctx, &gate.requests, 1).await;
            page.evaluate(
                r#"
                onOldAbort(() => {
                    observeResult(testWin.navigation.navigate('history.html?successor'), 'new:');
                });
                testWin.history.back();
                void 0;
            "#,
            )
            .await;
            let result = page
                .evaluate(
                    r#"(async () => {
                for (let n = 0; n < 100 && testWin.location.search !== '?successor'; ++n)
                    await new Promise(resolve => setTimeout(resolve, 5));
                return {search:testWin.location.search, snapshot:snapshot()};
            })()"#,
                )
                .await;
            gate.release.add_permits(1);
            ResponseGate::wait_for_with_context(&mut page.ctx, &gate.responses, 1).await;
            assert_eq!(result["search"], "?successor", "{owner}: {result}");
            assert_eq!(result["snapshot"]["states"]["old:finished"], "AbortError");
            assert_eq!(
                result["snapshot"]["aborted"].as_array().unwrap().last(),
                Some(&json!(false))
            );
            if owner == "popup" {
                page.evaluate("testWin.close(); void 0").await;
            }
        }
    })
    .await;
}

const OPAQUE_PAGE: &str = r#"<!doctype html><script>
const original = document;
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const snapshot = () => ({href:location.href, sameDocument:document === original,
    entries:navigation.entries().length, length:history.length});
addEventListener('message', async event => {
    const {id, command} = event.data;
    if (command === 'start') location.href = 'pending.html';
    if (command === 'back') history.back();
    if (command === 'settle') {
        for (let n = 0; n < 100 && location.hash !== '#1'; ++n) await tick();
    }
    parent.postMessage({id, value:snapshot()}, '*');
});
(async () => {
    location.hash = '#1'; await tick();
    location.hash = '#2'; await tick();
    parent.postMessage({id:0, value:snapshot()}, '*');
})();
</script>"#;

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_cancels_opaque_child_network_navigation() {
    run_history_test(async {
        let (mut page, gate) = gated_page().await;
        let initial = page.evaluate(r#"(async () => {
            const frame = document.createElement('iframe');
            frame.sandbox = 'allow-scripts';
            frame.src = 'opaque.html';
            const replies = new Map();
            let nextId = 0;
            addEventListener('message', event => {
                if (event.source !== frame.contentWindow) return;
                replies.get(event.data.id)?.(event.data.value);
                replies.delete(event.data.id);
            });
            globalThis.opaqueCommand = command => new Promise(resolve => {
                const id = ++nextId;
                replies.set(id, resolve);
                frame.contentWindow.postMessage({id,command}, '*');
                setTimeout(() => { if (replies.delete(id)) resolve(null); }, 1000);
            });
            const ready = new Promise(resolve => replies.set(0, resolve));
            document.body.appendChild(frame);
            return ready;
        })()"#).await;
        assert_eq!(initial["entries"], 0, "Navigation API must be disabled");
        page.evaluate("opaqueCommand('start')").await;
        ResponseGate::wait_for_with_context(&mut page.ctx, &gate.requests, 1).await;
        let sync = page.evaluate("opaqueCommand('back')").await;
        assert_eq!(sync["href"], initial["href"]);
        let traversed = page.evaluate("opaqueCommand('settle')").await;
        assert!(traversed["href"].as_str().unwrap().ends_with("#1"), "{traversed}");
        gate.release.add_permits(1);
        ResponseGate::wait_for_with_context(&mut page.ctx, &gate.responses, 1).await;
        let settled = page.evaluate("new Promise(resolve => setTimeout(resolve,100)).then(() => opaqueCommand('snapshot'))").await;
        assert_eq!(settled, traversed, "late response must not replace the opaque Document");
    }).await;
}

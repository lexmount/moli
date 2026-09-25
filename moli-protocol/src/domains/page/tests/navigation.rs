use super::*;

fn find_cdp_node_by_local_name<'a>(
    node: &'a serde_json::Value,
    local_name: &str,
) -> Option<&'a serde_json::Value> {
    if node["localName"] == json!(local_name) {
        return Some(node);
    }
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|child| find_cdp_node_by_local_name(child, local_name))
}

fn assert_runtime_navigation_context_reset(
    sent: &[serde_json::Value],
    session_id: &str,
    frame_id: &str,
) {
    let session_context_events = sent
        .iter()
        .enumerate()
        .filter(|(_, message)| message["sessionId"] == json!(session_id))
        .filter(|(_, message)| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
            )
        })
        .collect::<Vec<_>>();
    let last_clear_index = session_context_events
        .iter()
        .filter(|(_, message)| message["method"] == json!("Runtime.executionContextsCleared"))
        .map(|(index, _)| *index)
        .max()
        .unwrap_or_else(|| {
            panic!("navigation should clear old Runtime contexts for {session_id}: {sent:?}")
        });
    let default_context_index = session_context_events
        .iter()
        .filter(|(_, message)| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(frame_id)
        })
        .map(|(index, _)| *index)
        .next()
        .unwrap_or_else(|| {
            panic!(
                "navigation should create the new default Runtime context for {session_id}: {sent:?}"
            )
        });
    assert!(
        last_clear_index < default_context_index,
        "all old-context clears should precede the new default context for {session_id}: {sent:?}"
    );
}

async fn assert_joint_history_traversal_info(caller_first: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/{page}",
                axum::routing::get(|| async {
                    axum::response::Html("<!doctype html><body>loaded")
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({"id":9346,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("http://{addr}/top")}})).await;
    assert!(take_response_by_id(&mut ctx, 9346)["error"].is_null());
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "traversal info fixture load",
        |message| message["method"] == "Page.domContentEventFired",
    )
    .await;
    let expression = r#"(async () => {
        const caller = CALLER_FIRST ? 0 : 1;
        const sibling = 1 - caller;
        const windows = [];
        for (const id of ['a', 'b']) {
            const frame = document.createElement('iframe'); frame.src = '/' + id;
            await new Promise(resolve => {frame.onload = resolve; document.body.append(frame)});
            await new Promise(resolve => setTimeout(resolve, 0));
            frame.contentWindow.history.replaceState(id + '0', '');
            windows.push(frame.contentWindow);
        }
        const initialKeys = windows.map(w => w.navigation.currentEntry.key);
        for (const index of [caller, sibling]) {
            windows[index].history.pushState(['a1', 'b1'][index], '');
        }
        const finalKeys = windows.map(w => w.navigation.currentEntry.key);
        const traverse = async (initiator, keys) => {
            const marker = {}; marker.self = marker;
            const events = [[], []];
            const handlers = windows.map((w, index) => {
                const handler = event => events[index].push([
                    event.navigationType, event.destination.sameDocument,
                    event.info === marker, event.info === undefined
                ]);
                w.navigation.addEventListener('navigate', handler);
                return handler;
            });
            const popped = windows.map(w => new Promise(resolve =>
                w.addEventListener('popstate', () => resolve(), {once: true})));
            const result = windows[initiator].navigation.traverseTo(keys[initiator], {info: marker});
            const committed = await result.committed;
            await Promise.all([...popped, result.finished]);
            windows.forEach((w, i) => w.navigation.removeEventListener('navigate', handlers[i]));
            return {
                events,
                states: windows.map(w => w.history.state),
                entriesRestored: windows.every((w, i) => w.navigation.currentEntry.key === keys[i]),
                committedEntry: committed === windows[initiator].navigation.currentEntry
            };
        };
        return {
            backward: await traverse(caller, initialKeys),
            forward: await traverse(sibling, finalKeys)
        };
    })()"#
    .replace("CALLER_FIRST", if caller_first { "true" } else { "false" });
    let actual = joint_history_test_evaluate(&mut ctx, &expression).await;
    let events = |first: bool| {
        json!([
            [["traverse", true, first, !first]],
            [["traverse", true, !first, first]]
        ])
    };
    assert_eq!(
        actual,
        json!({
            "backward": {
                "events": events(caller_first),
                "states": ["a0", "b0"],
                "entriesRestored": true,
                "committedEntry": true
            },
            "forward": {
                "events": events(!caller_first),
                "states": ["a1", "b1"],
                "entriesRestored": true,
                "committedEntry": true
            }
        })
    );
    server.abort();
}

async fn assert_queued_history_same_then_cross_document(forward: bool) {
    let app = axum::Router::new()
        .fallback(|| async { axum::response::Html("<!doctype html><body>history fixture") });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 9350, "method": "Page.navigate", "sessionId": "SID-1",
        "params": {"url": format!("http://{addr}/top")}
    }))
    .await;
    assert!(take_response_by_id(&mut ctx, 9350)["error"].is_null());
    wait_until_message(&mut ctx, Some("SID-1"), "history fixture load", |message| {
        message["method"] == "Page.domContentEventFired"
    })
    .await;
    joint_history_test_evaluate(
        &mut ctx,
        r#"
        globalThis.frame = document.createElement('iframe');
        globalThis.withFrameLoad = action => new Promise(resolve => {
            frame.onload = () => resolve(true);
            action();
        });
        globalThis.withFrameHash = action => new Promise(resolve => {
            frame.contentWindow.addEventListener('hashchange', () => resolve(true), {once:true});
            action();
        });
        frame.src = '/child?first';
        withFrameLoad(() => document.body.append(frame));
        "#,
    )
    .await;
    // Separate protocol commands start the next navigation outside the
    // preceding load handler, preserving push rather than replace semantics.
    let hash = "withFrameHash(() => frame.contentWindow.location.hash = '#same')";
    let load = "withFrameLoad(() => frame.contentWindow.location.search = '?second')";
    for script in if forward { [hash, load] } else { [load, hash] } {
        joint_history_test_evaluate(&mut ctx, script).await;
    }
    if forward {
        joint_history_test_evaluate(
            &mut ctx,
            "withFrameLoad(() => frame.contentWindow.history.back())",
        )
        .await;
        joint_history_test_evaluate(
            &mut ctx,
            "withFrameHash(() => frame.contentWindow.history.back())",
        )
        .await;
    }
    let method = if forward { "forward" } else { "back" };
    let synchronous = joint_history_test_evaluate(
        &mut ctx,
        &format!(
            r#"
            globalThis.traversalEvents = [];
            const snapshot = kind => [kind, frame.contentWindow.location.search,
                frame.contentWindow.location.hash];
            frame.contentWindow.addEventListener('hashchange', event => {{
                const destination = new URL(event.newURL);
                traversalEvents.push(['hash', destination.search, destination.hash]);
            }}, {{once:true}});
            globalThis.traversalsFinished = new Promise(resolve => {{
                frame.onload = () => {{
                    traversalEvents.push(snapshot('load'));
                    resolve(traversalEvents);
                }};
            }});
            frame.contentWindow.history.{method}();
            frame.contentWindow.history.{method}();
            [frame.contentWindow.location.search, frame.contentWindow.location.hash];
            "#
        ),
    )
    .await;
    assert_eq!(
        synchronous,
        if forward {
            json!(["?first", ""])
        } else {
            json!(["?second", "#same"])
        }
    );
    let events = joint_history_test_evaluate(&mut ctx, "traversalsFinished").await;
    assert_eq!(
        events,
        if forward {
            json!([["hash", "?first", "#same"], ["load", "?second", "#same"]])
        } else {
            json!([["hash", "?second", ""], ["load", "?first", ""]])
        }
    );
    server.abort();
}

async fn assert_joint_history_late_response(update_state: bool) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let handler_requests = requests.clone();
    let handler_release = release.clone();
    let app = axum::Router::new().route(
        "/{page}",
        axum::routing::get(
            move |axum::extract::Path(name): axum::extract::Path<String>| {
                let requests = handler_requests.clone();
                let release = handler_release.clone();
                async move {
                    if name == "a0" && requests.fetch_add(1, Ordering::SeqCst) > 0 {
                        release.notified().await;
                    }
                    (
                        [("content-type", "text/html"), ("cache-control", "no-store")],
                        format!("<!doctype html><body>{name}"),
                    )
                }
            },
        ),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({"id":9343,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("http://{addr}/top")}})).await;
    assert!(take_response_by_id(&mut ctx, 9343)["error"].is_null());
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "late traversal fixture load",
        |message| message["method"] == "Page.domContentEventFired",
    )
    .await;
    joint_history_test_evaluate(&mut ctx, r#"(async()=>{
        history.replaceState('top0','');
        globalThis.frame=document.createElement('iframe');frame.src='/a0';
        await new Promise(resolve=>{frame.onload=resolve;document.body.append(frame)});
        await new Promise(resolve=>setTimeout(resolve,0));
        frame.contentWindow.history.replaceState('a0','');
        await new Promise(resolve=>{frame.onload=resolve;frame.contentWindow.location.assign('/a1')});
        await new Promise(resolve=>setTimeout(resolve,0));
        frame.contentWindow.history.replaceState('a1','');history.pushState('top1','');
        globalThis.loads=0;
        globalThis.frameLoaded=new Promise(resolve=>frame.onload=()=>{loads++;resolve(true)});
        globalThis.snapshot=()=>[history.state,frame.contentWindow.history.state,
            frame.contentWindow.location.pathname,frame.contentDocument.body.textContent,
            history.length,frame.contentWindow.history.length,loads];
    })()"#).await;
    ctx.process_async(json!({"id":9344,"method":"Network.enable","sessionId":"SID-1"}))
        .await;
    assert!(take_response_by_id(&mut ctx, 9344)["error"].is_null());
    ctx.sent.clear();
    joint_history_test_evaluate(
        &mut ctx,
        "new Promise(resolve=>{onpopstate=()=>resolve(true);history.go(-2)})",
    )
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "accepted child traversal reaches held response",
        |_| requests.load(Ordering::SeqCst) == 2,
    )
    .await;
    let mutation = if update_state {
        "frame.contentWindow.navigation.updateCurrentEntry({state:'updated'});snapshot()"
    } else {
        "frame.contentWindow.history.pushState('successor','');snapshot()"
    };
    let successor = joint_history_test_evaluate(&mut ctx, mutation).await;
    let expected = if update_state {
        json!(["top0", "a1", "/a1", "a1", 4, 4, 0])
    } else {
        json!(["top0", "successor", "/a1", "a1", 3, 3, 0])
    };
    assert_eq!(successor, expected);
    release.notify_one();
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "held child response completes",
        |messages| {
            messages.iter().any(|request| {
                request["method"] == "Network.requestWillBeSent"
                    && request["params"]["request"]["url"] == format!("http://{addr}/a0")
                    && messages.iter().any(|done| {
                        done["method"] == "Network.loadingFinished"
                            && done["params"]["requestId"] == request["params"]["requestId"]
                    })
            })
        },
    )
    .await;
    if update_state {
        let after = joint_history_test_evaluate(&mut ctx, "frameLoaded.then(()=>snapshot())").await;
        assert_eq!(after, json!(["top0", "a0", "/a0", "a0", 4, 4, 1]));
        assert_eq!(
            joint_history_test_evaluate(
                &mut ctx,
                "frame.contentWindow.navigation.entries()[1].getState()"
            )
            .await,
            json!("updated")
        );
    } else {
        assert_eq!(
            joint_history_test_evaluate(&mut ctx, "snapshot()").await,
            successor
        );
    }
    ctx.process_async(json!({"id":9345,"method":"Page.getNavigationHistory","sessionId":"SID-1"}))
        .await;
    let browser = take_response_by_id(&mut ctx, 9345);
    assert_eq!(
        browser["result"]["currentIndex"],
        json!(if update_state { 1 } else { 2 }),
        "{browser}"
    );
    assert_eq!(
        browser["result"]["entries"].as_array().unwrap().len(),
        if update_state { 4 } else { 3 }
    );
    server.abort();
}

async fn joint_history_test_evaluate(ctx: &mut TestContext, expression: &str) -> serde_json::Value {
    ctx.process_async(json!({"id":9340,"method":"Runtime.evaluate","sessionId":"SID-1","params":{"expression":expression,"awaitPromise":true,"returnByValue":true}})).await;
    wait_until_message(
        ctx,
        Some("SID-1"),
        "joint traversal script completes",
        |message| message["id"] == json!(9340),
    )
    .await;
    let response = take_response_by_id(ctx, 9340);
    assert!(
        response["error"].is_null() && response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    response["result"]["result"]["value"].clone()
}

async fn assert_joint_history_precommit(mode: &str) {
    async fn page(
        axum::extract::Path(name): axum::extract::Path<String>,
    ) -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><body>{name}"),
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/{page}", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({"id":9339,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("http://{addr}/top")}})).await;
    assert!(take_response_by_id(&mut ctx, 9339)["error"].is_null());
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "precommit test page load",
        |message| message["method"] == json!("Page.domContentEventFired"),
    )
    .await;
    joint_history_test_evaluate(
        &mut ctx,
        if mode == "attach-single" {
            "globalThis.testTraversalDelta = -1"
        } else {
            "globalThis.testTraversalDelta = -2"
        },
    )
    .await;
    joint_history_test_evaluate(
        &mut ctx,
        if mode == "replace-participant" {
            "globalThis.testTraversalApi = true"
        } else {
            "globalThis.testTraversalApi = false"
        },
    )
    .await;
    let pending=joint_history_test_evaluate(&mut ctx,r#"(async()=>{
        history.replaceState('top0','');
        globalThis.frame=document.createElement('iframe');frame.src='/a0';
        await new Promise(resolve=>{frame.onload=resolve;document.body.append(frame)});
        await new Promise(resolve=>setTimeout(resolve,0));
        frame.contentWindow.history.replaceState('a0','');
        if(testTraversalApi)history.pushState('top1','');
        await new Promise(resolve=>{frame.onload=resolve;frame.contentWindow.location.assign('/a1')});
        await new Promise(resolve=>setTimeout(resolve,0));
        frame.contentWindow.history.replaceState('a1','');
        if(!testTraversalApi)history.pushState('top1','');
        globalThis.pagehides=0;frame.contentWindow.onpagehide=()=>pagehides++;
        globalThis.snapshot=()=>[history.state,frame.contentWindow.history.state,
            frame.contentWindow.location.pathname,history.length,frame.contentWindow.history.length,pagehides];
        let admitted;const started=new Promise(resolve=>admitted=resolve);
        const gate=new Promise((resolve,reject)=>{globalThis.release=resolve;globalThis.block=reject});
        globalThis.failed=new Promise(resolve=>navigation.onnavigateerror=resolve);
        navigation.onnavigate=event=>{
            if(event.navigationType==='traverse')event.intercept({precommitHandler:()=>{admitted();return gate}});
        };
        globalThis.settlements=[];
        if(testTraversalApi){
            const result=navigation.back();
            result.committed.catch(error=>settlements.push('committed:'+error.name));
            result.finished.catch(error=>settlements.push('finished:'+error.name));
        }else{
            history.go(testTraversalDelta);
        }
        await started;return snapshot();
    })()"#).await;
    assert_eq!(pending, json!(["top1", "a1", "/a1", 4, 4, 0]), "{mode}");
    ctx.process_async(json!({"id":9341,"method":"Page.getNavigationHistory","sessionId":"SID-1"}))
        .await;
    let browser = take_response_by_id(&mut ctx, 9341);
    assert_eq!(
        browser["result"]["currentIndex"],
        json!(3),
        "{mode}: {browser}"
    );
    assert_eq!(browser["result"]["entries"].as_array().unwrap().len(), 4);
    if mode == "prune" {
        ctx.process_async(
            json!({"id":9342,"method":"Page.resetNavigationHistory","sessionId":"SID-1"}),
        )
        .await;
        assert!(take_response_by_id(&mut ctx, 9342)["error"].is_null());
    }
    if matches!(mode, "attach" | "attach-single") {
        let lengths = joint_history_test_evaluate(&mut ctx, r#"(async()=>{
            const added=document.createElement('iframe');added.src='/attached';
            await new Promise(resolve=>{added.onload=resolve;document.body.append(added)});
            await new Promise(resolve=>setTimeout(resolve,0));
            return [history.length,frame.contentWindow.history.length,added.contentWindow.history.length];
        })()"#).await;
        assert_eq!(lengths, json!([4, 4, 4]), "{mode}");
    }
    let expression = match mode {
        "resolve" | "attach" => {
            "new Promise(resolve=>{frame.onload=()=>setTimeout(()=>resolve(snapshot()),0);release()})"
        }
        "attach-single" => "new Promise(resolve=>{onpopstate=()=>resolve(snapshot());release()})",
        "reject" => "block(new Error('blocked'));failed.then(()=>snapshot())",
        "stop-pagehide" => {
            r#"new Promise(resolve=>{
            navigation.addEventListener('navigateerror',()=>resolve(snapshot()),{once:true});
            frame.onload=()=>setTimeout(()=>resolve(snapshot()),0);
            frame.contentWindow.onpagehide=()=>{pagehides++;stop()};
            release();
        })"#
        }
        "replace-participant" => {
            "new Promise(resolve=>{frame.onload=()=>setTimeout(()=>resolve(snapshot()),0);frame.src='/successor'})"
        }
        _ => "release();failed.then(()=>snapshot())",
    };
    let after = joint_history_test_evaluate(&mut ctx, expression).await;
    let (expected, index, length) = match mode {
        "resolve" | "attach" => (json!(["top0", "a0", "/a0", 4, 4, 1]), 1, 4),
        "attach-single" => (json!(["top0", "a1", "/a1", 4, 4, 0]), 2, 4),
        "reject" => (json!(["top1", "a1", "/a1", 4, 4, 0]), 3, 4),
        // pagehide runs at document commit, after admission has completed.
        // Chromium also completes this traversal when pagehide calls stop().
        "stop-pagehide" => (json!(["top0", "a0", "/a0", 4, 4, 1]), 1, 4),
        "replace-participant" => (json!(["top1", null, "/successor", 5, 5, 1]), 4, 5),
        _ => (json!(["top1", "a1", "/a1", 1, 1, 0]), 0, 1),
    };
    assert_eq!(after, expected, "{mode}");
    if mode == "replace-participant" {
        assert_eq!(
            joint_history_test_evaluate(
                &mut ctx,
                "release();Promise.resolve().then(()=>snapshot())"
            )
            .await,
            expected
        );
        assert_eq!(
            joint_history_test_evaluate(&mut ctx, "settlements").await,
            json!(["committed:AbortError", "finished:AbortError"])
        );
    }
    ctx.process_async(json!({"id":9341,"method":"Page.getNavigationHistory","sessionId":"SID-1"}))
        .await;
    let browser = take_response_by_id(&mut ctx, 9341);
    assert_eq!(
        browser["result"]["currentIndex"],
        json!(index),
        "{mode}: {browser}"
    );
    assert_eq!(
        browser["result"]["entries"].as_array().unwrap().len(),
        length
    );
    server.abort();
}

async fn assert_joint_history_multi_frame_traversal(cross: &str, both_cross: bool, reenter: bool) {
    async fn page(
        axum::extract::Path(name): axum::extract::Path<String>,
    ) -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><body data-page='{name}'>{name}"),
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/{page}", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({"id":9290,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("http://{addr}/top")}})).await;
    assert!(take_response_by_id(&mut ctx, 9290)["error"].is_null());
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "mixed traversal page load",
        |message| message["method"] == json!("Page.domContentEventFired"),
    )
    .await;
    let expression = r#"(async () => {
        const cross = CROSS, bothCross = BOTH_CROSS, reenter = REENTER;
        const other = cross === 'a' ? 'b' : 'a';
        const task = () => new Promise(resolve => setTimeout(resolve, 0));
        for (const id of ['a', 'b']) {
            const frame = document.createElement('iframe'); frame.id = id; frame.src = '/'+id+'0';
            await new Promise(resolve => {frame.onload=resolve;document.body.append(frame)});
            await task();
            frame.contentWindow.history.replaceState(id+'0', '');
        }
        const frame = id => document.getElementById(id);
        const navigate = async id => {
            await new Promise(resolve => {frame(id).onload=resolve;frame(id).contentWindow.location.assign('/'+id+'1')});
            await task(); frame(id).contentWindow.history.replaceState(id+'1', '');
        };
        await navigate(cross);
        if (bothCross) await navigate(other);
        else frame(other).contentWindow.history.pushState(other+'1', '');
        let admitted;
        const admittedPromise = new Promise(resolve => admitted=resolve);
        if (reenter) frame(cross).contentWindow.navigation.addEventListener('navigate', event => {
            if (event.navigationType==='traverse') { history.pushState('from-admission',''); admitted(); }
        }, {once:true});
        globalThis.jointMove = delta => new Promise(resolve => {
            const completed = new Set();
            const done = id => { completed.add(id); if(completed.size===2) {clearTimeout(timer);resolve()} };
            const timer = setTimeout(()=>resolve(), 3000);
            frame(cross).onload=()=>done(cross);
            if (bothCross) frame(other).onload=()=>done(other);
            else frame(other).contentWindow.addEventListener('popstate',()=>done(other),{once:true});
            history.go(delta);
        });
        globalThis.jointSnapshot = () => ({
            pages: ['a','b'].map(id=>frame(id).contentDocument.body.dataset.page),
            states: ['a','b'].map(id=>frame(id).contentWindow.history.state),
            lengths: [history.length,...['a','b'].map(id=>frame(id).contentWindow.history.length)],
            indices: ['a','b'].map(id=>frame(id).contentWindow.navigation.currentEntry.index)
        });
        if (reenter) { history.go(-2); await admittedPromise; await task(); }
        else await jointMove(-2);
        await task();
        return jointSnapshot();
    })()"#.replace("BOTH_CROSS", if both_cross {"true"} else {"false"})
        .replace("REENTER", if reenter {"true"} else {"false"})
        .replace("CROSS", &format!("'{cross}'"));
    ctx.process_async(json!({"id":9291,"method":"Runtime.evaluate","sessionId":"SID-1","params":{"expression":expression,"awaitPromise":true,"returnByValue":true}})).await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "mixed traversal settles",
        |message| message["id"] == json!(9291),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 9291);
    let expected = if reenter {
        json!({"pages":if cross=="a" {vec!["a1","b0"]} else {vec!["a0","b1"]},"states":["a1","b1"],"lengths":[5,5,5],"indices":[1,1]})
    } else {
        json!({"pages":["a0","b0"],"states":["a0","b0"],"lengths":[4,4,4],"indices":[0,0]})
    };
    assert_eq!(
        response["result"]["result"]["value"], expected,
        "cross={cross}, both_cross={both_cross}, reenter={reenter}: {response}"
    );
    ctx.process_async(json!({"id":9292,"method":"Page.getNavigationHistory","sessionId":"SID-1"}))
        .await;
    let browser = take_response_by_id(&mut ctx, 9292);
    assert_eq!(
        browser["result"]["currentIndex"],
        json!(if reenter { 4 } else { 1 }),
        "{browser}"
    );
    assert_eq!(
        browser["result"]["entries"].as_array().unwrap().len(),
        if reenter { 5 } else { 4 }
    );
    if !reenter {
        let forward = joint_history_test_evaluate(&mut ctx, "jointMove(2).then(()=>new Promise(resolve=>setTimeout(()=>resolve(jointSnapshot()),0)))").await;
        let pages = if both_cross {
            vec!["a1", "b1"]
        } else if cross == "a" {
            vec!["a1", "b0"]
        } else {
            vec!["a0", "b1"]
        };
        assert_eq!(
            forward,
            json!({"pages":pages,"states":["a1","b1"],"lengths":[4,4,4],"indices":[1,1]}),
            "cross={cross}, both_cross={both_cross}"
        );
        ctx.process_async(
            json!({"id":9292,"method":"Page.getNavigationHistory","sessionId":"SID-1"}),
        )
        .await;
        let browser = take_response_by_id(&mut ctx, 9292);
        assert_eq!(browser["result"]["currentIndex"], json!(3), "{browser}");
        assert_eq!(browser["result"]["entries"].as_array().unwrap().len(), 4);
    }
    server.abort();
}

fn take_navigated_within_document_event(
    ctx: &mut TestContext,
    expected_url: &str,
    expected_navigation_type: &str,
) {
    let position = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.navigatedWithinDocument"))
        .unwrap_or_else(|| {
            panic!(
                "expected Page.navigatedWithinDocument for {expected_url}; messages={:?}",
                ctx.sent
            )
        });
    let event = ctx.sent.remove(position);
    assert_eq!(event["params"]["frameId"], json!("TID-SAME-DOCUMENT"));
    assert_eq!(event["params"]["url"], json!(expected_url));
    assert_eq!(
        event["params"]["navigationType"],
        json!(expected_navigation_type)
    );
}

async fn assert_handler_navigation_renderer_lifecycle(
    event_name: &str,
    expected_source_milestone_sequences: &[&[&str]],
) {
    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Final document</title><main>handler final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let event_target = if event_name == "DOMContentLoaded" {
        "document"
    } else {
        "window"
    };
    let source_html = format!(
        "<!doctype html><script>{event_target}.addEventListener({event_name:?},()=>{{location.href='/final'}},{{once:true}})</script><main>handler-source-{event_name}</main>"
    );
    let source_handler = move || {
        let html = source_html.clone();
        async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                html,
            )
        }
    };
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/source", axum::routing::get(source_handler))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let final_url = format!("http://{addr}/final");
    let source_url = format!("http://{addr}/source");
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 253,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(253, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 254,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": source_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 254);
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "handler-triggered successor document load",
        |messages| {
            let Some(final_commit_index) = messages.iter().position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(final_url)
            }) else {
                return false;
            };
            messages[final_commit_index + 1..]
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    let source_commit_index = events
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(source_url)
        })
        .unwrap_or_else(|| panic!("source document should commit: {events:?}"));
    let source_loader = events[source_commit_index]["params"]["frame"]["loaderId"]
        .as_str()
        .expect("source loader id");
    let final_commit_index = events
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(final_url)
        })
        .unwrap_or_else(|| panic!("successor document should commit: {events:?}"));
    let final_loader = events[final_commit_index]["params"]["frame"]["loaderId"]
        .as_str()
        .expect("final loader id");
    assert_ne!(source_loader, final_loader);

    let milestones_for_loader = |loader_id: &str| {
        events
            .iter()
            .filter(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["loaderId"] == json!(loader_id)
                    && matches!(
                        message["params"]["name"].as_str(),
                        Some("DOMContentLoaded" | "load")
                    )
            })
            .map(|message| message["params"]["name"].as_str().unwrap())
            .collect::<Vec<_>>()
    };
    let source_milestones = milestones_for_loader(source_loader);
    assert!(
        expected_source_milestone_sequences.contains(&source_milestones.as_slice()),
        "source milestone sequence should reflect the handler return boundary: {events:?}"
    );
    assert_eq!(
        milestones_for_loader(final_loader),
        vec!["DOMContentLoaded", "load"],
        "successor document should complete normally: {events:?}"
    );
    assert!(
        source_milestones.iter().all(|name| {
            events.iter().position(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["loaderId"] == json!(source_loader)
                    && message["params"]["name"] == json!(name)
            }) < Some(final_commit_index)
        }),
        "source milestones must be emitted before the successor commit: {events:?}"
    );

    server.abort();
}

mod document_lifecycle;
mod frames;
mod history;
mod navigation_flow;
mod reload_and_policy;
mod requests;
mod runtime_contexts;

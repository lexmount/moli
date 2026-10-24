use super::*;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type FontGates = HashMap<String, tokio::sync::oneshot::Sender<()>>;

async fn gated_fonts(
    names: &[&str],
) -> (
    String,
    tokio::sync::mpsc::UnboundedReceiver<String>,
    FontGates,
    tokio::task::JoinHandle<()>,
) {
    gated_fonts_with_bytes(
        names,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        )),
    )
    .await
}

async fn gated_fonts_with_bytes(
    names: &[&str],
    font: &'static [u8],
) -> (
    String,
    tokio::sync::mpsc::UnboundedReceiver<String>,
    FontGates,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (requests_tx, requests_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut gates = HashMap::new();
    let mut releases = HashMap::new();
    for name in names {
        let (tx, rx) = tokio::sync::oneshot::channel();
        gates.insert((*name).to_owned(), tx);
        releases.insert((*name).to_owned(), rx);
    }
    let server = tokio::spawn(async move {
        let mut responses = Vec::new();
        while !releases.is_empty() {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            let release = releases.remove(path.rsplit('/').next().unwrap()).unwrap();
            requests_tx.send(path).unwrap();
            responses.push(tokio::spawn(async move {
                let _ = release.await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: font/woff2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    font.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(font).await;
            }));
        }
        for response in responses {
            response.await.unwrap();
        }
    });
    (origin, requests_rx, gates, server)
}

async fn assert_mixed_ready(js_starts_first: bool, js_finishes_first: bool) {
    let (origin, mut requests, mut gates, server) = gated_fonts(&["css.woff2", "js.woff2"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head></head><body>AAAA</body></html>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval("globalThis.fonts=document.fonts;globalThis.readyResults=[];globalThis.doneEvents=[];fonts.addEventListener('loadingdone',event=>doneEvents.push(event.fontfaces.map(face=>face.family).sort().join(',')))").unwrap();
    let start_css = format!(
        r#"const style=document.createElement('style');
        style.textContent='@font-face{{font-family:AuditCSS;src:url("{origin}/css.woff2")}}';
        document.head.append(style);document.body.style.fontFamily='AuditCSS';
        globalThis.cssReady=fonts.ready;
        globalThis.cssFace=[...fonts].find(face=>face.family==='AuditCSS');
        cssReady.then(value=>readyResults.push('css:'+String(value===fonts)));"#
    );
    let start_js = format!(
        r#"globalThis.jsFace=new FontFace('AuditJS','url("{origin}/js.woff2")');
        fonts.add(jsFace);jsFace.load();globalThis.jsReady=fonts.ready;
        jsReady.then(value=>readyResults.push('js:'+String(value===fonts)));"#
    );
    for (program, expected) in if js_starts_first {
        [(&start_js, "/js.woff2"), (&start_css, "/css.woff2")]
    } else {
        [(&start_css, "/css.woff2"), (&start_js, "/js.woff2")]
    } {
        vm.eval(program).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), requests.recv())
                .await
                .unwrap()
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        vm.eval("JSON.stringify([cssFace.status,jsFace.status,cssReady===jsReady,readyResults,doneEvents])").unwrap(),
        r#"["loading","loading",true,[],[]]"#
    );
    let order = if js_finishes_first {
        ["js.woff2", "css.woff2"]
    } else {
        ["css.woff2", "js.woff2"]
    };
    for (index, name) in order.into_iter().enumerate() {
        gates.remove(name).unwrap().send(()).unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(2),
                completions.wait_for_arrival_without_timeout()
            )
            .await
            .unwrap()
        );
        let completion = completions.pop_next_async_subresource_event().unwrap();
        let activity = vm
            .complete_async_subresource_fetch_event_body(completion)
            .unwrap();
        vm.finish_async_subresource_body_checkpoint_for_test(activity)
            .unwrap();
        run_font_tasks(&mut vm);
        let expected = if index == 1 {
            r#"["loaded","loaded",["css:true","js:true"],["AuditCSS,AuditJS"]]"#
        } else if js_finishes_first {
            r#"["loading","loaded",[],[]]"#
        } else {
            r#"["loaded","loading",[],[]]"#
        };
        // Read only captured promises/results here: another fonts.ready or
        // fonts.status getter must not be needed to finish the loading cycle.
        assert_eq!(
            vm.eval("JSON.stringify([cssFace.status,jsFace.status,readyResults.slice().sort(),doneEvents])").unwrap(),
            expected
        );
    }
    server.await.unwrap();
}

#[tokio::test]
async fn font_set_ready_waits_for_js_after_css_started_and_finished_first() {
    assert_mixed_ready(false, false).await;
}

#[tokio::test]
async fn font_set_ready_preserves_js_promise_when_css_finishes_first() {
    assert_mixed_ready(true, false).await;
}

#[tokio::test]
async fn font_set_ready_waits_for_css_after_js_finishes_first() {
    assert_mixed_ready(false, true).await;
}

#[tokio::test]
async fn font_set_ready_preserves_js_promise_when_js_finishes_first() {
    assert_mixed_ready(true, true).await;
}

#[tokio::test]
async fn font_set_ready_settles_css_started_by_explicit_font_face_load() {
    let (origin, mut requests, mut gates, server) = gated_fonts(&["css.woff2"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head></head><body>AAAA</body></html>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(&format!(
        r#"
        const style=document.createElement('style');
        style.textContent='@font-face{{font-family:AuditCSS;src:url("{origin}/css.woff2")}}';
        document.head.append(style);
        globalThis.fonts=document.fonts;globalThis.face=[...fonts][0];
        face.load();globalThis.ready=fonts.ready;globalThis.readyResolved=false;
        ready.then(value=>readyResolved=value===fonts);
    "#
    ))
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap(),
        "/css.woff2"
    );
    gates.remove("css.woff2").unwrap().send(()).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([face.status,readyResolved])")
            .unwrap(),
        r#"["loaded",true]"#
    );
    server.await.unwrap();
}

#[tokio::test]
async fn font_face_url_is_frozen_before_base_href_changes() {
    let (origin, mut requests, mut gates, server) = gated_fonts(&["font.woff2"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head><base href='/before/'></head><body></body></html>",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.face=new FontFace('Relative','url(font.woff2)');
        document.querySelector('base').href='/after/';
        globalThis.loaded=false;face.load().then(value=>loaded=value===face);
    "#,
    )
    .unwrap();
    let request = tokio::time::timeout(Duration::from_secs(2), requests.recv())
        .await
        .unwrap()
        .unwrap();
    gates.remove("font.woff2").unwrap().send(()).unwrap();
    server.await.unwrap();
    assert_eq!(request, "/before/font.woff2");
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([face.status,loaded])").unwrap(),
        r#"["loaded",true]"#
    );
}

async fn assert_loaded_promise_lifetime(mode: &str) {
    let failure = mode.ends_with("error");
    let (origin, mut requests, mut gates, server) = if failure {
        gated_fonts_with_bytes(&["lifetime.woff2"], b"invalid font bytes").await
    } else {
        gated_fonts(&["lifetime.woff2"]).await
    };
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head></head><body>AAAA</body></html>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    if mode == "js-only" {
        vm.eval(&format!(
            r#"globalThis.promiseResults=[];
            globalThis.retainedPromise=(() => {{
              const face=new FontFace('Lifetime','url("{origin}/lifetime.woff2")');
              globalThis.faceWitness=new WeakRef(face);
              return face.load();
            }})();
            retainedPromise.then(value=>promiseResults.push([value.family,value.status]),
              error=>promiseResults.push(['error',error.name]));"#
        ))
        .unwrap();
    } else {
        vm.eval(&format!(
            r#"const style=document.createElement('style');
            style.textContent='@font-face{{font-family:Lifetime;src:url("{origin}/lifetime.woff2")}}';
            document.head.append(style);document.body.style.fontFamily='Lifetime';"#
        )).unwrap();
        // Only native CSS resource admission starts this request. Do not call
        // FontFace.load(), which installs a separate strong request root.
        vm.reconcile_document_font_resources();
        let query_promise = if mode.contains("query-then") {
            "document.fonts.load('16px Lifetime','A').then(faces => faces)"
        } else if mode.contains("query-await") {
            "(async () => await document.fonts.load('16px Lifetime','A'))()"
        } else if mode.contains("query") {
            "document.fonts.load('16px Lifetime','A')"
        } else if mode.contains("loaded-then") {
            "face.loaded.then(face => face)"
        } else if mode.contains("loaded-await") {
            "(async () => await face.loaded)()"
        } else {
            "face.loaded"
        };
        assert_eq!(
            vm.eval(
                &r#"globalThis.promiseResults=[];
            globalThis.retainedPromise=(() => {
              const face=[...document.fonts][0];
              if(face.status!=='loading') throw new Error('CSS must already be loading');
              globalThis.faceWitness=new WeakRef(face);
              return QUERY_PROMISE;
            })();
            retainedPromise.then(value=>{const face=Array.isArray(value)?value[0]:value;
                promiseResults.push([face.family,face.status])},
              error=>promiseResults.push(['error',error.name]));
            document.fonts.size"#
                    .replace("QUERY_PROMISE", query_promise)
            )
            .unwrap(),
            "1"
        );
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap(),
        "/lifetime.woff2"
    );
    if mode.starts_with("css-removed") {
        assert_eq!(
            vm.eval("document.styleSheets[0].deleteRule(0);document.fonts.size")
                .unwrap(),
            "0"
        );
        vm.reconcile_document_font_resources();
        assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    }
    assert_eq!(vm.eval("JSON.stringify(promiseResults)").unwrap(), "[]");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );
    let pending_contexts = vm
        ._context_host
        .borrow()
        .pending_font_face_execution_contexts_for_test();
    assert_eq!(
        pending_contexts.len(),
        1,
        "{mode}: native load must root its face"
    );
    let execution_context = pending_contexts[0];
    let retired = mode.contains("retire-");
    if retired {
        let resource_owner_id = vm.resource_owner_id;
        let mut host = vm._context_host.borrow_mut();
        if mode.ends_with("retire-realm") {
            host.retire_window_realm_resources(execution_context.realm_token());
        } else if mode.ends_with("retire-owner") {
            assert!(host.retire_window_execution_context(execution_context.owner()));
        } else if mode.ends_with("retire-context") {
            assert_eq!(
                host.retire_window_execution_contexts_for_context_token(
                    execution_context.realm_token(),
                    Some(resource_owner_id),
                ),
                1
            );
        } else {
            panic!("unknown retirement mode {mode}");
        }
        assert!(
            host.pending_font_face_execution_contexts_for_test()
                .is_empty()
        );
    }
    // Every vm.eval HandleScope above has exited. Clear WeakRef's job-local
    // keepalive before forcing full GC so the witness cannot root the face.
    if mode != "css-removed-no-gc" {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(
        vm.eval("faceWitness.deref()===undefined").unwrap(),
        if retired { "true" } else { "false" },
        "{mode}: active loads must keep settlement endpoints alive, retired Windows must release them"
    );
    gates.remove("lifetime.woff2").unwrap().send(()).unwrap();
    server.await.unwrap();
    if retired {
        return;
    }
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_font_face_execution_contexts_for_test()
            .is_empty(),
        "{mode}: terminal loads must release their native roots"
    );
    let results = vm.eval("JSON.stringify(promiseResults)").unwrap();
    assert_eq!(
        results,
        if failure {
            r#"[["error","NetworkError"]]"#
        } else {
            r#"[["Lifetime","loaded"]]"#
        },
        "{mode}: retaining the promise chain must preserve its settlement endpoint"
    );
    // A fulfilled promise necessarily retains its FontFace result. A rejected
    // promise must release the wrapper even while that promise remains live.
    if !failure {
        vm.eval("globalThis.retainedPromise=null").unwrap();
    }
    vm.eval("if(document.styleSheets.length&&document.styleSheets[0].cssRules.length) document.styleSheets[0].deleteRule(0); void document.fonts.size").unwrap();
    run_font_tasks(&mut vm);
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        vm.eval("faceWitness.deref()===undefined").unwrap(),
        "true",
        "completed font wrappers must not be kept alive by their resolver"
    );
}

#[tokio::test]
async fn css_removed_loaded_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-gc").await;
}

#[tokio::test]
async fn css_removed_query_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-query-gc").await;
}

#[tokio::test]
async fn css_removed_rejected_query_promise_releases_wrapper_after_gc() {
    assert_loaded_promise_lifetime("css-removed-query-error").await;
}

#[tokio::test]
async fn css_removed_query_then_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-query-then-gc").await;
}

#[tokio::test]
async fn css_removed_rejected_query_then_promise_releases_wrapper_after_gc() {
    assert_loaded_promise_lifetime("css-removed-query-then-error").await;
}

#[tokio::test]
async fn css_removed_query_await_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-query-await-gc").await;
}

#[tokio::test]
async fn css_removed_rejected_query_await_promise_releases_wrapper_after_gc() {
    assert_loaded_promise_lifetime("css-removed-query-await-error").await;
}

#[tokio::test]
async fn css_removed_loaded_then_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-loaded-then-gc").await;
}

#[tokio::test]
async fn css_removed_rejected_loaded_then_promise_releases_wrapper_after_gc() {
    assert_loaded_promise_lifetime("css-removed-loaded-then-error").await;
}

#[tokio::test]
async fn css_removed_loaded_await_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-removed-loaded-await-gc").await;
}

#[tokio::test]
async fn css_removed_rejected_loaded_await_promise_releases_wrapper_after_gc() {
    assert_loaded_promise_lifetime("css-removed-loaded-await-error").await;
}

#[tokio::test]
async fn css_removed_query_chain_releases_wrapper_on_realm_retirement() {
    assert_loaded_promise_lifetime("css-removed-query-then-retire-realm").await;
}

#[tokio::test]
async fn css_removed_query_chain_releases_wrapper_on_owner_retirement() {
    assert_loaded_promise_lifetime("css-removed-query-await-retire-owner").await;
}

#[tokio::test]
async fn css_removed_query_chain_releases_wrapper_on_context_retirement() {
    assert_loaded_promise_lifetime("css-removed-query-then-retire-context").await;
}

#[tokio::test]
async fn css_removed_loaded_promise_without_forced_gc() {
    assert_loaded_promise_lifetime("css-removed-no-gc").await;
}

#[tokio::test]
async fn css_member_loaded_promise_survives_gc() {
    assert_loaded_promise_lifetime("css-member").await;
}

#[tokio::test]
async fn js_only_loaded_promise_survives_gc() {
    assert_loaded_promise_lifetime("js-only").await;
}

#[tokio::test]
async fn loading_handler_ready_cannot_finish_load_before_add() {
    let (origin, mut requests, mut gates, server) = gated_fonts(&["slow.woff2"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(&format!(r#"globalThis.fonts=document.fonts;
        globalThis.face=new FontFace('Slow','url("{origin}/slow.woff2")');
        globalThis.readyResolved=false;globalThis.doneEvents=[];globalThis.loadingEvents=0;
        fonts.addEventListener('loading',()=>{{loadingEvents++;fonts.ready.then(()=>readyResolved=true)}});
        fonts.addEventListener('loadingdone',event=>doneEvents.push(event.fontfaces.length));
        face.load();fonts.add(face);"#)).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap(),
        "/slow.woff2"
    );
    assert_eq!(
        vm.eval("loadingEvents").unwrap(),
        "0",
        "loading must run as a task after add returns"
    );
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("loadingEvents").unwrap(), "1");
    let blocked = vm
        .eval("JSON.stringify([face.status,fonts.status,readyResolved,doneEvents])")
        .unwrap();
    gates.remove("slow.woff2").unwrap().send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([face.status,readyResolved,doneEvents])")
            .unwrap(),
        r#"["loaded",true,[1]]"#
    );
    assert_eq!(blocked, r#"["loading","loading",false,[]]"#);
}

#[tokio::test]
async fn css_removed_loaded_promise_rejects_after_gc_and_releases_wrapper() {
    assert_loaded_promise_lifetime("css-removed-error").await;
}

#[tokio::test]
async fn font_set_finish_task_rechecks_fonts_started_before_dispatch() {
    let (origin, mut requests, mut gates, server) = gated_fonts(&["slow.woff2"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/page"),
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    let immediate = fixture_font_url();
    vm.eval(&format!(r#"
        const fonts=document.fonts;
        const first=new FontFace('First','url("{immediate}")');
        fonts.add(first);first.load();
        globalThis.firstReady=fonts.ready;globalThis.readyResolved=false;
        firstReady.then(()=>readyResolved=true);
        globalThis.doneEvents=[];
        fonts.addEventListener('loadingdone',event=>doneEvents.push(event.fontfaces.map(face=>face.family).sort()));
    "#)).unwrap();
    // Complete the first source, leaving its FontFaceSet finish task queued.
    // Starting another face before dispatch must make that task recheck idle.
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    vm.eval(&format!(
        r#"
        globalThis.slow=new FontFace('Slow','url("{origin}/slow.woff2")');
        fonts.add(slow);slow.load();
    "#
    ))
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap(),
        "/slow.woff2"
    );
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([slow.status,document.fonts.status,firstReady===document.fonts.ready,readyResolved,doneEvents])").unwrap(), r#"["loading","loading",true,false,[]]"#);
    gates.remove("slow.woff2").unwrap().send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([readyResolved,doneEvents])")
            .unwrap(),
        r#"[true,[["First","Slow"]]]"#
    );
}

#[tokio::test]
async fn font_loading_tasks_preserve_order_and_reentrant_cycle_promises() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://font-event-tasks.test/",
        &loader,
    );
    let source = fixture_font_url();
    page.eval(&format!(r#"
        globalThis.fontTrace=[];globalThis.secondCycle=false;
        const fonts=document.fonts;
        fonts.addEventListener('loading',()=>fontTrace.push('loading'));
        fonts.addEventListener('loadingerror',event=>fontTrace.push('error:'+event.fontfaces.map(face=>face.family).join(',')));
        fonts.addEventListener('loadingdone',event=>{{
          fontTrace.push('done:'+event.fontfaces.map(face=>face.family).join(','));
          if(!secondCycle) {{
            secondCycle=true;fonts.delete(first);
            const next=new FontFace('Next','url("{source}")');
            fonts.add(next);next.load();
            const nextReady=fonts.ready;
            fontTrace.push('new-cycle:'+(nextReady!==firstReady)+':'+fonts.status);
            nextReady.then(()=>fontTrace.push('next-ready'));
          }}
        }});
        const first=new FontFace('Failed','local("MoliDefinitelyNotAFont-009943")');
        fonts.add(first);first.load().catch(()=>{{}});
        const firstReady=fonts.ready;
        firstReady.then(()=>fontTrace.push('first-ready'));
        fontTrace.push('script');queueMicrotask(()=>fontTrace.push('microtask'));
    "#)).unwrap();
    assert_eq!(
        page.eval("JSON.stringify(fontTrace)").unwrap(),
        r#"["script","microtask"]"#
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "fontTrace.includes('next-ready')",
        "true",
        "font loading tasks should complete both cycles through the Page dispatcher",
    )
    .await;
    assert_eq!(
        page.eval("JSON.stringify(fontTrace)").unwrap(),
        r#"["script","microtask","loading","done:","new-cycle:true:loading","error:Failed","first-ready","loading","done:Next","next-ready"]"#
    );
}

#[tokio::test]
async fn font_loading_tasks_do_not_dispatch_after_their_window_retires() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://font-event-retirement.test/",
        &loader,
    );
    let source = fixture_font_url();
    page.eval(&format!(
        r#"
        globalThis.fontTrace=[];globalThis.fontTasksDrained=false;
        const frame=document.body.appendChild(document.createElement('iframe'));
        const child=frame.contentWindow,fonts=child.document.fonts;
        for(const type of ['loading','loadingdone','loadingerror'])
          fonts.addEventListener(type,()=>fontTrace.push(type));
        const face=new child.FontFace('Retired','url("{source}")');
        fonts.add(face);face.load();frame.remove();
        setTimeout(()=>fontTasksDrained=true,0);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "fontTasksDrained",
        "true",
        "the surviving Window timer should run after retired font tasks are discarded",
    )
    .await;
    assert_eq!(page.eval("JSON.stringify(fontTrace)").unwrap(), "[]");
}
